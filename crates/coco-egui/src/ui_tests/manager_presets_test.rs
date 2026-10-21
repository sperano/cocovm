//! The toolbar's New menu: the arrow beside New lists the built-in machine
//! templates, and picking one creates that machine on the spot.

use egui_kittest::kittest::Queryable;

use crate::machine_def::tests::TempDir;
use crate::manager::presets::Preset;
use crate::manager::toolbar::TEMPLATES_LABEL;

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
