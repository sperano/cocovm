//! Text-to-speech: the firmware's ROM-based English rules turn typed text
//! into allophones (26-3144 manual, "Speaking of the S/SC"). Needs both SSC
//! ROMs.

use coco_core::SystemBus;
use coco_core::ssc::{cmd, terminator};
use mc6809::Bus;

use super::common::{FF7E, SPEECH_READY, pump, send, try_bus_with_ssc_selected};

/// E-cycles per `pump` call.
const PUMP_CYCLES: u32 = 100;
/// Bound on the wait for speech to start (the rules run) and to finish.
const MAX_PUMPS: u32 = 60_000;
/// Peak level speech must clear at the mux output.
const AUDIBLE_PEAK: f32 = 0.05;

fn speaking(b: &mut SystemBus) -> bool {
    b.read(FF7E) & SPEECH_READY == 0
}

/// Wait for speech to start then stop; `(pumps while speaking, peak)`.
fn hear(b: &mut SystemBus) -> (u32, f32) {
    let mut waited = 0;
    while !speaking(b) {
        pump(b, 1);
        waited += 1;
        assert!(waited < MAX_PUMPS, "speech never started");
    }
    let mut pumps = 0;
    let mut peak = 0.0f32;
    while speaking(b) {
        peak = peak.max(pump(b, 1).abs());
        pumps += 1;
        assert!(pumps < MAX_PUMPS, "speech never finished");
    }
    (pumps, peak)
}

#[test]
fn typed_text_followed_by_a_carriage_return_is_spoken() {
    let Some(mut b) = try_bus_with_ssc_selected() else {
        eprintln!(
            "skipping typed_text_followed_by_a_carriage_return_is_spoken: SSC ROMs not present"
        );
        return;
    };
    for &byte in b"I CAN TALK " {
        send(&mut b, byte);
    }
    assert!(
        !speaking(&mut b),
        "nothing is said before the carriage return"
    );
    send(&mut b, terminator::SPEECH);
    let (pumps, peak) = hear(&mut b);
    let seconds = f64::from(pumps * PUMP_CYCLES) / 894_886.0;
    assert!((0.3..4.0).contains(&seconds), "spoke for {seconds:.2} s");
    assert!(
        peak > AUDIBLE_PEAK,
        "speech must reach the mux output: peak {peak}"
    );
}

#[test]
fn speech_string_buffer_load_then_execute_is_spoken() {
    let Some(mut b) = try_bus_with_ssc_selected() else {
        eprintln!(
            "skipping speech_string_buffer_load_then_execute_is_spoken: SSC ROMs not present"
        );
        return;
    };
    send(&mut b, cmd::LOAD_SPEECH_INDIVIDUAL_START); // $90: buffer 0
    for &byte in b"HELLO " {
        send(&mut b, byte);
    }
    send(&mut b, terminator::SPEECH);
    assert!(!speaking(&mut b), "loading does not speak");

    send(&mut b, cmd::EXEC_SPEECH_INDIVIDUAL_START); // $D0
    let (pumps, peak) = hear(&mut b);
    assert!(pumps > 0);
    assert!(peak > AUDIBLE_PEAK, "peak {peak}");
}
