//! Contrasts the two public single-step primitives directly: `step_instruction`
//! (the full per-scanline pipeline `run_field` is built on) must advance the
//! machine's scanline position and the GIME interval timer as it runs, while
//! `step_cpu_raw` (the raw MC6809-core escape hatch — see its doc comment
//! on [`Machine`]) must advance neither, since it bypasses the per-line
//! trailer entirely.

use coco_core::{Machine, MachineConfig};

const ROM_SIZE: usize = 32 * 1024;
const INIT1_REG: u16 = 0xFF91;
const TIMER_MSB_REG: u16 = 0xFF94;
const TIMER_LSB_REG: u16 = 0xFF95;
/// INIT1 TINS: selects the fast (3.58 MHz-class) timer input clock over the
/// horizontal-sync rate, so the timer visibly counts down within one field.
const INIT1_TINS: u8 = 0x20;
/// Loaded into the 12-bit timer reload (`TIMER_MSB_REG`/`TIMER_LSB_REG`):
/// large enough that the first (50-step) `step_instruction` loop doesn't
/// underflow it, so the timer-decreased assertion right after is unambiguous.
/// The longer field-completion loop below does underflow (and reload) it
/// several times, which is fine — nothing asserts on the timer value there.
const TIMER_RELOAD: u16 = 0x0FFF;

/// Synthetic ROM: reset vector -> `BRA *` (a 3-cycle no-op loop), so all
/// observed timing effects come from the run-loop plumbing itself, not from
/// program behavior.
fn test_rom() -> Box<[u8]> {
    let mut rom = vec![0u8; ROM_SIZE];
    rom[0x0000..0x0002].copy_from_slice(&[0x20, 0xFE]); // BRA *
    rom[ROM_SIZE - 2..ROM_SIZE].copy_from_slice(&[0x80, 0x00]); // reset vector -> $8000
    rom.into_boxed_slice()
}

fn boot_machine() -> Machine {
    let mut m = Machine::new(MachineConfig::default(), test_rom());
    m.poke(INIT1_REG, INIT1_TINS);
    m.poke(TIMER_MSB_REG, (TIMER_RELOAD >> 8) as u8);
    m.poke(TIMER_LSB_REG, (TIMER_RELOAD & 0xFF) as u8);
    m
}

#[test]
fn step_cpu_raw_advances_neither_scanline_nor_gime_timer() {
    let mut m = boot_machine();
    let timer_before = m.bus.gime.timer_count;

    for _ in 0..500 {
        m.step_cpu_raw();
    }

    assert_eq!(
        m.current_scanline(),
        0,
        "step_cpu_raw must never run the per-line trailer that advances the scanline"
    );
    assert_eq!(
        m.bus.gime.timer_count, timer_before,
        "step_cpu_raw must never tick the GIME interval timer"
    );
}

#[test]
fn step_instruction_advances_scanline_and_gime_timer_and_completes_a_field() {
    let mut m = boot_machine();
    let timer_before = m.bus.gime.timer_count;
    let lines_per_field = m.config.video.lines_per_field();

    // A couple of lines' worth of `BRA *` iterations is enough to cross at
    // least one line boundary and tick the timer, without waiting for a
    // whole field.
    for _ in 0..50 {
        m.step_instruction();
    }
    assert!(
        m.current_scanline() > 0,
        "step_instruction must run the per-line trailer and advance the scanline"
    );
    assert!(
        m.bus.gime.timer_count < timer_before,
        "step_instruction must tick the GIME interval timer"
    );

    // Enough remaining instructions to walk the rest of the field and wrap
    // the scanline back to 0, proving the field boundary (render + wrap) is
    // reached under normal single-stepping too.
    let max_remaining_steps = lines_per_field * 200;
    let mut field_completed = false;
    for _ in 0..max_remaining_steps {
        if m.step_instruction().field_complete {
            field_completed = true;
            break;
        }
    }
    assert!(
        field_completed,
        "step_instruction never completed a field within {max_remaining_steps} steps"
    );
    assert_eq!(
        m.current_scanline(),
        0,
        "field completion must wrap the scanline back to 0"
    );
}
