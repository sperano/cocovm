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
//! MMU is bypassed (SEB Unravelled II). Each mode renders at its native pixel
//! size into a variable-size framebuffer and the frontend scales to fit
//! (`video-output-architecture` Option A). Register semantics verified against
//! SEB Unravelled II and MAME `gime.cpp`.

use crate::font_gime::{GIME_FONT, GLYPH_ROWS};
use crate::gime::{self, GIME, hoff, vres};
use crate::video::{BYTES_PER_PIXEL, PALETTE_LEN};

/// Character cell width in pixels (fixed by the 8-bit font rows).
pub const CHAR_W: usize = 8;

/// Vertical border thickness in native lines.
pub const BORDER_Y: usize = 16;
/// The horizontal border scales with the active width (width ÷ 16) so every
/// mode keeps the same border-to-picture proportion; all legal GIME widths
/// (128–640) divide evenly.
pub const BORDER_X_DIVISOR: usize = 16;

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
pub fn decode_graphics(g: &GIME) -> GraphicsMode {
    let hres = ((g.vres & vres::HRES_MASK) >> vres::HRES_SHIFT) as usize;
    let bytes_per_row = gime::GFX_BYTES_PER_ROW[hres];
    let bpp = gime::GFX_BPP[(g.vres & vres::CRES_MASK) as usize];
    GraphicsMode {
        bytes_per_row,
        bpp,
        width: bytes_per_row * 8 / bpp,
        lines: g.lines_per_field(),
    }
}

/// Walks the GIME's video fetch addresses: a physical row base advancing by the
/// row pitch, with per-byte offsets (including the $FF9F X offset ×2) wrapping
/// at the 256-byte seam. Matches MAME `record_scanline_res` / `new_frame`.
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

/// Size `fb` for an active area plus border and fill it with the border colour;
/// returns (fb_w, fb_h).
fn prepare_fb(
    fb: &mut Vec<u8>,
    active_w: usize,
    active_h: usize,
    border: [u8; 4],
) -> (usize, usize) {
    let fb_w = active_w + 2 * (active_w / BORDER_X_DIVISOR);
    let fb_h = active_h + 2 * BORDER_Y;
    fb.resize(fb_w * fb_h * BYTES_PER_PIXEL, 0);
    for px in fb.chunks_exact_mut(BYTES_PER_PIXEL) {
        px.copy_from_slice(&border);
    }
    (fb_w, fb_h)
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

/// ASCII stand-in for a GIME hi-res text character code that has no printable
/// ASCII meaning: codes $00-$1F are accented/special glyphs, not C0 control
/// codes (`font_gime.rs`), so they can't be rendered as their own ASCII value.
const UNPRINTABLE_CHAR: char = '.';

/// Decode a GIME hi-res text field to plain ASCII strings, one per character
/// row — a debug/probe dump, not a renderer. Shares [`decode_text`] and
/// [`Scanout`] with [`render_text`] so the two can't drift apart; unlike
/// [`render_text`] this ignores attribute bytes' colour/blink/underline
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
                if (0x20..0x7F).contains(&code) { code as char } else { UNPRINTABLE_CHAR }
            })
            .collect();
        out.push(line);
        for _ in 0..mode.lines_per_row {
            scan.next_line();
        }
    }
    out
}

/// Render a GIME hi-res text field into `fb` (resized to fit); returns the new
/// framebuffer dimensions. `ram` is physical memory; `blink_on` is the blink
/// phase (blinking characters are blanked while it is true).
pub fn render_text(g: &GIME, ram: &[u8], blink_on: bool, fb: &mut Vec<u8>) -> (usize, usize) {
    let mode = decode_text(g);
    let (palette, border) = resolve_colors(g);
    let active_w = mode.cols * CHAR_W;
    let (fb_w, fb_h) = prepare_fb(fb, active_w, mode.lines, border);

    let bytes_per_char = if mode.attributes { 2 } else { 1 };
    let underline = underline_line(mode.lines_per_row);
    let mut scan = Scanout::new(g, ram, mode.cols * bytes_per_char, mode.lines_per_row);
    let border_x = active_w / BORDER_X_DIVISOR;

    for y in 0..mode.lines {
        let row_start = ((BORDER_Y + y) * fb_w + border_x) * BYTES_PER_PIXEL;
        for col in 0..mode.cols {
            let mut code = scan.fetch(col * bytes_per_char);
            let (fg, bg, underlined) = if mode.attributes {
                let attr = scan.fetch(col * bytes_per_char + 1);
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
            let row_bits = if scan.line_in_row < GLYPH_ROWS {
                glyph[scan.line_in_row]
            } else {
                0
            };
            let underline_here = underlined && Some(scan.line_in_row) == underline;
            let cell = row_start + col * CHAR_W * BYTES_PER_PIXEL;
            for cx in 0..CHAR_W {
                let on = underline_here || row_bits & (0x80 >> cx) != 0;
                let color = if on { fg } else { bg };
                let idx = cell + cx * BYTES_PER_PIXEL;
                fb[idx..idx + BYTES_PER_PIXEL].copy_from_slice(&color);
            }
        }
        scan.next_line();
    }
    (fb_w, fb_h)
}

/// Render a GIME graphics (HSCREEN) field into `fb` (resized to fit); returns
/// the new framebuffer dimensions. `ram` is physical memory.
pub fn render_graphics(g: &GIME, ram: &[u8], fb: &mut Vec<u8>) -> (usize, usize) {
    let mode = decode_graphics(g);
    let (palette, border) = resolve_colors(g);
    let (fb_w, fb_h) = prepare_fb(fb, mode.width, mode.lines, border);

    let mut scan = Scanout::new(g, ram, mode.bytes_per_row, g.lines_per_row());
    let border_x = mode.width / BORDER_X_DIVISOR;
    let pixels_per_byte = 8 / mode.bpp;
    let value_mask = (1u8 << mode.bpp) - 1;

    for y in 0..mode.lines {
        let row_start = ((BORDER_Y + y) * fb_w + border_x) * BYTES_PER_PIXEL;
        for bx in 0..mode.bytes_per_row {
            let byte = scan.fetch(bx);
            for j in 0..pixels_per_byte {
                // Pixels are packed MSB-first within the byte.
                let shift = 8 - mode.bpp * (j + 1);
                let color = palette[((byte >> shift) & value_mask) as usize];
                let idx = row_start + (bx * pixels_per_byte + j) * BYTES_PER_PIXEL;
                fb[idx..idx + BYTES_PER_PIXEL].copy_from_slice(&color);
            }
        }
        scan.next_line();
    }
    (fb_w, fb_h)
}
