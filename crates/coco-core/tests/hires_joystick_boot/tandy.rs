//! Boots a small hand-assembled 6809 program on a CoCo 3 that drives the
//! Tandy hi-res joystick trigger exactly like real software would: ramp the
//! DAC high, select an axis, drop the DAC low (arming the one-shot), then
//! poll PIA0 PA7 in a tight loop until it reads high, counting iterations.
//! Cross-checks the measured elapsed-cycle count against
//! `hires_joystick::duration_cycles` end to end through the bus/run-loop
//! wiring, not just the state machine in isolation (`hires_joystick_test.rs`).

use coco_core::hires_joystick::HiResInterface;
use coco_core::joystick::{AXIS_X, POT_MAX, RIGHT};

use super::common::{
    FAST_POKE, TOLERANCE_CYCLES, assemble_rom, assert_tracks_duration, done_addr, hires_machine,
    loop_addr, run_program,
};

/// Number of setup instructions before the polling loop's first iteration —
/// see [`setup_bytes`]. Used only to skip straight past setup with
/// `step_instruction`; [`run_for_pot`] adds `extra_setup`'s own instruction
/// count on top.
const SETUP_INSTRUCTIONS: usize = 14;
/// Byte length of the fixed setup block below (through the DAC = 0 write).
const SETUP_LEN: u16 = 37;

/// Hand-assembled 6809 setup block, placed at `$8000`, arming the Tandy trigger by dropping
/// the DAC to 0:
/// ```text
/// $8000  10 CE 5E FF   LDS  #$5EFF
/// $8004  86 34         LDA  #$34
/// $8006  B7 FF 01      STA  $FF01        ; PIA0 CRA: data reg, CA2 (axis) low
/// $8009  B7 FF 03      STA  $FF03        ; PIA0 CRB: data reg, CB2 (stick) low
/// $800C  86 00         LDA  #$00
/// $800E  B7 FF 21      STA  $FF21        ; PIA1 CRA: DDR access
/// $8011  86 FE         LDA  #$FE
/// $8013  B7 FF 20      STA  $FF20        ; PIA1 DDRA = $FE
/// $8016  86 34         LDA  #$34
/// $8018  B7 FF 21      STA  $FF21        ; PIA1 CRA: data reg
/// $801B  86 FC         LDA  #$FC
/// $801D  B7 FF 20      STA  $FF20        ; DAC = 63 (high)
/// $8020  86 00         LDA  #$00
/// $8022  B7 FF 20      STA  $FF20        ; DAC = 0 (low: arms the one-shot)
/// ```
/// followed by `extra_setup` (for example [`FAST_POKE`]), then the shared polling loop
/// (`common::loop_and_done_bytes`).
fn setup_bytes() -> Vec<u8> {
    vec![
        0x10, 0xCE, 0x5E, 0xFF, // LDS #$5EFF
        0x86, 0x34, // LDA #$34
        0xB7, 0xFF, 0x01, // STA $FF01 (PIA0 CRA)
        0xB7, 0xFF, 0x03, // STA $FF03 (PIA0 CRB)
        0x86, 0x00, // LDA #$00
        0xB7, 0xFF, 0x21, // STA $FF21 (PIA1 CRA -> DDR)
        0x86, 0xFE, // LDA #$FE
        0xB7, 0xFF, 0x20, // STA $FF20 (PIA1 DDRA)
        0x86, 0x34, // LDA #$34
        0xB7, 0xFF, 0x21, // STA $FF21 (PIA1 CRA -> data)
        0x86, 0xFC, // LDA #$FC
        0xB7, 0xFF, 0x20, // STA $FF20 (DAC = 63)
        0x86, 0x00, // LDA #$00
        0xB7, 0xFF, 0x20, // STA $FF20 (DAC = 0, arms)
    ]
}

/// Runs the program to completion for `pot`, splicing `extra_setup`
/// (`extra_instructions` worth of instructions) in before the polling loop.
/// Returns the loop's iteration count (from RAM) and the CPU cycles elapsed
/// from the DAC-low write (or `extra_setup`'s last instruction, if any) to
/// the moment PA7 first reads high.
fn run_for_pot(pot: u16, extra_setup: &[u8], extra_instructions: usize) -> (u16, u32) {
    let setup = setup_bytes();
    assert_eq!(
        setup.len(),
        SETUP_LEN as usize,
        "SETUP_LEN must track the setup block above"
    );
    let rom = assemble_rom(&setup, extra_setup);
    let mut m = hires_machine(rom);
    m.bus.joysticks.set_hires(RIGHT, HiResInterface::Tandy);
    m.bus.joysticks.set_pot(RIGHT, AXIS_X, pot);

    let loop_addr = loop_addr(SETUP_LEN, extra_setup.len());
    let done_addr = done_addr(SETUP_LEN, extra_setup.len());
    run_program(
        &mut m,
        SETUP_INSTRUCTIONS,
        extra_instructions,
        loop_addr,
        done_addr,
    )
}

#[test]
fn loop_count_is_monotonic_and_tracks_duration_cycles() {
    let (count_min, cycles_min) = run_for_pot(0, &[], 0);
    let (count_mid, cycles_mid) = run_for_pot(POT_MAX / 2, &[], 0);
    let (count_max, cycles_max) = run_for_pot(POT_MAX, &[], 0);

    assert_tracks_duration(
        HiResInterface::Tandy,
        [
            (0, count_min, cycles_min),
            (POT_MAX / 2, count_mid, cycles_mid),
            (POT_MAX, count_max, cycles_max),
        ],
    );
}

/// The RC one-shot is a real-time circuit: `duration_cycles` is expressed in slow-clock
/// terms, unaffected by the double-speed poke. Under it, the CPU retires roughly twice as
/// many raw cycles for the same real-time duration, so a `STA $FFD9` before the timing loop
/// must roughly double the measured raw-cycle count relative to the slow-clock baseline —
/// `Joysticks::tick`'s half-weight fast-clock credit must not let the one-shot saturate in
/// half the cycles instead.
#[test]
fn fast_clock_measures_roughly_double_the_raw_cycles() {
    let pot = POT_MAX / 2;
    let (_, cycles_slow) = run_for_pot(pot, &[], 0);
    let (_, cycles_fast) = run_for_pot(pot, FAST_POKE, 1);

    let expected_fast = cycles_slow.saturating_mul(2);
    let diff = cycles_fast.abs_diff(expected_fast);
    // Double the granularity (each loop iteration now costs ~2x the raw cycles), so double
    // the tolerance too.
    assert!(
        diff <= TOLERANCE_CYCLES * 2,
        "fast-clock cycles {cycles_fast} should be roughly double the slow-clock {cycles_slow} \
         (expected {expected_fast}, diff {diff})"
    );
}
