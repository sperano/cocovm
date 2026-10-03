use super::*;

/// egui's report of the platform's primary modifier: ⌘ on macOS, Ctrl elsewhere.
fn primary() -> egui::Modifiers {
    if SEPARATE_CTRL {
        egui::Modifiers::MAC_CMD.plus(egui::Modifiers::COMMAND)
    } else {
        egui::Modifiers::CTRL.plus(egui::Modifiers::COMMAND)
    }
}

fn press(key: egui::Key, modifiers: egui::Modifiers, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat,
        modifiers,
    }
}

fn input_with(events: Vec<egui::Event>) -> egui::InputState {
    let mut input = egui::InputState::default();
    input.events = events;
    input
}

#[test]
fn bare_function_key_parses_and_round_trips() {
    let hotkey: Hotkey = "F9".parse().expect("F9 is a valid hotkey");
    assert_eq!(hotkey, Hotkey::new(egui::Modifiers::NONE, egui::Key::F9));
    assert_eq!(hotkey.to_string(), "F9");
}

#[test]
fn names_are_case_insensitive_and_spaces_are_ignored() {
    let hotkey: Hotkey = "cmd + shift + k".parse().expect("valid hotkey");
    assert_eq!(
        hotkey,
        Hotkey::new(
            egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
            egui::Key::K
        )
    );
}

#[test]
fn every_default_round_trips_through_its_config_spelling() {
    for action in [
        HotkeyAction::KeyLayout,
        HotkeyAction::KeyboardMode,
        HotkeyAction::NewMachine,
        HotkeyAction::Debugger,
    ] {
        let hotkey = DEFAULT_HOTKEYS.get(action);
        assert_eq!(
            hotkey.to_string().parse::<Hotkey>(),
            Ok(hotkey),
            "{action:?}"
        );
    }
}

#[test]
fn command_is_written_the_platform_way() {
    let expected = if SEPARATE_CTRL { "Cmd+N" } else { "Ctrl+N" };
    assert_eq!(DEFAULT_HOTKEYS.new_machine.to_string(), expected);
}

/// On Windows/Linux the Control key *is* the primary modifier, so `Ctrl`
/// in the file must fire on the same press as `Cmd`.
#[test]
fn ctrl_means_the_control_key() {
    let ctrl: Hotkey = "Ctrl+K".parse().expect("valid hotkey");
    let cmd: Hotkey = "Cmd+K".parse().expect("valid hotkey");
    assert_eq!(ctrl == cmd, !SEPARATE_CTRL);
}

#[test]
fn option_is_an_alias_of_alt() {
    assert_eq!("Option+F5".parse::<Hotkey>(), "Alt+F5".parse::<Hotkey>());
}

#[test]
fn unknown_key_or_modifier_is_an_error() {
    let err = "F99".parse::<Hotkey>().expect_err("no such key");
    assert!(err.contains("unknown key"), "{err}");
    let err = "Hyper+F5".parse::<Hotkey>().expect_err("no such modifier");
    assert!(err.contains("unknown modifier"), "{err}");
}

#[test]
fn a_repeated_modifier_counts_once() {
    assert_eq!("Cmd+Command+K".parse::<Hotkey>(), "Cmd+K".parse::<Hotkey>());
}

#[test]
fn bare_typing_key_is_refused() {
    for text in [
        "K",
        "Shift+K",
        "Alt+K",
        "Option+Shift+K",
        "Escape",
        "Enter",
        "Space",
    ] {
        let err = text.parse::<Hotkey>().expect_err(text);
        assert!(err.contains("would type into the machine"), "{text}: {err}");
    }
}

/// F1/F2 are CoCo 3 keys (`keymap::key_to_pos`), so a bare F1 hotkey would
/// steal them; with a modifier they're fine.
#[test]
fn coco_function_keys_need_a_modifier() {
    assert!("F1".parse::<Hotkey>().is_err());
    assert!("F2".parse::<Hotkey>().is_err());
    assert!(
        "Alt+F1".parse::<Hotkey>().is_err(),
        "Alt is still CoCo ALT+F1"
    );
    assert!("Cmd+F1".parse::<Hotkey>().is_ok());
}

