use super::*;

#[test]
fn measurement_duration_rejects_values_that_prevent_bounded_exit() {
    for value in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -1.0,
        0.0,
        MAX_DURATION_SECS + 1.0,
    ] {
        assert!(checked_duration("duration", value).is_err());
    }
    assert_eq!(
        checked_duration("duration", MAX_DURATION_SECS)
            .unwrap()
            .as_secs_f64(),
        MAX_DURATION_SECS
    );
}
