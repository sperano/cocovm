use super::workloads::*;
use coco_core::{Machine, MachineConfig};

fn machine() -> Machine {
    Machine::new(
        MachineConfig::default(),
        vec![0; ROM_BYTES].into_boxed_slice(),
    )
}

#[test]
fn dac_loop_stays_in_program_and_changes_every_write() {
    let mut machine = machine();
    configure_dac(&mut machine);
    const LOOP_ITERATIONS: u16 = 128;
    const STORE_BYTES: u16 = 3;
    const ADD_BYTES: u16 = 2;
    for iteration in 0..LOOP_ITERATIONS {
        machine.step_instruction();
        assert_eq!(machine.cpu.pc, PROGRAM_START + STORE_BYTES);
        machine.step_instruction();
        assert_eq!(machine.cpu.pc, PROGRAM_START + STORE_BYTES + ADD_BYTES);
        assert_eq!(
            machine.cpu.a,
            (iteration as u8 + 1).wrapping_mul(DAC_INCREMENT)
        );
        machine.step_instruction();
        assert_eq!(machine.cpu.pc, PROGRAM_START);
    }
}

#[test]
fn dac_workload_continues_producing_changing_mono_audio() {
    let mut machine = machine();
    configure_dac(&mut machine);
    assert_changing_audio(&mut machine);
    assert_changing_audio(&mut machine);
    machine.run_field();
    let expected = machine.config.video.lines_per_field() * coco_core::audio::OVERSAMPLE;
    let samples: Vec<_> = machine.take_audio().collect();
    assert_eq!(samples.len(), expected as usize);
    assert!(samples.iter().all(|[left, right]| left == right));
}

#[test]
fn cartridge_workload_produces_changing_audio() {
    let mut machine = machine();
    configure_cartridge(&mut machine);
    assert_changing_audio(&mut machine);
    assert_changing_audio(&mut machine);
}

#[test]
fn graphics_workload_changes_after_multiple_complete_paint_passes() {
    const SETTLE_FIELDS: usize = 30;
    const OBSERVATION_FIELDS: usize = 30;
    let mut machine = machine();
    configure_graphics(&mut machine);
    for _ in 0..SETTLE_FIELDS {
        machine.run_field();
        machine.take_audio().count();
    }
    let before = machine.framebuffer.clone();
    for _ in 0..OBSERVATION_FIELDS {
        machine.run_field();
        machine.take_audio().count();
    }
    assert_ne!(
        machine.framebuffer, before,
        "graphics must keep changing after initial fill"
    );
}
