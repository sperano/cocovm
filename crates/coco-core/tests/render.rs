//! Deterministic coverage for the VDG text renderer (`video::render_text`):
//! geometry, alphanumeric glyphs, inverse video, and semigraphics-4 blocks.
//! Colours are supplied via a resolved palette, so tests use sentinel colours.

use coco_core::gime::init0;
use coco_core::video::{
    render_text, AlphaGenerator, BORDER, BYTES_PER_PIXEL, CELL_H, CELL_W, FB_H, FB_W, PALETTE_LEN,
    SCREEN_LEN, TEXT_BG_INDEX, TEXT_FG_INDEX,
};
use coco_core::{Machine, MachineConfig};
use mc6809::Bus;

/// These tests exercise `video::render_text` directly (not through a booted
/// `Machine`), so there's no real PIA1 $FF22 to read. `AlphaGenerator::Mc6847`
/// with GM0 clear reproduces the pre-`AlphaGenerator` behaviour exactly —
/// none of these assertions depend on which generator drew the glyph, only
/// on generic properties (fg/bg swap, border, blank cells).
const NO_GM0: u8 = 0;

const FG: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF]; // red   (palette[13])
const BG: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF]; // green (palette[12])
const BD: [u8; 4] = [0x00, 0x00, 0xFF, 0xFF]; // blue  (border)
const SG_COLOR: [u8; 4] = [0xFF, 0xFF, 0x00, 0xFF]; // yellow (palette[3])
const SG_OFF: [u8; 4] = [0x11, 0x11, 0x11, 0xFF]; // palette[8] (SG4 unlit)

/// A resolved palette with distinct sentinels in the entries the renderer reads.
fn palette() -> [[u8; 4]; PALETTE_LEN] {
    let mut p = [[0u8; 4]; PALETTE_LEN];
    p[TEXT_FG_INDEX] = FG;
    p[TEXT_BG_INDEX] = BG;
    p[3] = SG_COLOR;
    p[8] = SG_OFF;
    p
}

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
    render_text(&[SPACE; SCREEN_LEN], &palette(), BD, AlphaGenerator::Mc6847, NO_GM0, &mut fb);

    assert_eq!(px(&fb, 0, 0), BD);
    assert_eq!(px(&fb, FB_W - 1, FB_H - 1), BD);
    for y in BORDER..BORDER + CELL_H {
        for x in BORDER..BORDER + CELL_W {
            assert_eq!(px(&fb, x, y), BG);
        }
    }
}

#[test]
fn glyph_cell_has_foreground_pixels_blank_cell_does_not() {
    let mut screen = [SPACE; SCREEN_LEN];
    screen[0] = AT;
    let mut fb = fb();
    render_text(&screen, &palette(), BD, AlphaGenerator::Mc6847, NO_GM0, &mut fb);

    let mut fg_pixels = 0;
    for y in BORDER..BORDER + CELL_H {
        for x in BORDER..BORDER + CELL_W {
            if px(&fb, x, y) == FG {
                fg_pixels += 1;
            }
        }
    }
    assert!(fg_pixels > 0, "'@' cell rendered no foreground pixels");

    for y in BORDER..BORDER + CELL_H {
        for x in BORDER + CELL_W..BORDER + 2 * CELL_W {
            assert_eq!(px(&fb, x, y), BG, "blank cell should be all background");
        }
    }
}

#[test]
fn inverse_video_swaps_fg_and_bg() {
    const AT_INVERSE: u8 = AT | 0x40;
    let mut screen = [SPACE; SCREEN_LEN];
    screen[0] = AT_INVERSE;
    let mut fb = fb();
    render_text(&screen, &palette(), BD, AlphaGenerator::Mc6847, NO_GM0, &mut fb);

    let mut counts = (0, 0);
    for y in BORDER..BORDER + CELL_H {
        for x in BORDER..BORDER + CELL_W {
            match px(&fb, x, y) {
                p if p == FG => counts.0 += 1,
                p if p == BG => counts.1 += 1,
                _ => {}
            }
        }
    }
    assert!(counts.1 > 0, "inverse glyph strokes should use the background colour");
    assert!(counts.0 > counts.1, "inverse cell should be mostly foreground-filled");
}

