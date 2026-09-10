//! The VM window's status-bar disk entry: absent without an FD-502, a "No disks"
//! placeholder once one is installed, and one readout per mounted disk after that.

use egui_kittest::kittest::Queryable;

use crate::chrome::status_bar::{NO_DISKS_HOVER, NO_DISKS_READOUT as NO_DISKS};
use crate::machine_def::tests::TempDir;

use super::harness::*;

/// Installs an FD-502 in a booted harness (there is no runtime menu for that).
fn harness_with_fd502() -> AppHarness {
    let mut harness = boot_harness();
    harness
        .state_mut()
        .insert_disk_controller()
        .unwrap_or_else(|e| panic!("insert_disk_controller failed: {e}"));
    harness.step();
    harness
}

/// Without a controller there are no drives to report on, so no entry at all.
#[test]
fn no_disk_entry_without_an_fd502() {
    let harness = boot_harness();
    assert!(harness.query_by_label(NO_DISKS).is_none());
}

/// An installed FD-502 with nothing mounted still shows its icon — the drives exist and a
/// disk can be inserted later from the Machine menu.
#[test]
fn fd502_without_disks_shows_a_no_disks_entry() {
    let harness = harness_with_fd502();
    harness.get_by_label(NO_DISKS);
}

/// Mounting a disk replaces the placeholder with the per-drive readout; ejecting it brings
/// the placeholder back.
#[test]
fn mounting_a_disk_replaces_the_no_disks_entry() {
    let mut harness = harness_with_fd502();
    let dir = TempDir::new("status-bar-no-disks");
    let disk = dir.path().join("blank.dsk");
    harness.state_mut().new_blank_disk(0, disk);
    harness.step();

    harness.get_by_label("D0: blank.dsk");
    assert!(harness.query_by_label(NO_DISKS).is_none());

    harness.state_mut().eject_disk(0);
    harness.step();
    harness.get_by_label(NO_DISKS);
    assert!(harness.query_by_label("D0: blank.dsk").is_none());
}

/// Under `status_bar_icons_only` the placeholder folds into the icon's accessible name,
/// like every other passive readout — and the hover text stacked after it does not.
#[test]
fn icons_only_no_disks_readout_becomes_the_icons_accessible_name() {
    let mut harness = harness_with_fd502();
    harness.state_mut().status_bar_icons_only = true;
    harness.step();
    harness.get_by_label(NO_DISKS);
    assert!(
        harness.query_by_label(NO_DISKS_HOVER).is_none(),
        "the hover text must not become the accessible name"
    );
}
