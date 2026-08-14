//! Register transfer (TFR/EXG) support: mapping the postbyte's register-selector
//! nibbles ([`crate::regsel`]) to actual register reads/writes and honouring the
//! documented cross-size transfer rules.

use crate::{MC6809, regsel};

impl MC6809 {
    // ---- Register transfer (TFR/EXG) --------------------------------------
    // Postbyte nibble codes: 0=D 1=X 2=Y 3=U 4=S 5=PC 8=A 9=B A=CC B=DP.
    // Codes 0..5 are the 16-bit registers.

    fn reg_is16(code: u8) -> bool {
        code <= regsel::PC
    }

    pub(crate) fn reg_read(&self, code: u8) -> u16 {
        match code {
            regsel::D => self.d(),
            regsel::X => self.x,
            regsel::Y => self.y,
            regsel::U => self.u,
            regsel::S => self.s,
            regsel::PC => self.pc,
            regsel::A => self.a as u16,
            regsel::B => self.b as u16,
            regsel::CC => self.cc as u16,
            regsel::DP => self.dp as u16,
            _ => 0xFFFF, // invalid on 6809
        }
    }

    pub(crate) fn reg_write(&mut self, code: u8, value: u16) {
        match code {
            regsel::D => self.set_d(value),
            regsel::X => self.x = value,
            regsel::Y => self.y = value,
            regsel::U => self.u = value,
            regsel::S => self.load_s(value),
            regsel::PC => self.pc = value,
            regsel::A => self.a = value as u8,
            regsel::B => self.b = value as u8,
            regsel::CC => self.cc = value as u8,
            regsel::DP => self.dp = value as u8,
            _ => {} // invalid
        }
    }

    /// The value TFR writes to `dst` given `src`, honouring the documented 6809
    /// behaviour when register sizes differ (16→8 keeps the LSB — handled by
    /// `reg_write`; A/B→16 sets MSB=$FF; CC/DP→16 duplicates the byte).
    pub(crate) fn tfr_value(&self, src: u8, dst: u8) -> u16 {
        let sv = self.reg_read(src);
        match (Self::reg_is16(src), Self::reg_is16(dst)) {
            (false, true) => {
                let b = sv & 0x00FF;
                match src {
                    regsel::A | regsel::B => 0xFF00 | b, // A/B → 16: high byte = $FF
                    _ => (b << 8) | b,                   // CC/DP → 16: both bytes = source
                }
            }
            _ => sv, // same size, or 16 → 8 (reg_write truncates to the LSB)
        }
    }

    /// The 16-bit values EXG reads from postbyte registers `r0` (first-named)
    /// and `r1` before either is written, per the 6809's documented exchange
    /// rules (6x09 Instruction Sets, EXG). Unlike TFR, an 8-bit operand's
    /// widening depends on whether the *first-named* register is 8-bit, not on
    /// the operand's own size — this is what makes `EXG CC,X` and `EXG X,CC`
    /// widen CC differently even though CC is read both times.
    pub(crate) fn exg_values(&self, r0: u8, r1: u8) -> (u16, u16) {
        let first_is_8bit = !Self::reg_is16(r0);
        (
            self.exg_value(r0, first_is_8bit),
            self.exg_value(r1, first_is_8bit),
        )
    }

    fn exg_value(&self, code: u8, first_is_8bit: bool) -> u16 {
        let sv = self.reg_read(code);
        if Self::reg_is16(code) {
            return sv;
        }
        let b = sv & 0x00FF;
        match code {
            regsel::A | regsel::B => 0xFF00 | b, // A/B: always $FF in the high byte
            _ if first_is_8bit => (b << 8) | b,  // CC/DP, first-named 8-bit: duplicate
            _ => 0xFF00 | b,                     // CC/DP, first-named 16-bit: pad $FF
        }
    }
}
