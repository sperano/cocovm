//! Short and long branches (offset resolved to an absolute address).

use super::common::disasm_at;

#[test]
fn short_branch_offset_resolved_to_absolute_forward() {
    // BRA +4 at $1000: opcode@1000, offset@1001 -> next-pc 0x1002; target 0x1006.
    let insn = disasm_at(0x1000, &[0x20, 0x04]);
    assert_eq!(insn.mnemonic, "BRA");
    assert_eq!(insn.operand, "$1006");
    assert_eq!(insn.len, 2);
}

#[test]
fn short_branch_offset_resolved_to_absolute_backward() {
    // BEQ -2 -> self-loop: next-pc 0x1002, target 0x1000.
    let insn = disasm_at(0x1000, &[0x27, 0xFE]);
    assert_eq!(insn.mnemonic, "BEQ");
    assert_eq!(insn.operand, "$1000");
    assert_eq!(insn.len, 2);
}

#[test]
fn all_short_branch_mnemonics() {
    let expected = [
        "BRA", "BRN", "BHI", "BLS", "BCC", "BCS", "BNE", "BEQ", "BVC", "BVS", "BPL", "BMI", "BGE",
        "BLT", "BGT", "BLE",
    ];
    for (nibble, mnem) in expected.iter().enumerate() {
        let opcode = 0x20 + nibble as u8;
        let insn = disasm_at(0x1000, &[opcode, 0x00]);
        assert_eq!(insn.mnemonic, *mnem, "opcode ${opcode:02X}");
        assert_eq!(insn.len, 2);
    }
}

#[test]
fn bsr_is_relative_like_short_branch() {
    let insn = disasm_at(0x1000, &[0x8D, 0x10]); // BSR +16 -> target 0x1012
    assert_eq!(insn.mnemonic, "BSR");
    assert_eq!(insn.operand, "$1012");
    assert_eq!(insn.len, 2);
}

#[test]
fn lbra_long_branch() {
    let insn = disasm_at(0x1000, &[0x16, 0x01, 0x00]); // LBRA +0x100 -> target 0x1103
    assert_eq!(insn.mnemonic, "LBRA");
    assert_eq!(insn.operand, "$1103");
    assert_eq!(insn.len, 3);
}

#[test]
fn lbsr_long_branch() {
    let insn = disasm_at(0x1000, &[0x17, 0x00, 0x10]);
    assert_eq!(insn.mnemonic, "LBSR");
    assert_eq!(insn.operand, "$1013");
    assert_eq!(insn.len, 3);
}
