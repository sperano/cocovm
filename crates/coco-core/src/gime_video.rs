//! GIME-native hi-res text and graphics scanout (INIT0 COCO=0; `DESIGN.md` §6).
//!
//! Text (`$FF98` BP=0): 32/40/64/80 columns from the GIME's internal character
//! generator (`font_gime`), optionally with per-character attribute bytes
//! (foreground palette regs 8–15, background regs 0–7, blink, underline).
//! Graphics (BP=1, HSCREEN): 16–160 bytes per row unpacked at 1/2/4 bits per
//! pixel through the palette registers.
//!
//! Unlike the CoCo-compatible modes, GIME-native scanout addresses *physical*
//! RAM directly — the vertical offset registers give the start address and the
//! MMU is bypassed (SEB Unravelled II). Rendering is per scanline into the
//! canonical 640×240 raster (`raster.rs`, Option B): [`paint_scanline`] paints
//! one canvas row from the LIVE registers plus the per-field latched state in
//! [`FieldScan`], so mid-frame register writes take effect on the next line —
//! except the field-latched group ($FF9D/$FF9E base, $FF9C smooth-scroll
//! seed), which MAME `gime.cpp` `new_frame` samples once per field.
//! Register semantics verified against SEB Unravelled II and MAME `gime.cpp`
//! (see memory `gime-scanline-verified-facts`).

use serde::{Deserialize, Serialize};

use crate::font_gime::{GIME_FONT, GLYPH_ROWS};
use crate::gime::{self, GIME, hoff, vmode, vres};
use crate::raster::{CANVAS_H, CANVAS_W, NON_WIDE_ACTIVE_W, NON_WIDE_BORDER_X, vertical_window};
use crate::video::{BYTES_PER_PIXEL, PALETTE_LEN};

/// Character cell width in pixels (fixed by the 8-bit font rows).
pub const CHAR_W: usize = 8;

/// Attribute-byte fields (SEB Unravelled II Fig 4).
const ATTR_BLINK: u8 = 0x80;
const ATTR_UNDERLINE: u8 = 0x40;
const ATTR_FG_SHIFT: u8 = 3;
const ATTR_COLOR_MASK: u8 = 0x07;
/// Foreground colours come from palette registers 8–15.
const ATTR_FG_BASE: usize = 8;
/// Palette registers for text without attributes: background 0, foreground 1
/// (MAME `emit_gime_text_samples`).
const NO_ATTR_BG: usize = 0;
const NO_ATTR_FG: usize = 1;
/// A blinked-off character renders as a space (MAME `get_data_with_attributes`).
const BLANK_CHAR: u8 = 0x20;

/// Glyph-code mask: the font has 128 entries indexed by the low 7 bits.
const CHAR_CODE_MASK: u8 = 0x7F;

/// Border colour register mask ($FF9A): a 6-bit colour value.
const BORDER_COLOR_MASK: u8 = 0x3F;

/// Within a row, fetch offsets wrap at 256 bytes — the horizontal-virtual
/// "seam" (MAME `record_scanline_res`; SEB's "peculiar things" without HVEN).
const ROW_FETCH_WRAP: usize = 0x100;

/// Mask for the HRES field's low bit ($FF99 bit 2): the "wide" flag in
/// MAME's pixel path (`render_scanline`: `wide = !legacy && (ff99 & 0x04)`).
/// Wide modes fill the full 640 canvas px with no border; non-wide modes
/// fill the centre 512. (MAME's `update_geometry` tests bit 3 instead, but
/// only for field-sync timing — the emitted pixel widths follow bit 2.)
const WIDE_HRES_MASK: usize = 0x01;

/// A decoded GIME hi-res text mode.
pub struct TextMode {
    /// Character columns per row (32/40/64/80).
    pub cols: usize,
    /// Attribute bytes enabled: each char byte is followed by an attribute byte.
    pub attributes: bool,
    /// Active display lines in the field (LPF).
    pub lines: usize,
    /// Scan lines per character row (LPR).
    pub lines_per_row: usize,
}

/// Decode the text mode from the GIME video registers ($FF98/$FF99).
pub fn decode_text(g: &GIME) -> TextMode {
    let hres = ((g.vres & vres::HRES_MASK) >> vres::HRES_SHIFT) as usize;
    TextMode {
        cols: gime::TEXT_COLS[hres],
        attributes: g.vres & vres::TEXT_ATTR != 0,
        lines: g.lines_per_field(),
        lines_per_row: g.lines_per_row(),
    }
}

