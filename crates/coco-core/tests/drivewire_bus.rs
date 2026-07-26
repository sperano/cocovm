//! Becker-port ($FF41/$FF42) wiring of `DwServer` into `SystemBus`: disabled
//! default, register decode while enabled, full protocol round-trips over
//! the bus, precedence over cartridge dispatch, and the CoCo 1/2 plain-SAM
//! decode path. Style model: `tests/bus_map.rs` — a synthetic ROM, no
//! `Machine`/booting, registers driven directly via `mc6809::Bus`.

use std::cell::Cell;
use std::rc::Rc;

use coco_core::cart::Cart;
use coco_core::drivewire::{DwImage, SECTOR_SIZE, error, opcode};
use coco_core::{MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;

const ROM_SIZE: usize = 32 * 1024;

/// A 32K ROM whose every byte equals its low-address byte (same helper as
/// `tests/bus_map.rs`; content is irrelevant here, only size/presence).
fn marked_rom() -> Box<[u8]> {
    (0..ROM_SIZE)
        .map(|i| i as u8)
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn bus() -> SystemBus {
    SystemBus::new(MachineVariant::Coco3, MemorySize::K512, marked_rom())
}

const BECKER_STATUS: u16 = 0xFF41;
const BECKER_DATA: u16 = 0xFF42;

/// [`opcode::DWINIT`]'s reply byte, hardcoded here to match drivewire.rs's
/// private `DW_PROTOCOL_VERSION` constant (not reachable from this test
/// crate since it's private to the module).
const DW_PROTOCOL_VERSION: u8 = 0x04;

/// Fixed byte a [`MarkerCart`] answers on any `read`/SCS* dispatch, so tests
/// can tell "the cartridge answered" apart from "the Becker port answered".
const MARKER_CART_READ: u8 = 0x77;

/// A cartridge that answers a fixed marker byte on reads and records the
/// last byte written to it. `last_write` is an `Rc<Cell<u8>>` (not a plain
/// `Cell` field) so a test can keep an external handle to it after the
/// `MarkerCart` itself has been moved into `SystemBus::cart` behind
/// `Cart::custom`, whose trait-object box has no downcast.
struct MarkerCart {
    last_write: Rc<Cell<u8>>,
}

impl MarkerCart {
    fn new() -> (Self, Rc<Cell<u8>>) {
        let last_write = Rc::new(Cell::new(0u8));
        (
            Self {
                last_write: last_write.clone(),
            },
            last_write,
        )
    }
}

impl coco_core::cart::Cartridge for MarkerCart {
    fn read(&mut self, _addr: u16) -> u8 {
        MARKER_CART_READ
    }
    fn write(&mut self, _addr: u16, val: u8) {
        self.last_write.set(val);
    }
    fn rom_read(&mut self, _addr: u16) -> u8 {
        0xAA
    }
}

/// Feed one byte to $FF42. These register-decode-only tests never drive
/// `Machine::run_cycles`, so `SystemBus::cycle_clock` never advances past 0
/// — fine here since none of these tests straddle the transaction timeout.
fn feed(b: &mut SystemBus, byte: u8) {
    b.write(BECKER_DATA, byte);
}

/// Drain every queued reply byte from $FF41/$FF42.
fn drain(b: &mut SystemBus) -> Vec<u8> {
    let mut out = Vec::new();
    while b.read(BECKER_STATUS) != 0 {
        out.push(b.read(BECKER_DATA));
    }
    out
}

/// A recognizable, non-repeating 256-byte pattern: byte `i` has value
/// `i as u8` (mirrors `drivewire.rs`'s own test helper).
fn pattern_sector() -> Vec<u8> {
    (0..SECTOR_SIZE as u32).map(|i| i as u8).collect()
}

// ---- 1. Disabled Becker (default) ------------------------------------------

#[test]
fn disabled_becker_falls_through_to_cartridge() {
    let mut b = bus();
    // Default empty slot answers open-bus $FF for both registers, unchanged
    // from before this feature existed.
    assert_eq!(b.read(BECKER_STATUS), coco_core::cart::IO_OPEN_BUS);
    assert_eq!(b.read(BECKER_DATA), coco_core::cart::IO_OPEN_BUS);

    let (cart, last_write) = MarkerCart::new();
    b.cart = Cart::custom(cart);

    // Reads reach the cart's marker value.
    assert_eq!(b.read(BECKER_STATUS), MARKER_CART_READ);
    assert_eq!(b.read(BECKER_DATA), MARKER_CART_READ);

    // A write to $FF42 while Becker is disabled reaches the cart unaffected.
    b.write(BECKER_DATA, 0x42);
    assert_eq!(last_write.get(), 0x42);
}

// ---- 2. Enabled, idle -------------------------------------------------------

#[test]
fn enabled_becker_idle_status_and_stray_status_write() {
    let mut b = bus();
    b.enable_drivewire();
    assert_eq!(b.read(BECKER_STATUS), 0x00);

    feed(&mut b, opcode::DWINIT);
    feed(&mut b, 0x01);
    assert_eq!(b.read(BECKER_STATUS), 0x02);

    // A stray write to $FF41 must not panic and must not disturb the queued
    // reply.
    b.write(BECKER_STATUS, 0xAB);
    assert_eq!(b.read(BECKER_STATUS), 0x02);
    assert_eq!(b.read(BECKER_DATA), DW_PROTOCOL_VERSION);
    assert_eq!(b.read(BECKER_STATUS), 0x00);
}

// ---- 3. Full OP_DWINIT over the bus -----------------------------------------

#[test]
fn dwinit_round_trip_over_bus() {
    let mut b = bus();
    b.enable_drivewire();
    feed(&mut b, opcode::DWINIT);
    feed(&mut b, 0x01);
    assert_eq!(b.read(BECKER_STATUS), 0x02);
    assert_eq!(b.read(BECKER_DATA), DW_PROTOCOL_VERSION);
    assert_eq!(b.read(BECKER_STATUS), 0x00);
}

// ---- 4. Full OP_READ over the bus -------------------------------------------

#[test]
fn read_round_trip_over_bus() {
    let mut b = bus();
    b.enable_drivewire();

    let mut image = vec![0u8; SECTOR_SIZE * 2];
    image[SECTOR_SIZE..].copy_from_slice(&pattern_sector());
    b.drivewire
        .as_mut()
        .unwrap()
        .mount(0, DwImage::Memory(image));

    // opcode::READ, drive 0, LSN 1 (24-bit big-endian).
    feed(&mut b, opcode::READ);
    feed(&mut b, 0x00);
    feed(&mut b, 0x00);
    feed(&mut b, 0x00);
    feed(&mut b, 0x01);

    let reply = drain(&mut b);
    assert_eq!(reply[0], error::OK);
    assert_eq!(&reply[1..1 + SECTOR_SIZE], &pattern_sector()[..]);
    let expected_checksum: u16 = pattern_sector().iter().map(|&b| b as u16).sum();
    assert_eq!(reply[1 + SECTOR_SIZE], (expected_checksum >> 8) as u8);
    assert_eq!(reply[2 + SECTOR_SIZE], (expected_checksum & 0xFF) as u8);
    assert_eq!(reply.len(), 1 + SECTOR_SIZE + 2);

    assert_eq!(b.read(BECKER_STATUS), 0x00);
}

// ---- 5. Precedence over cartridge dispatch ----------------------------------

#[test]
fn becker_takes_precedence_over_cartridge() {
    let mut b = bus();
    b.enable_drivewire();
    let (cart, last_write) = MarkerCart::new();
    b.cart = Cart::custom(cart);

    // $FF41/$FF42 go to Becker (idle status), not the cart's marker value.
    assert_eq!(b.read(BECKER_STATUS), 0x00);
    assert_eq!(b.read(BECKER_DATA), 0x00);

    // $FF40/$FF43 (just outside the two Becker registers, still inside
    // CART_BASE..=CART_LAST) still reach the cartridge.
    assert_eq!(b.read(0xFF40), MARKER_CART_READ);
    assert_eq!(b.read(0xFF43), MARKER_CART_READ);

    // Writes: $FF42 and $FF41 feed/touch DriveWire, not the cart -- the
    // marker's last recorded write must stay at its initial value (0).
    b.write(BECKER_DATA, opcode::NOP);
    assert_eq!(last_write.get(), 0, "cart must not see the $FF42 write");
    b.write(BECKER_STATUS, 0x55);
    assert_eq!(last_write.get(), 0, "cart must not see the $FF41 write");

    // Writes just outside the Becker registers do reach the cart.
    b.write(0xFF40, 0x11);
    assert_eq!(last_write.get(), 0x11);
    b.write(0xFF43, 0x22);
    assert_eq!(last_write.get(), 0x22);
}

// ---- 6. CoCo 1/2 SAM path ----------------------------------------------------

#[test]
fn sam_path_becker_intercept_matches_gime_path() {
    // A small synthetic ROM: `Sam::map` routes all of $FF00-$FF9F
    // unconditionally to `SamTarget::Io` regardless of ROM contents/size, so
    // the ROM box just needs to exist.
    let rom: Box<[u8]> = vec![0u8; 1].into_boxed_slice();
    let mut b = SystemBus::new(MachineVariant::Coco1, MemorySize::K32, rom);

    // Disabled by default: falls through to the (empty) cartridge slot.
    assert_eq!(b.read(BECKER_STATUS), coco_core::cart::IO_OPEN_BUS);

    b.enable_drivewire();
    assert_eq!(b.read(BECKER_STATUS), 0x00);

    feed(&mut b, opcode::DWINIT);
    feed(&mut b, 0x01);
    assert_eq!(b.read(BECKER_STATUS), 0x02);
    assert_eq!(b.read(BECKER_DATA), DW_PROTOCOL_VERSION);
    assert_eq!(b.read(BECKER_STATUS), 0x00);
}
