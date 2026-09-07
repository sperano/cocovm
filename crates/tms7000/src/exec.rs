//! Instruction stepping, the reset sequence, flag helpers, and the irregular
//! single-byte opcodes (MAME `execute_run`, `device_reset`, and the `nop`
//! .. `setc` handlers in `tms7000op.cpp`). Addressing-mode grid ops live in
//! `exec/modes.rs` and `exec/ops.rs`, extended ops in `exec/extended.rs`.

mod extended;
mod modes;
mod ops;

use crate::decode::{Cond, Decoded, TABLE};
use crate::peripheral::pf;
use crate::{Bus, Step, StepKind, TMS7040, st};

/// Cycle costs of the irregular single-byte opcodes (MAME `tms7000op.cpp`).
mod cost {
    pub const NOP: u32 = 5;
    pub const IDLE: u32 = 6;
    pub const EINT: u32 = 5;
    pub const DINT: u32 = 5;
    pub const SETC: u32 = 5;
    pub const LDSP: u32 = 5;
    pub const STSP: u32 = 6;
    pub const PUSH_ST: u32 = 6;
    pub const POP_ST: u32 = 6;
    pub const RETS: u32 = 7;
    pub const RETI: u32 = 9;
    pub const TRAP: u32 = 14;
    /// MAME's guess for an illegal opcode.
    pub const ILLEGAL: u32 = 5;
    /// The reset sequence: TRAP 0 plus 3 (MAME `device_reset`: "17 total").
    pub const RESET_EXTRA: u32 = 3;
}

/// Bits of the status register RETI/POP ST restore (the low nibble is dropped).
const ST_RESTORE_MASK: u8 = 0xF0;

impl TMS7040 {
    /// Run one instruction, or take one interrupt, or run the pending reset
    /// sequence; the on-chip timer advances by the cycles consumed.
    pub fn step(&mut self, bus: &mut impl Bus) -> Step {
        self.burned = 0;
        if self.pending_reset {
            self.reset(bus);
            return self.finish(StepKind::Reset);
        }
        if let Some(line) = self.check_interrupts(bus) {
            return self.finish(StepKind::Interrupt(line));
        }
        let op = self.imm8(bus);
        self.execute(bus, op);
        self.finish(StepKind::Instruction)
    }

    fn finish(&mut self, kind: StepKind) -> Step {
        let cycles = self.burned;
        // Diagnostic-only running total; saturate instead of overflow-panicking
        // if a deserialized snapshot carried a cycles value near u64::MAX.
        self.cycles = self.cycles.saturating_add(u64::from(cycles));
        if self.timer1.tick(cycles) {
            self.flag_timer_interrupt();
        }
        Step { cycles, kind }
    }

    /// The RESET sequence (MAME `device_reset`), run by [`Self::step`] when
    /// a reset is pending so its 17 cycles are accounted: IOCNT0 clears
    /// first (SPND001B 3.6.1) so the port writes that follow always land
    /// on-chip regardless of the memory mode reset found the chip in, not
    /// as external bus cycles; the board sees port B all-ones and ports
    /// C/D all-zeros; SP is `$FF`, and TRAP 0 fetches the vector — pushing
    /// the old PC into R0/R1.
    pub(crate) fn reset(&mut self, bus: &mut impl Bus) {
        self.pending_reset = false;
        if self.idle {
            self.pc = self.pc.wrapping_add(1);
            self.idle = false;
        }
        self.st = 0;
        self.write_p(bus, pf::IOCNT0, 0x00);
        self.write_p(bus, pf::APORT, 0xFF);
        self.write_p(bus, pf::BPORT, 0xFF);
        self.write_p(bus, pf::ADDR, 0x00);
        self.write_p(bus, pf::CDDR, 0x00);
        self.write_p(bus, pf::DDDR, 0x00);
        // NMOS parts also drive ports C and D during reset.
        self.write_p(bus, pf::CPORT, 0xFF);
        self.write_p(bus, pf::DPORT, 0xFF);

        self.sp = 0xFF;
        self.execute(bus, 0xFF);
        self.burn(cost::RESET_EXTRA);
    }

    pub(crate) fn burn(&mut self, cycles: u32) {
        self.burned += cycles;
    }

