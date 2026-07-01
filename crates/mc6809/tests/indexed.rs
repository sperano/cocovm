//! Test-driven coverage for the indexed-addressing postbyte decoder — the mode
//! that ~a large fraction of 6809 instructions route through (see `DESIGN.md` §5).
//!
//! Each test drives a real instruction (usually `LDA`/`STA`/`LEA`) whose only
//! variable is the postbyte, and asserts the effective address that was reached
//! (via the loaded/stored value), any register side effects (auto inc/dec), and
//! the total cycle count (instruction base + postbyte extra).
//!
//! Postbyte reference (bit 7 set): `1 rr i mmmm`. Register field rr: 00=X, 01=Y,
//! 10=U, 11=S. Non-indexed base costs used here: LDA/STA/LEA = 4, LDD = 5.

mod common;

use common::Sys;
use mc6809::cc;

const LDA_INDEXED: u8 = 0xA6;
const STA_INDEXED: u8 = 0xA7;
const LDD_INDEXED: u8 = 0xEC;

/// Load a program at $0000, point a chosen index register somewhere useful, and
/// return the system ready to `step()`.
fn prog(bytes: &[u8]) -> Sys {
    Sys::code(0x0000, bytes)
}

// ---- 5-bit constant offset ----------------------------------------------

#[test]
fn offset5_positive() {
    // LDA 5,X   postbyte 0RRnnnnn = 000_00101 = 0x05
    let mut s = prog(&[LDA_INDEXED, 0x05]);
    s.cpu.x = 0x2000;
    s.set_mem(0x2005, 0x42);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x42);
    assert_eq!(s.cpu.x, 0x2000); // unchanged
    assert_eq!(cycles, 5); // 4 + 1
}

#[test]
fn offset5_negative_sign_extends() {
    // LDA -1,X   -1 in 5 bits = 11111 = 0x1F
    let mut s = prog(&[LDA_INDEXED, 0x1F]);
    s.cpu.x = 0x2000;
    s.set_mem(0x1FFF, 0x7E);
    s.step();
    assert_eq!(s.cpu.a, 0x7E);
}

// ---- no offset ----------------------------------------------------------

#[test]
fn no_offset() {
    // LDA ,X   1RR00100 with X = 0x84
    let mut s = prog(&[LDA_INDEXED, 0x84]);
    s.cpu.x = 0x3000;
    s.set_mem(0x3000, 0x11);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x11);
    assert_eq!(cycles, 4); // 4 + 0
}

// ---- auto increment / decrement -----------------------------------------

#[test]
fn auto_increment_by_one() {
    // LDA ,X+   1RR00000 = 0x80. EA is X *before* the increment.
    let mut s = prog(&[LDA_INDEXED, 0x80]);
    s.cpu.x = 0x2000;
    s.set_mem(0x2000, 0xAA);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0xAA);
    assert_eq!(s.cpu.x, 0x2001); // incremented
    assert_eq!(cycles, 6); // 4 + 2
}

#[test]
fn auto_increment_by_two() {
    // LDA ,X++   0x81
    let mut s = prog(&[LDA_INDEXED, 0x81]);
    s.cpu.x = 0x2000;
    s.set_mem(0x2000, 0xBB);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0xBB);
    assert_eq!(s.cpu.x, 0x2002);
    assert_eq!(cycles, 7); // 4 + 3
}

#[test]
fn auto_decrement_by_one() {
    // LDA ,-X   0x82. Register is decremented *first*, then used as EA.
    let mut s = prog(&[LDA_INDEXED, 0x82]);
    s.cpu.x = 0x2000;
    s.set_mem(0x1FFF, 0xCC);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0xCC);
    assert_eq!(s.cpu.x, 0x1FFF);
    assert_eq!(cycles, 6); // 4 + 2
}

#[test]
fn auto_decrement_by_two() {
    // LDA ,--X   0x83
    let mut s = prog(&[LDA_INDEXED, 0x83]);
    s.cpu.x = 0x2000;
    s.set_mem(0x1FFE, 0xDD);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0xDD);
    assert_eq!(s.cpu.x, 0x1FFE);
    assert_eq!(cycles, 7); // 4 + 3
}

// ---- 8/16-bit constant offset -------------------------------------------

