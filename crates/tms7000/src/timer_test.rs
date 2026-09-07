use super::*;

/// The only state a live chip is ever in: freshly defaulted.
#[test]
fn default_validates() {
    assert!(Timer1::default().validate().is_ok());
}

/// `tick`'s loop always leaves `phase` below the period; a timer caught
/// mid-period (however it got there) must still validate.
#[test]
fn phase_below_period_validates() {
    let t = Timer1 {
        control: START_BIT, // prescaler 0: period = CYCLES_PER_TICK
        phase: CYCLES_PER_TICK - 1,
        ..Timer1::default()
    };
    assert!(t.validate().is_ok());
}

/// `tick` assumes `phase < period` on entry; at or past it, the `while`
/// loop would spin `phase / period` times instead of the 0 a live chip
/// ever needs.
#[test]
fn phase_at_or_past_period_is_rejected() {
    let at_period = Timer1 {
        control: START_BIT,
        phase: CYCLES_PER_TICK,
        ..Timer1::default()
    };
    assert!(at_period.validate().is_err());

    let huge = Timer1 {
        control: START_BIT,
        phase: u32::MAX,
        ..Timer1::default()
    };
    assert!(huge.validate().is_err());
}

/// A larger prescaler widens the period `phase` must stay under.
#[test]
fn phase_bound_scales_with_prescaler() {
    let max_prescaler = Timer1 {
        control: START_BIT | PRESCALER_MASK,
        phase: CYCLES_PER_TICK, // below period at prescaler 0, but not at max
        ..Timer1::default()
    };
    assert!(max_prescaler.validate().is_ok());
}

/// `write_control` always clears the cascade/halt bit; a deserialized
/// control byte with it set is not a state the hardware can reach.
#[test]
fn cascade_bit_set_is_rejected() {
    let t = Timer1 {
        control: START_BIT | CASCADE_OR_HALT_BIT,
        ..Timer1::default()
    };
    assert!(t.validate().is_err());
}