    // ---- Flags (MAME `SET_C`/`SET_NZ`/`SET_CNZ` on a 16-bit intermediate) --

    pub(crate) fn carry(&self) -> u8 {
        self.st >> 7
    }

    /// C from bit 8 of `x`.
    pub(crate) fn set_c(&mut self, x: u16) {
        self.st = (self.st & !st::C) | ((x >> 1) & 0x80) as u8;
    }

    /// N from bit 7 of `x`, Z from its low byte.
    pub(crate) fn set_nz(&mut self, x: u16) {
        let z = if x & 0xFF == 0 { st::Z } else { 0 };
        self.st = (self.st & !(st::N | st::Z)) | ((x >> 1) & 0x40) as u8 | z;
    }

    /// C from bit 8, N from bit 7, Z from the low byte of `x`.
    pub(crate) fn set_cnz(&mut self, x: u16) {
        let z = if x & 0xFF == 0 { st::Z } else { 0 };
        self.st = (self.st & !(st::C | st::N | st::Z)) | ((x >> 1) & 0xC0) as u8 | z;
    }

    // ---- Dispatch -----------------------------------------------------------

    fn execute(&mut self, bus: &mut impl Bus, op: u8) {
        match TABLE[usize::from(op)] {
            Decoded::Am(mode, op) => self.exec_am(bus, mode, op),
            Decoded::Nop => self.burn(cost::NOP),
            Decoded::Idle => self.idle(),
            Decoded::Eint => {
                self.burn(cost::EINT);
                self.st |= st::N | st::Z | st::C | st::I;
            }
            Decoded::Dint => {
                self.burn(cost::DINT);
                self.st &= !(st::N | st::Z | st::C | st::I);
            }
            Decoded::Setc => {
                self.burn(cost::SETC);
                self.st = (self.st & !st::N) | st::C | st::Z;
            }
            Decoded::Ldsp => {
                self.burn(cost::LDSP);
                self.sp = self.rf[1];
            }
            Decoded::Stsp => {
                self.burn(cost::STSP);
                self.rf[1] = self.sp;
            }
            Decoded::PushSt => {
                self.burn(cost::PUSH_ST);
                self.push8(bus, self.st);
            }
            Decoded::PopSt => {
                self.burn(cost::POP_ST);
                self.st = self.pull8(bus) & ST_RESTORE_MASK;
            }
            Decoded::Rets => {
                self.burn(cost::RETS);
                self.pc = self.pull16(bus);
            }
            Decoded::Reti => {
                self.burn(cost::RETI);
                self.pc = self.pull16(bus);
                self.st = self.pull8(bus) & ST_RESTORE_MASK;
            }
            Decoded::Jmp(cond) => self.jmp(bus, self.cond(cond)),
            Decoded::Trap(op) => self.trap(bus, op << 1),
            Decoded::Illegal => {
                self.burn(cost::ILLEGAL);
                self.illegal_count = self.illegal_count.saturating_add(1);
            }
            other => self.exec_extended(bus, other),
        }
    }

    /// IDLE re-executes itself every 6 cycles until an interrupt (MAME backs
    /// the PC up onto the opcode).
    fn idle(&mut self) {
        self.burn(cost::IDLE);
        self.pc = self.pc.wrapping_sub(1);
        self.idle = true;
    }

    fn cond(&self, cond: Cond) -> bool {
        match cond {
            Cond::Always => true,
            Cond::N => self.st & st::N != 0,
            Cond::Z => self.st & st::Z != 0,
            Cond::C => self.st & st::C != 0,
            Cond::Positive => self.st & (st::Z | st::N) == 0,
            Cond::NotN => self.st & st::N == 0,
            Cond::NotZ => self.st & st::Z == 0,
            Cond::NotC => self.st & st::C == 0,
        }
    }

    /// `TRAP`: push the return address and vector through `$FF00 | address`.
    pub(crate) fn trap(&mut self, bus: &mut impl Bus, address: u8) {
        self.burn(cost::TRAP);
        self.push16(bus, self.pc);
        self.pc = self.read_mem16(bus, 0xFF00 | u16::from(address));
    }
}
