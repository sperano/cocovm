//! The VM window's status-bar tape entry/menu: mounting, tracking the
//! mounted tape in the entry's label, seeking to a byte position, the
//! idle auto-finalize's per-frame auto-save, and the seek field's
//! keyboard-focus interactions with the CoCo matrix and a keys-mode
//! joystick. Split out of [`super::vm_window_menus`] to keep that file
//! under the project's line-count guideline.

use coco_core::cassette::test_support::{SPINUP_BURN_CYCLES, record_bytes_fsk, tape_block};
use coco_core::cassette::{Cassette, RECORD_IDLE_FINALIZE_CYCLES};
use egui_kittest::kittest::{NodeT, Queryable};

use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;

/// Leader byte (Service Manual §5.10, `cassette-verified-facts`) — a plain
/// unstructured tape used by the seek/typing tests that follow, which don't care
/// about block framing.
const LEADER: u8 = 0x55;

/// The status bar's tape entry is the menu button for the tape menu. With
/// nothing mounted the label is "No tape"; both icon and label halves open it.
#[test]
fn status_bar_tape_entry_opens_the_tape_menu() {
    let mut harness = boot_harness();

    click(&mut harness, "No tape");
    harness.get_by_label("Insert Tape…"); // the menu is open
    assert!(!harness.state().save_tape_wav);
    click(&mut harness, "Also save tape audio (.wav)");
    assert!(harness.state().save_tape_wav);
}

/// A mounted tape puts its file name and position in the label; the mount
/// goes through `new_tape` directly since the menu item opens a native file dialog.
#[test]
fn status_bar_tape_entry_tracks_the_mounted_tape() {
    let mut harness = boot_harness();
    let dir = TempDir::new("tape-entry");
    harness
        .state_mut()
        .new_tape(dir.path().join("untitled.cas"));
    harness.step();

    harness.get_by_label("untitled.cas [0/0]");
    click(&mut harness, "Tape menu");
    for label in ["Rewind Tape", "Eject Tape (untitled.cas)"] {
        assert!(
            !harness.get_by_label(label).accesskit_node().is_disabled(),
            "{label} should be enabled with a tape mounted"
        );
    }

    click(&mut harness, "Eject Tape (untitled.cas)");
    assert!(harness.state().tape_path.is_none());
    harness.get_by_label("No tape");
}

/// The idle auto-finalize (`Cassette::tick`) and the per-frame save hook
/// together mean a recording saves itself to disk with no eject or quit.
#[test]
fn finalized_recording_saves_to_disk_without_eject() {
    /// Past `RECORD_IDLE_FINALIZE_CYCLES` — crosses the auto-finalize
    /// threshold with a small margin.
    const IDLE_PAST_THRESHOLD: u32 = RECORD_IDLE_FINALIZE_CYCLES as u32 + 1;

    let mut harness = boot_harness();
    let dir = TempDir::new("tape-auto-save");
    let path = dir.path().join("untitled.cas");
    harness.state_mut().new_tape(path.clone());
    harness.step();

    let mut block = vec![LEADER; 16];
    block.extend(tape_block(0x01, b"X"));
    {
        let cassette: &mut Cassette = &mut harness.state_mut().machine.bus.cassette;
        cassette.tick(SPINUP_BURN_CYCLES, true); // burn spin-up
        record_bytes_fsk(cassette, &block);
        cassette.tick(IDLE_PAST_THRESHOLD, false); // core auto-finalizes here
    }

    harness.step(); // the frame hook runs and saves the landed finalize

    let (pos, len) = harness.state().machine.bus.cassette.position();
    assert_eq!(
        pos, len,
        "the head parks at the end of the spliced-in stretch"
    );
    harness.get_by_label(&format!("untitled.cas [{pos}/{len}]"));

    let saved = std::fs::read(&path).expect("the .cas file must exist on disk after auto-save");
    assert_eq!(
        saved,
        harness.state().machine.bus.cassette.tape_bytes(),
        "the saved file must match the finalized tape"
    );
    assert!(!saved.is_empty());
    assert!(
        !harness.state().machine.bus.cassette.dirty(),
        "the auto-save must have cleared the dirty flag"
    );
}

/// The tape menu's "Seek to byte" field moves the deck's head directly:
/// typing an offset and pressing Enter calls `Cassette::seek`.
#[test]
fn status_bar_tape_menu_seeks_to_a_byte_position() {
    let mut harness = boot_harness();
    let dir = TempDir::new("tape-seek");
    let path = dir.path().join("untitled.cas");
    std::fs::write(&path, vec![LEADER; 100]).unwrap();
    harness.state_mut().insert_tape(path);
    harness.step();

    harness.get_by_label("untitled.cas [0/100]");
    click(&mut harness, "Tape menu");

    // Guards the popup's close behavior: the menu-default CloseOnClick would dismiss it here.
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .click();
    harness.step();
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .type_text("50");
    harness.step();
    harness.key_press(egui::Key::Enter);
    harness.step();

    assert_eq!(harness.state().machine.bus.cassette.position().0, 50);
    harness.get_by_label("untitled.cas [50/100]");
}

