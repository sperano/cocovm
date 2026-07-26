//! Static MC6809 disassembler — a pure function over a byte reader, no CPU
//! state.
//!
//! [`MC6809::step`](crate::MC6809::step) is the *executing* dispatcher (it
//! boots real BASIC) and is therefore the authority for which opcodes exist,
//! which addressing mode each uses, and how many bytes each consumes —
//! including which opcodes are illegal/undecoded. This module mirrors that
//! opcode map byte-for-byte. Mnemonic spellings and operand syntax are
//! cross-checked against `docs/6x09_Instruction_Sets.pdf` ("the PDF").
//!
//! Table-driven: "what mnemonic/addressing-mode does opcode X have" is data
//! (`tables::base_entry`, `tables::page10_entry`, `tables::page11_entry`, and
//! the small per-nibble arrays for the read-modify-write group and the
//! branch-condition group); "how do I render addressing mode Y into an
//! operand string" is shared logic ([`render`], `indexed::decode_indexed`)
//! used by every entry that shares that mode. This separation is what lets
//! HD6309-specific mnemonics be layered in later without restructuring.
//!
//! Illegal opcodes (including ones the real core silently falls through to a
//! 2-cycle default for) disassemble as mnemonic `"???"` with a length that
//! still matches exactly what [`MC6809::step`](crate::MC6809::step) would
//! have consumed for the same bytes — so a scrolling disassembly view never
//! desyncs from the byte stream, even across undecoded opcodes.

use crate::{regsel, stack_mask};

mod indexed;
mod tables;

/// One disassembled instruction.
pub struct Insn {
    /// Total bytes consumed: opcode (+ `$10`/`$11` prefix byte, if any) +
    /// operand bytes. Matches exactly what `MC6809::step` would have read for
    /// the same byte sequence.
    pub len: u8,
    pub mnemonic: &'static str,
    pub operand: String,
}

/// Disassemble one instruction starting at `pc`. `read` is called at most as
/// many times as the instruction actually needs (opcode, optional prefix
/// byte, and operand bytes) — never speculatively, so it is safe to back onto
/// a live bus with side-effecting reads is not assumed; callers wanting
/// side-effect-free reads should pass a peek-style closure (see
/// `coco-core`'s planned `SystemBus::peek`).
pub fn disassemble(read: &mut impl FnMut(u16) -> u8, pc: u16) -> Insn {
    let mut r = Reader {
        read,
        cur: pc,
        len: 0,
    };
    let opcode = r.u8();
    let (mnemonic, operand) = decode_base(&mut r, opcode);
    Insn {
        len: r.len,
        mnemonic,
        operand,
    }
}

/// Tracks the read cursor and byte count for one instruction's decode.
struct Reader<'a, F: FnMut(u16) -> u8> {
    read: &'a mut F,
    cur: u16,
    len: u8,
}

impl<F: FnMut(u16) -> u8> Reader<'_, F> {
    fn u8(&mut self) -> u8 {
        let v = (self.read)(self.cur);
        self.cur = self.cur.wrapping_add(1);
        self.len += 1;
        v
    }

    fn u16(&mut self) -> u16 {
        let hi = self.u8() as u16;
        let lo = self.u8() as u16;
        (hi << 8) | lo
    }
}

/// Addressing mode as it matters for *disassembly rendering* (operand syntax
/// and byte count) — not a 1:1 mirror of the core's internal EA helpers, but
/// close enough to reuse its byte-counting logic directly.
#[derive(Clone, Copy)]
enum Mode {
    /// No operand bytes. If paired with mnemonic `"???"`, the operand is
    /// rendered as the raw opcode/second-byte in hex (see [`render`]).
    Inherent,
    Imm8,
    Imm16,
    Direct,
    Extended,
    Indexed,
    /// 8-bit signed relative branch, target resolved to an absolute address.
    Rel8,
    /// 16-bit relative branch, target resolved to an absolute address.
    Rel16,
    /// TFR/EXG register-pair postbyte.
    RegPair,
    /// PSHS/PULS mask byte — bit 0x40 ("other stack pointer") means U.
    StackS,
    /// PSHU/PULU mask byte — bit 0x40 ("other stack pointer") means S.
    StackU,
}

#[derive(Clone, Copy)]
struct Entry {
    mnemonic: &'static str,
    mode: Mode,
}

const fn e(mnemonic: &'static str, mode: Mode) -> Entry {
    Entry { mnemonic, mode }
}

