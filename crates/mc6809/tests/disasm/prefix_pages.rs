//! Both prefix pages ($10/$11) and illegal opcodes in all three pages.

use super::common::disasm_at;

#[test]
fn page10_long_branch() {
    let insn = disasm_at(0x1000, &[0x10, 0x26, 0x00, 0x05]); // LBNE +5 -> target 0x1009
    assert_eq!(insn.mnemonic, "LBNE");
    assert_eq!(insn.operand, "$1009");
    assert_eq!(insn.len, 4);
}

#[test]
fn page10_ldy_immediate_and_sty_extended() {
    let insn = disasm_at(0x1000, &[0x10, 0x8E, 0x20, 0x00]);
    assert_eq!(insn.mnemonic, "LDY");
    assert_eq!(insn.operand, "#$2000");
    assert_eq!(insn.len, 4);

    let insn = disasm_at(0x1000, &[0x10, 0xBF, 0x30, 0x00]);
    assert_eq!(insn.mnemonic, "STY");
    assert_eq!(insn.operand, "$3000");
    assert_eq!(insn.len, 4);
}

#[test]
fn page10_swi2() {
    let insn = disasm_at(0x1000, &[0x10, 0x3F]);
    assert_eq!(insn.mnemonic, "SWI2");
    assert_eq!(insn.operand, "");
    assert_eq!(insn.len, 2);
}

#[test]
fn page10_illegal_second_byte_is_length_2() {
    // 0x10 0x00: not decoded by the core (falls to its 2-cycle default after
    // only the prefix + second byte are read).
    let insn = disasm_at(0x1000, &[0x10, 0x00]);
    assert_eq!(insn.mnemonic, "???");
    assert_eq!(insn.operand, "$00");
    assert_eq!(insn.len, 2);
}

#[test]
fn page11_cmpu_and_cmps() {
    let insn = disasm_at(0x1000, &[0x11, 0x83, 0x00, 0x10]);
    assert_eq!(insn.mnemonic, "CMPU");
    assert_eq!(insn.operand, "#$0010");
    assert_eq!(insn.len, 4);

    let insn = disasm_at(0x1000, &[0x11, 0xAC, 0x84]); // CMPS ,X
    assert_eq!(insn.mnemonic, "CMPS");
    assert_eq!(insn.operand, ",X");
    assert_eq!(insn.len, 3);
}

#[test]
fn page11_swi3() {
    let insn = disasm_at(0x1000, &[0x11, 0x3F]);
    assert_eq!(insn.mnemonic, "SWI3");
    assert_eq!(insn.operand, "");
    assert_eq!(insn.len, 2);
}

#[test]
fn page11_illegal_second_byte_is_length_2() {
    let insn = disasm_at(0x1000, &[0x11, 0xFF]);
    assert_eq!(insn.mnemonic, "???");
    assert_eq!(insn.operand, "$FF");
    assert_eq!(insn.len, 2);
}

#[test]
fn illegal_base_page_opcodes_are_length_1() {
    // The documented illegal set (0x14, 0x15, 0x18, 0x1B, 0x38, 0x3E) plus the
    // "STx immediate" slots that don't exist on real hardware: 0x87 (STA),
    // 0x8F (STX), 0xC7 (STB), 0xCD (STD), 0xCF (STU) — see the crate-level
    // task report for the two of these five a prior spec pass missed.
    for byte in [
        0x14u8, 0x15, 0x18, 0x1B, 0x38, 0x3E, 0x87, 0x8F, 0xC7, 0xCD, 0xCF,
    ] {
        let insn = disasm_at(0x1000, &[byte]);
        assert_eq!(insn.mnemonic, "???", "opcode ${byte:02X}");
        assert_eq!(insn.operand, format!("${byte:02X}"));
        assert_eq!(insn.len, 1, "opcode ${byte:02X}");
    }
}
