//! Test-driven coverage for the branch instructions: short `Bcc` (8-bit signed
//! offset, 3 cycles), long `LBcc` (16-bit offset, 5 cycles / 6 if taken), and
//! the unconditional/never forms (BRA/BRN/LBRA/LBRN).
//!
//! Branch offsets are relative to the address *after* the operand — i.e. the
//! next instruction. Condition logic verified against the Atkinson reference:
//! BHI = !C·!Z, BLS = C+Z, BGE = (N==V), BLT = (N!=V), BGT = !Z·(N==V),
//! BLE = Z+(N!=V).

mod common;

use common::Sys;
use mc6809::cc;

// ---- short branches: mechanics ------------------------------------------

#[test]
fn bra_always_taken_forward() {
    // BRA +4 at $1000: opcode@1000, offset@1001 -> next PC 0x1002; +4 = 0x1006.
    let mut s = Sys::code(0x1000, &[0x20, 0x04]);
    let cycles = s.step();
    assert_eq!(s.cpu.pc, 0x1006);
    assert_eq!(cycles, 3);
}

#[test]
fn bra_backward_negative_offset() {
    // BRA -2 -> infinite self-loop: next PC 0x1002, + (-2) = 0x1000.
    let mut s = Sys::code(0x1000, &[0x20, 0xFE]);
    s.step();
    assert_eq!(s.cpu.pc, 0x1000);
}

#[test]
fn brn_never_taken_but_consumes_offset() {
    // BRN +4: never branches, but the offset byte is still consumed.
    let mut s = Sys::code(0x1000, &[0x21, 0x04]);
    let cycles = s.step();
    assert_eq!(s.cpu.pc, 0x1002); // fell through past the operand
    assert_eq!(cycles, 3);
}

#[test]
fn not_taken_branch_still_consumes_offset_and_costs_3() {
    // BEQ +4 with Z clear -> not taken; PC advances past operand only.
    let mut s = Sys::code(0x1000, &[0x27, 0x04]);
    // Z is clear by default.
    let cycles = s.step();
    assert_eq!(s.cpu.pc, 0x1002);
    assert_eq!(cycles, 3);
}

// ---- conditional short branches: each flag ------------------------------

/// Helper: assemble `[opcode, +2]` at $1000, apply `setup`, step, and report
/// whether the branch was taken (PC moved to 0x1004 vs fell through to 0x1002).
fn took(opcode: u8, setup: impl FnOnce(&mut Sys)) -> bool {
    let mut s = Sys::code(0x1000, &[opcode, 0x02]);
    setup(&mut s);
    s.step();
    match s.cpu.pc {
        0x1004 => true,
        0x1002 => false,
        other => panic!("unexpected PC {other:#06x}"),
    }
}

fn set(flag: u8) -> impl FnOnce(&mut Sys) {
    move |s| s.cpu.cc |= flag
}
fn none(_: &mut Sys) {}

#[test]
fn beq_bne_on_zero() {
    assert!(took(0x27, set(cc::ZERO))); // BEQ, Z=1
    assert!(!took(0x27, none)); // BEQ, Z=0
    assert!(took(0x26, none)); // BNE, Z=0
    assert!(!took(0x26, set(cc::ZERO))); // BNE, Z=1
}

#[test]
fn bcc_bcs_on_carry() {
    assert!(took(0x24, none)); // BHS/BCC, C=0
    assert!(!took(0x24, set(cc::CARRY)));
    assert!(took(0x25, set(cc::CARRY))); // BLO/BCS, C=1
    assert!(!took(0x25, none));
}

#[test]
fn bmi_bpl_on_negative() {
    assert!(took(0x2B, set(cc::NEGATIVE))); // BMI
    assert!(!took(0x2B, none));
    assert!(took(0x2A, none)); // BPL
    assert!(!took(0x2A, set(cc::NEGATIVE)));
}

#[test]
fn bvs_bvc_on_overflow() {
    assert!(took(0x29, set(cc::OVERFLOW))); // BVS
    assert!(!took(0x29, none));
    assert!(took(0x28, none)); // BVC
    assert!(!took(0x28, set(cc::OVERFLOW)));
}

#[test]
fn bhi_bls_unsigned() {
    // BHI (0x22): taken iff C=0 and Z=0.
    assert!(took(0x22, none));
    assert!(!took(0x22, set(cc::CARRY)));
    assert!(!took(0x22, set(cc::ZERO)));
    // BLS (0x23): taken iff C=1 or Z=1.
    assert!(!took(0x23, none));
    assert!(took(0x23, set(cc::CARRY)));
    assert!(took(0x23, set(cc::ZERO)));
}

