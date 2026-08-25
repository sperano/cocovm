//! MC6847 SG6 decode and the MC6847T1's SG4 fallback for the same inputs.

use coco_core::video::{CELL_H, CELL_W, VDG_FIXED_PALETTE, VDG_GM0_INTEXT};
use mc6809::Bus;

use super::common::{
    SCREEN_BASE, boot_parked_machine_with, coco1_config, coco2_t1_config, sample_cell,
};

const SG_TEST_BYTE: u8 = 0b1000_1100;
const SG6_COLOR_INDEX: usize = 2;
const SG4_COLOR_INDEX: usize = 0;
const OFF_COLOR_INDEX: usize = 8;

fn expected_horizontal_band(first_row: usize, last_row: usize) -> [[bool; CELL_W]; CELL_H] {
    let mut expected = [[false; CELL_W]; CELL_H];
    for row in &mut expected[first_row..last_row] {
        row.fill(true);
    }
    expected
}

#[test]
fn plain_mc6847_intext_semigraphics_byte_uses_sg6() {
    let mut machine = boot_parked_machine_with(coco1_config());
    machine.bus.pia1.b.output = VDG_GM0_INTEXT;
    machine.bus.write(SCREEN_BASE, SG_TEST_BYTE);
    machine.run_field();

    let cell = sample_cell(
        &machine.framebuffer,
        0,
        0,
        VDG_FIXED_PALETTE[SG6_COLOR_INDEX],
        VDG_FIXED_PALETTE[OFF_COLOR_INDEX],
    );
    assert_eq!(cell, expected_horizontal_band(4, 8));
}

#[test]
fn mc6847t1_intext_semigraphics_byte_falls_back_to_sg4() {
    let mut machine = boot_parked_machine_with(coco2_t1_config());
    machine.bus.pia1.b.output = VDG_GM0_INTEXT;
    machine.bus.write(SCREEN_BASE, SG_TEST_BYTE);
    machine.run_field();

    let cell = sample_cell(
        &machine.framebuffer,
        0,
        0,
        VDG_FIXED_PALETTE[SG4_COLOR_INDEX],
        VDG_FIXED_PALETTE[OFF_COLOR_INDEX],
    );
    assert_eq!(cell, expected_horizontal_band(0, 6));
}
