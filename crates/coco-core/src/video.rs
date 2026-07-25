//! VDG-compatible 32×16 text scanout (`DESIGN.md` §6).
//!
//! Renders the CoCo-compatible alphanumeric text screen to an RGBA framebuffer
//! using the authentic MC6847 character generator (`font6847`). Each screen byte
//! holds the VDG internal glyph code in its low 6 bits (`code & 0x3F`).
//!
//! Colours are data-driven from the GIME palette registers the ROM programmed, not
//! hardcoded: CoCo-compatible text takes its background from palette reg
//! [`TEXT_BG_INDEX`] and its foreground from [`TEXT_FG_INDEX`] (the MC6847 text
//! `color_base_0`/`color_base_1`), and the legacy text border is black. At the
//! stock BASIC prompt that resolves to pure green (`#00FF00`) on black. The CSS
//! orange colour set is TODO (`§6`); GIME native text/graphics live in `gime_video`.

use crate::font6847::{MC6847_FONT, MC6847T1_FONT};
use crate::font_gime::GIME_LOWRES_FONT;

/// VDG character cell: 8 pixels wide × 12 raster lines (matches the font rows).
pub const CELL_W: usize = 8;
pub const CELL_H: usize = 12;

pub const COLS: usize = 32;
pub const ROWS: usize = 16;

/// Active display geometry.
pub const ACTIVE_W: usize = COLS * CELL_W; // 256
pub const ACTIVE_H: usize = ROWS * CELL_H; // 192
/// Border thickness around the active area.
pub const BORDER: usize = 16;
pub const FB_W: usize = ACTIVE_W + 2 * BORDER; // 288
pub const FB_H: usize = ACTIVE_H + 2 * BORDER; // 224

pub const BYTES_PER_PIXEL: usize = 4;

/// The text screen is 512 bytes (32×16).
pub const SCREEN_LEN: usize = COLS * ROWS;

/// GIME palette registers that colour CoCo-compatible text (MC6847 text
/// `color_base_0`/`color_base_1`): glyph background and glyph foreground.
pub const TEXT_BG_INDEX: usize = 12;
pub const TEXT_FG_INDEX: usize = 13;

/// Number of resolved palette entries (GIME palette registers).
pub const PALETTE_LEN: usize = 16;

// --- CoCo 1/2 fixed VDG colour source (`docs/coco12-plan.md` Phase 3) -----------
//
// A CoCo 1/2 has no palette registers: the MC6847's colours are hardwired analog
// levels. `VDG_FIXED_PALETTE` reproduces MAME's `mc6847_base_device::s_palette`
// (`mc6847.cpp`) in the same 16-entry index layout the GIME palette registers use
// for CoCo-compatible mode (SEB Fig 13) — entries 0-11 are exactly the values the
// CoCo 3 ROM programs into palette regs 0-11 at cold start, which is why
// `ColorSource::GimePalette` and `ColorSource::VdgFixed` can share every other
// piece of this module (`vdg_palette_indices`, `decode_vdg_graphics`, ...).

/// Palette index of the eight VDG colours (0-7), the 2-colour graphics table
/// (8-11: black/green/black/buff), and the two alphanumeric colour sets
/// (12-13 green, 14-15 orange) — MAME `mc6847.cpp` `s_palette`.
pub const VDG_FIXED_PALETTE: [[u8; 4]; PALETTE_LEN] = [
    [0x30, 0xd2, 0x00, 0xFF], // 0  GREEN
    [0xc1, 0xe5, 0x00, 0xFF], // 1  YELLOW
    [0x4c, 0x3a, 0xb4, 0xFF], // 2  BLUE
    [0x9a, 0x32, 0x36, 0xFF], // 3  RED
    [0xbf, 0xc8, 0xad, 0xFF], // 4  BUFF
    [0x41, 0xaf, 0x71, 0xFF], // 5  CYAN
    [0xc8, 0x4e, 0xf0, 0xFF], // 6  MAGENTA
    [0xd4, 0x7f, 0x00, 0xFF], // 7  ORANGE
    [0x26, 0x30, 0x16, 0xFF], // 8  BLACK
    [0x30, 0xd2, 0x00, 0xFF], // 9  GREEN
    [0x26, 0x30, 0x16, 0xFF], // 10 BLACK
    [0xbf, 0xc8, 0xad, 0xFF], // 11 BUFF
    [0x00, 0x7c, 0x00, 0xFF], // 12 ALPHANUMERIC DARK GREEN
    [0x30, 0xd2, 0x00, 0xFF], // 13 ALPHANUMERIC BRIGHT GREEN
    [0x6b, 0x27, 0x00, 0xFF], // 14 ALPHANUMERIC DARK ORANGE
    [0xff, 0xb7, 0x00, 0xFF], // 15 ALPHANUMERIC BRIGHT ORANGE
];

