//! The VM toolbar's quick-state group (`chrome/toolbar/quick_states.rs`)
//! and the quick actions it shares with the Machine menu and the numbered
//! chords (`save_state/quick.rs`): Load gated on the state file existing,
//! the per-window selection, the collapsed and hidden layouts, and the
//! empty-state chord. Every test points the window at its own scratch
//! quick-state directory, never the user's.

use egui_kittest::kittest::{NodeT, Queryable};

use crate::chrome::toolbar::quick_states::{SHARED_NOTE, STATES_LABEL};
use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;

/// Width step of the narrow-window sweep.
const SWEEP_STEP: f32 = 16.0;
/// Narrowest window the sweep tries: well below the transport tiles alone.
const SWEEP_MIN_WIDTH: f32 = 160.0;

/// [`boot_harness`] with quick states kept in `dir`.
fn harness_with_states(dir: &TempDir) -> AppHarness {
    let mut harness = boot_harness();
    harness.state_mut().quick_state_dir = Some(dir.path().to_path_buf());
    harness.step();
    harness
}

/// File of State `n` (1-based) in `dir`.
fn state_path(dir: &TempDir, n: usize) -> std::path::PathBuf {
    dir.path().join(format!("slot-{n}.ccstate"))
}

fn is_disabled(harness: &AppHarness, label: &str) -> bool {
    harness.get_by_label(label).accesskit_node().is_disabled()
}

fn toast(harness: &mut AppHarness) -> Option<String> {
    harness.state_mut().toast_message()
}

/// Load is disabled while the selected state has no file and enabled as
/// soon as one appears (here written behind the window's back, as another
/// VM window would), even one that won't load; Save stays enabled.
#[test]
fn load_follows_whether_the_state_file_exists() {
    let dir = TempDir::new("quick-load-gate");
    let mut harness = harness_with_states(&dir);

    assert!(is_disabled(&harness, "Load State 1"), "empty state");
    assert!(!is_disabled(&harness, "Save to State 1"));

    std::fs::write(state_path(&dir, 1), b"not a state").expect("write fixture");
    harness.step();
    assert!(!is_disabled(&harness, "Load State 1"), "file now exists");

    std::fs::remove_file(state_path(&dir, 1)).expect("remove fixture");
    harness.step();
    assert!(is_disabled(&harness, "Load State 1"), "file deleted again");
}

/// Save writes the selected state without a dialog and Load restores it,
/// each confirmed by its toast.
#[test]
fn toolbar_save_then_load_round_trip() {
    let dir = TempDir::new("quick-round-trip");
    let mut harness = harness_with_states(&dir);

    click(&mut harness, "Save to State 1");
    assert!(state_path(&dir, 1).is_file(), "Save writes State 1's file");
    assert_eq!(toast(&mut harness).as_deref(), Some("Saved State 1"));
    assert!(harness.state().cart_error.is_none());

    click(&mut harness, "Load State 1");
    assert!(harness.state().cart_error.is_none());
    assert_eq!(toast(&mut harness).as_deref(), Some("Loaded State 1"));
}

/// A file that exists but is corrupt keeps Load enabled; loading it reports
/// the error dialog and leaves the selection alone.
#[test]
fn a_corrupt_state_reports_an_error() {
    let dir = TempDir::new("quick-corrupt");
    let mut harness = harness_with_states(&dir);
    std::fs::write(state_path(&dir, 1), b"not a state").expect("write fixture");
    harness.step();

    click(&mut harness, "Load State 1");
    assert!(
        harness.state().cart_error.is_some(),
        "the error dialog reports it"
    );
    assert_eq!(harness.state().selected_quick_state, 0);
}

/// The selector lists all ten states plus the shared-storage note, and
/// picking a row (even an empty one) only retargets Save and Load.
#[test]
fn the_selector_only_changes_the_target() {
    let dir = TempDir::new("quick-selector");
    let mut harness = harness_with_states(&dir);

    open_combo_at(&mut harness, "State 1", 0);
    for n in 1..=save_state::QUICK_SLOTS {
        harness.get_by_label(&format!("State {n} — Empty"));
    }
    harness.get_by_label(SHARED_NOTE);
    click(&mut harness, "State 3 — Empty");

    assert_eq!(harness.state().selected_quick_state, 2);
    assert!(harness.state_mut().toast_message().is_none(), "nothing ran");
    assert!(std::fs::read_dir(dir.path()).expect("dir").next().is_none());
    harness.get_by_label("Save to State 3");
    assert!(is_disabled(&harness, "Load State 3"));
}

/// The load chord on an empty state skips the load (no error dialog) and
/// only toasts that the state is empty.
#[test]
fn the_load_chord_on_an_empty_state_does_not_load() {
    let dir = TempDir::new("quick-empty-chord");
    let mut harness = harness_with_states(&dir);

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num2);
    harness.step();

    assert!(
        harness.state().cart_error.is_none(),
        "no load was attempted"
    );
    assert_eq!(toast(&mut harness).as_deref(), Some("State 2 is empty"));
    assert_eq!(harness.state().selected_quick_state, 0);
}

