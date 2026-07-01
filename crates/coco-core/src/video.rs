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
//! stock BASIC prompt that resolves to pure green (`#00FF00`) on black. Inverse
//! video, the orange colour set, semigraphics, and GIME native text are TODO (`§6`).

use crate::font6847::MC6847_FONT;

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

/// Number of glyphs in the font (VDG codes $00–$3F).
const GLYPH_COUNT: usize = 64;
/// MC6847 alphanumeric attribute bits within a screen byte.
const GLYPH_CODE_MASK: u8 = 0x3F;
const INVERSE_BIT: u8 = 0x40; // bit 6 — inverse video (swaps fg/bg)

/// Render the text screen (`SCREEN_LEN` bytes) into `fb` (`FB_W*FB_H*4` bytes)
/// using the resolved `fg`/`bg`/`border` RGBA colours.
///
/// Each byte's low 6 bits pick the glyph; bit 6 is inverse video. The stock BASIC
/// text screen stores every character inverse (bit 6 set), which is why the
/// prompt is black-on-green rather than green-on-black. (bit 7 = semigraphics is
/// not handled yet.)
pub fn render_text(screen: &[u8], fg: [u8; 4], bg: [u8; 4], border: [u8; 4], fb: &mut [u8]) {
    debug_assert!(fb.len() >= FB_W * FB_H * BYTES_PER_PIXEL);

    // Border fills everything first; active cells overwrite the interior.
    for px in fb.chunks_exact_mut(BYTES_PER_PIXEL) {
        px.copy_from_slice(&border);
    }

    for row in 0..ROWS {
        for col in 0..COLS {
            let code = screen.get(row * COLS + col).copied().unwrap_or(0);
            let glyph = &MC6847_FONT[(code & GLYPH_CODE_MASK) as usize % GLYPH_COUNT];
            let (cell_fg, cell_bg) = if code & INVERSE_BIT != 0 { (bg, fg) } else { (fg, bg) };
            blit_cell(fb, row, col, glyph, cell_fg, cell_bg);
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
