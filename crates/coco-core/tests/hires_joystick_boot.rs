//! Boots a small hand-assembled 6809 program on a CoCo 3 that drives the
//! Tandy hi-res joystick trigger exactly like real software would: ramp the
//! DAC high, select an axis, drop the DAC low (arming the one-shot), then
//! poll PIA0 PA7 in a tight loop until it reads high, counting iterations.
//! Cross-checks the measured elapsed-cycle count against
//! `hires_joystick::duration_cycles` end to end through the bus/run-loop
//! wiring, not just the state machine in isolation (`hires_joystick_test.rs`).

use coco_core::hires_joystick::{HiResInterface, duration_cycles};
use coco_core::joystick::{AXIS_X, POT_MAX, RIGHT};
use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize};
use mc6809::Bus;

const ROM_SIZE: usize = 32 * 1024;
const BASE_ADDR: u16 = 0x8000;

/// Counter RAM address the loop increments (big-endian 16-bit).
const COUNTER_ADDR: u16 = 0x0400;

/// Number of setup instructions before the polling loop's first iteration —
/// see the assembly listing below. Used only to skip straight past setup with
/// `step_instruction`; [`run_for_pot`] adds `extra_setup`'s own instruction
/// count on top.
const SETUP_INSTRUCTIONS: usize = 14;
/// Byte length of the fixed setup block below (through the DAC = 0 write).
const SETUP_LEN: u16 = 37;
/// Byte length of the polling loop body (`LDA`/`BMI`/`INC`/`BNE`/`INC`/`BRA`).
const LOOP_LEN: u16 = 15;

/// `STA $FFD9` — SAM/GIME's R1 double-speed strobe, the classic
/// `POKE 65497,0`. Spliced into setup by the fast-clock regression test.
const FAST_POKE: &[u8] = &[0xB7, 0xFF, 0xD9];

/// The polling loop's entry point once `extra_setup` (`extra_setup.len()`
/// bytes) has been spliced in after the fixed setup block.
fn loop_addr(extra_setup_len: usize) -> u16 {
    BASE_ADDR + SETUP_LEN + extra_setup_len as u16
}

/// [`loop_addr`]'s terminal (self-looping) `done` label.
fn done_addr(extra_setup_len: usize) -> u16 {
    loop_addr(extra_setup_len) + LOOP_LEN
}

/// Hand-assembled 6809 program, 32K, mapped to `$8000-$FFFF`. `extra_setup`
/// (raw instruction bytes, for example [`FAST_POKE`]) is spliced in after the
/// DAC arms low and before the polling loop, shifting `loop`/`done` by its
/// length ([`loop_addr`]/[`done_addr`]):
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
/// [extra_setup]
/// loop:
///        B6 FF 00      LDA  $FF00        ; PIA0 PA -> N flag = comparator bit
///        2B 0A         BMI  done
///        7C 04 01      INC  $0401        ; counter low byte
///        26 F6         BNE  loop
///        7C 04 00      INC  $0400        ; counter high byte, on low-byte wrap
///        20 F1         BRA  loop
/// done:
///        20 FE         BRA  done
/// $FFFE  80 00         (RESET vector -> $8000)
/// ```
fn hires_rom(extra_setup: &[u8]) -> Box<[u8]> {
    let mut rom = vec![0u8; ROM_SIZE];
    let [counter_hi, counter_lo] = COUNTER_ADDR.to_be_bytes();
    let mut prog: Vec<u8> = Vec::with_capacity(64);
    prog.extend_from_slice(&[
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
    ]);
    assert_eq!(
        prog.len(),
        SETUP_LEN as usize,
        "SETUP_LEN must track the setup block above"
    );
    prog.extend_from_slice(extra_setup);
    prog.extend_from_slice(&[
        // loop:
        0xB6,
        0xFF,
        0x00, // LDA $FF00
        0x2B,
        0x0A, // BMI done (+10)
        0x7C,
        counter_hi,
        counter_lo + 1, // INC $0401 (counter low byte)
        0x26,
        0xF6, // BNE loop (-10)
        0x7C,
        counter_hi,
        counter_lo, // INC $0400 (counter high byte, on wrap)
        0x20,
        0xF1, // BRA loop (-15)
        // done:
        0x20,
        0xFE, // BRA done (-2)
    ]);
    rom[0x0000..prog.len()].copy_from_slice(&prog);
    rom[0x7FFE..0x8000].copy_from_slice(&[0x80, 0x00]); // RESET vector -> $8000
    rom.into_boxed_slice()
}

fn hires_machine(extra_setup: &[u8]) -> Machine {
    let config = MachineConfig {
        variant: MachineVariant::Coco3,
        memory: MemorySize::K128,
        ..MachineConfig::default()
    };
    Machine::new(config, hires_rom(extra_setup))
}

/// Runs the program to completion for `pot`, splicing `extra_setup`
/// (`extra_instructions` worth of instructions) in before the polling loop.
/// Returns the loop's iteration count (from RAM) and the CPU cycles elapsed
/// from the DAC-low write (or `extra_setup`'s last instruction, if any) to
/// the moment PA7 first reads high.
fn run_for_pot(pot: u16, extra_setup: &[u8], extra_instructions: usize) -> (u16, u32) {
    let mut m = hires_machine(extra_setup);
    m.bus.joysticks.set_hires(RIGHT, HiResInterface::Tandy);
    m.bus.joysticks.set_pot(RIGHT, AXIS_X, pot);

    for _ in 0..SETUP_INSTRUCTIONS + extra_instructions {
        m.step_instruction();
    }
    let loop_addr = loop_addr(extra_setup.len());
    assert_eq!(
        m.cpu.pc, loop_addr,
        "setup must land exactly on the polling loop"
    );
    let start_cycles = m.cpu.cycles;

    let done_addr = done_addr(extra_setup.len());
    const STEP_CAP: u32 = 200_000;
    let mut steps = 0;
    while m.cpu.pc != done_addr {
        m.step_instruction();
        steps += 1;
        assert!(steps < STEP_CAP, "PA7 never went high for pot {pot}");
    }
    let measured_cycles = u32::try_from(m.cpu.cycles - start_cycles).unwrap();

    let hi = u16::from(m.bus.read(COUNTER_ADDR));
    let lo = u16::from(m.bus.read(COUNTER_ADDR + 1));
    (hi << 8 | lo, measured_cycles)
}

/// A handful of loop iterations' worth of polling granularity: `tick()` only
/// runs once per CPU instruction (not per cycle), so saturation is only ever
/// observed on the next `LDA $FF00` after it actually happens.
const TOLERANCE_CYCLES: u32 = 80;

#[test]
fn loop_count_is_monotonic_and_tracks_duration_cycles() {
    let (count_min, cycles_min) = run_for_pot(0, &[], 0);
    let (count_mid, cycles_mid) = run_for_pot(POT_MAX / 2, &[], 0);
    let (count_max, cycles_max) = run_for_pot(POT_MAX, &[], 0);

    assert!(
        count_min < count_mid && count_mid < count_max,
        "loop count must increase with pot: {count_min} < {count_mid} < {count_max}"
    );

    for (pot, measured) in [
        (0, cycles_min),
        (POT_MAX / 2, cycles_mid),
        (POT_MAX, cycles_max),
    ] {
        let expected = duration_cycles(HiResInterface::Tandy, pot);
        let diff = measured.abs_diff(expected);
        assert!(
            diff <= TOLERANCE_CYCLES,
            "pot {pot}: measured {measured} cycles vs duration_cycles {expected} (diff {diff})"
        );
    }
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
