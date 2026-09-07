//! Extended addressing: LDA/STA/CMPA/MOVD/BR/CALL in direct (`@>addr`),
//! indexed (`@>addr(B)`), and indirect (`*Rn`) forms; TRAP and RETS.

mod common;

use common::{CODE, Sys};

#[test]
fn lda_forms() {
    let mut s = Sys::code(&[0x8A, 0x00, 0x20]); // LDA @>0020
    s.cpu.set_rf(0x20, 0x80);
    assert_eq!(s.insn(), 11);
    assert_eq!(s.a(), 0x80);
    assert_eq!(s.flags(), (false, true, false));

    let mut s = Sys::code(&[0xAA, 0x00, 0x20]); // LDA @>0020(B)
    s.set_b(0x03);
    s.cpu.set_rf(0x23, 0x42);
    assert_eq!(s.insn(), 13);
    assert_eq!(s.a(), 0x42);

    let mut s = Sys::code(&[0x9A, 0x11]); // LDA *R17 (pair R16:R17)
    s.cpu.set_rf(0x10, 0x00);
    s.cpu.set_rf(0x11, 0x30);
    s.cpu.set_rf(0x30, 0x55);
    assert_eq!(s.insn(), 10);
    assert_eq!(s.a(), 0x55);
}

#[test]
fn lda_from_external_memory_and_rom() {
    // $0200-$EFFF only reaches Bus in Full-Expansion mode (IOCNT0 bits
    // 7:6 = `10`); elsewhere it's Not Available.
    let mut s = Sys::code(&[0xA2, 0x80, 0x00, 0x8A, 0x20, 0x00]); // IOCNT0 = Full-Expansion, then LDA @>2000
    s.board.ext[0x2000] = 0x99;
    s.insn(); // MOVP selects Full-Expansion
    s.insn(); // LDA @>2000: external
    assert_eq!(s.a(), 0x99);

    let mut s = Sys::code(&[0x8A, 0xF0, 0x00]); // LDA @>F000: the opcode itself, ROM in every mode
    s.insn();
    assert_eq!(s.a(), 0x8A);
}

#[test]
fn sta_forms() {
    let mut s = Sys::code(&[0x8B, 0x00, 0x20]); // STA @>0020
    s.set_a(0x00);
    assert_eq!(s.insn(), 11);
    assert_eq!(s.cpu.rf(0x20), 0);
    assert_eq!(s.flags(), (false, false, true));

    // $0200-$EFFF only reaches Bus in Full-Expansion mode.
    let mut s = Sys::code(&[0xA2, 0x80, 0x00, 0xAB, 0x20, 0x00]); // IOCNT0 = Full-Expansion, then STA @>2000(B)
    s.set_a(0x7E);
    s.set_b(0x10);
    s.insn(); // MOVP selects Full-Expansion
    assert_eq!(s.insn(), 13);
    assert_eq!(s.board.ext[0x2010], 0x7E);

    let mut s = Sys::code(&[0x9B, 0x11]); // STA *R17
    s.set_a(0x01);
    s.cpu.set_rf(0x10, 0x00);
    s.cpu.set_rf(0x11, 0x40);
    assert_eq!(s.insn(), 10);
    assert_eq!(s.cpu.rf(0x40), 0x01);

    let mut s = Sys::code(&[0x8B, 0xF0, 0x10]); // STA into ROM is dropped
    s.set_a(0x01);
    s.insn();
    assert_eq!(s.cpu.peek(0xF010), 0);
}

#[test]
fn cmpa_forms() {
    let mut s = Sys::code(&[0x8D, 0x00, 0x20]); // CMPA @>0020
    s.set_a(0x05);
    s.cpu.set_rf(0x20, 0x06);
    assert_eq!(s.insn(), 12);
    assert_eq!(s.a(), 0x05);
    assert_eq!(s.flags(), (false, true, false), "5 - 6 borrows");

    let mut s = Sys::code(&[0xAD, 0x00, 0x20]); // CMPA @>0020(B)
    s.set_a(0x06);
    s.set_b(0x01);
    s.cpu.set_rf(0x21, 0x06);
    assert_eq!(s.insn(), 14);
    assert_eq!(s.flags(), (true, false, true));

    let mut s = Sys::code(&[0x9D, 0x11]); // CMPA *R17
    s.cpu.set_rf(0x11, 0x20);
    s.cpu.set_rf(0x20, 0x01);
    s.set_a(0x02);
    assert_eq!(s.insn(), 11);
    assert_eq!(s.flags(), (true, false, false));
}

