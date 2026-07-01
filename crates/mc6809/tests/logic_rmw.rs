//! Test-driven coverage for the rest of the 8-bit ALU: the logic ops
//! (AND/OR/EOR/BIT) and the read-modify-write group
//! (NEG/COM/LSR/ROR/ASR/ASL/ROL/DEC/INC/TST/CLR).
//!
//! Flag conventions verified against the Motorola MC6809 CC tables:
//! - Logic: N,Z from result; V cleared; C,H unaffected.
//! - COM: N,Z; V cleared; C forced to 1.
//! - LSR: N forced to 0; V unaffected; C = old bit 0.
//! - ASR/ROR: V unaffected; C = old bit 0.
//! - ASL/ROL: V = b7⊕b6 of original; C = old bit 7.
//! - INC/DEC: V on signed overflow; C unaffected.
//! - TST: N,Z; V cleared; C unaffected; memory not written.
//! - CLR: N=0, Z=1, V=0, C=0.

mod common;

use common::Sys;
use mc6809::cc;

/// (H, N, Z, V, C) as booleans.
fn flags(s: &Sys) -> (bool, bool, bool, bool, bool) {
    let cc = s.cpu.cc;
    (
        cc & cc::HALF_CARRY != 0,
        cc & cc::NEGATIVE != 0,
        cc & cc::ZERO != 0,
        cc & cc::OVERFLOW != 0,
        cc & cc::CARRY != 0,
    )
}

// ======================================================================
// Logic ops
// ======================================================================

#[test]
fn anda_clears_v_preserves_carry() {
    let mut s = Sys::code(0x0000, &[0x84, 0x0F]); // ANDA #$0F
    s.cpu.a = 0xF3;
    s.cpu.cc |= cc::OVERFLOW | cc::CARRY; // both pre-set
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x03);
    assert_eq!(cycles, 2);
    // V cleared, C preserved (untouched by logic), result positive nonzero.
    assert_eq!(flags(&s), (false, false, false, false, true));
}

#[test]
fn ora_sets_negative() {
    let mut s = Sys::code(0x0000, &[0x8A, 0x80]); // ORA #$80
    s.cpu.a = 0x01;
    s.step();
    assert_eq!(s.cpu.a, 0x81);
    assert_eq!(flags(&s), (false, true, false, false, false));
}

#[test]
fn eora_to_zero_sets_z() {
    let mut s = Sys::code(0x0000, &[0x88, 0xFF]); // EORA #$FF
    s.cpu.a = 0xFF;
    s.step();
    assert_eq!(s.cpu.a, 0x00);
    assert_eq!(flags(&s), (false, false, true, false, false));
}

#[test]
fn bita_sets_flags_without_changing_a() {
    let mut s = Sys::code(0x0000, &[0x85, 0x80]); // BITA #$80
    s.cpu.a = 0xC0;
    s.step();
    assert_eq!(s.cpu.a, 0xC0); // unchanged
    assert_eq!(flags(&s), (false, true, false, false, false)); // 0xC0 & 0x80 = 0x80
}

#[test]
fn andb_extended_cycles() {
    let mut s = Sys::code(0x0000, &[0xF4, 0x20, 0x00]); // ANDB >$2000
    s.cpu.b = 0xFF;
    s.set_mem(0x2000, 0x0F);
    let cycles = s.step();
    assert_eq!(s.cpu.b, 0x0F);
    assert_eq!(cycles, 5);
}

// ======================================================================
// NEG
// ======================================================================

#[test]
fn nega_ordinary() {
    let mut s = Sys::code(0x0000, &[0x40]); // NEGA
    s.cpu.a = 0x01;
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0xFF);
    assert_eq!(cycles, 2);
    // borrow (m != 0) -> C; result negative -> N; no signed overflow.
    assert_eq!(flags(&s), (false, true, false, false, true));
}

#[test]
fn nega_zero_clears_carry() {
    let mut s = Sys::code(0x0000, &[0x40]);
    s.cpu.a = 0x00;
    s.step();
    assert_eq!(s.cpu.a, 0x00);
    assert_eq!(flags(&s), (false, false, true, false, false)); // Z, no borrow
}

