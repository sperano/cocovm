//! Multi-selection UI tests: Cmd/Ctrl- and Shift-click on machine-list rows,
//! the bulk detail pane they reveal, the bulk context menu, and empty-space
//! click clearing a multi-selection. The toggle/range *math* is unit-tested
//! in `manager/selection_test.rs`; these drive the real widget tree end to
//! end, the way `manager_window.rs`'s single-selection tests do for a plain
//! click.
//!
//! `egui_kittest::Node::click_modifiers` (confirmed present in the pinned
//! 0.33.3 — `~/.cargo/registry/src/*/egui_kittest-0.33.3/src/node.rs`) is
//! what makes this possible: it hovers, then presses and releases with the
//! given `egui::Modifiers` set for that frame, same as a real modifier-held
//! click.

use egui_kittest::kittest::Queryable;

use crate::*;

use super::harness::*;

/// Cmd/Ctrl-clicking a second row adds it to the selection without
/// dropping the first, and the central panel switches to the bulk pane.
#[test]
fn cmd_click_adds_a_second_row_to_the_selection() {
    let entries = vec![
        sample_entry("alpha", "Alpha CoCo 3"),
        sample_entry("beta", "Beta CoCo 3"),
    ];
    let mut harness = manager_harness(None, entries);

    click(&mut harness, "Alpha CoCo 3");
    assert_eq!(harness.state().selection.single(), Some(0));

    click_modifiers(&mut harness, "Beta CoCo 3", egui::Modifiers::COMMAND);
    assert_eq!(harness.state().selection.len(), 2);
    assert!(harness.state().selection.contains(0) && harness.state().selection.contains(1));
    harness.get_by_label("2 machines selected");
}

/// Shift-clicking the last of three rows, with the first already selected,
/// selects the whole inclusive range.
#[test]
fn shift_click_selects_a_range() {
    let entries = vec![
        sample_entry("alpha", "Alpha CoCo 3"),
        sample_entry("beta", "Beta CoCo 3"),
        sample_entry("gamma", "Gamma CoCo 3"),
    ];
    let mut harness = manager_harness(None, entries);

    click(&mut harness, "Alpha CoCo 3");
    click_modifiers(&mut harness, "Gamma CoCo 3", egui::Modifiers::SHIFT);

    assert_eq!(harness.state().selection.len(), 3);
    for i in 0..3 {
        assert!(
            harness.state().selection.contains(i),
            "row {i} must be in the range"
        );
    }
    harness.get_by_label("3 machines selected");
}

/// A plain click after a multi-selection collapses it back to just the
/// clicked row and brings the single-machine edit form back — the existing
/// single-selection behavior, untouched by multi-select.
#[test]
fn plain_click_collapses_back_to_single_selection() {
    let entries = vec![
        sample_entry("alpha", "Alpha CoCo 3"),
        sample_entry("beta", "Beta CoCo 3"),
    ];
    let mut harness = manager_harness(None, entries);

    click(&mut harness, "Alpha CoCo 3");
    click_modifiers(&mut harness, "Beta CoCo 3", egui::Modifiers::COMMAND);
    assert_eq!(harness.state().selection.len(), 2);

    click(&mut harness, "Alpha CoCo 3");
    assert_eq!(harness.state().selection.single(), Some(0));
    assert_eq!(harness.state().detail_name(), Some("Alpha CoCo 3"));
}

/// Right-clicking a row that's part of the current multi-selection opens
/// the bulk menu: it has "Delete…" but no "Show config" (there is no single
/// machine to show config for).
#[test]
fn right_click_inside_multi_selection_shows_the_bulk_menu() {
    let entries = vec![
        sample_entry("alpha", "Alpha CoCo 3"),
        sample_entry("beta", "Beta CoCo 3"),
    ];
    let mut harness = manager_harness(None, entries);

    click(&mut harness, "Alpha CoCo 3");
    click_modifiers(&mut harness, "Beta CoCo 3", egui::Modifiers::COMMAND);

    right_click(&mut harness, "Alpha CoCo 3");
    harness.get_by_label("Delete…");
    assert!(
        harness.query_by_label("Show config").is_none(),
        "the bulk menu has no per-machine Show config"
    );
}

/// Right-clicking a row that is *not* part of the current multi-selection
/// falls back to the ordinary single-row menu (with "Show config") and, per
/// the existing right-click invariant, does not change the selection.
#[test]
fn right_click_outside_selection_shows_the_single_menu_and_does_not_select() {
    let entries = vec![
        sample_entry("alpha", "Alpha CoCo 3"),
        sample_entry("beta", "Beta CoCo 3"),
        sample_entry("gamma", "Gamma CoCo 3"),
    ];
    let mut harness = manager_harness(None, entries);

    click(&mut harness, "Alpha CoCo 3");
    click_modifiers(&mut harness, "Beta CoCo 3", egui::Modifiers::COMMAND);
    assert_eq!(harness.state().selection.len(), 2);

    right_click(&mut harness, "Gamma CoCo 3");
    harness.get_by_label("Show config");
    assert_eq!(
        harness.state().selection.len(),
        2,
        "right-click must not change the selection"
    );
    assert!(harness.state().selection.contains(0) && harness.state().selection.contains(1));
}

/// Clicking the empty space below the rows clears a multi-selection just
/// like it clears a single one (`manager_window.rs`'s
/// `manager_click_below_the_list_clears_the_selection`).
#[test]
fn empty_space_click_clears_a_multi_selection() {
    let entries = vec![
        sample_entry("alpha", "Alpha CoCo 3"),
        sample_entry("beta", "Beta CoCo 3"),
    ];
    let mut harness = manager_harness(None, entries);

    click(&mut harness, "Alpha CoCo 3");
    click_modifiers(&mut harness, "Beta CoCo 3", egui::Modifiers::COMMAND);
    assert_eq!(harness.state().selection.len(), 2);

    let empty_spot = egui::pos2(100.0, 650.0);
    harness.hover_at(empty_spot);
    harness.step();
    harness.drag_at(empty_spot);
    harness.step();
    harness.drop_at(empty_spot);
    harness.step();
    harness.step();

    assert!(harness.state().selection.is_empty());
}

/// Cmd/Ctrl-A selects every row, and only when no widget owns the
/// keyboard — with the Name field focused (mid-rename), it must be left
/// alone for the text field's own native select-all instead.
#[test]
fn cmd_a_selects_every_row_unless_a_text_field_is_focused() {
    let entries = vec![
        sample_entry("alpha", "Alpha CoCo 3"),
        sample_entry("beta", "Beta CoCo 3"),
    ];
    let mut harness = manager_harness(None, entries);

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.step();
    assert_eq!(
        harness.state().selection.len(),
        2,
        "⌘A must select every row"
    );

    click(&mut harness, "Alpha CoCo 3");
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .focus();
    harness.step();
    assert_eq!(
        harness.state().selection.single(),
        Some(0),
        "focusing the Name field must not select"
    );

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.step();
    assert_eq!(
        harness.state().selection.single(),
        Some(0),
        "⌘A with the Name field focused must not steal the row-select shortcut"
    );
}
