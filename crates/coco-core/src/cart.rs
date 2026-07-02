//! Cartridge port devices (FDC, ROM pack, Multi-Pak). See `DESIGN.md` §7.
//!
//! NOTE: a trait object (`Box<dyn Cartridge>`) is not `Serialize`, so `SystemBus`
//! and `Machine` don't yet derive serde. For save-states (`DESIGN.md` §9) this
//! becomes an enum or uses typetag — deferred.

/// Value read from the external ROM window when nothing drives the bus:
/// $00, matching real hardware / MAME coco3 (verified by MAME trace-diff,
/// 2026-07-02 — an empty slot's `LDD $C000` yields $0000, not $FFFF).
pub const ROM_OPEN_BUS: u8 = 0x00;

/// A device on the cartridge port.
pub trait Cartridge {
    /// Read cartridge I/O ($FF40–$FF5F, the SCS* decode).
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
    /// Read the external ROM window (the CTS* decode): `$C000–$FDFF`, or all
    /// of `$8000–$FDFF` when INIT0 MC1:MC0 selects the 32K-external map.
    fn rom_read(&mut self, _addr: u16) -> u8 {
        ROM_OPEN_BUS
    }
}

/// No cartridge inserted.
#[derive(Debug, Clone, Default)]
pub struct EmptySlot;

impl Cartridge for EmptySlot {
    fn read(&mut self, _addr: u16) -> u8 {
        0xFF
    }
    fn write(&mut self, _addr: u16, _val: u8) {}
}