/// Palette index of the orange alphanumeric set's background/foreground
/// (selected in place of [`TEXT_BG_INDEX`]/[`TEXT_FG_INDEX`] when CSS=1).
const ALPHA_ORANGE_BG: usize = 14;
const ALPHA_ORANGE_FG: usize = 15;

/// Palette index of the fixed text-mode border (MAME `mc6847.cpp`
/// `border_value`: alphanumeric/semigraphics border is always black).
pub const TEXT_BORDER_INDEX: usize = 8;
/// Palette index of the fixed graphics-mode border when CSS=0 (green — MAME
/// `mc6847.cpp` `border_value`: `(~mode & MODE_CSS) ? 0 : 4`).
const GRAPHICS_BORDER_CSS0_INDEX: usize = 0;
/// Palette index of the fixed graphics-mode border when CSS=1 (buff/white).
const GRAPHICS_BORDER_CSS1_INDEX: usize = 4;

/// The VDG graphics-mode border colour's palette index: green (CSS=0) or
/// buff (CSS=1) — real hardware does *not* border graphics modes in black
/// (MAME `mc6847.cpp` `border_value`).
pub fn vdg_graphics_border_index(css: bool) -> usize {
    if css {
        GRAPHICS_BORDER_CSS1_INDEX
    } else {
        GRAPHICS_BORDER_CSS0_INDEX
    }
}

/// Where the CoCo-compatible text/graphics renderers resolve their 16-entry
/// colour table from: the GIME palette registers (`GimePalette` — the
/// existing CoCo 3 behaviour, unchanged by this enum: the ROM initializes
/// those registers to the VDG defaults) or the hardwired VDG RGB table
/// (`VdgFixed` — CoCo 1/2, which has no palette registers to program). See
/// `docs/coco12-plan.md` Phase 3.
pub enum ColorSource<'a> {
    GIMEPalette(&'a [[u8; 4]; PALETTE_LEN]),
    VDGFixed,
}

impl ColorSource<'_> {
    /// Resolve to a 16-entry RGBA table in the shared index layout (see the
    /// module doc above). `css` (PIA1 $FF22 bit 3) only affects `VdgFixed`:
    /// it swaps the orange alphanumeric set into [`TEXT_BG_INDEX`]/
    /// [`TEXT_FG_INDEX`] so callers that only ever read those two constants
    /// (as [`render_text`] does) don't need to know about CSS themselves.
    /// `GimePalette` ignores `css` — the GIME's own registers already hold
    /// whatever the ROM programmed at 12/13 unconditionally, matching the
    /// pre-Phase-3 behaviour exactly.
    pub fn resolve(&self, css: bool) -> [[u8; 4]; PALETTE_LEN] {
        match *self {
            ColorSource::GIMEPalette(p) => *p,
            ColorSource::VDGFixed => {
                let mut table = VDG_FIXED_PALETTE;
                if css {
                    table[TEXT_BG_INDEX] = VDG_FIXED_PALETTE[ALPHA_ORANGE_BG];
                    table[TEXT_FG_INDEX] = VDG_FIXED_PALETTE[ALPHA_ORANGE_FG];
                }
                table
            }
        }
    }
}

