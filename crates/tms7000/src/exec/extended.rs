//! Extended-addressing and stack opcodes: LDA/STA/CMPA/MOVD/BR/CALL in
//! direct, indirect, and indexed forms, DECD, PUSH/POP (MAME
//! `tms7000op.cpp` "other opcodes").

use crate::decode::Decoded;
use crate::{Bus, TMS7040};

/// Cycle costs (MAME `tms7000op.cpp`).
mod cost {
    pub const DECD_A: u32 = 9;
    pub const DECD_B: u32 = 9;
    pub const DECD_R: u32 = 11;
    pub const CMPA_DIR: u32 = 12;
    pub const CMPA_INX: u32 = 14;
    pub const CMPA_IND: u32 = 11;
    pub const LDA_DIR: u32 = 11;
    pub const LDA_INX: u32 = 13;
    pub const LDA_IND: u32 = 10;
    pub const STA_DIR: u32 = 11;
    pub const STA_INX: u32 = 13;
    pub const STA_IND: u32 = 10;
    pub const MOVD_DIR: u32 = 15;
    pub const MOVD_INX: u32 = 17;
    pub const MOVD_IND: u32 = 14;
    pub const BR_DIR: u32 = 10;
    pub const BR_INX: u32 = 12;
    pub const BR_IND: u32 = 9;
    pub const CALL_DIR: u32 = 14;
    pub const CALL_INX: u32 = 16;
    pub const CALL_IND: u32 = 13;
    pub const PUSH_AB: u32 = 6;
    pub const PUSH_R: u32 = 8;
    pub const POP_AB: u32 = 6;
    pub const POP_R: u32 = 8;
}

impl TMS7040 {
    pub(super) fn exec_extended(&mut self, bus: &mut impl Bus, decoded: Decoded) {
        match decoded {
            Decoded::DecdA => self.decd(bus, 0, cost::DECD_A),
            Decoded::DecdB => self.decd(bus, 1, cost::DECD_B),
            Decoded::DecdR => {
                let r = self.imm8(bus);
                self.decd(bus, r, cost::DECD_R);
            }

            Decoded::CmpaDir => {
                self.burn(cost::CMPA_DIR);
                let addr = self.imm16(bus);
                self.cmpa(bus, addr);
            }
            Decoded::CmpaInx => {
                self.burn(cost::CMPA_INX);
                let addr = self.indexed(bus);
                self.cmpa(bus, addr);
            }
            Decoded::CmpaInd => {
                self.burn(cost::CMPA_IND);
                let addr = self.indirect(bus);
                self.cmpa(bus, addr);
            }

            Decoded::LdaDir => {
                self.burn(cost::LDA_DIR);
                let addr = self.imm16(bus);
                self.lda(bus, addr);
            }
            Decoded::LdaInx => {
                self.burn(cost::LDA_INX);
                let addr = self.indexed(bus);
                self.lda(bus, addr);
            }
            Decoded::LdaInd => {
                self.burn(cost::LDA_IND);
                let addr = self.indirect(bus);
                self.lda(bus, addr);
            }

            Decoded::StaDir => {
                self.burn(cost::STA_DIR);
                let addr = self.imm16(bus);
                self.sta(bus, addr);
            }
            Decoded::StaInx => {
                self.burn(cost::STA_INX);
                let addr = self.indexed(bus);
                self.sta(bus, addr);
            }
            Decoded::StaInd => {
                self.burn(cost::STA_IND);
                let addr = self.indirect(bus);
                self.sta(bus, addr);
            }

            Decoded::MovdDir => {
                self.burn(cost::MOVD_DIR);
                let t = self.imm16(bus);
                self.movd(bus, t);
            }
            Decoded::MovdInx => {
                self.burn(cost::MOVD_INX);
                let t = self.indexed(bus);
                self.movd(bus, t);
            }
            Decoded::MovdInd => {
                self.burn(cost::MOVD_IND);
                let t = self.indirect(bus);
                self.movd(bus, t);
            }

            Decoded::BrDir => {
                self.burn(cost::BR_DIR);
                self.pc = self.imm16(bus);
            }
            Decoded::BrInx => {
                self.burn(cost::BR_INX);
                self.pc = self.indexed(bus);
            }
            Decoded::BrInd => {
                self.burn(cost::BR_IND);
                self.pc = self.indirect(bus);
            }

            Decoded::CallDir => {
                self.burn(cost::CALL_DIR);
                let t = self.imm16(bus);
                self.call(bus, t);
            }
            Decoded::CallInx => {
                self.burn(cost::CALL_INX);
                let t = self.indexed(bus);
                self.call(bus, t);
            }
            Decoded::CallInd => {
                self.burn(cost::CALL_IND);
                let t = self.indirect(bus);
                self.call(bus, t);
            }

            Decoded::PushA => self.push_r(bus, 0, cost::PUSH_AB),
            Decoded::PushB => self.push_r(bus, 1, cost::PUSH_AB),
            Decoded::PushR => {
                let r = self.imm8(bus);
                self.push_r(bus, r, cost::PUSH_R);
            }
            Decoded::PopA => self.pop_r(bus, 0, cost::POP_AB),
            Decoded::PopB => self.pop_r(bus, 1, cost::POP_AB),
            Decoded::PopR => {
                let r = self.imm8(bus);
                self.pop_r(bus, r, cost::POP_R);
            }

            other => unreachable!("{other:?} is dispatched in exec.rs"),
        }
    }

