//! CoCo-compatible alphanumeric/semigraphics text rendering: the
//! MC6847/MC6847T1/GIME character-generator decode, the legacy border rule,
//! and the whole-field/per-scanline text painters.

use crate::font_gime::GIME_LOWRES_FONT;
use crate::font6847::{MC6847_FONT, MC6847T1_FONT};

use super::{
    BORDER, BYTES_PER_PIXEL, CELL_H, CELL_W, COLS, FB_H, FB_W, PALETTE_LEN, ROWS, TEXT_BG_INDEX,
    TEXT_FG_INDEX, VDG_AG, VDG_CSS, VDG_GM0_INTEXT, paint_px,
};

/// Number of glyphs in the font (VDG codes $00–$3F).
const GLYPH_COUNT: usize = 64;
/// MC6847 attribute bits within a screen byte.
const SEMIGRAPHICS_BIT: u8 = 0x80; // bit 7 — 1 = semigraphics, 0 = alphanumeric
const INVERSE_BIT: u8 = 0x40; // bit 6 — inverse video (alphanumeric only)
const GLYPH_CODE_MASK: u8 = 0x3F; // bits 5-0 — alphanumeric glyph code

// Semigraphics 4: bits 6-4 select a colour (GIME palette reg 0–7), bits 3-0 are a
// 2×2 block pattern; unlit blocks use palette reg 8 (black in CoCo-compat mode).
const SG4_COLOR_SHIFT: u8 = 4;
const SG4_COLOR_MASK: u8 = 0x07;
const SG4_PATTERN_MASK: u8 = 0x0F;
const SG4_OFF_INDEX: usize = 8;

// Semigraphics 6: bits 7-6 select one of four colours, bits 5-0 are a 2×3
// block pattern. On the CoCo, screen-byte bit 7 also drives the VDG's A/S
// input, so only colour codes 2 and 3 can select semigraphics. The MC6847
// selects SG6 when INT/EXT is high; MC6847T1 and GIME omit SG6 and decode
// the same byte as SG4.
const SG6_COLOR_SHIFT: u8 = 6;
const SG6_COLOR_MASK: u8 = 0x03;
const SG6_PATTERN_MASK: u8 = 0x3F;
const SG6_COLOR_SET_SIZE: usize = 4;
const SG6_ROW_HEIGHT: usize = CELL_H / 3;
const SG4_ROW_HEIGHT: usize = CELL_H / 2;
const LEFT_BLOCK_PIXELS: u8 = 0xF0;
const RIGHT_BLOCK_PIXELS: u8 = 0x0F;
const PIXEL_MSB: u8 = 0x80;

/// Decodes a VDG alphanumeric screen byte's low 6 bits to the ASCII
/// character it displays (ignoring semigraphics/inverse bits): `$00-$1F` →
/// `@A-Z[\]^_`, `$20-$3F` → a second ASCII block starting at space.
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

fn semigraphics_line(
    generator: AlphaGenerator,
    ff22: u8,
    code: u8,
    glyph_row: usize,
    palette: &[[u8; 4]; PALETTE_LEN],
) -> ([u8; 4], [u8; 4], u8) {
    let sg6 = generator == AlphaGenerator::MC6847 && ff22 & VDG_GM0_INTEXT != 0;
    let (color_index, pattern, row_height) = if sg6 {
        (
            ((code >> SG6_COLOR_SHIFT) & SG6_COLOR_MASK) as usize
                + usize::from(ff22 & VDG_CSS != 0) * SG6_COLOR_SET_SIZE,
            code & SG6_PATTERN_MASK,
            SG6_ROW_HEIGHT,
        )
    } else {
        (
            ((code >> SG4_COLOR_SHIFT) & SG4_COLOR_MASK) as usize,
            code & SG4_PATTERN_MASK,
            SG4_ROW_HEIGHT,
        )
    };
    let glyph_row = glyph_row % CELL_H;
    let slice = (CELL_H - 1 - glyph_row) / row_height;
    let right = pattern & (1 << (slice * 2)) != 0;
    let left = pattern & (1 << (slice * 2 + 1)) != 0;
    let pixels =
        if left { LEFT_BLOCK_PIXELS } else { 0 } | if right { RIGHT_BLOCK_PIXELS } else { 0 };
    (palette[color_index], palette[SG4_OFF_INDEX], pixels)
}

