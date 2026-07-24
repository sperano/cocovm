//! The per-scanline contract (Option B, `docs/plan-per-scanline-video.md`):
//! register writes mid-field take effect on the next scanline — raster
//! splits — while the field-latched group ($FF9D/$FF9E video base) stays
//! immune until the next field, per MAME `gime.cpp` `new_frame` (memory
//! `gime-scanline-verified-facts`).
//!
//! The machine runs a zero-filled ROM (reset vector → $0000, harmless NEG
//! loops) so scanlines advance deterministically with no ROM code touching
//! the video registers.

use coco_core::gime::{GIME, vmode};
use coco_core::raster::{CANVAS_H, CANVAS_W};
use coco_core::video::BYTES_PER_PIXEL;
use coco_core::{Machine, MachineConfig};

/// $FF9D/$FF9E value → physical $8000.
const VOFF: u16 = 0x1000;
const BASE: usize = (VOFF as usize) << 3;

/// The scanline the tests poke registers at: inside the 192-line body
/// (rows 25..217 for LPF=%00).
const SPLIT_LINE: u32 = 100;

fn px(fb: &[u8], x: usize, y: usize) -> [u8; 4] {
    let i = (y * CANVAS_W + x) * BYTES_PER_PIXEL;
    fb[i..i + 4].try_into().unwrap()
}

/// A CoCo 3 machine in GIME-native 320×192×16 graphics with an identity
/// palette, running zero-ROM filler code.
fn gime_graphics_machine() -> Machine {
    let rom = vec![0u8; 32 * 1024].into_boxed_slice();
    let mut m = Machine::new(MachineConfig::default(), rom);
    let g = &mut m.bus.gime;
    g.init0 = 0; // COCO=0: GIME-native video
    g.vmode = vmode::BP;
    g.vres = 0x1E; // HRES=%111 (160 bytes), CRES=%10 → 320px, 16 colours
    g.vertical_offset = VOFF;
    for (i, reg) in g.palette.iter_mut().enumerate() {
        *reg = i as u8;
    }
    m
}

/// Run to the end of the current field (the wrap back to line 0).
fn finish_field(m: &mut Machine) {
    while !m.step_instruction().field_complete {}
}

/// Run until the machine is at the start of `line` within the current field.
fn run_to_line(m: &mut Machine, line: u32) {
    while m.scanline() != line {
        m.step_instruction();
    }
}

#[test]
fn border_write_mid_field_splits_the_border_at_the_line() {
    let mut m = gime_graphics_machine();
    m.bus.gime.border = 0x09;
    finish_field(&mut m); // first full field with the old border

    run_to_line(&mut m, SPLIT_LINE);
    m.bus.gime.border = 0x2A;
    finish_field(&mut m);

    let fb = &m.framebuffer;
    assert_eq!(
        px(fb, 0, 0),
        GIME::rgb_color(0x09),
        "top border painted before the write keeps the old colour"
    );
    assert_eq!(
        px(fb, 0, CANVAS_H - 1),
        GIME::rgb_color(0x2A),
        "bottom border painted after the write has the new colour"
    );
}

#[test]
fn palette_write_mid_field_recolors_only_lines_below_it() {
    let mut m = gime_graphics_machine();
    // Zeroed video RAM → every body pixel reads palette register 0.
    m.bus.gime.palette[0] = 0x01;
    finish_field(&mut m);

    run_to_line(&mut m, SPLIT_LINE);
    m.bus.gime.palette[0] = 0x02;
    finish_field(&mut m);

    let fb = &m.framebuffer;
    let split = SPLIT_LINE as usize;
    assert_eq!(
        px(fb, 0, split - 1),
        GIME::rgb_color(0x01),
        "body above the split keeps the old palette"
    );
    assert_eq!(
        px(fb, 0, split),
        GIME::rgb_color(0x02),
        "the split line onward has the new palette"
    );
    // Both orders: the same field shows both colours at once.
    assert_eq!(px(fb, 0, 30), GIME::rgb_color(0x01));
    assert_eq!(px(fb, 0, 200), GIME::rgb_color(0x02));
}

#[test]
fn video_base_write_mid_field_waits_for_the_next_field() {
    let mut m = gime_graphics_machine();
    // Marker bytes for a body row BELOW the split (row 130 = body row 105),
    // distinct at the two candidate bases: only a row painted after the poke
    // can tell whether the base was re-latched mid-field.
    const MARKER_ROW: usize = 105; // body row → canvas row 25 + 105 = 130
    let other_voff = 0x1100u16;
    let other_base = (other_voff as usize) << 3;
    m.bus.ram[BASE + MARKER_ROW * 160] = 0x50; // palette 5 at the latched base
    m.bus.ram[other_base + MARKER_ROW * 160] = 0x70; // palette 7 at the new base
    finish_field(&mut m);

    run_to_line(&mut m, SPLIT_LINE);
    m.bus.gime.vertical_offset = other_voff;
    finish_field(&mut m);
    let marker_canvas_row = 25 + MARKER_ROW;
    assert_eq!(
        px(&m.framebuffer, 0, marker_canvas_row),
        GIME::rgb_color(5),
        "mid-field base write must NOT retarget this field (MAME new_frame)"
    );

    finish_field(&mut m);
    assert_eq!(
        px(&m.framebuffer, 0, marker_canvas_row),
        GIME::rgb_color(7),
        "the next field latches the new base"
    );
}

#[test]
fn mode_switch_mid_field_splits_text_and_graphics() {
    let mut m = gime_graphics_machine();
    finish_field(&mut m);

    run_to_line(&mut m, SPLIT_LINE);
    // Switch to 80-column text with attributes mid-field. The row pointer has
    // already advanced through the graphics rows, so fill a wide swath of
    // char/attr pairs (' ' on attr bg palette 2) wherever the fetch lands.
    m.bus.gime.vmode = 0x03; // BP=0, LPR=%011 (8-line rows)
    m.bus.gime.vres = 0x15;
    for i in (BASE..BASE + 0x8000).step_by(2) {
        m.bus.ram[i] = b' ';
        m.bus.ram[i + 1] = 0x02;
    }
    finish_field(&mut m);

    let fb = &m.framebuffer;
    assert_eq!(
        px(fb, 0, 30),
        GIME::rgb_color(0),
        "graphics decode above the split (zeroed RAM → palette 0)"
    );
    assert_eq!(
        px(fb, 0, SPLIT_LINE as usize + 1),
        GIME::rgb_color(2),
        "text decode below the split"
    );
}
