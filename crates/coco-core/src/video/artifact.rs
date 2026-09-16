//! NTSC artifact-color decoding for MC6847 RG6 scanlines.
//!
//! SOURCE / LICENSING: ported from MAME's
//! `src/devices/video/mc6847.cpp` and `mc6847.h` artifacter,
//! BSD-3-Clause, copyright Nathan Woods. See `NOTICE.md`.

use super::BYTES_PER_PIXEL;

/// Logical pixels in one MC6847 RG6 scanline.
pub const RG6_PIXELS_PER_LINE: usize = 256;
/// Packed bytes in one MC6847 RG6 scanline.
pub const RG6_BYTES_PER_LINE: usize = RG6_PIXELS_PER_LINE / u8::BITS as usize;

const ARTIFACT_COLOR_COUNT: usize = 16;
const ARTIFACT_FACTOR_COUNT: usize = ARTIFACT_COLOR_COUNT - 2;
const BASE_COLOR_COUNT: usize = 2;
const RGB_CHANNEL_COUNT: usize = 3;
const ALPHA_CHANNEL: usize = RGB_CHANNEL_COUNT;
const PIXELS_PER_PAIR: usize = 2;
const NEIGHBORHOOD_PIXELS: usize = 6;
const HALO_PIXELS_PER_SIDE: usize = 2;
const LEFT_HALO_PIXELS: isize = HALO_PIXELS_PER_SIDE as isize;
const ROUNDING_BIAS: f64 = 0.5;

/// NTSC color-burst phase used to assign the complementary artifact colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RG6ArtifactPhase {
    Standard,
    Reverse,
}

/// The two logical pixels immediately outside each side of the active scanline.
///
/// MAME decodes the active row after painting its border, so the six-pixel
/// neighborhood can read these four halo pixels. Callers derive each bit by
/// comparing the resolved border color with the logical-one base color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RG6ArtifactEdges {
    pub left: [bool; HALO_PIXELS_PER_SIDE],
    pub right: [bool; HALO_PIXELS_PER_SIDE],
}

impl RG6ArtifactEdges {
    /// Creates matching, solid-color halos on both sides of the scanline.
    pub const fn solid(pixel: bool) -> Self {
        Self {
            left: [pixel; HALO_PIXELS_PER_SIDE],
            right: [pixel; HALO_PIXELS_PER_SIDE],
        }
    }
}

/// Reusable MC6847 RG6 artifact-color decoder.
///
/// Construction resolves MAME's 16-entry artifact palette from the two base
/// colors. [`decode_scanline`](Self::decode_scanline) then applies the shared
/// six-pixel lookup at either native or horizontally scaled output widths.
pub struct RG6ArtifactDecoder {
    colors: [[u8; BYTES_PER_PIXEL]; ARTIFACT_COLOR_COUNT],
}

impl RG6ArtifactDecoder {
    /// Builds the artifact palette for the two resolved RG6 base colors.
    ///
    /// Both base colors must have the same alpha value. MAME blends RGB
    /// channels, and the decoder preserves the shared alpha channel.
    pub fn new(
        base_colors: [[u8; BYTES_PER_PIXEL]; BASE_COLOR_COUNT],
        phase: RG6ArtifactPhase,
    ) -> Self {
        assert_eq!(
            base_colors[0][ALPHA_CHANNEL], base_colors[1][ALPHA_CHANNEL],
            "RG6 base colors must use the same alpha value"
        );
        let mut colors = [[0; BYTES_PER_PIXEL]; ARTIFACT_COLOR_COUNT];
        colors[0] = base_colors[0];
        colors[ARTIFACT_COLOR_COUNT - 1] = base_colors[1];
        for (slot_index, color) in colors[1..ARTIFACT_COLOR_COUNT - 1].iter_mut().enumerate() {
            let factor_index = phase.factor_index(slot_index);
            *color = blend_color(base_colors, ARTIFACT_FACTORS[factor_index]);
        }
        Self { colors }
    }

    /// Decodes one packed, most-significant-bit-first RG6 scanline into RGBA.
    ///
    /// `xscale` is one for a 256-pixel output and two for a 512-pixel output.
    /// `out` must have room for `RG6_PIXELS_PER_LINE * xscale` RGBA pixels.
    pub fn decode_scanline(
        &self,
        row_data: &[u8; RG6_BYTES_PER_LINE],
        edges: RG6ArtifactEdges,
        xscale: usize,
        out: &mut [u8],
    ) {
        assert!(xscale > 0, "RG6 artifact output scale must be nonzero");
        let output_len = RG6_PIXELS_PER_LINE * xscale * BYTES_PER_PIXEL;
        assert!(out.len() >= output_len, "RG6 artifact output is too short");

        for pair_x in (0..RG6_PIXELS_PER_LINE).step_by(PIXELS_PER_PAIR) {
            let key = neighborhood_key(row_data, edges, pair_x);
            let correction_start = usize::from(key) * PIXELS_PER_PAIR;
            for pair_offset in 0..PIXELS_PER_PAIR {
                let color_index = usize::from(ARTIFACT_CORRECTION[correction_start + pair_offset]);
                let output_x = pair_x + pair_offset;
                paint_scaled_pixel(out, output_x, xscale, self.colors[color_index]);
            }
        }
    }
}

