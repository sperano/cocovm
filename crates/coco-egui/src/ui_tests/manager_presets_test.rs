//! The toolbar's New Machine dialog: New opens it with an empty machine
//! chosen, it lists every template beside that, and Create makes the
//! chosen machine on the spot.

use eframe::egui;
use egui_kittest::kittest::{NodeT, Queryable};

use crate::machine_def::tests::TempDir;
use crate::manager::NO_CONFIG_DIR;
use crate::manager::new_dialog::{
    CANCEL_LABEL, CREATE_LABEL, EMPTY_LABEL, NEW_DIALOG_TITLE, NO_TEMPLATES_TEXT,
};
use crate::manager::presets::Preset;
use crate::manager::template_install::{COPYING_TEXT, PendingInstall};
use crate::manager::templates::tests::{EOU_TEMPLATE, TemplateFixture};

use super::harness::*;

/// A manager over fresh machine directories and the seeded `templates`.
fn template_harness(name: &str, templates: &TemplateFixture) -> (ManagerHarness, TempDir, TempDir) {
    let machines = TempDir::new(&format!("{name}-machines"));
    let artifacts = TempDir::new(&format!("{name}-artifacts"));
    let mut harness = manager_harness_with_artifacts(
        Some(machines.path().to_path_buf()),
        Some(artifacts.path().to_path_buf()),
        Vec::new(),
    );
    harness.state_mut().templates = templates.state();
    (harness, machines, artifacts)
}

/// The dialog lists every template; creating one saves, inserts, and
/// selects a machine built from it, with its blank floppy in the artifact
/// dir, and closes the dialog.
#[test]
fn new_dialog_creates_a_machine_from_a_template() {
    let templates = TemplateFixture::seeded("new-dialog");
    templates.fill_eou(1);
    let (mut harness, machines, artifacts) = template_harness("new-dialog", &templates);

    click(&mut harness, "New");
    harness.get_by_label(NEW_DIALOG_TITLE);
    harness.get_by_label(EMPTY_LABEL);
    for preset in Preset::ALL {
        assert!(
            !harness
                .get_by_label(preset.name())
                .accesskit_node()
                .is_disabled(),
            "{} must be offered",
            preset.name()
        );
    }
    assert!(
        harness.state().entries.is_empty(),
        "opening creates nothing"
    );
    click(&mut harness, Preset::CoCo2DiskBasic.name());
    click(&mut harness, CREATE_LABEL);

    assert!(harness.query_by_label(NEW_DIALOG_TITLE).is_none());
    let state = harness.state();
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.selection.single(), Some(0));
    let entry = &state.entries[0];
    assert_eq!(entry.slug, "coco-2-disk-basic");
    assert_eq!(entry.def.name, Preset::CoCo2DiskBasic.name());
    assert!(machines.path().join("coco-2-disk-basic.toml").is_file());
    assert!(
        artifacts
            .path()
            .join("coco-2-disk-basic")
            .join("disk0.dsk")
            .is_file()
    );
}

/// Cancel closes the dialog without creating anything.
#[test]
fn new_dialog_cancel_creates_nothing() {
    let templates = TemplateFixture::seeded("new-dialog-cancel");
    let (mut harness, machines, _artifacts) = template_harness("new-dialog-cancel", &templates);

    click(&mut harness, "New");
    click(&mut harness, Preset::CoCo3DiskBasic.name());
    click(&mut harness, CANCEL_LABEL);

    assert!(harness.query_by_label(NEW_DIALOG_TITLE).is_none());
    assert!(harness.state().entries.is_empty());
    assert_eq!(std::fs::read_dir(machines.path()).unwrap().count(), 0);
}

/// Esc closes the dialog without creating anything.
#[test]
fn new_dialog_escape_creates_nothing() {
    let machines = TempDir::new("new-dialog-escape-machines");
    let mut harness = manager_harness(Some(machines.path().to_path_buf()), Vec::new());

    click(&mut harness, "New");
    harness.key_press(egui::Key::Escape);
    harness.step();

    assert!(harness.query_by_label(NEW_DIALOG_TITLE).is_none());
    assert!(harness.state().entries.is_empty());
}

/// The New machine hotkey leaves an open dialog's choice alone.
#[test]
fn new_hotkey_keeps_the_open_dialogs_choice() {
    let templates = TemplateFixture::seeded("new-dialog-hotkey");
    let (mut harness, _machines, _artifacts) = template_harness("new-dialog-hotkey", &templates);

    click(&mut harness, "New");
    click(&mut harness, Preset::CoCo2DiskBasic.name());
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::N);
    harness.step();
    click(&mut harness, CREATE_LABEL);

    let state = harness.state();
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.entries[0].def.name, Preset::CoCo2DiskBasic.name());
}

