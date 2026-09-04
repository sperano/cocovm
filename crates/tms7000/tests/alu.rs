//! Two-operand arithmetic/logic across the seven `$1x-$7x` addressing rows:
//! cycle counts per row (MAME `am_*` base costs) and result/flag semantics
//! per op (MAME `op_*`), including MPY's fixed A:B destination.

mod common;

use common::Sys;

/// `(row base opcode, base cycles)` for the `Rn,A` / `%n,A` / `Rn,B` /
/// `Rs,Rd` / `%n,B` / `B,A` / `%n,Rd` rows.
const ROWS: [(u8, u32); 7] = [
    (0x10, 8),
    (0x20, 7),
    (0x30, 8),
    (0x40, 10),
    (0x50, 7),
    (0x60, 5),
    (0x70, 9),
];

const MOV: u8 = 0x2;
const AND: u8 = 0x3;
const OR: u8 = 0x4;
const XOR: u8 = 0x5;
const ADD: u8 = 0x8;
const SUB: u8 = 0xA;

const MPY_EXTRA: u32 = 39;

/// Run `op` in every row with source 0x03 and destination 0x05, checking the
/// per-row cycle cost and that the destination ends up as `expect`.
fn every_row(op: u8, expect: u8) {
    for (row, cycles) in ROWS {
        let (code, dest): (Vec<u8>, u8) = match row {
            0x10 => (vec![row | op, 0x10], 0),          // R16,A
            0x20 => (vec![row | op, 0x03], 0),          // %3,A
            0x30 => (vec![row | op, 0x10], 1),          // R16,B
            0x40 => (vec![row | op, 0x10, 0x11], 0x11), // R16,R17
            0x50 => (vec![row | op, 0x03], 1),          // %3,B
            0x60 => (vec![row | op], 0),                // B,A
            0x70 => (vec![row | op, 0x03, 0x11], 0x11), // %3,R17
            _ => unreachable!(),
        };
        let mut s = Sys::code(&code);
        s.cpu.set_rf(0x10, 0x03);
        s.set_b(if row == 0x60 { 0x03 } else { 0x05 });
        s.set_a(0x05);
        s.cpu.set_rf(0x11, 0x05);
        s.cpu.set_rf(dest, 0x05);
        assert_eq!(s.insn(), cycles, "row {row:#04x} op {op:#x} cycles");
        assert_eq!(s.cpu.rf(dest), expect, "row {row:#04x} op {op:#x} result");
    }
}

#[test]
fn mov_and_or_xor_in_every_row() {
    every_row(MOV, 0x03);
    every_row(AND, 0x05 & 0x03);
    every_row(OR, 0x05 | 0x03);
    every_row(XOR, 0x05 ^ 0x03);
}

#[test]
fn add_sub_in_every_row() {
    every_row(ADD, 0x08);
    every_row(SUB, 0x02);
}

#[test]
fn add_sets_carry_and_zero_on_wrap() {
    let mut s = Sys::code(&[0x28, 0x01]); // ADD %1,A
    s.set_a(0xFF);
    s.insn();
    assert_eq!(s.a(), 0);
    assert_eq!(s.flags(), (true, false, true));
}

#[test]
fn adc_folds_the_carry_in() {
    let mut s = Sys::code(&[0x29, 0x10]); // ADC %>10,A
    s.set_a(0x20);
    s.set_c(true);
    s.insn();
    assert_eq!(s.a(), 0x31);
    assert_eq!(s.flags(), (false, false, false));
}

#[test]
fn sub_carry_means_no_borrow() {
    let mut s = Sys::code(&[0x2A, 0x01]); // SUB %1,A
    s.set_a(0x01);
    s.insn();
    assert_eq!(s.a(), 0);
    assert_eq!(s.flags(), (true, false, true), "C set: no borrow");

    let mut s = Sys::code(&[0x2A, 0x02]);
    s.set_a(0x01);
    s.insn();
    assert_eq!(s.a(), 0xFF);
    assert_eq!(s.flags(), (false, true, false), "C clear: borrow");
}

#[test]
fn sbb_subtracts_the_borrow() {
    let mut s = Sys::code(&[0x2B, 0x01]); // SBB %1,A
    s.set_a(0x10);
    s.set_c(false);
    s.insn();
    assert_eq!(s.a(), 0x0E);
    assert_eq!(s.flags(), (true, false, false));
}

#[test]
fn cmp_sets_flags_without_writing() {
    let mut s = Sys::code(&[0x2D, 0x05]); // CMP %5,A
    s.set_a(0x05);
    s.insn();
    assert_eq!(s.a(), 0x05);
    assert_eq!(s.flags(), (true, false, true));
}

#[test]
fn mpy_writes_a_b_and_flags_the_high_byte() {
    let mut s = Sys::code(&[0x6C]); // MPY B,A
    s.set_a(0x20);
    s.set_b(0x10);
    assert_eq!(s.insn(), 5 + MPY_EXTRA);
    assert_eq!((s.a(), s.b()), (0x02, 0x00));
    assert_eq!(s.flags(), (false, false, false), "high byte 0x02");

    let mut s = Sys::code(&[0x7C, 0x02, 0x20]); // MPY %2,R32
    s.cpu.set_rf(0x20, 0x00);
    assert_eq!(s.insn(), 9 + MPY_EXTRA);
    assert_eq!((s.a(), s.b()), (0, 0));
    assert_eq!(s.flags(), (false, false, true));
    assert_eq!(
        s.cpu.rf(0x20),
        0,
        "MPY never writes its nominal destination"
    );
}

#[test]
fn logic_ops_clear_carry_and_track_negative() {
    let mut s = Sys::code(&[0x23, 0x80]); // AND %>80,A
    s.set_a(0xFF);
    s.set_c(true);
    s.insn();
    assert_eq!(s.a(), 0x80);
    assert_eq!(s.flags(), (false, true, false));
}

#[test]
fn register_operands_past_the_file_read_zero_and_drop_writes() {
    let mut s = Sys::code(&[0x12, 0x90]); // MOV R144,A
    s.set_a(0x55);
    s.insn();
    assert_eq!(s.a(), 0, "unmapped register reads 0");

    let mut s = Sys::code(&[0x72, 0x77, 0x90]); // MOV %>77,R144
    s.insn();
    assert_eq!(s.cpu.rf(0x90), 0, "unmapped register write dropped");
}
