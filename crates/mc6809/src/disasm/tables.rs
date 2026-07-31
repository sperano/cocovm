//! Opcode -> (mnemonic, mode) tables for the base page and the `$10`/`$11`
//! prefix pages. Mirrors [`crate::MC6809::step`]'s dispatch byte-for-byte —
//! see that function's family-grouped helpers (`exec_*`) for the executing
//! side of the same opcode map.

use super::{Entry, ILLEGAL, Mode, e};

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
/// intercepted in [`super::decode_base`] before reaching here. Anything not
/// matched falls to [`ILLEGAL`] — this covers both the documented single-byte
/// illegal set (0x14, 0x15, 0x18, 0x1B, 0x38, 0x3E) and the "STx immediate"
/// slots (0x87, 0x8F, 0xC7, 0xCD, 0xCF) where storing to an immediate operand
/// is nonsensical (see the crate-level task report for the two of these five
/// — 0x87, 0xCF — that a prior spec pass missed).
pub(super) fn base_entry(op: u8) -> Entry {
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

        0x12
        | 0x13
        | 0x16
        | 0x17
        | 0x19
        | 0x1A
        | 0x1C
        | 0x1D
        | 0x1E
        | 0x1F
        | 0x20..=0x2F
        | 0x30
        | 0x31
        | 0x32
        | 0x33
        | 0x34
        | 0x35
        | 0x36
        | 0x37
        | 0x39
        | 0x3A
        | 0x3B
        | 0x3C
        | 0x3D
        | 0x3F => base_entry_misc(op),

        0x86 | 0x96 | 0xA6 | 0xB6 | 0x97 | 0xA7 | 0xB7 | 0xC6 | 0xD6 | 0xE6 | 0xF6 | 0xD7
        | 0xE7 | 0xF7 | 0xCC | 0xDC | 0xEC | 0xFC | 0xDD | 0xED | 0xFD | 0x8E | 0x9E | 0xAE
        | 0xBE | 0x9F | 0xAF | 0xBF | 0xCE | 0xDE | 0xEE | 0xFE | 0xDF | 0xEF | 0xFF => {
            base_entry_load_store(op)
        }

        0x8B | 0x9B | 0xAB | 0xBB | 0x89 | 0x99 | 0xA9 | 0xB9 | 0x80 | 0x90 | 0xA0 | 0xB0
        | 0x82 | 0x92 | 0xA2 | 0xB2 | 0x81 | 0x91 | 0xA1 | 0xB1 | 0x84 | 0x94 | 0xA4 | 0xB4
        | 0x8A | 0x9A | 0xAA | 0xBA | 0x88 | 0x98 | 0xA8 | 0xB8 | 0x85 | 0x95 | 0xA5 | 0xB5 => {
            base_entry_accum_a(op)
        }

        0xCB | 0xDB | 0xEB | 0xFB | 0xC9 | 0xD9 | 0xE9 | 0xF9 | 0xC0 | 0xD0 | 0xE0 | 0xF0
        | 0xC2 | 0xD2 | 0xE2 | 0xF2 | 0xC1 | 0xD1 | 0xE1 | 0xF1 | 0xC4 | 0xD4 | 0xE4 | 0xF4
        | 0xCA | 0xDA | 0xEA | 0xFA | 0xC8 | 0xD8 | 0xE8 | 0xF8 | 0xC5 | 0xD5 | 0xE5 | 0xF5 => {
            base_entry_accum_b(op)
        }

        0xC3 | 0xD3 | 0xE3 | 0xF3 | 0x83 | 0x93 | 0xA3 | 0xB3 | 0x8C | 0x9C | 0xAC | 0xBC
        | 0x8D | 0x9D | 0xAD | 0xBD => base_entry_wide_and_subr(op),

        _ => ILLEGAL,
    }
}

/// NOP/SYNC/LBRA/LBSR/DAA/ORCC/ANDCC/SEX/EXG/TFR, the short-branch range,
/// LEAX/LEAY/LEAS/LEAU, PSHS/PULS/PSHU/PULU, RTS/ABX/RTI/CWAI/MUL/SWI.
fn base_entry_misc(op: u8) -> Entry {
    use Mode::*;
    match op {
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

        _ => ILLEGAL,
    }
}

/// LDA/STA, LDB/STB, LDD/STD, LDX/STX, LDU/STU — immediate / direct / indexed /
/// extended.
fn base_entry_load_store(op: u8) -> Entry {
    use Mode::*;
    match op {
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

        _ => ILLEGAL,
    }
}

/// ADDA/ADCA/SUBA/SBCA/CMPA/ANDA/ORA/EORA/BITA — immediate / direct / indexed /
/// extended.
fn base_entry_accum_a(op: u8) -> Entry {
    use Mode::*;
    match op {
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

        _ => ILLEGAL,
    }
}

/// ADDB/ADCB/SUBB/SBCB/CMPB/ANDB/ORB/EORB/BITB — immediate / direct / indexed /
/// extended.
fn base_entry_accum_b(op: u8) -> Entry {
    use Mode::*;
    match op {
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

        _ => ILLEGAL,
    }
}

/// ADDD/SUBD/CMPX (immediate / direct / indexed / extended) and the
/// subroutine-call opcodes (BSR, JSR direct/indexed/extended).
fn base_entry_wide_and_subr(op: u8) -> Entry {
    use Mode::*;
    match op {
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
pub(super) fn page10_entry(op2: u8) -> Entry {
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
pub(super) fn page11_entry(op2: u8) -> Entry {
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
