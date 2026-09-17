//! CoCo 3 CoCo-compatible text: the GIME's own generator, not a VDG font.
//!
//! A real CoCo 3 has no VDG chip at all: when INIT0 COCO is set, the GIME
//! draws CoCo-compatible text with its own font ROM
//! (`crate::font_gime::GIME_LOWRES_FONT`, `video::AlphaGenerator::Gime`), not
//! either MC6847 font — even though `MachineConfig::vdg` is forced to
//! `Mc6847` for a CoCo 3 by `MachineConfig::validate` (it does not apply).
//! Expected glyph bit patterns are copied from `src/font_gime.rs`'s
//! `GIME_LOWRES_FONT`: 'O' at index 15, true-lowercase 'a' at index 64+1=65.

use coco_core::gime::init0;
use coco_core::video::{CELL_H, TEXT_FG_INDEX, VDG_GM0_INTEXT};
use coco_core::{Machine, MachineConfig};
use mc6809::Bus;

use super::common::{CODE_A, CODE_O, PLAIN_O_GLYPH, T1_O_GLYPH, glyph_bits, sample_cell};

/// `GIME_LOWRES_FONT[15]` ('O').
const GIME_O_GLYPH: [u8; CELL_H] = [
    0x00, 0x38, 0x44, 0x44, 0x44, 0x44, 0x44, 0x38, 0x00, 0x00, 0x00, 0x00,
];
/// `GIME_LOWRES_FONT[64 + 1]` (true-lowercase 'a').
const GIME_LOWER_A_GLYPH: [u8; CELL_H] = [
    0x00, 0x00, 0x00, 0x38, 0x04, 0x3C, 0x44, 0x3C, 0x00, 0x00, 0x00, 0x00,
];

/// GIME 6-bit RGB register value for white (all three 2-bit channels `0b11`
/// — see `tests/render.rs`'s `GIME_WHITE_RGBA` doc comment).
const GIME_WHITE6: u8 = 0x3F;
/// `GIME::rgb_color(GIME_WHITE6)`.
const GIME_WHITE_RGBA: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
/// `GIME::rgb_color(0)` — the untouched-palette-register default.
const GIME_BLACK_RGBA: [u8; 4] = [0x00, 0x00, 0x00, 0xFF];

/// Text/graphics base for the following CoCo 3 tests: SAM F0+F1 set moves the
/// display base to $0600 (page 3), keeping it clear of the parked `BRA *` at
/// $0000 — same trick as `tests/render.rs`'s
/// `text_renderer_follows_sam_page_register`.
const COCO3_SCREEN_BASE: u16 = 0x0600;

/// A CoCo 3 (default `MachineConfig`) parked on `BRA *`, forced into
/// CoCo-compatible text mode (INIT0 COCO bit), with the display base moved
/// to [`COCO3_SCREEN_BASE`] and a known white-on-black text palette (the
/// GIME's palette registers are otherwise zeroed = black-on-black, since
/// nothing here runs the ROM's cold-start palette init).
fn boot_parked_coco3() -> Machine {
    let rom = vec![0u8; 32 * 1024].into_boxed_slice();
    let mut m = Machine::new(MachineConfig::default(), rom);
    m.bus.gime.write_init0(init0::COCO);
    m.bus.write(0x0000, 0x20); // BRA
    m.bus.write(0x0001, 0xFE); // -2
    m.bus.gime.palette[TEXT_FG_INDEX] = GIME_WHITE6;
    // palette[TEXT_BG_INDEX] left at its zeroed default: black.
    m.bus.write(0xFFC7, 0); // SAM F0 set
    m.bus.write(0xFFC9, 0); // SAM F1 set -> display base $0600 (page 3)
    m
}

#[test]
fn coco3_compat_text_draws_gime_font_not_either_vdg_font() {
    let mut m = boot_parked_coco3();
    m.bus.pia1.b.output = 0; // text mode, CSS=0, GM0 clear
    m.bus.write(COCO3_SCREEN_BASE, CODE_O);
    m.run_field();

    let cell = sample_cell(&m.framebuffer, 0, 0, GIME_WHITE_RGBA, GIME_BLACK_RGBA);

    assert_eq!(
        cell,
        glyph_bits(&GIME_O_GLYPH),
        "CoCo 3 compat text must draw the GIME's own 'O' glyph (GIME_LOWRES_FONT)"
    );
    assert_ne!(
        cell,
        glyph_bits(&PLAIN_O_GLYPH),
        "must not draw the plain MC6847's 'O' — a CoCo 3 has no MC6847 at all"
    );
    assert_ne!(
        cell,
        glyph_bits(&T1_O_GLYPH),
        "must not draw the MC6847T1's 'O' — a CoCo 3 has no MC6847T1 at all"
    );
}

#[test]
fn coco3_compat_text_true_lowercase_uses_gime_lowercase_font() {
    let mut m = boot_parked_coco3();
    m.bus.pia1.b.output = VDG_GM0_INTEXT; // text mode, CSS=0, GM0 set
    m.bus.write(COCO3_SCREEN_BASE, CODE_A); // code=1, INV clear
    m.run_field();

    // True lowercase swaps fg/bg relative to the normal non-inverse mapping:
    // "on" pixels draw in the background colour (black), "off" pixels in
    // the foreground colour (white) — see `video.rs::resolve_alpha_cell`.
    let cell = sample_cell(&m.framebuffer, 0, 0, GIME_BLACK_RGBA, GIME_WHITE_RGBA);
    assert_eq!(
        cell,
        glyph_bits(&GIME_LOWER_A_GLYPH),
        "GM0 set + INV clear + code<$20 must draw the GIME's true-lowercase 'a' glyph, colour-swapped"
    );
}
