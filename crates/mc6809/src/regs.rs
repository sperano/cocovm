//! Register transfer (TFR/EXG) support: mapping the postbyte's register-selector
//! nibbles ([`crate::regsel`]) to actual register reads/writes and honouring the
//! documented cross-size transfer rules.

use crate::{regsel, MC6809};

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
}