#[test]
fn offset8_positive() {
    // LDA $10,X   1RR01000 = 0x88, offset byte 0x10
    let mut s = prog(&[LDA_INDEXED, 0x88, 0x10]);
    s.cpu.x = 0x2000;
    s.set_mem(0x2010, 0x55);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x55);
    assert_eq!(cycles, 5); // 4 + 1
}

#[test]
fn offset8_negative() {
    // LDA -1,X (8-bit form)   0x88, 0xFF
    let mut s = prog(&[LDA_INDEXED, 0x88, 0xFF]);
    s.cpu.x = 0x2000;
    s.set_mem(0x1FFF, 0x66);
    s.step();
    assert_eq!(s.cpu.a, 0x66);
}

#[test]
fn offset16() {
    // LDA $0100,X   1RR01001 = 0x89, then 0x01 0x00
    let mut s = prog(&[LDA_INDEXED, 0x89, 0x01, 0x00]);
    s.cpu.x = 0x2000;
    s.set_mem(0x2100, 0x77);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x77);
    assert_eq!(cycles, 8); // 4 + 4
}

// ---- accumulator offsets ------------------------------------------------

#[test]
fn accumulator_a_offset_signed() {
    // LDA A,X   1RR00110 = 0x86. A used as signed offset at decode time.
    let mut s = prog(&[LDA_INDEXED, 0x86]);
    s.cpu.a = 0xFF; // -1
    s.cpu.x = 0x2000;
    s.set_mem(0x1FFF, 0x88);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x88); // overwritten by the load
    assert_eq!(cycles, 5); // 4 + 1
}

#[test]
fn accumulator_b_offset() {
    // LDA B,X   1RR00101 = 0x85
    let mut s = prog(&[LDA_INDEXED, 0x85]);
    s.cpu.b = 0x05;
    s.cpu.x = 0x2000;
    s.set_mem(0x2005, 0x99);
    s.step();
    assert_eq!(s.cpu.a, 0x99);
}

#[test]
fn accumulator_d_offset() {
    // LDA D,X   1RR01011 = 0x8B, base extra 4 cycles
    let mut s = prog(&[LDA_INDEXED, 0x8B]);
    s.cpu.set_d(0x0100);
    s.cpu.x = 0x2000;
    s.set_mem(0x2100, 0xA1);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0xA1);
    assert_eq!(cycles, 8); // 4 + 4
}

// ---- PC-relative --------------------------------------------------------

#[test]
fn pc_relative_8bit() {
    // LDA n,PCR at $1000. Offset is from the address of the *next* instruction.
    // opcode@1000, postbyte@1001, offset@1002 -> PC=0x1003 after decode.
    // EA = 0x1003 + 0x10 = 0x1013.
    let mut s = Sys::code(0x1000, &[LDA_INDEXED, 0x8C, 0x10]);
    s.set_mem(0x1013, 0xB2);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0xB2);
    assert_eq!(cycles, 5); // 4 + 1
}

#[test]
fn pc_relative_16bit() {
    // LDA n,PCR 16-bit   0x8D, offset 0x0100.
    // opcode@1000, postbyte@1001, offset@1002..1003 -> PC=0x1004.
    // EA = 0x1004 + 0x0100 = 0x1104.
    let mut s = Sys::code(0x1000, &[LDA_INDEXED, 0x8D, 0x01, 0x00]);
    s.set_mem(0x1104, 0xB3);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0xB3);
    assert_eq!(cycles, 9); // 4 + 5
}

// ---- indirect forms -----------------------------------------------------

#[test]
fn indirect_no_offset() {
    // LDA [,X]   1RR10100 = 0x94. X points to a pointer; that points to the data.
    let mut s = prog(&[LDA_INDEXED, 0x94]);
    s.cpu.x = 0x2000;
    s.set_mem(0x2000, 0x30); // pointer hi
    s.set_mem(0x2001, 0x00); // pointer lo -> 0x3000
    s.set_mem(0x3000, 0xC4);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0xC4);
    assert_eq!(cycles, 7); // 4 + 3
}

