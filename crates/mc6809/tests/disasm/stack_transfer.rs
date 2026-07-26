//! PSHS/PULS/PSHU/PULU register masks and TFR/EXG register pairs.

use super::common::disasm_at;

#[test]
fn pshs_all_registers_low_to_high_order() {
    let insn = disasm_at(0x1000, &[0x34, 0xFF]);
    assert_eq!(insn.mnemonic, "PSHS");
    assert_eq!(insn.operand, "CC,A,B,DP,X,Y,U,PC");
    assert_eq!(insn.len, 2);
}

#[test]
fn puls_other_stack_ptr_bit_names_u() {
    // mask 0x40 alone: PULS names the "other stack pointer" bit U.
    let insn = disasm_at(0x1000, &[0x35, 0x40]);
    assert_eq!(insn.mnemonic, "PULS");
    assert_eq!(insn.operand, "U");
    assert_eq!(insn.len, 2);
}

#[test]
fn pshu_other_stack_ptr_bit_names_s() {
    let insn = disasm_at(0x1000, &[0x36, 0x40]);
    assert_eq!(insn.mnemonic, "PSHU");
    assert_eq!(insn.operand, "S");
    assert_eq!(insn.len, 2);
}

#[test]
fn pulu_partial_mask() {
    // mask 0x16 = A(0x02)+B(0x04)+X(0x10)
    let insn = disasm_at(0x1000, &[0x37, 0x16]);
    assert_eq!(insn.mnemonic, "PULU");
    assert_eq!(insn.operand, "A,B,X");
    assert_eq!(insn.len, 2);
}

#[test]
fn tfr_16bit_pair() {
    let insn = disasm_at(0x1000, &[0x1F, 0x12]); // TFR X,Y (src=1,dst=2)
    assert_eq!(insn.mnemonic, "TFR");
    assert_eq!(insn.operand, "X,Y");
    assert_eq!(insn.len, 2);
}

#[test]
fn exg_8bit_pair() {
    let insn = disasm_at(0x1000, &[0x1E, 0x89]); // EXG A,B (src=8,dst=9)
    assert_eq!(insn.mnemonic, "EXG");
    assert_eq!(insn.operand, "A,B");
    assert_eq!(insn.len, 2);
}

#[test]
fn tfr_invalid_register_code_marked_clearly() {
    let insn = disasm_at(0x1000, &[0x1F, 0x06]); // src=0 (D), dst=6 (invalid)
    assert_eq!(insn.mnemonic, "TFR");
    assert_eq!(insn.operand, "D,?6");
    assert_eq!(insn.len, 2);
}
