//! Branch-condition evaluation, shared by the short `Bcc` and long `LBcc`
//! encodings in [`crate::exec`].

use crate::{MC6809, cc};

impl MC6809 {
    /// Evaluates a branch condition selected by the opcode's low nibble — shared
    /// by the short `Bcc` and long `LBcc` encodings (0=BRA always … F=BLE).
    pub(crate) fn branch_taken(&self, cond: u8) -> bool {
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
}
