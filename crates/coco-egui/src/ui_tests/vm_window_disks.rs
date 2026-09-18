//! The VM window's status-bar disk entries: absent without an FD-502, a "No disks"
//! placeholder once one is installed, and one entry per mounted disk after that — each a
//! click target for its drive menu (insert, new blank, eject).

use egui_kittest::kittest::{NodeT, Queryable};

use crate::UI_DRIVES;
use crate::chrome::status_bar::{NO_DISKS_HOVER, NO_DISKS_READOUT as NO_DISKS};
use crate::machine_def::tests::TempDir;

use super::harness::*;

/// Without a controller there are no drives to report on, so no entry at all.
#[test]
fn no_disk_entry_without_an_fd502() {
    let harness = boot_harness();
    assert!(harness.query_by_label(NO_DISKS).is_none());
}

/// An installed FD-502 with nothing mounted still shows its icon — the drives exist and a
/// disk can be inserted later from its menu.
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

/// Under `status_bar_icons_only` the placeholder folds into the icon's hover text; the
/// icon keeps its own accessible name, like every other menu entry.
#[test]
fn icons_only_no_disks_entry_keeps_its_menu_name() {
    let mut harness = harness_with_fd502();
    harness.state_mut().status_bar_icons_only = true;
    harness.step();
    harness.get_by_label("Disks menu");
    assert!(
        harness.query_by_label(NO_DISKS).is_none(),
        "the readout must fold into hover text, not stay a label"
    );
    assert!(
        harness.query_by_label(NO_DISKS_HOVER).is_none(),
        "the hover text must not become the accessible name"
    );
}

/// The placeholder's menu lists insert and new-blank for every drive, with eject disabled
/// since nothing is mounted.
#[test]
fn disks_menu_offers_insert_and_new_blank_per_drive() {
    let mut harness = harness_with_fd502();

    click(&mut harness, "Disks menu");
    for drive in 0..UI_DRIVES {
        for label in [
            format!("Insert Disk in Drive {drive}…"),
            format!("New Blank Disk in Drive {drive}…"),
        ] {
            assert!(
                !harness.get_by_label(&label).accesskit_node().is_disabled(),
                "{label} should be enabled with an FD-502 installed"
            );
        }
        assert!(
            harness
                .get_by_label(&format!("Eject Drive {drive}"))
                .accesskit_node()
                .is_disabled(),
            "Eject Drive {drive} should be disabled with nothing mounted"
        );
    }
}

/// A mounted drive's entry opens a menu for that drive alone, with an enabled eject
/// naming the image; ejecting from it restores the placeholder.
#[test]
fn mounted_drive_entry_opens_its_own_drive_menu() {
    let mut harness = harness_with_fd502();
    let dir = TempDir::new("status-bar-drive-menu");
    harness
        .state_mut()
        .new_blank_disk(1, dir.path().join("blank.dsk"));
    harness.step();

    click(&mut harness, "Drive 1 menu");
    harness.get_by_label("Insert Disk in Drive 1…");
    assert!(
        harness.query_by_label("Insert Disk in Drive 0…").is_none(),
        "drive 1's menu must not list drive 0's items"
    );
    click(&mut harness, "Eject Drive 1 (blank.dsk)");
    harness.get_by_label(NO_DISKS);
    assert!(harness.state().disk_paths[1].is_none());
}
