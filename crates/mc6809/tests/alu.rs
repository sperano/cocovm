//! Test-driven coverage for the 8-bit arithmetic ALU: ADD/ADC/SUB/SBC/CMP for
//! both accumulators across immediate / direct / extended addressing. The focus
//! is flag semantics — H (half-carry), N, Z, V (signed overflow), and C (carry
//! out on add / borrow on subtract) — per the MC6809 datasheet.
//!
//! Convention: ADD/ADC set H, N, Z, V, C. SUB/SBC/CMP set N, Z, V, C and leave
//! H undefined (unaffected here).

mod common;

use common::Sys;
use mc6809::cc;

/// (H, N, Z, V, C) as booleans, for compact assertions.
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

// ---- ADDA ---------------------------------------------------------------

#[test]
fn adda_immediate_plain() {
    let mut s = Sys::code(0x0000, &[0x8B, 0x25]); // ADDA #$25
    s.cpu.a = 0x25;
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x4A);
    assert_eq!(cycles, 2);
    assert_eq!(flags(&s), (false, false, false, false, false));
}

#[test]
fn adda_half_carry_from_low_nibble() {
    let mut s = Sys::code(0x0000, &[0x8B, 0x01]); // ADDA #$01
    s.cpu.a = 0x0F;
    s.step();
    assert_eq!(s.cpu.a, 0x10);
    assert_eq!(flags(&s), (true, false, false, false, false)); // H set
}

#[test]
fn adda_carry_and_zero_wraps() {
    let mut s = Sys::code(0x0000, &[0x8B, 0x01]); // ADDA #$01
    s.cpu.a = 0xFF;
    s.step();
    assert_eq!(s.cpu.a, 0x00);
    // 0xF + 0x1 overflows the low nibble too -> H set; result 0 -> Z; carry out -> C.
    assert_eq!(flags(&s), (true, false, true, false, true));
}

#[test]
fn adda_signed_overflow() {
    let mut s = Sys::code(0x0000, &[0x8B, 0x01]); // ADDA #$01
    s.cpu.a = 0x7F;
    s.step();
    assert_eq!(s.cpu.a, 0x80);
    // 0x7F + 1 = 0x80: positive+positive -> negative -> V and N set; H from low nibble.
    assert_eq!(flags(&s), (true, true, false, true, false));
}

#[test]
fn adda_direct_and_extended_cycles() {
    let mut s = Sys::code(0x0000, &[0x9B, 0x40]); // ADDA <$40
    s.cpu.dp = 0x00;
    s.cpu.a = 0x01;
    s.set_mem(0x0040, 0x02);
    assert_eq!(s.step(), 4);
    assert_eq!(s.cpu.a, 0x03);

    let mut s = Sys::code(0x0000, &[0xBB, 0x12, 0x34]); // ADDA >$1234
    s.cpu.a = 0x10;
    s.set_mem(0x1234, 0x20);
    assert_eq!(s.step(), 5);
    assert_eq!(s.cpu.a, 0x30);
}

// ---- ADDB ---------------------------------------------------------------

#[test]
fn addb_immediate() {
    let mut s = Sys::code(0x0000, &[0xCB, 0x10]); // ADDB #$10
    s.cpu.b = 0x05;
    let cycles = s.step();
    assert_eq!(s.cpu.b, 0x15);
    assert_eq!(cycles, 2);
}

// ---- ADCA ---------------------------------------------------------------

#[test]
fn adca_adds_carry_in() {
    let mut s = Sys::code(0x0000, &[0x89, 0x00]); // ADCA #$00
    s.cpu.a = 0x00;
    s.cpu.cc |= cc::CARRY;
    s.step();
    assert_eq!(s.cpu.a, 0x01);
    assert_eq!(flags(&s), (false, false, false, false, false));
}

#[test]
fn adca_carry_in_completes_wrap() {
    let mut s = Sys::code(0x0000, &[0x89, 0xFF]); // ADCA #$FF
    s.cpu.a = 0x00;
    s.cpu.cc |= cc::CARRY;
    s.step();
    // 0x00 + 0xFF + 1 = 0x100 -> 0x00, carry out.
    assert_eq!(s.cpu.a, 0x00);
    assert_eq!(flags(&s), (true, false, true, false, true));
}

