//! Deterministic coverage for the GIME-native renderers (`gime_video`):
//! register decode, physical-RAM scanout from the vertical offset registers,
//! attribute colours / blink / underline in text, and 1/2/4-bpp unpacking in
//! graphics. Register values follow SEB Unravelled II and MAME `gime.cpp`.

use coco_core::gime::{GIME, vmode};
use coco_core::gime_video::{
    BORDER_X_DIVISOR, BORDER_Y, CHAR_W, decode_graphics, decode_text, render_graphics, render_text,
};
use coco_core::video::BYTES_PER_PIXEL;

/// 128K of physical RAM, like the base machine.
const RAM_LEN: usize = 0x20000;

/// Physical video base used by the tests, expressed as the $FF9D/$FF9E value.
const VOFF: u16 = 0x1000; // physical $8000 (VOFF × 8)
const BASE: usize = (VOFF as usize) << 3;

/// $FF98 for hi-res text: BP=0, LPR=%011 → 8 lines per character row.
const TEXT_LPR8: u8 = 0x03;
/// $FF99 for 80-column text with attributes (what WIDTH 80 programs): HRES=%101,
/// CRES bit 0 = attributes.
const VRES_TEXT80_ATTR: u8 = 0x15;
/// $FF99 for 40-column text without attributes: HRES=%001.
const VRES_TEXT40: u8 = 0x04;
/// $FF98 for graphics: BP=1, 1 line per row.
const GFX: u8 = vmode::BP;
/// $FF99 for HSCREEN 2 (320×192, 16 colours): HRES=%111 (160 bytes), CRES=%10.
const VRES_320X16: u8 = 0x1E;
/// $FF99 for 640×192×2: HRES=%101 (80 bytes), CRES=%00.
const VRES_640X2: u8 = 0x14;

fn gime_with(vmode_val: u8, vres_val: u8) -> GIME {
    let mut g = GIME::new();
    g.vmode = vmode_val;
    g.vres = vres_val;
    g.vertical_offset = VOFF;
    // Identity-ish palette: register i holds colour value i (0–15 fit in 6 bits).
    for (i, reg) in g.palette.iter_mut().enumerate() {
        *reg = i as u8;
    }
    g
}

fn px(fb: &[u8], fb_w: usize, x: usize, y: usize) -> [u8; 4] {
    let i = (y * fb_w + x) * BYTES_PER_PIXEL;
    fb[i..i + 4].try_into().unwrap()
}

#[test]
fn decodes_text_modes() {
    let g = gime_with(TEXT_LPR8, VRES_TEXT80_ATTR);
    let m = decode_text(&g);
    assert_eq!(
        (m.cols, m.attributes, m.lines, m.lines_per_row),
        (80, true, 192, 8)
    );

    let g = gime_with(TEXT_LPR8, VRES_TEXT40);
    let m = decode_text(&g);
    assert_eq!((m.cols, m.attributes), (40, false));

    // HRES bit 1 is ignored in text modes: %010 is still 32 columns.
    let g = gime_with(TEXT_LPR8, 0x08);
    assert_eq!(decode_text(&g).cols, 32);
}

#[test]
fn decodes_graphics_modes() {
    let g = gime_with(GFX, VRES_320X16);
    let m = decode_graphics(&g);
    assert_eq!(
        (m.bytes_per_row, m.bpp, m.width, m.lines),
        (160, 4, 320, 192)
    );

    let g = gime_with(GFX, VRES_640X2);
    let m = decode_graphics(&g);
    assert_eq!((m.bytes_per_row, m.bpp, m.width), (80, 1, 640));

    // LPF=%01 → 200 lines.
    let g = gime_with(GFX, VRES_640X2 | 0x20);
    assert_eq!(decode_graphics(&g).lines, 200);
}