#[test]
fn indirect_8bit_offset() {
    // LDA [$10,X]   1RR11000 = 0x98, offset 0x10
    let mut s = prog(&[LDA_INDEXED, 0x98, 0x10]);
    s.cpu.x = 0x2000;
    s.set_mem(0x2010, 0x40); // pointer -> 0x4000
    s.set_mem(0x2011, 0x00);
    s.set_mem(0x4000, 0xC5);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0xC5);
    assert_eq!(cycles, 8); // 4 + (1 + 3)
}

#[test]
fn extended_indirect() {
    // LDA [$3000]   postbyte 10011111 = 0x9F, then 0x30 0x00 (register field ignored)
    let mut s = prog(&[LDA_INDEXED, 0x9F, 0x30, 0x00]);
    s.set_mem(0x3000, 0x50); // pointer -> 0x5000
    s.set_mem(0x3001, 0x00);
    s.set_mem(0x5000, 0xC6);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0xC6);
    assert_eq!(cycles, 9); // 4 + (2 + 3)
}

// ---- register selection -------------------------------------------------

#[test]
fn register_field_selects_y() {
    // LDA ,Y   1RR00100 with rr=01 (Y) = 0xA4
    let mut s = prog(&[LDA_INDEXED, 0xA4]);
    s.cpu.y = 0x3000;
    s.set_mem(0x3000, 0xD1);
    s.step();
    assert_eq!(s.cpu.a, 0xD1);
}

#[test]
fn register_field_selects_u_with_autoinc() {
    // LDA ,U+   rr=10 (U), mode 0000 = 0b11000000 = 0xC0
    let mut s = prog(&[LDA_INDEXED, 0xC0]);
    s.cpu.u = 0x4000;
    s.set_mem(0x4000, 0xD2);
    s.step();
    assert_eq!(s.cpu.a, 0xD2);
    assert_eq!(s.cpu.u, 0x4001);
}

// ---- store and 16-bit via the same decoder ------------------------------

#[test]
fn store_indexed() {
    // STA ,X   0xA7 0x84
    let mut s = prog(&[STA_INDEXED, 0x84]);
    s.cpu.a = 0x42;
    s.cpu.x = 0x2000;
    let cycles = s.step();
    assert_eq!(s.mem(0x2000), 0x42);
    assert_eq!(cycles, 4);
}

#[test]
fn ldd_indexed_is_16bit_base5() {
    // LDD ,X   0xEC 0x84
    let mut s = prog(&[LDD_INDEXED, 0x84]);
    s.cpu.x = 0x2000;
    s.set_mem(0x2000, 0xBE);
    s.set_mem(0x2001, 0xEF);
    let cycles = s.step();
    assert_eq!(s.cpu.d(), 0xBEEF);
    assert_eq!(cycles, 5); // 5 + 0
}

// ---- LEA ----------------------------------------------------------------

#[test]
fn leax_computes_address_and_sets_z_when_zero() {
    // LEAX -1,X   opcode 0x30, postbyte 5-bit -1 = 0x1F
    let mut s = prog(&[0x30, 0x1F]);
    s.cpu.x = 0x0001;
    let cycles = s.step();
    assert_eq!(s.cpu.x, 0x0000);
    assert_ne!(s.cpu.cc & cc::ZERO, 0); // LEAX sets Z from the result
    assert_eq!(cycles, 5); // 4 + 1
}

#[test]
fn leax_nonzero_clears_z() {
    // LEAX 5,X
    let mut s = prog(&[0x30, 0x05]);
    s.cpu.x = 0x2000;
    s.cpu.cc |= cc::ZERO; // pre-set to prove it gets cleared
    s.step();
    assert_eq!(s.cpu.x, 0x2005);
    assert_eq!(s.cpu.cc & cc::ZERO, 0);
}

#[test]
fn leas_does_not_touch_flags() {
    // LEAS 4,S   opcode 0x32, postbyte rr=11 (S) 5-bit +4 = 0x64
    let mut s = prog(&[0x32, 0x64]);
    s.cpu.s = 0x4000;
    s.cpu.cc |= cc::ZERO; // must survive — LEAS/LEAU affect no CCs
    let cycles = s.step();
    assert_eq!(s.cpu.s, 0x4004);
    assert_ne!(s.cpu.cc & cc::ZERO, 0);
    assert_eq!(cycles, 5);
}
