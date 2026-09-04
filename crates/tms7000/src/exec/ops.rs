//! The operations of the addressing-mode grid (MAME `op_*`). Each returns
//! the value to write back, or `None` for compare/test/multiply which write
//! nowhere (or elsewhere). Intermediates are 16-bit exactly as in MAME so
//! the carry falls out of bit 8.

use crate::decode::Op;
use crate::{Bus, TMS7040, st};

/// Surcharges on top of the mode's base cost (MAME `op_*`'s own `m_icount -=`).
mod extra {
    pub const MPY: u32 = 39;
    pub const SWAP: u32 = 3;
    pub const XCHB: u32 = 1;
    pub const DAC: u32 = 2;
    pub const DSB: u32 = 2;
    /// Displacement fetch, and again if the branch is taken.
    pub const SHORT_BRANCH: u32 = 2;
    pub const SHORT_BRANCH_TAKEN: u32 = 2;
    /// `JMP`/`Jcc` on top of the short-branch cost.
    pub const JMP: u32 = 3;
}

/// BCD correction constants indexed by MAME's `d` (MAME `lut_bcd_out`).
const BCD_CORRECTION: [u8; 6] = [0x00, 0x06, 0x00, 0x66, 0x60, 0x66];

impl TMS7040 {
    pub(super) fn apply(&mut self, bus: &mut impl Bus, op: Op, p1: u8, p2: u8) -> Option<u8> {
        let p1w = u16::from(p1);
        let p2w = u16::from(p2);
        match op {
            Op::Clr => {
                self.set_cnz(0);
                Some(0)
            }
            Op::Dec => {
                let t = p1w.wrapping_sub(1);
                self.set_nz(t);
                self.set_c(!t);
                Some(t as u8)
            }
            Op::Inc => {
                let t = p1w + 1;
                self.set_cnz(t);
                Some(t as u8)
            }
            Op::Inv => {
                let t = u16::from(!p1);
                self.set_cnz(t);
                Some(t as u8)
            }
            Op::Rl => {
                let t = (p1w << 1) | (p1w >> 7);
                self.set_cnz(t);
                Some(t as u8)
            }
            Op::Rlc => {
                let t = (p1w << 1) | u16::from(self.carry());
                self.set_cnz(t);
                Some(t as u8)
            }
            Op::Rr => {
                let t = (p1w >> 1) | (p1w << 8) | ((p1w << 7) & 0x80);
                self.set_cnz(t);
                Some(t as u8)
            }
            Op::Rrc => {
                let t = (p1w >> 1) | (p1w << 8) | (u16::from(self.carry()) << 7);
                self.set_cnz(t);
                Some(t as u8)
            }
            Op::Swap => {
                self.burn(extra::SWAP);
                let t = (p1w >> 4) | (p1w << 4);
                self.set_cnz(t);
                Some(t as u8)
            }
            Op::Xchb => {
                self.burn(extra::XCHB);
                let t = self.rf[1];
                self.set_cnz(u16::from(t));
                self.rf[1] = p1;
                Some(t)
            }
            Op::Adc => {
                let t = p1w + p2w + u16::from(self.carry());
                self.set_cnz(t);
                Some(t as u8)
            }
            Op::Add => {
                let t = p1w + p2w;
                self.set_cnz(t);
                Some(t as u8)
            }
            Op::And => {
                let t = u16::from(p1 & p2);
                self.set_cnz(t);
                Some(t as u8)
            }
            Op::Cmp => {
                let t = p1w.wrapping_sub(p2w);
                self.set_nz(t);
                self.set_c(!t);
                None
            }
            Op::Dac => Some(self.dac(p1, p2)),
            Op::Dsb => Some(self.dsb(p1, p2)),
            Op::Mpy => {
                self.burn(extra::MPY);
                let t = p1w * p2w;
                self.set_cnz(t >> 8);
                // The product always lands in A:B.
                self.write_mem16(bus, 0, t);
                None
            }
            Op::Mov => {
                self.set_cnz(p2w);
                Some(p2)
            }
            Op::Or => {
                let t = u16::from(p1 | p2);
                self.set_cnz(t);
                Some(t as u8)
            }
            Op::Sbb => {
                let borrow = u16::from(self.carry() == 0);
                let t = p1w.wrapping_sub(p2w).wrapping_sub(borrow);
                self.set_nz(t);
                self.set_c(!t);
                Some(t as u8)
            }
            Op::Sub => {
                let t = p1w.wrapping_sub(p2w);
                self.set_nz(t);
                self.set_c(!t);
                Some(t as u8)
            }
            Op::Xor => {
                let t = u16::from(p1 ^ p2);
                self.set_cnz(t);
                Some(t as u8)
            }
            Op::Djnz => {
                let t = p1w.wrapping_sub(1);
                self.short_branch(bus, t != 0);
                Some(t as u8)
            }
            Op::Btjo => {
                let t = p1 & p2;
                self.set_cnz(u16::from(t));
                self.short_branch(bus, t != 0);
                None
            }
            Op::Btjz => {
                let t = !p1 & p2;
                self.set_cnz(u16::from(t));
                self.short_branch(bus, t != 0);
                None
            }
        }
    }

    /// Decimal add with carry (MAME `op_dac`).
    fn dac(&mut self, p1: u8, p2: u8) -> u8 {
        self.burn(extra::DAC);
        let c = self.carry();
        let (h1, l1) = (p1 >> 4, p1 & 0xF);
        let (h2, l2) = (p2 >> 4, p2 & 0xF);
        let mut d = u8::from(l1 + l2 + c >= 10);
        if h1 + h2 == 9 {
            d |= 2;
        } else if h1 + h2 > 9 {
            d |= 4;
        }
        let t = p1
            .wrapping_add(p2)
            .wrapping_add(c)
            .wrapping_add(BCD_CORRECTION[usize::from(d)]);
        self.set_cnz(u16::from(t));
        if d > 2 {
            self.st |= st::C;
        }
        t
    }

    /// Decimal subtract with borrow (MAME `op_dsb`).
    fn dsb(&mut self, p1: u8, p2: u8) -> u8 {
        self.burn(extra::DSB);
        let c = u8::from(self.carry() == 0);
        let (h1, l1) = (p1 >> 4, p1 & 0xF);
        let (h2, l2) = (p2 >> 4, p2 & 0xF);
        let mut d = u8::from(i16::from(l1) - i16::from(c) < i16::from(l2));
        if h1 == h2 {
            d |= 2;
        } else if h1 < h2 {
            d |= 4;
        }
        let t = p1
            .wrapping_sub(p2)
            .wrapping_sub(c)
            .wrapping_sub(BCD_CORRECTION[usize::from(d)]);
        self.set_cnz(u16::from(t));
        if d <= 2 {
            self.st |= st::C;
        }
        t
    }

    /// Fetch a signed displacement; add it to PC if `take` (MAME `shortbranch`).
    pub(super) fn short_branch(&mut self, bus: &mut impl Bus, take: bool) {
        self.burn(extra::SHORT_BRANCH);
        let d = self.imm8(bus) as i8;
        if take {
            self.pc = self.pc.wrapping_add_signed(i16::from(d));
            self.burn(extra::SHORT_BRANCH_TAKEN);
        }
    }

    /// `JMP`/`Jcc` (MAME `jmp`).
    pub(super) fn jmp(&mut self, bus: &mut impl Bus, take: bool) {
        self.burn(extra::JMP);
        self.short_branch(bus, take);
    }
}
