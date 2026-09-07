//! Texas Instruments TMS7000-family 8-bit microcontroller core, as the
//! TMS7040 variant: 128-byte register file, 4 KB internal ROM, timer 1,
//! ports A-D, and interrupts INT1/INT2/INT3. Port of MAME
//! `src/devices/cpu/tms7000/` (`tms7000.cpp`, `tms7000op.cpp`); every cycle
//! count and quirk is cited against that source.
//!
//! Single-chip use only: everything outside the register file, the
//! peripheral file, and the ROM goes to [`Bus::read_ext`] /
//! [`Bus::write_ext`], which default to reading 0. The four I/O ports reach
//! the board through [`Bus::read_port`] / [`Bus::write_port`].
//!
//! Clocking: one CPU cycle is two oscillator clocks (MAME `m_divider = 2`).
//! [`TMS7040::step`] runs one instruction, or takes one interrupt, and
//! returns the cycles it consumed; the on-chip timer advances by that same
//! count inside `step`, so a driver needs no separate timer tick.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub mod disasm;

mod decode;
mod exec;
mod memory;
mod peripheral;
#[cfg(feature = "serde")]
mod serde_util;
mod timer;

use timer::Timer1;

/// Internal ROM size of the TMS7040.
pub const ROM_SIZE: usize = 4096;
/// Where the internal ROM sits (MAME `tms7040_mem`: `$F000-$FFFF`).
pub const ROM_BASE: u16 = 0xF000;
/// Register file size: `$0000-$007F`, with A = R0 and B = R1.
pub const REGISTER_FILE_SIZE: usize = 128;
/// Peripheral file base: `$0100-$010B` on the 70x0 family.
pub const PERIPHERAL_FILE_BASE: u16 = 0x0100;

/// Interrupt and reset vectors (MAME `do_interrupt`: `0xfffc - irqline * 2`;
/// reset is TRAP 0 through `$FFFE`).
pub const VECTOR_INT1: u16 = 0xFFFC;
pub const VECTOR_INT2: u16 = 0xFFFA;
pub const VECTOR_INT3: u16 = 0xFFF8;
pub const VECTOR_RESET: u16 = 0xFFFE;

/// Status register bits (MAME `SR_C`/`SR_N`/`SR_Z`/`SR_I`); bits 3-0 are
/// unused and read as written.
pub mod st {
    pub const C: u8 = 0x80;
    pub const N: u8 = 0x40;
    pub const Z: u8 = 0x20;
    pub const I: u8 = 0x10;
}

/// The four I/O ports. Port A is input-only and port B output-only on the
/// 70x0 family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Port {
    A,
    B,
    C,
    D,
}

impl Port {
    pub(crate) const ALL: [Port; 4] = [Port::A, Port::B, Port::C, Port::D];

    /// Position in per-port arrays (A = 0 .. D = 3).
    pub fn index(self) -> usize {
        match self {
            Port::A => 0,
            Port::B => 1,
            Port::C => 2,
            Port::D => 3,
        }
    }
}

/// What the chip is soldered to: its port pins and, in expansion modes
/// (unused on single-chip parts), external memory, which reads 0 unless
/// overridden.
pub trait Bus {
    /// Input pin levels of `port` (MAME `m_port_in_cb`, default `0xFF`).
    fn read_port(&mut self, port: Port) -> u8;
    /// Output pin levels of `port`; `val` is already masked by the port's
    /// data-direction register.
    fn write_port(&mut self, port: Port, val: u8);
    /// External memory read for addresses the chip doesn't decode itself.
    fn read_ext(&mut self, _addr: u16) -> u8 {
        0
    }
    /// External memory write for addresses the chip doesn't decode itself.
    fn write_ext(&mut self, _addr: u16, _val: u8) {}
}

/// What one [`TMS7040::step`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    Instruction,
    /// An interrupt was taken (1, 2, or 3); no instruction executed.
    Interrupt(u8),
    /// The pending reset sequence ran; no instruction executed.
    Reset,
}

