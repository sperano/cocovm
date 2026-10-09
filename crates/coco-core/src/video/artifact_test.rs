use super::*;

const BLACK: [u8; BYTES_PER_PIXEL] = [0, 0, 0, u8::MAX];
const WHITE: [u8; BYTES_PER_PIXEL] = [u8::MAX; BYTES_PER_PIXEL];
const NATIVE_OUTPUT_BYTES: usize = RG6_PIXELS_PER_LINE * BYTES_PER_PIXEL;
const MAME_CSS0_BASE_COLORS: [[u8; BYTES_PER_PIXEL]; BASE_COLOR_COUNT] =
    [[0x26, 0x30, 0x16, u8::MAX], [0x30, 0xd2, 0x00, u8::MAX]];
const MAME_CSS0_STANDARD_COLORS: [[u8; BYTES_PER_PIXEL]; ARTIFACT_COLOR_COUNT] = [
    [0x26, 0x30, 0x16, 0xff],
    [0x26, 0x49, 0x16, 0xff],
    [0x28, 0x30, 0x13, 0xff],
    [0x2e, 0xd2, 0x04, 0xff],
    [0x30, 0xb5, 0x00, 0xff],
    [0x26, 0x50, 0x0c, 0xff],
    [0x2d, 0x56, 0x13, 0xff],
    [0x29, 0xaf, 0x00, 0xff],
    [0x30, 0x89, 0x0d, 0xff],
    [0x30, 0x81, 0x16, 0xff],
    [0x26, 0x81, 0x00, 0xff],
    [0x2a, 0xc9, 0x00, 0xff],
    [0x30, 0xc9, 0x05, 0xff],
    [0x26, 0x30, 0x11, 0xff],
    [0x28, 0x30, 0x16, 0xff],
    [0x30, 0xd2, 0x00, 0xff],
];

fn decode(
    row: &[u8; RG6_BYTES_PER_LINE],
    edge_pixel: bool,
    phase: RG6ArtifactPhase,
    xscale: usize,
) -> Vec<u8> {
    let decoder = RG6ArtifactDecoder::new([BLACK, WHITE], phase);
    let mut output = vec![0; NATIVE_OUTPUT_BYTES * xscale];
    decoder.decode_scanline(
        row,
        RG6ArtifactEdges::solid(edge_pixel),
        xscale,
        &mut output,
    );
    output
}

fn pixel(output: &[u8], x: usize) -> [u8; BYTES_PER_PIXEL] {
    output[x * BYTES_PER_PIXEL..][..BYTES_PER_PIXEL]
        .try_into()
        .expect("RGBA pixel")
}

#[test]
fn standard_palette_matches_mame_fixed_rg6_colors() {
    let decoder = RG6ArtifactDecoder::new(MAME_CSS0_BASE_COLORS, RG6ArtifactPhase::Standard);

    assert_eq!(decoder.colors, MAME_CSS0_STANDARD_COLORS);
}

#[test]
fn alternating_columns_produce_complementary_colors() {
    let starts_with_zero = [0x55; RG6_BYTES_PER_LINE];
    let starts_with_one = [0xaa; RG6_BYTES_PER_LINE];
    let standard_zero = decode(&starts_with_zero, false, RG6ArtifactPhase::Standard, 1);
    let standard_one = decode(&starts_with_one, true, RG6ArtifactPhase::Standard, 1);

    assert_eq!(
        pixel(&standard_zero, RG6_PIXELS_PER_LINE / 2),
        [0, 128, 255, 255]
    );
    assert_eq!(
        pixel(&standard_one, RG6_PIXELS_PER_LINE / 2),
        [255, 128, 0, 255]
    );
}

#[test]
fn reverse_phase_swaps_alternating_pattern_colors() {
    let starts_with_zero = [0x55; RG6_BYTES_PER_LINE];
    let starts_with_one = [0xaa; RG6_BYTES_PER_LINE];
    let standard_zero = decode(&starts_with_zero, false, RG6ArtifactPhase::Standard, 1);
    let standard_one = decode(&starts_with_one, true, RG6ArtifactPhase::Standard, 1);
    let reverse_zero = decode(&starts_with_zero, false, RG6ArtifactPhase::Reverse, 1);
    let reverse_one = decode(&starts_with_one, true, RG6ArtifactPhase::Reverse, 1);
    let middle = RG6_PIXELS_PER_LINE / 2;

    assert_eq!(pixel(&standard_zero, middle), pixel(&reverse_one, middle));
    assert_eq!(pixel(&standard_one, middle), pixel(&reverse_zero, middle));
}

#[test]
fn solid_scanlines_preserve_base_colors() {
    let zero = decode(
        &[0x00; RG6_BYTES_PER_LINE],
        false,
        RG6ArtifactPhase::Standard,
        1,
    );
    let one = decode(
        &[0xff; RG6_BYTES_PER_LINE],
        true,
        RG6ArtifactPhase::Standard,
        1,
    );

    assert!(
        zero.as_chunks::<BYTES_PER_PIXEL>()
            .0
            .iter()
            .all(|color| *color == BLACK)
    );
    assert!(
        one.as_chunks::<BYTES_PER_PIXEL>()
            .0
            .iter()
            .all(|color| *color == WHITE)
    );
}

#[test]
fn scanline_edges_use_halo_pixels() {
    let row = [0x00; RG6_BYTES_PER_LINE];
    let decoder = RG6ArtifactDecoder::new([BLACK, WHITE], RG6ArtifactPhase::Reverse);
    let output = decode(&row, true, RG6ArtifactPhase::Reverse, 1);

    assert_eq!(pixel(&output, 0), decoder.colors[2]);
    assert_eq!(pixel(&output, 1), decoder.colors[13]);
    assert_eq!(pixel(&output, RG6_PIXELS_PER_LINE - 2), BLACK);
    assert_eq!(pixel(&output, RG6_PIXELS_PER_LINE - 1), decoder.colors[2]);
}

#[test]
fn scaled_output_duplicates_each_decoded_pixel() {
    let mut row = [0x00; RG6_BYTES_PER_LINE];
    for (index, byte) in row.iter_mut().enumerate() {
        *byte = index as u8 ^ 0xa5;
    }
    let native = decode(&row, true, RG6ArtifactPhase::Standard, 1);
    let scaled = decode(&row, true, RG6ArtifactPhase::Standard, 2);

    for x in 0..RG6_PIXELS_PER_LINE {
        assert_eq!(pixel(&scaled, x * 2), pixel(&native, x));
        assert_eq!(pixel(&scaled, x * 2 + 1), pixel(&native, x));
    }
}
