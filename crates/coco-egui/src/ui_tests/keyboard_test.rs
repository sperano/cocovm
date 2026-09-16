use egui_kittest::kittest::{NodeT, Queryable};

use super::harness::{AppHarness, boot_harness, click};
use crate::{CocoApp, KbMode, TYPE_HOLD_FIELDS, egui, kbd, typeahead};

const ALL_COLUMNS: u8 = 0x00;
const NO_KEYS_DOWN: u8 = 0xFF;

fn keyboard_harness(mode: KbMode) -> AppHarness {
    let mut harness = boot_harness();
    let app = harness.state_mut();
    // Advance taps explicitly so assertions do not depend on wall-clock time.
    app.running = false;
    app.set_mode(mode);
    app.show_kbd_help = true;
    harness.step();
    harness.step();
    harness
}

fn is_down(app: &CocoApp, pos: kbd::Pos) -> bool {
    app.machine.bus.keyboard.sense(!(1 << pos.1)) & (1 << pos.0) == 0
}

/// Advance a tap with the paused machine standing in for a ROM that scans
/// the keyboard every field.
fn advance_tap(app: &mut CocoApp) {
    app.type_ahead.advance(&mut app.machine.bus.keyboard);
    for _ in 0..typeahead::TAP_READS_TO_REGISTER {
        typeahead::tests::scan_matrix(&mut app.machine.bus.keyboard);
    }
}

#[test]
fn keyboard_click_reaches_matrix_and_releases_in_both_modes() {
    for mode in [KbMode::Positional, KbMode::Symbolic] {
        let mut harness = keyboard_harness(mode);
        click(&mut harness, "CTRL");
        click(&mut harness, "ALT");
        click(&mut harness, "A");
        harness.step();
        let app = harness.state_mut();
        assert!(app.type_ahead.is_active(), "click survives the focus gate");
        let (letter, _) = kbd::char_key('a').unwrap();
        for _ in 0..=TYPE_HOLD_FIELDS {
            advance_tap(app);
            assert!(is_down(app, letter));
            assert!(is_down(app, kbd::CTRL));
            assert!(is_down(app, kbd::ALT));
            assert!(!is_down(app, kbd::SHIFT));
        }
        advance_tap(app);
        assert_eq!(app.machine.bus.keyboard.sense(ALL_COLUMNS), NO_KEYS_DOWN);
    }
}

#[test]
fn keyboard_click_keeps_host_keyboard_available() {
    let mut harness = keyboard_harness(KbMode::Positional);
    click(&mut harness, "A");
    for _ in 0..typeahead::FIELDS_PER_TAP {
        advance_tap(harness.state_mut());
    }
    harness.key_down(egui::Key::B);
    harness.step();
    assert!(is_down(harness.state(), kbd::char_key('b').unwrap().0));
    harness.key_up(egui::Key::B);
    harness.step();
    assert_eq!(
        harness.state().machine.bus.keyboard.sense(ALL_COLUMNS),
        NO_KEYS_DOWN
    );
}

#[test]
fn hiding_keyboard_clears_modifier_latches() {
    let mut harness = keyboard_harness(KbMode::Positional);
    click(&mut harness, "CTRL");
    assert!(harness.state().keyboard_modifiers.ctrl);
    harness.key_press(egui::Key::F10);
    harness.step();
    assert!(!harness.state().show_kbd_help);
    assert_eq!(harness.state().keyboard_modifiers, Default::default());
    harness.key_press(egui::Key::F10);
    harness.step();
    click(&mut harness, "A");
    advance_tap(harness.state_mut());
    assert!(!is_down(harness.state(), kbd::CTRL));
}

#[test]
fn focus_loss_cancels_clicked_chord_and_modifier_latches() {
    let mut harness = keyboard_harness(KbMode::Positional);
    click(&mut harness, "ALT");
    click(&mut harness, "A");
    advance_tap(harness.state_mut());
    assert!(is_down(harness.state(), kbd::ALT));
    harness.input_mut().focused = false;
    harness.step();
    assert!(!harness.state().type_ahead.is_active());
    assert_eq!(harness.state().keyboard_modifiers, Default::default());
    assert_eq!(
        harness.state().machine.bus.keyboard.sense(ALL_COLUMNS),
        NO_KEYS_DOWN
    );
}

#[test]
fn remote_hold_disables_keyboard_clicks() {
    let mut harness = keyboard_harness(KbMode::Positional);
    harness.state_mut().remote_held = Some(crate::app::RemoteHold {
        keys: vec![kbd::ENTER],
        fields_left: u32::from(TYPE_HOLD_FIELDS),
    });
    harness.step();
    assert!(harness.get_by_label("A").accesskit_node().is_disabled());
}
