//! Single-operand ops on A, B, and Rn (`$Bx`/`$Cx`/`$Dx` rows), DECD, and
//! the MOV aliases `$B0`/`$B1`/`$C0`/`$C1`/`$D0`/`$D1`.

mod common;

use common::Sys;

#[test]
fn row_costs_are_5_for_a_b_and_7_for_rn() {
    let mut s = Sys::code(&[0xB3]); // INC A
    assert_eq!(s.insn(), 5);
    let mut s = Sys::code(&[0xC3]); // INC B
    assert_eq!(s.insn(), 5);
    let mut s = Sys::code(&[0xD3, 0x10]); // INC R16
    assert_eq!(s.insn(), 7);
    assert_eq!(s.cpu.rf(0x10), 1);
}

#[test]
fn inc_and_dec_carry_semantics() {
    let mut s = Sys::code(&[0xB3]); // INC A
    s.set_a(0xFF);
    s.insn();
    assert_eq!(s.a(), 0);
    assert_eq!(s.flags(), (true, false, true));

    let mut s = Sys::code(&[0xB2]); // DEC A
    s.set_a(0x00);
    s.insn();
    assert_eq!(s.a(), 0xFF);
    assert_eq!(s.flags(), (false, true, false), "borrow clears C");

    let mut s = Sys::code(&[0xB2]);
    s.set_a(0x01);
    s.insn();
    assert_eq!(s.flags(), (true, false, true));
}

#[test]
fn inv_and_clr() {
    let mut s = Sys::code(&[0xC4]); // INV B
    s.set_b(0x0F);
    s.insn();
    assert_eq!(s.b(), 0xF0);
    assert_eq!(s.flags(), (false, true, false));

    let mut s = Sys::code(&[0xB5]); // CLR A
    s.set_a(0x12);
    s.set_c(true);
    s.insn();
    assert_eq!(s.a(), 0);
    assert_eq!(s.flags(), (false, false, true));
}

#[test]
fn rotates() {
    let mut s = Sys::code(&[0xBE]); // RL A
    s.set_a(0x81);
    s.insn();
    assert_eq!(s.a(), 0x03);
    assert_eq!(s.flags(), (true, false, false), "C = old bit 7");

    let mut s = Sys::code(&[0xBF]); // RLC A
    s.set_a(0x40);
    s.set_c(true);
    s.insn();
    assert_eq!(s.a(), 0x81);
    assert_eq!(s.flags(), (false, true, false));

    let mut s = Sys::code(&[0xBC]); // RR A
    s.set_a(0x01);
    s.insn();
    assert_eq!(s.a(), 0x80);
    assert_eq!(s.flags(), (true, true, false), "C and bit 7 = old bit 0");

    let mut s = Sys::code(&[0xBD]); // RRC A
    s.set_a(0x02);
    s.set_c(true);
    s.insn();
    assert_eq!(s.a(), 0x81);
    assert_eq!(s.flags(), (false, true, false));
}

#[test]
fn swap_costs_three_extra_and_flags_the_wide_intermediate() {
    let mut s = Sys::code(&[0xB7]); // SWAP A
    s.set_a(0x1A);
    assert_eq!(s.insn(), 5 + 3);
    assert_eq!(s.a(), 0xA1);
    // MAME: t = p >> 4 | p << 4 as 16 bits: C = old bit 4, N = old bit 3.
    assert_eq!(s.flags(), (true, true, false));
}

#[test]
fn xchb_swaps_with_b_and_flags_old_b() {
    let mut s = Sys::code(&[0xD6, 0x10]); // XCHB R16
    s.cpu.set_rf(0x10, 0x11);
    s.set_b(0x80);
    assert_eq!(s.insn(), 7 + 1);
    assert_eq!(s.cpu.rf(0x10), 0x80);
    assert_eq!(s.b(), 0x11);
    assert_eq!(s.flags(), (false, true, false));

    let mut s = Sys::code(&[0xC6]); // XCHB B: a TSTB
    s.set_b(0);
    assert_eq!(s.insn(), 6);
    assert_eq!(s.b(), 0);
    assert_eq!(s.flags(), (false, false, true));
}

#[test]
fn mov_aliases() {
    let mut s = Sys::code(&[0xB0]); // MOV A,A (CLRC/TSTA)
    s.set_a(0x80);
    s.set_c(true);
    assert_eq!(s.insn(), 6);
    assert_eq!(s.flags(), (false, true, false));

    let mut s = Sys::code(&[0xB1]); // MOV B,A, undocumented
    s.set_b(0x42);
    assert_eq!(s.insn(), 5);
    assert_eq!(s.a(), 0x42);

    let mut s = Sys::code(&[0xC0]); // MOV A,B
    s.set_a(0x24);
    assert_eq!(s.insn(), 6);
    assert_eq!(s.b(), 0x24);

    let mut s = Sys::code(&[0xC1]); // MOV B,B (TSTB)
    s.set_b(0);
    assert_eq!(s.insn(), 6);
    assert_eq!(s.flags(), (false, false, true));

    let mut s = Sys::code(&[0xD0, 0x30]); // MOV A,R48
    s.set_a(0x99);
    assert_eq!(s.insn(), 8);
    assert_eq!(s.cpu.rf(0x30), 0x99);

    let mut s = Sys::code(&[0xD1, 0x30]); // MOV B,R48
    s.set_b(0x77);
    assert_eq!(s.insn(), 7);
    assert_eq!(s.cpu.rf(0x30), 0x77);
}

#[test]
fn decd_decrements_a_register_pair() {
    let mut s = Sys::code(&[0xDB, 0x11]); // DECD R17 (pair R16:R17)
    s.cpu.set_rf(0x10, 0x01);
    s.cpu.set_rf(0x11, 0x00);
    assert_eq!(s.insn(), 11);
    assert_eq!((s.cpu.rf(0x10), s.cpu.rf(0x11)), (0x00, 0xFF));
    assert_eq!(s.flags(), (true, false, true), "flags from the high byte");

    let mut s = Sys::code(&[0xBB]); // DECD A: pair R255:R0 — R255 is unmapped
    s.set_a(0x00);
    assert_eq!(s.insn(), 9);
    assert_eq!(s.a(), 0xFF);
    assert_eq!(s.flags(), (false, true, false), "0x0000 - 1 borrows");

    let mut s = Sys::code(&[0xCB]); // DECD B: pair A:B
    s.set_a(0x12);
    s.set_b(0x00);
    assert_eq!(s.insn(), 9);
    assert_eq!((s.a(), s.b()), (0x11, 0xFF));
}