/// Chords and Machine-menu quick actions select their state in this window,
/// but only when they succeed.
#[test]
fn quick_actions_select_their_state_only_on_success() {
    let dir = TempDir::new("quick-selection");
    let mut harness = harness_with_states(&dir);

    let cmd_shift = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);
    harness.key_press_modifiers(cmd_shift, egui::Key::Num3);
    harness.step();
    assert!(state_path(&dir, 3).is_file());
    assert_eq!(harness.state().selected_quick_state, 2, "saved State 3");

    std::fs::write(state_path(&dir, 2), b"not a state").expect("write fixture");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num2);
    harness.step();
    assert!(
        harness.state().cart_error.is_some(),
        "corrupt State 2 fails"
    );
    assert_eq!(
        harness.state().selected_quick_state,
        2,
        "failure keeps State 3"
    );
    harness.state_mut().cart_error = None;
    harness.step();

    click(&mut harness, "Machine");
    click_containing(&mut harness, "Quick Save");
    click(&mut harness, "State 7 — Empty");
    assert!(state_path(&dir, 7).is_file());
    assert_eq!(harness.state().selected_quick_state, 6, "saved State 7");
    harness.get_by_label("Load State 7");
}

/// The Machine menu's Quick Load lists all ten states and disables the
/// ones without a file.
#[test]
fn machine_menu_quick_load_disables_empty_states() {
    let dir = TempDir::new("quick-menu-load");
    let mut harness = harness_with_states(&dir);
    std::fs::write(state_path(&dir, 4), b"not a state").expect("write fixture");
    harness.step();

    click(&mut harness, "Machine");
    click_containing(&mut harness, "Quick Load");
    // Rows carry their chord in the accessible label ("State 1 — Empty Ctrl+1").
    for n in 1..=save_state::QUICK_SLOTS {
        let prefix = format!("State {n} — ");
        let row = harness.get_by_label_contains(&prefix);
        let label = row.accesskit_node().label().unwrap_or_default();
        let empty = label.contains("Empty");
        assert_eq!(empty, n != 4, "{label}");
        assert_eq!(row.accesskit_node().is_disabled(), empty, "{label}");
    }
}

/// Suspended disables both actions; a debugger pause does not.
#[test]
fn save_and_load_follow_suspend_but_not_pause() {
    let dir = TempDir::new("quick-suspend");
    let mut harness = harness_with_states(&dir);
    std::fs::write(state_path(&dir, 1), b"not a state").expect("write fixture");

    harness.state_mut().running = false;
    harness.step();
    assert!(!is_disabled(&harness, "Save to State 1"), "paused");
    assert!(!is_disabled(&harness, "Load State 1"), "paused");

    harness.state_mut().suspended = true;
    harness.step();
    assert!(is_disabled(&harness, "Save to State 1"), "suspended");
    assert!(is_disabled(&harness, "Load State 1"), "suspended");
}

/// Icons-only mode keeps the textual selector and the tiles' accessible
/// names.
#[test]
fn icons_only_keeps_the_selector_and_names() {
    let dir = TempDir::new("quick-icons-only");
    let mut harness = harness_with_states(&dir);
    harness.state_mut().toolbar_icons_only = true;
    harness.step();

    assert!(harness.get_all_by_value("State 1").next().is_some());
    harness.get_by_label("Save to State 1");
    harness.get_by_label("Load State 1");
}

/// Which layout of the group the toolbar shows right now.
fn shown_layout(harness: &AppHarness) -> &'static str {
    let shows = |label| harness.query_all_by_label(label).next().is_some();
    if shows("Save to State 1") {
        "full"
    } else if shows(STATES_LABEL) {
        "collapsed"
    } else {
        "hidden"
    }
}

/// Narrowing the window steps the group from full to collapsed to hidden,
/// in that order, without the toolbar's later tiles leaving the window.
#[test]
fn narrowing_collapses_then_hides_the_group() {
    let dir = TempDir::new("quick-narrow");
    let mut harness = harness_with_states(&dir);
    let height = harness.ctx.content_rect().height();

    let mut seen: Vec<&str> = Vec::new();
    let mut width = harness.ctx.content_rect().width();
    while width >= SWEEP_MIN_WIDTH {
        harness.set_size(egui::vec2(width, height));
        harness.step();
        let layout = shown_layout(&harness);
        if seen.last() != Some(&layout) {
            seen.push(layout);
        }
        #[cfg(feature = "debug-ui")]
        if layout != "hidden" {
            let debug = harness.get_by_label("Debug").rect();
            assert!(
                debug.max.x <= width,
                "{layout} at {width}: Debug off-window"
            );
        }
        width -= SWEEP_STEP;
    }
    assert_eq!(seen, ["full", "collapsed", "hidden"]);
}

/// The collapsed States menu offers Save, Load, and the state rows.
#[test]
fn the_collapsed_menu_saves_to_the_selected_state() {
    let dir = TempDir::new("quick-collapsed");
    let mut harness = harness_with_states(&dir);
    let height = harness.ctx.content_rect().height();
    let mut width = harness.ctx.content_rect().width();
    while shown_layout(&harness) != "collapsed" {
        width -= SWEEP_STEP;
        assert!(width >= SWEEP_MIN_WIDTH, "never collapsed");
        harness.set_size(egui::vec2(width, height));
        harness.step();
    }

    click(&mut harness, STATES_LABEL);
    assert!(is_disabled(&harness, "Load State 1"));
    harness.get_by_label(SHARED_NOTE);
    click(&mut harness, "Save to State 1");
    assert!(state_path(&dir, 1).is_file());
    assert_eq!(toast(&mut harness).as_deref(), Some("Saved State 1"));
}