#[test]
fn text_renderer_follows_sam_page_register() {
    // The CoCo-compatible text base is the SAM F0-F6 page register, not a fixed
    // $0400: move the screen to $0600 (page 3) and check the renderer follows.
    const MOVED_BASE: u16 = 0x0600;
    const SPACE_FILL_START: u16 = 0x0400;
    const SPACE_FILL_END: u16 = 0x0800;
    /// White in GIME 6-bit RGBrgb.
    const WHITE6: u8 = 0x3F;

    let mut m = Machine::new(
        MachineConfig::default(),
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    // CoCo-compat mode, VDG alphanumerics; park the CPU on a BRA * so the
    // zeroed synthetic ROM never executes anything with side effects.
    m.bus.gime.write_init0(init0::COCO);
    m.bus.write(0x0000, 0x20); // BRA
    m.bus.write(0x0001, 0xFE); // -2
    m.bus.gime.palette[TEXT_FG_INDEX] = WHITE6;

    // SAM page 3 = $0600: set F0+F1 (odd strobe addresses set the bit).
    m.bus.write(0xFFC7, 0);
    m.bus.write(0xFFC9, 0);

    // Spaces over both candidate bases, then one '@' (VDG code $00, lots of
    // strokes) only on the MOVED screen. Any white pixel therefore proves the
    // renderer read $0600; the old hardcoded $0400 screen is all spaces.
    for addr in SPACE_FILL_START..SPACE_FILL_END {
        m.bus.write(addr, SPACE);
    }
    m.bus.write(MOVED_BASE + 33, AT);
    m.run_field();

    let white = GIME_WHITE_RGBA;
    let mut white_pixels = 0;
    let (w, h) = (m.fb_width as usize, m.fb_height as usize);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * BYTES_PER_PIXEL;
            if m.framebuffer[i..i + 4] == white {
                white_pixels += 1;
            }
        }
    }
    assert!(
        white_pixels > 0,
        "'@' on the SAM-selected screen never rendered: text base is not \
         following the SAM page register"
    );
}

/// `GIME::rgb_color(0x3F)` — all channels 0b11 × 0x55.
const GIME_WHITE_RGBA: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];

#[test]
fn semigraphics4_renders_2x2_color_blocks() {
    // bit7=1 (SG4), colour = palette reg 3, pattern = upper-left + lower-right lit.
    const SG4: u8 = 0x80 | (3 << 4) | 0b1001; // upper-left (0x08) + lower-right (0x01)
    let mut screen = [SPACE; SCREEN_LEN];
    screen[0] = SG4;
    let mut fb = fb();
    render_text(&screen, &palette(), BD, AlphaGenerator::Mc6847, NO_GM0, &mut fb);

    let quad_x = CELL_W / 2;
    let quad_y = CELL_H / 2;
    // Upper-left quadrant: lit → SG_COLOR.
    assert_eq!(px(&fb, BORDER, BORDER), SG_COLOR);
    assert_eq!(px(&fb, BORDER + quad_x - 1, BORDER + quad_y - 1), SG_COLOR);
    // Upper-right quadrant: unlit → SG_OFF.
    assert_eq!(px(&fb, BORDER + quad_x, BORDER), SG_OFF);
    // Lower-left quadrant: unlit → SG_OFF.
    assert_eq!(px(&fb, BORDER, BORDER + quad_y), SG_OFF);
    // Lower-right quadrant: lit → SG_COLOR.
    assert_eq!(px(&fb, BORDER + quad_x, BORDER + quad_y), SG_COLOR);
}
