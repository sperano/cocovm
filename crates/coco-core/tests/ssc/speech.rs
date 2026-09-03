//! Allophone-stream speech through the SP0256-AL2: EXECUTE commands feed the
//! chip, `$FF7E` bit 6 (SBY) tracks it, its output reaches the mux's
//! cartridge input, and `$FF7D`/`$C7`/`$00` cut it short. Every test here
//! needs `roms/sp0256-al2.rom` and skips without it.

use coco_core::SystemBus;
use coco_core::ssc::{ALLOPHONE_COUNT, cmd, terminator};
use mc6809::Bus;

use super::common::{CLEAR_BUSY, FF7D, FF7E, pump, try_bus_with_speech_selected};

/// `$FF7E` bit 6: SP0256 SBY, set while idle.
const SPEECH_READY: u8 = 0x40;
/// `$FF7E` bit 5: Sound Activity Circuit, set while the PSG is quiet.
const QUIET: u8 = 0x20;
/// `$FF7D` bit 0: the SP0256 RESET pin.
const SP0256_RESET: u8 = 0x01;

/// The 26-3144 manual's page-13 "Color Computer" stream: KK3 AX LL ER1,
/// three PA5s, KK3 AX MM PP YY1 UW2 TT2 ER1, PA5.
const COLOR_COMPUTER: [u8; 16] = [8, 15, 45, 51, 4, 4, 4, 8, 15, 16, 9, 49, 31, 13, 51, 4];

/// E-cycles per `pump` call.
const PUMP_CYCLES: u32 = 100;
/// Bound on how long a stream may take to finish: well past the manual's
/// ~2.4 s worth of allophones at 894,886 cycles/s.
const MAX_PUMPS: u32 = 40_000;
/// Peak level a spoken vowel must clear at the mux output.
const AUDIBLE_PEAK: f32 = 0.05;

fn send(b: &mut SystemBus, byte: u8) {
    b.write(FF7E, byte);
    b.cart.tick(CLEAR_BUSY);
}

fn load_buffer0(b: &mut SystemBus, allophones: &[u8]) {
    send(b, cmd::LOAD_ALLOPHONE_INDIVIDUAL_START);
    for &a in allophones {
        send(b, a);
    }
    send(b, terminator::SOUND);
}

fn speaking(b: &mut SystemBus) -> bool {
    b.read(FF7E) & SPEECH_READY == 0
}

/// Pumps until SBY rises, returning `(pumps, peak level)`.
fn pump_until_idle(b: &mut SystemBus) -> (u32, f32) {
    let mut peak = 0.0f32;
    let mut pumps = 0;
    while speaking(b) {
        peak = peak.max(pump(b, 1).abs());
        pumps += 1;
        assert!(pumps < MAX_PUMPS, "speech never finished");
    }
    (pumps, peak)
}

#[test]
fn execute_allophone_stream_speaks_then_goes_idle() {
    let Some(mut b) = try_bus_with_speech_selected() else {
        eprintln!(
            "skipping execute_allophone_stream_speaks_then_goes_idle: sp0256-al2.rom not present"
        );
        return;
    };
    assert!(!speaking(&mut b), "idle before any command");
    load_buffer0(&mut b, &COLOR_COMPUTER);
    assert!(!speaking(&mut b), "loading must not start speech");

    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    assert!(
        speaking(&mut b),
        "SBY drops as soon as the first allophone is latched"
    );
    let (pumps, peak) = pump_until_idle(&mut b);
    // The stream is roughly 2.4 s of allophones; 1 s and 4 s bracket it generously.
    let seconds = f64::from(pumps * PUMP_CYCLES) / 894_886.0;
    assert!((1.0..4.0).contains(&seconds), "spoke for {seconds:.2} s");
    assert!(
        peak > AUDIBLE_PEAK,
        "speech must reach the mux output: peak {peak}"
    );
}