impl RG6ArtifactPhase {
    fn factor_index(self, index: usize) -> usize {
        match self {
            Self::Standard => index ^ 1,
            Self::Reverse => index,
        }
    }
}

fn blend_color(
    base_colors: [[u8; BYTES_PER_PIXEL]; BASE_COLOR_COUNT],
    factors: [f64; RGB_CHANNEL_COUNT],
) -> [u8; BYTES_PER_PIXEL] {
    let mut color = [0; BYTES_PER_PIXEL];
    for channel in 0..RGB_CHANNEL_COUNT {
        let factor = factors[channel];
        let low = f64::from(base_colors[0][channel]);
        let high = f64::from(base_colors[1][channel]);
        color[channel] = (low * (1.0 - factor) + high * factor + ROUNDING_BIAS) as u8;
    }
    color[ALPHA_CHANNEL] = base_colors[0][ALPHA_CHANNEL];
    color
}

fn neighborhood_key(
    row_data: &[u8; RG6_BYTES_PER_LINE],
    edges: RG6ArtifactEdges,
    pair_x: usize,
) -> u8 {
    let mut key = 0;
    let first_x = pair_x as isize - LEFT_HALO_PIXELS;
    for offset in 0..NEIGHBORHOOD_PIXELS {
        key <<= 1;
        key |= u8::from(logical_pixel(row_data, edges, first_x + offset as isize));
    }
    key
}

fn logical_pixel(row_data: &[u8; RG6_BYTES_PER_LINE], edges: RG6ArtifactEdges, x: isize) -> bool {
    if x < 0 {
        return edges.left[(x + LEFT_HALO_PIXELS) as usize];
    }
    let x = x as usize;
    if x >= RG6_PIXELS_PER_LINE {
        let edge_x = x - RG6_PIXELS_PER_LINE;
        debug_assert!(edge_x < HALO_PIXELS_PER_SIDE);
        return edges.right[edge_x];
    }
    let byte = row_data[x / u8::BITS as usize];
    let shift = u8::BITS as usize - 1 - x % u8::BITS as usize;
    byte & (1 << shift) != 0
}

fn paint_scaled_pixel(
    out: &mut [u8],
    logical_x: usize,
    xscale: usize,
    color: [u8; BYTES_PER_PIXEL],
) {
    let first_byte = logical_x * xscale * BYTES_PER_PIXEL;
    let last_byte = first_byte + xscale * BYTES_PER_PIXEL;
    for pixel in out[first_byte..last_byte].chunks_exact_mut(BYTES_PER_PIXEL) {
        pixel.copy_from_slice(&color);
    }
}

// MAME `mc6847_base_device::artifacter::update_colors`, paired factors F1-F14.
const ARTIFACT_FACTORS: [[f64; RGB_CHANNEL_COUNT]; ARTIFACT_FACTOR_COUNT] = [
    [0.157, 0.000, 0.157],
    [0.000, 0.157, 0.000],
    [1.000, 0.824, 1.000],
    [0.824, 1.000, 0.824],
    [0.706, 0.236, 0.118],
    [0.000, 0.197, 0.471],
    [1.000, 0.550, 0.393],
    [0.275, 0.785, 1.000],
    [0.000, 0.500, 1.000],
    [1.000, 0.500, 0.000],
    [1.000, 0.942, 0.785],
    [0.393, 0.942, 1.000],
    [0.236, 0.000, 0.000],
    [0.000, 0.000, 0.236],
];

// MAME `mc6847_base_device::artifacter::artifact_correction`, indexed by the
// six-pixel neighborhood followed by even/odd output position.
const ARTIFACT_CORRECTION: [u8; 1 << (NEIGHBORHOOD_PIXELS + 1)] = [
    0, 0, 0, 0, 0, 6, 0, 2, 5, 7, 5, 7, 1, 3, 1, 11, 8, 6, 8, 14, 8, 9, 8, 9, 4, 4, 4, 15, 12, 12,
    12, 15, 5, 13, 5, 13, 13, 0, 13, 2, 10, 10, 10, 10, 10, 15, 10, 11, 3, 1, 3, 1, 15, 9, 15, 9,
    11, 11, 11, 11, 15, 15, 15, 15, 14, 0, 14, 0, 14, 6, 14, 2, 0, 7, 0, 7, 1, 3, 1, 11, 9, 6, 9,
    14, 9, 9, 9, 9, 15, 4, 15, 15, 12, 12, 12, 15, 2, 13, 2, 13, 2, 0, 2, 2, 10, 10, 10, 10, 10,
    15, 10, 11, 12, 1, 12, 1, 12, 9, 12, 9, 15, 11, 15, 11, 15, 15, 15, 15,
];

#[cfg(test)]
#[path = "artifact_test.rs"]
mod tests;
