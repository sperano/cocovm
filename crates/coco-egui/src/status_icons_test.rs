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
