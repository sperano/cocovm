//! MC6847 versus MC6847T1 font and lowercase tests (`crates/coco-core/src/font6847.rs`).

use coco_core::video::{CELL_H, VDG_FIXED_PALETTE, VDG_GM0_INTEXT};
use mc6809::Bus;

use super::common::{
    CODE_A, CODE_O, PLAIN_O_GLYPH, SCREEN_BASE, T1_O_GLYPH, boot_parked_machine_with, coco1_config,
    coco2_t1_config, glyph_bits, sample_cell,
};

/// MC6847 alphanumeric INVERSE_BIT (bit 6).
const INVERSE_BIT: u8 = 0x40;

/// `MC6847T1_FONT[64 + 1]` (true-lowercase 'a').
const T1_LOWER_A_GLYPH: [u8; CELL_H] = [
    0x00, 0x00, 0x00, 0x1C, 0x02, 0x1E, 0x22, 0x1E, 0x00, 0x00, 0x00, 0x00,
];
/// `MC6847_FONT[1]` ('A'): plain chip, rows 3-9.
const PLAIN_A_GLYPH: [u8; CELL_H] = [
    0x00, 0x00, 0x00, 0x08, 0x14, 0x22, 0x22, 0x3E, 0x22, 0x22, 0x00, 0x00,
];
/// `MC6847T1_FONT[1]` ('A'): T1 chip, rows 1-7 (2 rows higher than the plain
/// chip — see `src/font6847.rs`'s module doc on the per-chip row offset).
const T1_A_GLYPH: [u8; CELL_H] = [
    0x00, 0x08, 0x14, 0x22, 0x22, 0x3E, 0x22, 0x22, 0x00, 0x00, 0x00, 0x00,
];

#[test]
fn plain_vdg_draws_square_o_t1_draws_rounded_o() {
    let mut plain = boot_parked_machine_with(coco1_config());
    plain.bus.pia1.b.output = 0; // text mode, CSS=0
    plain.bus.write(SCREEN_BASE, CODE_O);
    plain.run_field();

    let mut t1 = boot_parked_machine_with(coco2_t1_config());
    t1.bus.pia1.b.output = 0; // text mode, CSS=0, GM0 clear (no lowercase; code >= $0F irrelevant anyway)
    t1.bus.write(SCREEN_BASE, CODE_O);
    t1.run_field();

    let fg = VDG_FIXED_PALETTE[13];
    let bg = VDG_FIXED_PALETTE[12];

    let plain_cell = sample_cell(&plain.framebuffer, 0, 0, fg, bg);
    let t1_cell = sample_cell(&t1.framebuffer, 0, 0, fg, bg);

    assert_eq!(
        plain_cell,
        glyph_bits(&PLAIN_O_GLYPH),
        "plain MC6847 must draw the square 'O'"
    );
    assert_eq!(
        t1_cell,
        glyph_bits(&T1_O_GLYPH),
        "MC6847T1 must draw the rounded 'O'"
    );
    assert_ne!(
        plain_cell, t1_cell,
        "plain and T1 'O' glyphs must differ in shape"
    );
}

#[test]
fn t1_true_lowercase_requires_gm0_and_swaps_colors() {
    let mut m = boot_parked_machine_with(coco2_t1_config());
    // Text mode, CSS=0, GM0 ($FF22 bit 4) set: enables true lowercase.
    m.bus.pia1.b.output = VDG_GM0_INTEXT;
    m.bus.write(SCREEN_BASE, CODE_A); // code=1, INV clear
    m.run_field();

    let fg = VDG_FIXED_PALETTE[13]; // ALPHANUMERIC BRIGHT GREEN
    let bg = VDG_FIXED_PALETTE[12]; // ALPHANUMERIC DARK GREEN

    // True lowercase swaps fg/bg relative to the normal non-inverse mapping:
    // "on" pixels draw in `bg`, "off" pixels draw in `fg` (see
    // `video.rs::resolve_alpha_cell`).
    let cell = sample_cell(&m.framebuffer, 0, 0, bg, fg);
    assert_eq!(
        cell,
        glyph_bits(&T1_LOWER_A_GLYPH),
        "GM0 set + INV clear + code<$20 must draw the true-lowercase 'a' glyph, colour-swapped"
    );
}

#[test]
fn t1_without_gm0_code_01_is_not_lowercase() {
    let mut m = boot_parked_machine_with(coco2_t1_config());
    m.bus.pia1.b.output = 0; // GM0 clear
    m.bus.write(SCREEN_BASE, CODE_A); // code=1, INV clear
    m.run_field();

    let fg = VDG_FIXED_PALETTE[13];
    let bg = VDG_FIXED_PALETTE[12];
    // Without GM0, code $01 draws the ordinary uppercase 'A' from
    // `MC6847T1_FONT[1]`, not the lowercase glyph.
    let cell = sample_cell(&m.framebuffer, 0, 0, fg, bg);
    assert_ne!(
        cell,
        glyph_bits(&T1_LOWER_A_GLYPH),
        "code $01 with GM0 clear must not draw the lowercase glyph"
    );
}

#[test]
fn inverse_uppercase_a_is_unaffected_by_t1_lowercase_mode() {
    // Byte $41 = code 1 ('A'), INV set. Plain MC6847 (no T1 knob at all) and
    // T1-with-GM0-clear each draw their own font's ordinary uppercase 'A'
    // with the ordinary INV colour swap — the existing inverse-uppercase
    // behaviour, unaffected by the new lowercase code path. (The two chips'
    // glyphs occupy different rows within the cell — see `PLAIN_A_GLYPH` /
    // `T1_A_GLYPH` — so this does *not* assert the two renders are pixel-
    // identical to each other, only that neither took the lowercase path.)
    let inverse_a = CODE_A | INVERSE_BIT;

    let mut plain = boot_parked_machine_with(coco1_config());
    plain.bus.pia1.b.output = 0;
    plain.bus.write(SCREEN_BASE, inverse_a);
    plain.run_field();

    let mut t1 = boot_parked_machine_with(coco2_t1_config());
    t1.bus.pia1.b.output = 0; // GM0 clear
    t1.bus.write(SCREEN_BASE, inverse_a);
    t1.run_field();

    let fg = VDG_FIXED_PALETTE[13];
    let bg = VDG_FIXED_PALETTE[12];
    // INV set -> normal mapping swaps to (bg, fg): "on" pixels draw in `bg`.
    let plain_cell = sample_cell(&plain.framebuffer, 0, 0, bg, fg);
    let t1_cell = sample_cell(&t1.framebuffer, 0, 0, bg, fg);
    assert_eq!(
        plain_cell,
        glyph_bits(&PLAIN_A_GLYPH),
        "plain MC6847 inverse 'A' must draw its own font's uppercase glyph"
    );
    assert_eq!(
        t1_cell,
        glyph_bits(&T1_A_GLYPH),
        "T1-with-GM0-clear inverse 'A' must draw its own font's uppercase glyph, not lowercase"
    );
}
