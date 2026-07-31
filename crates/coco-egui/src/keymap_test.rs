use super::*;

/// The traditional CLEAR binding and the laptop alias must both land on the
/// same matrix position — the alias exists because laptop keyboards have no
/// Home key, not to move CLEAR somewhere new.
#[test]
fn clear_answers_to_home_and_to_backquote() {
    assert_eq!(key_to_pos(egui::Key::Home), Some(kbd::CLEAR));
    assert_eq!(key_to_pos(egui::Key::Backtick), Some(kbd::CLEAR));
    assert_eq!(control_key_pos(egui::Key::Home), Some(kbd::CLEAR));
    assert_eq!(control_key_pos(egui::Key::Backtick), Some(kbd::CLEAR));
}

/// Positional mode maps by *position*, so the host keys next to `0` drive the
/// CoCo's `:` and `-` rather than the host's own punctuation — the single
/// most surprising thing the help window has to teach.
#[test]
fn punctuation_maps_by_position_not_by_character() {
    assert_eq!(key_to_pos(egui::Key::Minus), Some((5, 2)));
    assert_eq!(key_to_pos(egui::Key::Equals), Some((5, 5)));
    assert_eq!(key_to_pos(egui::Key::OpenBracket), Some(kbd::AT));
}
