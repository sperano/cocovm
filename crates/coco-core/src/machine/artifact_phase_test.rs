use super::*;
use crate::config::{MachineConfig, MachineVariant, MemorySize, VDGVariant, VideoStandard};

const ROM_SIZE: usize = 16 * 1024;
const TEST_SEED: u64 = 0x1234_5678_9abc_def0;
const SEQUENCE_LENGTH: usize = 16;

fn coco2_config() -> MachineConfig {
    MachineConfig {
        variant: MachineVariant::Coco2,
        video: VideoStandard::NTSC,
        memory: MemorySize::K64,
        monitor: None,
        vdg: Some(VDGVariant::MC6847),
    }
}

fn seeded_machine() -> Machine {
    Machine::new_with_artifact_seed(
        coco2_config(),
        vec![0; ROM_SIZE].into_boxed_slice(),
        TEST_SEED,
    )
}

fn phase_sequence(machine: &mut Machine) -> Vec<video::RG6ArtifactPhase> {
    let mut phases = Vec::with_capacity(SEQUENCE_LENGTH);
    phases.push(machine.ntsc_rg6_artifact_phase());
    for _ in 1..SEQUENCE_LENGTH {
        machine.reset();
        phases.push(machine.ntsc_rg6_artifact_phase());
    }
    phases
}

#[test]
fn explicit_seed_reproduces_reset_phase_sequence() {
    assert_eq!(
        phase_sequence(&mut seeded_machine()),
        phase_sequence(&mut seeded_machine())
    );
}

#[test]
fn reset_sequence_can_select_both_phases() {
    let phases = phase_sequence(&mut seeded_machine());

    assert!(phases.contains(&video::RG6ArtifactPhase::Standard));
    assert!(phases.contains(&video::RG6ArtifactPhase::Reverse));
}

#[test]
fn snapshot_round_trip_preserves_phase_and_future_sequence() {
    let mut original = seeded_machine();
    original.reset();
    let mut encoded = Vec::new();
    ciborium::into_writer(&original, &mut encoded).expect("serialize machine");
    let mut restored: Machine = ciborium::from_reader(encoded.as_slice()).expect("restore machine");

    assert_eq!(
        original.ntsc_rg6_artifact_phase(),
        restored.ntsc_rg6_artifact_phase()
    );
    assert_eq!(phase_sequence(&mut original), phase_sequence(&mut restored));
}
