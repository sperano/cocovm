use super::*;
use crate::MachineConfig;
use crate::audio::{AudioEvent, AudioInputs};

const ROM_SIZE: usize = 32 * 1024;
const LINE_START: u64 = 1_000;
const LINE_SPAN: u64 = 57;
const FIRST_BOUNDARY: u64 = LINE_SPAN / audio::OVERSAMPLE as u64;
const AFTER_SECOND_BOUNDARY: u64 = LINE_SPAN * 2 / audio::OVERSAMPLE as u64 + 1;
const EVENT_CAPACITY: usize = 32;
const EVENT_COUNTS: [usize; 4] = [EVENT_CAPACITY, 4, 0, EVENT_CAPACITY / 2];

fn machine() -> Machine {
    let mut machine = Machine::new(
        MachineConfig::default(),
        vec![0; ROM_SIZE].into_boxed_slice(),
    );
    machine.audio_line_start = LINE_START;
    machine.bus.cycle_clock = LINE_START;
    machine
}

fn inputs(level: f32) -> AudioInputs {
    AudioInputs {
        cart_left: level,
        cart_right: -level,
        ..AudioInputs::default()
    }
}

fn enqueue(machine: &mut Machine, offset: u64, inputs: AudioInputs) {
    machine.bus.audio_events.push(AudioEvent {
        cycle: machine.audio_line_start + offset,
        inputs,
    });
    machine.bus.audio_inputs = inputs;
}

#[test]
fn scanlines_retain_event_storage_through_busy_and_silent_lines() {
    let mut machine = machine();
    machine.bus.audio_events.reserve(EVENT_CAPACITY);
    let capacity = machine.bus.audio_events.capacity();
    let allocation = machine.bus.audio_events.as_ptr();

    for event_count in EVENT_COUNTS {
        for offset in 0..event_count {
            enqueue(&mut machine, offset as u64, inputs(offset as f32));
        }
        machine.bus.cycle_clock += LINE_SPAN;
        machine.flush_line_audio();

        assert!(machine.bus.audio_events.is_empty());
        assert_eq!(machine.bus.audio_events.capacity(), capacity);
        assert_eq!(machine.bus.audio_events.as_ptr(), allocation);
        assert_eq!(machine.take_audio().count(), audio::OVERSAMPLE as usize);
    }
}

#[test]
fn event_replay_preserves_slots_equal_timestamp_order_and_tail_state() {
    const START_LEVEL: f32 = 0.125;
    const BETWEEN_LEVEL: f32 = 0.25;
    const BOUNDARY_LEVEL: f32 = 0.5;
    const LATE_LEVEL: f32 = 0.75;
    const TAIL_LEVEL: f32 = 1.0;
    let mut machine = machine();
    let events = [
        (0, BETWEEN_LEVEL),
        (0, START_LEVEL),
        (1, BETWEEN_LEVEL),
        (FIRST_BOUNDARY, START_LEVEL),
        (FIRST_BOUNDARY, BOUNDARY_LEVEL),
        (AFTER_SECOND_BOUNDARY, LATE_LEVEL),
        (LINE_SPAN - 1, TAIL_LEVEL),
    ];
    for (offset, level) in events {
        enqueue(&mut machine, offset, inputs(level));
    }
    machine.bus.cycle_clock += LINE_SPAN;
    machine.flush_line_audio();

    let expected = [START_LEVEL, BOUNDARY_LEVEL, BOUNDARY_LEVEL, LATE_LEVEL]
        .map(|level| audio::mix(&inputs(level), false, 0.0, (0.0, 0.0)));
    assert_eq!(machine.take_audio().collect::<Vec<_>>(), expected);
    assert!(machine.bus.audio_events.is_empty());

    machine.bus.cycle_clock += LINE_SPAN;
    machine.flush_line_audio();
    let tail = audio::mix(&inputs(TAIL_LEVEL), false, 0.0, (0.0, 0.0));
    assert_eq!(
        machine.take_audio().collect::<Vec<_>>(),
        vec![tail; audio::OVERSAMPLE as usize]
    );
}
