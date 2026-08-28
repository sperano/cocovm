//! The opcode dispatcher: [`MC6809::step`] and the per-family execution groups
//! it delegates to. The top-level match in `step` is the authoritative opcode
//! map (mirrored byte-for-byte by [`crate::disasm`]); each family function
//! below owns one contiguous doc-comment section of that map (addressing-mode
//! row or operation family) so no single function has to hold the whole ISA
//! in view at once.

use crate::{Bus, CWAI_STACK_CYCLES, MC6809, State, VECTOR_SWI, VECTOR_SWI2, VECTOR_SWI3, cc};

mod exec_data;

impl MC6809 {
    /// Executes one instruction; returns the cycles it consumed. External
    /// interrupts are delivered via [`Self::irq`]/[`Self::firq`]/[`Self::nmi`].
    pub fn step(&mut self, bus: &mut impl Bus) -> u32 {
        if self.state != State::Running {
            // Halted by SYNC/CWAI: burn an idle cycle until an interrupt resumes execution.
            self.cycles += 1;
            return 1;
        }
        let opcode = self.fetch_u8(bus);
        let cycles = match opcode {
            0x12 => 2, // NOP

            // ---- Branches: offset is relative to the PC after the operand is fetched ----
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
            // $10/$11 prefix pages: long conditional branches, 16-bit Y/D/S/U ops, SWI2/SWI3.
            0x10 => self.exec_page10(bus),
            0x11 => self.exec_page11(bus),

            // ---- Subroutines/jumps/stack: JMP arms must precede the RMW ranges below
            // or they'd be swallowed ----
            0x0E | 0x6E | 0x7E | 0x9D | 0xAD | 0xBD | 0x8D | 0x17 | 0x39 | 0x1F | 0x1E | 0x34
            | 0x36 | 0x35 | 0x37 => self.exec_control_transfer(bus, opcode),

            // ---- CC manipulation, misc inherent -------------------------------
            0x1A | 0x1C | 0x1D | 0x3A | 0x3D | 0x19 => self.exec_misc_inherent(bus, opcode),

            // ---- Interrupt / halt ---------------------------------------------
            0x3F | 0x3B | 0x3C | 0x13 => self.exec_interrupt_halt(bus, opcode),

            // LDA/LDB/STA/STB/LDD/STD — immediate / direct / extended
            0x86 | 0x96 | 0xB6 | 0xC6 | 0xD6 | 0xF6 | 0x97 | 0xB7 | 0xD7 | 0xF7 | 0xCC | 0xDC
            | 0xFC | 0xDD | 0xFD => self.exec_load_store(bus, opcode),

            // ---- 8-bit ALU: carry-in for ADC/SBC is cc::CARRY masked to 0/1 ----
            0x8B | 0x9B | 0xBB | 0xCB | 0xDB | 0xFB | 0x89 | 0x99 | 0xB9 | 0xC9 | 0xD9 | 0xF9
            | 0x80 | 0x90 | 0xB0 | 0xC0 | 0xD0 | 0xF0 | 0x82 | 0x92 | 0xB2 | 0xC2 | 0xD2 | 0xF2
            | 0x81 | 0x91 | 0xB1 | 0xC1 | 0xD1 | 0xF1 => self.exec_alu8(bus, opcode),

            // ---- Indexed: cost is base + postbyte extra cycles ----
            0x30 | 0x31 | 0x32 | 0x33 | 0xA6 | 0xE6 | 0xA7 | 0xE7 | 0xEC | 0xED | 0xAB | 0xEB
            | 0xA9 | 0xE9 | 0xA0 | 0xE0 | 0xA2 | 0xE2 | 0xA1 | 0xE1 => {
                self.exec_indexed(bus, opcode)
            }

            // ---- 8-bit logic (AND/OR/EOR/BIT): N,Z from result, V cleared,
            // C/H unaffected; BIT is flags-only ----
            0x84 | 0x94 | 0xA4 | 0xB4 | 0xC4 | 0xD4 | 0xE4 | 0xF4 | 0x8A | 0x9A | 0xAA | 0xBA
            | 0xCA | 0xDA | 0xEA | 0xFA | 0x88 | 0x98 | 0xA8 | 0xB8 | 0xC8 | 0xD8 | 0xE8 | 0xF8
            | 0x85 | 0x95 | 0xA5 | 0xB5 | 0xC5 | 0xD5 | 0xE5 | 0xF5 => {
                self.exec_logic8(bus, opcode)
            }

            // ---- 16-bit ALU/ld/st (D,X,U): ADDD/SUBD affect N,Z,V,C; CMPX discards result;
            // LDx/STx set N,Z, clear V ----
            0xC3 | 0xD3 | 0xE3 | 0xF3 | 0x83 | 0x93 | 0xA3 | 0xB3 | 0x8C | 0x9C | 0xAC | 0xBC
            | 0x8E | 0x9E | 0xAE | 0xBE | 0x9F | 0xAF | 0xBF | 0xCE | 0xDE | 0xEE | 0xFE | 0xDF
            | 0xEF | 0xFF => self.exec_16bit(bus, opcode),

            // ---- 8-bit RMW (NEG/COM/LSR/ROR/ASR/ASL/ROL/DEC/INC/TST/CLR): low nibble selects
            // the op; TST never writes back ----
            0x40..=0x4F | 0x50..=0x5F | 0x00..=0x0F | 0x60..=0x6F | 0x70..=0x7F => {
                self.exec_rmw(bus, opcode)
            }

            _ => 2, // illegal opcode: 2-cycle NOP
        };
        self.cycles += cycles as u64;
        cycles
    }