/// Without its NitrOS-9 EOU images, the dialog turns the template off and
/// offers their download beneath it; the other templates stay available.
#[test]
fn new_dialog_offers_the_eou_download_when_images_are_missing() {
    let templates = TemplateFixture::seeded("new-dialog-no-eou");
    let (mut harness, _machines, _artifacts) = template_harness("new-dialog-no-eou", &templates);

    click(&mut harness, "New");

    let eou_choice = harness.get_by_label(Preset::NitrOS9Eou.name());
    assert!(eou_choice.accesskit_node().is_disabled());
    let download = harness
        .get_all_by_label_contains("Download NitrOS-9")
        .count();
    assert_eq!(download, 1);
    let disk_basic = harness.get_by_label(Preset::CoCo3DiskBasic.name());
    assert!(!disk_basic.accesskit_node().is_disabled());
}

/// A template missing another file is off, with the file named beneath it.
#[test]
fn new_dialog_names_a_templates_missing_files() {
    let templates = TemplateFixture::seeded("new-dialog-missing");
    std::fs::remove_file(templates.media_dir("coco-3-disk-basic").join("disk0.dsk")).unwrap();
    let (mut harness, _machines, _artifacts) = template_harness("new-dialog-missing", &templates);

    click(&mut harness, "New");

    let choice = harness.get_by_label(Preset::CoCo3DiskBasic.name());
    assert!(choice.accesskit_node().is_disabled());
    harness.get_by_label("Missing from the template's folder: disk0.dsk");
}

/// While a machine is copied from a template, the dialog turns that
/// template off and shows a spinner beneath it, so it can't be picked
/// again meanwhile.
#[test]
fn new_dialog_shows_a_running_install() {
    let templates = TemplateFixture::seeded("new-dialog-install");
    templates.fill_eou(1);
    let (mut harness, _machines, _artifacts) = template_harness("new-dialog-install", &templates);
    let (_tx, rx) = std::sync::mpsc::channel();
    let pending = PendingInstall::new(EOU_TEMPLATE.to_string(), EOU_TEMPLATE.to_string(), rx);
    harness.state_mut().templates.installs.push(pending);

    click(&mut harness, "New");

    harness.get_by_label(COPYING_TEXT);
    let eou_choice = harness.get_by_label(Preset::NitrOS9Eou.name());
    assert!(eou_choice.accesskit_node().is_disabled());
    let disk_basic = harness.get_by_label(Preset::CoCo3DiskBasic.name());
    assert!(!disk_basic.accesskit_node().is_disabled());
}

/// With every template deleted, the dialog says how to make one; the
/// empty machine is still there.
#[test]
fn new_dialog_without_templates_says_how_to_make_one() {
    let machines = TempDir::new("new-dialog-none-machines");
    let mut harness = manager_harness(Some(machines.path().to_path_buf()), Vec::new());

    click(&mut harness, "New");

    harness.get_by_label(NO_TEMPLATES_TEXT);
    click(&mut harness, CREATE_LABEL);
    assert_eq!(harness.state().entries.len(), 1);
}

/// A Create that fails with nothing selected shows its error in the
/// welcome pane, which is what the closed dialog leaves in view.
#[test]
fn failed_create_with_nothing_selected_shows_in_the_welcome_pane() {
    let mut harness = manager_harness(None, Vec::new());

    create_empty_machine(&mut harness);

    assert!(harness.query_by_label(NEW_DIALOG_TITLE).is_none());
    assert!(harness.state().entries.is_empty());
    harness.get_by_label(NO_CONFIG_DIR);
}

/// A Create that fails with several rows selected shows its error in the
/// bulk pane.
#[test]
fn failed_create_with_several_selected_shows_in_the_bulk_pane() {
    let entries = vec![
        sample_entry("alpha", "Alpha CoCo 3"),
        sample_entry("beta", "Beta CoCo 3"),
    ];
    let mut harness = manager_harness(None, entries);
    click(&mut harness, "Alpha CoCo 3");
    click_modifiers(&mut harness, "Beta CoCo 3", egui::Modifiers::COMMAND);
    assert_eq!(harness.state().selection.len(), 2);

    create_empty_machine(&mut harness);

    assert_eq!(harness.state().entries.len(), 2);
    assert_eq!(harness.state().selection.len(), 2);
    harness.get_by_label(NO_CONFIG_DIR);
}