/// Placeholder for an opcode nothing decodes (see module doc: mirrors the
/// core's silent 2-cycle-default fallthrough).
const ILLEGAL: Entry = e("???", Mode::Inherent);

fn decode_base<F: FnMut(u16) -> u8>(r: &mut Reader<F>, opcode: u8) -> (&'static str, String) {
    match opcode {
        0x10 => {
            let op2 = r.u8();
            render(r, op2, tables::page10_entry(op2))
        }
        0x11 => {
            let op2 = r.u8();
            render(r, op2, tables::page11_entry(op2))
        }
        _ => render(r, opcode, tables::base_entry(opcode)),
    }
}

/// Render an [`Entry`]'s operand given the mode, consuming exactly the bytes
/// that mode requires. `raw_byte` is the opcode (base page) or second byte
/// (prefixed pages) — used only for the illegal-inherent operand rendering.
fn render<F: FnMut(u16) -> u8>(
    r: &mut Reader<F>,
    raw_byte: u8,
    entry: Entry,
) -> (&'static str, String) {
    let operand = match entry.mode {
        Mode::Inherent => {
            if entry.mnemonic == "???" {
                format!("${raw_byte:02X}")
            } else {
                String::new()
            }
        }
        Mode::Imm8 => format!("#${:02X}", r.u8()),
        Mode::Imm16 => format!("#${:04X}", r.u16()),
        Mode::Direct => format!("${:02X}", r.u8()),
        Mode::Extended => format!("${:04X}", r.u16()),
        Mode::Indexed => indexed::decode_indexed(r),
        Mode::Rel8 => {
            let offset = r.u8() as i8 as i16 as u16;
            format!("${:04X}", r.cur.wrapping_add(offset))
        }
        Mode::Rel16 => {
            let offset = r.u16();
            format!("${:04X}", r.cur.wrapping_add(offset))
        }
        Mode::RegPair => {
            let pb = r.u8();
            format!("{},{}", reg_name(pb >> 4), reg_name(pb & 0x0F))
        }
        Mode::StackS => format_stack_mask(r.u8(), true),
        Mode::StackU => format_stack_mask(r.u8(), false),
    };
    (entry.mnemonic, operand)
}

/// Register name for a TFR/EXG postbyte nibble. Codes 0x6, 0x7, 0xC, 0xD,
/// 0xE, 0xF are invalid/reserved on real hardware (see `MC6809::reg_read`'s
/// `_ => 0xFFFF` fallback) — rendered as `?N` rather than guessing a name.
fn reg_name(code: u8) -> String {
    match code {
        regsel::D => "D".to_string(),
        regsel::X => "X".to_string(),
        regsel::Y => "Y".to_string(),
        regsel::U => "U".to_string(),
        regsel::S => "S".to_string(),
        regsel::PC => "PC".to_string(),
        regsel::A => "A".to_string(),
        regsel::B => "B".to_string(),
        regsel::CC => "CC".to_string(),
        regsel::DP => "DP".to_string(),
        _ => format!("?{code:X}"),
    }
}

/// Render a PSHS/PULS/PSHU/PULU mask byte as a comma-separated register list.
/// Display convention (not a hardware fact — the mask is a bitset, not an
/// order): low-to-high bit order, i.e. CC,A,B,DP,X,Y,(U-or-S),PC. `is_s_op`
/// selects whether bit 0x40 ("the *other* stack pointer", see
/// `mc6809::stack_mask::OTHER_STACK_PTR`) names U (for PSHS/PULS) or S (for
/// PSHU/PULU).
fn format_stack_mask(mask: u8, is_s_op: bool) -> String {
    let mut regs: Vec<&str> = Vec::with_capacity(8);
    if mask & stack_mask::CC != 0 {
        regs.push("CC");
    }
    if mask & stack_mask::A != 0 {
        regs.push("A");
    }
    if mask & stack_mask::B != 0 {
        regs.push("B");
    }
    if mask & stack_mask::DP != 0 {
        regs.push("DP");
    }
    if mask & stack_mask::X != 0 {
        regs.push("X");
    }
    if mask & stack_mask::Y != 0 {
        regs.push("Y");
    }
    if mask & stack_mask::OTHER_STACK_PTR != 0 {
        regs.push(if is_s_op { "U" } else { "S" });
    }
    if mask & stack_mask::PC != 0 {
        regs.push("PC");
    }
    regs.join(",")
}
