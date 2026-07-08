//! Test-driven coverage for the disassembler's indexed-addressing postbyte
//! decoder (`disasm::decode_indexed`, exercised through `disassemble`) —
//! mirrors `crates/mc6809/tests/indexed.rs` (the executing-core coverage) but
//! asserts rendered operand text and `len` instead of effective address /
//! cycles.
//!
//! Postbyte reference (bit 7 set): `1 rr i mmmm`. Register field rr: 00=X,
//! 01=Y, 10=U, 11=S. All tests drive `LDA` (opcode 0xA6, indexed) so the only
//! variable is the postbyte (and any offset bytes it pulls in).

use mc6809::disasm::disassemble;

const LDA_INDEXED: u8 = 0xA6;

fn disasm(bytes: &[u8]) -> mc6809::disasm::Insn {
    let addr = 0x1000u16;
    let mem = bytes.to_vec();
    disassemble(
        &mut move |a: u16| {
            let idx = a.wrapping_sub(addr) as usize;
            mem.get(idx).copied().unwrap_or(0)
        },
        addr,
    )
}

// ---- 5-bit constant offset ----------------------------------------------

#[test]
fn offset5_positive() {
    let insn = disasm(&[LDA_INDEXED, 0x05]); // 0RRnnnnn, rr=00(X), n=5
    assert_eq!(insn.mnemonic, "LDA");
    assert_eq!(insn.operand, "5,X");
    assert_eq!(insn.len, 2);
}

#[test]
fn offset5_negative_sign_extends() {
    let insn = disasm(&[LDA_INDEXED, 0x1F]); // -1 in 5 bits
    assert_eq!(insn.operand, "-1,X");
    assert_eq!(insn.len, 2);
}

// ---- no offset --------------------------------------------------------

#[test]
fn no_offset() {
    let insn = disasm(&[LDA_INDEXED, 0x84]); // 1RR00100, rr=00(X)
    assert_eq!(insn.operand, ",X");
    assert_eq!(insn.len, 2);
}

// ---- auto increment / decrement -------------------------------------------

#[test]
fn auto_increment_by_one() {
    let insn = disasm(&[LDA_INDEXED, 0x80]); // ,X+
    assert_eq!(insn.operand, ",X+");
    assert_eq!(insn.len, 2);
}

#[test]
fn auto_increment_by_two() {
    let insn = disasm(&[LDA_INDEXED, 0x81]); // ,X++
    assert_eq!(insn.operand, ",X++");
    assert_eq!(insn.len, 2);
}

#[test]
fn auto_decrement_by_one() {
    let insn = disasm(&[LDA_INDEXED, 0x82]); // ,-X
    assert_eq!(insn.operand, ",-X");
    assert_eq!(insn.len, 2);
}

#[test]
fn auto_decrement_by_two() {
    let insn = disasm(&[LDA_INDEXED, 0x83]); // ,--X
    assert_eq!(insn.operand, ",--X");
    assert_eq!(insn.len, 2);
}

// ---- 8/16-bit constant offset -------------------------------------------

#[test]
fn offset8_positive() {
    let insn = disasm(&[LDA_INDEXED, 0x88, 0x10]); // $10,X
    assert_eq!(insn.operand, "16,X");
    assert_eq!(insn.len, 3);
}

#[test]
fn offset8_negative() {
    let insn = disasm(&[LDA_INDEXED, 0x88, 0xFF]); // -1,X (8-bit form)
    assert_eq!(insn.operand, "-1,X");
    assert_eq!(insn.len, 3);
}

#[test]
fn offset16_positive() {
    let insn = disasm(&[LDA_INDEXED, 0x89, 0x01, 0x00]); // $0100,X
    assert_eq!(insn.operand, "256,X");
    assert_eq!(insn.len, 4);
}

#[test]
fn offset16_negative() {
    let insn = disasm(&[LDA_INDEXED, 0x89, 0xFF, 0xFF]); // -1,X (16-bit form)
    assert_eq!(insn.operand, "-1,X");
    assert_eq!(insn.len, 4);
}

// ---- accumulator offsets -------------------------------------------------

#[test]
fn accumulator_a_offset() {
    let insn = disasm(&[LDA_INDEXED, 0x86]); // A,X
    assert_eq!(insn.operand, "A,X");
    assert_eq!(insn.len, 2);
}

#[test]
fn accumulator_b_offset() {
    let insn = disasm(&[LDA_INDEXED, 0x85]); // B,X
    assert_eq!(insn.operand, "B,X");
    assert_eq!(insn.len, 2);
}

#[test]
fn accumulator_d_offset() {
    let insn = disasm(&[LDA_INDEXED, 0x8B]); // D,X
    assert_eq!(insn.operand, "D,X");
    assert_eq!(insn.len, 2);
}