#[test]
fn bge_blt_signed() {
    // BGE (0x2C): taken iff N==V.
    assert!(took(0x2C, none)); // N=0,V=0
    assert!(took(0x2C, |s| s.cpu.cc |= cc::NEGATIVE | cc::OVERFLOW)); // N=1,V=1
    assert!(!took(0x2C, set(cc::NEGATIVE))); // N=1,V=0
    assert!(!took(0x2C, set(cc::OVERFLOW))); // N=0,V=1
    // BLT (0x2D): taken iff N!=V.
    assert!(took(0x2D, set(cc::NEGATIVE)));
    assert!(!took(0x2D, none));
}

#[test]
fn bgt_ble_signed() {
    // BGT (0x2E): taken iff Z=0 and N==V.
    assert!(took(0x2E, none)); // Z=0, N==V
    assert!(!took(0x2E, set(cc::ZERO))); // Z=1 blocks it
    assert!(!took(0x2E, set(cc::NEGATIVE))); // N!=V blocks it
    // BLE (0x2F): taken iff Z=1 or N!=V.
    assert!(!took(0x2F, none));
    assert!(took(0x2F, set(cc::ZERO)));
    assert!(took(0x2F, set(cc::NEGATIVE)));
}

// ---- long branches ------------------------------------------------------

#[test]
fn lbra_16bit_offset() {
    // LBRA +$0100 at $1000: opcode@1000, offset@1001-1002 -> next PC 0x1003;
    // +0x0100 = 0x1103.
    let mut s = Sys::code(0x1000, &[0x16, 0x01, 0x00]);
    let cycles = s.step();
    assert_eq!(s.cpu.pc, 0x1103);
    assert_eq!(cycles, 5);
}

#[test]
fn lbra_backward() {
    // LBRA -3 -> self-loop: next PC 0x1003, + (-3) = 0x1000.
    let mut s = Sys::code(0x1000, &[0x16, 0xFF, 0xFD]);
    s.step();
    assert_eq!(s.cpu.pc, 0x1000);
}

#[test]
fn lbeq_taken_costs_6() {
    // LBEQ +$0100 with Z set: $10 prefix, op 0x27.
    let mut s = Sys::code(0x1000, &[0x10, 0x27, 0x01, 0x00]);
    s.cpu.cc |= cc::ZERO;
    let cycles = s.step();
    // opcode bytes: prefix@1000, op@1001, offset@1002-1003 -> next PC 0x1004; +0x100.
    assert_eq!(s.cpu.pc, 0x1104);
    assert_eq!(cycles, 6);
}

#[test]
fn lbeq_not_taken_costs_5() {
    // LBEQ with Z clear: not taken, falls through, 5 cycles.
    let mut s = Sys::code(0x1000, &[0x10, 0x27, 0x01, 0x00]);
    let cycles = s.step();
    assert_eq!(s.cpu.pc, 0x1004); // past the 4 instruction bytes
    assert_eq!(cycles, 5);
}

#[test]
fn lbrn_never_taken_costs_5() {
    // LBRN: $10 0x21, never branches, consumes 4 bytes, 5 cycles.
    let mut s = Sys::code(0x1000, &[0x10, 0x21, 0x12, 0x34]);
    let cycles = s.step();
    assert_eq!(s.cpu.pc, 0x1004);
    assert_eq!(cycles, 5);
}

// ---- integration: a countdown loop --------------------------------------

#[test]
fn countdown_loop_runs_to_zero() {
    // LDB #3; loop: DECB; BNE loop  — B should reach 0 after the loop.
    //  $0000: C6 03      LDB #3
    //  $0002: 5A         DECB
    //  $0003: 26 FD      BNE -3  (back to $0002)
    let mut s = Sys::code(0x0000, &[0xC6, 0x03, 0x5A, 0x26, 0xFD]);
    s.step(); // LDB #3
    assert_eq!(s.cpu.b, 3);
    // Run DECB/BNE until the branch falls through.
    for _ in 0..3 {
        s.step(); // DECB
        s.step(); // BNE
    }
    assert_eq!(s.cpu.b, 0);
    assert_ne!(s.cpu.cc & cc::ZERO, 0); // last DECB set Z
    assert_eq!(s.cpu.pc, 0x0005); // fell past the branch
}
