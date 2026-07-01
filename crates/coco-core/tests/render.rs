//! Deterministic coverage for the VDG text renderer (`video::render_text`):
//! geometry (border vs. active area) and glyph vs. blank cells. Colours are passed
//! in, so tests use distinct sentinel colours to check placement.

use coco_core::video::{
    render_text, BORDER, BYTES_PER_PIXEL, CELL_H, CELL_W, FB_H, FB_W, SCREEN_LEN,
};

const FG: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF]; // red
const BG: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF]; // green
const BD: [u8; 4] = [0x00, 0x00, 0xFF, 0xFF]; // blue

fn fb() -> Vec<u8> {
    vec![0u8; FB_W * FB_H * BYTES_PER_PIXEL]
}

fn px(fb: &[u8], x: usize, y: usize) -> [u8; 4] {
    let i = (y * FB_W + x) * BYTES_PER_PIXEL;
    fb[i..i + 4].try_into().unwrap()
}

/// VDG code for a blank cell (space).
const SPACE: u8 = 0x20;
/// VDG code for '@' (glyph $00) — a character with many set pixels.
const AT: u8 = 0x00;

#[test]
fn border_and_active_area_use_their_colors() {
    let mut fb = fb();
    render_text(&[SPACE; SCREEN_LEN], FG, BG, BD, &mut fb);

    // The outer border is the border colour.
    assert_eq!(px(&fb, 0, 0), BD);
    assert_eq!(px(&fb, FB_W - 1, FB_H - 1), BD);
    // A fully-blank screen: the whole active interior is the background colour.
    for y in BORDER..BORDER + CELL_H {
        for x in BORDER..BORDER + CELL_W {
            assert_eq!(px(&fb, x, y), BG);
        }
    }
}

#[test]
fn glyph_cell_has_foreground_pixels_blank_cell_does_not() {
    let mut screen = [SPACE; SCREEN_LEN];
    screen[0] = AT; // cell (0,0) = '@', cell (0,1) stays blank
    let mut fb = fb();
    render_text(&screen, FG, BG, BD, &mut fb);

    // Cell (0,0) must contain at least one foreground pixel.
    let mut fg_pixels = 0;
    for y in BORDER..BORDER + CELL_H {
        for x in BORDER..BORDER + CELL_W {
            if px(&fb, x, y) == FG {
                fg_pixels += 1;
            }
        }
    }
    assert!(fg_pixels > 0, "'@' cell rendered no foreground pixels");

    // Cell (0,1) is a space — entirely background, no foreground.
    for y in BORDER..BORDER + CELL_H {
        for x in BORDER + CELL_W..BORDER + 2 * CELL_W {
            assert_eq!(px(&fb, x, y), BG, "blank cell should be all background");
        }
    }
}

#[test]
fn inverse_video_swaps_fg_and_bg() {
    // Bit 6 set = inverse: the glyph strokes take the background colour and the
    // cell fills with the foreground colour. This is why the CoCo prompt (all
    // bytes have bit 6 set) is black-on-green rather than green-on-black.
    const AT_INVERSE: u8 = AT | 0x40;
    let mut screen = [SPACE; SCREEN_LEN];
    screen[0] = AT_INVERSE;
    let mut fb = fb();
    render_text(&screen, FG, BG, BD, &mut fb);

    let mut fg_bgcount = (0, 0);
    for y in BORDER..BORDER + CELL_H {
        for x in BORDER..BORDER + CELL_W {
            match px(&fb, x, y) {
                p if p == FG => fg_bgcount.0 += 1,
                p if p == BG => fg_bgcount.1 += 1,
                _ => {}
            }
        }
    }
    // Inverse '@': strokes (a few pixels) are BG, the rest of the cell is FG.
    assert!(fg_bgcount.1 > 0, "inverse glyph strokes should use the background colour");
    assert!(
        fg_bgcount.0 > fg_bgcount.1,
        "inverse cell should be mostly foreground-filled"
    );
}
