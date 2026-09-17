//! VDG-compatible 32×16 text scanout (`DESIGN.md` §6).
//!
//! Renders the CoCo-compatible alphanumeric text screen onto the canonical
//! 640×240 canvas (`raster`), each VDG dot doubled into the 512 px non-wide
//! span — the same geometry the CoCo 3's legacy modes paint — using the
//! authentic MC6847 character generator (`font6847`). Each screen byte holds
//! the VDG internal glyph code in its low 6 bits (`code & 0x3F`).
//!
//! Colours are data-driven from the GIME palette registers the ROM programmed, not
//! hardcoded: CoCo-compatible text takes its background from palette reg
//! [`TEXT_BG_INDEX`] and its foreground from [`TEXT_FG_INDEX`] (the MC6847 text
//! `color_base_0`/`color_base_1`), and the legacy text border is black. At the
//! stock BASIC prompt that resolves to pure green (`#00FF00`) on black. The CSS
//! orange colour set is TODO (`§6`); GIME native text/graphics live in `gime_video`.

mod artifact;
mod graphics;
mod text;

use crate::raster;

pub use artifact::{
    RG6_BYTES_PER_LINE, RG6_PIXELS_PER_LINE, RG6ArtifactDecoder, RG6ArtifactEdges, RG6ArtifactPhase,
};
pub use graphics::{
    LEGACY_GFX_LINES_PER_ROW, MAX_VDG_COLORS, VDG_AG, VDG_CSS, VDG_GM0_INTEXT, VDGGraphicsMode,
    decode_vdg_graphics, paint_legacy_graphics_line, render_graphics, render_sampled_graphics,
    vdg_palette_indices,
};
pub use text::{
    AlphaGenerator, decode_alpha_char, legacy_border_value, paint_legacy_text_line, render_text,
};

/// VDG character cell: 8 pixels wide × 12 raster lines (matches the font rows).
pub const CELL_W: usize = 8;
pub const CELL_H: usize = 12;

pub const COLS: usize = 32;
pub const ROWS: usize = 16;

/// Active display geometry, in VDG dots.
pub const ACTIVE_W: usize = COLS * CELL_W; // 256
pub const ACTIVE_H: usize = ROWS * CELL_H; // 192

/// Canvas pixels per VDG dot: the 256-dot line fills the canonical raster's
/// 512 px non-wide span, exactly as the GIME's legacy modes scan it.
pub const VDG_XSCALE: usize = raster::NON_WIDE_ACTIVE_W / ACTIVE_W;
/// LPF field value for the 192-line body the VDG always scans.
const LPF_192: usize = 0;
/// Canvas row where the VDG's active body starts: the GIME's LPF=%00 window,
/// so a CoCo 1/2 field lands on the same rows as a CoCo 3 legacy field.
pub const VDG_ACTIVE_TOP: usize = raster::vertical_window(LPF_192).0;

pub const BYTES_PER_PIXEL: usize = 4;

/// Byte range of active scanline `y` (`0..ACTIVE_H`) within a canonical
/// canvas: the 512 px span behind the 64 px side border.
pub fn active_row_range(y: usize) -> std::ops::Range<usize> {
    let start =
        ((VDG_ACTIVE_TOP + y) * raster::CANVAS_W + raster::NON_WIDE_BORDER_X) * BYTES_PER_PIXEL;
    start..start + raster::NON_WIDE_ACTIVE_W * BYTES_PER_PIXEL
}

/// Paints a whole VDG field into a canonical canvas: `border` everywhere,
/// then `paint_row(y, row)` over each active scanline's 512 px span.
fn paint_field(border: [u8; 4], fb: &mut [u8], mut paint_row: impl FnMut(usize, &mut [u8])) {
    debug_assert!(fb.len() >= raster::CANVAS_W * raster::CANVAS_H * BYTES_PER_PIXEL);
    for px in fb.chunks_exact_mut(BYTES_PER_PIXEL) {
        px.copy_from_slice(&border);
    }
    for y in 0..ACTIVE_H {
        paint_row(y, &mut fb[active_row_range(y)]);
    }
}

/// The text screen is 512 bytes (32×16).
pub const SCREEN_LEN: usize = COLS * ROWS;

/// GIME palette registers that colour CoCo-compatible text (MC6847 text
/// `color_base_0`/`color_base_1`): glyph background and glyph foreground.
pub const TEXT_BG_INDEX: usize = 12;
pub const TEXT_FG_INDEX: usize = 13;

/// Number of resolved palette entries (GIME palette registers).
pub const PALETTE_LEN: usize = 16;

// --- CoCo 1/2 fixed VDG colour source -----------
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
/// (12-13 green, 14-15 orange) — RGB values verbatim from MAME
/// `src/devices/video/mc6847.cpp` `s_palette` (BSD-3-Clause, Nathan Woods;
/// see NOTICE.md).
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
/// buff (CSS=1) — real hardware does *not* border graphics modes in black.
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
/// (`VdgFixed` — CoCo 1/2, which has no palette registers to program).
pub enum ColorSource<'a> {
    GIMEPalette(&'a [[u8; 4]; PALETTE_LEN]),
    VDGFixed,
}

impl ColorSource<'_> {
    /// Resolves to a 16-entry RGBA table in the shared index layout. `css`
    /// only affects `VdgFixed` (swaps the orange alphanumeric set into
    /// [`TEXT_BG_INDEX`]/[`TEXT_FG_INDEX`]); `GimePalette` ignores it — the
    /// GIME's own registers already hold whatever the ROM programmed.
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

/// Writes one native pixel as `xscale` canvas pixels at `*x`, advancing it
/// (shared by the legacy text/graphics line painters).
fn paint_px(out: &mut [u8], x: &mut usize, xscale: usize, color: [u8; 4]) {
    for px in
        out[*x * BYTES_PER_PIXEL..][..xscale * BYTES_PER_PIXEL].chunks_exact_mut(BYTES_PER_PIXEL)
    {
        px.copy_from_slice(&color);
    }
    *x += xscale;
}