/// Keystrokes belong to a focused text widget alone: `handle_input` must not
/// forward them to the CoCo matrix, and an already-held key must be released on focus.
#[test]
fn typing_in_the_seek_field_does_not_reach_the_coco_keyboard() {
    /// `Keyboard::sense` with every column strobed (active low) reads all
    /// rows high exactly when no key is pressed.
    const ALL_COLUMNS: u8 = 0x00;
    const NO_KEYS_DOWN: u8 = 0xFF;

    let mut harness = boot_harness();
    let dir = TempDir::new("tape-seek-leak");
    let path = dir.path().join("untitled.cas");
    std::fs::write(&path, vec![LEADER; 100]).unwrap();
    harness.state_mut().insert_tape(path);
    harness.step();

    // Precondition: with nothing focused, a held key lands on the matrix.
    // Otherwise, these assertions are vacuous.
    harness.key_down(egui::Key::A);
    harness.step();
    assert_ne!(
        harness.state().machine.bus.keyboard.sense(ALL_COLUMNS),
        NO_KEYS_DOWN,
        "sanity: an unfocused keypress must reach the CoCo matrix"
    );

    // Focus the seek field while the key is still held: the matrix must release it, not stick.
    click(&mut harness, "Tape menu");
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .click();
    harness.step();
    assert_eq!(
        harness.state().machine.bus.keyboard.sense(ALL_COLUMNS),
        NO_KEYS_DOWN,
        "focusing a text widget must release held matrix keys"
    );
    harness.key_up(egui::Key::A);
    harness.step();

    // Typing with the field focused stays out of the machine entirely.
    harness.key_down(egui::Key::Num5);
    harness.step();
    assert_eq!(
        harness.state().machine.bus.keyboard.sense(ALL_COLUMNS),
        NO_KEYS_DOWN,
        "keystrokes in the seek field must not reach the CoCo matrix"
    );
    harness.key_up(egui::Key::Num5);
    harness.step();

    // F-key hotkeys are NOT gated: handle_hotkeys runs before the focus gate.
    let mode_before = harness.state().kb_mode;
    harness.key_press(egui::Key::F12);
    harness.step();
    assert_ne!(
        harness.state().kb_mode,
        mode_before,
        "F-key hotkeys must stay live while a text widget is focused"
    );
    harness.key_press(egui::Key::F12);
    harness.step();

    #[cfg(feature = "debug-ui")]
    {
        // ⌘D is likewise not gated by focus: consume_app_shortcuts runs before this frame's
        // widgets.
        assert!(!harness.state().debugger.open);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
        harness.step();
        assert!(
            harness.state().debugger.open,
            "the debugger shortcut must stay live while a text widget is focused"
        );
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
        harness.step();
    }
}

/// The keys-mode joystick polls raw arrow/Z/X state outside `handle_input`,
/// so it needs its own focus gate against a focused text widget.
#[test]
fn typing_in_the_seek_field_does_not_move_a_keys_joystick() {
    let mut harness = boot_harness();
    harness.state_mut().joysticks.sources[coco_core::joystick::RIGHT] = joy::JoySource::Keys;
    let dir = TempDir::new("tape-seek-joy");
    let path = dir.path().join("untitled.cas");
    std::fs::write(&path, vec![LEADER; 100]).unwrap();
    harness.state_mut().insert_tape(path);
    harness.step();

    // At center (pot 32), a DAC level of 16 sits at-or-below the pot, so compare() reads true.
    const BELOW_CENTER_DAC: u8 = 16;
    let stick_centered = |harness: &AppHarness| {
        harness.state().machine.bus.joysticks.compare(
            coco_core::joystick::RIGHT,
            coco_core::joystick::AXIS_X,
            BELOW_CENTER_DAC,
        )
    };

    // Precondition: with nothing focused, a held arrow deflects the stick.
    harness.key_down(egui::Key::ArrowLeft);
    harness.step();
    assert!(
        !stick_centered(&harness),
        "sanity: keys-mode joystick must react to an unfocused arrow key"
    );
    harness.key_up(egui::Key::ArrowLeft);
    harness.step();

    click(&mut harness, "Tape menu");
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .click();
    harness.step();
    harness.key_down(egui::Key::ArrowLeft);
    harness.step();
    assert!(
        stick_centered(&harness),
        "arrows typed into the seek field must not nudge the emulated stick"
    );
    harness.key_up(egui::Key::ArrowLeft);
}
