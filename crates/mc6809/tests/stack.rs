//! Test-driven coverage for the subroutine / stack / register-transfer group:
//! JMP, JSR, BSR/LBSR, RTS, TFR, EXG, and PSHS/PULS/PSHU/PULU.
//!
//! Stack conventions (verified against the Atkinson reference): push is
//! pre-decrement, storing 16-bit values big-endian (high byte at the lower
//! address); push order is PC,U/S,Y,X,DP,B,A,CC so CC ends up on top. PSH/PUL
//! cost 5 cycles + 1 per byte. Register nibble codes: D=0 X=1 Y=2 U=3 S=4 PC=5
//! A=8 B=9 CC=A DP=B.

mod common;

use common::Sys;

// ======================================================================
// JMP
// ======================================================================

#[test]
fn jmp_extended() {
    let mut s = Sys::code(0x1000, &[0x7E, 0x12, 0x34]);
    let cycles = s.step();
    assert_eq!(s.cpu.pc, 0x1234);
    assert_eq!(cycles, 4);
}

#[test]
fn jmp_direct_uses_dp() {
    let mut s = Sys::code(0x1000, &[0x0E, 0x10]); // JMP <$10, DP=$05 -> $0510
    s.cpu.dp = 0x05;
    let cycles = s.step();
    assert_eq!(s.cpu.pc, 0x0510);
    assert_eq!(cycles, 3);
}

#[test]
fn jmp_indexed() {
    let mut s = Sys::code(0x1000, &[0x6E, 0x84]); // JMP ,X
    s.cpu.x = 0x3000;
    let cycles = s.step();
    assert_eq!(s.cpu.pc, 0x3000);
    assert_eq!(cycles, 3);
}

// ======================================================================
// JSR / BSR / LBSR / RTS
// ======================================================================

#[test]
fn jsr_extended_pushes_return_then_rts_returns() {
    // $1000: JSR $2000   (return addr = $1003)
    // $2000: RTS
    let mut s = Sys::code(0x1000, &[0xBD, 0x20, 0x00]);
    s.bus.load(0x2000, &[0x39]);
    s.cpu.s = 0x2000;

    let c1 = s.step(); // JSR
    assert_eq!(s.cpu.pc, 0x2000);
    assert_eq!(s.cpu.s, 0x1FFE);
    assert_eq!(s.mem(0x1FFE), 0x10); // high byte of return addr (big-endian)
    assert_eq!(s.mem(0x1FFF), 0x03); // low byte
    assert_eq!(c1, 8);

    let c2 = s.step(); // RTS
    assert_eq!(s.cpu.pc, 0x1003);
    assert_eq!(s.cpu.s, 0x2000);
    assert_eq!(c2, 5);
}

#[test]
fn bsr_and_rts_round_trip() {
    // $1000: BSR +$0E -> $1010 ; return addr = $1002
    // $1010: RTS
    let mut s = Sys::code(0x1000, &[0x8D, 0x0E]);
    s.bus.load(0x1010, &[0x39]);
    s.cpu.s = 0x2000;

    let c1 = s.step(); // BSR
    assert_eq!(s.cpu.pc, 0x1010);
    assert_eq!(s.cpu.s, 0x1FFE);
    assert_eq!(c1, 7);

    let c2 = s.step(); // RTS
    assert_eq!(s.cpu.pc, 0x1002);
    assert_eq!(s.cpu.s, 0x2000);
    assert_eq!(c2, 5);
}

#[test]
fn lbsr_pushes_and_costs_9() {
    // $1000: LBSR +$0020 ; return addr = $1003 -> target $1023
    let mut s = Sys::code(0x1000, &[0x17, 0x00, 0x20]);
    s.cpu.s = 0x2000;
    let cycles = s.step();
    assert_eq!(s.cpu.pc, 0x1023);
    assert_eq!(s.cpu.s, 0x1FFE);
    assert_eq!(s.mem(0x1FFE), 0x10);
    assert_eq!(s.mem(0x1FFF), 0x03);
    assert_eq!(cycles, 9);
}

// ======================================================================
// TFR
// ======================================================================

#[test]
fn tfr_d_to_x_16bit() {
    let mut s = Sys::code(0x0000, &[0x1F, 0x01]); // TFR D,X
    s.cpu.set_d(0x1234);
    let cycles = s.step();
    assert_eq!(s.cpu.x, 0x1234);
    assert_eq!(cycles, 6);
}

#[test]
fn tfr_a_to_b_8bit() {
    let mut s = Sys::code(0x0000, &[0x1F, 0x89]); // TFR A,B
    s.cpu.a = 0x55;
    s.step();
    assert_eq!(s.cpu.b, 0x55);
}