/// Cycles consumed by one [`TMS7040::step`] and what it did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub cycles: u32,
    pub kind: StepKind,
}

/// The ROM image handed to [`TMS7040::new`] wasn't exactly [`ROM_SIZE`] bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ROMSizeError {
    pub actual: usize,
}

impl std::fmt::Display for ROMSizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TMS7040 ROM must be exactly {ROM_SIZE} bytes, got {}",
            self.actual
        )
    }
}

impl std::error::Error for ROMSizeError {}

/// The CPU plus its on-chip peripherals. `Default` is a powered-off chip with
/// no ROM and a reset pending, so a snapshot from before the chip existed
/// deserializes into something that boots on its first step once
/// [`TMS7040::reattach_rom`] has run.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Clone, Debug)]
pub struct TMS7040 {
    pub pc: u16,
    /// Stack pointer into the register file (the stack lives in RF).
    pub sp: u8,
    /// Status register — see [`st`].
    pub st: u8,
    #[cfg_attr(feature = "serde", serde(with = "serde_util::byte_array"))]
    rf: [u8; REGISTER_FILE_SIZE],
    #[cfg_attr(feature = "serde", serde(skip))]
    rom: Box<[u8]>,
    /// IOCNT0: d0/d2/d4 INT1/2/3 enables, d1/d3/d5 their flags.
    io_control: u8,
    port_latch: [u8; 4],
    port_ddr: [u8; 4],
    timer1: Timer1,
    /// Last level told to [`Self::set_int1`] / [`Self::set_int3`].
    int_line: [bool; 2],
    /// The INTn Pulse flip-flop (SPND001B 3-31/3-33). `#[serde(default)]`:
    /// false reproduces a pre-latch snapshot's level-only behavior.
    #[cfg_attr(feature = "serde", serde(default))]
    pulse_latch: [bool; 2],
    /// Last level told to [`Self::set_ec1`]. `#[serde(default)]`: an old
    /// snapshot predates Event-Counter mode, and false (no pending edge)
    /// reproduces its behavior.
    #[cfg_attr(feature = "serde", serde(default))]
    ec1_line: bool,
    /// Parked on an IDLE instruction until an interrupt.
    idle: bool,
    /// The reset sequence runs on the next [`Self::step`].
    pending_reset: bool,
    /// Total cycles consumed by [`Self::step`] since construction.
    pub cycles: u64,
    /// Illegal opcodes executed, for tracing tools to assert on.
    pub illegal_count: u32,
    /// Cycles charged so far by the step in progress.
    #[cfg_attr(feature = "serde", serde(skip))]
    burned: u32,
}

impl Default for TMS7040 {
    fn default() -> Self {
        Self {
            pc: 0,
            sp: 0,
            st: 0,
            rf: [0; REGISTER_FILE_SIZE],
            rom: Box::default(),
            io_control: 0,
            port_latch: [0; 4],
            // MAME `device_start`: port B's DDR is hardwired to all-outputs.
            port_ddr: [0, 0xFF, 0, 0],
            timer1: Timer1::default(),
            int_line: [false; 2],
            pulse_latch: [false; 2],
            ec1_line: false,
            idle: false,
            pending_reset: true,
            cycles: 0,
            illegal_count: 0,
            burned: 0,
        }
    }
}

impl TMS7040 {
    /// A powered-on chip around its 4 KB ROM, with the reset sequence
    /// pending for the first [`Self::step`].
    pub fn new(rom: &[u8]) -> Result<Self, ROMSizeError> {
        let mut cpu = Self::default();
        cpu.reattach_rom(rom)?;
        Ok(cpu)
    }

    /// Restore-path: re-supply the ROM image a snapshot doesn't carry.
    pub fn reattach_rom(&mut self, rom: &[u8]) -> Result<(), ROMSizeError> {
        if rom.len() != ROM_SIZE {
            return Err(ROMSizeError { actual: rom.len() });
        }
        self.rom = rom.into();
        Ok(())
    }

    /// Whether the reset sequence will run on the next [`Self::step`].
    pub fn pending_reset(&self) -> bool {
        self.pending_reset
    }