/// Number of glyphs in the font (VDG codes $00–$3F).
const GLYPH_COUNT: usize = 64;
/// MC6847 attribute bits within a screen byte.
const SEMIGRAPHICS_BIT: u8 = 0x80; // bit 7 — 1 = semigraphics 4, 0 = alphanumeric
const INVERSE_BIT: u8 = 0x40; // bit 6 — inverse video (alphanumeric only)
const GLYPH_CODE_MASK: u8 = 0x3F; // bits 5-0 — alphanumeric glyph code

// Semigraphics 4: bits 6-4 select a colour (GIME palette reg 0–7), bits 3-0 are a
// 2×2 block pattern; unlit blocks use palette reg 8 (black in CoCo-compat mode).
const SG4_COLOR_SHIFT: u8 = 4;
const SG4_COLOR_MASK: u8 = 0x07;
const SG4_OFF_INDEX: usize = 8;
const SG4_UPPER_LEFT: u8 = 0x08;
const SG4_UPPER_RIGHT: u8 = 0x04;
const SG4_LOWER_LEFT: u8 = 0x02;
const SG4_LOWER_RIGHT: u8 = 0x01;

/// Decode one VDG alphanumeric screen byte's low 6 bits to the ASCII
/// character it displays (ignores the semigraphics/inverse attribute bits —
/// callers doing a plain-text dump don't care which glyph variant drew it).
/// The MC6847 alphanumeric code space is `$00-$1F` -> `@A-Z[\]^_` (`@` + code)
/// and `$20-$3F` -> a second copy of the ASCII block starting at space
/// (`' '` + (code - `$20`)) — the same mapping used throughout this crate's
/// tests and probes for the CoCo-compatible text screen.
pub fn decode_alpha_char(code: u8) -> char {
    let code = code & GLYPH_CODE_MASK;
    if code < 0x20 {
        (b'@' + code) as char
    } else {
        (b' ' + (code - 0x20)) as char
    }
}

/// Which character-generator ROM is actually driving CoCo-compatible text
/// mode. Distinct from [`crate::config::VDGVariant`]: that's "which VDG chip
/// is this CoCo 1/2" and doesn't apply to a CoCo 3 at all — a real CoCo 3 has
/// no VDG; the GIME does its own compat-text generation with its own font
/// ROM ([`crate::font_gime::GIME_LOWRES_FONT`]), which happens to share the
/// MC6847T1's true-lowercase semantics (MAME `gime.cpp`'s `gime_device` ctor
/// constructs its `mc6847_friend_device` base with `is_mc6847t1 = true`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaGenerator {
    /// CoCo 1/2 with the original MC6847.
    MC6847,
    /// CoCo 1/2 with the MC6847T1.
    MC6847T1,
    /// CoCo 3 CoCo-compatible text mode: the GIME's own generator, T1-style
    /// lowercase semantics, [`crate::font_gime::GIME_LOWRES_FONT`] glyphs.
    GIME,
}