    /// `@>addr(B)`: 16-bit immediate plus B, unsigned.
    fn indexed(&mut self, bus: &mut impl Bus) -> u16 {
        self.imm16(bus).wrapping_add(u16::from(self.rf[1]))
    }

    /// `*Rn`: the register pair `Rn-1:Rn`.
    fn indirect(&mut self, bus: &mut impl Bus) -> u16 {
        let r = self.imm8(bus);
        self.read_r16(bus, r)
    }

    fn decd(&mut self, bus: &mut impl Bus, r: u8, cycles: u32) {
        self.burn(cycles);
        let t = u32::from(self.read_r16(bus, r)).wrapping_sub(1);
        self.write_r16(bus, r, t as u16);
        self.set_nz((t >> 8) as u16);
        self.set_c(!((t >> 8) as u16));
    }

    fn cmpa(&mut self, bus: &mut impl Bus, addr: u16) {
        let m = self.read_mem(bus, addr);
        let t = u16::from(self.rf[0]).wrapping_sub(u16::from(m));
        self.set_nz(t);
        self.set_c(!t);
    }

    fn lda(&mut self, bus: &mut impl Bus, addr: u16) {
        let t = self.read_mem(bus, addr);
        self.rf[0] = t;
        self.set_cnz(u16::from(t));
    }

    fn sta(&mut self, bus: &mut impl Bus, addr: u16) {
        let t = self.rf[0];
        self.write_mem(bus, addr, t);
        self.set_cnz(u16::from(t));
    }

    fn movd(&mut self, bus: &mut impl Bus, t: u16) {
        let r = self.imm8(bus);
        self.write_r16(bus, r, t);
        self.set_cnz(t >> 8);
    }

    fn call(&mut self, bus: &mut impl Bus, target: u16) {
        self.push16(bus, self.pc);
        self.pc = target;
    }

    fn push_r(&mut self, bus: &mut impl Bus, r: u8, cycles: u32) {
        self.burn(cycles);
        let t = self.read_r8(bus, r);
        self.push8(bus, t);
        self.set_cnz(u16::from(t));
    }

    fn pop_r(&mut self, bus: &mut impl Bus, r: u8, cycles: u32) {
        self.burn(cycles);
        let t = self.pull8(bus);
        self.write_r8(bus, r, t);
        self.set_cnz(u16::from(t));
    }
}
