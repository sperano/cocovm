//! The State selector's Save to File… and Load from File… items
//! (`save_state/file.rs`), and the machine-type prompt in front of every
//! user-requested load (`save_state/load_request.rs`). The native file
//! dialogs can't run headless, so the file tests call what the dialogs
//! hand their path to. Every test keeps its states in a scratch directory.

use std::path::{Path, PathBuf};

use egui_kittest::kittest::{NodeT, Queryable};

use coco_core::{MachineVariant, MemorySize, VDGVariant, VideoStandard};

use crate::chrome::toolbar::quick_states::{LOAD_FROM_FILE, SAVE_TO_FILE};
use crate::machine_def::tests::TempDir;
use crate::rom_load::load_default_rom;
use crate::save_state::{CANCEL, LOAD_ANYWAY, LoadSource};
use crate::*;

use super::harness::*;

/// Heading of the prompt for a CoCo 2 state loading into the CoCo 3 window.
const COCO2_PROMPT: &str = "Load a CoCo 2 state?";

/// [`boot_harness`] (a CoCo 3) with quick states kept in `dir`.
fn harness_with_states(dir: &TempDir) -> AppHarness {
    let mut harness = boot_harness();
    harness.state_mut().quick_state_dir = Some(dir.path().to_path_buf());
    harness.step();
    harness
}

/// File of State `n` (1-based) in `dir`.
fn state_path(dir: &TempDir, n: usize) -> PathBuf {
    dir.path().join(format!("slot-{n}.ccstate"))
}

/// Save a freshly booted CoCo 2's state to `path`.
fn save_coco2_state(path: &Path) {
    let config = MachineConfig {
        variant: MachineVariant::Coco2,
        video: VideoStandard::NTSC,
        memory: MemorySize::K64,
        monitor: None,
        vdg: Some(VDGVariant::MC6847T1),
    };
    let (rom, rom_source) = load_default_rom(config.variant, &test_assets::roms_dir())
        .expect("the CoCo 2 ROMs are required in the cocovm XDG data directory");
    let mut app = CocoApp::new(
        config,
        rom,
        rom_source,
        AppParams::default(),
        crate::joy::SharedGamepad::without_backend(),
    );
    app.save_state_to(path)
        .unwrap_or_else(|e| panic!("save_state_to failed: {e}"));
}

fn variant(harness: &AppHarness) -> MachineVariant {
    harness.state().machine.config.variant
}

fn toast(harness: &mut AppHarness) -> Option<String> {
    harness.state_mut().toast_message()
}

fn is_disabled(harness: &AppHarness, label: &str) -> bool {
    harness.get_by_label(label).accesskit_node().is_disabled()
}

/// Press COMMAND+2, the load chord for State 2.
fn load_state_2_chord(harness: &mut AppHarness) {
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num2);
    harness.step();
}

/// The selector's list ends with both file items, disabled while suspended
/// like the tiles.
#[test]
fn the_selector_offers_the_file_items() {
    let dir = TempDir::new("state-file-items");
    let mut harness = harness_with_states(&dir);

    open_combo_at(&mut harness, "State 1", 0);
    assert!(!is_disabled(&harness, SAVE_TO_FILE));
    assert!(!is_disabled(&harness, LOAD_FROM_FILE));

    // Suspending closes every popup, so reopen the selector.
    harness.state_mut().suspended = true;
    harness.step();
    open_combo_at(&mut harness, "State 1", 0);
    assert!(is_disabled(&harness, SAVE_TO_FILE), "suspended");
    assert!(is_disabled(&harness, LOAD_FROM_FILE), "suspended");
}

/// A state file saves and loads under its own name, and neither changes
/// the selected state.
#[test]
fn a_state_file_round_trips_without_changing_the_selection() {
    let dir = TempDir::new("state-file-round-trip");
    let mut harness = harness_with_states(&dir);
    harness.state_mut().selected_quick_state = 4;
    let path = dir.path().join("mine.ccstate");

    harness.state_mut().save_state_file(&path);
    assert!(path.is_file());
    assert_eq!(toast(&mut harness).as_deref(), Some("Saved mine.ccstate"));

    let ctx = harness.ctx.clone();
    harness
        .state_mut()
        .request_load(LoadSource::File(path.clone()), &path, &ctx);
    assert!(harness.state().cart_error.is_none());
    assert!(harness.state().pending_load.is_none(), "same machine type");
    assert_eq!(toast(&mut harness).as_deref(), Some("Loaded mine.ccstate"));
    assert_eq!(harness.state().selected_quick_state, 4);
}

