//! Test-driven coverage for the 16-bit ALU / load / store set and the
//! `$10`/`$11` prefix pages: ADDD/SUBD, CMPD/CMPX/CMPY/CMPU/CMPS,
//! LDX/LDY/LDU/LDS and their stores.
//!
//! Flag conventions: ADDD/SUBD/CMP set N,Z,V,C (H unaffected); LD/ST set N,Z and
//! clear V, leaving C,H. Prefixed ($10/$11) ops cost one more cycle than the
//! base-page equivalent.

mod common;

use common::Sys;
use mc6809::cc;

/// (N, Z, V, C) as booleans.
fn nzvc(s: &Sys) -> (bool, bool, bool, bool) {
    let cc = s.cpu.cc;
    (
        cc & cc::NEGATIVE != 0,
        cc & cc::ZERO != 0,
        cc & cc::OVERFLOW != 0,
        cc & cc::CARRY != 0,
    )
}

// ======================================================================
// ADDD / SUBD
// ======================================================================

#[test]
fn addd_immediate() {
    let mut s = Sys::code(0x0000, &[0xC3, 0x02, 0x34]); // ADDD #$0234
    s.cpu.set_d(0x1000);
    let cycles = s.step();
    assert_eq!(s.cpu.d(), 0x1234);
    assert_eq!(cycles, 4);
    assert_eq!(nzvc(&s), (false, false, false, false));
}

#[test]
fn addd_carry_and_zero() {
    let mut s = Sys::code(0x0000, &[0xC3, 0x00, 0x01]);
    s.cpu.set_d(0xFFFF);
    s.step();
    assert_eq!(s.cpu.d(), 0x0000);
    assert_eq!(nzvc(&s), (false, true, false, true)); // Z, C
}

#[test]
fn addd_signed_overflow() {
    let mut s = Sys::code(0x0000, &[0xC3, 0x00, 0x01]);
    s.cpu.set_d(0x7FFF);
    s.step();
    assert_eq!(s.cpu.d(), 0x8000);
    assert_eq!(nzvc(&s), (true, false, true, false)); // N, V
}

#[test]
fn subd_immediate() {
    let mut s = Sys::code(0x0000, &[0x83, 0x10, 0x00]); // SUBD #$1000
    s.cpu.set_d(0x5000);
    let cycles = s.step();
    assert_eq!(s.cpu.d(), 0x4000);
    assert_eq!(cycles, 4);
    assert_eq!(nzvc(&s), (false, false, false, false));
}

#[test]
fn subd_borrow() {
    let mut s = Sys::code(0x0000, &[0x83, 0x00, 0x01]);
    s.cpu.set_d(0x0000);
    s.step();
    assert_eq!(s.cpu.d(), 0xFFFF);
    assert_eq!(nzvc(&s), (true, false, false, true)); // N, C (borrow)
}

#[test]
fn subd_extended_cycles() {
    let mut s = Sys::code(0x0000, &[0xB3, 0x40, 0x00]);
    s.cpu.set_d(0x0002);
    s.set_mem(0x4000, 0x00);
    s.set_mem(0x4001, 0x01);
    let cycles = s.step();
    assert_eq!(s.cpu.d(), 0x0001);
    assert_eq!(cycles, 7);
}

// ======================================================================
// CMP (16-bit) — base page CMPX
// ======================================================================

#[test]
fn cmpx_equal_sets_z_preserves_x() {
    let mut s = Sys::code(0x0000, &[0x8C, 0x12, 0x34]); // CMPX #$1234
    s.cpu.x = 0x1234;
    let cycles = s.step();
    assert_eq!(s.cpu.x, 0x1234); // unchanged
    assert_eq!(cycles, 4);
    assert_eq!(nzvc(&s), (false, true, false, false));
}

#[test]
fn cmpx_less_than_sets_carry() {
    let mut s = Sys::code(0x0000, &[0x8C, 0x20, 0x00]); // CMPX #$2000
    s.cpu.x = 0x1000;
    s.step();
    assert_eq!(s.cpu.x, 0x1000);
    assert_eq!(nzvc(&s), (true, false, false, true)); // 0x1000-0x2000 borrows
}

#[test]
fn cmpx_direct_and_extended_cycles() {
    let mut s = Sys::code(0x0000, &[0x9C, 0x40]); // CMPX <$40
    s.cpu.dp = 0x00;
    s.cpu.x = 0x1111;
    s.set_mem(0x0040, 0x11);
    s.set_mem(0x0041, 0x11);
    assert_eq!(s.step(), 6);
    assert!(s.cpu.cc & cc::ZERO != 0);

    let mut s = Sys::code(0x0000, &[0xBC, 0x50, 0x00]); // CMPX >$5000
    s.cpu.x = 0x0000;
    s.set_mem(0x5000, 0x00);
    s.set_mem(0x5001, 0x00);
    assert_eq!(s.step(), 7);
}

// ======================================================================
// LDX / STX / LDU / STU
// ======================================================================

#[test]
fn ldx_immediate_clears_v_keeps_carry() {
    let mut s = Sys::code(0x0000, &[0x8E, 0x12, 0x34]); // LDX #$1234
    s.cpu.cc |= cc::OVERFLOW | cc::CARRY;
    let cycles = s.step();
    assert_eq!(s.cpu.x, 0x1234);
    assert_eq!(cycles, 3);
    assert_eq!(nzvc(&s), (false, false, false, true)); // V cleared, C kept
}

