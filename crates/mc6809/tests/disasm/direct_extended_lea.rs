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
    // 0x43 = COMA (0x40-0x4F, nibble 3 = COM); 0x5B = XDECB (undocumented).
    let insn = disasm_at(0x1000, &[0x43]);
    assert_eq!(insn.mnemonic, "COMA");
    assert_eq!(insn.operand, "");
    assert_eq!(insn.len, 1);

    let insn = disasm_at(0x1000, &[0x5B]);
    assert_eq!(insn.mnemonic, "XDECB");
    assert_eq!(insn.operand, "");
    assert_eq!(insn.len, 1);
}

#[test]
fn direct_rmw_undocumented_alias_consumes_direct_byte() {
    // 0x01 is the undocumented direct-mode NEG alias.
    let insn = disasm_at(0x1000, &[0x01, 0x42]);
    assert_eq!(insn.mnemonic, "NEG");
    assert_eq!(insn.operand, "$42");
    assert_eq!(insn.len, 2);
}

#[test]
fn undocumented_rmw_aliases_have_named_disassembly() {
    for (opcode, mnemonic) in [(0x02, "XNC"), (0x05, "LSR"), (0x0B, "XDEC")] {
        let insn = disasm_at(0x1000, &[opcode, 0x42]);
        assert_eq!(insn.mnemonic, mnemonic, "opcode ${opcode:02X}");
        assert_eq!(insn.operand, "$42", "opcode ${opcode:02X}");
        assert_eq!(insn.len, 2, "opcode ${opcode:02X}");
    }

    let insn = disasm_at(0x1000, &[0x4E]);
    assert_eq!(insn.mnemonic, "XCLRA");
    assert_eq!(insn.operand, "");
    assert_eq!(insn.len, 1);
}

#[test]
fn leax_indexed() {
    let insn = disasm_at(0x1000, &[0x30, 0x05]); // LEAX 5,X
    assert_eq!(insn.mnemonic, "LEAX");
    assert_eq!(insn.operand, "5,X");
    assert_eq!(insn.len, 2);
}