#[test]
fn movd_forms() {
    let mut s = Sys::code(&[0x88, 0x12, 0x34, 0x21]); // MOVD %>1234,R33
    assert_eq!(s.insn(), 15);
    assert_eq!((s.cpu.rf(0x20), s.cpu.rf(0x21)), (0x12, 0x34));
    assert_eq!(s.flags(), (false, false, false), "flags from the high byte");

    let mut s = Sys::code(&[0xA8, 0x12, 0x34, 0x21]); // MOVD %>1234(B),R33
    s.set_b(0xFF);
    assert_eq!(s.insn(), 17);
    assert_eq!((s.cpu.rf(0x20), s.cpu.rf(0x21)), (0x13, 0x33));

    let mut s = Sys::code(&[0x98, 0x11, 0x21]); // MOVD R17,R33 (pair copy)
    s.cpu.set_rf(0x10, 0x80);
    s.cpu.set_rf(0x11, 0x00);
    assert_eq!(s.insn(), 14);
    assert_eq!((s.cpu.rf(0x20), s.cpu.rf(0x21)), (0x80, 0x00));
    assert_eq!(s.flags(), (false, true, false));
}

#[test]
fn br_forms() {
    let mut s = Sys::code(&[0x8C, 0xF1, 0x00]); // BR @>F100
    assert_eq!(s.insn(), 10);
    assert_eq!(s.cpu.pc, 0xF100);

    let mut s = Sys::code(&[0xAC, 0xF1, 0x00]); // BR @>F100(B)
    s.set_b(0x20);
    assert_eq!(s.insn(), 12);
    assert_eq!(s.cpu.pc, 0xF120);

    let mut s = Sys::code(&[0x9C, 0x11]); // BR *R17
    s.cpu.set_rf(0x10, 0xF2);
    s.cpu.set_rf(0x11, 0x00);
    assert_eq!(s.insn(), 9);
    assert_eq!(s.cpu.pc, 0xF200);
}

#[test]
fn call_pushes_the_return_address_and_rets_pops_it() {
    let mut s = Sys::code(&[0x8E, 0xF1, 0x00]); // CALL @>F100
    s.cpu.sp = 0x40;
    assert_eq!(s.insn(), 14);
    assert_eq!(s.cpu.pc, 0xF100);
    assert_eq!(s.cpu.sp, 0x42);
    assert_eq!(
        (s.cpu.rf(0x41), s.cpu.rf(0x42)),
        (0xF0, 0x03),
        "high byte first, low byte on top"
    );

    let mut rom = Sys::rom(&[0x8E, 0xF1, 0x00, 0x00]);
    rom[0x100] = 0x0A; // RETS at $F100
    let mut s = Sys::from_rom(&rom);
    s.cpu.sp = 0x40;
    s.insn();
    assert_eq!(s.insn(), 7);
    assert_eq!(s.cpu.pc, CODE + 3);
    assert_eq!(s.cpu.sp, 0x40);

    let mut s = Sys::code(&[0xAE, 0xF1, 0x00]); // CALL @>F100(B)
    s.set_b(2);
    assert_eq!(s.insn(), 16);
    assert_eq!(s.cpu.pc, 0xF102);

    let mut s = Sys::code(&[0x9E, 0x11]); // CALL *R17
    s.cpu.set_rf(0x10, 0xF3);
    assert_eq!(s.insn(), 13);
    assert_eq!(s.cpu.pc, 0xF300);
}

#[test]
fn trap_vectors_through_the_top_of_rom() {
    let mut rom = Sys::rom(&[0xFC]); // TRAP 3: vector $FFF8
    rom[0xFF8] = 0xF4;
    rom[0xFF9] = 0x56;
    let mut s = Sys::from_rom(&rom);
    s.cpu.sp = 0x40;
    assert_eq!(s.insn(), 14);
    assert_eq!(s.cpu.pc, 0xF456);
    assert_eq!((s.cpu.rf(0x41), s.cpu.rf(0x42)), (0xF0, 0x01));

    let mut rom = Sys::rom(&[0xE8]); // TRAP 23: vector $FFD0
    rom[0xFD0] = 0xF7;
    rom[0xFD1] = 0x89;
    let mut s = Sys::from_rom(&rom);
    s.insn();
    assert_eq!(s.cpu.pc, 0xF789);
}
