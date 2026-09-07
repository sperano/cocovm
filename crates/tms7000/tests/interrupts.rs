//! Reset sequence, interrupt entry, priority, and the level semantics of
//! the external lines (MAME `device_reset`, `execute_set_input`,
//! `check_interrupts`, `do_interrupt`).

mod common;

use common::{CODE, Sys};
use tms7000::{FlatBoard, Port, ROM_SIZE, StepKind, TMS7040, st};

const IOCNT0: u8 = 0x00;
const INT1_ENABLE: u8 = 0x01;
const INT3_ENABLE: u8 = 0x10;
const INT1_FLAG: u8 = 0x02;
const INT3_FLAG: u8 = 0x20;

/// A ROM whose three interrupt vectors point at RETI stubs at `$F800`,
/// `$F900`, `$FA00` and whose reset code is `code`.
fn rom_with_handlers(code: &[u8]) -> Vec<u8> {
    let mut rom = Sys::rom(code);
    for (vector, handler) in [(0xFFC, 0xF800u16), (0xFFA, 0xF900), (0xFF8, 0xFA00)] {
        rom[vector..vector + 2].copy_from_slice(&handler.to_be_bytes());
        rom[usize::from(handler - 0xF000)] = 0x0B; // RETI
    }
    rom
}

#[test]
fn reset_sequence_matches_mame_order_and_state() {
    let mut rom = vec![0; ROM_SIZE];
    rom[ROM_SIZE - 2..].copy_from_slice(&0xF123u16.to_be_bytes());
    let mut cpu = TMS7040::new(&rom).unwrap();
    let mut board = FlatBoard::new();
    cpu.pc = 0xABCD;
    cpu.st = 0xFF;
    let step = cpu.step(&mut board);
    assert_eq!(
        step,
        tms7000::Step {
            cycles: 17,
            kind: StepKind::Reset
        }
    );
    // Port B all ones (DDR B fixed), ports C/D driven with DDR cleared.
    assert_eq!(
        board.writes,
        vec![(Port::B, 0xFF), (Port::C, 0x00), (Port::D, 0x00)]
    );
    assert_eq!(cpu.port_latch(Port::C), 0xFF);
    assert_eq!(cpu.port_ddr(Port::C), 0);
    assert_eq!(cpu.port_ddr(Port::D), 0);
    assert_eq!(cpu.st, 0);
    assert_eq!(cpu.io_control() & 0x15, 0, "enables cleared");
    assert_eq!(cpu.pc, 0xF123);
    assert_eq!(cpu.sp, 0x01, "TRAP 0 pushed the old PC into R0/R1");
    assert_eq!((cpu.a(), cpu.b()), (0xAB, 0xCD));
}

#[test]
fn reset_keeps_pending_flags_but_drops_enables() {
    let mut s = Sys::code(&[0x00]);
    s.cpu.set_int3(true);
    s.cpu.assert_reset();
    assert_eq!(s.step().kind, StepKind::Reset);
    assert_eq!(s.cpu.io_control() & INT3_FLAG, INT3_FLAG);
    assert_eq!(s.cpu.io_control() & INT3_ENABLE, 0);
}

/// Reset does not change the INTn flag bits (SPND001B 3.6.1): a pulse
/// already latched and gone still shows in IOCNT0 after reset, not just a
/// still-high level.
#[test]
fn reset_preserves_a_latched_pulse() {
    let mut s = Sys::code(&[0x00]);
    s.cpu.set_int1(true);
    s.cpu.set_int1(false);
    assert_eq!(s.cpu.io_control() & INT1_FLAG, INT1_FLAG);
    s.cpu.assert_reset();
    assert_eq!(s.step().kind, StepKind::Reset);
    assert_eq!(s.cpu.io_control() & INT1_FLAG, INT1_FLAG);
}

