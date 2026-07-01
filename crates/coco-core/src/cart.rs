//! Cartridge port devices (FDC, ROM pack, Multi-Pak). See `DESIGN.md` §7.
//!
//! NOTE: a trait object (`Box<dyn Cartridge>`) is not `Serialize`, so `SystemBus`
//! and `Machine` don't yet derive serde. For save-states (`DESIGN.md` §9) this
//! becomes an enum or uses typetag — deferred.

/// A device on the cartridge port.
pub trait Cartridge {
    /// Read cartridge I/O ($FF40–$FF5F) or cartridge ROM space.
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
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
