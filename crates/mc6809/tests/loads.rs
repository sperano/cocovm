//! Test-driven coverage for 8-bit and 16-bit load/store instructions across
//! immediate / direct / extended addressing. Flag semantics: LD/ST set N and Z
//! from the value, clear V, and leave C and H unaffected.

mod common;

use common::Sys;
use mc6809::cc;

fn nz(s: &Sys) -> (bool, bool) {
    (s.cpu.cc & cc::NEGATIVE != 0, s.cpu.cc & cc::ZERO != 0)
}

// ---- LDA ----------------------------------------------------------------

#[test]
fn lda_immediate_positive() {
    let mut s = Sys::code(0x0000, &[0x86, 0x42]);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x42);
    assert_eq!(cycles, 2);
    assert_eq!(nz(&s), (false, false));
    assert_eq!(s.cpu.cc & cc::OVERFLOW, 0);
}

#[test]
fn lda_immediate_negative_sets_n() {
    let mut s = Sys::code(0x0000, &[0x86, 0x80]);
    s.step();
    assert_eq!(s.cpu.a, 0x80);
    assert_eq!(nz(&s), (true, false));
}

#[test]
fn lda_immediate_zero_sets_z() {
    let mut s = Sys::code(0x0000, &[0x86, 0x00]);
    s.step();
    assert_eq!(nz(&s), (false, true));
}

#[test]
fn lda_direct_uses_dp() {
    // DP=$12, operand $34 -> effective address $1234.
    let mut s = Sys::code(0x0000, &[0x96, 0x34]);
    s.cpu.dp = 0x12;
    s.set_mem(0x1234, 0x56);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x56);
    assert_eq!(cycles, 4);
}

#[test]
fn lda_extended() {
    let mut s = Sys::code(0x0000, &[0xB6, 0x12, 0x34]);
    s.set_mem(0x1234, 0x7E);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x7E);
    assert_eq!(cycles, 5);
}

// ---- LDB ----------------------------------------------------------------

#[test]
fn ldb_immediate() {
    let mut s = Sys::code(0x0000, &[0xC6, 0x99]);
    let cycles = s.step();
    assert_eq!(s.cpu.b, 0x99);
    assert_eq!(cycles, 2);
    assert_eq!(nz(&s), (true, false));
}

#[test]
fn ldb_extended() {
    let mut s = Sys::code(0x0000, &[0xF6, 0x40, 0x00]);
    s.set_mem(0x4000, 0x01);
    let cycles = s.step();
    assert_eq!(s.cpu.b, 0x01);
    assert_eq!(cycles, 5);
}

// ---- STA / STB ----------------------------------------------------------

#[test]
fn sta_extended_writes_and_flags_from_value() {
    let mut s = Sys::code(0x0000, &[0xB7, 0x20, 0x00]);
    s.cpu.a = 0x00;
    let cycles = s.step();
    assert_eq!(s.mem(0x2000), 0x00);
    assert_eq!(cycles, 5);
    assert_eq!(nz(&s), (false, true)); // stored value is zero -> Z
}

#[test]
fn sta_direct() {
    let mut s = Sys::code(0x0000, &[0x97, 0x10]);
    s.cpu.dp = 0x05;
    s.cpu.a = 0xAB;
    let cycles = s.step();
    assert_eq!(s.mem(0x0510), 0xAB);
    assert_eq!(cycles, 4);
    assert_eq!(nz(&s), (true, false));
}

#[test]
fn stb_extended() {
    let mut s = Sys::code(0x0000, &[0xF7, 0x60, 0x00]);
    s.cpu.b = 0x7F;
    let cycles = s.step();
    assert_eq!(s.mem(0x6000), 0x7F);
    assert_eq!(cycles, 5);
}

// ---- LDD / STD (16-bit) -------------------------------------------------

#[test]
fn ldd_immediate() {
    let mut s = Sys::code(0x0000, &[0xCC, 0x12, 0x34]);
    let cycles = s.step();
    assert_eq!(s.cpu.d(), 0x1234);
    assert_eq!(s.cpu.a, 0x12);
    assert_eq!(s.cpu.b, 0x34);
    assert_eq!(cycles, 3);
    assert_eq!(nz(&s), (false, false));
}

#[test]
fn ldd_immediate_zero_sets_z() {
    let mut s = Sys::code(0x0000, &[0xCC, 0x00, 0x00]);
    s.step();
    assert_eq!(nz(&s), (false, true));
}

#[test]
fn ldd_extended_is_big_endian() {
    let mut s = Sys::code(0x0000, &[0xFC, 0x50, 0x00]);
    s.set_mem(0x5000, 0xBE);
    s.set_mem(0x5001, 0xEF);
    let cycles = s.step();
    assert_eq!(s.cpu.d(), 0xBEEF);
    assert_eq!(cycles, 6);
    assert_eq!(nz(&s), (true, false)); // bit 15 set -> N
}

#[test]
fn std_extended_is_big_endian() {
    let mut s = Sys::code(0x0000, &[0xFD, 0x30, 0x00]);
    s.cpu.set_d(0xBEEF);
    let cycles = s.step();
    assert_eq!(s.mem(0x3000), 0xBE);
    assert_eq!(s.mem(0x3001), 0xEF);
    assert_eq!(cycles, 6);
    assert_eq!(nz(&s), (true, false));
}

#[test]
fn std_direct() {
    let mut s = Sys::code(0x0000, &[0xDD, 0x00]);
    s.cpu.dp = 0x70;
    s.cpu.set_d(0x0102);
    let cycles = s.step();
    assert_eq!(s.mem(0x7000), 0x01);
    assert_eq!(s.mem(0x7001), 0x02);
    assert_eq!(cycles, 5);
}
