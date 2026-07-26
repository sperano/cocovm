//! The event-timestamped audio grid (`docs/plan-audio-pipeline.md`):
//! sub-scanline DAC timing must land in the right grid slot, and a level
//! pulse entirely inside one scanline — invisible to the old once-per-line
//! point sampler, the digitized-PCM aliasing defect — must reach the grid.
//!
//! The machine runs a zero-filled ROM (reset vector → $0000; the harness
//! parks a `BRA *` there) so cycles advance deterministically, 3 per loop.

use coco_core::audio::OVERSAMPLE;
use coco_core::{Machine, MachineConfig, StepKind};
use mc6809::Bus;

const PIA1_DA: u16 = 0xFF20;
const PIA1_CRA: u16 = 0xFF21;
const PIA1_DDRB: u16 = 0xFF22;
const PIA1_CRB: u16 = 0xFF23;
const PIA1_DDRA: u16 = 0xFF20;
/// Control value: data register selected, C2 set/reset output low/high.
const CR_C2_LOW: u8 = 0x34;
const CR_C2_HIGH: u8 = 0x3C;
/// Control value selecting the DDR (bit 2 clear).
const CR_DDR: u8 = 0x30;

/// A parked CoCo 3 with the DAC speaker path enabled: PA2-7 outputs,
/// SNDEN high, mux SEL=00.
fn dac_machine() -> Machine {
    let mut m = Machine::new(
        MachineConfig::default(),
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    m.bus.write(0x0000, 0x20); // BRA
    m.bus.write(0x0001, 0xFE); // -2
    m.bus.write(PIA1_CRA, CR_DDR);
    m.bus.write(PIA1_DDRA, 0xFC);
    m.bus.write(PIA1_CRB, CR_DDR);
    m.bus.write(PIA1_DDRB, 0x02);
    m.bus.write(PIA1_CRA, CR_C2_LOW);
    m.bus.write(PIA1_CRB, CR_C2_HIGH); // SNDEN on (PIA0 CA2/CB2 reset low = SEL 00)
    m
}

/// Step to the start of the next scanline, then `cycles` further into it.
/// Returns nothing; the caller pokes registers at the reached point.
fn step_into_line(m: &mut Machine, cycles: u32) {
    let line = m.scanline();
    while m.scanline() == line {
        m.step_instruction();
    }
    let mut spent = 0;
    while spent < cycles {
        if let StepKind::Instruction { cycles: c } = m.step_instruction().kind {
            spent += c;
        }
    }
}

/// Finish the current scanline and return its `OVERSAMPLE` grid frames.
fn finish_line(m: &mut Machine) -> Vec<[f32; 2]> {
    let line = m.scanline();
    while m.scanline() == line {
        m.step_instruction();
    }
    let all: Vec<[f32; 2]> = m.take_audio().collect();
    assert!(all.len() >= OVERSAMPLE as usize, "at least one line flushed");
    all[all.len() - OVERSAMPLE as usize..].to_vec()
}

#[test]
fn dac_write_mid_line_splits_the_grid_slots() {
    let mut m = dac_machine();
    m.run_field(); // let the setup writes settle into a flushed field
    m.take_audio().count();

    // ~30 cycles into a ~57-cycle line: past the second slot boundary.
    step_into_line(&mut m, 30);
    m.bus.write(PIA1_DA, 0xFC); // DAC full scale
    let grid = finish_line(&mut m);

    assert_eq!(grid[0], [0.0; 2], "slot 0 predates the write");
    let loud = *grid.last().unwrap();
    assert!(loud[0] > 0.5, "last slot carries the DAC level: {loud:?}");
    let transitions = grid.windows(2).filter(|w| w[0] != w[1]).count();
    assert_eq!(transitions, 1, "exactly one level step within the line: {grid:?}");

    // The write held: the next full line is loud in every slot.
    let next = finish_line(&mut m);
    assert!(next.iter().all(|s| s[0] > 0.5), "level holds: {next:?}");
}

#[test]
fn dac_pulse_within_one_line_reaches_the_grid() {
    // The aliasing defect the pipeline exists to fix: raise the DAC ~10
    // cycles into a line and drop it ~30 cycles later. The line's FINAL
    // state is silent — the old once-per-line point sampler read exactly
    // that and heard nothing — but the grid must carry the pulse.
    let mut m = dac_machine();
    m.run_field();
    m.take_audio().count();

    step_into_line(&mut m, 10);
    m.bus.write(PIA1_DA, 0xFC);
    let mut spent = 0;
    while spent < 30 {
        if let StepKind::Instruction { cycles } = m.step_instruction().kind {
            spent += cycles;
        }
    }
    m.bus.write(PIA1_DA, 0x00);
    let grid = finish_line(&mut m);

    assert!(
        grid.iter().any(|s| s[0] > 0.5),
        "the intra-line pulse must be audible on the grid: {grid:?}"
    );
    assert_eq!(
        *grid.last().unwrap(),
        [0.0; 2],
        "the line ends silent — the state the old sampler was limited to"
    );
}

#[test]
fn beeper_toggle_is_centred_on_both_channels() {
    let mut m = dac_machine();
    m.run_field();
    m.take_audio().count();

    step_into_line(&mut m, 20);
    m.bus.write(PIA1_DDRB, 0x02); // PB1 high (data reg selected, bit 1 set)
    let grid = finish_line(&mut m);
    let on = grid.last().unwrap();
    assert!(on[0] > 0.0, "beeper reaches the grid");
    assert_eq!(on[0], on[1], "internal sources are centred");
}
