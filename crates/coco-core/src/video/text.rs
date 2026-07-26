//! CoCo-compatible alphanumeric/semigraphics-4 text rendering: the
//! MC6847/MC6847T1/GIME character-generator decode, the legacy border rule,
//! and the whole-field/per-scanline text painters.

use crate::font6847::{MC6847T1_FONT, MC6847_FONT};
use crate::font_gime::GIME_LOWRES_FONT;

use super::{
    paint_px, BORDER, BYTES_PER_PIXEL, CELL_H, CELL_W, COLS, FB_H, FB_W, PALETTE_LEN, ROWS,
    TEXT_BG_INDEX, TEXT_FG_INDEX, VDG_AG, VDG_CSS, VDG_GM0_INTEXT,
};

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
/// [`super::vdg_graphics_border_index`]/[`super::TEXT_BORDER_INDEX`].)
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
