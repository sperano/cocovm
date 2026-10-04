use super::*;

#[test]
fn stick_index_maps_to_the_coco_core_joystick_ports() {
    assert_eq!(Stick::Left.index(), coco_core::joystick::LEFT);
    assert_eq!(Stick::Right.index(), coco_core::joystick::RIGHT);
}

#[test]
fn vm_status_as_str_is_the_snake_case_wire_spelling() {
    assert_eq!(VmStatus::Running.as_str(), "running");
    assert_eq!(VmStatus::Suspended.as_str(), "suspended");
    assert_eq!(VmStatus::PoweredOff.as_str(), "powered_off");
}

fn snapshot(lines: &[&str]) -> ScreenSnapshot {
    ScreenSnapshot {
        lines: lines.iter().map(|line| (*line).to_string()).collect(),
        mode: "test".to_string(),
        cursor: None,
    }
}

#[test]
fn text_matchers_search_the_newline_joined_screen() {
    let screen = snapshot(&["LOADING", "OK"]);
    let literal = TextMatcher::new("LOADING\nOK".to_string(), false).unwrap();
    let regex = TextMatcher::new(r"(?m)^OK$".to_string(), true).unwrap();

    assert!(literal.is_match(&screen));
    assert!(regex.is_match(&screen));
}

#[test]
fn text_matcher_rejects_invalid_regex_and_overlong_patterns() {
    assert!(TextMatcher::new("(".to_string(), true).is_err());
    let overlong = "x".repeat(crate::control::MAX_WAIT_PATTERN_CHARS + 1);
    assert!(TextMatcher::new(overlong, false).is_err());
}
