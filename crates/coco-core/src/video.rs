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

/// Number of resolved palette entries (GIME palette registers).
pub const PALETTE_LEN: usize = 16;

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

/// Render the text screen (`SCREEN_LEN` bytes) into `fb` (`FB_W*FB_H*4` bytes).
///
/// `palette` is the resolved 16-entry GIME palette (RGBA). Each byte is either an
/// alphanumeric character (bit 7 = 0: low 6 bits pick the glyph, bit 6 = inverse,
/// coloured from palette regs 12/13) or a semigraphics-4 block (bit 7 = 1). The
/// stock BASIC screen stores alphanumerics inverse (bit 6 set), so the prompt is
/// black-on-green; the blinking cursor is an SG4 cell that cycles colours.
pub fn render_text(screen: &[u8], palette: &[[u8; 4]; PALETTE_LEN], border: [u8; 4], fb: &mut [u8]) {
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
                let glyph = &MC6847_FONT[(code & GLYPH_CODE_MASK) as usize % GLYPH_COUNT];
                let (cell_fg, cell_bg) = if code & INVERSE_BIT != 0 { (bg, fg) } else { (fg, bg) };
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
// lower-resolution modes are pixel-doubled to fill it. The mode, colour set, and
// colour depth come from PIA1 $FF22 (A/G, GM2–0, CSS); the display base from the SAM
// page register; and the actual colours from the GIME palette (SEB Fig 13).

/// PIA1 $FF22 bit 7: 1 = VDG graphics, 0 = alphanumeric/semigraphics.
pub const VDG_AG: u8 = 0x80;
/// PIA1 $FF22 bit 3: colour-set select (picks which GIME palette registers apply).
pub const VDG_CSS: u8 = 0x08;
/// PIA1 $FF22 bits 6–4: VDG graphics-mode select (GM2–GM0).
const VDG_GM_MASK: u8 = 0x70;
const VDG_GM_SHIFT: u8 = 4;

/// First GIME palette register for 2-colour modes, indexed by CSS (SEB Fig 13):
/// CSS=0 → regs 8,9; CSS=1 → regs 10,11.
const G2_PALETTE_BASE: [usize; 2] = [8, 10];
/// First GIME palette register for 4-colour modes, indexed by CSS (SEB Fig 13):
/// CSS=0 → regs 0–3; CSS=1 → regs 4–7.
const G4_PALETTE_BASE: [usize; 2] = [0, 4];

/// A decoded VDG resolution-graphics mode.
pub struct VdgGraphicsMode {
    /// Bytes fetched per displayed row.
    pub bytes_per_row: usize,
    /// Displayed rows (before vertical doubling into [`ACTIVE_H`]).
    pub rows: usize,
    /// Bits per pixel: 1 = 2 colours, 2 = 4 colours.
    pub bpp: usize,
    /// Logical pixels across (before horizontal doubling into [`ACTIVE_W`]).
    pub logical_w: usize,
}

/// Decode the VDG graphics mode from PIA1 $FF22. GM2–0 select one of the eight
/// resolution-graphics modes; the five BASIC PMODEs are RG2/CG3/RG3/CG6/RG6.
pub fn decode_vdg_graphics(ff22: u8) -> VdgGraphicsMode {
    let gm = (ff22 & VDG_GM_MASK) >> VDG_GM_SHIFT;
    // (logical width, rows, 4-colour?) for GM2..GM0 = 0..7.
    let (logical_w, rows, four_colour) = match gm {
        0 => (64, 64, true),    // CG1
        1 => (128, 64, false),  // RG1
        2 => (128, 64, true),   // CG2
        3 => (128, 96, false),  // RG2  (PMODE 0)
        4 => (128, 96, true),   // CG3  (PMODE 1)
        5 => (128, 192, false), // RG3  (PMODE 2)
        6 => (128, 192, true),  // CG6  (PMODE 3)
        _ => (256, 192, false), // RG6  (PMODE 4)
    };
    let bpp = if four_colour { 2 } else { 1 };
    VdgGraphicsMode { bytes_per_row: logical_w * bpp / 8, rows, bpp, logical_w }
}

/// GIME palette-register indices for a VDG graphics mode, in pixel-value order
/// (SEB Fig 13). `css` is 0 or 1.
pub fn vdg_palette_indices(bpp: usize, css: usize) -> Vec<usize> {
    if bpp == 1 {
        let b = G2_PALETTE_BASE[css];
        vec![b, b + 1]
    } else {
        let b = G4_PALETTE_BASE[css];
        vec![b, b + 1, b + 2, b + 3]
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
