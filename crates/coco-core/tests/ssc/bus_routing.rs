//! Sound/Speech Cartridge bus-routing coverage.

use coco_core::cart::{EmptySlot, MultiPak};
use coco_core::{MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;

use super::common::{FF7D, FF7E, NOT_BUSY, blank_ssc, bus_with_ssc};

#[test]
fn empty_slot_still_reads_open_bus_across_ff60_to_ff7e() {
    let mut b = SystemBus::new(
        MachineVariant::Coco3,
        MemorySize::K512,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    b.cart = EmptySlot.into();
    for addr in 0xFF60u16..=0xFF7E {
        assert_eq!(
            b.read(addr),
            0xFF,
            "addr {addr:#06x} must be open bus with an empty slot"
        );
    }
}

#[test]
fn ssc_reaches_ff7d_ff7e_on_the_coco1_2_plain_sam_path() {
    let mut b = bus_with_ssc(MachineVariant::Coco1, MemorySize::K64);
    assert_eq!(b.read(FF7D), 0xFF);

    b.write(FF7E, 0x12);
    assert_eq!(
        b.read(FF7E) & NOT_BUSY,
        0x00,
        "busy must be set on the plain-SAM (CoCo 1/2) path too"
    );
}

#[test]
fn ssc_in_a_non_scs_selected_mpi_slot_still_receives_ff7d_ff7e() {
    let mut b = SystemBus::new(
        MachineVariant::Coco3,
        MemorySize::K512,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    let mut mp = MultiPak::new(3); // switch on slot 4 (index 3)
    mp.insert(1, blank_ssc()); // SSC lives in slot 2 (index 1)
    b.cart = mp.into();

    // Re-point the SCS/CTS select at slot 0, definitely not the SSC's slot 1.
    b.write(0xFF7F, 0x00);
    assert_eq!(
        b.cart.as_multipak().unwrap().scs_slot(),
        0,
        "test setup: slot 0 must be SCS-selected, not the SSC's slot 1"
    );

    // $FF7D/$FF7E is outside the standard SCS window ($FF40-$FF5F), so the
    // MPI must broadcast it to every slot regardless of SCS selection.
    assert_eq!(b.read(FF7D), 0xFF);
    b.write(FF7E, 0x77);
    assert_eq!(
        b.read(FF7E) & NOT_BUSY,
        0x00,
        "busy must be set even though the SSC's slot isn't SCS-selected"
    );
}
