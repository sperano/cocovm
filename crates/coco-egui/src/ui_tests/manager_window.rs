//! Manager window scaffold and machine-list/detail-pane tests: toolbar,
//! divider drag, photo pane, row selection, context menu, delete
//! confirmation, instant-create, auto-save, and rename/slug migration.

use std::fs;

use egui_kittest::kittest::{NodeT, Queryable};

use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;

/// The detail pane's RAM fieldset: the group's title is a real node in the
/// accessibility tree (the `titled_group` widget promises this), and
/// clicking a size radio auto-saves the definition like any other form
/// edit.
#[test]
fn ram_radio_autosaves_the_definition() {
    let dir = TempDir::new("ram-radio");
    let entries = vec![sample_entry("dev-coco-3", "Dev CoCo 3")];
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), entries);

    click(&mut harness, "Dev CoCo 3");
    harness.get_by_label("RAM");
    click(&mut harness, "2048K");

    let config = harness.state().entries[0]
        .def
        .to_machine_config()
        .expect("saved definitions validate");
    assert_eq!(config.memory, coco_core::MemorySize::K2048);
    let toml = fs::read_to_string(dir.path().join("dev-coco-3.toml")).unwrap();
    assert!(
        toml.contains("2048"),
        "auto-save must write the new RAM size: {toml}"
    );
}

/// ⌘N/Ctrl+N in the manager is the toolbar's "New…": it creates a machine
/// on the spot — saved to disk, inserted, selected — with no dialog.
#[test]
fn cmd_n_creates_a_machine_immediately() {
    let dir = TempDir::new("cmd-n-manager");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());
    assert!(harness.state().entries.is_empty());

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::N);
    harness.step();
    harness.step();
    assert_eq!(
        harness.state().entries.len(),
        1,
        "Cmd/Ctrl+N must create a machine immediately in the manager"
    );
    assert_eq!(harness.state().selection.single(), Some(0));
    assert!(dir.path().join("coco-3.toml").is_file());
}

/// The manager window scaffold: toolbar buttons present — New, the four
/// transport tiles, Settings, Help — the machine-list panel and photo pane
/// laid out without a photo injected. This harness has no machines dir (no
/// home), so "New" must report that instead of creating or panicking, and
/// with nothing selected (no entries at all, here) the transport tiles must
/// start disabled — the toolbar's own version of the old detail pane's
/// "nothing to act on yet" state.
#[test]
fn manager_window_shows_its_toolbar() {
    let mut harness = egui_kittest::Harness::new_eframe(|_cc| {
        manager::ManagerApp::new(None, None, None, Vec::new())
    });
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();

    for label in [
        "New", "Start", "Suspend", "Stop", "Reset", "Settings", "Help",
    ] {
        harness.get_by_label(label);
    }
    assert!(
        harness.get_by_label("Start").accesskit_node().is_disabled(),
        "Start must be disabled with no selection"
    );

    harness.get_by_label("New").hover();
    harness.step();
    harness.get_by_label("New").click();
    harness.step();
    harness.step();
    assert!(
        harness.state().entries.is_empty(),
        "no config dir: New must fail gracefully, not add a row"
    );
}

/// The divider between the machine list and the photo pane must be
/// draggable. Regression test for the empty-panel gotcha: a `SidePanel`
/// whose ui claims no space silently loses its resize drag
/// (`SidePanel::resizable` docs — hence `take_available_space` in
/// `ManagerApp::update`).
#[test]
fn manager_list_divider_is_draggable() {
    let mut harness = egui_kittest::Harness::new_eframe(|_cc| {
        manager::ManagerApp::new(None, None, None, Vec::new())
    });
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();

    let panel_id = egui::Id::new("manager_machine_list");
    let width = |harness: &egui_kittest::Harness<'_, manager::ManagerApp>| {
        egui::containers::panel::PanelState::load(&harness.ctx, panel_id)
            .expect("machine-list panel state exists")
            .rect
            .width()
    };
    let before = width(&harness);

    // Grab the divider (the panel's right edge) at mid-height and drag it
    // 80 px right: hover, press, move while pressed, release.
    let grab = egui::pos2(before, 360.0);
    let target = egui::pos2(before + 80.0, 360.0);
    harness.hover_at(grab);
    harness.step();
    harness.drag_at(grab);
    harness.step();
    harness.hover_at(target);
    harness.step();
    harness.drop_at(target);
    harness.step();
    harness.step();

    let after = width(&harness);
    assert!(
        (after - before) > 60.0,
        "dragging the divider must widen the list panel (before {before}, after {after})"
    );
}