#[test]
fn tfr_16_to_8_takes_lsb() {
    let mut s = Sys::code(0x0000, &[0x1F, 0x18]); // TFR X,A
    s.cpu.x = 0x1234;
    s.step();
    assert_eq!(s.cpu.a, 0x34); // low byte only
}

#[test]
fn tfr_accumulator_to_16_sets_ff_high() {
    let mut s = Sys::code(0x0000, &[0x1F, 0x81]); // TFR A,X
    s.cpu.a = 0x7F;
    s.step();
    assert_eq!(s.cpu.x, 0xFF7F); // A/B -> 16: MSB = $FF
}

#[test]
fn tfr_cc_to_16_duplicates_byte() {
    let mut s = Sys::code(0x0000, &[0x1F, 0xA1]); // TFR CC,X
    s.cpu.cc = 0x42;
    s.step();
    assert_eq!(s.cpu.x, 0x4242); // CC/DP -> 16: both bytes = source
}

// ======================================================================
// EXG
// ======================================================================

#[test]
fn exg_a_b() {
    let mut s = Sys::code(0x0000, &[0x1E, 0x89]); // EXG A,B
    s.cpu.a = 0x11;
    s.cpu.b = 0x22;
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x22);
    assert_eq!(s.cpu.b, 0x11);
    assert_eq!(cycles, 8);
}

#[test]
fn exg_x_y() {
    let mut s = Sys::code(0x0000, &[0x1E, 0x12]); // EXG X,Y
    s.cpu.x = 0x1111;
    s.cpu.y = 0x2222;
    s.step();
    assert_eq!(s.cpu.x, 0x2222);
    assert_eq!(s.cpu.y, 0x1111);
}

#[test]
fn exg_a_x_sets_ff_high() {
    // EXG A,X: A is first-named (8-bit) -> both operands read/write as
    // 16-bit with A/X's widening rules; A ends up as X's low byte.
    let mut s = Sys::code(0x0000, &[0x1E, 0x81]); // EXG A,X
    s.cpu.a = 0x12;
    s.cpu.x = 0x3456;
    let cycles = s.step();
    assert_eq!(s.cpu.x, 0xFF12);
    assert_eq!(s.cpu.a, 0x56);
    assert_eq!(cycles, 8);
}

#[test]
fn exg_x_a_same_as_a_x() {
    // EXG X,A: A/B widening is order-independent, so this matches EXG A,X.
    let mut s = Sys::code(0x0000, &[0x1E, 0x18]); // EXG X,A
    s.cpu.a = 0x12;
    s.cpu.x = 0x3456;
    s.step();
    assert_eq!(s.cpu.x, 0xFF12);
    assert_eq!(s.cpu.a, 0x56);
}

#[test]
fn exg_cc_x_duplicates() {
    // EXG CC,X: CC is first-named (8-bit) -> CC's read duplicates its byte
    // into both halves.
    let mut s = Sys::code(0x0000, &[0x1E, 0xA1]); // EXG CC,X
    s.cpu.cc = 0x42;
    s.cpu.x = 0x3456;
    s.step();
    assert_eq!(s.cpu.x, 0x4242);
    assert_eq!(s.cpu.cc, 0x56);
}

#[test]
fn exg_x_cc_pads_ff() {
    // EXG X,CC: X is first-named (16-bit) -> CC's read pads $FF instead of
    // duplicating, unlike EXG CC,X. Order matters for CC/DP.
    let mut s = Sys::code(0x0000, &[0x1E, 0x1A]); // EXG X,CC
    s.cpu.cc = 0x42;
    s.cpu.x = 0x3456;
    s.step();
    assert_eq!(s.cpu.x, 0xFF42);
    assert_eq!(s.cpu.cc, 0x56);
}

#[test]
fn exg_a_d_acts_like_a_b() {
    // EXG A,D: A overlaps D's high half, and the write order (D then A)
    // makes A's later write override D's $FF MSB, reducing to EXG A,B.
    let mut s = Sys::code(0x0000, &[0x1E, 0x80]); // EXG A,D
    s.cpu.a = 0x11;
    s.cpu.b = 0x22;
    s.step();
    assert_eq!(s.cpu.a, 0x22);
    assert_eq!(s.cpu.b, 0x11);
}

#[test]
fn exg_d_a_leaves_ff_in_a() {
    // EXG D,A: D is first-named (16-bit), so the later D write clobbers A
    // with D's original low byte's $FF-padded exchange, leaving A=$FF.
    let mut s = Sys::code(0x0000, &[0x1E, 0x08]); // EXG D,A
    s.cpu.a = 0x11;
    s.cpu.b = 0x22;
    s.step();
    assert_eq!(s.cpu.a, 0xFF);
    assert_eq!(s.cpu.b, 0x11);
}