#[test]
fn speech_bypasses_the_sound_activity_circuit() {
    let Some(mut b) = try_bus_with_speech_selected() else {
        eprintln!(
            "skipping speech_bypasses_the_sound_activity_circuit: sp0256-al2.rom not present"
        );
        return;
    };
    load_buffer0(&mut b, &COLOR_COMPUTER);
    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
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
    let Some(mut b) = try_bus_with_speech_selected() else {
        eprintln!("skipping abort_all_speech_cuts_the_stream_short: sp0256-al2.rom not present");
        return;
    };
    load_buffer0(&mut b, &COLOR_COMPUTER);
    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    let (full, _) = pump_until_idle(&mut b);

    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    pump(&mut b, 100);
    send(&mut b, cmd::ABORT_ALL_SPEECH);
    let (aborted, _) = pump_until_idle(&mut b);
    // The latched allophone (and the one already queued) still play out.
    assert!(
        aborted < full / 2,
        "aborted after {aborted} pumps vs {full} for the full stream"
    );
}

#[test]
fn stop_all_sound_00_also_stops_speech_but_cf_does_not() {
    let Some(mut b) = try_bus_with_speech_selected() else {
        eprintln!(
            "skipping stop_all_sound_00_also_stops_speech_but_cf_does_not: sp0256-al2.rom not present"
        );
        return;
    };
    load_buffer0(&mut b, &COLOR_COMPUTER);
    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    let (full, _) = pump_until_idle(&mut b);

    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    pump(&mut b, 100);
    send(&mut b, cmd::STOP_ALL_SOUND_ALT);
    let (after_cf, _) = pump_until_idle(&mut b);
    assert!(
        after_cf > full / 2,
        "$CF is sound-only: {after_cf} vs {full}"
    );

    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    pump(&mut b, 100);
    send(&mut b, cmd::STOP_ALL_SOUND);
    let (after_00, _) = pump_until_idle(&mut b);
    assert!(
        after_00 < full / 2,
        "$00 stops speech too: {after_00} vs {full}"
    );
}

#[test]
fn ff7d_bit0_resets_the_chip_mid_allophone() {
    let Some(mut b) = try_bus_with_speech_selected() else {
        eprintln!("skipping ff7d_bit0_resets_the_chip_mid_allophone: sp0256-al2.rom not present");
        return;
    };
    load_buffer0(&mut b, &COLOR_COMPUTER);
    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    pump(&mut b, 200);
    assert!(speaking(&mut b));

    b.write(FF7D, SP0256_RESET);
    assert!(!speaking(&mut b), "RESET halts the chip at once");
    // The stream cursor is firmware state, untouched by the chip's reset pin:
    // on the next tick it hands the chip the rest of the stream.
    pump(&mut b, 1);
    assert!(speaking(&mut b), "the rest of the stream follows");
    pump_until_idle(&mut b);
}

#[test]
fn bytes_at_or_above_the_allophone_count_are_skipped() {
    let Some(mut b) = try_bus_with_speech_selected() else {
        eprintln!(
            "skipping bytes_at_or_above_the_allophone_count_are_skipped: sp0256-al2.rom not present"
        );
        return;
    };
    const PA5: u8 = 4;
    load_buffer0(&mut b, &[PA5]);
    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    let (pause_only, _) = pump_until_idle(&mut b);

    load_buffer0(&mut b, &[ALLOPHONE_COUNT, 0xFE, PA5]);
    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    let (with_junk, _) = pump_until_idle(&mut b);
    // The chip picks a load up at its next period boundary (up to 64
    // samples, ~7 ms, ~60 pumps), so the two runs differ by well under the
    // shortest real allophone (~40 ms, ~360 pumps).
    const PICKUP_SLACK: u32 = 100;
    assert!(
        with_junk.abs_diff(pause_only) <= PICKUP_SLACK,
        "{with_junk} vs {pause_only}"
    );
}

#[test]
fn consecutive_execute_spans_buffers() {
    let Some(mut b) = try_bus_with_speech_selected() else {
        eprintln!("skipping consecutive_execute_spans_buffers: sp0256-al2.rom not present");
        return;
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
    let (buffer0_only, _) = pump_until_idle(&mut b);
    send(&mut b, cmd::EXEC_ALLOPHONE_CONSECUTIVE_START);
    let (whole, _) = pump_until_idle(&mut b);
    assert!(whole > buffer0_only + 100, "{whole} vs {buffer0_only}");
}
