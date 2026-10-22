//! The toolbar's New Machine dialog: New opens it with an empty machine
//! chosen, it lists every built-in template beside that, and Create makes
//! the chosen machine on the spot.

use egui_kittest::kittest::{NodeT, Queryable};

use crate::machine_def::tests::TempDir;
use crate::manager::eou_install::{COPYING_TEXT, PendingInstall};
use crate::manager::new_dialog::{CREATE_LABEL, EMPTY_LABEL, NEW_DIALOG_TITLE};
use crate::manager::presets::Preset;
use crate::nitros9::EOU_IMAGES;

use super::harness::*;

/// Fills a temporary NitrOS-9 EOU images directory, so the template can be
/// created.
fn eou_images(name: &str) -> TempDir {
    let eou = TempDir::new(name);
    for image in EOU_IMAGES {
        std::fs::write(eou.path().join(image), b"eou image").unwrap();
    }
    eou
}

/// The dialog lists every template; creating one saves, inserts, and
/// selects a machine built from it, with its blank floppy in the artifact
/// dir, and closes the dialog.
#[test]
fn new_dialog_creates_a_machine_from_a_template() {
    let machines = TempDir::new("new-dialog-machines");
    let artifacts = TempDir::new("new-dialog-artifacts");
    let mut harness = manager_harness_with_artifacts(
        Some(machines.path().to_path_buf()),
        Some(artifacts.path().to_path_buf()),
        Vec::new(),
    );
    let eou = eou_images("new-dialog-eou");
    harness.state_mut().eou_dir = Some(eou.path().to_path_buf());

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
    let machines = TempDir::new("new-dialog-cancel-machines");
    let mut harness = manager_harness(Some(machines.path().to_path_buf()), Vec::new());

    click(&mut harness, "New");
    click(&mut harness, Preset::CoCo3DiskBasic.name());
    click(&mut harness, "Cancel");

    assert!(harness.query_by_label(NEW_DIALOG_TITLE).is_none());
    assert!(harness.state().entries.is_empty());
    assert_eq!(std::fs::read_dir(machines.path()).unwrap().count(), 0);
}

/// Without the NitrOS-9 EOU images, the dialog turns the template off and
/// offers their download beneath it; the other templates stay available.
#[test]
fn new_dialog_offers_the_eou_download_when_images_are_missing() {
    let machines = TempDir::new("new-dialog-no-eou-machines");
    let eou = TempDir::new("new-dialog-no-eou");
    let mut harness =
        manager_harness_with_artifacts(Some(machines.path().to_path_buf()), None, Vec::new());
    harness.state_mut().eou_dir = Some(eou.path().to_path_buf());

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

/// While a NitrOS-9 EOU machine is being written, the dialog turns the
/// template off and shows a spinner beneath it, so it can't be picked again
/// meanwhile.
#[test]
fn new_dialog_shows_a_running_eou_install() {
    let machines = TempDir::new("new-dialog-eou-install-machines");
    let eou = eou_images("new-dialog-eou-install");
    let mut harness =
        manager_harness_with_artifacts(Some(machines.path().to_path_buf()), None, Vec::new());
    let (_tx, rx) = std::sync::mpsc::channel();
    harness.state_mut().eou_dir = Some(eou.path().to_path_buf());
    harness.state_mut().eou_install = Some(PendingInstall::new("nitros-9-eou".to_string(), rx));

    click(&mut harness, "New");

    harness.get_by_label(COPYING_TEXT);
    let eou_choice = harness.get_by_label(Preset::NitrOS9Eou.name());
    assert!(eou_choice.accesskit_node().is_disabled());
    let disk_basic = harness.get_by_label(Preset::CoCo3DiskBasic.name());
    assert!(!disk_basic.accesskit_node().is_disabled());
}
