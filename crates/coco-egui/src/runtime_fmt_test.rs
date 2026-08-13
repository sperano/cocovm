use super::*;

#[test]
fn zero_seconds() {
    assert_eq!(humanize_runtime(0), "0 s");
}

#[test]
fn under_a_minute() {
    assert_eq!(humanize_runtime(42), "42 s");
}

#[test]
fn exactly_one_minute() {
    assert_eq!(humanize_runtime(60), "1 m 0 s");
}

#[test]
fn minutes_and_seconds() {
    assert_eq!(humanize_runtime(12 * 60 + 5), "12 m 5 s");
}

#[test]
fn exactly_one_hour() {
    assert_eq!(humanize_runtime(3600), "1 h 0 m");
}

#[test]
fn hours_and_minutes() {
    assert_eq!(humanize_runtime(3 * 3600 + 12 * 60), "3 h 12 m");
}

#[test]
fn exactly_one_day() {
    assert_eq!(humanize_runtime(86_400), "1 d 0 h");
}

#[test]
fn days_and_hours() {
    assert_eq!(humanize_runtime(2 * 86_400 + 3 * 3600), "2 d 3 h");
}
