//! CoCo Max Hi-Res Input Module coverage on the plain-SAM bus path (CoCo
//! 1/2): the `$FF90-$FF97` ADC window's latch-lag read protocol reached
//! through `SystemBus`, that neighboring addresses stay open bus, MPI slot
//! routing, and a snapshot round-trip of a bus holding the module.

use coco_core::cart::{COCOMAX_IO_BASE, COCOMAX_IO_LAST, Cartridge, CoCoMaxModule, MultiPak};
use coco_core::snapshot::{self, MediaRefs, MediaSources, SnapshotPayload};
use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;

/// `$FF91`: the X-axis channel.
const X_REG: u16 = COCOMAX_IO_BASE + 1;
/// `$FF92`/`$FF93`: the left/right button channels.
const LEFT_BUTTON_REG: u16 = COCOMAX_IO_BASE + 2;
const RIGHT_BUTTON_REG: u16 = COCOMAX_IO_BASE + 3;
/// Just past the module's window — must stay open bus.
const NEIGHBOR_REG: u16 = COCOMAX_IO_LAST + 1;

/// A CoCo 2 bus (plain SAM, no GIME) with a small placeholder ROM — these
/// tests only poke `$FF90-$FF97` directly, never execute cart code.
fn bus_with_cocomax() -> SystemBus {
    let mut b = SystemBus::new(
        MachineVariant::Coco2,
        MemorySize::K64,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    b.cart = CoCoMaxModule::new().into();
    b
}

#[test]
fn two_reads_of_the_x_channel_return_the_axis() {
    let mut b = bus_with_cocomax();
    b.cart.as_cocomax().unwrap().set_position(0x11, 0x22);

    b.read(X_REG); // starts a conversion on X
    assert_eq!(
        b.read(X_REG),
        0x11,
        "second read sees the started conversion"
    );
}

#[test]
fn button_channels_read_through_the_bus() {
    let mut b = bus_with_cocomax();
    b.cart.as_cocomax().unwrap().set_buttons(true, false);

    b.read(LEFT_BUTTON_REG);
    assert_eq!(b.read(LEFT_BUTTON_REG), 0x00, "left pressed");
    b.read(RIGHT_BUTTON_REG);
    assert_eq!(b.read(RIGHT_BUTTON_REG), 0xFF, "right released");
}

#[test]
fn neighboring_addresses_stay_open_bus() {
    let mut b = bus_with_cocomax();
    assert_eq!(
        b.read(NEIGHBOR_REG),
        0xFF,
        "$FF98 is outside the module's window"
    );
}

#[test]
fn writes_to_the_window_are_ignored() {
    let mut b = bus_with_cocomax();
    b.write(X_REG, 0xAA);
    let cocomax = b.cart.as_cocomax().unwrap();
    // A write starts no conversion, so the latched result stays at its
    // power-on value of 0 rather than picking up anything from the write.
    assert_eq!(cocomax.upper_io_peek(X_REG), 0);
}

#[test]
fn peek_does_not_disturb_a_pending_conversion() {
    let mut b = bus_with_cocomax();
    b.cart.as_cocomax().unwrap().set_position(0x11, 0x22);
    b.read(X_REG); // starts a conversion on X; the latched result is now 0x11

    assert_eq!(
        b.peek(X_REG),
        0x11,
        "peek reports the conversion the read started"
    );
    assert_eq!(
        b.peek(X_REG),
        0x11,
        "repeated peeks don't perturb the latch"
    );
    assert_eq!(
        b.read(X_REG),
        0x11,
        "a real read still sees the same latched value"
    );
}

// ---- Through the MPI --------------------------------------------------------

/// The module has no SCS* wiring of its own — like the Deluxe RS-232 Pak's
/// ACIA, it decodes the raw address bus, which the MPI does not switch. So
/// `$FF90-$FF97` must reach it from any slot regardless of which one `$FF7F`
/// currently selects, unlike the SCS window (`$FF40-$FF5F`).
#[test]
fn mpi_broadcasts_the_window_to_every_slot_regardless_of_switch() {
    const COCOMAX_SLOT: usize = 1;
    const OTHER_SLOT: usize = 3;
    let mut b = SystemBus::new(
        MachineVariant::Coco2,
        MemorySize::K64,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    let mut mp = MultiPak::new(OTHER_SLOT);
    let mut cocomax = CoCoMaxModule::new();
    cocomax.set_position(0x33, 0x44);
    mp.insert(COCOMAX_SLOT, cocomax);
    b.cart = mp.into();

    // `$FF7F` selects OTHER_SLOT (empty), not COCOMAX_SLOT — a real SCS
    // window (like `$FF40`) would answer open bus here.
    assert_eq!(b.cart.as_multipak().unwrap().scs_slot(), OTHER_SLOT);

    b.read(X_REG); // starts a conversion on X, reaching the module despite the switch
    assert_eq!(
        b.read(X_REG),
        0x33,
        "the module answers from its own slot even though another slot is selected"
    );
}

// ---- Snapshot round trip ----------------------------------------------------

#[test]
fn snapshot_round_trip_preserves_module_state() {
    let mut b = bus_with_cocomax();
    let cocomax = b.cart.as_cocomax().unwrap();
    cocomax.set_position(0x11, 0x22);
    cocomax.set_buttons(true, false);
    b.read(X_REG); // starts a conversion, so `result` is non-default too
    b.read(X_REG);

    let mut bytes = Vec::new();
    ciborium::into_writer(&b, &mut bytes).expect("serialize bus with CoCo Max module");
    let mut restored: SystemBus = ciborium::from_reader(bytes.as_slice()).expect("deserialize bus");

    let original = b.cart.as_cocomax().unwrap().upper_io_peek(X_REG);
    let round_tripped = restored.cart.as_cocomax().unwrap().upper_io_peek(X_REG);
    assert_eq!(
        original, round_tripped,
        "latched result survives the round trip"
    );
    assert_eq!(original, 0x11);

    // Buttons round-trip too: prime a conversion on the left-button channel,
    // then compare what the next read latches on each bus.
    b.read(LEFT_BUTTON_REG);
    restored.read(LEFT_BUTTON_REG);
    let left_before = b.read(LEFT_BUTTON_REG);
    let left_after = restored.read(LEFT_BUTTON_REG);
    assert_eq!(
        left_before, left_after,
        "button state survives the round trip"
    );
    assert_eq!(left_before, 0x00, "left button was held before serializing");
}

/// Defense-in-depth against a hand-crafted (or future-schema) payload that
/// pairs the module with a CoCo 3: the CoCo 3's GIME owns `$FF90-$FF97`, so
/// nothing on that variant could ever reach the module. The frontend never
/// produces this combination itself (`new_vm`/`launch` refuse it), but
/// `restore` must reject it too.
#[test]
fn restore_rejects_the_module_on_a_coco3() {
    // `MachineConfig::default()` is already a CoCo 3.
    let mut machine = Machine::new(
        MachineConfig::default(),
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    machine.insert_cartridge(CoCoMaxModule::new());

    let payload = SnapshotPayload {
        media: MediaRefs::default(),
        machine,
    };
    let message = match snapshot::restore(payload, MediaSources::default()) {
        Ok(_) => panic!("a CoCo 3 payload with a CoCo Max module must be rejected"),
        Err(e) => e.to_string(),
    };
    assert!(
        message.contains("CoCo Max"),
        "error should name the module: {message}"
    );
}
