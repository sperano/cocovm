use super::datafmt::{DATAFMT, block_range};
use super::lpc::inverse_quantize;
use super::*;

use test_assets::rom::SP0256_AL2;

/// Allophone addresses from the 26-3144 manual's Appendix C.
const PA1: u8 = 0x00;
const PA5: u8 = 0x04;
const OY: u8 = 0x05;
const AY: u8 = 0x06;
const AW: u8 = 0x20;

/// Samples (10 kHz) from ALD to SBY per allophone address, played in order
/// from a fresh chip. Golden values from this port, cross-checked against
/// MAME: its coco3 + S/SC speaking the manual's "Color Computer" stream
/// spans ~320 ms and ~990 ms for the two words, matching these figures
/// (Appendix C's nominal durations run 25-40% longer and are not what the
/// core produces).
#[rustfmt::skip]
const GOLDEN_SAMPLES: [u32; 64] = [
      66,  324,  519, 1039, 2079, 3008, 1839,  643,  863, 1548, 1055, 1839,  551, 1056, 1351,  643,
    1931,  865, 1443, 1839, 2115,  551,  735,  827,  735, 1379,  919,  987,  431, 1371,  799, 1839,
    2667,  819, 1207, 1379,  819, 2075, 1411,  891, 1179, 1425, 1222, 1559, 2115,  919, 1563, 2575,
    1547, 1011, 1568, 1167, 2207, 1839, 1931,  731, 1443, 1355, 2483, 2115, 2575,  791, 1471,  599,
];

fn load_rom() -> Option<Vec<u8>> {
    std::fs::read(test_assets::rom(SP0256_AL2)).ok()
}

fn chip() -> Option<SP0256> {
    let rom = load_rom()?;
    Some(SP0256::new(&rom).expect("installed AL2 ROM is 2 KB"))
}

/// Load `address` and generate samples until SBY rises again, returning
/// them. Bounded so a sequencer bug can't spin the test forever.
fn play(chip: &mut SP0256, address: u8) -> Vec<i16> {
    const MAX_SAMPLES: usize = 100_000;
    chip.ald_write(address);
    let mut samples = Vec::new();
    while !chip.sby() {
        samples.push(chip.generate_sample());
        assert!(
            samples.len() < MAX_SAMPLES,
            "allophone {address:#04x} never finished"
        );
    }
    samples
}

fn peak(samples: &[i16]) -> i16 {
    samples
        .iter()
        .map(|s| s.saturating_abs())
        .max()
        .unwrap_or(0)
}

// ---- No ROM needed ---------------------------------------------------------

#[test]
fn new_rejects_a_wrong_sized_rom() {
    assert_eq!(
        SP0256::new(&[0; ROM_SIZE - 1]).err(),
        Some(ROMSizeError {
            actual: ROM_SIZE - 1
        })
    );
    assert!(SP0256::new(&[0; ROM_SIZE]).is_ok());
}

#[test]
fn reset_state_is_idle_and_ready() {
    let chip = SP0256::new(&[0; ROM_SIZE]).unwrap();
    assert!(chip.sby());
    assert!(chip.lrq());
    assert_eq!(chip.output(), 0.0);
}

#[test]
fn zero_rom_halts_immediately_and_stays_silent() {
    // Every fetch reads opcode 0 with immed4 0: RTS on an empty stack = HLT.
    let mut chip = SP0256::new(&[0; ROM_SIZE]).unwrap();
    let samples = play(&mut chip, OY);
    assert!(
        samples.len() < 10,
        "HLT should finish within a period: {}",
        samples.len()
    );
    assert_eq!(peak(&samples), 0);
    assert!(chip.lrq());
}

#[test]
fn ald_write_is_dropped_while_lrq_is_low() {
    let mut chip = SP0256::new(&[0; ROM_SIZE]).unwrap();
    chip.ald_write(OY);
    assert!(!chip.lrq());
    chip.ald_write(AY);
    assert_eq!(
        chip.ald,
        u32::from(OY) << ALD_SHIFT,
        "second load must not replace the first"
    );
}

#[test]
fn step_accrues_samples_at_the_fixed_ratio() {
    let mut chip = SP0256::new(&[0; ROM_SIZE]).unwrap();
    // One sample per E_CLOCK_HZ / SAMPLE_RATE_HZ = 89.4886 cycles.
    chip.step(89);
    assert_eq!(chip.cycle_acc, 89 * SAMPLE_RATE_HZ);
    chip.step(1);
    assert_eq!(chip.cycle_acc, 90 * SAMPLE_RATE_HZ - E_CLOCK_HZ);
    // A whole second lands exactly on a sample boundary.
    chip.cycle_acc = 0;
    chip.step(E_CLOCK_HZ as u32);
    assert_eq!(chip.cycle_acc, 0);
}

