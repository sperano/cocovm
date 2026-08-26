//! Tandy Multi-Pak Interface (MPI) coverage: the `$FF7F` select register
//! (readback masking, full-byte-replace semantics), front-panel-switch vs.
//! software-write control, SCS/CTS/CART* per-slot routing, the HALT*/NMI
//! wire-OR across all 4 slots regardless of selection, and an end-to-end
//! boot integration with the FD-502 nested in slot 4. Facts per MAME
//! `coco_multi.cpp` (`coco_multipak_device`) — see `crate::cart` doc comments.

use coco_core::cart::{Cart, Cartridge, MultiPak, ROMPak};
use coco_core::fdc::DiskCart;
use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;
use test_assets::rom::{COCO3, DISK11};

/// `$FF7F`: the MPI's own select register (see `crate::bus::MPI_CONTROL_REG`).
const MPI_CONTROL: u16 = 0xFF7F;
/// Front-panel switch position for physical slot 4 — the conventional
/// disk-controller default (`mpi::SWITCH_VALUES` index 3 -> `0xFF`).
const SWITCH_SLOT4: usize = 3;

/// A bus with a small dummy ROM — these tests never boot code, only drive
/// registers directly (same pattern as `tests/gime_irq.rs`).
/// Unit-level bus pokes never boot a ROM, so state the SCS-window
/// precondition (INIT0 MC2) explicitly instead of relying on the ROM's own gating.
fn open_scs_gate(b: &mut SystemBus) {
    b.gime.write_init0(coco_core::gime::init0::MC2);
}

fn bus() -> SystemBus {
    SystemBus::new(
        MachineVariant::Coco3,
        MemorySize::K512,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    )
}

/// A `Machine` with a small dummy ROM, for the reset-propagation test, which
/// needs `Machine::reset` (CPU + cartridge) but never runs any code.
fn dummy_machine() -> Machine {
    Machine::new(
        MachineConfig::default(),
        vec![0u8; 32 * 1024].into_boxed_slice(),
    )
}

// ============================================================================
// A minimal recording cartridge for observing MultiPak routing.
// ============================================================================

/// Address (within the SCS window) [`TestCart::read`] treats as a tick-count
/// probe instead of the normal read-your-last-write behaviour.
const PROBE_TICKS: u16 = 0xFF44;

/// Records the last value written to it (so a bus read can prove which slot
/// answered without needing to downcast out of the `MultiPak`'s trait-object
/// slots), and can be told to assert HALT*/a pending NMI/CART*-tie-to-Q, or
/// count ticks.
struct TestCart {
    id: u8,
    last_write: Option<u8>,
    ties_q: bool,
    halt: bool,
    nmi_pending: bool,
    ticks: u32,
}

impl TestCart {
    fn new(id: u8) -> Self {
        Self {
            id,
            last_write: None,
            ties_q: false,
            halt: false,
            nmi_pending: false,
            ticks: 0,
        }
    }
}

impl Cartridge for TestCart {
    fn read(&mut self, addr: u16) -> u8 {
        if addr == PROBE_TICKS {
            self.ticks as u8
        } else {
            self.last_write.unwrap_or(self.id)
        }
    }
    fn write(&mut self, _addr: u16, val: u8) {
        self.last_write = Some(val);
    }
    fn cart_line_ties_q(&self) -> bool {
        self.ties_q
    }
    fn tick(&mut self, cycles: u32) {
        self.ticks += cycles;
    }
    fn halt_asserted(&self) -> bool {
        self.halt
    }
    fn take_nmi(&mut self) -> bool {
        std::mem::replace(&mut self.nmi_pending, false)
    }
}

// ============================================================================
// $FF7F read-back: forced-high unused bits, full-byte replace.
// ============================================================================

#[test]
fn readback_after_reset_uses_the_default_switch_slot4() {
    let mut b = bus();
    b.cart = MultiPak::new(SWITCH_SLOT4).into();
    b.cart.reset();
    assert_eq!(b.read(MPI_CONTROL), 0xFF);
}

#[test]
fn write_ors_the_unused_bits_high_on_readback() {
    let mut b = bus();
    b.cart = MultiPak::new(SWITCH_SLOT4).into();
    b.write(MPI_CONTROL, 0x10);
    assert_eq!(b.read(MPI_CONTROL), 0xDC, "0x10 | 0xCC");
}

