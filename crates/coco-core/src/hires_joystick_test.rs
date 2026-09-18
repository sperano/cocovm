use super::*;
use crate::joystick::{AXIS_X, AXIS_Y, POT_MAX};

fn tandy() -> HiResPort {
    let mut p = HiResPort::default();
    p.set_kind(HiResInterface::Tandy);
    p
}

#[test]
fn duration_cycles_endpoints_and_is_monotonic() {
    // 560us at pot 0, (4856+560)us at pot POT_MAX, converted at CPU_HZ — see this
    // module's doc for the MAME ctor these figures are cited from.
    let at_min = duration_cycles(HiResInterface::Tandy, 0);
    let at_max = duration_cycles(HiResInterface::Tandy, POT_MAX);
    assert_eq!(at_min, 501);
    assert_eq!(at_max, 4847);

    let mut prev = at_min;
    for pot in [1, 100, 500, 900, POT_MAX - 1] {
        let d = duration_cycles(HiResInterface::Tandy, pot);
        assert!(d > prev, "duration must strictly increase with pot ({pot})");
        prev = d;
    }
    assert!(prev < at_max);
}

#[test]
fn none_kind_never_saturates() {
    let mut p = HiResPort::default();
    p.observe(true, AXIS_X, POT_MAX);
    p.tick(1_000_000);
    assert!(!p.comparator(AXIS_X, 0));
    assert!(!p.comparator(AXIS_Y, 0));
}

#[test]
fn high_to_low_arms_the_selected_axis() {
    let mut p = tandy();
    let duration = duration_cycles(HiResInterface::Tandy, 0);
    p.observe(true, AXIS_X, 0); // was_low starts false: high -> low.
    p.tick(duration - 1);
    assert!(!p.comparator(AXIS_X, 0), "must not saturate early");
    p.tick(1);
    assert!(
        p.comparator(AXIS_X, 0),
        "must saturate exactly at the duration boundary"
    );
}

#[test]
fn expiry_boundary_is_exact() {
    let mut p = tandy();
    let duration = duration_cycles(HiResInterface::Tandy, 300);
    p.observe(true, AXIS_Y, 300);
    p.tick(duration - 1);
    assert!(!p.comparator(AXIS_Y, 0));
    p.tick(1);
    assert!(p.comparator(AXIS_Y, 0));
}

#[test]
fn high_cancels_the_timer_and_clears_only_the_selected_axis() {
    let mut p = tandy();
    let duration = duration_cycles(HiResInterface::Tandy, 0);
    p.observe(true, AXIS_X, 0);
    p.tick(duration);
    assert!(p.comparator(AXIS_X, 0), "saturated before the high edge");

    p.observe(false, AXIS_X, 0); // high: cancels, clears AXIS_X's slot only.
    assert!(!p.comparator(AXIS_X, 0));

    // Ticking further must not resurrect saturation — the timer was cancelled, not paused.
    p.tick(duration * 10);
    assert!(!p.comparator(AXIS_X, 0));
}

#[test]
fn low_to_low_mid_charge_reschedules_against_the_new_axis() {
    let mut p = tandy();
    p.observe(true, AXIS_X, POT_MAX);
    p.tick(100); // still well short of `long_duration`.

    // The mux swaps to a dormant axis mid-charge, DAC still low: a low -> low call with a much
    // shorter duration, which must reschedule (not restart) against the 100 cycles already spent.
    let short_duration = duration_cycles(HiResInterface::Tandy, 0);
    assert!(
        100 < short_duration,
        "test needs the reschedule branch, not a fresh-charge one"
    );
    p.observe(true, AXIS_Y, 0);

    p.tick(short_duration - 100 - 1);
    assert!(!p.comparator(AXIS_Y, 0));
    p.tick(1);
    assert!(p.comparator(AXIS_Y, 0));
}

#[test]
fn low_to_low_after_expiry_rearms_fresh_and_leaves_the_old_axis_saturated() {
    let mut p = tandy();
    let duration = duration_cycles(HiResInterface::Tandy, 0);
    p.observe(true, AXIS_X, 0);
    p.tick(duration);
    assert!(p.comparator(AXIS_X, 0), "AXIS_X saturated");

    // DAC stays low, but the mux moves to AXIS_Y after the prior charge already fully elapsed:
    // a fresh charge on AXIS_Y, leaving AXIS_X's slot untouched (only a high edge clears it).
    p.observe(true, AXIS_Y, 0);
    assert!(!p.comparator(AXIS_Y, 0), "AXIS_Y starts charging from zero");
    assert!(
        p.comparator(AXIS_X, 0),
        "AXIS_X's slot survives the axis switch"
    );

    p.tick(duration - 1);
    assert!(!p.comparator(AXIS_Y, 0));
    p.tick(1);
    assert!(p.comparator(AXIS_Y, 0));
}

#[test]
fn dac_63_reads_low_even_while_saturated() {
    let mut p = tandy();
    let duration = duration_cycles(HiResInterface::Tandy, 0);
    p.observe(true, AXIS_X, 0);
    p.tick(duration);
    assert!(p.comparator(AXIS_X, 0));
    assert!(!p.comparator(AXIS_X, 63));
}
