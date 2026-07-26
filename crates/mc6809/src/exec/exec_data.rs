//! Data-manipulation opcode families dispatched from [`crate::exec::MC6809::step`]:
//! basic load/store, the 8-bit ALU, indexed addressing, 8-bit logic, the 16-bit
//! ALU/load/store set, and read-modify-write. Split out from `exec.rs` because
//! together these opcodes make up most of the ISA's raw byte count while sharing
//! none of `step`'s control-flow logic.

use crate::{cc, Bus, MC6809};

impl MC6809 {
    /// LDA / LDB / STA / STB / LDD / STD — immediate / direct / extended.
    pub(super) fn exec_load_store(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        match opcode {
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

            _ => unreachable!("exec_load_store called for opcode {opcode:#04X}"),
        }
    }

    /// ADDA/ADDB, ADCA/ADCB, SUBA/SUBB, SBCA/SBCB, CMPA/CMPB — immediate / direct
    /// / extended.
    pub(super) fn exec_alu8(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        match opcode {
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

            _ => unreachable!("exec_alu8 called for opcode {opcode:#04X}"),
        }
    }

    /// LEAX/LEAY/LEAS/LEAU and the indexed forms of LDA/STA/LDB/STB/LDD/STD and
    /// the 8-bit ALU (ADD/ADC/SUB/SBC/CMP for A and B).
    pub(super) fn exec_indexed(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        match opcode {
            // LEA — load effective address into a register.
            // LEAX/LEAY set Z from the result; LEAS/LEAU affect no flags.
            0x30 => { let (ea, ic) = self.ea_indexed(bus); self.x = ea; self.set_z16(ea); 4 + ic }
            0x31 => { let (ea, ic) = self.ea_indexed(bus); self.y = ea; self.set_z16(ea); 4 + ic }
            0x32 => { let (ea, ic) = self.ea_indexed(bus); self.load_s(ea); 4 + ic }
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

            _ => unreachable!("exec_indexed called for opcode {opcode:#04X}"),
        }
    }

    /// ANDA/ANDB, ORA/ORB, EORA/EORB, BITA/BITB — immediate / direct / indexed /
    /// extended.
    pub(super) fn exec_logic8(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        match opcode {
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

            _ => unreachable!("exec_logic8 called for opcode {opcode:#04X}"),
        }
    }

    /// ADDD/SUBD/CMPX and LDX/STX/LDU/STU — immediate / direct / indexed /
    /// extended.
    pub(super) fn exec_16bit(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        match opcode {
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

            _ => unreachable!("exec_16bit called for opcode {opcode:#04X}"),
        }
    }

    /// NEG/COM/LSR/ROR/ASR/ASL/ROL/DEC/INC/TST/CLR — inherent (A/B) / direct /
    /// indexed / extended. Low nibble selects the op (see `rmw_apply`).
    pub(super) fn exec_rmw(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
        match opcode {
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

            _ => unreachable!("exec_rmw called for opcode {opcode:#04X}"),
        }
    }
}