#[test]
fn ldx_zero_and_negative() {
    let mut s = Sys::code(0x0000, &[0x8E, 0x00, 0x00]);
    s.step();
    assert_eq!(nzvc(&s), (false, true, false, false)); // Z

    let mut s = Sys::code(0x0000, &[0x8E, 0x80, 0x00]);
    s.step();
    assert_eq!(nzvc(&s), (true, false, false, false)); // N
}

#[test]
fn ldx_indexed_via_y() {
    let mut s = Sys::code(0x0000, &[0xAE, 0xA4]); // LDX ,Y
    s.cpu.y = 0x3000;
    s.set_mem(0x3000, 0xBE);
    s.set_mem(0x3001, 0xEF);
    let cycles = s.step();
    assert_eq!(s.cpu.x, 0xBEEF);
    assert_eq!(cycles, 5); // 5 + 0
}

#[test]
fn stx_extended_big_endian() {
    let mut s = Sys::code(0x0000, &[0xBF, 0x20, 0x00]); // STX >$2000
    s.cpu.x = 0xCAFE;
    let cycles = s.step();
    assert_eq!(s.mem(0x2000), 0xCA);
    assert_eq!(s.mem(0x2001), 0xFE);
    assert_eq!(cycles, 6);
}

#[test]
fn ldu_and_stu() {
    let mut s = Sys::code(0x0000, &[0xCE, 0x12, 0x34]); // LDU #$1234
    assert_eq!(s.step(), 3);
    assert_eq!(s.cpu.u, 0x1234);

    let mut s = Sys::code(0x0000, &[0xFF, 0x60, 0x00]); // STU >$6000
    s.cpu.u = 0x0102;
    assert_eq!(s.step(), 6);
    assert_eq!(s.mem(0x6000), 0x01);
    assert_eq!(s.mem(0x6001), 0x02);
}

// ======================================================================
// $10 prefix page: CMPD, CMPY, LDY, STY, LDS, STS
// ======================================================================

#[test]
fn cmpd_prefixed_costs_one_more() {
    let mut s = Sys::code(0x0000, &[0x10, 0x83, 0x12, 0x34]); // CMPD #$1234
    s.cpu.set_d(0x1234);
    let cycles = s.step();
    assert_eq!(s.cpu.d(), 0x1234);
    assert_eq!(cycles, 5); // CMPX imm is 4; prefix adds 1
    assert!(s.cpu.cc & cc::ZERO != 0);
}

#[test]
fn cmpy_immediate() {
    let mut s = Sys::code(0x0000, &[0x10, 0x8C, 0x20, 0x00]); // CMPY #$2000
    s.cpu.y = 0x1000;
    let cycles = s.step();
    assert_eq!(s.cpu.y, 0x1000);
    assert_eq!(cycles, 5);
    assert_eq!(nzvc(&s), (true, false, false, true));
}

#[test]
fn ldy_immediate_and_extended() {
    let mut s = Sys::code(0x0000, &[0x10, 0x8E, 0x12, 0x34]); // LDY #$1234
    assert_eq!(s.step(), 4);
    assert_eq!(s.cpu.y, 0x1234);

    let mut s = Sys::code(0x0000, &[0x10, 0xBE, 0x50, 0x00]); // LDY >$5000
    s.set_mem(0x5000, 0xAB);
    s.set_mem(0x5001, 0xCD);
    assert_eq!(s.step(), 7);
    assert_eq!(s.cpu.y, 0xABCD);
}

#[test]
fn sty_direct() {
    let mut s = Sys::code(0x0000, &[0x10, 0x9F, 0x00]); // STY <$00
    s.cpu.dp = 0x70;
    s.cpu.y = 0x0102;
    let cycles = s.step();
    assert_eq!(s.mem(0x7000), 0x01);
    assert_eq!(s.mem(0x7001), 0x02);
    assert_eq!(cycles, 6);
}

#[test]
fn lds_and_sts() {
    let mut s = Sys::code(0x0000, &[0x10, 0xCE, 0xC0, 0x00]); // LDS #$C000
    let cycles = s.step();
    assert_eq!(s.cpu.s, 0xC000);
    assert_eq!(cycles, 4);

    let mut s = Sys::code(0x0000, &[0x10, 0xFF, 0x70, 0x00]); // STS >$7000
    s.cpu.s = 0x0304;
    assert_eq!(s.step(), 7);
    assert_eq!(s.mem(0x7000), 0x03);
    assert_eq!(s.mem(0x7001), 0x04);
}

// ======================================================================
// $11 prefix page: CMPU, CMPS
// ======================================================================

#[test]
fn cmpu_immediate() {
    let mut s = Sys::code(0x0000, &[0x11, 0x83, 0x12, 0x34]); // CMPU #$1234
    s.cpu.u = 0x1234;
    let cycles = s.step();
    assert_eq!(s.cpu.u, 0x1234);
    assert_eq!(cycles, 5);
    assert!(s.cpu.cc & cc::ZERO != 0);
}

#[test]
fn cmps_immediate_and_extended() {
    let mut s = Sys::code(0x0000, &[0x11, 0x8C, 0x00, 0x00]); // CMPS #$0000
    s.cpu.s = 0x0000;
    assert_eq!(s.step(), 5);
    assert!(s.cpu.cc & cc::ZERO != 0);

    let mut s = Sys::code(0x0000, &[0x11, 0xBC, 0x40, 0x00]); // CMPS >$4000
    s.cpu.s = 0x8000;
    s.set_mem(0x4000, 0x80);
    s.set_mem(0x4001, 0x00);
    let cycles = s.step();
    assert_eq!(cycles, 8);
    assert!(s.cpu.cc & cc::ZERO != 0); // 0x8000 == 0x8000
}
