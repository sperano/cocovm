//! Allophone-stream speech: the firmware feeds the SP0256, `$FF7E` bit 6
//! (SBY) tracks it, its output reaches the mux's cartridge input, and
//! `$FF7D`/`$C7`/`$00` cut it short. Every test here needs both SSC ROMs.

use coco_core::SystemBus;
use coco_core::ssc::{cmd, terminator};
use mc6809::Bus;

use super::common::{
    AUDIBLE_PEAK, E_CLOCK_HZ, FF7D, FF7E, PUMP_CYCLES, QUIET, SPEECH_MAX_PUMPS, hear, pump, send,
    skip, speaking, try_bus_with_ssc_selected,
};

/// `$FF7D` bit 0: the SP0256 RESET pin.
const SP0256_RESET: u8 = 0x01;

/// The 26-3144 manual's page-13 "Color Computer" stream: KK3 AX LL ER1,
/// three PA5s, KK3 AX MM PP YY1 UW2 TT2 ER1, PA5.
const COLOR_COMPUTER: [u8; 16] = [8, 15, 45, 51, 4, 4, 4, 8, 15, 16, 9, 49, 31, 13, 51, 4];

fn load_buffer0(b: &mut SystemBus, allophones: &[u8]) {
    send(b, cmd::LOAD_ALLOPHONE_INDIVIDUAL_START);
    for &a in allophones {
        send(b, a);
    }
    send(b, terminator::SOUND);
}

#[test]
fn execute_allophone_stream_speaks_then_goes_idle() {
    let Some(mut b) = try_bus_with_ssc_selected() else {
        return skip("execute_allophone_stream_speaks_then_goes_idle");
    };
    assert!(!speaking(&mut b), "idle before any command");
    load_buffer0(&mut b, &COLOR_COMPUTER);
    assert!(!speaking(&mut b), "loading must not start speech");

    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    let (pumps, peak) = hear(&mut b);
    // The stream is roughly 2.4 s of allophones; 1 s and 4 s bracket it generously.
    let seconds = f64::from(pumps * PUMP_CYCLES) / E_CLOCK_HZ;
    assert!((1.0..4.0).contains(&seconds), "spoke for {seconds:.2} s");
    assert!(
        peak > AUDIBLE_PEAK,
        "speech must reach the mux output: peak {peak}"
    );
}

#[test]
fn speech_bypasses_the_sound_activity_circuit() {
    let Some(mut b) = try_bus_with_ssc_selected() else {
        return skip("speech_bypasses_the_sound_activity_circuit");
    };
    load_buffer0(&mut b, &COLOR_COMPUTER);
    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    while !speaking(&mut b) {
        pump(&mut b, 1);
    }
    let mut quiet_throughout = true;
    while speaking(&mut b) {
        pump(&mut b, 1);
        quiet_throughout &= b.read(FF7E) & QUIET != 0;
    }
    assert!(
        quiet_throughout,
        "bit 5 reports only the PSG; speech must not clear it"
    );
}

#[test]
fn abort_all_speech_cuts_the_stream_short() {
    let Some(mut b) = try_bus_with_ssc_selected() else {
        return skip("abort_all_speech_cuts_the_stream_short");
    };
    load_buffer0(&mut b, &COLOR_COMPUTER);
    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    let (full, _) = hear(&mut b);

    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    while !speaking(&mut b) {
        pump(&mut b, 1);
    }
    pump(&mut b, 100);
    send(&mut b, cmd::ABORT_ALL_SPEECH);
    let mut aborted = 0;
    while speaking(&mut b) {
        pump(&mut b, 1);
        aborted += 1;
        assert!(aborted < SPEECH_MAX_PUMPS);
    }
    assert!(
        aborted < full / 2,
        "aborted after {aborted} pumps vs {full} for the full stream"
    );
}

#[test]
fn stop_all_sound_00_also_stops_speech() {
    let Some(mut b) = try_bus_with_ssc_selected() else {
        return skip("stop_all_sound_00_also_stops_speech");
    };
    load_buffer0(&mut b, &COLOR_COMPUTER);
    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    let (full, _) = hear(&mut b);

    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    while !speaking(&mut b) {
        pump(&mut b, 1);
    }
    pump(&mut b, 100);
    send(&mut b, cmd::STOP_ALL_SOUND);
    let mut stopped = 0;
    while speaking(&mut b) {
        pump(&mut b, 1);
        stopped += 1;
        assert!(stopped < SPEECH_MAX_PUMPS);
    }
    assert!(
        stopped < full / 2,
        "$00 stops speech too: {stopped} vs {full}"
    );
}

#[test]
fn ff7d_bit0_resets_the_chip_mid_allophone() {
    let Some(mut b) = try_bus_with_ssc_selected() else {
        return skip("ff7d_bit0_resets_the_chip_mid_allophone");
    };
    load_buffer0(&mut b, &COLOR_COMPUTER);
    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    while !speaking(&mut b) {
        pump(&mut b, 1);
    }
    pump(&mut b, 200);
    assert!(speaking(&mut b));

    b.write(FF7D, SP0256_RESET);
    assert!(!speaking(&mut b), "RESET halts the chip at once");
}

#[test]
fn consecutive_execute_spans_buffers() {
    let Some(mut b) = try_bus_with_ssc_selected() else {
        return skip("consecutive_execute_spans_buffers");
    };
    // A 70-allophone stream loaded with $A0 spills from buffer 0 into 1.
    const PA1: u8 = 0;
    const PA5: u8 = 4;
    let mut stream = vec![PA1; 69];
    stream.push(PA5);
    send(&mut b, cmd::LOAD_ALLOPHONE_CONSECUTIVE_START);
    for &a in &stream {
        send(&mut b, a);
    }
    send(&mut b, terminator::SOUND);

    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    let (buffer0_only, _) = hear(&mut b);
    send(&mut b, cmd::EXEC_ALLOPHONE_CONSECUTIVE_START);
    let (whole, _) = hear(&mut b);
    assert!(whole > buffer0_only + 100, "{whole} vs {buffer0_only}");
}