#[test]
fn write_replaces_the_whole_byte_not_a_nibble_merge() {
    let mut b = bus();
    b.cart = MultiPak::new(SWITCH_SLOT4).into();
    b.write(MPI_CONTROL, 0x10); // SCS slot index 0, CTS slot index 1 (physical slot 2)
    assert_eq!(b.read(MPI_CONTROL), 0xDC);
    b.write(MPI_CONTROL, 0x01); // SCS slot index 1 (physical slot 2), CTS slot index 0 (physical slot 1)
    assert_eq!(b.read(MPI_CONTROL), 0xCD, "0x01 | 0xCC");
    let mp = b.cart.as_multipak().expect("a MultiPak is inserted");
    assert_eq!(mp.scs_slot(), 1, "SCS bits from the 0x01 write must stand");
    assert_eq!(
        mp.cts_slot(),
        0,
        "CTS bits must have been cleared by the full-byte replace, not left over from the 0x10 write"
    );
}

// ============================================================================
// Switch vs. software-write control.
// ============================================================================

#[test]
fn set_switch_updates_select_before_any_software_write() {
    let mut mp = MultiPak::new(SWITCH_SLOT4);
    mp.set_switch(1); // physical slot 2
    assert_eq!(mp.control_read(), 0xDD);
    assert_eq!(mp.scs_slot(), 1);
    assert_eq!(mp.cts_slot(), 1);
}

#[test]
fn software_write_blocks_the_switch_until_the_next_reset() {
    let mut mp = MultiPak::new(SWITCH_SLOT4);
    mp.control_write(0x00); // software selects slot index 0 everywhere
    assert_eq!(mp.control_read(), 0xCC);

    mp.set_switch(1); // physical switch moved to slot 2 -- must be ignored now
    assert_eq!(
        mp.control_read(),
        0xCC,
        "a software write must block the switch until the next reset"
    );
    assert_eq!(
        mp.switch_slot(),
        1,
        "the new switch position is still recorded while blocked"
    );

    mp.reset();
    assert_eq!(
        mp.control_read(),
        0xDD,
        "reset must restore switch control and reload the (moved) switch's value"
    );
}

#[test]
fn machine_reset_restores_switch_control_and_reloads_value() {
    let mut m = dummy_machine();
    let mut mp = MultiPak::new(SWITCH_SLOT4);
    mp.control_write(0x10); // software takes over
    mp.set_switch(1); // physical slot 2 -- blocked, must not apply yet
    m.insert_cartridge(mp);

    assert_eq!(
        m.bus.read(MPI_CONTROL),
        0xDC,
        "software value still in effect pre-reset"
    );
    m.reset();
    assert_eq!(
        m.bus.read(MPI_CONTROL),
        0xDD,
        "Machine::reset must reload the MPI's select register from the (moved) switch"
    );
}

// ============================================================================
// SCS routing ($FF40-$FF5F): follows bits 1-0 only.
// ============================================================================

#[test]
fn scs_routing_follows_bits_1_0_and_tracks_changes() {
    let mut b = bus();
    open_scs_gate(&mut b);
    let mut mp = MultiPak::new(SWITCH_SLOT4);
    mp.insert(0, Cart::custom(TestCart::new(0xA0)));
    mp.insert(1, Cart::custom(TestCart::new(0xB0)));
    b.cart = mp.into();

    b.write(MPI_CONTROL, 0x00); // SCS slot 0
    assert_eq!(b.read(0xFF40), 0xA0);
    b.write(0xFF41, 0x55);
    assert_eq!(b.read(0xFF40), 0x55, "slot 0 must have recorded the write");

    b.write(MPI_CONTROL, 0x01); // SCS slot 1
    assert_eq!(
        b.read(0xFF40),
        0xB0,
        "slot 1 has no write recorded yet, so its id answers"
    );
}

// ============================================================================
// CTS routing (external ROM window): follows bits 5-4 only.
// ============================================================================

#[test]
fn cts_routing_follows_bits_5_4() {
    let mut b = bus();
    let mut mp = MultiPak::new(SWITCH_SLOT4);
    mp.insert(0, ROMPak::from_bytes(&[0xAA], false).unwrap());
    mp.insert(1, ROMPak::from_bytes(&[0xBB], false).unwrap());
    b.cart = mp.into();

    b.write(MPI_CONTROL, 0x00); // CTS slot 0
    assert_eq!(b.cart.rom_read(0x8000), 0xAA);
    b.write(MPI_CONTROL, 0x10); // CTS slot 1 (bits 5:4 = 01)
    assert_eq!(b.cart.rom_read(0x8000), 0xBB);
}

// ============================================================================
// CART*: follows the CTS select only.
// ============================================================================

