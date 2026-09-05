//! PUSH/POP, LDSP/STSP, EINT/DINT/SETC, NOP, IDLE, and the illegal-opcode
//! policy.

mod common;

use common::{CODE, Sys};
use tms7000::{StepKind, st};

#[test]
fn push_and_pop_a_b_and_rn_are_lifo() {
    let mut s = Sys::code(&[0xB8, 0xC8, 0xD8, 0x10, 0xD9, 0x20, 0xC9, 0xB9]);
    s.cpu.sp = 0x40;
    s.set_a(0x11);
    s.set_b(0x22);
    s.cpu.set_rf(0x10, 0x33);

    assert_eq!(s.insn(), 6, "PUSH A");
    assert_eq!(s.insn(), 6, "PUSH B");
    assert_eq!(s.insn(), 8, "PUSH R16");
    assert_eq!(s.cpu.sp, 0x43);
    assert_eq!(
        (s.cpu.rf(0x41), s.cpu.rf(0x42), s.cpu.rf(0x43)),
        (0x11, 0x22, 0x33)
    );

    assert_eq!(s.insn(), 8, "POP R32");
    assert_eq!(s.cpu.rf(0x20), 0x33);
    assert_eq!(s.insn(), 6, "POP B");
    assert_eq!(s.b(), 0x22);
    assert_eq!(s.insn(), 6, "POP A");
    assert_eq!(s.a(), 0x11);
    assert_eq!(s.cpu.sp, 0x40);
}

#[test]
fn push_st_and_pop_st_round_trip_the_status_register() {
    let mut s = Sys::code(&[0x0E, 0x08]);
    s.cpu.sp = 0x40;
    s.cpu.st = st::C | st::I;
    assert_eq!(s.insn(), 6, "PUSH ST");
    assert_eq!(s.cpu.rf(0x41), st::C | st::I);
    s.cpu.st = 0;
    assert_eq!(s.insn(), 6, "POP ST");
    assert_eq!(s.cpu.st, st::C | st::I);
    assert_eq!(s.cpu.sp, 0x40);
}

#[test]
fn push_flags_the_pushed_value_and_pop_st_drops_the_low_nibble() {
    let mut s = Sys::code(&[0xB8]);
    s.set_a(0x80);
    s.insn();
    assert_eq!(s.flags(), (false, true, false));

    let mut s = Sys::code(&[0x08]); // POP ST
    s.cpu.sp = 0x41;
    s.cpu.set_rf(0x41, 0xFF);
    s.insn();
    assert_eq!(s.cpu.st, 0xF0);
}

#[test]
fn ldsp_and_stsp_move_between_b_and_sp() {
    let mut s = Sys::code(&[0x0D, 0x09]);
    s.set_b(0x55);
    assert_eq!(s.insn(), 5, "LDSP");
    assert_eq!(s.cpu.sp, 0x55);
    s.set_b(0);
    assert_eq!(s.insn(), 6, "STSP");
    assert_eq!(s.b(), 0x55);
}

#[test]
fn eint_and_dint_write_the_whole_high_nibble() {
    let mut s = Sys::code(&[0x05, 0x06]);
    s.cpu.st = 0;
    assert_eq!(s.insn(), 5, "EINT");
    assert_eq!(s.cpu.st, st::N | st::Z | st::C | st::I);
    assert_eq!(s.insn(), 5, "DINT");
    assert_eq!(s.cpu.st, 0);
}

#[test]
fn setc_sets_c_and_z_and_clears_n() {
    let mut s = Sys::code(&[0x07]);
    s.cpu.st = st::N | st::I;
    assert_eq!(s.insn(), 5);
    assert_eq!(s.cpu.st, st::C | st::Z | st::I);
}

#[test]
fn nop_costs_five() {
    let mut s = Sys::code(&[0x00]);
    assert_eq!(s.insn(), 5);
    assert_eq!(s.cpu.pc, CODE + 1);
}

#[test]
fn idle_re_executes_itself_until_an_interrupt() {
    let mut s = Sys::code(&[0x01, 0x00]);
    assert_eq!(s.insn(), 6);
    assert_eq!(s.cpu.pc, CODE, "PC backed onto the IDLE");
    assert!(s.cpu.is_idle());
    assert_eq!(s.insn(), 6);
    assert_eq!(s.cpu.pc, CODE);
}

#[test]
fn illegal_opcodes_cost_five_and_are_counted() {
    for op in [0x02u8, 0x0C, 0x10, 0x81, 0x90, 0xA0, 0xAF] {
        let mut s = Sys::code(&[op]);
        s.set_a(0x5A);
        assert_eq!(s.insn(), 5, "{op:#04x}");
        assert_eq!(s.cpu.pc, CODE + 1);
        assert_eq!(s.a(), 0x5A, "no state change");
        assert_eq!(s.cpu.illegal_count, 1);
    }
}

#[test]
fn step_reports_instruction_kind() {
    let mut s = Sys::code(&[0x00]);
    assert_eq!(s.step().kind, StepKind::Instruction);
}
