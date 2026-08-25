use super::*;

/// The traditional CLEAR binding and the laptop alias must land on the same
/// matrix position — the alias exists because laptops often lack Home.
#[test]
fn clear_answers_to_home_and_to_backquote() {
    assert_eq!(key_to_pos(egui::Key::Home), Some(kbd::CLEAR));
    assert_eq!(key_to_pos(egui::Key::Backtick), Some(kbd::CLEAR));
    assert_eq!(control_key_pos(egui::Key::Home), Some(kbd::CLEAR));
    assert_eq!(control_key_pos(egui::Key::Backtick), Some(kbd::CLEAR));
}

/// Positional mode maps by *position*: the host keys next to `0` drive the
/// CoCo's `:`/`-`, not the host's own punctuation.
#[test]
fn punctuation_maps_by_position_not_by_character() {
    assert_eq!(key_to_pos(egui::Key::Minus), Some((5, 2))); // CoCo ':'
    assert_eq!(key_to_pos(egui::Key::Equals), Some((5, 5))); // CoCo '-'
    assert_eq!(key_to_pos(egui::Key::OpenBracket), Some(kbd::AT));
}
