//! Machine-level proof that CoCo 1/2 graphics fetches follow the MC6883 stream.

use coco_core::video::VDG_FIXED_PALETTE;
use mc6809::Bus;

use super::common::{SCREEN_BASE, boot_parked_machine, dot};

const FF22_AG: u8 = 0x80;
const FF22_GM_RG6: u8 = 7 << 4;
const SAM_V0_SET: u16 = 0xFFC1;
const SAM_V1_SET: u16 = 0xFFC3;
const SAM_M1_SET: u16 = 0xFFDD;
const PIXELS_PER_RG6_BYTE: usize = 8;
const REPEATED_HALF_OFFSET: usize = 16;

#[test]
fn mismatched_rg6_v3_repeats_each_sixteen_byte_half_line() {
    let mut machine = boot_parked_machine();
    machine.bus.write(SAM_M1_SET, 0);
    machine.bus.write(SAM_V0_SET, 0);
    machine.bus.write(SAM_V1_SET, 0);
    machine.bus.pia1.b.output = FF22_AG | FF22_GM_RG6;

    machine.bus.write(SCREEN_BASE, 0x80);
    machine
        .bus
        .write(SCREEN_BASE + REPEATED_HALF_OFFSET as u16, 0x00);
    machine.run_field();

    let on = VDG_FIXED_PALETTE[9];
    let off = VDG_FIXED_PALETTE[8];
    let repeated_x = REPEATED_HALF_OFFSET * PIXELS_PER_RG6_BYTE;
    assert_eq!(dot(&machine.framebuffer, 0, 0), on);
    assert_eq!(
        dot(&machine.framebuffer, repeated_x, 0),
        on,
        "the second half-line must re-read the first half's SAM addresses"
    );
    assert_eq!(dot(&machine.framebuffer, repeated_x + 1, 0), off);
}