#[test]
fn exg_invalid_code_gives_ffff() {
    // EXG X,<invalid $C>: the 6x09 Instruction Sets doc specifies an invalid
    // encoding exchanges a constant $FF/$FFFF; the write to it is dropped.
    let mut s = Sys::code(0x0000, &[0x1E, 0x1C]); // EXG X,$C
    s.cpu.x = 0x3456;
    s.step();
    assert_eq!(s.cpu.x, 0xFFFF);
}

// ======================================================================
// PSH / PUL
// ======================================================================

#[test]
fn pshs_single_byte_cost_6() {
    let mut s = Sys::code(0x0000, &[0x34, 0x02]); // PSHS A
    s.cpu.s = 0x2000;
    s.cpu.a = 0x42;
    let cycles = s.step();
    assert_eq!(s.cpu.s, 0x1FFF);
    assert_eq!(s.mem(0x1FFF), 0x42);
    assert_eq!(cycles, 6); // 5 + 1
}

#[test]
fn pshs_16bit_is_big_endian() {
    let mut s = Sys::code(0x0000, &[0x34, 0x10]); // PSHS X
    s.cpu.s = 0x2000;
    s.cpu.x = 0x1234;
    let cycles = s.step();
    assert_eq!(s.cpu.s, 0x1FFE);
    assert_eq!(s.mem(0x1FFE), 0x12); // high byte at lower address
    assert_eq!(s.mem(0x1FFF), 0x34);
    assert_eq!(cycles, 7); // 5 + 2
}

#[test]
fn pshs_all_registers_cost_17() {
    // Mask $FF pushes PC,U,Y,X,DP,B,A,CC = 12 bytes.
    let mut s = Sys::code(0x0000, &[0x34, 0xFF]);
    s.cpu.s = 0x2000;
    let cycles = s.step();
    assert_eq!(s.cpu.s, 0x2000 - 12);
    assert_eq!(cycles, 17); // 5 + 12
}

#[test]
fn pshs_pulls_restore_all_registers() {
    // Push A,B,X,Y,DP,CC (mask $3F), clobber, pull back.
    const MASK: u8 = 0x3F;
    let mut s = Sys::code(0x0000, &[0x34, MASK, 0x35, MASK]); // PSHS then PULS
    s.cpu.s = 0x2000;
    s.cpu.a = 0xAA;
    s.cpu.b = 0xBB;
    s.cpu.x = 0x1234;
    s.cpu.y = 0x5678;
    s.cpu.dp = 0xCD;
    s.cpu.cc = 0x21;

    let push_cycles = s.step(); // PSHS
    assert_eq!(s.cpu.s, 0x2000 - 8); // 8 bytes
    assert_eq!(push_cycles, 13); // 5 + 8

    // Clobber everything.
    s.cpu.a = 0;
    s.cpu.b = 0;
    s.cpu.x = 0;
    s.cpu.y = 0;
    s.cpu.dp = 0;
    s.cpu.cc = 0;

    let pull_cycles = s.step(); // PULS
    assert_eq!(s.cpu.a, 0xAA);
    assert_eq!(s.cpu.b, 0xBB);
    assert_eq!(s.cpu.x, 0x1234);
    assert_eq!(s.cpu.y, 0x5678);
    assert_eq!(s.cpu.dp, 0xCD);
    assert_eq!(s.cpu.cc, 0x21);
    assert_eq!(s.cpu.s, 0x2000); // pointer back to start
    assert_eq!(pull_cycles, 13);
}

#[test]
fn pshu_uses_user_stack() {
    let mut s = Sys::code(0x0000, &[0x36, 0x02]); // PSHU A
    s.cpu.u = 0x3000;
    s.cpu.a = 0x42;
    s.step();
    assert_eq!(s.cpu.u, 0x2FFF);
    assert_eq!(s.mem(0x2FFF), 0x42);
}

#[test]
fn pshs_can_push_and_pull_u_via_bit6() {
    // PSHS U (bit6) then PULS U — the "other" stack pointer round-trips.
    let mut s = Sys::code(0x0000, &[0x34, 0x40, 0x35, 0x40]);
    s.cpu.s = 0x2000;
    s.cpu.u = 0x1234;
    s.step(); // PSHS U
    assert_eq!(s.cpu.s, 0x1FFE);
    assert_eq!(s.mem(0x1FFE), 0x12);
    assert_eq!(s.mem(0x1FFF), 0x34);
    s.cpu.u = 0; // clobber
    s.step(); // PULS U
    assert_eq!(s.cpu.u, 0x1234);
    assert_eq!(s.cpu.s, 0x2000);
}
