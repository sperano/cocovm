//! Rebindable hotkeys (`hotkeys.rs`): capturing a new binding in the
//! Settings dialog (`manager/settings/hotkeys.rs`), including a quick-state
//! chord, and a VM window acting on the binding it was given instead of the
//! built-in F10/F12.

use egui_kittest::kittest::Queryable;

use crate::hotkeys::{DEFAULT_HOTKEYS, Hotkey};
use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;
use super::manager_settings::{open_vm_entry, settings_harness, settings_harness_with};

/// The binding button's text while it waits for a key.
const CAPTURE_PROMPT: &str = "Press a key…";

fn hotkey(text: &str) -> Hotkey {
    text.parse().expect("valid hotkey")
}

/// Opens Settings and starts capturing the Key layout window's hotkey,
/// whose button shows its default, F10.
fn start_key_layout_capture(harness: &mut ManagerHarness) {
    click(harness, "Settings");
    click(harness, "Hotkeys");
    click(harness, "F10");
    harness.get_by_label(CAPTURE_PROMPT);
}

fn press(harness: &mut ManagerHarness, modifiers: egui::Modifiers, key: egui::Key) {
    harness.key_press_modifiers(modifiers, key);
    harness.step();
}

/// The captured key becomes the binding; Save writes it to `config.toml`
/// and applies it to the manager.
#[test]
fn a_captured_hotkey_is_saved_and_applied() {
    let dir = TempDir::new("settings-hotkey-capture");
    let config_path = dir.path().join("config.toml");
    let mut harness = settings_harness(config_path.clone());

    start_key_layout_capture(&mut harness);
    press(&mut harness, egui::Modifiers::NONE, egui::Key::F9);
    harness.get_by_label("F9");
    click(&mut harness, "Save");

    assert!(harness.state().settings.is_none(), "Save closes the dialog");
    assert_eq!(harness.state().hotkeys.key_layout, hotkey("F9"));
    let saved = std::fs::read_to_string(&config_path).expect("save_file creates the file");
    assert!(
        saved
            .lines()
            .any(|l| l.trim() == "hotkey_key_layout = \"F9\""),
        "config.toml must contain the saved key: {saved}"
    );
}

/// Save also reaches VM windows that are already open.
#[test]
fn a_saved_hotkey_reaches_open_vm_windows() {
    let dir = TempDir::new("settings-hotkey-vms");
    let mut harness = settings_harness_with(dir.path().join("config.toml"), |app| {
        app.entries.push(open_vm_entry());
    });

    start_key_layout_capture(&mut harness);
    press(&mut harness, egui::Modifiers::NONE, egui::Key::F9);
    click(&mut harness, "Save");

    let vm = harness.state().entries[0]
        .vm
        .as_ref()
        .expect("VM stays open");
    assert_eq!(vm.hotkeys.key_layout, hotkey("F9"));
}

/// Esc ends the capture and keeps the old binding; it must not also close
/// the modal, which Esc otherwise dismisses.
#[test]
fn escape_cancels_a_capture_without_closing_the_dialog() {
    let dir = TempDir::new("settings-hotkey-escape");
    let mut harness = settings_harness(dir.path().join("config.toml"));

    start_key_layout_capture(&mut harness);
    press(&mut harness, egui::Modifiers::NONE, egui::Key::Escape);

    assert!(harness.state().settings.is_some(), "the dialog stays open");
    harness.get_by_label("F10");
    assert!(harness.query_by_label(CAPTURE_PROMPT).is_none());
}

/// A key another action already uses is refused, naming that action.
#[test]
fn a_hotkey_taken_by_another_action_is_refused() {
    let dir = TempDir::new("settings-hotkey-taken");
    let mut harness = settings_harness(dir.path().join("config.toml"));

    start_key_layout_capture(&mut harness);
    press(&mut harness, egui::Modifiers::NONE, egui::Key::F12);

    harness.get_by_label_contains("already the Keyboard mode hotkey");
    harness.get_by_label("F10");
}

/// A bare letter would type into the machine, so it is refused.
#[test]
fn a_typing_key_is_refused() {
    let dir = TempDir::new("settings-hotkey-typing");
    let mut harness = settings_harness(dir.path().join("config.toml"));

    start_key_layout_capture(&mut harness);
    press(&mut harness, egui::Modifiers::NONE, egui::Key::K);

    harness.get_by_label_contains("would type into the machine");
    harness.get_by_label("F10");
}

