//! Timer 1: `(T1DATA + 1) * 8 * (prescaler + 1)` CPU cycles per INT2, restart
//! on a control write, stop bit, capture latch on INT3, live decrementer read.

mod common;

use common::Sys;
use tms7000::{StepKind, VECTOR_INT2, st};

const T1DATA: u8 = 0x02;
const T1CTL: u8 = 0x03;
const IOCNT0: u8 = 0x00;
const INT2_FLAG: u8 = 0x08;
const INT2_ENABLE: u8 = 0x04;
const START_INTERNAL: u8 = 0x80;

fn program(data: u8, control: u8) -> Vec<u8> {
    let mut code = vec![0xA2, data, T1DATA, 0xA2, control, T1CTL];
    code.extend(std::iter::repeat_n(0x00, 64)); // NOPs to run against
    code
}

/// Cycles from the end of the T1CTL write until the INT2 flag first shows.
fn cycles_to_int2(s: &mut Sys) -> u64 {
    s.cpu.st = 0; // count the flag, never dispatch
    s.insn();
    s.insn();
    let start = s.cpu.cycles;
    while s.cpu.io_control() & INT2_FLAG == 0 {
        s.insn();
        assert!(s.cpu.cycles - start < 100_000, "INT2 never flagged");
    }
    s.cpu.cycles - start
}

#[test]
fn period_is_data_plus_one_times_eight_times_prescaler_plus_one() {
    for (data, prescaler) in [(0u8, 0u8), (3, 0), (0, 3), (9, 4)] {
        let mut s = Sys::code(&program(data, START_INTERNAL | prescaler));
        let cycles = cycles_to_int2(&mut s);
        let expected = (u64::from(data) + 1) * 8 * (u64::from(prescaler) + 1);
        // NOPs are 5 cycles, so the flag shows within one instruction of the boundary.
        assert!(
            cycles >= expected && cycles < expected + 5,
            "data {data} prescaler {prescaler}: {cycles} cycles, expected {expected}"
        );
    }
}

#[test]
fn stopped_timer_never_flags() {
    let mut s = Sys::code(&program(0, 0x00));
    s.cpu.st = 0;
    s.insn();
    s.insn();
    for _ in 0..100 {
        s.insn();
    }
    assert_eq!(s.cpu.io_control() & INT2_FLAG, 0);
}

#[test]
fn external_source_is_not_counted() {
    let mut s = Sys::code(&program(0, START_INTERNAL | 0x40));
    s.cpu.st = 0;
    s.insn();
    s.insn();
    for _ in 0..100 {
        s.insn();
    }
    assert_eq!(s.cpu.io_control() & INT2_FLAG, 0);
}

#[test]
fn t1data_reads_the_live_decrementer_and_t1ctl_the_capture_latch() {
    let mut code = program(5, START_INTERNAL);
    code.truncate(6);
    code.extend([0x00, 0x00, 0x00, 0x00]); // 20 cycles: 2 decrements
    code.extend([0x80, T1DATA]); // MOVP P2,A
    code.extend([0x80, T1CTL]); // MOVP P3,A
    let mut s = Sys::code(&code);
    s.cpu.st = 0;
    for _ in 0..6 {
        s.insn();
    }
    s.insn();
    assert_eq!(s.a(), 3, "5 reloaded, then two 8-cycle decrements");
    s.cpu.set_int3(true);
    s.insn();
    assert_eq!(s.a(), 2, "the decrementer at the INT3 rising edge");
}

#[test]
fn control_write_restarts_from_the_reload_value() {
    let mut code = program(4, START_INTERNAL);
    code.truncate(6);
    code.extend([0x00, 0x00, 0x00]); // 15 cycles: one decrement (4 -> 3)
    code.extend([0xA2, START_INTERNAL, T1CTL]); // rewrite control
    code.extend([0x80, T1DATA]);
    let mut s = Sys::code(&code);
    s.cpu.st = 0;
    for _ in 0..6 {
        s.insn();
    }
    s.insn();
    assert_eq!(
        s.a(),
        4,
        "back to the reload value after only 9 more cycles"
    );
}

#[test]
fn int2_dispatches_through_its_vector_when_enabled() {
    let mut rom = Sys::rom(&{
        let mut code = vec![0xA2, INT2_ENABLE, IOCNT0]; // enable INT2
        code.extend(program(0, START_INTERNAL));
        code
    });
    rom[0xFFA] = 0xF8;
    rom[0xFFB] = 0x00;
    let mut s = Sys::from_rom(&rom);
    s.cpu.st = st::I;
    s.cpu.sp = 0x40;
    let mut kinds = Vec::new();
    while kinds.last() != Some(&StepKind::Interrupt(2)) {
        kinds.push(s.step().kind);
        assert!(kinds.len() < 16, "{kinds:?}");
    }
    assert_eq!(s.cpu.pc, 0xF800);
    assert_eq!(s.cpu.peek(VECTOR_INT2), 0xF8);
}