// ---- PC-relative --------------------------------------------------------

#[test]
fn pc_relative_8bit() {
    let insn = disasm(&[LDA_INDEXED, 0x8C, 0x10]); // n,PCR (8-bit), raw offset 0x10
    assert_eq!(insn.operand, "16,PCR");
    assert_eq!(insn.len, 3);
}

#[test]
fn pc_relative_8bit_negative() {
    let insn = disasm(&[LDA_INDEXED, 0x8C, 0xFF]); // -1,PCR
    assert_eq!(insn.operand, "-1,PCR");
    assert_eq!(insn.len, 3);
}

#[test]
fn pc_relative_16bit() {
    let insn = disasm(&[LDA_INDEXED, 0x8D, 0x01, 0x00]); // 256,PCR
    assert_eq!(insn.operand, "256,PCR");
    assert_eq!(insn.len, 4);
}

// ---- indirect forms -----------------------------------------------------

#[test]
fn indirect_no_offset() {
    let insn = disasm(&[LDA_INDEXED, 0x94]); // [,X]
    assert_eq!(insn.operand, "[,X]");
    assert_eq!(insn.len, 2);
}

#[test]
fn indirect_8bit_offset() {
    let insn = disasm(&[LDA_INDEXED, 0x98, 0x10]); // [16,X]
    assert_eq!(insn.operand, "[16,X]");
    assert_eq!(insn.len, 3);
}

#[test]
fn indirect_16bit_offset() {
    let insn = disasm(&[LDA_INDEXED, 0x99, 0x01, 0x00]); // [256,X]
    assert_eq!(insn.operand, "[256,X]");
    assert_eq!(insn.len, 4);
}

#[test]
fn indirect_d_offset() {
    let insn = disasm(&[LDA_INDEXED, 0x9B]); // [D,X]
    assert_eq!(insn.operand, "[D,X]");
    assert_eq!(insn.len, 2);
}

#[test]
fn indirect_auto_increment_by_two() {
    // ,X++ is the only auto-inc/dec form the datasheet calls indirectable;
    // the core applies indirection uniformly regardless (see `ea_indexed`
    // doc comment) — disasm mirrors that.
    let insn = disasm(&[LDA_INDEXED, 0x91]); // [,X++]
    assert_eq!(insn.operand, "[,X++]");
    assert_eq!(insn.len, 2);
}

#[test]
fn indirect_pcr_8bit() {
    let insn = disasm(&[LDA_INDEXED, 0x9C, 0x10]); // [16,PCR]
    assert_eq!(insn.operand, "[16,PCR]");
    assert_eq!(insn.len, 3);
}

#[test]
fn extended_indirect() {
    let insn = disasm(&[LDA_INDEXED, 0x9F, 0x30, 0x00]); // [$3000]
    assert_eq!(insn.operand, "[$3000]");
    assert_eq!(insn.len, 4);
}

// ---- register selection ---------------------------------------------------

#[test]
fn register_field_selects_y_u_s() {
    let insn = disasm(&[LDA_INDEXED, 0xA4]); // ,Y  (rr=01)
    assert_eq!(insn.operand, ",Y");

    let insn = disasm(&[LDA_INDEXED, 0xC4]); // ,U  (rr=10)
    assert_eq!(insn.operand, ",U");

    let insn = disasm(&[LDA_INDEXED, 0xE4]); // ,S  (rr=11)
    assert_eq!(insn.operand, ",S");
}

// ---- reserved/illegal sub-modes -------------------------------------------

#[test]
fn reserved_submodes_marked_illegal_but_zero_extra_bytes() {
    // mmmm = 0111, 1010, 1110 are reserved; core falls back to a plain
    // register read with 0 extra bytes. Mnemonic stays LDA (a real opcode —
    // only the postbyte sub-mode is invalid), operand is clearly marked.
    for (postbyte, reg) in [(0x87u8, "X"), (0x8Au8, "X"), (0x8Eu8, "X")] {
        let insn = disasm(&[LDA_INDEXED, postbyte]);
        assert_eq!(insn.mnemonic, "LDA");
        assert_eq!(insn.operand, format!(",{reg}???"));
        assert_eq!(insn.len, 2, "postbyte ${postbyte:02X}");
    }
}

#[test]
fn reserved_submode_with_indirect_bit_still_wraps() {
    // 0x97 = 1 00 1 0111: indirect bit set on a reserved sub-mode. The
    // indirect bit costs no extra *instruction* bytes (only an extra data
    // read at execution time), so len stays 2.
    let insn = disasm(&[LDA_INDEXED, 0x97]);
    assert_eq!(insn.operand, "[,X???]");
    assert_eq!(insn.len, 2);
}
