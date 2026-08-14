//! Motorola 6809E CPU core — bus-generic, dependency-light.
//!
//! There is no per-instruction JSON conformance suite for the 6809 (unlike the
//! 6502/68000). Validate via flexemu's `cputest.txt` self-checking program and by
//! trace-diffing against XRoar/MAME. See `DESIGN.md` §5.
//!
//! The complete documented 6809 user-mode ISA is implemented — every
//! documented instruction including the `$10`/`$11` prefix pages, all
//! addressing modes, and the interrupt set (`SWI`/`SWI2`/`SWI3`, `RTI`,
//! `CWAI`, `SYNC`, plus external NMI/IRQ/FIRQ delivery) — with
//! per-instruction cycle counts ([`MC6809::step`]). Undecoded illegal
//! opcodes execute as 2-cycle NOPs.

#![forbid(unsafe_code)]

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub mod disasm;

mod addressing;
mod alu;
mod branch;
mod exec;
mod regs;
mod stack;

/// The CPU's view of the outside world.
///
/// `read` takes `&mut self` deliberately: reads can have side effects (PIA flags,
/// GIME status registers clear-on-read). See `DESIGN.md` §2a.
pub trait Bus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);

    /// Big-endian 16-bit read (6809 is big-endian).
    fn read_u16(&mut self, addr: u16) -> u16 {
        let hi = self.read(addr) as u16;
        let lo = self.read(addr.wrapping_add(1)) as u16;
        (hi << 8) | lo
    }

    /// Big-endian 16-bit write.
    fn write_u16(&mut self, addr: u16, val: u16) {
        self.write(addr, (val >> 8) as u8);
        self.write(addr.wrapping_add(1), val as u8);
    }
}

/// Condition Code register bit masks. CC = `E F H I N Z V C`.
pub mod cc {
    pub const CARRY: u8 = 0x01;
    pub const OVERFLOW: u8 = 0x02;
    pub const ZERO: u8 = 0x04;
    pub const NEGATIVE: u8 = 0x08;
    pub const IRQ_MASK: u8 = 0x10;
    pub const HALF_CARRY: u8 = 0x20;
    pub const FIRQ_MASK: u8 = 0x40;
    pub const ENTIRE: u8 = 0x80;
}

/// Register-selector nibble codes used by the TFR/EXG postbyte and by
/// [`crate::MC6809::reg_read`]/[`crate::MC6809::reg_write`]/[`crate::MC6809::tfr_value`].
/// Codes `D..=PC` (0x0-0x5) name the 16-bit registers; `A..=DP` (0x8-0xB) the
/// 8-bit ones.
mod regsel {
    pub const D: u8 = 0x0;
    pub const X: u8 = 0x1;
    pub const Y: u8 = 0x2;
    pub const U: u8 = 0x3;
    pub const S: u8 = 0x4;
    pub const PC: u8 = 0x5;
    pub const A: u8 = 0x8;
    pub const B: u8 = 0x9;
    pub const CC: u8 = 0xA;
    pub const DP: u8 = 0xB;
}

/// Field masks for the indexed-addressing postbyte (`1 rr i mmmm`).
mod postbyte {
    /// Indirect bit.
    pub const INDIRECT: u8 = 0x10;
    /// Shift to bring the 2-bit register selector (bits 5-6) to the low bits.
    pub const REG_SHIFT: u8 = 5;
    /// Sub-mode field (low nibble), valid only when bit 7 is set.
    pub const MODE_MASK: u8 = 0x0F;
    /// 5-bit constant-offset field, valid only when bit 7 is clear.
    pub const OFFSET5_MASK: u8 = 0x1F;
    /// Sign bit of the 5-bit offset.
    pub const OFFSET5_SIGN: u8 = 0x10;
    /// Extra cycles for an indirect fetch (the `[...]` forms).
    pub const INDIRECT_CYCLES: u32 = 3;
}

/// Field masks for the PSH/PUL register-mask postbyte. Each bit selects one
/// register (or register pair) to push/pull; see [`crate::MC6809::psh`] and
/// [`crate::MC6809::pul`] for the transfer order.
mod stack_mask {
    pub const CC: u8 = 0x01;
    pub const A: u8 = 0x02;
    pub const B: u8 = 0x04;
    pub const DP: u8 = 0x08;
    pub const X: u8 = 0x10;
    pub const Y: u8 = 0x20;
    /// The *other* stack pointer: U when pushing/pulling S, S when pushing/pulling U.
    pub const OTHER_STACK_PTR: u8 = 0x40;
    pub const PC: u8 = 0x80;
}

/// Base cycle count for PSH/PUL, before adding one cycle per byte transferred.
const PUSH_PULL_BASE_CYCLES: u32 = 5;

/// PSH/PUL register mask selecting only PC and CC — the FIRQ interrupt stack frame.
const PC_CC_MASK: u8 = stack_mask::PC | stack_mask::CC;

/// Hardware interrupt / exception vectors (top of the address space).
pub const VECTOR_SWI3: u16 = 0xFFF2;
pub const VECTOR_SWI2: u16 = 0xFFF4;
pub const VECTOR_FIRQ: u16 = 0xFFF6;
pub const VECTOR_IRQ: u16 = 0xFFF8;
pub const VECTOR_SWI: u16 = 0xFFFA;
pub const VECTOR_NMI: u16 = 0xFFFC;
/// RESET vector address (`$FFFE`/`$FFFF`).
pub const VECTOR_RESET: u16 = 0xFFFE;