#[test]
fn nega_0x80_sets_overflow() {
    let mut s = Sys::code(0x0000, &[0x40]);
    s.cpu.a = 0x80;
    s.step();
    assert_eq!(s.cpu.a, 0x80); // -(-128) overflows
    assert_eq!(flags(&s), (false, true, false, true, true)); // N, V, C
}

// ======================================================================
// COM
// ======================================================================

#[test]
fn coma_always_sets_carry() {
    let mut s = Sys::code(0x0000, &[0x43]); // COMA
    s.cpu.a = 0x00;
    s.step();
    assert_eq!(s.cpu.a, 0xFF);
    // N from result; V cleared; C forced to 1.
    assert_eq!(flags(&s), (false, true, false, false, true));
}

#[test]
fn comb_to_zero() {
    let mut s = Sys::code(0x0000, &[0x53]); // COMB
    s.cpu.b = 0xFF;
    s.step();
    assert_eq!(s.cpu.b, 0x00);
    assert_eq!(flags(&s), (false, false, true, false, true)); // Z, C=1
}

// ======================================================================
// Shifts / rotates
// ======================================================================

#[test]
fn lsra_forces_n_zero_leaves_v() {
    let mut s = Sys::code(0x0000, &[0x44]); // LSRA
    s.cpu.a = 0x03;
    s.cpu.cc |= cc::OVERFLOW; // V must survive (LSR leaves V unaffected)
    s.step();
    assert_eq!(s.cpu.a, 0x01);
    assert_eq!(flags(&s), (false, false, false, true, true)); // N=0, C=bit0=1, V preserved
}

#[test]
fn asra_preserves_sign_bit() {
    let mut s = Sys::code(0x0000, &[0x47]); // ASRA
    s.cpu.a = 0x80;
    s.step();
    assert_eq!(s.cpu.a, 0xC0); // sign extended
    assert_eq!(flags(&s), (false, true, false, false, false)); // N set, bit0=0 -> C=0
}

#[test]
fn rora_rotates_carry_in() {
    let mut s = Sys::code(0x0000, &[0x46]); // RORA
    s.cpu.a = 0x01;
    s.cpu.cc |= cc::CARRY; // carry rotates into bit 7
    s.step();
    assert_eq!(s.cpu.a, 0x80);
    assert_eq!(flags(&s), (false, true, false, false, true)); // N set, C = old bit0 = 1
}

#[test]
fn asla_sets_overflow_and_carry() {
    let mut s = Sys::code(0x0000, &[0x48]); // ASLA
    s.cpu.a = 0x80;
    s.step();
    assert_eq!(s.cpu.a, 0x00);
    // C = old b7 = 1; V = b7⊕b6 = 1⊕0 = 1; result 0 -> Z.
    assert_eq!(flags(&s), (false, false, true, true, true));
}

#[test]
fn asla_no_overflow_when_signs_stable() {
    let mut s = Sys::code(0x0000, &[0x48]);
    s.cpu.a = 0xFF;
    s.step();
    assert_eq!(s.cpu.a, 0xFE);
    // C = 1; V = b7⊕b6 = 1⊕1 = 0; N set.
    assert_eq!(flags(&s), (false, true, false, false, true));
}

#[test]
fn rola_rotates_carry_in_and_out() {
    let mut s = Sys::code(0x0000, &[0x49]); // ROLA
    s.cpu.a = 0x40;
    s.cpu.cc |= cc::CARRY; // in
    s.step();
    assert_eq!(s.cpu.a, 0x81); // (0x40 << 1) | 1
    // C = old b7 = 0; V = b7⊕b6 = 0⊕1 = 1; N set.
    assert_eq!(flags(&s), (false, true, false, true, false));
}

// ======================================================================
// INC / DEC (C must be untouched)
// ======================================================================

#[test]
fn inca_overflow_leaves_carry() {
    let mut s = Sys::code(0x0000, &[0x4C]); // INCA
    s.cpu.a = 0x7F;
    s.cpu.cc |= cc::CARRY; // must be preserved
    s.step();
    assert_eq!(s.cpu.a, 0x80);
    assert_eq!(flags(&s), (false, true, false, true, true)); // N, V, C preserved
}

