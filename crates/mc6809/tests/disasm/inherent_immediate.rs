//! Inherent and immediate addressing.

use super::common::disasm_at;

#[test]
fn inherent_nop() {
    let insn = disasm_at(0x1000, &[0x12]);
    assert_eq!(insn.mnemonic, "NOP");
    assert_eq!(insn.operand, "");
    assert_eq!(insn.len, 1);
}

#[test]
fn inherent_rts_abx_rti_mul_sex_daa_sync_swi() {
    for (byte, mnem) in [
        (0x13u8, "SYNC"),
        (0x19, "DAA"),
        (0x1D, "SEX"),
        (0x39, "RTS"),
        (0x3A, "ABX"),
        (0x3B, "RTI"),
        (0x3D, "MUL"),
        (0x3F, "SWI"),
    ] {
        let insn = disasm_at(0x2000, &[byte]);
        assert_eq!(insn.mnemonic, mnem, "opcode ${byte:02X}");
        assert_eq!(insn.operand, "");
        assert_eq!(insn.len, 1);
    }
}

#[test]
fn immediate_8bit_lda() {
    let insn = disasm_at(0x1000, &[0x86, 0x3F]);
    assert_eq!(insn.mnemonic, "LDA");
    assert_eq!(insn.operand, "#$3F");
    assert_eq!(insn.len, 2);
}

#[test]
fn immediate_16bit_ldd() {
    let insn = disasm_at(0x1000, &[0xCC, 0x12, 0x34]);
    assert_eq!(insn.mnemonic, "LDD");
    assert_eq!(insn.operand, "#$1234");
    assert_eq!(insn.len, 3);
}

#[test]
fn orcc_andcc_cwai_are_immediate8() {
    for (byte, mnem) in [(0x1Au8, "ORCC"), (0x1C, "ANDCC"), (0x3C, "CWAI")] {
        let insn = disasm_at(0x1000, &[byte, 0x50]);
        assert_eq!(insn.mnemonic, mnem);
        assert_eq!(insn.operand, "#$50");
        assert_eq!(insn.len, 2);
    }
}
