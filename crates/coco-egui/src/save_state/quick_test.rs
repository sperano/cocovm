use chrono::TimeZone;
use eframe::egui;

use super::quick::*;
use super::{empty_state_hover, with_notes};
use crate::machine_def::tests::TempDir;

/// A fixed "now" for the label tests: 2026-10-07 22:45 local time.
fn now() -> chrono::DateTime<chrono::Local> {
    chrono::Local
        .with_ymd_and_hms(2026, 10, 7, 22, 45, 0)
        .single()
        .expect("unambiguous local time")
}

/// `now()` shifted by `minutes`, as the `SystemTime` a file's mtime gives.
fn minutes_from_now(minutes: i64) -> std::time::SystemTime {
    (now() + chrono::Duration::minutes(minutes)).into()
}

/// Five states named State 1 to State 5, none called a slot.
#[test]
fn there_are_five_states_named_state_1_to_5() {
    assert_eq!(QUICK_SLOTS, 5);
    let names: Vec<String> = (0..QUICK_SLOTS).map(state_name).collect();
    assert_eq!(names.first().map(String::as_str), Some("State 1"));
    assert_eq!(names.last().map(String::as_str), Some("State 5"));
    for (slot, name) in names.iter().enumerate() {
        assert_eq!(*name, format!("State {}", slot + 1));
    }
}

/// Rows read "State N — saved HH:MM" for today's saves, add the date for
/// older ones, and say Empty or "timestamp unavailable" otherwise.
#[test]
fn state_row_labels_describe_each_state() {
    let today = state_row_label(0, StateFile::Saved(Some(minutes_from_now(-5))), now());
    assert_eq!(today, "State 1 — saved 22:40");
    let older = state_row_label(
        4,
        StateFile::Saved(Some(minutes_from_now(-2 * 24 * 60))),
        now(),
    );
    assert_eq!(older, "State 5 — saved 2026-10-05 22:45");
    assert_eq!(
        state_row_label(1, StateFile::Empty, now()),
        "State 2 — Empty"
    );
    assert_eq!(
        state_row_label(2, StateFile::Saved(None), now()),
        "State 3 — timestamp unavailable"
    );
}

/// The keyboard-help line names every state's load chord, then every
/// save chord, as the current bindings spell them.
#[test]
fn the_shortcuts_hint_names_every_state_chord() {
    let ctx = egui::Context::default();
    let mut hotkeys = crate::hotkeys::DEFAULT_HOTKEYS;
    hotkeys.load_state[4] = "Cmd+Alt+F5".parse().expect("valid hotkey");
    let hint = slot_shortcuts_hint(&ctx, &hotkeys);
    let name = |hotkey: crate::hotkeys::Hotkey| ctx.format_shortcut(&hotkey.shortcut());
    let loads: Vec<String> = hotkeys.load_state.iter().copied().map(name).collect();
    let saves: Vec<String> = hotkeys.save_state.iter().copied().map(name).collect();
    assert_eq!(
        hint,
        format!(
            "{}: quick-load State 1 to 5   ·   {}: quick-save",
            loads.join(" / "),
            saves.join(" / ")
        )
    );
    assert!(hint.contains(&name(hotkeys.load_state[4])), "{hint}");
}

/// Only a missing file is empty: any file there, even an unloadable one,
/// counts as saved so Load stays enabled and reports the failure.
#[test]
fn probe_treats_only_a_missing_file_as_empty() {
    let dir = TempDir::new("quick-state-probe");
    let path = dir.path().join("slot-1.ccstate");
    assert_eq!(StateFile::probe(&path), StateFile::Empty);

    std::fs::write(&path, b"not a state").expect("write corrupt fixture");
    assert!(matches!(StateFile::probe(&path), StateFile::Saved(Some(_))));
    assert!(!StateFile::probe(&path).is_empty());

    std::fs::remove_file(&path).expect("remove fixture");
    assert!(StateFile::probe(&path).is_empty());
}

/// The empty-state texts name the state.
#[test]
fn empty_state_messages_name_the_state() {
    assert_eq!(empty_state_toast(3), "State 4 is empty");
    assert_eq!(
        empty_state_hover(0),
        "State 1 is empty. Save a state first."
    );
}

/// Restore notes follow the toast head after a colon.
#[test]
fn with_notes_appends_restore_notes() {
    assert_eq!(with_notes("Loaded State 2", &[]), "Loaded State 2");
    let notes = ["a".to_string(), "b".to_string()];
    assert_eq!(with_notes("Loaded State 2", &notes), "Loaded State 2: a; b");
}
