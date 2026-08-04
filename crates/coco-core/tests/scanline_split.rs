//! The per-scanline contract (Option B):
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

/// The plan's flagship acceptance test, driven entirely by EMULATED code:
/// a hand-assembled test ROM arms the GIME interval timer (TINS=0: one tick
/// per hsync) for [`SPLIT_LINE`] lines and switches the border colour from
/// its FIRQ handler — asserting the whole interrupt-to-video path splits
/// the canonical raster at the right line, with no harness register pokes.
#[test]
fn timer_firq_from_rom_code_splits_the_border() {
    const OLD_BORDER: u8 = 0x09;
    const NEW_BORDER: u8 = 0x2A;
    /// FIRQ handler location in the ROM image ($8000 + offset).
    const ISR: u16 = 0x8040;

    let mut rom = vec![0u8; 32 * 1024];
    let program: &[u8] = &[
        0x10,
        0xCE,
        0x1F,
        0xF0, // LDS  #$1FF0      stack in low RAM
        0x86,
        OLD_BORDER, // LDA  #OLD_BORDER
        0xB7,
        0xFF,
        0x9A, // STA  $FF9A       border = old colour
        0x86,
        0x20, // LDA  #intr::TMR
        0xB7,
        0xFF,
        0x93, // STA  $FF93       FIRQENR: timer source
        0x7F,
        0xFF,
        0x91, // CLR  $FF91       INIT1: TINS=0 (hsync rate)
        0x7F,
        0xFF,
        0x94, // CLR  $FF94       timer MSB = 0
        0x86,
        SPLIT_LINE as u8, // LDA  #SPLIT_LINE
        0xB7,
        0xFF,
        0x95, // STA  $FF95       timer LSB (restarts count)
        0x86,
        0x10, // LDA  #init0::FEN
        0xB7,
        0xFF,
        0x90, // STA  $FF90       INIT0: FIRQ out, COCO=0
        0x1C,
        0xAF, // ANDCC #$AF       unmask FIRQ/IRQ
        0x20,
        0xFE, // BRA  *           wait for the timer
    ];
    rom[..program.len()].copy_from_slice(program);
    let isr: &[u8] = &[
        0xB6, 0xFF, 0x93, // LDA  $FF93   read status (clears the latch)
        0x86, NEW_BORDER, // LDA  #NEW_BORDER
        0xB7, 0xFF, 0x9A, // STA  $FF9A   border = new colour
        0x7F, 0xFF, 0x93, // CLR  $FF93   no further timer FIRQs
        0x3B, // RTI
    ];
    let isr_off = (ISR - 0x8000) as usize;
    rom[isr_off..isr_off + isr.len()].copy_from_slice(isr);
    // Vectors (hardwired-internal $FFE0+ region): FIRQ → ISR, RESET → $8000.
    rom[0x7FF6..0x7FF8].copy_from_slice(&ISR.to_be_bytes());
    rom[0x7FFE..0x8000].copy_from_slice(&0x8000u16.to_be_bytes());

    let mut m = Machine::new(MachineConfig::default(), rom.into_boxed_slice());
    finish_field(&mut m);

    // Column 0 is border at every visible row (GIME-native default mode is
    // non-wide). Exactly one old→new transition, at the timer's line — the
    // count starts when the LSB write lands (line 0, a few instructions in)
    // and runs SPLIT_LINE+2 hsync ticks (the 1986 reload offset), with the
    // FIRQ handler's border write landing within the following line.
    let old = GIME::rgb_color(OLD_BORDER);
    let new = GIME::rgb_color(NEW_BORDER);
    let column: Vec<[u8; 4]> = (0..CANVAS_H).map(|y| px(&m.framebuffer, 0, y)).collect();
    assert_eq!(column[0], old, "field starts on the old border");
    assert_eq!(column[CANVAS_H - 1], new, "field ends on the new border");
    let transitions: Vec<usize> = (1..CANVAS_H)
        .filter(|&y| column[y] != column[y - 1])
        .collect();
    assert_eq!(
        transitions.len(),
        1,
        "exactly one border split, got {transitions:?}"
    );
    let split_row = transitions[0];
    let expected = SPLIT_LINE as usize;
    assert!(
        (expected..=expected + 4).contains(&split_row),
        "split at row {split_row}, expected within {expected}..={}",
        expected + 4
    );
}
