//! 8/16-bit ALU primitives and condition-code flag helpers shared by every
//! instruction family in [`crate::exec`].

use crate::{MC6809, cc};

impl MC6809 {
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

    pub(crate) fn set_carry(&mut self, on: bool) {
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
    pub(crate) fn set_nz8(&mut self, value: u8) {
        self.set_overflow(false);
        self.set_nz8_only(value);
    }

    /// Set Z (only) from a 16-bit result. Used by LEAX/LEAY, which touch no other
    /// condition codes.
    pub(crate) fn set_z16(&mut self, value: u16) {
        if value == 0 {
            self.cc |= cc::ZERO;
        } else {
            self.cc &= !cc::ZERO;
        }
    }

    /// Set N and Z from a 16-bit result, leaving V, C, H untouched.
    pub(crate) fn set_nz16_only(&mut self, value: u16) {
        self.cc &= !(cc::NEGATIVE | cc::ZERO);
        if value == 0 {
            self.cc |= cc::ZERO;
        }
        if value & 0x8000 != 0 {
            self.cc |= cc::NEGATIVE;
        }
    }

    /// Set N and Z from a 16-bit result and clear V (the LD/ST convention).
    pub(crate) fn set_nz16(&mut self, value: u16) {
        self.set_overflow(false);
        self.set_nz16_only(value);
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
    pub(crate) fn rmw_apply(&mut self, op_nibble: u8, m: u8) -> u8 {
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
    pub(crate) fn daa(&mut self) -> u32 {
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

    /// 8-bit add with carry-in: `a + m + carry_in`. Sets H, N, Z, V, C per the
    /// 6809 datasheet. Used by ADD (carry_in=0) and ADC (carry_in=C).
    ///
    /// - C: carry out of bit 7.
    /// - H: carry out of bit 3 (used by DAA).
    /// - V: signed overflow — operands share a sign that differs from the result.
    pub(crate) fn add8(&mut self, a: u8, m: u8, carry_in: u8) -> u8 {
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
    pub(crate) fn sub8(&mut self, a: u8, m: u8, borrow_in: u8) -> u8 {
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

    /// 16-bit add (ADDD): `a + m`. Sets N, Z, V, C; H is unaffected.
    pub(crate) fn add16(&mut self, a: u16, m: u16) -> u16 {
        let sum = a as u32 + m as u32;
        let r = sum as u16;
        self.set_carry(sum > 0xFFFF);
        self.set_overflow((a ^ r) & (m ^ r) & 0x8000 != 0);
        self.set_nz16_only(r);
        r
    }

    /// 16-bit subtract (SUBD / CMPx): `a - m`. Sets N, Z, V, C.
    pub(crate) fn sub16(&mut self, a: u16, m: u16) -> u16 {
        let diff = (a as u32).wrapping_sub(m as u32);
        let r = diff as u16;
        self.set_carry(diff & 0x1_0000 != 0);
        self.set_overflow((a ^ m) & (a ^ r) & 0x8000 != 0);
        self.set_nz16_only(r);
        r
    }
}
