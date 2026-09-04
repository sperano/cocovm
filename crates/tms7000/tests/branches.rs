//! Relative branches: JMP/Jcc (`$E0-$E7`), BTJO/BTJZ, DJNZ. Costs are the
//! row base plus 2 for the displacement and 2 more when taken (MAME
//! `shortbranch`/`jmp`).

mod common;

use common::{CODE, Sys};
use tms7000::st;

#[test]
fn jmp_costs_seven_and_is_relative_to_the_next_instruction() {
    let mut s = Sys::code(&[0xE0, 0x10]);
    assert_eq!(s.insn(), 7);
    assert_eq!(s.cpu.pc, CODE + 2 + 0x10);

    let mut s = Sys::code(&[0xE0, 0xFE]); // JMP to itself
    s.insn();
    assert_eq!(s.cpu.pc, CODE);
}

#[test]
fn conditional_jumps_cost_five_when_not_taken() {
    // (opcode, ST value that takes the branch, ST value that doesn't)
    const CASES: [(u8, u8, u8); 7] = [
        (0xE1, st::N, 0), // JN
        (0xE2, st::Z, 0), // JZ
        (0xE3, st::C, 0), // JC
        (0xE4, 0, st::N), // JP: !(Z|N)
        (0xE5, 0, st::N), // JPZ: !N
        (0xE6, 0, st::Z), // JNZ
        (0xE7, 0, st::C), // JNC
    ];
    for (op, taken_st, fallthrough_st) in CASES {
        let mut s = Sys::code(&[op, 0x04]);
        s.cpu.st = taken_st;
        assert_eq!(s.insn(), 7, "{op:#04x} taken");
        assert_eq!(s.cpu.pc, CODE + 6, "{op:#04x} taken");

        let mut s = Sys::code(&[op, 0x04]);
        s.cpu.st = fallthrough_st;
        assert_eq!(s.insn(), 5, "{op:#04x} not taken");
        assert_eq!(s.cpu.pc, CODE + 2, "{op:#04x} not taken");
    }
}

#[test]
fn jp_falls_through_on_zero_too() {
    let mut s = Sys::code(&[0xE4, 0x04]);
    s.cpu.st = st::Z;
    s.insn();
    assert_eq!(s.cpu.pc, CODE + 2);
}

#[test]
fn btjo_branches_on_any_common_bit() {
    let mut s = Sys::code(&[0x26, 0x0F, 0x10]); // BTJO %>F,A,+16
    s.set_a(0x08);
    assert_eq!(s.insn(), 7 + 2 + 2);
    assert_eq!(s.cpu.pc, CODE + 3 + 0x10);
    assert_eq!(s.flags(), (false, false, false));

    let mut s = Sys::code(&[0x26, 0x0F, 0x10]);
    s.set_a(0xF0);
    assert_eq!(s.insn(), 7 + 2);
    assert_eq!(s.cpu.pc, CODE + 3);
    assert_eq!(s.flags(), (false, false, true));
}

#[test]
fn btjz_branches_on_any_clear_masked_bit() {
    let mut s = Sys::code(&[0x76, 0x80, 0x10, 0x02]); // BTJO %>80,R16,+2 (mask in R16)
    s.cpu.set_rf(0x10, 0x80);
    assert_eq!(s.insn(), 9 + 2 + 2);
    assert_eq!(s.cpu.pc, CODE + 4 + 2);

    let mut s = Sys::code(&[0x77, 0x80, 0x10, 0x02]); // BTJZ %>80,R16,+2
    s.cpu.set_rf(0x10, 0x80);
    assert_eq!(s.insn(), 9 + 2, "bit set: no branch");
    assert_eq!(s.cpu.pc, CODE + 4);

    let mut s = Sys::code(&[0x77, 0x80, 0x10, 0x02]);
    s.cpu.set_rf(0x10, 0x00);
    s.insn();
    assert_eq!(s.cpu.pc, CODE + 6, "bit clear: branch");
}

#[test]
fn djnz_decrements_then_branches_while_nonzero() {
    let mut s = Sys::code(&[0xDA, 0x10, 0xFD]); // DJNZ R16,self
    s.cpu.set_rf(0x10, 2);
    assert_eq!(s.insn(), 7 + 2 + 2);
    assert_eq!(s.cpu.rf(0x10), 1);
    assert_eq!(s.cpu.pc, CODE);
    assert_eq!(s.insn(), 7 + 2, "1 -> 0 falls through");
    assert_eq!(s.cpu.rf(0x10), 0);
    assert_eq!(s.cpu.pc, CODE + 3);

    let mut s = Sys::code(&[0xBA, 0x02]); // DJNZ A,+2 with A = 0 wraps and branches
    s.set_a(0);
    assert_eq!(s.insn(), 5 + 2 + 2);
    assert_eq!(s.a(), 0xFF);
    assert_eq!(s.cpu.pc, CODE + 4);
}

#[test]
fn djnz_leaves_flags_alone() {
    let mut s = Sys::code(&[0xCA, 0x00]); // DJNZ B,+0
    s.set_b(1);
    s.cpu.st = st::C | st::N | st::Z;
    s.insn();
    assert_eq!(s.flags(), (true, true, true));
}
