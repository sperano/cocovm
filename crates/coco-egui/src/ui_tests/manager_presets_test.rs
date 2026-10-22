//! The toolbar's New menu: the arrow beside New lists the built-in machine
//! templates, and picking one creates that machine on the spot.

use egui_kittest::kittest::Queryable;

use crate::machine_def::tests::TempDir;
use crate::manager::TEMPLATES_LABEL;
use crate::manager::eou_install::{COPYING_TEXT, PendingInstall};
use crate::manager::presets::Preset;
use crate::nitros9::EOU_IMAGES;

use super::harness::*;

/// The menu lists every template; picking one saves, inserts, and selects
/// a machine built from it, with its blank floppy in the artifact dir.
#[test]
fn new_menu_creates_a_machine_from_a_template() {
    let machines = TempDir::new("new-menu-machines");
    let artifacts = TempDir::new("new-menu-artifacts");
    let mut harness = manager_harness_with_artifacts(
        Some(machines.path().to_path_buf()),
        Some(artifacts.path().to_path_buf()),
        Vec::new(),
    );

    let eou = TempDir::new("new-menu-eou");
    for name in EOU_IMAGES {
        std::fs::write(eou.path().join(name), b"eou image").unwrap();
    }
    harness.state_mut().eou_dir = Some(eou.path().to_path_buf());

    click(&mut harness, TEMPLATES_LABEL);
    for preset in Preset::ALL {
        harness.get_by_label(preset.name());
    }
    click(&mut harness, Preset::CoCo2DiskBasic.name());

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

/// Without the NitrOS-9 EOU images the menu offers their download in place
/// of the template; the other templates stay.
#[test]
fn new_menu_offers_the_eou_download_when_images_are_missing() {
    let machines = TempDir::new("new-menu-no-eou-machines");
    let eou = TempDir::new("new-menu-no-eou");
    let mut harness =
        manager_harness_with_artifacts(Some(machines.path().to_path_buf()), None, Vec::new());
    harness.state_mut().eou_dir = Some(eou.path().to_path_buf());

    click(&mut harness, TEMPLATES_LABEL);

    assert!(harness.query_by_label(Preset::NitrOS9Eou.name()).is_none());
    let download = harness
        .get_all_by_label_contains("Download NitrOS-9")
        .count();
    assert_eq!(download, 1);
    harness.get_by_label(Preset::CoCo3DiskBasic.name());
}

/// While a NitrOS-9 EOU machine is being written, the menu shows a spinner
/// in place of the template, so it can't be picked again meanwhile.
#[test]
fn new_menu_shows_a_running_eou_install() {
    let machines = TempDir::new("new-menu-eou-install-machines");
    let eou = TempDir::new("new-menu-eou-install");
    for name in EOU_IMAGES {
        std::fs::write(eou.path().join(name), b"eou image").unwrap();
    }
    let mut harness =
        manager_harness_with_artifacts(Some(machines.path().to_path_buf()), None, Vec::new());
    let (_tx, rx) = std::sync::mpsc::channel();
    harness.state_mut().eou_dir = Some(eou.path().to_path_buf());
    harness.state_mut().eou_install = Some(PendingInstall::new("nitros-9-eou".to_string(), rx));

    click(&mut harness, TEMPLATES_LABEL);

    harness.get_by_label(COPYING_TEXT);
    assert!(harness.query_by_label(Preset::NitrOS9Eou.name()).is_none());
    harness.get_by_label(Preset::CoCo3DiskBasic.name());
}