/// A decoded GIME graphics (HSCREEN) mode.
pub struct GraphicsMode {
    /// Bytes fetched per displayed row (HRES).
    pub bytes_per_row: usize,
    /// Bits per pixel (CRES): 1/2/4 for 2/4/16 colours.
    pub bpp: usize,
    /// Pixels across: `bytes_per_row * 8 / bpp`.
    pub width: usize,
    /// Active display lines in the field (LPF).
    pub lines: usize,
}

/// Decode the graphics mode from the GIME video registers ($FF98/$FF99).
///
/// HRES=%110/%111 (128/160 bytes per row) with CRES=%00 is not a guaranteed
/// combination (SEB Unravelled II Fig 5) and the chip does not produce a
/// 1024/1280-px picture: MAME `gime.cpp` (cases `0x18/0x19`, `0x1c/0x1d`)
/// aliases CRES=0 to the CRES=1 renderer there, so this decode does too.
pub fn decode_graphics(g: &GIME) -> GraphicsMode {
    let hres = ((g.vres & vres::HRES_MASK) >> vres::HRES_SHIFT) as usize;
    let bytes_per_row = gime::GFX_BYTES_PER_ROW[hres];
    let mut bpp = gime::GFX_BPP[(g.vres & vres::CRES_MASK) as usize];
    if bytes_per_row > gime::GFX_BYTES_PER_ROW[5] && bpp == 1 {
        bpp = 2;
    }
    GraphicsMode {
        bytes_per_row,
        bpp,
        width: bytes_per_row * 8 / bpp,
        lines: g.lines_per_field(),
    }
}

/// Per-field video scanout state, latched at field start — the register group
/// MAME `gime.cpp` `new_frame` samples once per field and never re-reads
/// mid-frame: the video base address ($FF9D/$FF9E), the INIT0 COCO
/// legacy-vs-GIME switch, and the VSC smooth-scroll seed ($FF9C). Everything
/// else ($FF98/$FF99 mode bits, $FF9F offset/HVEN, $FF9A border) is read live
/// per line by [`paint_scanline`].
#[derive(Serialize, Deserialize)]
pub struct FieldScan {
    /// Field latched with INIT0 COCO set: the whole field renders on the
    /// legacy VDG path (whole-frame, at field end) and per-line painting is
    /// skipped — a mid-frame COCO flip waits for the next field, like MAME's
    /// `m_legacy_video`.
    pub legacy: bool,
    /// Address of the current data row's first byte: physical (from the
    /// vertical-offset registers) for GIME-native fields, the 16-bit logical
    /// SAM page base for legacy fields (read through the bus/MMU). Advances
    /// by the current line's live pitch once per LPR lines (MAME
    /// `record_full_body_scanline`).
    pub(crate) row_base: usize,
    /// Scan line within the current data row: the smooth-scroll phase and,
    /// in text modes, the glyph row index — one shared counter, like MAME's
    /// `m_line_in_row`.
    pub(crate) line_in_row: usize,
}

impl FieldScan {
    /// Latch the per-field register group (MAME `new_frame`). Legacy fields
    /// seed from the SAM-compat page base with `line_in_row` 0 (MAME:
    /// `m_line_in_row = COCO ? 0 : vsc`).
    pub fn latch(g: &GIME, legacy: bool) -> Self {
        let vsc = (g.vertical_scroll & 0x0F) as usize;
        let lpr = g.lines_per_row();
        Self {
            legacy,
            row_base: if legacy {
                g.sam_display_base() as usize
            } else {
                g.video_base()
            },
            line_in_row: if legacy || vsc >= lpr { 0 } else { vsc },
        }
    }
}

/// Resolve the 16 GIME palette registers and the $FF9A border to RGBA.
fn resolve_colors(g: &GIME) -> ([[u8; 4]; PALETTE_LEN], [u8; 4]) {
    let mut palette = [[0u8; 4]; PALETTE_LEN];
    for (entry, &reg) in palette.iter_mut().zip(&g.palette) {
        *entry = g.color(reg);
    }
    (palette, g.color(g.border & BORDER_COLOR_MASK))
}