    /// `$10`-prefixed page: long conditional branches, CMPD/CMPY/LDY/STY/LDS/STS,
    /// SWI2. Prefixed ops cost one more cycle than their base-page equivalents.
    fn exec_page10(&mut self, bus: &mut impl Bus) -> u32 {
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
            0x83 => {
                let m = self.fetch_u16(bus);
                self.sub16(self.d(), m);
                5
            }
            0x93 => {
                let m = self.read_direct16(bus);
                self.sub16(self.d(), m);
                7
            }
            0xA3 => {
                let (ea, ic) = self.ea_indexed(bus);
                let m = bus.read_u16(ea);
                self.sub16(self.d(), m);
                7 + ic
            }
            0xB3 => {
                let m = self.read_extended16(bus);
                self.sub16(self.d(), m);
                8
            }
            // CMPY
            0x8C => {
                let m = self.fetch_u16(bus);
                self.sub16(self.y, m);
                5
            }
            0x9C => {
                let m = self.read_direct16(bus);
                self.sub16(self.y, m);
                7
            }
            0xAC => {
                let (ea, ic) = self.ea_indexed(bus);
                let m = bus.read_u16(ea);
                self.sub16(self.y, m);
                7 + ic
            }
            0xBC => {
                let m = self.read_extended16(bus);
                self.sub16(self.y, m);
                8
            }

            // LDY
            0x8E => {
                let v = self.fetch_u16(bus);
                self.y = v;
                self.set_nz16(v);
                4
            }
            0x9E => {
                let v = self.read_direct16(bus);
                self.y = v;
                self.set_nz16(v);
                6
            }
            0xAE => {
                let (ea, ic) = self.ea_indexed(bus);
                let v = bus.read_u16(ea);
                self.y = v;
                self.set_nz16(v);
                6 + ic
            }
            0xBE => {
                let v = self.read_extended16(bus);
                self.y = v;
                self.set_nz16(v);
                7
            }
            // STY
            0x9F => {
                let ea = self.ea_direct(bus);
                bus.write_u16(ea, self.y);
                self.set_nz16(self.y);
                6
            }
            0xAF => {
                let (ea, ic) = self.ea_indexed(bus);
                bus.write_u16(ea, self.y);
                self.set_nz16(self.y);
                6 + ic
            }
            0xBF => {
                let ea = self.ea_extended(bus);
                bus.write_u16(ea, self.y);
                self.set_nz16(self.y);
                7
            }

            // LDS
            0xCE => {
                let v = self.fetch_u16(bus);
                self.load_s(v);
                self.set_nz16(v);
                4
            }
            0xDE => {
                let v = self.read_direct16(bus);
                self.load_s(v);
                self.set_nz16(v);
                6
            }
            0xEE => {
                let (ea, ic) = self.ea_indexed(bus);
                let v = bus.read_u16(ea);
                self.load_s(v);
                self.set_nz16(v);
                6 + ic
            }
            0xFE => {
                let v = self.read_extended16(bus);
                self.load_s(v);
                self.set_nz16(v);
                7
            }
            // STS
            0xDF => {
                let ea = self.ea_direct(bus);
                bus.write_u16(ea, self.s);
                self.set_nz16(self.s);
                6
            }
            0xEF => {
                let (ea, ic) = self.ea_indexed(bus);
                bus.write_u16(ea, self.s);
                self.set_nz16(self.s);
                6 + ic
            }
            0xFF => {
                let ea = self.ea_extended(bus);
                bus.write_u16(ea, self.s);
                self.set_nz16(self.s);
                7
            }

            0x3F => {
                self.enter_interrupt(bus, VECTOR_SWI2, false, false, true);
                20
            } // SWI2

            _ => 2, // illegal $10-page opcode: 2-cycle NOP
        }
    }

    /// `$11`-prefixed page: 16-bit compares of U and S, plus SWI3.
    fn exec_page11(&mut self, bus: &mut impl Bus) -> u32 {
        let op2 = self.fetch_u8(bus);
        match op2 {
            // CMPU
            0x83 => {
                let m = self.fetch_u16(bus);
                self.sub16(self.u, m);
                5
            }
            0x93 => {
                let m = self.read_direct16(bus);
                self.sub16(self.u, m);
                7
            }
            0xA3 => {
                let (ea, ic) = self.ea_indexed(bus);
                let m = bus.read_u16(ea);
                self.sub16(self.u, m);
                7 + ic
            }
            0xB3 => {
                let m = self.read_extended16(bus);
                self.sub16(self.u, m);
                8
            }
            // CMPS
            0x8C => {
                let m = self.fetch_u16(bus);
                self.sub16(self.s, m);
                5
            }
            0x9C => {
                let m = self.read_direct16(bus);
                self.sub16(self.s, m);
                7
            }
            0xAC => {
                let (ea, ic) = self.ea_indexed(bus);
                let m = bus.read_u16(ea);
                self.sub16(self.s, m);
                7 + ic
            }
            0xBC => {
                let m = self.read_extended16(bus);
                self.sub16(self.s, m);
                8
            }

            0x3F => {
                self.enter_interrupt(bus, VECTOR_SWI3, false, false, true);
                20
            } // SWI3

            _ => 2, // illegal $11-page opcode: 2-cycle NOP
        }
    }

    /// JMP / JSR / BSR / LBSR / RTS / TFR / EXG / PSHS / PULS / PSHU / PULU.
    fn exec_control_transfer(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        match opcode {
            // JMP — direct / indexed / extended
            0x0E => {
                let ea = self.ea_direct(bus);
                self.pc = ea;
                3
            }
            0x6E => {
                let (ea, ic) = self.ea_indexed(bus);
                self.pc = ea;
                3 + ic
            }
            0x7E => {
                let ea = self.ea_extended(bus);
                self.pc = ea;
                4
            }

            // JSR — push return address, then jump
            0x9D => {
                let ea = self.ea_direct(bus);
                self.push16_s(bus, self.pc);
                self.pc = ea;
                7
            }
            0xAD => {
                let (ea, ic) = self.ea_indexed(bus);
                self.push16_s(bus, self.pc);
                self.pc = ea;
                7 + ic
            }
            0xBD => {
                let ea = self.ea_extended(bus);
                self.push16_s(bus, self.pc);
                self.pc = ea;
                8
            }

            // BSR / LBSR — relative branch to subroutine
            0x8D => {
                let offset = self.fetch_u8(bus) as i8 as i16 as u16;
                self.push16_s(bus, self.pc);
                self.pc = self.pc.wrapping_add(offset);
                7
            }
            0x17 => {
                let offset = self.fetch_u16(bus);
                self.push16_s(bus, self.pc);
                self.pc = self.pc.wrapping_add(offset);
                9
            }

            // RTS — pull return address
            0x39 => {
                self.pc = self.pull16_s(bus);
                5
            }

            // TFR / EXG
            0x1F => {
                let pb = self.fetch_u8(bus);
                let v = self.tfr_value(pb >> 4, pb & 0x0F);
                self.reg_write(pb & 0x0F, v);
                6
            }
            0x1E => {
                let pb = self.fetch_u8(bus);
                let (r0, r1) = (pb >> 4, pb & 0x0F);
                let (v0, v1) = self.exg_values(r0, r1);
                // r1 first: EXG A,D relies on the later A write overriding D's MSB.
                self.reg_write(r1, v0);
                self.reg_write(r0, v1);
                8
            }

            // PSHS / PULS / PSHU / PULU
            0x34 => {
                let mask = self.fetch_u8(bus);
                self.psh(bus, mask, true)
            }
            0x36 => {
                let mask = self.fetch_u8(bus);
                self.psh(bus, mask, false)
            }
            0x35 => {
                let mask = self.fetch_u8(bus);
                self.pul(bus, mask, true)
            }
            0x37 => {
                let mask = self.fetch_u8(bus);
                self.pul(bus, mask, false)
            }

            _ => unreachable!("exec_control_transfer called for opcode {opcode:#04X}"),
        }
    }

    /// ORCC / ANDCC / SEX / ABX / MUL / DAA.
    fn exec_misc_inherent(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        match opcode {
            0x1A => {
                let m = self.fetch_u8(bus);
                self.cc |= m;
                3
            } // ORCC #i8
            0x1C => {
                let m = self.fetch_u8(bus);
                self.cc &= m;
                3
            } // ANDCC #i8
            0x1D => {
                // SEX — sign-extend B into A; N,Z from D, V unaffected
                self.a = if self.b & 0x80 != 0 { 0xFF } else { 0x00 };
                let d = self.d();
                self.set_nz16_only(d);
                2
            }
            0x3A => {
                self.x = self.x.wrapping_add(self.b as u16);
                3
            } // ABX (B unsigned)
            0x3D => {
                // MUL — D = A*B unsigned; Z from result, C = bit 7 of B
                let product = self.a as u16 * self.b as u16;
                self.set_d(product);
                self.set_z16(product);
                self.set_carry(product & 0x0080 != 0);
                11
            }
            0x19 => self.daa(),

            _ => unreachable!("exec_misc_inherent called for opcode {opcode:#04X}"),
        }
    }

    /// SWI / RTI / CWAI / SYNC.
    fn exec_interrupt_halt(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        match opcode {
            0x3F => {
                self.enter_interrupt(bus, VECTOR_SWI, true, true, true);
                19
            } // SWI
            0x3B => {
                // RTI — pull CC, then full frame if E set else PC only
                self.pul(bus, 0x01, true);
                if self.cc & cc::ENTIRE != 0 {
                    self.pul(bus, 0xFE, true);
                    15
                } else {
                    self.pul(bus, 0x80, true);
                    6
                }
            }
            0x3C => {
                // CWAI — clear CC bits, stack full frame, then halt
                let m = self.fetch_u8(bus);
                self.cc &= m;
                self.cc |= cc::ENTIRE;
                self.psh(bus, 0xFF, true);
                self.state = State::Waiting;
                CWAI_STACK_CYCLES
            }
            0x13 => {
                self.state = State::Syncing;
                2
            } // SYNC — halt until interrupt

            _ => unreachable!("exec_interrupt_halt called for opcode {opcode:#04X}"),
        }
    }
}