/// Resolve one alphanumeric cell's glyph and (foreground, background) colours.
///
/// Implements MAME `mc6847.cpp`'s `character_map` ctor precisely (and, for
/// [`AlphaGenerator::GIME`], `gime.cpp`'s equivalent, which shares the same
/// true-lowercase logic per its `is_mc6847t1 = true` construction):
/// - True lowercase only applies on the MC6847T1 or the GIME generator, when
///   this character's own inverse bit is clear, PIA1 $FF22 GM0
///   ([`VDG_GM0_INTEXT`]) is set, and the code is in `$00-$1F` — in which
///   case the glyph comes from the selected font's lowercase section (index
///   `0x40 + code`) and the fg/bg pair is *swapped* relative to the normal
///   non-inverse mapping (equivalent to MAME's `raw_glyph ^ 0xFF` drawn
///   non-inverted, in the inverse-toggle style this module already uses for
///   [`INVERSE_BIT`]).
/// - Codes `$20-$3F` are never affected by lowercase mode (MAME's ctor copies
///   them unchanged into the lowercase table too).
/// - Every other case (plain [`AlphaGenerator::MC6847`], a lowercase-capable
///   generator with GM0 clear, a lowercase-capable generator with this
///   character's own inverse bit set, or code >= `$20`) draws from the
///   normal 64-entry range of the selected font, fg/bg swapped by the
///   inverse bit alone.
///
/// PIA1 $FF22 GM1 (bit 5) drives a second, lowercase-independent colour
/// inversion on the T1 (MAME's `is_inverse2`) that is out of scope here — see
/// the note by [`VDG_GM0_INTEXT`].
fn resolve_alpha_cell(
    generator: AlphaGenerator,
    ff22: u8,
    code: u8,
    fg: [u8; 4],
    bg: [u8; 4],
) -> ([u8; 4], [u8; 4], &'static [u8; CELL_H]) {
    let glyph_code = code & GLYPH_CODE_MASK;
    let inverse = code & INVERSE_BIT != 0;
    let lowercase_capable = matches!(generator, AlphaGenerator::MC6847T1 | AlphaGenerator::GIME);
    let true_lowercase =
        lowercase_capable && !inverse && ff22 & VDG_GM0_INTEXT != 0 && glyph_code < 0x20;
    if true_lowercase {
        let glyph: &[u8; CELL_H] = match generator {
            AlphaGenerator::MC6847T1 => &MC6847T1_FONT[0x40 + glyph_code as usize],
            AlphaGenerator::GIME => &GIME_LOWRES_FONT[0x40 + glyph_code as usize],
            AlphaGenerator::MC6847 => unreachable!("Mc6847 is never lowercase_capable"),
        };
        (bg, fg, glyph)
    } else {
        let glyph: &[u8; CELL_H] = match generator {
            AlphaGenerator::MC6847 => &MC6847_FONT[glyph_code as usize % GLYPH_COUNT],
            AlphaGenerator::MC6847T1 => &MC6847T1_FONT[glyph_code as usize % GLYPH_COUNT],
            AlphaGenerator::GIME => &GIME_LOWRES_FONT[glyph_code as usize % GLYPH_COUNT],
        };
        let (cell_fg, cell_bg) = if inverse { (bg, fg) } else { (fg, bg) };
        (cell_fg, cell_bg, glyph)
    }
}

/// PIA1 $FF22 bit 6 (GM2) and bit 5 (GM1), used by the CoCo 3 legacy border
/// rule below.
const VDG_GM2: u8 = 0x40;
const VDG_GM1: u8 = 0x20;

/// GIME 6-bit colour values the CoCo 3 legacy border resolves to (MAME
/// `gime.cpp` `update_border`, legacy branch).
const BORDER6_BLACK: u8 = 0x00;
const BORDER6_GREEN: u8 = 0x12;
const BORDER6_ORANGE: u8 = 0x26;
const BORDER6_WHITE: u8 = 0x3F;

/// The CoCo 3 legacy-mode border colour as a GIME 6-bit value, from the live
/// $FF22 (MAME `gime.cpp` `update_border`): graphics borders are green
/// (CSS=0) or white (CSS=1); the GM2-without-GM1 text variant borders green
/// or orange; every other text/semigraphics mode borders black. (The CoCo
/// 1/2 path resolves its border from the fixed VDG palette instead — see
/// [`vdg_graphics_border_index`]/[`TEXT_BORDER_INDEX`].)
pub fn legacy_border_value(ff22: u8) -> u8 {
    let css = ff22 & VDG_CSS != 0;
    if ff22 & VDG_AG != 0 {
        if css { BORDER6_WHITE } else { BORDER6_GREEN }
    } else if ff22 & VDG_GM2 != 0 && ff22 & VDG_GM1 == 0 {
        if css { BORDER6_ORANGE } else { BORDER6_GREEN }
    } else {
        BORDER6_BLACK
    }
}