/// The scan line within a character row that the underline attribute lights,
/// per LPR — only defined for 8/9/10-line rows (SockMaster via MAME).
fn underline_line(lines_per_row: usize) -> Option<usize> {
    match lines_per_row {
        8 => Some(7),
        9 | 10 => Some(8),
        11 => Some(9),
        _ => None,
    }
}

/// Fill a pixel span with one colour.
fn fill(px: &mut [u8], color: [u8; 4]) {
    for p in px.chunks_exact_mut(BYTES_PER_PIXEL) {
        p.copy_from_slice(&color);
    }
}

/// True when canvas `row` falls within the active (non-border) vertical
/// window, from the LIVE LPF bits (applies even mid-frame; the glitched %10
/// value is approximated, see [`vertical_window`]).
fn in_active_rows(g: &GIME, row: usize) -> bool {
    let lpf = ((g.vres & vres::LPF_MASK) >> vres::LPF_SHIFT) as usize;
    let (top, body) = vertical_window(lpf);
    row >= top && row < top + body
}

/// Fill a body row's side borders per the LIVE wide flag ($FF99 HRES low
/// bit), returning the active-area `(x0, width)` slice bounds within it.
/// Wide modes fill the full `CANVAS_W` with no border; non-wide modes leave
/// the border strips.
fn paint_side_borders(g: &GIME, row_px: &mut [u8], border: [u8; 4]) -> (usize, usize) {
    let hres = ((g.vres & vres::HRES_MASK) >> vres::HRES_SHIFT) as usize;
    let wide = hres & WIDE_HRES_MASK != 0;
    let (x0, active_w) = if wide {
        (0, CANVAS_W)
    } else {
        (NON_WIDE_BORDER_X, NON_WIDE_ACTIVE_W)
    };
    if !wide {
        fill(&mut row_px[..x0 * BYTES_PER_PIXEL], border);
        fill(&mut row_px[(x0 + active_w) * BYTES_PER_PIXEL..], border);
    }
    (x0, active_w)
}

/// Paint one body row's active span (text or graphics, per the LIVE $FF98 BP
/// bit) from `row_base`/`x_offset`-derived fetch addresses. Returns the
/// number of bytes this row consumed, for [`advance_scan`]'s pitch.
#[allow(clippy::too_many_arguments)]
fn paint_body_row(
    g: &GIME,
    ram: &[u8],
    row_base: usize,
    x_offset: usize,
    palette: &[[u8; 4]; PALETTE_LEN],
    blink_on: bool,
    line_in_row: usize,
    active_w: usize,
    active: &mut [u8],
) -> usize {
    let fetch = |i: usize| ram[(row_base + ((x_offset + i) % ROW_FETCH_WRAP)) % ram.len()];
    if g.vmode & vmode::BP != 0 {
        let mode = decode_graphics(g);
        paint_graphics_row(&mode, palette, active_w, fetch, active);
        mode.bytes_per_row
    } else {
        let mode = decode_text(g);
        paint_text_row(
            &mode,
            palette,
            blink_on,
            line_in_row,
            active_w,
            fetch,
            active,
        );
        mode.cols * if mode.attributes { 2 } else { 1 }
    }
}

/// Advance `scan`'s shared vertical counter after painting a body row of
/// `row_bytes` bytes: the row pointer steps by the CURRENT line's live pitch
/// once per LPR lines (MAME `record_full_body_scanline`; LPR=%111's huge
/// count never wraps).
fn advance_scan(scan: &mut FieldScan, g: &GIME, row_bytes: usize) {
    let pitch = if g.horizontal_offset & hoff::HVEN != 0 {
        gime::HVEN_ROW_BYTES
    } else {
        row_bytes
    };
    scan.line_in_row += 1;
    if scan.line_in_row >= g.lines_per_row() {
        scan.line_in_row = 0;
        scan.row_base += pitch;
    }
}

