//! Shared ROM-assembly plumbing for the hi-res joystick boot tests
//! (`tandy.rs`, `cocomax3.rs`): the polling-loop bytes both interfaces'
//! setups share, and the machine/run-loop helpers that measure elapsed
//! cycles against `hires_joystick::duration_cycles`.

use coco_core::hires_joystick::{HiResInterface, duration_cycles};
use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize};
use mc6809::Bus;

pub const ROM_SIZE: usize = 32 * 1024;
pub const BASE_ADDR: u16 = 0x8000;

/// Counter RAM address the loop increments (big-endian 16-bit).
pub const COUNTER_ADDR: u16 = 0x0400;

/// Byte length of the polling loop body (`LDA`/`BMI`/`INC`/`BNE`/`INC`/`BRA`/`done`).
pub const LOOP_LEN: u16 = 15;

/// `STA $FFD9` — SAM/GIME's R1 double-speed strobe, the classic
/// `POKE 65497,0`. Spliced into setup by the fast-clock regression test.
pub const FAST_POKE: &[u8] = &[0xB7, 0xFF, 0xD9];

/// A handful of loop iterations' worth of polling granularity: `tick()` only
/// runs once per CPU instruction (not per cycle), so saturation is only ever
/// observed on the next `LDA $FF00` after it actually happens.
pub const TOLERANCE_CYCLES: u32 = 80;

/// The polling loop body + terminal `done` label, shared by every variant's ROM:
/// ```text
/// loop:
///        B6 FF 00      LDA  $FF00        ; PIA0 PA -> N flag = comparator bit
///        2B 0A         BMI  done
///        7C 04 01      INC  $0401        ; counter low byte
///        26 F6         BNE  loop
///        7C 04 00      INC  $0400        ; counter high byte, on low-byte wrap
///        20 F1         BRA  loop
/// done:
///        20 FE         BRA  done
/// ```
fn loop_and_done_bytes() -> Vec<u8> {
    let [counter_hi, counter_lo] = COUNTER_ADDR.to_be_bytes();
    vec![
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
    ]
}

/// The polling loop's entry point once `setup` (`setup_len` bytes) and
/// `extra_setup` (for example [`FAST_POKE`]) have been placed after `$8000`.
pub fn loop_addr(setup_len: u16, extra_setup_len: usize) -> u16 {
    BASE_ADDR + setup_len + extra_setup_len as u16
}

/// [`loop_addr`]'s terminal (self-looping) `done` label.
pub fn done_addr(setup_len: u16, extra_setup_len: usize) -> u16 {
    loop_addr(setup_len, extra_setup_len) + LOOP_LEN
}

/// Assemble a 32K ROM image at `$8000`: a variant's own `setup` block, then
/// `extra_setup`, then the shared polling loop, then the RESET vector.
pub fn assemble_rom(setup: &[u8], extra_setup: &[u8]) -> Box<[u8]> {
    let mut rom = vec![0u8; ROM_SIZE];
    let mut prog = Vec::with_capacity(setup.len() + extra_setup.len() + LOOP_LEN as usize);
    prog.extend_from_slice(setup);
    prog.extend_from_slice(extra_setup);
    prog.extend_from_slice(&loop_and_done_bytes());
    rom[0x0000..prog.len()].copy_from_slice(&prog);
    rom[0x7FFE..0x8000].copy_from_slice(&[0x80, 0x00]); // RESET vector -> $8000
    rom.into_boxed_slice()
}

pub fn hires_machine(rom: Box<[u8]>) -> Machine {
    let config = MachineConfig {
        variant: MachineVariant::Coco3,
        memory: MemorySize::K128,
        ..MachineConfig::default()
    };
    Machine::new(config, rom)
}

/// Step past `setup_instructions + extra_instructions` instructions, assert landing exactly
/// on `loop_addr`, then run until `done_addr`, returning the loop's RAM counter and the CPU
/// cycles elapsed from the last setup instruction to the moment PA7 first reads high.
pub fn run_program(
    m: &mut Machine,
    setup_instructions: usize,
    extra_instructions: usize,
    loop_addr: u16,
    done_addr: u16,
) -> (u16, u32) {
    for _ in 0..setup_instructions + extra_instructions {
        m.step_instruction();
    }
    assert_eq!(
        m.cpu.pc, loop_addr,
        "setup must land exactly on the polling loop"
    );
    let start_cycles = m.cpu.cycles;

    const STEP_CAP: u32 = 200_000;
    let mut steps = 0;
    while m.cpu.pc != done_addr {
        m.step_instruction();
        steps += 1;
        assert!(steps < STEP_CAP, "PA7 never went high");
    }
    let measured_cycles = u32::try_from(m.cpu.cycles - start_cycles).unwrap();

    let hi = u16::from(m.bus.read(COUNTER_ADDR));
    let lo = u16::from(m.bus.read(COUNTER_ADDR + 1));
    (hi << 8 | lo, measured_cycles)
}

/// `loop_count_is_monotonic_and_tracks_duration_cycles`'s assertion body, shared by every
/// variant: `samples` is `(pot, loop_count, measured_cycles)` for three ascending pots (each
/// from [`run_program`]'s return). Asserts the loop counts strictly increase with `pot`, and
/// that each measured cycle count is within [`TOLERANCE_CYCLES`] of `duration_cycles(kind, pot)`.
pub fn assert_tracks_duration(kind: HiResInterface, samples: [(u16, u16, u32); 3]) {
    let [(_, count_min, _), (_, count_mid, _), (_, count_max, _)] = samples;
    assert!(
        count_min < count_mid && count_mid < count_max,
        "loop count must increase with pot: {count_min} < {count_mid} < {count_max}"
    );
    for (pot, _, measured) in samples {
        let expected = duration_cycles(kind, pot);
        let diff = measured.abs_diff(expected);
        assert!(
            diff <= TOLERANCE_CYCLES,
            "pot {pot}: measured {measured} cycles vs duration_cycles {expected} (diff {diff})"
        );
    }
}
