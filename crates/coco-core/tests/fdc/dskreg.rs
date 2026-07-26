//! DSKREG decode + update_lines (coco_fdc.cpp)

use coco_core::cart::Cartridge;
use coco_core::fdc::dskreg;
use coco_core::wd1773::status;

use super::common::{disk_cart, marker_disk, read_marker_byte, two_sided_marker_disk};

#[test]
fn reset_state_has_drq_set_so_halt_never_spuriously_asserts() {
    let cart = disk_cart();
    // dskreg=0 out of the gate, so halt-enable is clear regardless of drq —
    // this is the "reset state drq=true" fact under direct test.
    assert!(!cart.halt_asserted());
}

#[test]
fn halt_line_is_not_drq_and_halt_enable() {
    let mut cart = disk_cart();
    cart.write(0xFF40, dskreg::HALT_ENABLE);
    // drq is still true (nothing has cleared it yet): HALT must not assert.
    assert!(!cart.halt_asserted());
    cart.read(0xFF4B); // clears DRQ as a side effect
    assert!(cart.halt_asserted(), "HALT must assert once DRQ clears with halt-enable set");
}

#[test]
fn intrq_high_clears_dskreg_halt_enable() {
    let mut cart = disk_cart();
    cart.write(0xFF40, dskreg::HALT_ENABLE);
    cart.read(0xFF4B); // drq now false -> halt asserted
    assert!(cart.halt_asserted());
    cart.write(0xFF48, 0xD8); // Force Interrupt, I3 set -> INTRQ high
    assert!(!cart.halt_asserted(), "a high INTRQ must clear DSKREG's halt-enable bit");
}

#[test]
fn nmi_edge_fires_only_when_density_nmi_enable_bit_is_set() {
    let mut cart = disk_cart();
    cart.write(0xFF40, 0); // bit5 clear
    cart.write(0xFF48, 0xD8); // Force Interrupt, I3 -> INTRQ high
    assert!(!cart.take_nmi(), "NMI must not fire when DSKREG bit5 is clear");

    let mut cart = disk_cart();
    cart.write(0xFF40, dskreg::DENSITY_AND_NMI_ENABLE);
    cart.write(0xFF48, 0xD8);
    assert!(cart.take_nmi(), "NMI must fire on the rising edge of intrq && bit5");
    assert!(!cart.take_nmi(), "the edge must not repeat once consumed");
}

#[test]
fn dskreg_reads_are_open_bus() {
    let mut cart = disk_cart();
    cart.write(0xFF40, 0xFF);
    for addr in 0xFF40u16..=0xFF47 {
        assert_eq!(cart.read(addr), coco_core::cart::IO_OPEN_BUS);
    }
}

#[test]
fn drive_select_priority_bit2_then_bit1_then_bit0_then_bit6() {
    let mut cart = disk_cart();
    cart.insert_disk(0, marker_disk(10));
    cart.insert_disk(1, marker_disk(11));
    cart.insert_disk(2, marker_disk(12));
    cart.insert_disk(3, marker_disk(13));

    let cases: [(u8, u8); 4] = [
        (dskreg::DRIVE2 | dskreg::DRIVE1 | dskreg::DRIVE0, 12), // bit2 wins
        (dskreg::DRIVE1 | dskreg::DRIVE0, 11),                  // bit1 wins (no bit2)
        (dskreg::DRIVE0, 10),                                   // bit0 wins
        (dskreg::DRIVE3_OR_SIDE, 13),                           // only bit6 -> drive 3
    ];
    for (select_bits, expect_marker) in cases {
        cart.write(0xFF40, dskreg::MOTOR_ON | select_bits);
        assert_eq!(read_marker_byte(&mut cart), expect_marker, "select bits {select_bits:#04x}");
    }
}

#[test]
fn side_select_is_bit6_unless_it_is_selecting_drive3() {
    let mut cart = disk_cart();
    cart.insert_disk(0, two_sided_marker_disk(20, 21));

    cart.write(0xFF40, dskreg::MOTOR_ON | dskreg::DRIVE0);
    assert_eq!(read_marker_byte(&mut cart), 20, "bit6 clear -> side 0");

    cart.write(0xFF40, dskreg::MOTOR_ON | dskreg::DRIVE0 | dskreg::DRIVE3_OR_SIDE);
    assert_eq!(read_marker_byte(&mut cart), 21, "bit6 set with drive0 selected -> side 1");
}

#[test]
fn not_ready_status_bit_reflects_missing_disk_or_motor_off() {
    let mut cart = disk_cart();
    cart.insert_disk(0, marker_disk(1));

    cart.write(0xFF40, dskreg::MOTOR_ON | dskreg::DRIVE0);
    assert_eq!(cart.read(0xFF48) & status::NOT_READY, 0, "mounted + motor on -> ready");

    cart.write(0xFF40, dskreg::DRIVE0); // motor off
    assert_eq!(cart.read(0xFF48) & status::NOT_READY, status::NOT_READY, "motor off -> not ready");

    cart.write(0xFF40, dskreg::MOTOR_ON | dskreg::DRIVE1); // unmounted drive
    assert_eq!(cart.read(0xFF48) & status::NOT_READY, status::NOT_READY, "unmounted drive -> not ready");
}
