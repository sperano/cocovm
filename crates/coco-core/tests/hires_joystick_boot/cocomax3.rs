//! Boots a small hand-assembled 6809 program on a CoCo 3 that drives the
//! CoCo Max III hi-res joystick trigger exactly like real software would:
//! ramp the DAC to a nonzero, non-saturating value and leave it there
//! (proving the DAC is NOT this interface's trigger), select an axis, then drive the PIA0
//! port-A PA0-3 nibble to 0 (arming the one-shot) via a DDR write followed
//! by a data-register write, then poll PIA0 PA7 in a tight loop until it
//! reads high, counting iterations. Cross-checks the measured elapsed-cycle
//! count against `hires_joystick::duration_cycles` end to end through the
//! bus/run-loop wiring, mirroring `tandy.rs`.

use coco_core::hires_joystick::HiResInterface;
use coco_core::joystick::{AXIS_X, POT_MAX, RIGHT};

use super::common::{
    assemble_rom, assert_tracks_duration, done_addr, hires_machine, loop_addr, run_program,
};

/// Number of setup instructions before the polling loop's first iteration —
/// see [`setup_bytes`].
const SETUP_INSTRUCTIONS: usize = 22;
/// Byte length of the fixed setup block below (through the port-A output = 0 write).
const SETUP_LEN: u16 = 57;

/// Hand-assembled 6809 setup block, placed at `$8000`, arming the CoCo Max III trigger by
/// driving the PIA0 port-A PA0-3 nibble to 0 — while the DAC is ramped to a nonzero value and
/// never written to 0, proving the DAC is not this interface's trigger source:
/// ```text
/// $8000  10 CE 5E FF   LDS  #$5EFF
/// $8004  86 34         LDA  #$34
/// $8006  B7 FF 01      STA  $FF01        ; PIA0 CRA: data reg, CA2 (axis) low
/// $8009  B7 FF 03      STA  $FF03        ; PIA0 CRB: data reg, CB2 (stick) low
/// $800C  86 00         LDA  #$00
/// $800E  B7 FF 21      STA  $FF21        ; PIA1 CRA -> DDR access
/// $8011  86 FE         LDA  #$FE
/// $8013  B7 FF 20      STA  $FF20        ; PIA1 DDRA = $FE
/// $8016  86 34         LDA  #$34
/// $8018  B7 FF 21      STA  $FF21        ; PIA1 CRA -> data reg
/// $801B  86 80         LDA  #$80
/// $801D  B7 FF 20      STA  $FF20        ; DAC = 32 (nonzero, stays there forever — must stay
///                                        ; below $3F too, or the stock comparator's own
///                                        ; `dac < SATURATED_POT` guard would mask saturation)
/// $8020  86 0F         LDA  #$0F
/// $8022  B7 FF 00      STA  $FF00        ; PIA0 port-A output = $0F (DDR still 0: no-op on pins)
/// $8025  86 30         LDA  #$30
/// $8027  B7 FF 01      STA  $FF01        ; PIA0 CRA -> DDR-access mode
/// $802A  86 0F         LDA  #$0F
/// $802C  B7 FF 00      STA  $FF00        ; PIA0 DDRA = $0F (PA0-3 outputs; nibble = $0F, still unarmed)
/// $802F  86 34         LDA  #$34
/// $8031  B7 FF 01      STA  $FF01        ; PIA0 CRA -> data-access mode (axis-select preserved)
/// $8034  86 00         LDA  #$00
/// $8036  B7 FF 00      STA  $FF00        ; PIA0 port-A output = $00 -> nibble 0: ARMS (last setup instr)
/// ```
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
        0x86, 0x80, // LDA #$80
        0xB7, 0xFF, 0x20, // STA $FF20 (DAC = 32, stays nonzero and below $3F forever)
        0x86, 0x0F, // LDA #$0F
        0xB7, 0xFF, 0x00, // STA $FF00 (PIA0 port-A output = $0F, DDR still 0: no-op)
        0x86, 0x30, // LDA #$30
        0xB7, 0xFF, 0x01, // STA $FF01 (PIA0 CRA -> DDR-access mode)
        0x86, 0x0F, // LDA #$0F
        0xB7, 0xFF, 0x00, // STA $FF00 (PIA0 DDRA = $0F: PA0-3 outputs, nibble = $0F)
        0x86, 0x34, // LDA #$34
        0xB7, 0xFF, 0x01, // STA $FF01 (PIA0 CRA -> data-access mode)
        0x86, 0x00, // LDA #$00
        0xB7, 0xFF, 0x00, // STA $FF00 (PIA0 port-A output = $00 -> nibble 0: ARMS)
    ]
}

/// Runs the program to completion for `pot`. Returns the loop's iteration count (from RAM)
/// and the CPU cycles elapsed from the arming write to the moment PA7 first reads high.
fn run_for_pot(pot: u16) -> (u16, u32) {
    let setup = setup_bytes();
    assert_eq!(
        setup.len(),
        SETUP_LEN as usize,
        "SETUP_LEN must track the setup block above"
    );
    let rom = assemble_rom(&setup, &[]);
    let mut m = hires_machine(rom);
    m.bus.joysticks.set_hires(RIGHT, HiResInterface::CoCoMax3);
    m.bus.joysticks.set_pot(RIGHT, AXIS_X, pot);

    let loop_addr = loop_addr(SETUP_LEN, 0);
    let done_addr = done_addr(SETUP_LEN, 0);
    run_program(&mut m, SETUP_INSTRUCTIONS, 0, loop_addr, done_addr)
}

#[test]
fn loop_count_is_monotonic_and_tracks_duration_cycles() {
    let (count_min, cycles_min) = run_for_pot(0);
    let (count_mid, cycles_mid) = run_for_pot(POT_MAX / 2);
    let (count_max, cycles_max) = run_for_pot(POT_MAX);

    assert_tracks_duration(
        HiResInterface::CoCoMax3,
        [
            (0, count_min, cycles_min),
            (POT_MAX / 2, count_mid, cycles_mid),
            (POT_MAX, count_max, cycles_max),
        ],
    );
}

// No fast-clock variant here: the fast-clock credit path (`Joysticks::tick`'s half-weight
// carry) is shared verbatim between every hi-res interface kind, and `tandy.rs`'s
// `fast_clock_measures_roughly_double_the_raw_cycles` already exercises it end to end through
// this same bus/run-loop wiring — re-running it against a second `duration_cycles` curve would
// only prove arithmetic already covered.
