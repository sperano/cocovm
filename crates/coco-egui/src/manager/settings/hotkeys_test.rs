use super::*;

fn key_event(key: egui::Key, pressed: bool, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers: egui::Modifiers::SHIFT,
    }
}

/// Releases and repeats are skipped; the first fresh press is removed
/// and returned with its modifiers, and nothing else is touched.
#[test]
fn take_first_press_takes_only_the_first_fresh_press() {
    let mut input = egui::InputState::default();
    input.events = vec![
        key_event(egui::Key::F8, false, false),
        key_event(egui::Key::F8, true, true),
        key_event(egui::Key::F9, true, false),
        key_event(egui::Key::F11, true, false),
    ];
    let press = |key| {
        Some(Press {
            key,
            physical_key: None,
            modifiers: egui::Modifiers::SHIFT,
        })
    };
    assert_eq!(take_first_press(&mut input), press(egui::Key::F9));
    assert_eq!(input.events.len(), 3);
    assert_eq!(take_first_press(&mut input), press(egui::Key::F11));
    assert_eq!(take_first_press(&mut input), None);
}