// ---- SUBA ---------------------------------------------------------------

#[test]
fn suba_plain_no_borrow() {
    let mut s = Sys::code(0x0000, &[0x80, 0x03]); // SUBA #$03
    s.cpu.a = 0x05;
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x02);
    assert_eq!(cycles, 2);
    // H is left unaffected by subtraction; starts clear here.
    assert_eq!(flags(&s), (false, false, false, false, false));
}

#[test]
fn suba_borrow_sets_carry() {
    let mut s = Sys::code(0x0000, &[0x80, 0x01]); // SUBA #$01
    s.cpu.a = 0x00;
    s.step();
    assert_eq!(s.cpu.a, 0xFF);
    // borrow -> C; result negative -> N; no signed overflow.
    assert_eq!(flags(&s), (false, true, false, false, true));
}

#[test]
fn suba_signed_overflow() {
    let mut s = Sys::code(0x0000, &[0x80, 0x01]); // SUBA #$01
    s.cpu.a = 0x80;
    s.step();
    assert_eq!(s.cpu.a, 0x7F);
    // 0x80 - 1 = 0x7F: negative - positive -> positive -> V set, N clear, no borrow.
    assert_eq!(flags(&s), (false, false, false, true, false));
}

#[test]
fn suba_equal_sets_zero() {
    let mut s = Sys::code(0x0000, &[0x80, 0x42]); // SUBA #$42
    s.cpu.a = 0x42;
    s.step();
    assert_eq!(s.cpu.a, 0x00);
    assert_eq!(flags(&s), (false, false, true, false, false));
}

#[test]
fn suba_leaves_half_carry_untouched() {
    let mut s = Sys::code(0x0000, &[0x80, 0x01]); // SUBA #$01
    s.cpu.a = 0x10;
    s.cpu.cc |= cc::HALF_CARRY; // pre-set H
    s.step();
    assert_eq!(s.cpu.a, 0x0F);
    assert!(s.cpu.cc & cc::HALF_CARRY != 0); // still set — subtraction doesn't clear it
}

// ---- SUBB ---------------------------------------------------------------

#[test]
fn subb_extended() {
    let mut s = Sys::code(0x0000, &[0xF0, 0x20, 0x00]); // SUBB >$2000
    s.cpu.b = 0x50;
    s.set_mem(0x2000, 0x30);
    let cycles = s.step();
    assert_eq!(s.cpu.b, 0x20);
    assert_eq!(cycles, 5);
}

// ---- SBCA ---------------------------------------------------------------

#[test]
fn sbca_subtracts_borrow_in() {
    let mut s = Sys::code(0x0000, &[0x82, 0x01]); // SBCA #$01
    s.cpu.a = 0x05;
    s.cpu.cc |= cc::CARRY; // borrow-in
    s.step();
    assert_eq!(s.cpu.a, 0x03); // 5 - 1 - 1
    assert_eq!(flags(&s), (false, false, false, false, false));
}

// ---- CMPA / CMPB --------------------------------------------------------

#[test]
fn cmpa_equal_sets_zero_preserves_a() {
    let mut s = Sys::code(0x0000, &[0x81, 0x42]); // CMPA #$42
    s.cpu.a = 0x42;
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x42); // A unchanged
    assert_eq!(cycles, 2);
    assert_eq!(flags(&s), (false, false, true, false, false));
}

#[test]
fn cmpa_less_than_sets_carry_and_negative() {
    let mut s = Sys::code(0x0000, &[0x81, 0x10]); // CMPA #$10
    s.cpu.a = 0x05;
    s.step();
    assert_eq!(s.cpu.a, 0x05); // A unchanged
    // 0x05 - 0x10 borrows -> C; result 0xF5 negative -> N.
    assert_eq!(flags(&s), (false, true, false, false, true));
}

#[test]
fn cmpb_direct_preserves_b() {
    let mut s = Sys::code(0x0000, &[0xD1, 0x80]); // CMPB <$80
    s.cpu.dp = 0x00;
    s.cpu.b = 0x20;
    s.set_mem(0x0080, 0x20);
    let cycles = s.step();
    assert_eq!(s.cpu.b, 0x20);
    assert_eq!(cycles, 4);
    assert_eq!(flags(&s), (false, false, true, false, false));
}