#[test]
fn reset_from_idle_steps_past_the_idle() {
    let mut s = Sys::code(&[0x01, 0x00]);
    s.insn();
    assert!(s.cpu.is_idle());
    s.cpu.assert_reset();
    let before = s.cpu.cycles;
    assert_eq!(s.step().kind, StepKind::Reset);
    assert!(!s.cpu.is_idle());
    assert_eq!(
        s.cpu.cycles - before,
        17,
        "a host reset costs its 17 cycles"
    );
    assert_eq!(s.cpu.pc, CODE);
}

#[test]
fn int3_entry_costs_19_pushes_st_then_pc_and_clears_st() {
    let rom = rom_with_handlers(&[0xA2, INT3_ENABLE, IOCNT0, 0x00, 0x00]);
    let mut s = Sys::from_rom(&rom);
    s.cpu.sp = 0x40;
    s.insn(); // enable INT3 (MOVP clears C/N/Z)
    s.cpu.st = st::I | st::C;
    s.cpu.set_int3(true);
    let step = s.step();
    assert_eq!(
        step,
        tms7000::Step {
            cycles: 19,
            kind: StepKind::Interrupt(3)
        }
    );
    assert_eq!(s.cpu.pc, 0xFA00);
    assert_eq!(s.cpu.st, 0);
    assert_eq!(s.cpu.sp, 0x43);
    assert_eq!(s.cpu.rf(0x41), st::I | st::C, "ST first");
    assert_eq!((s.cpu.rf(0x42), s.cpu.rf(0x43)), (0xF0, 0x03), "then PC");
    assert_eq!(
        s.cpu.io_control() & INT3_FLAG,
        INT3_FLAG,
        "line still high: the acked flag springs back"
    );
    s.cpu.set_int3(false);
    assert_eq!(s.cpu.io_control() & INT3_FLAG, 0);
    assert_eq!(s.insn(), 9, "RETI");
    assert_eq!(s.cpu.pc, CODE + 3);
    assert_eq!(s.cpu.st, st::I | st::C);
}

#[test]
fn a_still_high_line_re_enters_the_handler_after_reti() {
    let rom = rom_with_handlers(&[0xA2, INT3_ENABLE, IOCNT0, 0x00, 0x00]);
    let mut s = Sys::from_rom(&rom);
    s.cpu.sp = 0x40;
    s.cpu.st = st::I;
    s.insn();
    s.cpu.set_int3(true);
    assert_eq!(s.step().kind, StepKind::Interrupt(3));
    s.insn(); // RETI restores I
    assert_eq!(s.step().kind, StepKind::Interrupt(3));
}

#[test]
fn global_disable_holds_the_flag_until_eint() {
    let rom = rom_with_handlers(&[0xA2, INT1_ENABLE, IOCNT0, 0x00, 0x05, 0x00]);
    let mut s = Sys::from_rom(&rom);
    s.cpu.sp = 0x40;
    s.cpu.st = 0;
    s.insn();
    s.cpu.set_int1(true);
    assert_eq!(s.cpu.io_control() & INT1_FLAG, INT1_FLAG);
    assert_eq!(s.step().kind, StepKind::Instruction, "NOP runs, I clear");
    assert_eq!(s.step().kind, StepKind::Instruction, "EINT");
    assert_eq!(s.step().kind, StepKind::Interrupt(1));
    assert_eq!(s.cpu.pc, 0xF800);
}

#[test]
fn int1_wins_over_int2_wins_over_int3() {
    let rom = rom_with_handlers(&[0xA2, 0x15, IOCNT0, 0x00]);
    let mut s = Sys::from_rom(&rom);
    s.cpu.sp = 0x40;
    s.cpu.st = st::I;
    s.insn();
    s.cpu.set_int3(true);
    s.cpu.set_int1(true);
    assert_eq!(s.step().kind, StepKind::Interrupt(1));
    s.cpu.set_int1(false);
    s.insn(); // RETI
    assert_eq!(s.step().kind, StepKind::Interrupt(3));
}

