//! Shared test harness: a CPU wired to a flat 64K bus, with helpers to load a
//! program, run instructions, and inspect state. Used by the per-instruction
//! test-driven suites.

use mc6809::{Bus, FlatBus, MC6809};

pub struct Sys {
    pub cpu: MC6809,
    pub bus: FlatBus,
}

// Each integration test compiles this harness independently, so not every helper
// is exercised by every suite — that's expected for shared test scaffolding.
#[allow(dead_code)]
impl Sys {
    pub fn new() -> Self {
        Self {
            cpu: MC6809::new(),
            bus: FlatBus::new(),
        }
    }

    /// Build a system with `bytes` loaded at `addr` and PC pointed at it.
    pub fn code(addr: u16, bytes: &[u8]) -> Self {
        let mut s = Self::new();
        s.bus.load(addr, bytes);
        s.cpu.pc = addr;
        s
    }

    /// Execute one instruction; returns cycles consumed.
    pub fn step(&mut self) -> u32 {
        self.cpu.step(&mut self.bus)
    }

    pub fn mem(&mut self, addr: u16) -> u8 {
        self.bus.read(addr)
    }

    pub fn set_mem(&mut self, addr: u16, val: u8) {
        self.bus.write(addr, val);
    }
}
