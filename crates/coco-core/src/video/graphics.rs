//! VDG resolution graphics (CoCo-compatible PMODE, `DESIGN.md` §6).
//!
//! All VDG graphics modes scan out into the same 256×192 active area as text, so
//! lower-resolution modes are pixel-doubled to fill it. PIA1 $FF22 GM selects the
//! MC6847's 16/32 sample requests, 64/96/192-line cadence, and pixel decode. On a
//! CoCo 1/2, the discrete MC6883 sees only those requests' DA0 transitions and
//! applies its independently selected SAM V divider/carry rules. Software normally
//! programs a stock GM/V pairing, but deliberate mismatches produce a non-linear
//! address stream. The CoCo 3's GIME compatibility path uses its own row model.

use super::{ACTIVE_H, ACTIVE_W, BORDER, BYTES_PER_PIXEL, FB_H, FB_W, paint_px};

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

/// Lines-per-row for CoCo 3 GIME-compatible legacy graphics, indexed by its SAM
/// V compatibility bits packed `V2:V1:V0` (0–7). Hardware-verified (MAME
/// `gime_legacy_lines_per_row_graphic`): RAM rows fetched = `ACTIVE_H /
/// LEGACY_GFX_LINES_PER_ROW[v]` (64, 96, or 192), each repeated this many times
/// vertically to fill the 192-line active area. The discrete CoCo 1/2 MC6883
/// instead uses [`crate::sam::SAMVideoAddressStream`].
pub const LEGACY_GFX_LINES_PER_ROW: [usize; 8] = [3, 3, 3, 2, 2, 1, 1, 1];

/// A decoded VDG resolution-graphics mode.
pub struct VDGGraphicsMode {
    /// Bytes fetched per displayed row.
    pub bytes_per_row: usize,
    /// MC6847 logical rows (before vertical repetition into [`ACTIVE_H`]).
    pub rows: usize,
    /// Bits per pixel: 1 = 2 colours, 2 = 4 colours.
    pub bpp: usize,
    /// Logical pixels across (before horizontal doubling into [`ACTIVE_W`]).
    pub logical_w: usize,
}

/// Decode the MC6847 graphics mode selected by PIA1 $FF22 GM2-0.
///
/// All four geometry/decode fields belong to the VDG. The discrete MC6883
/// transforms its sample requests into physical addresses separately.
pub fn decode_vdg_graphics(ff22: u8) -> VDGGraphicsMode {
    let gm = (ff22 & VDG_GM_MASK) >> VDG_GM_SHIFT;
    // (logical width, logical rows, 4-colour?) for GM2..GM0 = 0..7.
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
    VDGGraphicsMode {
        bytes_per_row: logical_w * bpp / 8,
        rows,
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

/// Paints one scan line of a legacy VDG graphics (PMODE) row into `out`.
/// `row_data` is the current RAM row's bytes; each logical pixel is
/// duplicated `xscale` times (already folding the mode's own doubling).
pub fn paint_legacy_graphics_line(
    row_data: &[u8],
    mode: &VDGGraphicsMode,
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

/// Renders a VDG graphics field. `data` is the video RAM snapshot
/// (`bytes_per_row * rows` bytes); `colors` is the resolved 2- or 4-entry
/// LUT, scaled to fill the 256×192 active area.
pub fn render_graphics(
    data: &[u8],
    mode: &VDGGraphicsMode,
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

/// Render the 192 physical scanlines sampled through the discrete MC6883.
/// `data` contains `bytes_per_row` MC6847 samples for every active scanline;
/// repeated or discontinuous SAM addresses have already been resolved.
pub fn render_sampled_graphics(
    data: &[u8],
    mode: &VDGGraphicsMode,
    colors: &[[u8; 4]],
    border: [u8; 4],
    fb: &mut [u8],
) {
    debug_assert!(fb.len() >= FB_W * FB_H * BYTES_PER_PIXEL);
    for px in fb.chunks_exact_mut(BYTES_PER_PIXEL) {
        px.copy_from_slice(&border);
    }

    let xscale = ACTIVE_W / mode.logical_w;
    for y in 0..ACTIVE_H {
        let data_start = y * mode.bytes_per_row;
        let output_start = ((BORDER + y) * FB_W + BORDER) * BYTES_PER_PIXEL;
        paint_legacy_graphics_line(
            data.get(data_start..data_start + mode.bytes_per_row)
                .unwrap_or_default(),
            mode,
            colors,
            xscale,
            &mut fb[output_start..output_start + ACTIVE_W * BYTES_PER_PIXEL],
        );
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