/// The manager's New machine hotkey must not fire while Settings is
/// capturing it: the press belongs to the capture.
#[test]
fn capturing_the_new_machine_hotkey_does_not_create_a_machine() {
    let dir = TempDir::new("settings-hotkey-new-machine");
    let mut harness = settings_harness(dir.path().join("config.toml"));
    let new_machine = harness
        .ctx
        .format_shortcut(&DEFAULT_HOTKEYS.new_machine.shortcut());

    click(&mut harness, "Settings");
    click(&mut harness, "Hotkeys");
    click(&mut harness, &new_machine);
    press(&mut harness, egui::Modifiers::COMMAND, egui::Key::N);

    assert!(harness.state().entries.is_empty(), "no machine was created");
    // Rebinding to the same chord is allowed and leaves it unchanged.
    harness.get_by_label(&new_machine);
}

/// The quick-state chords are hotkeys like the others: the Hotkeys tab has
/// a row per state and action, a captured chord is saved under the state's
/// `config.toml` key, and open VM windows act on it.
#[test]
fn a_captured_state_chord_is_saved_and_reaches_vm_windows() {
    let dir = TempDir::new("settings-hotkey-state-chord");
    let config_path = dir.path().join("config.toml");
    let mut harness = settings_harness_with(config_path.clone(), |app| {
        app.entries.push(open_vm_entry());
    });
    let load_state_4 = harness
        .ctx
        .format_shortcut(&DEFAULT_HOTKEYS.load_state[3].shortcut());
    let rebound = hotkey("Cmd+Alt+F5");
    let rebound_label = harness.ctx.format_shortcut(&rebound.shortcut());

    click(&mut harness, "Settings");
    click(&mut harness, "Hotkeys");
    for slot in 1..=save_state::QUICK_SLOTS {
        harness.get_by_label(&format!("Load State {slot}"));
        harness.get_by_label(&format!("Save State {slot}"));
    }
    click(&mut harness, &load_state_4);
    harness.get_by_label(CAPTURE_PROMPT);
    let cmd_alt = egui::Modifiers::COMMAND.plus(egui::Modifiers::ALT);
    press(&mut harness, cmd_alt, egui::Key::F5);
    harness.get_by_label(&rebound_label);
    click(&mut harness, "Save");

    assert!(harness.state().settings.is_none(), "Save closes the dialog");
    assert_eq!(harness.state().hotkeys.load_state[3], rebound);
    let vm = harness.state().entries[0]
        .vm
        .as_ref()
        .expect("VM stays open");
    assert_eq!(vm.hotkeys.load_state[3], rebound);
    let saved = std::fs::read_to_string(&config_path).expect("save_file creates the file");
    let expected = format!("hotkey_load_state_4 = \"{rebound}\"");
    assert!(
        saved.lines().any(|l| l.trim() == expected),
        "config.toml must contain {expected}: {saved}"
    );
}

/// A VM window acts on the bindings the manager hands it: F10/F12 are inert
/// once rebound, and the match is exact (Shift+F9 is not F9).
#[test]
fn a_vm_window_follows_rebound_hotkeys() {
    let mut harness = boot_harness();
    harness.state_mut().hotkeys.key_layout = hotkey("F9");
    harness.state_mut().hotkeys.keyboard_mode = hotkey("Shift+F9");

    harness.key_press(egui::Key::F10);
    harness.key_press(egui::Key::F12);
    harness.step();
    assert!(!harness.state().show_kbd_help, "F10 is no longer bound");
    assert!(
        harness.state().kb_mode == KbMode::Positional,
        "F12 is no longer bound"
    );

    harness.key_press(egui::Key::F9);
    harness.step();
    assert!(
        harness.state().show_kbd_help,
        "F9 opens the key layout window"
    );

    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F9);
    harness.step();
    assert!(
        harness.state().kb_mode == KbMode::Symbolic,
        "Shift+F9 switches mode"
    );
    assert!(harness.state().show_kbd_help, "Shift+F9 is not F9");
    harness.key_press(egui::Key::F9);
    harness.step();
    assert!(!harness.state().show_kbd_help, "F9 closes it again");

    // The keyboard menu names the current binding.
    click(&mut harness, "Symbolic");
    harness.get_by_label("Key layout (F9)");
}

#[test]
fn leaving_the_hotkeys_tab_cancels_capture() {
    let dir = TempDir::new("settings-hotkey-tab-switch");
    let mut harness = settings_harness(dir.path().join("config.toml"));
    start_key_layout_capture(&mut harness);
    click(&mut harness, "General");
    press(&mut harness, egui::Modifiers::NONE, egui::Key::F9);
    click(&mut harness, "Hotkeys");
    harness.get_by_label("F10");
    assert!(harness.query_by_label(CAPTURE_PROMPT).is_none());
    press(&mut harness, egui::Modifiers::NONE, egui::Key::Escape);
    assert!(harness.state().settings.is_none());
}
