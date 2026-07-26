use super::*;

#[test]
fn civil_round_trips_across_leap_years() {
    for &(y, m, d) in &[
        (1970, 1, 1),
        (1999, 12, 31),
        (2000, 2, 29),
        (2024, 2, 29),
        (2026, 7, 8),
        (2100, 3, 1),
    ] {
        let days = days_from_civil(y, m, d);
        assert_eq!(civil_from_days(days), (y, m, d));
    }
    assert_eq!(days_from_civil(1970, 1, 1), 0);
}

#[test]
fn weekday_matches_known_dates() {
    // 1970-01-01 Thursday, 2026-07-08 Wednesday.
    let t = RTCTime {
        year: 1970,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    };
    assert_eq!(t.weekday(), 4);
    let t = RTCTime {
        year: 2026,
        month: 7,
        day: 8,
        hour: 0,
        minute: 0,
        second: 0,
    };
    assert_eq!(t.weekday(), 3);
}

#[test]
fn to_secs_from_secs_round_trip() {
    let t = RTCTime {
        year: 2026,
        month: 7,
        day: 8,
        hour: 21,
        minute: 34,
        second: 56,
    };
    assert_eq!(RTCTime::from_secs(t.to_secs()), t);
}