#[test]
fn interrupt_from_idle_costs_17_and_resumes_after_it() {
    let rom = rom_with_handlers(&[0xA2, INT3_ENABLE, IOCNT0, 0x01, 0x00]);
    let mut s = Sys::from_rom(&rom);
    s.cpu.sp = 0x40;
    s.cpu.st = st::I;
    s.insn();
    s.insn(); // IDLE
    assert!(s.cpu.is_idle());
    s.cpu.set_int3(true);
    let step = s.step();
    assert_eq!(
        step,
        tms7000::Step {
            cycles: 17,
            kind: StepKind::Interrupt(3)
        }
    );
    assert!(!s.cpu.is_idle());
    assert_eq!(
        (s.cpu.rf(0x42), s.cpu.rf(0x43)),
        (0xF0, 0x04),
        "return past the IDLE"
    );
}

/// SPND001B 3-31/3-33: an inactive-to-active transition sets the line's
/// Pulse flip-flop, which stays set after the line goes inactive again —
/// unlike a plain level mirror, a pulse gone before the CPU ever looks is
/// still pending once the interrupt is enabled.
#[test]
fn pulse_retention_vectors_once_enabled() {
    let rom = rom_with_handlers(&[0xA2, INT1_ENABLE, IOCNT0, 0x05]);
    let mut s = Sys::from_rom(&rom);
    s.cpu.sp = 0x40;
    s.cpu.set_int1(true);
    s.cpu.set_int1(false); // pulse over before INT1 is even enabled
    s.insn(); // MOVP enables INT1
    s.insn(); // EINT
    assert_eq!(s.step().kind, StepKind::Interrupt(1));
    assert_eq!(s.cpu.pc, 0xF800);
}

/// SPND001B 3-31: writing a 1 to the INTn clear bit clears the Pulse
/// flip-flop; with the line already back down, nothing keeps the flag set.
#[test]
fn write_1_to_clear_removes_a_deasserted_pulse() {
    let mut s = Sys::code(&[0xA2, INT1_FLAG, IOCNT0]);
    s.cpu.set_int1(true);
    s.cpu.set_int1(false);
    assert_eq!(s.cpu.io_control() & INT1_FLAG, INT1_FLAG);
    s.insn(); // MOVP writes a 1 to the INT1 clear bit
    assert_eq!(s.cpu.io_control() & INT1_FLAG, 0);
}

/// SPND001B 3-31: "This allows external interrupt pins to be polled as
/// inputs" — even with the interrupt never enabled, a pulse the pin has
/// already dropped stays visible in IOCNT0 for software to test.
#[test]
fn polling_a_short_pulse_via_iocnt0_read() {
    let mut s = Sys::code(&[0x00]);
    s.cpu.set_int1(true);
    s.cpu.set_int1(false);
    assert_eq!(s.cpu.io_control() & INT1_FLAG, INT1_FLAG);
}

/// SPND001B 3-33: CPU acknowledgment clears only the Pulse flip-flop; the
/// flag bit is the OR of that latch and the live level, so it springs back
/// at once if the pin is still asserted, and only actually clears once the
/// pin drops too.
#[test]
fn ack_clears_the_latch_while_a_still_high_level_re_flags() {
    let rom = rom_with_handlers(&[0xA2, INT1_ENABLE, IOCNT0, 0x05]);
    let mut s = Sys::from_rom(&rom);
    s.cpu.sp = 0x40;
    s.cpu.set_int1(true);
    s.insn(); // MOVP enables INT1
    s.insn(); // EINT
    assert_eq!(s.step().kind, StepKind::Interrupt(1));
    assert_eq!(
        s.cpu.io_control() & INT1_FLAG,
        INT1_FLAG,
        "latch cleared by ack, but the still-high line re-flags it"
    );
    s.cpu.set_int1(false);
    assert_eq!(
        s.cpu.io_control() & INT1_FLAG,
        0,
        "latch already clear, and now the line is too"
    );
}