/// Paint one scan line of the legacy 32-column text screen into `out`
/// (an active-area pixel span), duplicating each native pixel `xscale`
/// times. `row_bytes` is the 32 screen bytes of the current character row;
/// `glyph_row` the scan line within it (`0..CELL_H`). Cell resolution
/// (alpha vs SG4, inverse, true lowercase) matches [`render_text`], which
/// shares [`resolve_alpha_cell`].
pub fn paint_legacy_text_line(
    row_bytes: &[u8],
    palette: &[[u8; 4]; PALETTE_LEN],
    generator: AlphaGenerator,
    ff22: u8,
    glyph_row: usize,
    xscale: usize,
    out: &mut [u8],
) {
    let fg = palette[TEXT_FG_INDEX];
    let bg = palette[TEXT_BG_INDEX];
    let mut x = 0;
    for col in 0..COLS {
        let code = row_bytes.get(col).copied().unwrap_or(0);
        if code & SEMIGRAPHICS_BIT != 0 {
            // Semigraphics 4: upper blocks on rows 0..CELL_H/2, lower below.
            let on = palette[((code >> SG4_COLOR_SHIFT) & SG4_COLOR_MASK) as usize];
            let off = palette[SG4_OFF_INDEX];
            let bottom = glyph_row >= CELL_H / 2;
            for cx in 0..CELL_W {
                let right = cx >= CELL_W / 2;
                let block = match (bottom, right) {
                    (false, false) => SG4_UPPER_LEFT,
                    (false, true) => SG4_UPPER_RIGHT,
                    (true, false) => SG4_LOWER_LEFT,
                    (true, true) => SG4_LOWER_RIGHT,
                };
                let color = if code & block != 0 { on } else { off };
                paint_px(out, &mut x, xscale, color);
            }
        } else {
            let (cell_fg, cell_bg, glyph) = resolve_alpha_cell(generator, ff22, code, fg, bg);
            let bits = glyph.get(glyph_row).copied().unwrap_or(0);
            for cx in 0..CELL_W {
                let color = if bits & (0x80 >> cx) != 0 { cell_fg } else { cell_bg };
                paint_px(out, &mut x, xscale, color);
            }
        }
    }
}

/// Paint one scan line of a legacy VDG graphics (PMODE) row into `out` (an
/// active-area pixel span). `row_data` is the current RAM row's bytes;
/// each logical pixel is duplicated `xscale` times (`xscale` already folds
/// the mode's own doubling into the canvas width).
pub fn paint_legacy_graphics_line(
    row_data: &[u8],
    mode: &VdgGraphicsMode,
    colors: &[[u8; 4]],
    xscale: usize,
    out: &mut [u8],
) {
    let pixels_per_byte = 8 / mode.bpp;
    let mask = (1u8 << mode.bpp) - 1;
    let mut x = 0;
    for bx in 0..mode.bytes_per_row {
        let byte = row_data.get(bx).copied().unwrap_or(0);
        for j in 0..pixels_per_byte {
            // Pixels are packed MSB-first within the byte.
            let shift = 8 - mode.bpp * (j + 1);
            let value = ((byte >> shift) & mask) as usize;
            let color = colors[value.min(colors.len() - 1)];
            paint_px(out, &mut x, xscale, color);
        }
    }
}

/// Write one native pixel as `xscale` canvas pixels at `*x`, advancing it.
fn paint_px(out: &mut [u8], x: &mut usize, xscale: usize, color: [u8; 4]) {
    for px in out[*x * BYTES_PER_PIXEL..][..xscale * BYTES_PER_PIXEL].chunks_exact_mut(BYTES_PER_PIXEL)
    {
        px.copy_from_slice(&color);
    }
    *x += xscale;
}