    /// Restore-time payload-shape validation: invariants `Deserialize` can't
    /// check itself, catching a hand-crafted or corrupt snapshot before its
    /// fields drive `step` into a panic or a runaway loop.
    pub fn validate(&self) -> Result<(), &'static str> {
        self.timer1.validate()?;
        if self.port_ddr[Port::A.index()] != 0 {
            return Err("port A has no DDR and is hardwired all-input (0)");
        }
        if self.port_ddr[Port::B.index()] != 0xFF {
            return Err("port B's DDR is hardwired all-output (0xFF)");
        }
        if !self.ext_flags_are_consistent() {
            return Err("IOCNT0's INT1/INT3 flag bits disagree with the pulse-latch/level state");
        }
        Ok(())
    }

    /// Pull the RESET pin: the reset sequence runs on the next
    /// [`Self::step`] (as `Step { cycles: 17, kind: StepKind::Reset }`).
    pub fn assert_reset(&mut self) {
        self.pending_reset = true;
    }

    /// Accumulator A (register file 0).
    pub fn a(&self) -> u8 {
        self.rf[0]
    }

    /// Accumulator B (register file 1).
    pub fn b(&self) -> u8 {
        self.rf[1]
    }

    /// Register file entry `n`; entries past the 128-byte file read 0.
    pub fn rf(&self, n: u8) -> u8 {
        self.rf.get(usize::from(n)).copied().unwrap_or(0)
    }

    /// Set register file entry `n`; entries past the file are ignored.
    pub fn set_rf(&mut self, n: u8, val: u8) {
        if let Some(slot) = self.rf.get_mut(usize::from(n)) {
            *slot = val;
        }
    }

    /// IOCNT0 as the chip currently holds it.
    pub fn io_control(&self) -> u8 {
        self.io_control
    }

    /// Output latch of `port` (unmasked by its DDR).
    pub fn port_latch(&self, port: Port) -> u8 {
        self.port_latch[port.index()]
    }

    /// Data-direction register of `port` (1 = output).
    pub fn port_ddr(&self, port: Port) -> u8 {
        self.port_ddr[port.index()]
    }

    /// Parked on IDLE, waiting for an interrupt.
    pub fn is_idle(&self) -> bool {
        self.idle
    }

    /// Side-effect-free read for debuggers: register file, ROM, and the
    /// peripheral file's latched values; ports return their output latch.
    pub fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x007F => self.rf[usize::from(addr)],
            0x0080..=0x00FF => 0,
            0x0100..=0x010B => self.pf_peek((addr - PERIPHERAL_FILE_BASE) as u8),
            ROM_BASE..=0xFFFF => self.rom_byte(addr),
            _ => 0,
        }
    }
}

/// A minimal [`Bus`] for tests and tracing tools: settable input pins, a
/// log of every port write, and a flat 64 KB external memory.
#[derive(Clone, Debug)]
pub struct FlatBoard {
    /// Input pin levels returned by [`Bus::read_port`].
    pub inputs: [u8; 4],
    /// Every `(port, value)` write, in order.
    pub writes: Vec<(Port, u8)>,
    /// External memory for addresses the chip doesn't decode.
    pub ext: Box<[u8]>,
}

impl Default for FlatBoard {
    fn default() -> Self {
        Self {
            inputs: [0xFF; 4],
            writes: Vec::new(),
            ext: vec![0; 0x1_0000].into_boxed_slice(),
        }
    }
}

impl FlatBoard {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Bus for FlatBoard {
    fn read_port(&mut self, port: Port) -> u8 {
        self.inputs[port.index()]
    }

    fn write_port(&mut self, port: Port, val: u8) {
        self.writes.push((port, val));
    }

    fn read_ext(&mut self, addr: u16) -> u8 {
        self.ext[usize::from(addr)]
    }

    fn write_ext(&mut self, addr: u16, val: u8) {
        self.ext[usize::from(addr)] = val;
    }
}

#[cfg(test)]
#[path = "lib_test.rs"]
mod tests;
