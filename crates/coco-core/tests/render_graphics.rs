//! Deterministic coverage for the VDG graphics renderer (`video::render_graphics`):
//! mode decoding, MSB-first pixel unpacking, colour-set selection, and the
//! pixel-doubling that scales lower resolutions into the 256×192 active area.

use coco_core::video::{
    BORDER, BYTES_PER_PIXEL, FB_H, FB_W, decode_vdg_graphics, render_graphics, vdg_palette_indices,
};

const C0: [u8; 4] = [0x10, 0x10, 0x10, 0xFF];
const C1: [u8; 4] = [0x20, 0x20, 0x20, 0xFF];
const C2: [u8; 4] = [0x30, 0x30, 0x30, 0xFF];
const C3: [u8; 4] = [0x40, 0x40, 0x40, 0xFF];
const BD: [u8; 4] = [0x00, 0x00, 0xFF, 0xFF]; // border sentinel

// $FF22 with A/G set (graphics) and the given GM2–0 in bits 6–4.
const AG: u8 = 0x80;
const RG6: u8 = AG | (7 << 4); // PMODE 4: 256×192, 2 colour
const CG6: u8 = AG | (6 << 4); // PMODE 3: 128×192, 4 colour
const RG3: u8 = AG | (5 << 4); // PMODE 2: 128×192, 2 colour

fn fb() -> Vec<u8> {
    vec![0u8; FB_W * FB_H * BYTES_PER_PIXEL]
}

fn px(fb: &[u8], x: usize, y: usize) -> [u8; 4] {
    let i = (y * FB_W + x) * BYTES_PER_PIXEL;
    fb[i..i + 4].try_into().unwrap()
}

#[test]
fn decodes_pmode_geometry() {
    let m = decode_vdg_graphics(RG6);
    assert_eq!(
        (m.bytes_per_row, m.rows, m.bpp, m.logical_w),
        (32, 192, 1, 256)
    );

    let m = decode_vdg_graphics(CG6);
    assert_eq!(
        (m.bytes_per_row, m.rows, m.bpp, m.logical_w),
        (32, 192, 2, 128)
    );

    let m = decode_vdg_graphics(RG3);
    assert_eq!(
        (m.bytes_per_row, m.rows, m.bpp, m.logical_w),
        (16, 192, 1, 128)
    );
}

#[test]
fn palette_indices_follow_css_and_depth() {
    assert_eq!(vdg_palette_indices(1, 0), [8, 9].as_slice());
    assert_eq!(vdg_palette_indices(1, 1), [10, 11].as_slice());
    assert_eq!(vdg_palette_indices(2, 0), [0, 1, 2, 3].as_slice());
    assert_eq!(vdg_palette_indices(2, 1), [4, 5, 6, 7].as_slice());
}

#[test]
fn two_color_unpacks_msb_first_with_border() {
    let mode = decode_vdg_graphics(RG6); // 1:1, no scaling
    let mut data = vec![0u8; mode.bytes_per_row * mode.rows];
    data[0] = 0b1000_0000; // only the leftmost pixel is colour 1
    let mut fb = fb();
    render_graphics(&data, &mode, &[C0, C1], BD, &mut fb);

    assert_eq!(px(&fb, 0, 0), BD, "corner is border");
    assert_eq!(px(&fb, BORDER, BORDER), C1, "MSB pixel = colour 1");
    assert_eq!(px(&fb, BORDER + 1, BORDER), C0, "next pixel = colour 0");
}

#[test]
fn four_color_maps_two_bit_values_and_doubles_width() {
    let mode = decode_vdg_graphics(CG6); // 128 wide → hscale 2
    let mut data = vec![0u8; mode.bytes_per_row * mode.rows];
    data[0] = 0b00_01_10_11; // pixel values 0,1,2,3 left→right
    let mut fb = fb();
    render_graphics(&data, &mode, &[C0, C1, C2, C3], BD, &mut fb);

    // Each logical pixel is 2 host pixels wide.
    assert_eq!(px(&fb, BORDER, BORDER), C0);
    assert_eq!(px(&fb, BORDER + 1, BORDER), C0, "pixel 0 doubled");
    assert_eq!(px(&fb, BORDER + 2, BORDER), C1);
    assert_eq!(px(&fb, BORDER + 4, BORDER), C2);
    assert_eq!(px(&fb, BORDER + 6, BORDER), C3);
}

#[test]
fn logical_row_repetition_follows_the_vdg_mode() {
    let mode = decode_vdg_graphics(AG); // CG1: 64 logical rows
    let mut data = vec![0u8; mode.bytes_per_row * mode.rows];
    for row in 0..mode.rows {
        data[row * mode.bytes_per_row] = 0x80 >> (row % 8);
    }
    let mut fb = fb();
    render_graphics(&data, &mode, &[C0, C1, C2, C3], BD, &mut fb);

    let row_pixels = |fb: &[u8], y: usize| -> Vec<[u8; 4]> {
        (0..8).map(|x| px(fb, BORDER + x, BORDER + y)).collect()
    };

    for row in 0..mode.rows {
        let first_line = row * 3;
        for repeated_line in first_line + 1..first_line + 3 {
            assert_eq!(row_pixels(&fb, first_line), row_pixels(&fb, repeated_line));
        }
    }
    // Distinct fetched rows must produce distinct pixels. This rules out a
    // stub that leaves everything at its default color.
    assert_ne!(row_pixels(&fb, 0), row_pixels(&fb, 3));
}
