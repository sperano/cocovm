//! Text-to-speech: the firmware's ROM-based English rules turn typed text
//! into allophones (26-3144 manual, "Speaking of the S/SC"). Needs both SSC
//! ROMs.

use coco_core::ssc::{cmd, terminator};

use super::common::{
    AUDIBLE_PEAK, E_CLOCK_HZ, PUMP_CYCLES, hear, send, skip, speaking, try_bus_with_ssc_selected,
};

#[test]
fn typed_text_followed_by_a_carriage_return_is_spoken() {
    let Some(mut b) = try_bus_with_ssc_selected() else {
        return skip("typed_text_followed_by_a_carriage_return_is_spoken");
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
    let seconds = f64::from(pumps * PUMP_CYCLES) / E_CLOCK_HZ;
    assert!((0.3..4.0).contains(&seconds), "spoke for {seconds:.2} s");
    assert!(
        peak > AUDIBLE_PEAK,
        "speech must reach the mux output: peak {peak}"
    );
}

#[test]
fn speech_string_buffer_load_then_execute_is_spoken() {
    let Some(mut b) = try_bus_with_ssc_selected() else {
        return skip("speech_string_buffer_load_then_execute_is_spoken");
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