/// Paint one canvas row of the canonical raster from the live GIME registers
/// plus the field-latched state in `scan`, advancing `scan`'s vertical
/// counters on body rows. `fb` is the full `CANVAS_W`×`CANVAS_H` buffer;
/// `row` is the canvas row (== machine scanline) to paint.
pub fn paint_scanline(
    g: &GIME,
    ram: &[u8],
    scan: &mut FieldScan,
    blink_on: bool,
    row: usize,
    fb: &mut [u8],
) {
    debug_assert!(row < CANVAS_H);
    let (palette, border) = resolve_colors(g);
    let row_px = &mut fb[row * CANVAS_W * BYTES_PER_PIXEL..][..CANVAS_W * BYTES_PER_PIXEL];

    if !in_active_rows(g, row) {
        fill(row_px, border);
        return;
    }

    let (x0, active_w) = paint_side_borders(g, row_px, border);
    let active = &mut row_px[x0 * BYTES_PER_PIXEL..][..active_w * BYTES_PER_PIXEL];

    // Per-line live fetch parameters ($FF9F offset).
    let x_offset = (g.horizontal_offset & hoff::X_MASK) as usize * 2;
    let row_bytes = paint_body_row(
        g,
        ram,
        scan.row_base,
        x_offset,
        &palette,
        blink_on,
        scan.line_in_row,
        active_w,
        active,
    );

    advance_scan(scan, g, row_bytes);
}

/// Paint one text scan line into the active span, `xscale`-duplicating each
/// native pixel to fill `active_w`.
fn paint_text_row(
    mode: &TextMode,
    palette: &[[u8; 4]; PALETTE_LEN],
    blink_on: bool,
    line_in_row: usize,
    active_w: usize,
    fetch: impl Fn(usize) -> u8,
    out: &mut [u8],
) {
    let bytes_per_char = if mode.attributes { 2 } else { 1 };
    let underline = underline_line(mode.lines_per_row);
    let native_w = mode.cols * CHAR_W;
    let xscale = (active_w / native_w).max(1);

    let mut x = 0;
    for col in 0..mode.cols {
        let mut code = fetch(col * bytes_per_char);
        let (fg, bg, underlined) = if mode.attributes {
            let attr = fetch(col * bytes_per_char + 1);
            if attr & ATTR_BLINK != 0 && blink_on {
                code = BLANK_CHAR;
            }
            (
                palette[ATTR_FG_BASE + ((attr >> ATTR_FG_SHIFT) & ATTR_COLOR_MASK) as usize],
                palette[(attr & ATTR_COLOR_MASK) as usize],
                attr & ATTR_UNDERLINE != 0 && !(attr & ATTR_BLINK != 0 && blink_on),
            )
        } else {
            (palette[NO_ATTR_FG], palette[NO_ATTR_BG], false)
        };

        let glyph = &GIME_FONT[(code & CHAR_CODE_MASK) as usize];
        let row_bits = if line_in_row < GLYPH_ROWS {
            glyph[line_in_row]
        } else {
            0
        };
        let underline_here = underlined && Some(line_in_row) == underline;
        for cx in 0..CHAR_W {
            let on = underline_here || row_bits & (0x80 >> cx) != 0;
            let color = if on { fg } else { bg };
            fill(
                &mut out[x * BYTES_PER_PIXEL..][..xscale * BYTES_PER_PIXEL],
                color,
            );
            x += xscale;
        }
    }
}

/// Paint one graphics scan line into the active span, `xscale`-duplicating
/// each native pixel to fill `active_w`.
fn paint_graphics_row(
    mode: &GraphicsMode,
    palette: &[[u8; 4]; PALETTE_LEN],
    active_w: usize,
    fetch: impl Fn(usize) -> u8,
    out: &mut [u8],
) {
    let pixels_per_byte = 8 / mode.bpp;
    let value_mask = (1u8 << mode.bpp) - 1;
    let xscale = (active_w / mode.width.max(1)).max(1);

    let mut x = 0;
    for bx in 0..mode.bytes_per_row {
        let byte = fetch(bx);
        for j in 0..pixels_per_byte {
            // Pixels are packed MSB-first within the byte.
            let shift = 8 - mode.bpp * (j + 1);
            let color = palette[((byte >> shift) & value_mask) as usize];
            if x + xscale > active_w {
                return; // defensive: never paint past the active span
            }
            fill(
                &mut out[x * BYTES_PER_PIXEL..][..xscale * BYTES_PER_PIXEL],
                color,
            );
            x += xscale;
        }
    }
}