#[test]
fn reserved_shortcuts_are_refused() {
    for text in ["Cmd+1", "Cmd+Shift+3", "Cmd+A", "Cmd+V", "Cmd+C", "Cmd+X"] {
        let err = text.parse::<Hotkey>().expect_err(text);
        assert!(err.contains("built-in shortcut"), "{text}: {err}");
    }
    // A different chord on the same key is free.
    assert!("Cmd+Alt+1".parse::<Hotkey>().is_ok());
}

/// A build without the debugger still keeps its binding out of other
/// hands, so the file it saves loads in a `debug-ui` build.
#[test]
fn the_debugger_binding_is_held_in_every_build() {
    assert_eq!(
        DEFAULT_HOTKEYS.holder(DEFAULT_HOTKEYS.debugger, HotkeyAction::NewMachine),
        Some(HotkeyAction::Debugger)
    );
}

#[test]
fn matches_the_platform_primary_modifier_exactly() {
    let new_machine = DEFAULT_HOTKEYS.new_machine;
    assert!(new_machine.matches(egui::Key::N, primary()));
    assert!(!new_machine.matches(egui::Key::N, egui::Modifiers::NONE));
    assert!(
        !new_machine.matches(egui::Key::N, primary().plus(egui::Modifiers::SHIFT)),
        "an extra Shift is a different hotkey"
    );
    assert!(
        !DEFAULT_HOTKEYS
            .key_layout
            .matches(egui::Key::F10, egui::Modifiers::SHIFT)
    );
}

#[test]
fn from_press_normalizes_the_primary_modifier() {
    let hotkey = Hotkey::from_press(egui::Key::K, primary()).expect("valid hotkey");
    assert_eq!(hotkey, "Cmd+K".parse().expect("valid hotkey"));
}

#[test]
fn from_press_validates() {
    assert!(Hotkey::from_press(egui::Key::K, egui::Modifiers::NONE).is_err());
}

#[test]
fn consume_takes_every_press_but_fires_once() {
    let f10 = DEFAULT_HOTKEYS.key_layout;
    let mut input = input_with(vec![
        press(egui::Key::F10, egui::Modifiers::NONE, false),
        press(egui::Key::F10, egui::Modifiers::NONE, true),
        press(egui::Key::F11, egui::Modifiers::NONE, false),
    ]);
    assert!(f10.consume(&mut input));
    assert_eq!(input.events.len(), 1, "only the F11 press is left");
    assert!(!f10.consume(&mut input));
}

#[test]
fn consume_swallows_a_held_repeat_without_firing() {
    let f10 = DEFAULT_HOTKEYS.key_layout;
    let mut input = input_with(vec![press(egui::Key::F10, egui::Modifiers::NONE, true)]);
    assert!(!f10.consume(&mut input));
    assert!(
        input.events.is_empty(),
        "the repeat must not reach the machine"
    );
}

#[test]
fn defaults_are_distinct() {
    assert_eq!(DEFAULT_HOTKEYS.check_distinct(), Ok(()));
}

#[test]
fn a_shared_binding_is_reported() {
    let mut hotkeys = DEFAULT_HOTKEYS;
    hotkeys.set(HotkeyAction::KeyLayout, DEFAULT_HOTKEYS.keyboard_mode);
    let err = hotkeys.check_distinct().expect_err("two actions on F12");
    assert!(
        err.contains("Key layout window") && err.contains("Keyboard mode"),
        "{err}"
    );
    assert_eq!(
        hotkeys.holder(DEFAULT_HOTKEYS.keyboard_mode, HotkeyAction::KeyboardMode),
        Some(HotkeyAction::KeyLayout)
    );
}