#[test]
fn cart_line_ties_q_follows_cts_select_only() {
    let mut b = bus();
    let mut mp = MultiPak::new(SWITCH_SLOT4);
    mp.insert(0, ROMPak::from_bytes(&[0u8], true).unwrap()); // autostart
    mp.insert(1, ROMPak::from_bytes(&[0u8], false).unwrap()); // not autostart
    b.cart = mp.into();

    b.write(MPI_CONTROL, 0x00); // CTS slot 0 (autostart)
    assert!(b.cart.cart_line_ties_q());
    b.write(MPI_CONTROL, 0x10); // CTS slot 1 (not autostart)
    assert!(!b.cart.cart_line_ties_q());
}

// ============================================================================
// Wire-OR: HALT*/NMI/tick reach every slot regardless of selection.
// ============================================================================

#[test]
fn halt_and_nmi_are_wire_ored_across_all_slots_regardless_of_selection() {
    let mut b = bus();
    let mut mp = MultiPak::new(SWITCH_SLOT4);
    let mut halting = TestCart::new(0x10);
    halting.halt = true;
    mp.insert(1, Cart::custom(halting));
    let mut nmi_cart = TestCart::new(0x20);
    nmi_cart.nmi_pending = true;
    mp.insert(2, Cart::custom(nmi_cart));
    b.cart = mp.into();

    b.write(MPI_CONTROL, 0x00); // selects slot 0 for both SCS and CTS -- neither slot 1 nor 2
    assert!(
        b.halt_asserted(),
        "HALT* must be wire-ORed even though slot 1 (which asserts it) isn't selected"
    );
    assert!(
        b.take_nmi(),
        "NMI must be wire-ORed even though slot 2 (which has a pending edge) isn't selected"
    );
}

#[test]
fn tick_advances_every_slot_regardless_of_selection() {
    let mut b = bus();
    open_scs_gate(&mut b);
    let mut mp = MultiPak::new(SWITCH_SLOT4);
    for i in 0..4u8 {
        mp.insert(i as usize, Cart::custom(TestCart::new(i)));
    }
    b.cart = mp.into();

    b.write(MPI_CONTROL, 0x00); // only slot 0 is ever selected
    b.cart.tick(7);

    for slot in 0..4u8 {
        b.write(MPI_CONTROL, slot); // select each slot as SCS in turn to probe it
        assert_eq!(
            b.read(PROBE_TICKS),
            7,
            "slot {slot} must have ticked even though only slot 0 was ever selected"
        );
    }
}

// ============================================================================
// Real-ROM integration: FD-502 nested in slot 4, switch on slot 4.
// ============================================================================

/// Like `tests/fdc.rs`'s `try_load_rom`: returns `None` instead of panicking
/// when the (git-ignored) ROM image isn't present, so this test skips
/// gracefully in an asset-less checkout.
fn try_load_rom(name: &str) -> Option<Box<[u8]>> {
    let path = test_assets::rom(name);
    std::fs::read(&path).ok().map(Vec::into_boxed_slice)
}

fn screen_row(m: &mut Machine, row: u16) -> String {
    (0..32)
        .map(|c| {
            let code = m.bus.read(0x0400 + row * 32 + c) & 0x3F;
            if code < 0x20 {
                (b'@' + code) as char
            } else {
                (b' ' + (code - 0x20)) as char
            }
        })
        .collect()
}

#[test]
fn mpi_with_fd502_in_slot4_and_switch_on_slot4_boots_disk_basic() {
    const FIELDS: usize = 400;
    let (Some(coco), Some(disk_rom)) = (try_load_rom(COCO3), try_load_rom(DISK11)) else {
        eprintln!(
            "skipping mpi_with_fd502_in_slot4_and_switch_on_slot4_boots_disk_basic: roms/ assets not present"
        );
        return;
    };

    let mut m = Machine::new(MachineConfig::default(), coco);
    let mut mp = MultiPak::new(SWITCH_SLOT4);
    mp.insert(SWITCH_SLOT4, DiskCart::new(disk_rom));
    m.insert_cartridge(mp);
    m.reset();
    for _ in 0..FIELDS {
        m.run_field();
    }
    let banner = (0..16).any(|r| screen_row(&mut m, r).contains("DISK EXTENDED COLOR BASIC"));
    assert!(
        banner,
        "expected the Disk BASIC banner (proves CTS ROM forwarding + reset switch value); row0 = {:?}",
        screen_row(&mut m, 0)
    );
}