/// With a photo injected, the manager uploads it on the first frame and
/// keeps rendering (the image itself is not an accessible node — this
/// guards the upload path against panics/regressions).
#[test]
fn manager_window_renders_an_injected_photo() {
    let photo = photo_view::Photo {
        title: "test-photo".to_string(),
        pixels: egui::ColorImage::from_rgba_unmultiplied([8, 6], &[0x20; 8 * 6 * 4]),
    };
    let mut harness = egui_kittest::Harness::new_eframe(|_cc| {
        manager::ManagerApp::new(Some(photo), None, None, Vec::new())
    });
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();
    harness.step();
}

/// Selecting a row shows its detail pane, seeded from that entry's
/// definition — not whatever the previously-selected row left behind.
#[test]
fn manager_list_shows_entries_and_selecting_shows_detail() {
    let entries = vec![
        sample_entry("alpha", "Alpha CoCo 3"),
        sample_entry("beta", "Beta CoCo 3"),
    ];
    let mut harness = manager_harness(None, entries);

    harness.get_by_label("Alpha CoCo 3");
    harness.get_by_label("Beta CoCo 3");
    assert_eq!(harness.state().detail_name(), None, "nothing selected yet");

    click(&mut harness, "Beta CoCo 3");
    assert_eq!(harness.state().selection.single(), Some(1));
    assert_eq!(harness.state().detail_name(), Some("Beta CoCo 3"));

    click(&mut harness, "Alpha CoCo 3");
    assert_eq!(harness.state().selection.single(), Some(0));
    assert_eq!(
        harness.state().detail_name(),
        Some("Alpha CoCo 3"),
        "switching rows must reseed the draft, not keep editing the old one"
    );
}

/// Clicking the empty space below the last list row clears the selection —
/// the detail pane gives way to the photo pane again. Positional click: the
/// empty area is no accessible node, so this drives the pointer directly
/// (same primitives as the divider-drag test) at a point well below the
/// single row but inside the list panel.
#[test]
fn manager_click_below_the_list_clears_the_selection() {
    let entries = vec![sample_entry("alpha", "Alpha CoCo 3")];
    let mut harness = manager_harness(None, entries);

    click(&mut harness, "Alpha CoCo 3");
    assert_eq!(harness.state().selection.single(), Some(0));
    assert_eq!(harness.state().detail_name(), Some("Alpha CoCo 3"));

    let empty_spot = egui::pos2(100.0, 650.0);
    harness.hover_at(empty_spot);
    harness.step();
    harness.drag_at(empty_spot);
    harness.step();
    harness.drop_at(empty_spot);
    harness.step();
    harness.step();

    assert_eq!(
        harness.state().selection.single(),
        None,
        "empty-space click must deselect"
    );
    assert_eq!(
        harness.state().detail_name(),
        None,
        "detail draft must be dropped"
    );
}