/// Render the text screen (`SCREEN_LEN` bytes) into `fb` (`FB_W*FB_H*4` bytes).
///
/// `palette` is the resolved 16-entry GIME palette (RGBA). Each byte is either an
/// alphanumeric character (bit 7 = 0: low 6 bits pick the glyph, bit 6 = inverse,
/// coloured from palette regs 12/13) or a semigraphics-4 block (bit 7 = 1). The
/// stock BASIC screen stores alphanumerics inverse (bit 6 set), so the prompt is
/// black-on-green; the blinking cursor is an SG4 cell that cycles colours.
///
/// `generator`/`ff22` select the font and (on lowercase-capable generators)
/// true-lowercase decode — see [`resolve_alpha_cell`] and [`AlphaGenerator`].
/// `ff22` should be PIA1 $FF22's current value; only [`VDG_GM0_INTEXT`] is
/// consulted here.
pub fn render_text(
    screen: &[u8],
    palette: &[[u8; 4]; PALETTE_LEN],
    border: [u8; 4],
    generator: AlphaGenerator,
    ff22: u8,
    fb: &mut [u8],
) {
    debug_assert!(fb.len() >= FB_W * FB_H * BYTES_PER_PIXEL);

    // Border fills everything first; active cells overwrite the interior.
    for px in fb.chunks_exact_mut(BYTES_PER_PIXEL) {
        px.copy_from_slice(&border);
    }

    let fg = palette[TEXT_FG_INDEX];
    let bg = palette[TEXT_BG_INDEX];

    for row in 0..ROWS {
        for col in 0..COLS {
            let code = screen.get(row * COLS + col).copied().unwrap_or(0);
            if code & SEMIGRAPHICS_BIT != 0 {
                blit_semigraphics4(fb, row, col, code, palette);
            } else {
                let (cell_fg, cell_bg, glyph) = resolve_alpha_cell(generator, ff22, code, fg, bg);
                blit_cell(fb, row, col, glyph, cell_fg, cell_bg);
            }
        }
    }
}

/// Render one semigraphics-4 cell: a 2×2 grid of blocks in the selected colour.
fn blit_semigraphics4(fb: &mut [u8], row: usize, col: usize, code: u8, palette: &[[u8; 4]; PALETTE_LEN]) {
    let on = palette[((code >> SG4_COLOR_SHIFT) & SG4_COLOR_MASK) as usize];
    let off = palette[SG4_OFF_INDEX];
    let x0 = BORDER + col * CELL_W;
    let y0 = BORDER + row * CELL_H;
    for cy in 0..CELL_H {
        let bottom = cy >= CELL_H / 2;
        for cx in 0..CELL_W {
            let right = cx >= CELL_W / 2;
            let block = match (bottom, right) {
                (false, false) => SG4_UPPER_LEFT,
                (false, true) => SG4_UPPER_RIGHT,
                (true, false) => SG4_LOWER_LEFT,
                (true, true) => SG4_LOWER_RIGHT,
            };
            let color = if code & block != 0 { on } else { off };
            let idx = ((y0 + cy) * FB_W + (x0 + cx)) * BYTES_PER_PIXEL;
            fb[idx..idx + BYTES_PER_PIXEL].copy_from_slice(&color);
        }
    }
}

fn blit_cell(fb: &mut [u8], row: usize, col: usize, glyph: &[u8; CELL_H], fg: [u8; 4], bg: [u8; 4]) {
    let x0 = BORDER + col * CELL_W;
    let y0 = BORDER + row * CELL_H;
    for (cy, &bits) in glyph.iter().enumerate() {
        for cx in 0..CELL_W {
            let on = bits & (0x80 >> cx) != 0;
            let color = if on { fg } else { bg };
            let idx = ((y0 + cy) * FB_W + (x0 + cx)) * BYTES_PER_PIXEL;
            fb[idx..idx + BYTES_PER_PIXEL].copy_from_slice(&color);
        }
    }
}

// --- VDG resolution graphics (CoCo-compatible PMODE, `DESIGN.md` §6) -------------
//
// All VDG graphics modes scan out into the same 256×192 active area as text, so
// lower-resolution modes are pixel-doubled to fill it. The horizontal decode (bytes
// per row, bits per pixel, colour set) comes from PIA1 $FF22 (A/G, GM2–0, CSS); the
// display base from the SAM page register; and the actual colours from the GIME
// palette (SEB Fig 13). The *vertical* cadence (how many RAM rows are fetched, and
// how many times each is repeated to fill the 192-line active area) instead comes
// from the SAM V0–V2 bits — see [`LEGACY_GFX_LINES_PER_ROW`]. Real hardware doesn't
// reconcile the two: if a program sets V and GM to a non-standard pairing, the
// vertical cadence follows V and the horizontal decode follows GM independently.

