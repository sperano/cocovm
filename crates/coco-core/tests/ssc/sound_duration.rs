//! Sound-data event timing as the firmware's timer 1 paces it: measured
//! from the AY's volume register, not invented. Needs both SSC ROMs.

use coco_core::SystemBus;
use coco_core::ay8913::reg as ay_reg;
use coco_core::ssc::{cmd, group, terminator};

use super::common::{E_CLOCK_HZ, POLL_STEP, send, skip, try_coco3_bus_with_ssc};

/// Bound on one event's length while polling.
const MAX_EVENT_CYCLES: u32 = 40_000_000;

fn vol_a(b: &mut SystemBus) -> u8 {
    b.cart.as_ssc().unwrap().ay_read(ay_reg::VOL_A)
}

/// Load `[tone A at LOUD for `duration`, tone A at 0 for 0]` into buffer 0.
fn load_loud_then_silent(b: &mut SystemBus, duration: u8) {
    const LOUD: u8 = 0x0F;
    let tone = |amp, dur| [(group::TONE_A << group::OPCODE_SHIFT) | amp, 1, 100, dur];
    send(b, cmd::LOAD_SOUND_INDIVIDUAL_START);
    for byte in tone(LOUD, duration).into_iter().chain(tone(0, 0)) {
        send(b, byte);
    }
    send(b, terminator::SOUND);
}

/// Execute buffer 0 and measure cycles from the loud event's volume write
/// to the silent event's.
fn measure_loud_event(b: &mut SystemBus) -> u32 {
    send(b, cmd::EXEC_SOUND_INDIVIDUAL_START);
    let mut cycles = 0;
    while vol_a(b) != 0x0F {
        b.cart.tick(POLL_STEP);
        cycles += POLL_STEP;
        assert!(cycles < MAX_EVENT_CYCLES, "loud event never started");
    }
    let mut loud = 0;
    while vol_a(b) != 0 {
        b.cart.tick(POLL_STEP);
        loud += POLL_STEP;
        assert!(loud < MAX_EVENT_CYCLES, "loud event never ended");
    }
    loud
}

#[test]
fn event_duration_scales_with_the_duration_byte() {
    let Some(mut b) = try_coco3_bus_with_ssc() else {
        return skip("event_duration_scales_with_the_duration_byte");
    };
    load_loud_then_silent(&mut b, 10);
    let short = measure_loud_event(&mut b);
    load_loud_then_silent(&mut b, 30);
    let long = measure_loud_event(&mut b);
    let ratio = f64::from(long) / f64::from(short);
    assert!(
        (2.5..3.5).contains(&ratio),
        "duration 30 vs 10: {long} vs {short} cycles (ratio {ratio:.2})"
    );
}

#[test]
fn timer_base_scales_event_duration() {
    let Some(mut b) = try_coco3_bus_with_ssc() else {
        return skip("timer_base_scales_event_duration");
    };
    let mut measured = Vec::new();
    for base in [8u8, 16, 32] {
        send(&mut b, cmd::LOAD_TIMER_BASE);
        send(&mut b, base);
        load_loud_then_silent(&mut b, 10);
        measured.push((base, measure_loud_event(&mut b)));
    }
    for pair in measured.windows(2) {
        let (base_a, cycles_a) = pair[0];
        let (base_b, cycles_b) = pair[1];
        let expected = f64::from(base_b) / f64::from(base_a);
        let ratio = f64::from(cycles_b) / f64::from(cycles_a);
        assert!(
            (ratio - expected).abs() < expected * 0.15,
            "base {base_b} vs {base_a}: {cycles_b} vs {cycles_a} cycles (ratio {ratio:.2})"
        );
    }
}

/// MAME 0.289's coco3 + S/SC playing this same event (tone A, amplitude
/// 15, duration byte 100, power-on timer base) sounds for 7.36 s in a
/// `-wavwrite` capture; this cartridge measures 7.38 s from the AY's volume
/// register. Pinned to ±3 % so a timer or firmware-pacing regression shows.
#[test]
fn duration_100_at_the_power_on_base_matches_mame() {
    let Some(mut b) = try_coco3_bus_with_ssc() else {
        return skip("duration_100_at_the_power_on_base_matches_mame");
    };
    const MAME_SECONDS: f64 = 7.36;
    load_loud_then_silent(&mut b, 100);
    let seconds = f64::from(measure_loud_event(&mut b)) / E_CLOCK_HZ;
    assert!(
        (seconds - MAME_SECONDS).abs() < MAME_SECONDS * 0.03,
        "{seconds:.3} s vs MAME {MAME_SECONDS} s"
    );
}