/// Right-clicking a list row opens its context menu *without* moving the
/// visual selection — the menu's items act on the row under the cursor, not
/// on `selected` (user decision 2026-07-23). The one exception is "Show
/// config", whose whole job is to select; it (like every pick) also closes
/// the menu.
#[test]
fn manager_row_right_click_opens_context_menu_without_selecting() {
    let entries = vec![
        sample_entry("alpha", "Alpha CoCo 3"),
        sample_entry("beta", "Beta CoCo 3"),
    ];
    let mut harness = manager_harness(None, entries);
    assert!(
        harness.query_by_label("Show config").is_none(),
        "menu must start closed"
    );

    click(&mut harness, "Alpha CoCo 3");
    assert_eq!(harness.state().selection.single(), Some(0));

    right_click(&mut harness, "Beta CoCo 3");
    assert_eq!(
        harness.state().selection.single(),
        Some(0),
        "right-click must leave the selection cue where it was"
    );

    click(&mut harness, "Show config");
    assert_eq!(
        harness.state().selection.single(),
        Some(1),
        "Show config selects the right-clicked row, not the old selection"
    );
    assert!(
        harness.query_by_label("Show config").is_none(),
        "picking an item closes the menu"
    );
}

/// The context menu's "Delete…" asks for confirmation first: Cancel keeps
/// the machine untouched; Delete removes the list row and its `<slug>.toml`,
/// and the selection follows the surviving row as indices shift.
#[test]
fn manager_row_context_menu_delete_confirms_and_removes() {
    let dir = TempDir::new("ctx-delete");
    let entries = vec![
        sample_entry("alpha", "Alpha CoCo 3"),
        sample_entry("beta", "Beta CoCo 3"),
    ];
    for entry in &entries {
        machine_def::save(dir.path(), &entry.slug, &entry.def).expect("seed definition files");
    }
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), entries);

    click(&mut harness, "Beta CoCo 3");
    assert_eq!(harness.state().selection.single(), Some(1));

    right_click(&mut harness, "Alpha CoCo 3");
    click(&mut harness, "Delete…");
    click(&mut harness, "Cancel");
    assert_eq!(
        harness.state().entries.len(),
        2,
        "Cancel must keep the machine"
    );
    assert!(
        dir.path().join("alpha.toml").exists(),
        "Cancel must keep the definition file"
    );

    right_click(&mut harness, "Alpha CoCo 3");
    click(&mut harness, "Delete…");
    click(&mut harness, "Delete");
    assert_eq!(harness.state().entries.len(), 1);
    assert!(
        !dir.path().join("alpha.toml").exists(),
        "the definition file must be removed"
    );
    assert!(
        dir.path().join("beta.toml").exists(),
        "only the confirmed machine is deleted"
    );
    assert_eq!(
        harness.state().selection.single(),
        Some(0),
        "the selection must follow the surviving row as indices shift, not be cleared"
    );
    assert_eq!(
        harness.state().detail_name(),
        Some("Beta CoCo 3"),
        "the selection must follow the surviving row as indices shift"
    );
}

/// "New" creates a definition file *immediately* — default name under a
/// uniquified slug, saved, selected, no dialog and no Create button (macOS
/// System-Settings-style, user decision 2026-07-24). A second "New"
/// uniquifies against the first.
#[test]
fn manager_new_creates_a_definition_file_immediately() {
    let dir = TempDir::new("create");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());
    assert!(harness.state().entries.is_empty());

    click_containing(&mut harness, "New");

    assert_eq!(
        harness.state().entries.len(),
        1,
        "New must add a list row on the spot"
    );
    assert!(
        harness.query_by_label("Create").is_none(),
        "no dialog is involved"
    );
    let slug = harness.state().entries[0].slug.clone();
    assert_eq!(slug, "coco-3", "slugified from the default name");
    assert_eq!(harness.state().entries[0].def.name, "CoCo 3");

    let file = dir.path().join(format!("{slug}.toml"));
    let contents = fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    let parsed: machine_def::MachineDef =
        toml::from_str(&contents).expect("New must write a parseable definition");
    assert_eq!(parsed.name, "CoCo 3");
    assert_eq!(
        harness.state().selection.single(),
        Some(0),
        "New must select the new row"
    );
    // Not `get_by_label("CoCo 3")`: the now-visible detail pane's hardware
    // form has its own "CoCo 3" Machine combo button, so the name would be
    // ambiguous between that and the list row.
    assert_eq!(harness.state().detail_name(), Some("CoCo 3"));

    click_containing(&mut harness, "New");
    assert_eq!(harness.state().entries.len(), 2);
    assert_eq!(
        harness.state().entries[1].slug,
        "coco-3-2",
        "second default uniquifies"
    );
    assert!(dir.path().join("coco-3-2.toml").is_file());
}