/// Render a full GIME-native field into `fb` (resized to the canonical
/// 640×240) from the CURRENT register latch — the whole-field equivalent of
/// stepping [`paint_scanline`] over every visible row. Headless tests poke
/// registers and call this; the machine loop instead paints line by line so
/// mid-frame changes split the raster. Returns the canvas dimensions.
pub fn render_field(g: &GIME, ram: &[u8], blink_on: bool, fb: &mut Vec<u8>) -> (usize, usize) {
    fb.resize(CANVAS_W * CANVAS_H * BYTES_PER_PIXEL, 0);
    let mut scan = FieldScan::latch(g, false);
    for row in 0..CANVAS_H {
        paint_scanline(g, ram, &mut scan, blink_on, row, fb);
    }
    (CANVAS_W, CANVAS_H)
}

/// Walks the GIME's video fetch addresses for the text-dump probe: a physical
/// row base advancing by the row pitch, with per-byte offsets (including the
/// $FF9F X offset ×2) wrapping at the 256-byte seam. Matches MAME
/// `record_scanline_res` / `new_frame`.
struct Scanout<'a> {
    ram: &'a [u8],
    row_base: usize,
    x_offset: usize,
    pitch: usize,
    line_in_row: usize,
    lines_per_row: usize,
}

impl<'a> Scanout<'a> {
    fn new(g: &GIME, ram: &'a [u8], row_bytes: usize, lines_per_row: usize) -> Self {
        let hven = g.horizontal_offset & hoff::HVEN != 0;
        // The field starts mid-row at the vertical-scroll line (smooth scroll).
        let vsc = (g.vertical_scroll & 0x0F) as usize;
        Self {
            ram,
            row_base: g.video_base(),
            x_offset: (g.horizontal_offset & hoff::X_MASK) as usize * 2,
            pitch: if hven {
                gime::HVEN_ROW_BYTES
            } else {
                row_bytes
            },
            line_in_row: if vsc < lines_per_row { vsc } else { 0 },
            lines_per_row,
        }
    }

    /// Fetch the `i`th byte of the current row.
    fn fetch(&self, i: usize) -> u8 {
        let addr = self.row_base + ((self.x_offset + i) % ROW_FETCH_WRAP);
        self.ram[addr % self.ram.len()]
    }

    /// Advance one scan line; steps to the next data row every `lines_per_row`.
    fn next_line(&mut self) {
        self.line_in_row += 1;
        if self.line_in_row >= self.lines_per_row {
            self.line_in_row = 0;
            self.row_base += self.pitch;
        }
    }
}

/// ASCII stand-in for a GIME hi-res text character code that has no printable
/// ASCII meaning: codes $00-$1F are accented/special glyphs, not C0 control
/// codes (`font_gime.rs`), so they can't be rendered as their own ASCII value.
const UNPRINTABLE_CHAR: char = '.';

/// Decode a GIME hi-res text field to plain ASCII strings, one per character
/// row — a debug/probe dump, not a renderer. Shares [`decode_text`] and
/// [`Scanout`] with the real painters so the two can't drift apart; unlike
/// [`paint_scanline`] this ignores attribute bytes' colour/blink/underline
/// fields (only the character byte of each cell is read) and scan lines
/// (each text row is fetched once, not once per [`TextMode::lines_per_row`]).
///
/// Character bytes are ASCII from $20 up (`font_gime.rs`); $00-$1F are
/// accented/special glyphs with no ASCII equivalent and print as
/// [`UNPRINTABLE_CHAR`].
pub fn text_lines(g: &GIME, ram: &[u8]) -> Vec<String> {
    let mode = decode_text(g);
    let bytes_per_char = if mode.attributes { 2 } else { 1 };
    let mut scan = Scanout::new(g, ram, mode.cols * bytes_per_char, mode.lines_per_row);
    let rows = mode.lines.checked_div(mode.lines_per_row).unwrap_or(0);

    let mut out = Vec::with_capacity(rows);
    for _ in 0..rows {
        let line = (0..mode.cols)
            .map(|col| {
                let code = scan.fetch(col * bytes_per_char) & CHAR_CODE_MASK;
                if (0x20..0x7F).contains(&code) {
                    code as char
                } else {
                    UNPRINTABLE_CHAR
                }
            })
            .collect();
        out.push(line);
        for _ in 0..mode.lines_per_row {
            scan.next_line();
        }
    }
    out
}