#[test]
fn deca_overflow_leaves_carry() {
    let mut s = Sys::code(0x0000, &[0x4A]); // DECA
    s.cpu.a = 0x80;
    s.step();
    assert_eq!(s.cpu.a, 0x7F);
    assert_eq!(flags(&s), (false, false, false, true, false)); // V set, C untouched (clear)
}

#[test]
fn deca_to_zero() {
    let mut s = Sys::code(0x0000, &[0x4A]);
    s.cpu.a = 0x01;
    s.step();
    assert_eq!(s.cpu.a, 0x00);
    assert_eq!(flags(&s), (false, false, true, false, false));
}

// ======================================================================
// TST / CLR
// ======================================================================

#[test]
fn tsta_flags_only_preserves_carry() {
    let mut s = Sys::code(0x0000, &[0x4D]); // TSTA
    s.cpu.a = 0x80;
    s.cpu.cc |= cc::CARRY; // TST leaves C untouched
    s.step();
    assert_eq!(s.cpu.a, 0x80);
    assert_eq!(flags(&s), (false, true, false, false, true)); // N set, V cleared, C preserved
}

#[test]
fn clra_clears_everything_but_z() {
    let mut s = Sys::code(0x0000, &[0x4F]); // CLRA
    s.cpu.a = 0xAA;
    s.cpu.cc |= cc::NEGATIVE | cc::OVERFLOW | cc::CARRY;
    s.step();
    assert_eq!(s.cpu.a, 0x00);
    assert_eq!(flags(&s), (false, false, true, false, false));
}

// ======================================================================
// Memory (direct / indexed / extended) forms
// ======================================================================

#[test]
fn neg_extended_writes_back() {
    let mut s = Sys::code(0x0000, &[0x70, 0x20, 0x00]); // NEG >$2000
    s.set_mem(0x2000, 0x01);
    let cycles = s.step();
    assert_eq!(s.mem(0x2000), 0xFF);
    assert_eq!(cycles, 7);
    assert_eq!(flags(&s), (false, true, false, false, true));
}

#[test]
fn inc_direct_writes_back() {
    let mut s = Sys::code(0x0000, &[0x0C, 0x40]); // INC <$40
    s.cpu.dp = 0x00;
    s.set_mem(0x0040, 0x7F);
    let cycles = s.step();
    assert_eq!(s.mem(0x0040), 0x80);
    assert_eq!(cycles, 6);
    assert_eq!(flags(&s), (false, true, false, true, false)); // V on 0x7F->0x80
}

#[test]
fn clr_extended_zeroes_memory() {
    let mut s = Sys::code(0x0000, &[0x7F, 0x30, 0x00]); // CLR >$3000
    s.set_mem(0x3000, 0xAA);
    let cycles = s.step();
    assert_eq!(s.mem(0x3000), 0x00);
    assert_eq!(cycles, 7);
    assert_eq!(flags(&s), (false, false, true, false, false));
}

#[test]
fn tst_extended_does_not_write() {
    let mut s = Sys::code(0x0000, &[0x7D, 0x30, 0x00]); // TST >$3000
    s.set_mem(0x3000, 0x80);
    let cycles = s.step();
    assert_eq!(s.mem(0x3000), 0x80); // untouched
    assert_eq!(cycles, 7);
    assert_eq!(flags(&s), (false, true, false, false, false)); // N from value
}

#[test]
fn lsr_indexed_writes_back_with_extra_cycles() {
    let mut s = Sys::code(0x0000, &[0x64, 0x84]); // LSR ,X
    s.cpu.x = 0x2000;
    s.set_mem(0x2000, 0x02);
    let cycles = s.step();
    assert_eq!(s.mem(0x2000), 0x01);
    assert_eq!(cycles, 6); // 6 + 0 (no-offset postbyte)
    assert_eq!(flags(&s), (false, false, false, false, false)); // bit0=0 -> C=0
}