#[test]
fn text_without_attributes_uses_palette_0_and_1() {
    let g = gime_with(TEXT_LPR8, VRES_TEXT40);
    let mut ram = vec![0u8; RAM_LEN];
    ram[BASE] = b'A';

    let mut fb = Vec::new();
    let (fb_w, fb_h) = render_text(&g, &ram, false, &mut fb);
    let active_w = 40 * CHAR_W;
    assert_eq!(fb_w, active_w + 2 * (active_w / BORDER_X_DIVISOR));
    assert_eq!(fb_h, 192 + 2 * BORDER_Y);

    let bg = GIME::rgb_color(0); // palette reg 0
    let fg = GIME::rgb_color(1); // palette reg 1
    let x0 = active_w / BORDER_X_DIVISOR;
    // 'A' row 0 is 0x10: pixel 3 lit, pixel 0 dark.
    assert_eq!(px(&fb, fb_w, x0 + 3, BORDER_Y), fg);
    assert_eq!(px(&fb, fb_w, x0, BORDER_Y), bg);
    // Border corner carries the $FF9A colour (0 here → black).
    assert_eq!(px(&fb, fb_w, 0, 0), GIME::rgb_color(0));
}

#[test]
fn text_attributes_select_fg_bg_palettes() {
    let g = gime_with(TEXT_LPR8, VRES_TEXT80_ATTR);
    let mut ram = vec![0u8; RAM_LEN];
    ram[BASE] = b' '; // all-background cell
    ram[BASE + 1] = (5 << 3) | 2; // fg = palette 8+5, bg = palette 2

    let mut fb = Vec::new();
    let (fb_w, _) = render_text(&g, &ram, false, &mut fb);
    let x0 = (80 * CHAR_W) / BORDER_X_DIVISOR;
    assert_eq!(
        px(&fb, fb_w, x0, BORDER_Y),
        GIME::rgb_color(2),
        "background from regs 0-7"
    );

    ram[BASE] = b'A';
    let (fb_w, _) = render_text(&g, &ram, false, &mut fb);
    assert_eq!(
        px(&fb, fb_w, x0 + 3, BORDER_Y),
        GIME::rgb_color(8 + 5),
        "foreground from regs 8-15"
    );
}

#[test]
fn blink_attribute_blanks_character_during_blink_phase() {
    let g = gime_with(TEXT_LPR8, VRES_TEXT80_ATTR);
    let mut ram = vec![0u8; RAM_LEN];
    ram[BASE] = b'A';
    ram[BASE + 1] = 0x80 | (1 << 3); // blink, fg = palette 9, bg = palette 0

    let x0 = (80 * CHAR_W) / BORDER_X_DIVISOR;
    let mut fb = Vec::new();
    let (fb_w, _) = render_text(&g, &ram, false, &mut fb);
    assert_eq!(
        px(&fb, fb_w, x0 + 3, BORDER_Y),
        GIME::rgb_color(9),
        "visible phase"
    );

    let (fb_w, _) = render_text(&g, &ram, true, &mut fb);
    assert_eq!(
        px(&fb, fb_w, x0 + 3, BORDER_Y),
        GIME::rgb_color(0),
        "blanked phase"
    );
}

#[test]
fn underline_lights_bottom_line_of_8_line_rows() {
    let g = gime_with(TEXT_LPR8, VRES_TEXT80_ATTR);
    let mut ram = vec![0u8; RAM_LEN];
    ram[BASE] = b' '; // no glyph pixels — only the underline can light
    ram[BASE + 1] = 0x40 | (3 << 3); // underline, fg = palette 11

    let mut fb = Vec::new();
    let (fb_w, _) = render_text(&g, &ram, false, &mut fb);
    let x0 = (80 * CHAR_W) / BORDER_X_DIVISOR;
    assert_eq!(
        px(&fb, fb_w, x0, BORDER_Y + 7),
        GIME::rgb_color(11),
        "line 7 underlined"
    );
    assert_eq!(
        px(&fb, fb_w, x0, BORDER_Y + 6),
        GIME::rgb_color(0),
        "line 6 untouched"
    );
}

