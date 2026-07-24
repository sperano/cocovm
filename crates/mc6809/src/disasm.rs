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
//! ([`base_entry`], [`page10_entry`], [`page11_entry`], and the small
//! per-nibble arrays for the read-modify-write group and the branch-condition
//! group); "how do I render addressing mode Y into an operand string" is
//! shared logic ([`render`], [`decode_indexed`]) used by every entry that
//! shares that mode. This separation is what lets HD6309-specific mnemonics
//! be layered in later without restructuring.
//!
//! Illegal opcodes (including ones the real core silently falls through to a
//! 2-cycle default for) disassemble as mnemonic `"???"` with a length that
//! still matches exactly what [`MC6809::step`](crate::MC6809::step) would
//! have consumed for the same bytes — so a scrolling disassembly view never
//! desyncs from the byte stream, even across undecoded opcodes.

use crate::{postbyte, regsel, stack_mask};

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

// ---- Read-modify-write nibble tables ------------------------------------
// Low nibble of opcodes 0x00-0x0F/0x40-0x4F/0x50-0x5F/0x60-0x6F/0x70-0x7F
// selects the op (mirrors `MC6809::rmw_apply`). Nibbles 1,2,5,B,E are
// illegal on real hardware. ASL/LSL are the same opcode; the PDF's primary
// entry (and the one used throughout this crate/tests) is ASL — see the
// "ASL vs LSL" note in the crate-level task report.
const RMW_MEM: [&str; 16] = [
    "NEG", "???", "???", "COM", "LSR", "???", "ROR", "ASR", "ASL", "ROL", "DEC", "???", "INC",
    "TST", "???", "CLR",
];
const RMW_A: [&str; 16] = [
    "NEGA", "???", "???", "COMA", "LSRA", "???", "RORA", "ASRA", "ASLA", "ROLA", "DECA", "???",
    "INCA", "TSTA", "???", "CLRA",
];
const RMW_B: [&str; 16] = [
    "NEGB", "???", "???", "COMB", "LSRB", "???", "RORB", "ASRB", "ASLB", "ROLB", "DECB", "???",
    "INCB", "TSTB", "???", "CLRB",
];