/// PIA1 $FF22 bit 7: 1 = VDG graphics, 0 = alphanumeric/semigraphics.
pub const VDG_AG: u8 = 0x80;
/// PIA1 $FF22 bit 3: colour-set select (picks which GIME palette registers apply).
pub const VDG_CSS: u8 = 0x08;

/// PIA1 $FF22 bit 4: on the plain MC6847 this is INTEXT (external ROM
/// character generator select, not modeled — always the internal generator
/// here); on the MC6847T1 the SAME physical pin is wired as GM0, which in
/// alpha mode enables true lowercase (`coco12_m.cpp` `pia1_pb_changed`: both
/// `intext_w` and `gm0_w` are driven from `data & 0x10`). Named for its T1
/// meaning since that's the only one with an observable effect here.
pub const VDG_GM0_INTEXT: u8 = 0x10;
// NOTE: PIA1 $FF22 bit 5 (GM1) drives MAME's `is_inverse2` on the T1 (a
// second, lowercase-independent colour inversion in alpha mode). That knob
// is a known real-hardware behaviour not modeled here.
/// PIA1 $FF22 bits 6–4: VDG graphics-mode select (GM2–GM0).
const VDG_GM_MASK: u8 = 0x70;
const VDG_GM_SHIFT: u8 = 4;

/// Palette-register indices for 2-colour modes, indexed by CSS (SEB Fig 13):
/// CSS=0 → regs 8,9; CSS=1 → regs 10,11.
const G2_PALETTE_INDICES: [[usize; 2]; 2] = [[8, 9], [10, 11]];
/// Palette-register indices for 4-colour modes, indexed by CSS (SEB Fig 13):
/// CSS=0 → regs 0–3; CSS=1 → regs 4–7.
const G4_PALETTE_INDICES: [[usize; 4]; 2] = [[0, 1, 2, 3], [4, 5, 6, 7]];

/// Upper bound on palette entries a VDG graphics mode can resolve to (the
/// 4-colour modes use all four; 2-colour modes use a `[..2]` prefix).
pub const MAX_VDG_COLORS: usize = 4;

/// Lines-per-row for CoCo-compatible legacy graphics, indexed by the SAM V bits
/// packed `V2:V1:V0` (0–7). Hardware-verified (MAME
/// `gime_legacy_lines_per_row_graphic`): RAM rows fetched = `ACTIVE_H /
/// LEGACY_GFX_LINES_PER_ROW[v]` (64, 96, or 192), each repeated this many times
/// vertically to fill the 192-line active area.
pub const LEGACY_GFX_LINES_PER_ROW: [usize; 8] = [3, 3, 3, 2, 2, 1, 1, 1];

/// A decoded VDG resolution-graphics mode.
pub struct VdgGraphicsMode {
    /// Bytes fetched per displayed row.
    pub bytes_per_row: usize,
    /// RAM rows fetched (before vertical repetition into [`ACTIVE_H`]); driven by
    /// the SAM V bits, not the GM bits (see [`LEGACY_GFX_LINES_PER_ROW`]).
    pub rows: usize,
    /// Bits per pixel: 1 = 2 colours, 2 = 4 colours.
    pub bpp: usize,
    /// Logical pixels across (before horizontal doubling into [`ACTIVE_W`]).
    pub logical_w: usize,
}

/// Mask for the 3-bit SAM V value (`V2:V1:V0`) passed to [`decode_vdg_graphics`].
const SAM_VIDEO_MASK: u8 = 0x07;