#[test]
fn second_text_row_starts_after_row_pitch() {
    let g = gime_with(TEXT_LPR8, VRES_TEXT80_ATTR);
    let mut ram = vec![0u8; RAM_LEN];
    // Row 1 (bytes 160..) first cell: 'A' with fg palette 9.
    ram[BASE + 160] = b'A';
    ram[BASE + 160 + 1] = 1 << 3;

    let mut fb = Vec::new();
    let (fb_w, _) = render_text(&g, &ram, false, &mut fb);
    let x0 = (80 * CHAR_W) / BORDER_X_DIVISOR;
    // 'A' row 1 is 0x28: pixel 2 lit, 8 lines into the active area.
    assert_eq!(px(&fb, fb_w, x0 + 2, BORDER_Y + 8 + 1), GIME::rgb_color(9));
}

#[test]
fn graphics_unpacks_4bpp_msb_first_from_physical_base() {
    let g = gime_with(GFX, VRES_320X16);
    let mut ram = vec![0u8; RAM_LEN];
    ram[BASE] = 0x5C; // pixels 5, 12
    ram[BASE + 160] = 0x70; // row 1 first pixel = 7

    let mut fb = Vec::new();
    let (fb_w, fb_h) = render_graphics(&g, &ram, &mut fb);
    assert_eq!(fb_w, 320 + 2 * (320 / BORDER_X_DIVISOR));
    assert_eq!(fb_h, 192 + 2 * BORDER_Y);

    let x0 = 320 / BORDER_X_DIVISOR;
    assert_eq!(px(&fb, fb_w, x0, BORDER_Y), GIME::rgb_color(5));
    assert_eq!(px(&fb, fb_w, x0 + 1, BORDER_Y), GIME::rgb_color(12));
    assert_eq!(
        px(&fb, fb_w, x0, BORDER_Y + 1),
        GIME::rgb_color(7),
        "row pitch = 160 bytes"
    );
}

#[test]
fn graphics_border_uses_ff9a_color() {
    let mut g = gime_with(GFX, VRES_320X16);
    g.border = 0x12;
    let ram = vec![0u8; RAM_LEN];
    let mut fb = Vec::new();
    let (fb_w, _) = render_graphics(&g, &ram, &mut fb);
    assert_eq!(px(&fb, fb_w, 0, 0), GIME::rgb_color(0x12));
}

#[test]
fn horizontal_offset_shifts_fetch_with_seam_wrap() {
    // HVEN on: rows are 256 bytes wide; X offset 1 shifts the window 2 bytes.
    let mut g = gime_with(GFX, VRES_320X16);
    g.horizontal_offset = 0x80 | 0x01;
    let mut ram = vec![0u8; RAM_LEN];
    ram[BASE + 2] = 0xF0; // lands at pixel 0 with the 2-byte shift
    ram[BASE + 256 + 2] = 0x90; // row 1: HVEN pitch 256, same 2-byte shift

    let mut fb = Vec::new();
    let (fb_w, _) = render_graphics(&g, &ram, &mut fb);
    let x0 = 320 / BORDER_X_DIVISOR;
    assert_eq!(px(&fb, fb_w, x0, BORDER_Y), GIME::rgb_color(15));
    assert_eq!(
        px(&fb, fb_w, x0, BORDER_Y + 1),
        GIME::rgb_color(9),
        "row 1 window shifted too"
    );
}

#[test]
fn vertical_scroll_starts_field_mid_character_row() {
    let mut g = gime_with(TEXT_LPR8, VRES_TEXT80_ATTR);
    g.vertical_scroll = 2;
    let mut ram = vec![0u8; RAM_LEN];
    ram[BASE] = b'A';
    ram[BASE + 1] = 1 << 3; // fg = palette 9

    let mut fb = Vec::new();
    let (fb_w, _) = render_text(&g, &ram, false, &mut fb);
    let x0 = (80 * CHAR_W) / BORDER_X_DIVISOR;
    // With VSC=2 the first displayed line is glyph row 2 of 'A' (0x44): pixel 1 lit.
    assert_eq!(px(&fb, fb_w, x0 + 1, BORDER_Y), GIME::rgb_color(9));
}
