//! Direct/extended addressing and LEA.

use super::common::disasm_at;

#[test]
fn direct_sta() {
    let insn = disasm_at(0x1000, &[0x97, 0x80]);
    assert_eq!(insn.mnemonic, "STA");
    assert_eq!(insn.operand, "$80");
    assert_eq!(insn.len, 2);
}

#[test]
fn extended_jsr() {
    let insn = disasm_at(0x1000, &[0xBD, 0xA9, 0x28]);
    assert_eq!(insn.mnemonic, "JSR");
    assert_eq!(insn.operand, "$A928");
    assert_eq!(insn.len, 3);
}

#[test]
fn extended_clr_from_rmw_group() {
    // 0x7F: extended-mode RMW, low nibble 0xF = CLR.
    let insn = disasm_at(0x1000, &[0x7F, 0xFF, 0x90]);
    assert_eq!(insn.mnemonic, "CLR");
    assert_eq!(insn.operand, "$FF90");
    assert_eq!(insn.len, 3);
}

#[test]
fn inherent_rmw_on_accumulators() {
    // 0x43 = COMA (0x40-0x4F, nibble 3 = COM); 0x5B = illegal nibble (0xB) on B.
    let insn = disasm_at(0x1000, &[0x43]);
    assert_eq!(insn.mnemonic, "COMA");
    assert_eq!(insn.operand, "");
    assert_eq!(insn.len, 1);

    let insn = disasm_at(0x1000, &[0x5B]);
    assert_eq!(insn.mnemonic, "???");
    assert_eq!(insn.operand, "$5B");
    assert_eq!(insn.len, 1);
}

#[test]
fn direct_rmw_illegal_nibble_still_consumes_direct_byte() {
    // 0x01: direct-mode RMW, nibble 1 is illegal — but `step()` calls
    // `ea_direct` unconditionally before checking the nibble, so the direct
    // byte IS consumed (len stays 2, matching the addressing mode).
    let insn = disasm_at(0x1000, &[0x01, 0x42]);
    assert_eq!(insn.mnemonic, "???");
    assert_eq!(insn.operand, "$42");
    assert_eq!(insn.len, 2);
}

#[test]
fn leax_indexed() {
    let insn = disasm_at(0x1000, &[0x30, 0x05]); // LEAX 5,X
    assert_eq!(insn.mnemonic, "LEAX");
    assert_eq!(insn.operand, "5,X");
    assert_eq!(insn.len, 2);
}
