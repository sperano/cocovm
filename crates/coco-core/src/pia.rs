//! MC6821 Peripheral Interface Adapter. See `DESIGN.md` §7.
//!
//! STATUS: skeleton — register file only. The DDR/data access split, the
//! CA1/CA2/CB1/CB2 line logic, and clear-on-read interrupt flags are TODO.

use serde::{Deserialize, Serialize};

/// One side (A or B) of an MC6821.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PiaPort {
    pub output: u8,
    pub ddr: u8,
    pub control: u8,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MC6821 {
    pub a: PiaPort,
    pub b: PiaPort,
}

impl MC6821 {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read one of the 4 PIA registers (`addr & 0x03`). Stub.
    pub fn read(&mut self, _reg: u8) -> u8 {
        0
    }

    /// Write one of the 4 PIA registers (`addr & 0x03`). Stub.
    pub fn write(&mut self, _reg: u8, _val: u8) {}
}