/// Execution state. The 6809 can halt itself waiting for an interrupt.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    #[default]
    Running,
    /// `SYNC`: halted until any interrupt line asserts; resumes with the next
    /// instruction (or services the interrupt if it is unmasked).
    Syncing,
    /// `CWAI`: the full register frame is already stacked; halted until an
    /// unmasked interrupt, which is then serviced without re-stacking.
    Waiting,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Default)]
pub struct MC6809 {
    pub a: u8,
    pub b: u8,
    pub x: u16,
    pub y: u16,
    pub u: u16,
    /// Hardware stack pointer. Writing this field directly bypasses NMI
    /// arming ([`Self::nmi_armed`]); use [`Self::load_s`] for a write that
    /// should count as the program's stack setup.
    pub s: u16,
    pub pc: u16,
    pub dp: u8,
    pub cc: u8,
    /// Total cycles executed since reset (for scheduling/debugging).
    pub cycles: u64,
    /// Running vs halted (SYNC/CWAI).
    pub state: State,
    /// NMI is not recognized until the first program load of the stack
    /// pointer after reset (MC6809 programming manual §1.11.10.1) — before S
    /// is valid an NMI frame push would scribble through a garbage pointer.
    /// Cleared by reset; set by every path that routes through
    /// [`Self::load_s`]: LDS, LEAS, TFR/EXG into S, PULU with the S bit, and
    /// indexed auto-inc/dec writeback through S. The S arithmetic inside
    /// PSHS/PULS, RTS/RTI, JSR, and interrupt frames does not arm.
    pub nmi_armed: bool,
}

impl MC6809 {
    pub fn new() -> Self {
        Self::default()
    }

    /// Accumulator `D` is the `A:B` pair (A high, B low).
    pub fn d(&self) -> u16 {
        ((self.a as u16) << 8) | self.b as u16
    }

    pub fn set_d(&mut self, value: u16) {
        self.a = (value >> 8) as u8;
        self.b = value as u8;
    }

    /// RESET: DP=0, IRQ+FIRQ masked, PC loaded from the reset vector.
    pub fn reset(&mut self, bus: &mut impl Bus) {
        self.dp = 0;
        self.cc |= cc::IRQ_MASK | cc::FIRQ_MASK;
        self.pc = bus.read_u16(VECTOR_RESET);
        self.state = State::Running;
        self.nmi_armed = false;
    }

    /// Load the stack pointer as a program action, arming NMI recognition
    /// ([`Self::nmi_armed`]). External writers (e.g. the debugger's register
    /// editor) should use this rather than writing `s` directly.
    //
    // The manual doesn't enumerate which instructions count as "loading" S
    // and the references disagree (XRoar arms on any S write including
    // PSHS/PULS; MAME only on LDS/LEAS/TFR into S): our arming set is
    // XRoar's minus the push/pull pointer-movement cases, a superset of
    // MAME's.
    pub fn load_s(&mut self, v: u16) {
        self.s = v;
        self.nmi_armed = true;
    }

    /// Deliver a non-maskable interrupt: full frame, sets I+F. Ignored until
    /// the first program load of S arms recognition (see `nmi_armed`).
    pub fn nmi(&mut self, bus: &mut impl Bus) {
        if !self.nmi_armed {
            return;
        }
        self.take_interrupt(bus, VECTOR_NMI, true, true, true);
    }

    /// Deliver an IRQ. Ignored (returns `false`) while the I mask is set — but a
    /// masked line still wakes a `SYNC`. Returns `true` if serviced.
    pub fn irq(&mut self, bus: &mut impl Bus) -> bool {
        if self.cc & cc::IRQ_MASK != 0 {
            if self.state == State::Syncing {
                self.state = State::Running;
            }
            return false;
        }
        self.take_interrupt(bus, VECTOR_IRQ, true, false, true);
        true
    }

    /// Deliver a FIRQ. Ignored while the F mask is set (but wakes a `SYNC`).
    /// Uses the fast partial frame (CC+PC only) and sets both I and F.
    pub fn firq(&mut self, bus: &mut impl Bus) -> bool {
        if self.cc & cc::FIRQ_MASK != 0 {
            if self.state == State::Syncing {
                self.state = State::Running;
            }
            return false;
        }
        self.take_interrupt(bus, VECTOR_FIRQ, true, true, false);
        true
    }

    /// Common interrupt sequence: stack the frame (unless `CWAI` already did),
    /// set the requested masks, and vector. `entire` selects the full frame (E=1)
    /// vs the FIRQ partial frame (E=0).
    fn take_interrupt(
        &mut self,
        bus: &mut impl Bus,
        vector: u16,
        set_i: bool,
        set_f: bool,
        entire: bool,
    ) {
        if self.state != State::Waiting {
            if entire {
                self.cc |= cc::ENTIRE;
                self.psh(bus, 0xFF, true);
            } else {
                self.cc &= !cc::ENTIRE;
                self.psh(bus, PC_CC_MASK, true);
            }
        }
        if set_i {
            self.cc |= cc::IRQ_MASK;
        }
        if set_f {
            self.cc |= cc::FIRQ_MASK;
        }
        self.pc = bus.read_u16(vector);
        self.state = State::Running;
    }
}

/// A flat 64K address space — for unit tests and the flexemu `cputest` harness.
pub struct FlatBus {
    pub mem: Box<[u8; 0x10000]>,
}

impl FlatBus {
    pub fn new() -> Self {
        Self {
            mem: Box::new([0u8; 0x10000]),
        }
    }

    pub fn load(&mut self, addr: u16, bytes: &[u8]) {
        let start = addr as usize;
        self.mem[start..start + bytes.len()].copy_from_slice(bytes);
    }
}

impl Default for FlatBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for FlatBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.mem[addr as usize]
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.mem[addr as usize] = val;
    }
}

#[cfg(test)]
#[path = "lib_test.rs"]
mod tests;
