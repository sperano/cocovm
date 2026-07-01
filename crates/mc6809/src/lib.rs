//! Motorola 6809E CPU core — bus-generic, dependency-light.
//!
//! There is no per-instruction JSON conformance suite for the 6809 (unlike the
//! 6502/68000). Validate via flexemu's `cputest.txt` self-checking program and by
//! trace-diffing against XRoar/MAME. See `DESIGN.md` §5.
//!
//! STATUS: skeleton. Registers, reset, the `Bus` seam, and a flat test bus are in
//! place; only a few opcodes are decoded. The full opcode set, the `$10`/`$11`
//! prefix pages, indexed addressing, and interrupts are TODO.

#![forbid(unsafe_code)]
#![allow(clippy::upper_case_acronyms)]

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

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

/// Base cycle count for PSH/PUL, before adding one cycle per byte transferred.
const PUSH_PULL_BASE_CYCLES: u32 = 5;

/// PSH/PUL register mask selecting only PC (bit 7) and CC (bit 0) — the FIRQ
/// interrupt stack frame.
const PC_CC_MASK: u8 = 0x81;

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
    pub s: u16,
    pub pc: u16,
    pub dp: u8,
    pub cc: u8,
    /// Total cycles executed since reset (for scheduling/debugging).
    pub cycles: u64,
    /// Running vs halted (SYNC/CWAI).
    pub state: State,
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
    }

    /// Deliver a non-maskable interrupt. Always serviced (full frame, sets I+F).
    pub fn nmi(&mut self, bus: &mut impl Bus) {
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

    /// Execute one instruction; returns the cycles it consumed.
    ///
    /// Partial decode: NOP, 8/16-bit load/store, LEA, and the full 8-bit ALU —
    /// arithmetic (ADD/ADC/SUB/SBC/CMP), logic (AND/OR/EOR/BIT), and
    /// read-modify-write (NEG/COM/LSR/ROR/ASR/ASL/ROL/DEC/INC/TST/CLR) for A, B,
    /// and memory — across immediate / direct / extended / indexed addressing
    /// (the full indexed postbyte is decoded by [`Self::ea_indexed`]); and the
    /// full branch set (short `Bcc`, long `LBcc`, `BRA`/`BRN`/`LBRA`/`LBRN`); and
    /// the subroutine / stack group (`JMP`, `JSR`, `BSR`/`LBSR`, `RTS`, `TFR`,
    /// `EXG`, `PSHS`/`PULS`/`PSHU`/`PULU`); and the 16-bit ALU / load / store set
    /// (`ADDD`/`SUBD`, `CMPD`/`CMPX`/`CMPY`/`CMPU`/`CMPS`, `LDX`/`LDY`/`LDU`/`LDS`
    /// and their stores) via the `$10`/`$11` prefix pages; the misc inherent ops
    /// (`ORCC`/`ANDCC`/`SEX`/`ABX`/`MUL`/`DAA`); and the interrupt/halt set
    /// (`SWI`/`SWI2`/`SWI3`, `RTI`, `CWAI`, `SYNC`). External interrupts are
    /// delivered via [`Self::irq`]/[`Self::firq`]/[`Self::nmi`]. This is the
    /// complete 6809 user-mode ISA; only a handful of illegal opcodes remain
    /// undecoded and are treated as 2-cycle NOPs during bring-up.
    pub fn step(&mut self, bus: &mut impl Bus) -> u32 {
        if self.state != State::Running {
            // Halted by SYNC/CWAI: burn an idle cycle until the machine delivers
            // an interrupt (via nmi/irq/firq) that resumes execution.
            self.cycles += 1;
            return 1;
        }
        let opcode = self.fetch_u8(bus);
        let cycles = match opcode {
            0x12 => 2, // NOP

            // ---- Branches -----------------------------------------------------
            // Offsets are relative to the PC *after* the operand is consumed.
            // Short Bcc: 8-bit signed offset, 3 cycles (taken or not). Long LBcc:
            // 16-bit offset, 5 cycles / 6 if taken. LBRA is always, 5 cycles.
            0x20..=0x2F => {
                let offset = self.fetch_u8(bus) as i8 as i16 as u16;
                if self.branch_taken(opcode) {
                    self.pc = self.pc.wrapping_add(offset);
                }
                3
            }
            0x16 => {
                // LBRA — unconditional, 16-bit offset
                let offset = self.fetch_u16(bus);
                self.pc = self.pc.wrapping_add(offset);
                5
            }
            0x10 => {
                // $10 prefix page: long conditional branches, plus the 16-bit ops
                // targeting Y/D/S. Prefixed ops cost one more cycle than their
                // base-page equivalents. SWI2 and others are still TODO.
                let op2 = self.fetch_u8(bus);
                match op2 {
                    0x21..=0x2F => {
                        let offset = self.fetch_u16(bus);
                        if self.branch_taken(op2) {
                            self.pc = self.pc.wrapping_add(offset);
                            6
                        } else {
                            5
                        }
                    }

                    // CMPD (result discarded)
                    0x83 => { let m = self.fetch_u16(bus);       self.sub16(self.d(), m); 5 }
                    0x93 => { let m = self.read_direct16(bus);   self.sub16(self.d(), m); 7 }
                    0xA3 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read_u16(ea); self.sub16(self.d(), m); 7 + ic }
                    0xB3 => { let m = self.read_extended16(bus); self.sub16(self.d(), m); 8 }
                    // CMPY
                    0x8C => { let m = self.fetch_u16(bus);       self.sub16(self.y, m); 5 }
                    0x9C => { let m = self.read_direct16(bus);   self.sub16(self.y, m); 7 }
                    0xAC => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read_u16(ea); self.sub16(self.y, m); 7 + ic }
                    0xBC => { let m = self.read_extended16(bus); self.sub16(self.y, m); 8 }

                    // LDY
                    0x8E => { let v = self.fetch_u16(bus);       self.y = v; self.set_nz16(v); 4 }
                    0x9E => { let v = self.read_direct16(bus);   self.y = v; self.set_nz16(v); 6 }
                    0xAE => { let (ea, ic) = self.ea_indexed(bus); let v = bus.read_u16(ea); self.y = v; self.set_nz16(v); 6 + ic }
                    0xBE => { let v = self.read_extended16(bus); self.y = v; self.set_nz16(v); 7 }
                    // STY
                    0x9F => { let ea = self.ea_direct(bus);      bus.write_u16(ea, self.y); self.set_nz16(self.y); 6 }
                    0xAF => { let (ea, ic) = self.ea_indexed(bus); bus.write_u16(ea, self.y); self.set_nz16(self.y); 6 + ic }
                    0xBF => { let ea = self.ea_extended(bus);    bus.write_u16(ea, self.y); self.set_nz16(self.y); 7 }

                    // LDS
                    0xCE => { let v = self.fetch_u16(bus);       self.s = v; self.set_nz16(v); 4 }
                    0xDE => { let v = self.read_direct16(bus);   self.s = v; self.set_nz16(v); 6 }
                    0xEE => { let (ea, ic) = self.ea_indexed(bus); let v = bus.read_u16(ea); self.s = v; self.set_nz16(v); 6 + ic }
                    0xFE => { let v = self.read_extended16(bus); self.s = v; self.set_nz16(v); 7 }
                    // STS
                    0xDF => { let ea = self.ea_direct(bus);      bus.write_u16(ea, self.s); self.set_nz16(self.s); 6 }
                    0xEF => { let (ea, ic) = self.ea_indexed(bus); bus.write_u16(ea, self.s); self.set_nz16(self.s); 6 + ic }
                    0xFF => { let ea = self.ea_extended(bus);    bus.write_u16(ea, self.s); self.set_nz16(self.s); 7 }

                    0x3F => { self.take_interrupt(bus, VECTOR_SWI2, false, false, true); 20 } // SWI2

                    _ => 2, // TODO: other $10-page opcodes
                }
            }
            0x11 => {
                // $11 prefix page: 16-bit compares of U and S (plus SWI3, TODO).
                let op2 = self.fetch_u8(bus);
                match op2 {
                    // CMPU
                    0x83 => { let m = self.fetch_u16(bus);       self.sub16(self.u, m); 5 }
                    0x93 => { let m = self.read_direct16(bus);   self.sub16(self.u, m); 7 }
                    0xA3 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read_u16(ea); self.sub16(self.u, m); 7 + ic }
                    0xB3 => { let m = self.read_extended16(bus); self.sub16(self.u, m); 8 }
                    // CMPS
                    0x8C => { let m = self.fetch_u16(bus);       self.sub16(self.s, m); 5 }
                    0x9C => { let m = self.read_direct16(bus);   self.sub16(self.s, m); 7 }
                    0xAC => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read_u16(ea); self.sub16(self.s, m); 7 + ic }
                    0xBC => { let m = self.read_extended16(bus); self.sub16(self.s, m); 8 }

                    0x3F => { self.take_interrupt(bus, VECTOR_SWI3, false, false, true); 20 } // SWI3

                    _ => 2, // TODO: other $11-page opcodes
                }
            }

            // ---- Subroutines, jumps, register transfer, stack -----------------
            // JMP arms MUST precede the RMW range arms below (0x0E/0x6E/0x7E would
            // otherwise be swallowed by 0x00-0x0F / 0x60-0x6F / 0x70-0x7F).

            // JMP — direct / indexed / extended
            0x0E => { let ea = self.ea_direct(bus); self.pc = ea; 3 }
            0x6E => { let (ea, ic) = self.ea_indexed(bus); self.pc = ea; 3 + ic }
            0x7E => { let ea = self.ea_extended(bus); self.pc = ea; 4 }

            // JSR — push return address, then jump
            0x9D => { let ea = self.ea_direct(bus); self.push16_s(bus, self.pc); self.pc = ea; 7 }
            0xAD => { let (ea, ic) = self.ea_indexed(bus); self.push16_s(bus, self.pc); self.pc = ea; 7 + ic }
            0xBD => { let ea = self.ea_extended(bus); self.push16_s(bus, self.pc); self.pc = ea; 8 }

            // BSR / LBSR — relative branch to subroutine
            0x8D => { let offset = self.fetch_u8(bus) as i8 as i16 as u16; self.push16_s(bus, self.pc); self.pc = self.pc.wrapping_add(offset); 7 }
            0x17 => { let offset = self.fetch_u16(bus); self.push16_s(bus, self.pc); self.pc = self.pc.wrapping_add(offset); 9 }

            // RTS — pull return address
            0x39 => { self.pc = self.pull16_s(bus); 5 }

            // TFR / EXG
            0x1F => { let pb = self.fetch_u8(bus); let v = self.tfr_value(pb >> 4, pb & 0x0F); self.reg_write(pb & 0x0F, v); 6 }
            0x1E => {
                let pb = self.fetch_u8(bus);
                let (r0, r1) = (pb >> 4, pb & 0x0F);
                let v0 = self.reg_read(r0);
                let v1 = self.reg_read(r1);
                self.reg_write(r0, v1);
                self.reg_write(r1, v0);
                6
            }

            // PSHS / PULS / PSHU / PULU
            0x34 => { let mask = self.fetch_u8(bus); self.psh(bus, mask, true) }
            0x36 => { let mask = self.fetch_u8(bus); self.psh(bus, mask, false) }
            0x35 => { let mask = self.fetch_u8(bus); self.pul(bus, mask, true) }
            0x37 => { let mask = self.fetch_u8(bus); self.pul(bus, mask, false) }

            // ---- CC manipulation, misc inherent -------------------------------
            0x1A => { let m = self.fetch_u8(bus); self.cc |= m; 3 }  // ORCC #i8
            0x1C => { let m = self.fetch_u8(bus); self.cc &= m; 3 }  // ANDCC #i8
            0x1D => { // SEX — sign-extend B into A; N,Z from D, V unaffected
                self.a = if self.b & 0x80 != 0 { 0xFF } else { 0x00 };
                let d = self.d();
                self.set_nz16_only(d);
                2
            }
            0x3A => { self.x = self.x.wrapping_add(self.b as u16); 3 } // ABX (B unsigned)
            0x3D => { // MUL — D = A*B unsigned; Z from result, C = bit 7 of B
                let product = self.a as u16 * self.b as u16;
                self.set_d(product);
                self.set_z16(product);
                self.set_carry(product & 0x0080 != 0);
                11
            }
            0x19 => self.daa(),

            // ---- Interrupt / halt ---------------------------------------------
            0x3F => { self.take_interrupt(bus, VECTOR_SWI, true, true, true); 19 }  // SWI
            0x3B => { // RTI — pull CC, then full frame if E set else PC only
                self.pul(bus, 0x01, true);
                if self.cc & cc::ENTIRE != 0 {
                    self.pul(bus, 0xFE, true);
                    15
                } else {
                    self.pul(bus, 0x80, true);
                    6
                }
            }
            0x3C => { // CWAI — clear CC bits, stack full frame, then halt
                let m = self.fetch_u8(bus);
                self.cc &= m;
                self.cc |= cc::ENTIRE;
                self.psh(bus, 0xFF, true);
                self.state = State::Waiting;
                22
            }
            0x13 => { self.state = State::Syncing; 2 } // SYNC — halt until interrupt

            // LDA — immediate / direct / extended
            0x86 => { let ea = self.fetch_u8(bus); self.a = ea; self.set_nz8(ea); 2 }
            0x96 => { let v = self.read_direct8(bus); self.a = v; self.set_nz8(v); 4 }
            0xB6 => { let v = self.read_extended8(bus); self.a = v; self.set_nz8(v); 5 }

            // LDB — immediate / direct / extended
            0xC6 => { let v = self.fetch_u8(bus); self.b = v; self.set_nz8(v); 2 }
            0xD6 => { let v = self.read_direct8(bus); self.b = v; self.set_nz8(v); 4 }
            0xF6 => { let v = self.read_extended8(bus); self.b = v; self.set_nz8(v); 5 }

            // STA — direct / extended
            0x97 => { let ea = self.ea_direct(bus); bus.write(ea, self.a); self.set_nz8(self.a); 4 }
            0xB7 => { let ea = self.ea_extended(bus); bus.write(ea, self.a); self.set_nz8(self.a); 5 }

            // STB — direct / extended
            0xD7 => { let ea = self.ea_direct(bus); bus.write(ea, self.b); self.set_nz8(self.b); 4 }
            0xF7 => { let ea = self.ea_extended(bus); bus.write(ea, self.b); self.set_nz8(self.b); 5 }

            // LDD — immediate / direct / extended
            0xCC => { let v = self.fetch_u16(bus); self.set_d(v); self.set_nz16(v); 3 }
            0xDC => { let ea = self.ea_direct(bus); let v = bus.read_u16(ea); self.set_d(v); self.set_nz16(v); 5 }
            0xFC => { let ea = self.ea_extended(bus); let v = bus.read_u16(ea); self.set_d(v); self.set_nz16(v); 6 }

            // STD — direct / extended
            0xDD => { let ea = self.ea_direct(bus); let v = self.d(); bus.write_u16(ea, v); self.set_nz16(v); 5 }
            0xFD => { let ea = self.ea_extended(bus); let v = self.d(); bus.write_u16(ea, v); self.set_nz16(v); 6 }

            // ---- 8-bit ALU (immediate / direct / extended) --------------------
            // Cycles: immediate 2, direct 4, extended 5. Carry-in for ADC/SBC is
            // the current C flag (cc::CARRY == 0x01, so masking yields 0 or 1).

            // ADDA
            0x8B => { let m = self.fetch_u8(bus);        self.a = self.add8(self.a, m, 0); 2 }
            0x9B => { let m = self.read_direct8(bus);    self.a = self.add8(self.a, m, 0); 4 }
            0xBB => { let m = self.read_extended8(bus);  self.a = self.add8(self.a, m, 0); 5 }
            // ADDB
            0xCB => { let m = self.fetch_u8(bus);        self.b = self.add8(self.b, m, 0); 2 }
            0xDB => { let m = self.read_direct8(bus);    self.b = self.add8(self.b, m, 0); 4 }
            0xFB => { let m = self.read_extended8(bus);  self.b = self.add8(self.b, m, 0); 5 }

            // ADCA
            0x89 => { let c = self.cc & cc::CARRY; let m = self.fetch_u8(bus);       self.a = self.add8(self.a, m, c); 2 }
            0x99 => { let c = self.cc & cc::CARRY; let m = self.read_direct8(bus);   self.a = self.add8(self.a, m, c); 4 }
            0xB9 => { let c = self.cc & cc::CARRY; let m = self.read_extended8(bus); self.a = self.add8(self.a, m, c); 5 }
            // ADCB
            0xC9 => { let c = self.cc & cc::CARRY; let m = self.fetch_u8(bus);       self.b = self.add8(self.b, m, c); 2 }
            0xD9 => { let c = self.cc & cc::CARRY; let m = self.read_direct8(bus);   self.b = self.add8(self.b, m, c); 4 }
            0xF9 => { let c = self.cc & cc::CARRY; let m = self.read_extended8(bus); self.b = self.add8(self.b, m, c); 5 }

            // SUBA
            0x80 => { let m = self.fetch_u8(bus);        self.a = self.sub8(self.a, m, 0); 2 }
            0x90 => { let m = self.read_direct8(bus);    self.a = self.sub8(self.a, m, 0); 4 }
            0xB0 => { let m = self.read_extended8(bus);  self.a = self.sub8(self.a, m, 0); 5 }
            // SUBB
            0xC0 => { let m = self.fetch_u8(bus);        self.b = self.sub8(self.b, m, 0); 2 }
            0xD0 => { let m = self.read_direct8(bus);    self.b = self.sub8(self.b, m, 0); 4 }
            0xF0 => { let m = self.read_extended8(bus);  self.b = self.sub8(self.b, m, 0); 5 }

            // SBCA
            0x82 => { let c = self.cc & cc::CARRY; let m = self.fetch_u8(bus);       self.a = self.sub8(self.a, m, c); 2 }
            0x92 => { let c = self.cc & cc::CARRY; let m = self.read_direct8(bus);   self.a = self.sub8(self.a, m, c); 4 }
            0xB2 => { let c = self.cc & cc::CARRY; let m = self.read_extended8(bus); self.a = self.sub8(self.a, m, c); 5 }
            // SBCB
            0xC2 => { let c = self.cc & cc::CARRY; let m = self.fetch_u8(bus);       self.b = self.sub8(self.b, m, c); 2 }
            0xD2 => { let c = self.cc & cc::CARRY; let m = self.read_direct8(bus);   self.b = self.sub8(self.b, m, c); 4 }
            0xF2 => { let c = self.cc & cc::CARRY; let m = self.read_extended8(bus); self.b = self.sub8(self.b, m, c); 5 }

            // CMPA (result discarded, flags only)
            0x81 => { let m = self.fetch_u8(bus);        self.sub8(self.a, m, 0); 2 }
            0x91 => { let m = self.read_direct8(bus);    self.sub8(self.a, m, 0); 4 }
            0xB1 => { let m = self.read_extended8(bus);  self.sub8(self.a, m, 0); 5 }
            // CMPB
            0xC1 => { let m = self.fetch_u8(bus);        self.sub8(self.b, m, 0); 2 }
            0xD1 => { let m = self.read_direct8(bus);    self.sub8(self.b, m, 0); 4 }
            0xF1 => { let m = self.read_extended8(bus);  self.sub8(self.b, m, 0); 5 }

            // ---- Indexed addressing (base cost + postbyte extra cycles) --------
            // 8-bit load/store base 4; 16-bit LDD/STD base 5; LEA base 4.

            // LEA — load effective address into a register.
            // LEAX/LEAY set Z from the result; LEAS/LEAU affect no flags.
            0x30 => { let (ea, ic) = self.ea_indexed(bus); self.x = ea; self.set_z16(ea); 4 + ic }
            0x31 => { let (ea, ic) = self.ea_indexed(bus); self.y = ea; self.set_z16(ea); 4 + ic }
            0x32 => { let (ea, ic) = self.ea_indexed(bus); self.s = ea; 4 + ic }
            0x33 => { let (ea, ic) = self.ea_indexed(bus); self.u = ea; 4 + ic }

            // LDA / LDB / STA / STB indexed
            0xA6 => { let (ea, ic) = self.ea_indexed(bus); let v = bus.read(ea); self.a = v; self.set_nz8(v); 4 + ic }
            0xE6 => { let (ea, ic) = self.ea_indexed(bus); let v = bus.read(ea); self.b = v; self.set_nz8(v); 4 + ic }
            0xA7 => { let (ea, ic) = self.ea_indexed(bus); bus.write(ea, self.a); self.set_nz8(self.a); 4 + ic }
            0xE7 => { let (ea, ic) = self.ea_indexed(bus); bus.write(ea, self.b); self.set_nz8(self.b); 4 + ic }

            // LDD / STD indexed
            0xEC => { let (ea, ic) = self.ea_indexed(bus); let v = bus.read_u16(ea); self.set_d(v); self.set_nz16(v); 5 + ic }
            0xED => { let (ea, ic) = self.ea_indexed(bus); let v = self.d(); bus.write_u16(ea, v); self.set_nz16(v); 5 + ic }

            // ADDA / ADDB indexed
            0xAB => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.a = self.add8(self.a, m, 0); 4 + ic }
            0xEB => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.b = self.add8(self.b, m, 0); 4 + ic }
            // ADCA / ADCB indexed
            0xA9 => { let c = self.cc & cc::CARRY; let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.a = self.add8(self.a, m, c); 4 + ic }
            0xE9 => { let c = self.cc & cc::CARRY; let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.b = self.add8(self.b, m, c); 4 + ic }
            // SUBA / SUBB indexed
            0xA0 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.a = self.sub8(self.a, m, 0); 4 + ic }
            0xE0 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.b = self.sub8(self.b, m, 0); 4 + ic }
            // SBCA / SBCB indexed
            0xA2 => { let c = self.cc & cc::CARRY; let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.a = self.sub8(self.a, m, c); 4 + ic }
            0xE2 => { let c = self.cc & cc::CARRY; let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.b = self.sub8(self.b, m, c); 4 + ic }
            // CMPA / CMPB indexed (result discarded)
            0xA1 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.sub8(self.a, m, 0); 4 + ic }
            0xE1 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.sub8(self.b, m, 0); 4 + ic }

            // ---- 8-bit logic (AND/OR/EOR/BIT) --------------------------------
            // N,Z from result; V cleared; C and H unaffected. BIT sets flags only.
            // Cycles: immediate 2, direct 4, indexed 4+, extended 5.

            // ANDA
            0x84 => { let m = self.fetch_u8(bus);        self.a &= m; self.set_nz8(self.a); 2 }
            0x94 => { let m = self.read_direct8(bus);    self.a &= m; self.set_nz8(self.a); 4 }
            0xA4 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.a &= m; self.set_nz8(self.a); 4 + ic }
            0xB4 => { let m = self.read_extended8(bus);  self.a &= m; self.set_nz8(self.a); 5 }
            // ANDB
            0xC4 => { let m = self.fetch_u8(bus);        self.b &= m; self.set_nz8(self.b); 2 }
            0xD4 => { let m = self.read_direct8(bus);    self.b &= m; self.set_nz8(self.b); 4 }
            0xE4 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.b &= m; self.set_nz8(self.b); 4 + ic }
            0xF4 => { let m = self.read_extended8(bus);  self.b &= m; self.set_nz8(self.b); 5 }

            // ORA
            0x8A => { let m = self.fetch_u8(bus);        self.a |= m; self.set_nz8(self.a); 2 }
            0x9A => { let m = self.read_direct8(bus);    self.a |= m; self.set_nz8(self.a); 4 }
            0xAA => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.a |= m; self.set_nz8(self.a); 4 + ic }
            0xBA => { let m = self.read_extended8(bus);  self.a |= m; self.set_nz8(self.a); 5 }
            // ORB
            0xCA => { let m = self.fetch_u8(bus);        self.b |= m; self.set_nz8(self.b); 2 }
            0xDA => { let m = self.read_direct8(bus);    self.b |= m; self.set_nz8(self.b); 4 }
            0xEA => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.b |= m; self.set_nz8(self.b); 4 + ic }
            0xFA => { let m = self.read_extended8(bus);  self.b |= m; self.set_nz8(self.b); 5 }

            // EORA
            0x88 => { let m = self.fetch_u8(bus);        self.a ^= m; self.set_nz8(self.a); 2 }
            0x98 => { let m = self.read_direct8(bus);    self.a ^= m; self.set_nz8(self.a); 4 }
            0xA8 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.a ^= m; self.set_nz8(self.a); 4 + ic }
            0xB8 => { let m = self.read_extended8(bus);  self.a ^= m; self.set_nz8(self.a); 5 }
            // EORB
            0xC8 => { let m = self.fetch_u8(bus);        self.b ^= m; self.set_nz8(self.b); 2 }
            0xD8 => { let m = self.read_direct8(bus);    self.b ^= m; self.set_nz8(self.b); 4 }
            0xE8 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.b ^= m; self.set_nz8(self.b); 4 + ic }
            0xF8 => { let m = self.read_extended8(bus);  self.b ^= m; self.set_nz8(self.b); 5 }

            // BITA (A AND m, discard result)
            0x85 => { let m = self.fetch_u8(bus);        self.set_nz8(self.a & m); 2 }
            0x95 => { let m = self.read_direct8(bus);    self.set_nz8(self.a & m); 4 }
            0xA5 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.set_nz8(self.a & m); 4 + ic }
            0xB5 => { let m = self.read_extended8(bus);  self.set_nz8(self.a & m); 5 }
            // BITB
            0xC5 => { let m = self.fetch_u8(bus);        self.set_nz8(self.b & m); 2 }
            0xD5 => { let m = self.read_direct8(bus);    self.set_nz8(self.b & m); 4 }
            0xE5 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.set_nz8(self.b & m); 4 + ic }
            0xF5 => { let m = self.read_extended8(bus);  self.set_nz8(self.b & m); 5 }

            // ---- 16-bit ALU / load / store (D, X, U) --------------------------
            // ADDD/SUBD affect N,Z,V,C. CMPX is sub16 discarded. LDX/LDU/STX/STU
            // set N,Z and clear V. Cycles: ADD/SUB/CMP imm 4/dir 6/idx 6+/ext 7;
            // LD imm 3/dir 5/idx 5+/ext 6; ST dir 5/idx 5+/ext 6.

            // ADDD
            0xC3 => { let m = self.fetch_u16(bus);       let r = self.add16(self.d(), m); self.set_d(r); 4 }
            0xD3 => { let m = self.read_direct16(bus);   let r = self.add16(self.d(), m); self.set_d(r); 6 }
            0xE3 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read_u16(ea); let r = self.add16(self.d(), m); self.set_d(r); 6 + ic }
            0xF3 => { let m = self.read_extended16(bus); let r = self.add16(self.d(), m); self.set_d(r); 7 }
            // SUBD
            0x83 => { let m = self.fetch_u16(bus);       let r = self.sub16(self.d(), m); self.set_d(r); 4 }
            0x93 => { let m = self.read_direct16(bus);   let r = self.sub16(self.d(), m); self.set_d(r); 6 }
            0xA3 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read_u16(ea); let r = self.sub16(self.d(), m); self.set_d(r); 6 + ic }
            0xB3 => { let m = self.read_extended16(bus); let r = self.sub16(self.d(), m); self.set_d(r); 7 }
            // CMPX (result discarded)
            0x8C => { let m = self.fetch_u16(bus);       self.sub16(self.x, m); 4 }
            0x9C => { let m = self.read_direct16(bus);   self.sub16(self.x, m); 6 }
            0xAC => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read_u16(ea); self.sub16(self.x, m); 6 + ic }
            0xBC => { let m = self.read_extended16(bus); self.sub16(self.x, m); 7 }

            // LDX
            0x8E => { let v = self.fetch_u16(bus);       self.x = v; self.set_nz16(v); 3 }
            0x9E => { let v = self.read_direct16(bus);   self.x = v; self.set_nz16(v); 5 }
            0xAE => { let (ea, ic) = self.ea_indexed(bus); let v = bus.read_u16(ea); self.x = v; self.set_nz16(v); 5 + ic }
            0xBE => { let v = self.read_extended16(bus); self.x = v; self.set_nz16(v); 6 }
            // STX
            0x9F => { let ea = self.ea_direct(bus);      bus.write_u16(ea, self.x); self.set_nz16(self.x); 5 }
            0xAF => { let (ea, ic) = self.ea_indexed(bus); bus.write_u16(ea, self.x); self.set_nz16(self.x); 5 + ic }
            0xBF => { let ea = self.ea_extended(bus);    bus.write_u16(ea, self.x); self.set_nz16(self.x); 6 }

            // LDU
            0xCE => { let v = self.fetch_u16(bus);       self.u = v; self.set_nz16(v); 3 }
            0xDE => { let v = self.read_direct16(bus);   self.u = v; self.set_nz16(v); 5 }
            0xEE => { let (ea, ic) = self.ea_indexed(bus); let v = bus.read_u16(ea); self.u = v; self.set_nz16(v); 5 + ic }
            0xFE => { let v = self.read_extended16(bus); self.u = v; self.set_nz16(v); 6 }
            // STU
            0xDF => { let ea = self.ea_direct(bus);      bus.write_u16(ea, self.u); self.set_nz16(self.u); 5 }
            0xEF => { let (ea, ic) = self.ea_indexed(bus); bus.write_u16(ea, self.u); self.set_nz16(self.u); 5 + ic }
            0xFF => { let ea = self.ea_extended(bus);    bus.write_u16(ea, self.u); self.set_nz16(self.u); 6 }

            // ---- 8-bit read-modify-write (NEG/COM/LSR/ROR/ASR/ASL/ROL/DEC/INC/TST/CLR)
            // Low nibble selects the op (see rmw_apply). Cycles: inherent 2,
            // direct 6, indexed 6+, extended 7. TST reads but never writes back.

            // Inherent — operate on accumulator A / B
            0x40..=0x4F => { let r = self.rmw_apply(opcode & 0x0F, self.a); self.a = r; 2 }
            0x50..=0x5F => { let r = self.rmw_apply(opcode & 0x0F, self.b); self.b = r; 2 }

            // Direct
            0x00..=0x0F => {
                let op = opcode & 0x0F;
                let ea = self.ea_direct(bus);
                let m = bus.read(ea);
                let r = self.rmw_apply(op, m);
                if op != 0x0D { bus.write(ea, r); } // TST: no write-back
                6
            }
            // Indexed
            0x60..=0x6F => {
                let op = opcode & 0x0F;
                let (ea, ic) = self.ea_indexed(bus);
                let m = bus.read(ea);
                let r = self.rmw_apply(op, m);
                if op != 0x0D { bus.write(ea, r); }
                6 + ic
            }
            // Extended
            0x70..=0x7F => {
                let op = opcode & 0x0F;
                let ea = self.ea_extended(bus);
                let m = bus.read(ea);
                let r = self.rmw_apply(op, m);
                if op != 0x0D { bus.write(ea, r); }
                7
            }

            _ => 2, // TODO: unimplemented opcode
        };
        self.cycles += cycles as u64;
        cycles
    }

    fn fetch_u8(&mut self, bus: &mut impl Bus) -> u8 {
        let v = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    fn fetch_u16(&mut self, bus: &mut impl Bus) -> u16 {
        let hi = self.fetch_u8(bus) as u16;
        let lo = self.fetch_u8(bus) as u16;
        (hi << 8) | lo
    }

    /// Direct-mode effective address: `DP:operand_byte`.
    fn ea_direct(&mut self, bus: &mut impl Bus) -> u16 {
        let lo = self.fetch_u8(bus) as u16;
        ((self.dp as u16) << 8) | lo
    }

    /// Extended-mode effective address: a 16-bit operand.
    fn ea_extended(&mut self, bus: &mut impl Bus) -> u16 {
        self.fetch_u16(bus)
    }

    /// One of the four index registers selected by a 2-bit postbyte field
    /// (00=X, 01=Y, 10=U, 11=S).
    fn index_reg(&self, sel: u8) -> u16 {
        match sel & 0b11 {
            0b00 => self.x,
            0b01 => self.y,
            0b10 => self.u,
            _ => self.s,
        }
    }

    fn set_index_reg(&mut self, sel: u8, val: u16) {
        match sel & 0b11 {
            0b00 => self.x = val,
            0b01 => self.y = val,
            0b10 => self.u = val,
            _ => self.s = val,
        }
    }

    /// Decode an indexed-addressing postbyte and return the effective address and
    /// the *extra* cycles it costs (added to the instruction's indexed base cost).
    /// A large fraction of instructions route through here (see `DESIGN.md` §5), so
    /// it aims to be a faithful transcription of the datasheet's indexed-mode table.
    ///
    /// Postbyte layout when bit 7 is set: `1 rr i mmmm` — `rr` selects the register,
    /// `i` is the indirect bit, `mmmm` the sub-mode. When bit 7 is clear the whole
    /// low 5 bits are a signed offset (`0 rr nnnnn`), which has no indirect form.
    fn ea_indexed(&mut self, bus: &mut impl Bus) -> (u16, u32) {
        use postbyte::*;
        let pb = self.fetch_u8(bus);

        // 5-bit signed constant offset — the only non-indirect-capable form.
        if pb & 0x80 == 0 {
            let reg = self.index_reg(pb >> REG_SHIFT);
            let n = pb & OFFSET5_MASK;
            let offset = if n & OFFSET5_SIGN != 0 {
                n as i16 - (OFFSET5_SIGN as i16 * 2)
            } else {
                n as i16
            };
            return (reg.wrapping_add(offset as u16), 1);
        }

        let sel = pb >> REG_SHIFT;
        let indirect = pb & INDIRECT != 0;
        let (mut ea, mut extra) = match pb & MODE_MASK {
            0b0000 => {
                // ,R+  (auto-increment by 1; not indirectable on real silicon)
                let r = self.index_reg(sel);
                self.set_index_reg(sel, r.wrapping_add(1));
                (r, 2)
            }
            0b0001 => {
                // ,R++  (auto-increment by 2)
                let r = self.index_reg(sel);
                self.set_index_reg(sel, r.wrapping_add(2));
                (r, 3)
            }
            0b0010 => {
                // ,-R  (auto-decrement by 1; not indirectable on real silicon)
                let r = self.index_reg(sel).wrapping_sub(1);
                self.set_index_reg(sel, r);
                (r, 2)
            }
            0b0011 => {
                // ,--R  (auto-decrement by 2)
                let r = self.index_reg(sel).wrapping_sub(2);
                self.set_index_reg(sel, r);
                (r, 3)
            }
            0b0100 => (self.index_reg(sel), 0), // ,R  (no offset)
            0b0101 => {
                // B,R  (signed accumulator offset)
                let ofs = self.b as i8 as i16 as u16;
                (self.index_reg(sel).wrapping_add(ofs), 1)
            }
            0b0110 => {
                // A,R
                let ofs = self.a as i8 as i16 as u16;
                (self.index_reg(sel).wrapping_add(ofs), 1)
            }
            0b1000 => {
                // n,R  (8-bit signed offset)
                let ofs = self.fetch_u8(bus) as i8 as i16 as u16;
                (self.index_reg(sel).wrapping_add(ofs), 1)
            }
            0b1001 => {
                // n,R  (16-bit offset)
                let ofs = self.fetch_u16(bus);
                (self.index_reg(sel).wrapping_add(ofs), 4)
            }
            0b1011 => {
                // D,R  (16-bit accumulator offset)
                let ofs = self.d();
                (self.index_reg(sel).wrapping_add(ofs), 4)
            }
            0b1100 => {
                // n,PCR  (8-bit signed offset from the *next* instruction)
                let ofs = self.fetch_u8(bus) as i8 as i16 as u16;
                (self.pc.wrapping_add(ofs), 1)
            }
            0b1101 => {
                // n,PCR  (16-bit offset)
                let ofs = self.fetch_u16(bus);
                (self.pc.wrapping_add(ofs), 5)
            }
            0b1111 => {
                // [n]  extended indirect (register field ignored). The base cost
                // here plus the indirect fetch below sum to the datasheet's 5.
                (self.fetch_u16(bus), 2)
            }
            // Reserved/illegal postbytes (0b0111, 0b1010, 0b1110): behaviour is
            // undefined on hardware; fall back to a plain register read.
            _ => (self.index_reg(sel), 0),
        };

        if indirect {
            ea = bus.read_u16(ea);
            extra += INDIRECT_CYCLES;
        }
        (ea, extra)
    }

    fn read_direct8(&mut self, bus: &mut impl Bus) -> u8 {
        let ea = self.ea_direct(bus);
        bus.read(ea)
    }

    fn read_extended8(&mut self, bus: &mut impl Bus) -> u8 {
        let ea = self.ea_extended(bus);
        bus.read(ea)
    }

    fn read_direct16(&mut self, bus: &mut impl Bus) -> u16 {
        let ea = self.ea_direct(bus);
        bus.read_u16(ea)
    }

    fn read_extended16(&mut self, bus: &mut impl Bus) -> u16 {
        let ea = self.ea_extended(bus);
        bus.read_u16(ea)
    }

    /// Set N and Z from an 8-bit result, leaving V, C, H untouched.
    fn set_nz8_only(&mut self, value: u8) {
        self.cc &= !(cc::NEGATIVE | cc::ZERO);
        if value == 0 {
            self.cc |= cc::ZERO;
        }
        if value & 0x80 != 0 {
            self.cc |= cc::NEGATIVE;
        }
    }

    fn set_carry(&mut self, on: bool) {
        if on {
            self.cc |= cc::CARRY;
        } else {
            self.cc &= !cc::CARRY;
        }
    }

    fn set_overflow(&mut self, on: bool) {
        if on {
            self.cc |= cc::OVERFLOW;
        } else {
            self.cc &= !cc::OVERFLOW;
        }
    }

    /// Set N and Z from an 8-bit result and clear V (the LD/ST/logic convention;
    /// C and H are left unaffected).
    fn set_nz8(&mut self, value: u8) {
        self.set_overflow(false);
        self.set_nz8_only(value);
    }

    // ---- 8-bit read-modify-write primitives -------------------------------
    // Each returns the transformed value and sets condition codes per the
    // MC6809 datasheet. Flag subtleties verified against the Motorola CC tables:
    // COM forces C=1; LSR forces N=0 and leaves V untouched; LSR/ASR/ROR leave V
    // unaffected; INC/DEC leave C unaffected.

    /// COM: one's complement. N,Z from result; V cleared; C set to 1.
    fn com8(&mut self, m: u8) -> u8 {
        let r = !m;
        self.set_overflow(false);
        self.set_nz8_only(r);
        self.cc |= cc::CARRY;
        r
    }

    /// CLR: N=0, Z=1, V=0, C=0.
    fn clr8(&mut self) -> u8 {
        self.set_overflow(false);
        self.set_carry(false);
        self.set_nz8_only(0);
        0
    }

    /// INC: N,Z from result; V set iff signed overflow ($7F→$80); C unaffected.
    fn inc8(&mut self, m: u8) -> u8 {
        let r = m.wrapping_add(1);
        self.set_overflow(m == 0x7F);
        self.set_nz8_only(r);
        r
    }

    /// DEC: N,Z from result; V set iff signed overflow ($80→$7F); C unaffected.
    fn dec8(&mut self, m: u8) -> u8 {
        let r = m.wrapping_sub(1);
        self.set_overflow(m == 0x80);
        self.set_nz8_only(r);
        r
    }

    /// LSR: 0→b7, b0→C. N always 0; Z from result; V unaffected.
    fn lsr8(&mut self, m: u8) -> u8 {
        let r = m >> 1;
        self.set_carry(m & 0x01 != 0);
        self.set_nz8_only(r);
        r
    }

    /// ASR: b7 preserved (sign), b0→C. N,Z from result; V unaffected.
    fn asr8(&mut self, m: u8) -> u8 {
        let r = (m >> 1) | (m & 0x80);
        self.set_carry(m & 0x01 != 0);
        self.set_nz8_only(r);
        r
    }

    /// ROR: C→b7, b0→C. N,Z from result; V unaffected.
    fn ror8(&mut self, m: u8) -> u8 {
        let carry_in = (self.cc & cc::CARRY) << 7;
        let r = (m >> 1) | carry_in;
        self.set_carry(m & 0x01 != 0);
        self.set_nz8_only(r);
        r
    }

    /// ASL/LSL: 0→b0, b7→C. N,Z from result; V = b7⊕b6 of the original operand.
    fn asl8(&mut self, m: u8) -> u8 {
        let r = m << 1;
        self.set_carry(m & 0x80 != 0);
        self.set_overflow((m ^ (m << 1)) & 0x80 != 0);
        self.set_nz8_only(r);
        r
    }

    /// ROL: C→b0, b7→C. N,Z from result; V = b7⊕b6 of the original operand.
    fn rol8(&mut self, m: u8) -> u8 {
        let carry_in = self.cc & cc::CARRY;
        let r = (m << 1) | carry_in;
        self.set_carry(m & 0x80 != 0);
        self.set_overflow((m ^ (m << 1)) & 0x80 != 0);
        self.set_nz8_only(r);
        r
    }

    /// Dispatch an 8-bit read-modify-write op by the opcode's low nibble and
    /// return the new value (flags set as a side effect). NEG(0), COM(3), LSR(4),
    /// ROR(6), ASR(7), ASL/LSL(8), ROL(9), DEC(A), INC(C), TST(D), CLR(F). TST
    /// returns its input unchanged (flags only — caller must not write it back);
    /// illegal nibbles (1,2,5,B,E) are no-ops.
    fn rmw_apply(&mut self, op_nibble: u8, m: u8) -> u8 {
        match op_nibble {
            0x0 => self.sub8(0, m, 0), // NEG is 0 - m
            0x3 => self.com8(m),
            0x4 => self.lsr8(m),
            0x6 => self.ror8(m),
            0x7 => self.asr8(m),
            0x8 => self.asl8(m),
            0x9 => self.rol8(m),
            0xA => self.dec8(m),
            0xC => self.inc8(m),
            0xD => {
                self.set_nz8(m); // TST: flags only
                m
            }
            0xF => self.clr8(),
            _ => m, // illegal nibble
        }
    }

    /// DAA — decimal-adjust A after a BCD addition. Correction rules and carry
    /// per the datasheet; V is left undefined (untouched here). Returns cycles.
    fn daa(&mut self) -> u32 {
        let a = self.a;
        let lsn = a & 0x0F;
        let msn = a >> 4;
        let mut corr = 0u8;
        if self.cc & cc::HALF_CARRY != 0 || lsn > 9 {
            corr |= 0x06;
        }
        if self.cc & cc::CARRY != 0 || msn > 9 || (msn > 8 && lsn > 9) {
            corr |= 0x60;
        }
        let result = a.wrapping_add(corr);
        self.a = result;
        self.set_carry(corr & 0x60 != 0);
        self.set_nz8_only(result);
        2
    }

    /// Evaluate a branch condition selected by the opcode's low nibble — shared
    /// by the short `Bcc` and long `LBcc` encodings (0=BRA always … F=BLE).
    /// Conditions verified against the Atkinson reference (see [[coco-reference-pdfs]]).
    fn branch_taken(&self, cond: u8) -> bool {
        let c = self.cc & cc::CARRY != 0;
        let z = self.cc & cc::ZERO != 0;
        let n = self.cc & cc::NEGATIVE != 0;
        let v = self.cc & cc::OVERFLOW != 0;
        match cond & 0x0F {
            0x0 => true,           // BRA — always
            0x1 => false,          // BRN — never
            0x2 => !c && !z,       // BHI — higher (unsigned)
            0x3 => c || z,         // BLS — lower or same (unsigned)
            0x4 => !c,             // BHS/BCC — carry clear
            0x5 => c,              // BLO/BCS — carry set
            0x6 => !z,             // BNE
            0x7 => z,              // BEQ
            0x8 => !v,             // BVC
            0x9 => v,              // BVS
            0xA => !n,             // BPL
            0xB => n,              // BMI
            0xC => n == v,         // BGE — >= (signed)
            0xD => n != v,         // BLT — <  (signed)
            0xE => !z && (n == v), // BGT — >  (signed)
            _ => z || (n != v),    // BLE — <= (signed)
        }
    }

    // ---- Register transfer (TFR/EXG) --------------------------------------
    // Postbyte nibble codes: 0=D 1=X 2=Y 3=U 4=S 5=PC 8=A 9=B A=CC B=DP.
    // Codes 0..5 are the 16-bit registers.

    fn reg_is16(code: u8) -> bool {
        code < 0x6
    }

    fn reg_read(&self, code: u8) -> u16 {
        match code {
            0x0 => self.d(),
            0x1 => self.x,
            0x2 => self.y,
            0x3 => self.u,
            0x4 => self.s,
            0x5 => self.pc,
            0x8 => self.a as u16,
            0x9 => self.b as u16,
            0xA => self.cc as u16,
            0xB => self.dp as u16,
            _ => 0xFFFF, // invalid on 6809
        }
    }

    fn reg_write(&mut self, code: u8, value: u16) {
        match code {
            0x0 => self.set_d(value),
            0x1 => self.x = value,
            0x2 => self.y = value,
            0x3 => self.u = value,
            0x4 => self.s = value,
            0x5 => self.pc = value,
            0x8 => self.a = value as u8,
            0x9 => self.b = value as u8,
            0xA => self.cc = value as u8,
            0xB => self.dp = value as u8,
            _ => {} // invalid
        }
    }

    /// The value TFR writes to `dst` given `src`, honouring the documented 6809
    /// behaviour when register sizes differ (16→8 keeps the LSB — handled by
    /// `reg_write`; A/B→16 sets MSB=$FF; CC/DP→16 duplicates the byte).
    fn tfr_value(&self, src: u8, dst: u8) -> u16 {
        let sv = self.reg_read(src);
        match (Self::reg_is16(src), Self::reg_is16(dst)) {
            (false, true) => {
                let b = sv & 0x00FF;
                match src {
                    0x8 | 0x9 => 0xFF00 | b, // A/B → 16: high byte = $FF
                    _ => (b << 8) | b,       // CC/DP → 16: both bytes = source
                }
            }
            _ => sv, // same size, or 16 → 8 (reg_write truncates to the LSB)
        }
    }

    // ---- Stack push/pull --------------------------------------------------

    /// Push PC (or any 16-bit value) onto the hardware (S) stack, big-endian.
    fn push16_s(&mut self, bus: &mut impl Bus, val: u16) {
        self.s = self.s.wrapping_sub(2);
        bus.write_u16(self.s, val);
    }

    /// Pull a 16-bit value from the hardware (S) stack.
    fn pull16_s(&mut self, bus: &mut impl Bus) -> u16 {
        let v = bus.read_u16(self.s);
        self.s = self.s.wrapping_add(2);
        v
    }

    /// PSHS/PSHU. `to_s` selects the hardware (S) stack; otherwise the user (U)
    /// stack. Push order is PC, U/S, Y, X, DP, B, A, CC (highest address first),
    /// so CC ends up on top. Bit 6 of the mask pushes the *other* stack pointer.
    /// Returns the cycle count (base + 1 per byte).
    fn psh(&mut self, bus: &mut impl Bus, mask: u8, to_s: bool) -> u32 {
        // Work on a local pointer; a 16-bit push stores low byte first (at the
        // higher address) then high byte, leaving the value big-endian in memory.
        let mut sp = if to_s { self.s } else { self.u };
        let other = if to_s { self.u } else { self.s };
        let mut push8 = |sp: &mut u16, v: u8, n: &mut u32| {
            *sp = sp.wrapping_sub(1);
            bus.write(*sp, v);
            *n += 1;
        };
        let mut bytes = 0u32;
        if mask & 0x80 != 0 { push8(&mut sp, self.pc as u8, &mut bytes); push8(&mut sp, (self.pc >> 8) as u8, &mut bytes); }
        if mask & 0x40 != 0 { push8(&mut sp, other as u8, &mut bytes); push8(&mut sp, (other >> 8) as u8, &mut bytes); }
        if mask & 0x20 != 0 { push8(&mut sp, self.y as u8, &mut bytes); push8(&mut sp, (self.y >> 8) as u8, &mut bytes); }
        if mask & 0x10 != 0 { push8(&mut sp, self.x as u8, &mut bytes); push8(&mut sp, (self.x >> 8) as u8, &mut bytes); }
        if mask & 0x08 != 0 { push8(&mut sp, self.dp, &mut bytes); }
        if mask & 0x04 != 0 { push8(&mut sp, self.b, &mut bytes); }
        if mask & 0x02 != 0 { push8(&mut sp, self.a, &mut bytes); }
        if mask & 0x01 != 0 { push8(&mut sp, self.cc, &mut bytes); }
        if to_s { self.s = sp; } else { self.u = sp; }
        PUSH_PULL_BASE_CYCLES + bytes
    }

    /// PULS/PULU — inverse of [`Self::psh`]. Pull order is CC, A, B, DP, X, Y,
    /// U/S, PC. Bit 6 pulls the *other* stack pointer.
    fn pul(&mut self, bus: &mut impl Bus, mask: u8, from_s: bool) -> u32 {
        let mut sp = if from_s { self.s } else { self.u };
        let mut bytes = 0u32;
        let mut pull8 = |sp: &mut u16, n: &mut u32| {
            let v = bus.read(*sp);
            *sp = sp.wrapping_add(1);
            *n += 1;
            v
        };
        if mask & 0x01 != 0 { self.cc = pull8(&mut sp, &mut bytes); }
        if mask & 0x02 != 0 { self.a = pull8(&mut sp, &mut bytes); }
        if mask & 0x04 != 0 { self.b = pull8(&mut sp, &mut bytes); }
        if mask & 0x08 != 0 { self.dp = pull8(&mut sp, &mut bytes); }
        if mask & 0x10 != 0 { let hi = pull8(&mut sp, &mut bytes); let lo = pull8(&mut sp, &mut bytes); self.x = ((hi as u16) << 8) | lo as u16; }
        if mask & 0x20 != 0 { let hi = pull8(&mut sp, &mut bytes); let lo = pull8(&mut sp, &mut bytes); self.y = ((hi as u16) << 8) | lo as u16; }
        if mask & 0x40 != 0 {
            let hi = pull8(&mut sp, &mut bytes);
            let lo = pull8(&mut sp, &mut bytes);
            let v = ((hi as u16) << 8) | lo as u16;
            if from_s { self.u = v; } else { self.s = v; }
        }
        if mask & 0x80 != 0 { let hi = pull8(&mut sp, &mut bytes); let lo = pull8(&mut sp, &mut bytes); self.pc = ((hi as u16) << 8) | lo as u16; }
        if from_s { self.s = sp; } else { self.u = sp; }
        PUSH_PULL_BASE_CYCLES + bytes
    }

    /// 8-bit add with carry-in: `a + m + carry_in`. Sets H, N, Z, V, C per the
    /// 6809 datasheet. Used by ADD (carry_in=0) and ADC (carry_in=C).
    ///
    /// - C: carry out of bit 7.
    /// - H: carry out of bit 3 (used by DAA).
    /// - V: signed overflow — operands share a sign that differs from the result.
    fn add8(&mut self, a: u8, m: u8, carry_in: u8) -> u8 {
        let sum = a as u16 + m as u16 + carry_in as u16;
        let r = sum as u8;
        let half = (a & 0x0F) + (m & 0x0F) + carry_in;
        self.cc &= !(cc::HALF_CARRY | cc::NEGATIVE | cc::ZERO | cc::OVERFLOW | cc::CARRY);
        if half > 0x0F {
            self.cc |= cc::HALF_CARRY;
        }
        if sum > 0xFF {
            self.cc |= cc::CARRY;
        }
        if (a ^ r) & (m ^ r) & 0x80 != 0 {
            self.cc |= cc::OVERFLOW;
        }
        if r & 0x80 != 0 {
            self.cc |= cc::NEGATIVE;
        }
        if r == 0 {
            self.cc |= cc::ZERO;
        }
        r
    }

    /// 8-bit subtract with borrow-in: `a - m - borrow_in`. Sets N, Z, V, C per the
    /// 6809 datasheet; H is left undefined (unaffected here). Used by SUB
    /// (borrow_in=0), SBC (borrow_in=C), and CMP (result discarded).
    ///
    /// - C: set on borrow (`a < m + borrow_in`).
    /// - V: signed overflow — minuend and subtrahend differ in sign and the result
    ///   sign differs from the minuend.
    fn sub8(&mut self, a: u8, m: u8, borrow_in: u8) -> u8 {
        let diff = (a as u16)
            .wrapping_sub(m as u16)
            .wrapping_sub(borrow_in as u16);
        let r = diff as u8;
        self.cc &= !(cc::NEGATIVE | cc::ZERO | cc::OVERFLOW | cc::CARRY);
        if diff & 0x100 != 0 {
            self.cc |= cc::CARRY;
        }
        if (a ^ m) & (a ^ r) & 0x80 != 0 {
            self.cc |= cc::OVERFLOW;
        }
        if r & 0x80 != 0 {
            self.cc |= cc::NEGATIVE;
        }
        if r == 0 {
            self.cc |= cc::ZERO;
        }
        r
    }

    /// Set Z (only) from a 16-bit result. Used by LEAX/LEAY, which touch no other
    /// condition codes.
    fn set_z16(&mut self, value: u16) {
        if value == 0 {
            self.cc |= cc::ZERO;
        } else {
            self.cc &= !cc::ZERO;
        }
    }

    /// Set N and Z from a 16-bit result, leaving V, C, H untouched.
    fn set_nz16_only(&mut self, value: u16) {
        self.cc &= !(cc::NEGATIVE | cc::ZERO);
        if value == 0 {
            self.cc |= cc::ZERO;
        }
        if value & 0x8000 != 0 {
            self.cc |= cc::NEGATIVE;
        }
    }

    /// Set N and Z from a 16-bit result and clear V (the LD/ST convention).
    fn set_nz16(&mut self, value: u16) {
        self.set_overflow(false);
        self.set_nz16_only(value);
    }

    /// 16-bit add (ADDD): `a + m`. Sets N, Z, V, C; H is unaffected.
    fn add16(&mut self, a: u16, m: u16) -> u16 {
        let sum = a as u32 + m as u32;
        let r = sum as u16;
        self.set_carry(sum > 0xFFFF);
        self.set_overflow((a ^ r) & (m ^ r) & 0x8000 != 0);
        self.set_nz16_only(r);
        r
    }

    /// 16-bit subtract (SUBD / CMPx): `a - m`. Sets N, Z, V, C.
    fn sub16(&mut self, a: u16, m: u16) -> u16 {
        let diff = (a as u32).wrapping_sub(m as u32);
        let r = diff as u16;
        self.set_carry(diff & 0x1_0000 != 0);
        self.set_overflow((a ^ m) & (a ^ r) & 0x8000 != 0);
        self.set_nz16_only(r);
        r
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
mod tests {
    use super::*;

    #[test]
    fn reset_loads_pc_from_vector_and_masks_interrupts() {
        let mut bus = FlatBus::new();
        bus.load(VECTOR_RESET, &[0x80, 0x00]); // reset vector -> $8000
        let mut cpu = MC6809::new();
        cpu.reset(&mut bus);
        assert_eq!(cpu.pc, 0x8000);
        assert_ne!(cpu.cc & cc::IRQ_MASK, 0);
        assert_ne!(cpu.cc & cc::FIRQ_MASK, 0);
    }

    #[test]
    fn lda_immediate_sets_a_and_zero_flag() {
        let mut bus = FlatBus::new();
        bus.load(0x0000, &[0x86, 0x00]); // LDA #$00
        let mut cpu = MC6809::new();
        let cycles = cpu.step(&mut bus);
        assert_eq!(cpu.a, 0x00);
        assert_eq!(cycles, 2);
        assert_ne!(cpu.cc & cc::ZERO, 0);
        assert_eq!(cpu.cc & cc::NEGATIVE, 0);
    }

    #[test]
    fn d_pairs_a_and_b() {
        let mut cpu = MC6809::new();
        cpu.set_d(0x1234);
        assert_eq!(cpu.a, 0x12);
        assert_eq!(cpu.b, 0x34);
        assert_eq!(cpu.d(), 0x1234);
    }
}
