use std::time::{Duration, Instant};

use super::*;

/// A fixed base instant plus an offset, since [`Instant::now`] itself can't
/// be constructed with a chosen value.
fn at(base: Instant, millis: u64) -> Instant {
    base + Duration::from_millis(millis)
}

#[test]
fn priming_does_not_light() {
    let mut latch = ActivityLatch::default();
    let base = Instant::now();
    // Even a large nonzero first value (e.g. a save state restored with
    // tx_bytes already at 40000) must not flash on the first observation.
    assert!(!latch.observe_at(40_000, base));
}

#[test]
fn a_change_lights_the_latch() {
    let mut latch = ActivityLatch::default();
    let base = Instant::now();
    latch.observe_at(10, base);
    assert!(latch.observe_at(11, at(base, 1)));
}

#[test]
fn hold_expires_after_activity_hold() {
    let mut latch = ActivityLatch::default();
    let base = Instant::now();
    latch.observe_at(10, base);
    latch.observe_at(11, at(base, 1));
    // Just before the hold elapses: still lit.
    assert!(latch.observe_at(11, at(base, 1) + ACTIVITY_HOLD - Duration::from_millis(1)));
    // At/after the hold: idle again.
    assert!(!latch.observe_at(11, at(base, 1) + ACTIVITY_HOLD));
}

#[test]
fn a_decrease_also_lights_the_latch() {
    let mut latch = ActivityLatch::default();
    let base = Instant::now();
    latch.observe_at(100, base);
    // A rewound counter (e.g. after loading an older save state) still
    // counts as a change and blips the light once.
    assert!(latch.observe_at(40, at(base, 1)));
}

#[test]
fn same_value_reobserved_within_hold_stays_lit() {
    let mut latch = ActivityLatch::default();
    let base = Instant::now();
    latch.observe_at(10, base);
    latch.observe_at(11, at(base, 1));
    // Re-observing the same value doesn't reset the hold window, but
    // doesn't cut it short either.
    assert!(latch.observe_at(11, at(base, 50)));
}

#[test]
fn a_second_change_before_the_first_hold_expires_resets_the_hold_from_itself() {
    let mut latch = ActivityLatch::default();
    let base = Instant::now();
    latch.observe_at(10, base);
    latch.observe_at(11, at(base, 1)); // first change; hold alone would expire at t=1+HOLD
    latch.observe_at(12, at(base, 50)); // second change, well inside that hold window
    // Past when the FIRST change's hold alone would have expired, but still
    // within the hold re-extended by the second change: still lit. This is
    // the only way to prove the timer reset from the second change rather
    // than just outlasting the first.
    assert!(latch.observe_at(12, at(base, 1) + ACTIVITY_HOLD));
    // Past the second change's own hold window: idle again.
    assert!(!latch.observe_at(12, at(base, 50) + ACTIVITY_HOLD));
}

const TAU: f32 = std::f32::consts::TAU;

#[test]
fn reel_advances_forward_with_playback_position() {
    let mut reel = TapeReel {
        last_pos: 100,
        ..Default::default()
    };
    let angle = reel.advance(110, true, 0.0);
    assert!((angle - 10.0 * REEL_ANGLE_PER_BYTE).abs() < 1e-6);
}

#[test]
fn reel_forward_accumulation_past_tau_wraps_via_rem_euclid() {
    // 45 bytes at REEL_ANGLE_PER_BYTE (TAU/40) is more than one full turn —
    // the forward-wrap counterpart to the rewind case below.
    let mut reel = TapeReel::default();
    let raw = 45.0 * REEL_ANGLE_PER_BYTE;
    assert!(
        raw > TAU,
        "test is only meaningful if the raw angle actually exceeds TAU"
    );
    let angle = reel.advance(45, true, 0.0);
    assert!((angle - raw.rem_euclid(TAU)).abs() < 1e-6);
}

#[test]
fn reel_spins_backward_on_rewind() {
    let mut reel = TapeReel {
        last_pos: 110,
        ..Default::default()
    };
    let angle = reel.advance(100, true, 0.0);
    // Rewinding 10 bytes must turn the reel the opposite way, wrapped into
    // 0..TAU (a bare negative angle would be a bug: the icon compares raw
    // radians, and comparisons must stay well-defined across a rewind).
    let expected = (-10.0 * REEL_ANGLE_PER_BYTE).rem_euclid(TAU);
    assert!((angle - expected).abs() < 1e-6);
}

#[test]
fn reel_keeps_turning_while_parked_with_motor_running() {
    // Position didn't move (spin-up, or MOTOR ON with the tape parked) but
    // the motor's on: the reel still turns, at MOTOR_REEL_SPEED.
    let mut reel = TapeReel {
        last_pos: 50,
        ..Default::default()
    };
    let angle = reel.advance(50, true, 0.5);
    assert!((angle - MOTOR_REEL_SPEED * 0.5).abs() < 1e-6);
}

#[test]
fn reel_parks_when_motor_is_off_and_position_is_unchanged() {
    let mut reel = TapeReel {
        angle: 1.23,
        last_pos: 50,
    };
    let angle = reel.advance(50, false, 0.5);
    assert_eq!(angle, 1.23);
}