/// Resolves one alphanumeric cell's glyph and (fg, bg) colours, matching
/// MAME `mc6847.cpp`'s `character_map` ctor. True lowercase (font's
/// `0x40 + code` glyphs, fg/bg swapped) applies only on MC6847T1/GIME when
/// non-inverse, GM0 is set, and code < `$20`; everything else draws the
/// normal font range with fg/bg swapped by the inverse bit alone.
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

/// The CoCo 3 legacy-mode border colour as a GIME 6-bit value: graphics
/// modes border green (CSS=0)/white (CSS=1); the GM2-without-GM1 text
/// variant borders green/orange; every other text/semigraphics mode borders black.
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

/// Paints one scan line of the legacy 32-column text screen into `out`,
/// duplicating each pixel `xscale` times. `row_bytes` is the current
/// character row's 32 screen bytes; `glyph_row` the scan line within it.
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
            let (on, off, pixels) = semigraphics_line(generator, ff22, code, glyph_row, palette);
            for cx in 0..CELL_W {
                let color = if pixels & (PIXEL_MSB >> cx) != 0 {
                    on
                } else {
                    off
                };
                paint_px(out, &mut x, xscale, color);
            }
        } else {
            let (cell_fg, cell_bg, glyph) = resolve_alpha_cell(generator, ff22, code, fg, bg);
            let bits = glyph.get(glyph_row).copied().unwrap_or(0);
            for cx in 0..CELL_W {
                let color = if bits & (0x80 >> cx) != 0 {
                    cell_fg
                } else {
                    cell_bg
                };
                paint_px(out, &mut x, xscale, color);
            }
        }
    }
}

/// Renders the text screen (`SCREEN_LEN` bytes) into `fb`. Each byte is an
/// alphanumeric character (bit 7 clear, coloured from palette regs 12/13) or
/// a semigraphics block (bit 7 set); `generator`/`ff22` select SG4 versus
/// SG6, the font, and true-lowercase decode (see [`resolve_alpha_cell`]).
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
                blit_semigraphics(fb, row, col, code, palette, generator, ff22);
            } else {
                let (cell_fg, cell_bg, glyph) = resolve_alpha_cell(generator, ff22, code, fg, bg);
                blit_cell(fb, row, col, glyph, cell_fg, cell_bg);
            }
        }
    }
}

/// Render one semigraphics cell: SG4 is a 2×2 grid and SG6 a 2×3 grid.
fn blit_semigraphics(
    fb: &mut [u8],
    row: usize,
    col: usize,
    code: u8,
    palette: &[[u8; 4]; PALETTE_LEN],
    generator: AlphaGenerator,
    ff22: u8,
) {
    let x0 = BORDER + col * CELL_W;
    let y0 = BORDER + row * CELL_H;
    for cy in 0..CELL_H {
        let (on, off, pixels) = semigraphics_line(generator, ff22, code, cy, palette);
        for cx in 0..CELL_W {
            let color = if pixels & (PIXEL_MSB >> cx) != 0 {
                on
            } else {
                off
            };
            let idx = ((y0 + cy) * FB_W + (x0 + cx)) * BYTES_PER_PIXEL;
            fb[idx..idx + BYTES_PER_PIXEL].copy_from_slice(&color);
        }
    }
}

fn blit_cell(
    fb: &mut [u8],
    row: usize,
    col: usize,
    glyph: &[u8; CELL_H],
    fg: [u8; 4],
    bg: [u8; 4],
) {
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