/// Editing in the detail pane saves immediately — there are no Save/Revert
/// buttons anymore (auto-save, user decision 2026-07-24) — while merely
/// selecting a row must not rewrite its file.
#[test]
fn manager_detail_edits_save_immediately() {
    let dir = TempDir::new("auto-save");
    let entry = sample_entry("dev-coco-3", "Dev CoCo 3");
    machine_def::save(dir.path(), "dev-coco-3", &entry.def)
        .expect("seed the file the entry claims to be");
    assert!(
        entry.def.ui.aspect_correct,
        "test assumes the sample starts aspect-corrected"
    );

    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![entry]);
    let file = dir.path().join("dev-coco-3.toml");
    let before = fs::read_to_string(&file).unwrap();

    click(&mut harness, "Dev CoCo 3");
    harness.step();
    assert!(
        harness.query_by_label("Save").is_none(),
        "auto-save: no Save button"
    );
    assert!(
        harness.query_by_label("Revert").is_none(),
        "auto-save: no Revert button"
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        before,
        "selecting a row must not rewrite its definition"
    );

    click(&mut harness, "4:3 aspect correction");
    let saved: machine_def::MachineDef =
        toml::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert!(
        !saved.ui.aspect_correct,
        "the toggle must reach the file without any Save click"
    );
    assert!(!harness.state().entries[0].def.ui.aspect_correct);
}

/// Committing a new name (focus leaves the Name field) saves it and
/// migrates the slug: `<slug>.toml` and the artifact directory follow the
/// display name. Nothing else persists the slug — relative `[media]`
/// entries name files *inside* the artifact dir — so a rename is exactly
/// those two filesystem moves.
#[test]
fn manager_rename_migrates_definition_file_and_artifact_dir() {
    let machines = TempDir::new("rename-machines");
    let artifacts = TempDir::new("rename-artifacts");
    let entry = sample_entry("alpha", "Alpha");
    machine_def::save(machines.path(), "alpha", &entry.def).expect("seed the definition");
    fs::create_dir_all(artifacts.path().join("alpha")).unwrap();
    fs::write(artifacts.path().join("alpha").join("disk0.dsk"), b"").unwrap();

    let mut harness = manager_harness_with_artifacts(
        Some(machines.path().to_path_buf()),
        Some(artifacts.path().to_path_buf()),
        vec![entry],
    );
    click(&mut harness, "Alpha");

    // Type into the Name field — the pane's only text input; by-value
    // lookup would be ambiguous with the list row's own "Alpha" label —
    // and commit with Enter: the TextEdit surrenders focus, which is the
    // commit signal.
    let name_field = || harness.get_by_role(egui::accesskit::Role::TextInput);
    name_field().focus();
    harness.step();
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .type_text(" Two");
    harness.step();
    assert_eq!(harness.state().detail_name(), Some("Alpha Two"));
    harness.key_press(egui::Key::Enter);
    harness.step();
    harness.step(); // commit frame, then the deferred migration frame
    harness.step();

    assert_eq!(harness.state().entries[0].slug, "alpha-two");
    assert_eq!(harness.state().entries[0].def.name, "Alpha Two");
    assert!(machines.path().join("alpha-two.toml").is_file());
    assert!(!machines.path().join("alpha.toml").exists());
    assert!(
        artifacts
            .path()
            .join("alpha-two")
            .join("disk0.dsk")
            .is_file(),
        "the artifact dir must follow the slug"
    );
    assert!(!artifacts.path().join("alpha").exists());
    assert_eq!(
        harness.state().selection.single(),
        Some(0),
        "selection follows the renamed row"
    );
    assert_eq!(harness.state().detail_name(), Some("Alpha Two"));
}