/// Decode the VDG graphics mode. The horizontal geometry (bytes per row, bits
/// per pixel, logical width) comes from PIA1 $FF22 GM2–0; the five BASIC PMODEs
/// are RG2/CG3/RG3/CG6/RG6. The vertical geometry (RAM rows fetched) instead
/// comes from `sam_video`, the SAM V0–V2 bits (`V2:V1:V0`, 0–7) — see
/// [`LEGACY_GFX_LINES_PER_ROW`]. Real BASIC always programs matching GM/V pairs,
/// but the two are independent on hardware and this function does not reconcile
/// a mismatched pairing: it just follows each source for its own axis.
pub fn decode_vdg_graphics(ff22: u8, sam_video: u8) -> VdgGraphicsMode {
    let gm = (ff22 & VDG_GM_MASK) >> VDG_GM_SHIFT;
    // (logical width, 4-colour?) for GM2..GM0 = 0..7.
    let (logical_w, four_colour) = match gm {
        0 => (64, true),   // CG1
        1 => (128, false), // RG1
        2 => (128, true),  // CG2
        3 => (128, false), // RG2  (PMODE 0)
        4 => (128, true),  // CG3  (PMODE 1)
        5 => (128, false), // RG3  (PMODE 2)
        6 => (128, true),  // CG6  (PMODE 3)
        _ => (256, false), // RG6  (PMODE 4)
    };
    let bpp = if four_colour { 2 } else { 1 };
    let lines_per_row = LEGACY_GFX_LINES_PER_ROW[(sam_video & SAM_VIDEO_MASK) as usize];
    VdgGraphicsMode {
        bytes_per_row: logical_w * bpp / 8,
        rows: ACTIVE_H / lines_per_row,
        bpp,
        logical_w,
    }
}

/// GIME palette-register indices for a VDG graphics mode, in pixel-value order
/// (SEB Fig 13). `css` is 0 or 1. Borrows a compile-time table — no allocation.
pub fn vdg_palette_indices(bpp: usize, css: usize) -> &'static [usize] {
    if bpp == 1 {
        &G2_PALETTE_INDICES[css]
    } else {
        &G4_PALETTE_INDICES[css]
    }
}

/// Render a VDG graphics field. `data` is the video RAM snapshot
/// (`bytes_per_row * rows` bytes); `colors` is the resolved 2- or 4-entry LUT
/// (pixel value → RGBA). Each logical pixel is scaled to fill the 256×192 active area.
pub fn render_graphics(
    data: &[u8],
    mode: &VdgGraphicsMode,
    colors: &[[u8; 4]],
    border: [u8; 4],
    fb: &mut [u8],
) {
    debug_assert!(fb.len() >= FB_W * FB_H * BYTES_PER_PIXEL);
    for px in fb.chunks_exact_mut(BYTES_PER_PIXEL) {
        px.copy_from_slice(&border);
    }

    let hscale = ACTIVE_W / mode.logical_w;
    let vscale = ACTIVE_H / mode.rows;
    let pixels_per_byte = 8 / mode.bpp;
    let mask = (1u8 << mode.bpp) - 1;

    for ly in 0..mode.rows {
        for bx in 0..mode.bytes_per_row {
            let byte = data.get(ly * mode.bytes_per_row + bx).copied().unwrap_or(0);
            for j in 0..pixels_per_byte {
                // Pixels are packed MSB-first within the byte.
                let shift = 8 - mode.bpp * (j + 1);
                let value = ((byte >> shift) & mask) as usize;
                let color = colors[value.min(colors.len() - 1)];
                let lx = bx * pixels_per_byte + j;
                blit_block(fb, lx * hscale, ly * vscale, hscale, vscale, color);
            }
        }
    }
}

/// Fill an `w`×`h` block of the active area (offset by [`BORDER`]) with one colour.
fn blit_block(fb: &mut [u8], x: usize, y: usize, w: usize, h: usize, color: [u8; 4]) {
    for dy in 0..h {
        for dx in 0..w {
            let idx = ((BORDER + y + dy) * FB_W + (BORDER + x + dx)) * BYTES_PER_PIXEL;
            fb[idx..idx + BYTES_PER_PIXEL].copy_from_slice(&color);
        }
    }
}