/// A quick state saved on another machine type waits for the prompt;
/// Cancel and Esc each leave the machine as it was.
#[test]
fn cancelling_the_prompt_keeps_the_machine() {
    let dir = TempDir::new("state-variant-cancel");
    let mut harness = harness_with_states(&dir);
    save_coco2_state(&state_path(&dir, 2));

    load_state_2_chord(&mut harness);
    harness.step();
    harness.get_by_label(COCO2_PROMPT);
    assert_eq!(variant(&harness), MachineVariant::Coco3, "not loaded yet");
    click(&mut harness, CANCEL);
    assert!(harness.state().pending_load.is_none());

    load_state_2_chord(&mut harness);
    assert!(harness.state().pending_load.is_some());
    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(harness.state().pending_load.is_none());

    assert_eq!(variant(&harness), MachineVariant::Coco3);
    assert!(toast(&mut harness).is_none(), "nothing loaded");
    assert_eq!(harness.state().selected_quick_state, 0);
}

/// Load Anyway restores the other machine type and selects the state.
#[test]
fn load_anyway_restores_the_other_machine_type() {
    let dir = TempDir::new("state-variant-confirm");
    let mut harness = harness_with_states(&dir);
    save_coco2_state(&state_path(&dir, 2));

    load_state_2_chord(&mut harness);
    harness.step();
    click(&mut harness, LOAD_ANYWAY);

    assert!(harness.state().pending_load.is_none());
    assert!(harness.state().cart_error.is_none());
    assert_eq!(variant(&harness), MachineVariant::Coco2);
    assert!(toast(&mut harness).is_some_and(|t| t.starts_with("Loaded State 2")));
    assert_eq!(harness.state().selected_quick_state, 1);
}

/// A state file from another machine type is named in the prompt, and
/// loading it leaves the selection alone.
#[test]
fn a_state_file_from_another_machine_type_asks_first() {
    let dir = TempDir::new("state-variant-file");
    let mut harness = harness_with_states(&dir);
    let path = dir.path().join("coco2.ccstate");
    save_coco2_state(&path);

    let ctx = harness.ctx.clone();
    harness
        .state_mut()
        .request_load(LoadSource::File(path.clone()), &path, &ctx);
    harness.step();
    harness.get_by_label_contains("coco2.ccstate was saved on a CoCo 2");
    click(&mut harness, LOAD_ANYWAY);

    assert_eq!(variant(&harness), MachineVariant::Coco2);
    assert!(toast(&mut harness).is_some_and(|t| t.starts_with("Loaded coco2.ccstate")));
    assert_eq!(harness.state().selected_quick_state, 0);
}

/// Suspending while the prompt is open drops the pending state unloaded.
#[test]
fn suspending_drops_a_pending_load() {
    let dir = TempDir::new("state-variant-suspend");
    let mut harness = harness_with_states(&dir);
    save_coco2_state(&state_path(&dir, 2));

    load_state_2_chord(&mut harness);
    assert!(harness.state().pending_load.is_some());
    harness.state_mut().suspended = true;
    harness.step();

    assert!(harness.state().pending_load.is_none());
    assert_eq!(variant(&harness), MachineVariant::Coco3);
}

/// While the prompt is open, the keyboard belongs to it: a save chord
/// does nothing.
#[test]
fn chords_wait_while_the_prompt_is_open() {
    let dir = TempDir::new("state-variant-chords");
    let mut harness = harness_with_states(&dir);
    save_coco2_state(&state_path(&dir, 2));

    load_state_2_chord(&mut harness);
    let cmd_shift = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);
    harness.key_press_modifiers(cmd_shift, egui::Key::Num1);
    harness.step();

    assert!(!state_path(&dir, 1).exists(), "State 1 was not saved");
    assert!(harness.state().pending_load.is_some());
}
