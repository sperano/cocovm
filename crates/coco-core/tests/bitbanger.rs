//! Bus-level check for the printer BUSY line (PIA1 PB0, `$FF22`) polarity:
//! 0 = ready, 1 = busy (wiki `cocovm/bitbanger-spec` "Register map"). BASIC's
//! driver spins while carry is set after `LDB $FF22 / LSRB`, that is, while bit 0
//! is 1, so the not-busy default must present bit 0 clear or every `PRINT`
//! statement would hang waiting for a BUSY that never clears.
//!
//! The pure decode-logic tests (framing, timing slop, LSB-first ordering,
//! resync) live as inline `#[cfg(test)]` unit tests in
//! `crates/coco-core/src/bitbanger.rs`, next to the state machine they
//! exercise. This file only covers the bus wiring (`SystemBus::pia1_pb_pins`)
//! that inline tests can't reach.

use coco_core::{MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;

const PIA1_DDRB: u16 = 0xFF22; // with control DDR-selected
const PIA1_DB: u16 = 0xFF22; // with control data-selected
const PIA1_CRB: u16 = 0xFF23;
/// Control value selecting the DDR (bit 2 clear).
const CR_DDR: u8 = 0x30;
/// Control value selecting the data register (bit 2 set), C2 low.
const CR_DATA: u8 = 0x34;

fn bus() -> SystemBus {
    let mut b = SystemBus::new(
        MachineVariant::Coco3,
        MemorySize::K512,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    // PB0 (BUSY) stays an input pin (DDRB bit 0 = 0), matching the ROM's
    // DDRB = $F8 (`bitbanger-spec.md` "CoCo 3 differences").
    b.write(PIA1_CRB, CR_DDR);
    b.write(PIA1_DDRB, 0xF8);
    b.write(PIA1_CRB, CR_DATA);
    b
}

#[test]
fn busy_line_defaults_to_ready_bit_clear() {
    let mut b = bus();
    let pb = b.read(PIA1_DB);
    assert_eq!(
        pb & 0x01,
        0,
        "not-busy default must read PB0=0 (ready): {pb:#04x}"
    );
}

#[test]
fn busy_line_reads_bit_set_when_asserted() {
    let mut b = bus();
    b.bitbanger.set_busy(true);
    let pb = b.read(PIA1_DB);
    assert_eq!(pb & 0x01, 1, "asserted busy must read PB0=1: {pb:#04x}");
}

#[test]
fn busy_line_clears_again_after_release() {
    let mut b = bus();
    b.bitbanger.set_busy(true);
    assert_eq!(b.read(PIA1_DB) & 0x01, 1);
    b.bitbanger.set_busy(false);
    assert_eq!(b.read(PIA1_DB) & 0x01, 0);
}
