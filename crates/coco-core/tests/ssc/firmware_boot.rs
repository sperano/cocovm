//! The firmware on the cartridge: boots to its idle loop and leaves the
//! board in a known state. Needs both SSC ROMs.

use tms7000::Port;

use super::common::{FF7E, NOT_BUSY, skip, try_coco3_bus_with_ssc};
use mc6809::Bus;

/// INT3 enable in IOCNT0: the firmware waits for host bytes.
const INT3_ENABLE: u8 = 0x10;

#[test]
fn firmware_boots_to_its_idle_loop() {
    let Some(mut b) = try_coco3_bus_with_ssc() else {
        return skip("firmware_boots_to_its_idle_loop");
    };
    assert_eq!(b.read(FF7E) & NOT_BUSY, NOT_BUSY);
    let ssc = b.cart.as_ssc().unwrap();
    let cpu = ssc.firmware();
    assert_eq!(cpu.illegal_count, 0, "no illegal opcode during boot");
    assert!(!cpu.pending_reset());
    assert!(cpu.pc >= tms7000::ROM_BASE, "running from ROM");
    assert_eq!(cpu.port_ddr(Port::C), 0xFF, "port C drives the board");
    assert_eq!(cpu.io_control() & INT3_ENABLE, INT3_ENABLE);
}
