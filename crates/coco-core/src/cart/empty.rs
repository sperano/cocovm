//! The empty cartridge slot.

use serde::{Deserialize, Serialize};

use super::{Cartridge, IO_OPEN_BUS};

/// No cartridge inserted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmptySlot;

impl Cartridge for EmptySlot {
    fn read(&mut self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    fn write(&mut self, _addr: u16, _val: u8) {}
}
