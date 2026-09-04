//! TMS7000 disassembler, a pure function over a byte reader (like
//! `mc6809::disasm::disassemble`) with MAME `7000dasm.cpp`'s spellings:
//! `MOVP %>00,P0`, `LDA @>F123`, `BR *R5`, `STA @>F000(B)`, `JMP >F012`.
//! Undecoded bytes print as `Illegal Opcode`, length 1 — including `$B1`,
//! which the core executes as MOV B,A but MAME's table does not name.

mod tables;

use std::fmt;

use tables::{Arg, FORMATS, LOOKUP};

/// One decoded instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Insn {
    /// Bytes the instruction occupies.
    pub len: u8,
    /// MAME's mnemonic, which for some opcodes carries its operand
    /// (`CLR A`, `PUSH ST`, `TRAP 3`).
    pub mnemonic: &'static str,
    /// Remaining operand text, empty when none.
    pub operand: String,
}

impl fmt::Display for Insn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.operand.is_empty() {
            f.write_str(self.mnemonic)
        } else {
            write!(f, "{} {}", self.mnemonic, self.operand)
        }
    }
}

/// What MAME prints for a byte its table doesn't cover.
pub const ILLEGAL: &str = "Illegal Opcode";

/// Decode the instruction at `pc`, reading bytes through `read`.
pub fn disassemble(read: &mut impl FnMut(u16) -> u8, pc: u16) -> Insn {
    let mut pos = pc;
    let mut next = |pos: &mut u16| {
        let b = read(*pos);
        *pos = pos.wrapping_add(1);
        b
    };
    let opcode = next(&mut pos);
    let Some((mnemonic, fmt)) = LOOKUP[usize::from(opcode)] else {
        return Insn {
            len: 1,
            mnemonic: ILLEGAL,
            operand: String::new(),
        };
    };
    let mut operand = String::new();
    for &(prefix, arg, suffix) in FORMATS[fmt] {
        operand.push_str(prefix);
        match arg {
            Arg::None => {}
            Arg::Dec => operand.push_str(&next(&mut pos).to_string()),
            Arg::Hex => operand.push_str(&format!("{:X}", next(&mut pos))),
            Arg::Hex2 => operand.push_str(&format!("{:02X}", next(&mut pos))),
            Arg::Hex4 => {
                let hi = next(&mut pos);
                let lo = next(&mut pos);
                operand.push_str(&format!("{:04X}", u16::from_be_bytes([hi, lo])));
            }
            Arg::Rel => {
                let d = next(&mut pos) as i8;
                let target = pos.wrapping_add_signed(i16::from(d));
                operand.push_str(&format!(">{target:04X}"));
            }
            Arg::Abs => {
                let hi = next(&mut pos);
                let lo = next(&mut pos);
                operand.push_str(&format!(">{:04X}", u16::from_be_bytes([hi, lo])));
            }
        }
        operand.push_str(suffix);
    }
    Insn {
        len: pos.wrapping_sub(pc) as u8,
        mnemonic,
        operand,
    }
}