fn rmw_entry(nibble: u8, table: &[&'static str; 16], mode: Mode) -> Entry {
    e(table[(nibble & 0x0F) as usize], mode)
}

// ---- Branch condition tables ---------------------------------------------
// Same low-nibble numbering as `MC6809::branch_taken` (0=always..F=BLE).
// Canonical spellings per docs/6x09_Instruction_Sets.pdf: the PDF gives BCC
// and BCS their own primary entries and documents BHS/BLO as assembler
// *alternate* mnemonics for BCC/BCS respectively ("BHS is an alternate
// mnemonic for the BCC instruction. Both produce the same object code.") —
// so BCC/BCS are used here, not BHS/BLO.
const SHORT_BRANCH: [&str; 16] = [
    "BRA", "BRN", "BHI", "BLS", "BCC", "BCS", "BNE", "BEQ", "BVC", "BVS", "BPL", "BMI", "BGE",
    "BLT", "BGT", "BLE",
];
const LONG_BRANCH: [&str; 16] = [
    "LBRA", "LBRN", "LBHI", "LBLS", "LBCC", "LBCS", "LBNE", "LBEQ", "LBVC", "LBVS", "LBPL", "LBMI",
    "LBGE", "LBLT", "LBGT", "LBLE",
];

// ---- Base page ------------------------------------------------------------

/// Opcode -> (mnemonic, mode) for the base (unprefixed) page. `$10`/`$11` are
/// intercepted in [`decode_base`] before reaching here. Anything not matched
/// falls to [`ILLEGAL`] — this covers both the documented single-byte illegal
/// set (0x14, 0x15, 0x18, 0x1B, 0x38, 0x3E) and the "STx immediate" slots
/// (0x87, 0x8F, 0xC7, 0xCD, 0xCF) where storing to an immediate operand is
/// nonsensical (see the crate-level task report for the two of these five
/// — 0x87, 0xCF — that a prior spec pass missed).
fn base_entry(op: u8) -> Entry {
    use Mode::*;
    match op {
        // JMP is spliced into the RMW ranges and MUST be checked before the
        // generic nibble dispatch (mirrors the ordering note in `step`).
        0x0E => e("JMP", Direct),
        0x6E => e("JMP", Indexed),
        0x7E => e("JMP", Extended),

        0x00..=0x0F => rmw_entry(op, &RMW_MEM, Direct),
        0x40..=0x4F => rmw_entry(op, &RMW_A, Inherent),
        0x50..=0x5F => rmw_entry(op, &RMW_B, Inherent),
        0x60..=0x6F => rmw_entry(op, &RMW_MEM, Indexed),
        0x70..=0x7F => rmw_entry(op, &RMW_MEM, Extended),

        0x12 => e("NOP", Inherent),
        0x13 => e("SYNC", Inherent),
        0x16 => e("LBRA", Rel16),
        0x17 => e("LBSR", Rel16),
        0x19 => e("DAA", Inherent),
        0x1A => e("ORCC", Imm8),
        0x1C => e("ANDCC", Imm8),
        0x1D => e("SEX", Inherent),
        0x1E => e("EXG", RegPair),
        0x1F => e("TFR", RegPair),

        0x20..=0x2F => e(SHORT_BRANCH[(op & 0x0F) as usize], Rel8),

        0x30 => e("LEAX", Indexed),
        0x31 => e("LEAY", Indexed),
        0x32 => e("LEAS", Indexed),
        0x33 => e("LEAU", Indexed),
        0x34 => e("PSHS", StackS),
        0x35 => e("PULS", StackS),
        0x36 => e("PSHU", StackU),
        0x37 => e("PULU", StackU),
        0x39 => e("RTS", Inherent),
        0x3A => e("ABX", Inherent),
        0x3B => e("RTI", Inherent),
        0x3C => e("CWAI", Imm8),
        0x3D => e("MUL", Inherent),
        0x3F => e("SWI", Inherent),

        // LDA/STA
        0x86 => e("LDA", Imm8),
        0x96 => e("LDA", Direct),
        0xA6 => e("LDA", Indexed),
        0xB6 => e("LDA", Extended),
        0x97 => e("STA", Direct),
        0xA7 => e("STA", Indexed),
        0xB7 => e("STA", Extended),
        // LDB/STB
        0xC6 => e("LDB", Imm8),
        0xD6 => e("LDB", Direct),
        0xE6 => e("LDB", Indexed),
        0xF6 => e("LDB", Extended),
        0xD7 => e("STB", Direct),
        0xE7 => e("STB", Indexed),
        0xF7 => e("STB", Extended),
        // LDD/STD
        0xCC => e("LDD", Imm16),
        0xDC => e("LDD", Direct),
        0xEC => e("LDD", Indexed),
        0xFC => e("LDD", Extended),
        0xDD => e("STD", Direct),
        0xED => e("STD", Indexed),
        0xFD => e("STD", Extended),
        // LDX/STX
        0x8E => e("LDX", Imm16),
        0x9E => e("LDX", Direct),
        0xAE => e("LDX", Indexed),
        0xBE => e("LDX", Extended),
        0x9F => e("STX", Direct),
        0xAF => e("STX", Indexed),
        0xBF => e("STX", Extended),
        // LDU/STU
        0xCE => e("LDU", Imm16),
        0xDE => e("LDU", Direct),
        0xEE => e("LDU", Indexed),
        0xFE => e("LDU", Extended),
        0xDF => e("STU", Direct),
        0xEF => e("STU", Indexed),
        0xFF => e("STU", Extended),

        // ADDA/ADCA/SUBA/SBCA/CMPA/ANDA/ORA/EORA/BITA
        0x8B => e("ADDA", Imm8),
        0x9B => e("ADDA", Direct),
        0xAB => e("ADDA", Indexed),
        0xBB => e("ADDA", Extended),
        0x89 => e("ADCA", Imm8),
        0x99 => e("ADCA", Direct),
        0xA9 => e("ADCA", Indexed),
        0xB9 => e("ADCA", Extended),
        0x80 => e("SUBA", Imm8),
        0x90 => e("SUBA", Direct),
        0xA0 => e("SUBA", Indexed),
        0xB0 => e("SUBA", Extended),
        0x82 => e("SBCA", Imm8),
        0x92 => e("SBCA", Direct),
        0xA2 => e("SBCA", Indexed),
        0xB2 => e("SBCA", Extended),
        0x81 => e("CMPA", Imm8),
        0x91 => e("CMPA", Direct),
        0xA1 => e("CMPA", Indexed),
        0xB1 => e("CMPA", Extended),
        0x84 => e("ANDA", Imm8),
        0x94 => e("ANDA", Direct),
        0xA4 => e("ANDA", Indexed),
        0xB4 => e("ANDA", Extended),
        0x8A => e("ORA", Imm8),
        0x9A => e("ORA", Direct),
        0xAA => e("ORA", Indexed),
        0xBA => e("ORA", Extended),
        0x88 => e("EORA", Imm8),
        0x98 => e("EORA", Direct),
        0xA8 => e("EORA", Indexed),
        0xB8 => e("EORA", Extended),
        0x85 => e("BITA", Imm8),
        0x95 => e("BITA", Direct),
        0xA5 => e("BITA", Indexed),
        0xB5 => e("BITA", Extended),

        // ADDB/ADCB/SUBB/SBCB/CMPB/ANDB/ORB/EORB/BITB
        0xCB => e("ADDB", Imm8),
        0xDB => e("ADDB", Direct),
        0xEB => e("ADDB", Indexed),
        0xFB => e("ADDB", Extended),
        0xC9 => e("ADCB", Imm8),
        0xD9 => e("ADCB", Direct),
        0xE9 => e("ADCB", Indexed),
        0xF9 => e("ADCB", Extended),
        0xC0 => e("SUBB", Imm8),
        0xD0 => e("SUBB", Direct),
        0xE0 => e("SUBB", Indexed),
        0xF0 => e("SUBB", Extended),
        0xC2 => e("SBCB", Imm8),
        0xD2 => e("SBCB", Direct),
        0xE2 => e("SBCB", Indexed),
        0xF2 => e("SBCB", Extended),
        0xC1 => e("CMPB", Imm8),
        0xD1 => e("CMPB", Direct),
        0xE1 => e("CMPB", Indexed),
        0xF1 => e("CMPB", Extended),
        0xC4 => e("ANDB", Imm8),
        0xD4 => e("ANDB", Direct),
        0xE4 => e("ANDB", Indexed),
        0xF4 => e("ANDB", Extended),
        0xCA => e("ORB", Imm8),
        0xDA => e("ORB", Direct),
        0xEA => e("ORB", Indexed),
        0xFA => e("ORB", Extended),
        0xC8 => e("EORB", Imm8),
        0xD8 => e("EORB", Direct),
        0xE8 => e("EORB", Indexed),
        0xF8 => e("EORB", Extended),
        0xC5 => e("BITB", Imm8),
        0xD5 => e("BITB", Direct),
        0xE5 => e("BITB", Indexed),
        0xF5 => e("BITB", Extended),

        // ADDD/SUBD/CMPX
        0xC3 => e("ADDD", Imm16),
        0xD3 => e("ADDD", Direct),
        0xE3 => e("ADDD", Indexed),
        0xF3 => e("ADDD", Extended),
        0x83 => e("SUBD", Imm16),
        0x93 => e("SUBD", Direct),
        0xA3 => e("SUBD", Indexed),
        0xB3 => e("SUBD", Extended),
        0x8C => e("CMPX", Imm16),
        0x9C => e("CMPX", Direct),
        0xAC => e("CMPX", Indexed),
        0xBC => e("CMPX", Extended),

        // Subroutine calls
        0x8D => e("BSR", Rel8),
        0x9D => e("JSR", Direct),
        0xAD => e("JSR", Indexed),
        0xBD => e("JSR", Extended),

        _ => ILLEGAL,
    }
}

/// `$10`-prefixed page: long conditional branches, CMPD/CMPY/LDY/STY/LDS/STS,
/// SWI2. Everything else is undecoded by the core (falls to its 2-cycle
/// default after only the prefix + second byte are read) and disassembles as
/// `"???"` at length 2.
fn page10_entry(op2: u8) -> Entry {
    use Mode::*;
    match op2 {
        0x21..=0x2F => e(LONG_BRANCH[(op2 & 0x0F) as usize], Rel16),

        0x83 => e("CMPD", Imm16),
        0x93 => e("CMPD", Direct),
        0xA3 => e("CMPD", Indexed),
        0xB3 => e("CMPD", Extended),
        0x8C => e("CMPY", Imm16),
        0x9C => e("CMPY", Direct),
        0xAC => e("CMPY", Indexed),
        0xBC => e("CMPY", Extended),

        0x8E => e("LDY", Imm16),
        0x9E => e("LDY", Direct),
        0xAE => e("LDY", Indexed),
        0xBE => e("LDY", Extended),
        0x9F => e("STY", Direct),
        0xAF => e("STY", Indexed),
        0xBF => e("STY", Extended),

        0xCE => e("LDS", Imm16),
        0xDE => e("LDS", Direct),
        0xEE => e("LDS", Indexed),
        0xFE => e("LDS", Extended),
        0xDF => e("STS", Direct),
        0xEF => e("STS", Indexed),
        0xFF => e("STS", Extended),

        0x3F => e("SWI2", Inherent),

        _ => ILLEGAL,
    }
}

/// `$11`-prefixed page: CMPU, CMPS, SWI3. Everything else undecoded (see
/// [`page10_entry`] doc for the fallback rule).
fn page11_entry(op2: u8) -> Entry {
    use Mode::*;
    match op2 {
        0x83 => e("CMPU", Imm16),
        0x93 => e("CMPU", Direct),
        0xA3 => e("CMPU", Indexed),
        0xB3 => e("CMPU", Extended),
        0x8C => e("CMPS", Imm16),
        0x9C => e("CMPS", Direct),
        0xAC => e("CMPS", Indexed),
        0xBC => e("CMPS", Extended),

        0x3F => e("SWI3", Inherent),

        _ => ILLEGAL,
    }
}

fn decode_base<F: FnMut(u16) -> u8>(r: &mut Reader<F>, opcode: u8) -> (&'static str, String) {
    match opcode {
        0x10 => {
            let op2 = r.u8();
            render(r, op2, page10_entry(op2))
        }
        0x11 => {
            let op2 = r.u8();
            render(r, op2, page11_entry(op2))
        }
        _ => render(r, opcode, base_entry(opcode)),
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
        Mode::Indexed => decode_indexed(r),
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

/// Decode one indexed-addressing postbyte into its operand string, mirroring
/// `MC6809::ea_indexed` byte-for-byte (see that function's doc comment for
/// the postbyte layout: `1 rr i mmmm` when bit 7 is set, `0 rr nnnnn` — a
/// non-indirectable 5-bit constant offset — when clear).
///
/// Numeric constant offsets (5-bit, 8-bit, 16-bit, and both PCR forms) are
/// rendered in signed decimal (e.g. `5,Y`, `-1,X`, `300,PCR`) rather than
/// hex — see the crate-level task report's "indexed offset radix" judgment
/// call, which follows the task spec's own `LDX 5,Y` example literally.
/// Absolute addresses (extended indirect `[$XXXX]`) stay hex.
fn decode_indexed<F: FnMut(u16) -> u8>(r: &mut Reader<F>) -> String {
    let pb = r.u8();

    // 5-bit signed constant offset — not indirectable (bit 7 clear).
    if pb & 0x80 == 0 {
        let reg = index_reg_name(pb >> postbyte::REG_SHIFT);
        let n = pb & postbyte::OFFSET5_MASK;
        let offset: i16 = if n & postbyte::OFFSET5_SIGN != 0 {
            n as i16 - (postbyte::OFFSET5_SIGN as i16 * 2)
        } else {
            n as i16
        };
        return format!("{offset},{reg}");
    }

    let sel = pb >> postbyte::REG_SHIFT;
    let indirect = pb & postbyte::INDIRECT != 0;
    let reg = index_reg_name(sel);

    let body = match pb & postbyte::MODE_MASK {
        0b0000 => format!(",{reg}+"),
        0b0001 => format!(",{reg}++"),
        0b0010 => format!(",-{reg}"),
        0b0011 => format!(",--{reg}"),
        0b0100 => format!(",{reg}"),
        0b0101 => format!("B,{reg}"),
        0b0110 => format!("A,{reg}"),
        0b1000 => {
            let offset = r.u8() as i8;
            format!("{offset},{reg}")
        }
        0b1001 => {
            let offset = r.u16() as i16;
            format!("{offset},{reg}")
        }
        0b1011 => format!("D,{reg}"),
        0b1100 => {
            let offset = r.u8() as i8;
            format!("{offset},PCR")
        }
        0b1101 => {
            let offset = r.u16() as i16;
            format!("{offset},PCR")
        }
        0b1111 => {
            // Extended indirect: register field ignored. The `[...]` wrap is
            // applied uniformly below via the postbyte's indirect bit, same
            // as every other sub-mode (the core does not special-case this
            // one — see `ea_indexed`'s doc comment).
            let addr = r.u16();
            format!("${addr:04X}")
        }
        // Reserved/illegal postbytes (0b0111, 0b1010, 0b1110): the core
        // falls back to a plain register read with 0 extra bytes. Marked
        // with a `???` suffix so this doesn't read as a valid addressing
        // form.
        _ => format!(",{reg}???"),
    };

    if indirect {
        format!("[{body}]")
    } else {
        body
    }
}

/// Index register name for the indexed-postbyte `rr` field (raw, unmasked —
/// callers pass `pb >> REG_SHIFT` directly, matching `MC6809::index_reg`'s
/// own calling convention, which masks internally).
fn index_reg_name(sel: u8) -> &'static str {
    match sel & 0b11 {
        0b00 => "X",
        0b01 => "Y",
        0b10 => "U",
        _ => "S",
    }
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