#[test]
fn output_interpolates_between_the_last_two_samples() {
    let mut chip = SP0256::new(&[0; ROM_SIZE]).unwrap();
    chip.prev_sample = 0;
    chip.cur_sample = 16384;
    chip.cycle_acc = E_CLOCK_HZ / 2;
    assert!((chip.output() - 0.25).abs() < 1e-3);
    chip.cycle_acc = 0;
    assert_eq!(chip.output(), 0.0);
}

#[test]
fn coefficient_decode_is_odd_symmetric() {
    assert_eq!(inverse_quantize(0x00), 0);
    assert_eq!(inverse_quantize(0x80), 0);
    assert_eq!(inverse_quantize(0x7F), -511);
    assert_eq!(inverse_quantize(0x81), 511);
    assert_eq!(inverse_quantize(0x01), -9);
    assert_eq!(inverse_quantize(0xFF), 9);
}

#[test]
fn every_operand_opcode_has_a_well_formed_layout_row() {
    const OPERAND_OPCODES: [u8; 12] = [0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8, 0x9, 0xA, 0xB, 0xC, 0xF];
    const CONTROL_OPCODES: [u8; 4] = [0x0, 0x1, 0xD, 0xE];
    for mode in [0u8, 2, 4, 6] {
        for opcode in OPERAND_OPCODES {
            let (first, last) = block_range(opcode, mode).expect("operand opcode has a row");
            assert!(first <= last && last < DATAFMT.len(), "{opcode:#x}/{mode}");
        }
        for opcode in CONTROL_OPCODES {
            assert!(block_range(opcode, mode).is_none(), "{opcode:#x}/{mode}");
        }
    }
}

// ---- Real AL2 ROM (skipped when roms/sp0256-al2.rom is absent) ------------

#[test]
fn allophone_durations_match_the_golden_table() {
    let Some(mut chip) = chip() else {
        eprintln!(
            "skipping allophone_durations_match_the_golden_table: sp0256-al2.rom not present"
        );
        return;
    };
    let mut mismatches = Vec::new();
    for (address, &golden) in GOLDEN_SAMPLES.iter().enumerate() {
        let samples = play(&mut chip, address as u8).len() as u32;
        if samples != golden {
            mismatches.push(format!(
                "allophone {address:#04x}: {samples} samples, golden {golden}"
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn vowel_is_audible_and_pauses_are_silent() {
    let Some(mut chip) = chip() else {
        eprintln!("skipping vowel_is_audible_and_pauses_are_silent: sp0256-al2.rom not present");
        return;
    };
    let vowel = play(&mut chip, AW);
    assert!(peak(&vowel) > 2000, "peak {}", peak(&vowel));
    let pause = play(&mut chip, PA5);
    assert_eq!(peak(&pause), 0);
    let pause = play(&mut chip, PA1);
    assert_eq!(peak(&pause), 0);
}

#[test]
fn a_trailing_pause_silences_the_idle_chip() {
    let Some(mut chip) = chip() else {
        eprintln!("skipping a_trailing_pause_silences_the_idle_chip: sp0256-al2.rom not present");
        return;
    };
    play(&mut chip, OY);
    play(&mut chip, PA5);
    let idle: Vec<i16> = (0..1000).map(|_| chip.generate_sample()).collect();
    assert_eq!(peak(&idle), 0);
    assert!(chip.sby());
}

#[test]
fn queued_load_is_accepted_once_the_current_allophone_starts() {
    let Some(mut chip) = chip() else {
        eprintln!(
            "skipping queued_load_is_accepted_once_the_current_allophone_starts: sp0256-al2.rom not present"
        );
        return;
    };
    let solo = play(&mut chip, OY).len();
    chip.ald_write(OY);
    // The sequencer picks OY up (raising LRQ) once the idle period expires.
    let mut pair = 0;
    while !chip.lrq() {
        chip.generate_sample();
        pair += 1;
        assert!(pair < 200, "LRQ never rose");
    }
    assert!(!chip.sby(), "OY is playing");
    chip.ald_write(AY);
    while !chip.sby() {
        chip.generate_sample();
        pair += 1;
    }
    assert!(
        pair > solo + 100,
        "OY then AY ({pair}) must outlast OY alone ({solo})"
    );
}

#[test]
fn reset_mid_allophone_halts_and_silences() {
    let Some(mut chip) = chip() else {
        eprintln!("skipping reset_mid_allophone_halts_and_silences: sp0256-al2.rom not present");
        return;
    };
    chip.ald_write(AW);
    for _ in 0..500 {
        chip.generate_sample();
    }
    assert!(!chip.sby());
    chip.reset();
    assert!(chip.sby());
    assert!(chip.lrq());
    let after: Vec<i16> = (0..500).map(|_| chip.generate_sample()).collect();
    assert_eq!(peak(&after), 0);
}
