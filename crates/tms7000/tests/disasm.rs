//! Disassembler: every opcode's length agrees with what `step` consumes,
//! and spellings match MAME's `7000dasm.cpp`.

mod common;

use common::{CODE, Sys};
use tms7000::disasm::{ILLEGAL, disassemble};

fn dis(bytes: &[u8], pc: u16) -> String {
    let mut read = |addr: u16| bytes[usize::from(addr.wrapping_sub(pc))];
    disassemble(&mut read, pc).to_string()
}

/// Opcodes whose PC after execution is not `pc + len`.
fn transfers_control(op: u8) -> bool {
    matches!(
        op,
        0x01 | 0x0A | 0x0B | 0x8C | 0x9C | 0xAC | 0x8E | 0x9E | 0xAE | 0xE8..=0xFF
    )
}

#[test]
fn every_opcode_length_matches_execution() {
    for op in 0..=255u8 {
        if transfers_control(op) {
            continue;
        }
        let code = [op, 0x00, 0x00, 0x00, 0x00];
        let mut s = Sys::code(&code);
        // Zero operands and A = 1 keep every relative branch untaken with
        // displacement 0, so PC advances by exactly the instruction length.
        s.set_a(1);
        s.set_b(1);
        s.insn();
        let mut read = |addr: u16| code[usize::from(addr - CODE)];
        let insn = disassemble(&mut read, CODE);
        assert_eq!(u16::from(insn.len), s.cpu.pc - CODE, "{op:#04x} ({})", insn);
    }
}

#[test]
fn spellings_match_mame() {
    assert_eq!(dis(&[0xA2, 0x00, 0x00], 0xF000), "MOVP %>00,P0");
    assert_eq!(dis(&[0x8A, 0xF1, 0x23], 0xF000), "LDA @>F123");
    assert_eq!(dis(&[0x9C, 0x05], 0xF000), "BR *R5");
    assert_eq!(dis(&[0xAB, 0xF0, 0x00], 0xF000), "STA @>F000(B)");
    assert_eq!(dis(&[0xE0, 0x00], 0xF010), "JMP >F012");
    assert_eq!(dis(&[0xE0, 0xFE], 0xF010), "JMP >F010");
    assert_eq!(dis(&[0x26, 0x01, 0x05], 0xF000), "BTJO %>1,A,>F008");
    assert_eq!(
        dis(&[0x76, 0x80, 0x10, 0xFC], 0xF000),
        "BTJO %>80,R16,>F000"
    );
    assert_eq!(
        dis(&[0xA6, 0x01, 0x08, 0x02], 0xF000),
        "BTJOP %>01,P8,>F006"
    );
    assert_eq!(dis(&[0xFC], 0xF000), "TRAP 3");
    assert_eq!(dis(&[0xB5], 0xF000), "CLR A");
    assert_eq!(dis(&[0x0E], 0xF000), "PUSH ST");
    assert_eq!(
        dis(&[0xB0], 0xF000),
        "CLRC",
        "first table match wins over TSTA"
    );
    assert_eq!(dis(&[0x88, 0x12, 0x34, 0x21], 0xF000), "MOVD %>1234,R33");
    assert_eq!(dis(&[0xA8, 0x12, 0x34, 0x21], 0xF000), "MOVD %>1234(B),R33");
    assert_eq!(dis(&[0x42, 0x10, 0x11], 0xF000), "MOV R16,R17");
    assert_eq!(dis(&[0x22, 0x0F], 0xF000), "MOV %>F,A");
    assert_eq!(dis(&[0x80, 0x04], 0xF000), "MOVP P4,A");
    assert_eq!(dis(&[0xDA, 0x10, 0xFD], 0xF000), "DJNZ R16,>F000");
    assert_eq!(dis(&[0x8E, 0xF2, 0x00], 0xF000), "CALL >F200");
    assert_eq!(dis(&[0x62], 0xF000), "MOV B,A");
    assert_eq!(dis(&[0x6C], 0xF000), "MPY B,A");
}

#[test]
fn illegal_bytes() {
    for op in [0x02u8, 0x0C, 0x81, 0xAF] {
        let insn = disassemble(&mut |_| op, 0xF000);
        assert_eq!(insn.mnemonic, ILLEGAL, "{op:#04x}");
        assert_eq!(insn.len, 1);
    }
}

/// `$B1` is undocumented but real: `execute_one` runs it as MOV B,A, so the
/// disassembler must name it rather than report it illegal.
#[test]
fn b1_disassembles_as_mov_b_a() {
    assert_eq!(dis(&[0xB1], 0xF000), "MOV B,A");
}

/// Every one of the 256 opcodes must agree between the two independent
/// decode tables: whether execution flags it illegal (`illegal_count`
/// increments) and whether the disassembler names it. A legality change on
/// either side that isn't mirrored on the other fails this test.
#[test]
fn execution_and_disassembly_legality_agree() {
    for op in 0..=255u8 {
        let code = [op, 0x00, 0x00, 0x00, 0x00];
        let mut s = Sys::code(&code);
        s.set_a(1);
        s.set_b(1);
        let before = s.cpu.illegal_count;
        s.insn();
        let executed_illegal = s.cpu.illegal_count != before;

        let mut read = |addr: u16| code[usize::from(addr - CODE)];
        let insn = disassemble(&mut read, CODE);
        let disassembled_illegal = insn.mnemonic == ILLEGAL;

        assert_eq!(
            executed_illegal, disassembled_illegal,
            "{op:#04x}: execute illegal={executed_illegal}, disasm illegal={disassembled_illegal} ({insn})"
        );
    }
}
