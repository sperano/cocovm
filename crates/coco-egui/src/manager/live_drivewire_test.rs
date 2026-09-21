//! Saved `[drivewire]` edits reconcile into a Running VM; Suspended ones wait.

use std::fs;
use std::path::{Path, PathBuf};

use coco_core::MachineConfig;

use super::*;
use crate::machine_def::{self, tests::TempDir};
use crate::manager::MachineEntry;

const TEST_SLUG: &str = "drivewire-live";

fn write_disk(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, vec![0_u8; coco_core::drivewire::SECTOR_SIZE]).unwrap();
    path
}

/// A started manager whose one machine boots with `disk0` on DW0, if any.
fn running_manager(dir: &Path, disk0: Option<&Path>) -> ManagerApp {
    let mut def = machine_def::MachineDef::from_config(
        "DriveWire live".to_string(),
        None,
        &MachineConfig::default(),
    );
    def.drivewire.enabled = disk0.is_some();
    def.drivewire.disk0 = disk0.map(|path| path.display().to_string());
    let mut manager = ManagerApp::new(
        None,
        Some(dir.join("machines")),
        Some(dir.join("artifacts")),
        vec![MachineEntry::new(TEST_SLUG.to_string(), def)],
        None,
    );
    manager.start_vm(0);
    assert!(manager.entries[0].launch_error.is_none());
    manager
}

fn vm(manager: &ManagerApp) -> &crate::CocoApp {
    manager.entries[0].vm.as_ref().expect("VM is alive")
}

#[test]
fn enabling_mounts_the_saved_disks_in_the_running_vm() {
    let dir = TempDir::new("live-dw-enable");
    let disk = write_disk(dir.path(), "live.dsk");
    let mut manager = running_manager(dir.path(), None);
    assert!(vm(&manager).machine.bus.drivewire.is_none());

    let drivewire = &mut manager.entries[0].def.drivewire;
    drivewire.enabled = true;
    drivewire.hdbdos_mode = true;
    drivewire.disk1 = Some(disk.display().to_string());
    manager.apply_live_drivewire(0);

    assert!(manager.entries[0].launch_error.is_none());
    let dw = vm(&manager).machine.bus.drivewire.as_ref().unwrap();
    assert!(dw.hdbdos_mode());
    assert!(!dw.is_mounted(0));
    assert!(dw.is_mounted(1));
    assert_eq!(vm(&manager).dw_paths[1], Some(disk));
}

#[test]
fn swapping_and_clearing_a_path_remounts_and_ejects() {
    let dir = TempDir::new("live-dw-swap");
    let first = write_disk(dir.path(), "first.dsk");
    let second = write_disk(dir.path(), "second.dsk");
    let mut manager = running_manager(dir.path(), Some(&first));

    manager.entries[0].def.drivewire.disk0 = Some(second.display().to_string());
    manager.apply_live_drivewire(0);
    assert_eq!(vm(&manager).dw_paths[0], Some(second));

    manager.entries[0].def.drivewire.disk0 = None;
    manager.apply_live_drivewire(0);
    let dw = vm(&manager).machine.bus.drivewire.as_ref().unwrap();
    assert!(!dw.is_mounted(0));
    assert_eq!(vm(&manager).dw_paths[0], None);
}

#[test]
fn disabling_removes_the_becker_port() {
    let dir = TempDir::new("live-dw-disable");
    let disk = write_disk(dir.path(), "live.dsk");
    let mut manager = running_manager(dir.path(), Some(&disk));

    manager.entries[0].def.drivewire.enabled = false;
    manager.apply_live_drivewire(0);

    assert!(vm(&manager).machine.bus.drivewire.is_none());
    assert_eq!(vm(&manager).dw_paths[0], None);
}

#[test]
fn an_unopenable_path_keeps_the_current_disk_and_reports_why() {
    let dir = TempDir::new("live-dw-missing");
    let disk = write_disk(dir.path(), "live.dsk");
    let mut manager = running_manager(dir.path(), Some(&disk));

    manager.entries[0].def.drivewire.disk0 =
        Some(dir.path().join("missing.dsk").display().to_string());
    manager.apply_live_drivewire(0);

    let error = manager.entries[0].launch_error.as_deref().unwrap();
    assert!(error.contains("missing.dsk"), "unexpected error: {error}");
    assert_eq!(vm(&manager).dw_paths[0], Some(disk.clone()));

    manager.entries[0].def.drivewire.disk0 = Some(disk.display().to_string());
    manager.apply_live_drivewire(0);
    assert!(manager.entries[0].launch_error.is_none());
}

#[test]
fn a_suspended_machine_keeps_its_saved_session() {
    let dir = TempDir::new("live-dw-suspended");
    let disk = write_disk(dir.path(), "live.dsk");
    let mut manager = running_manager(dir.path(), Some(&disk));
    manager.suspend_vm(0);
    assert!(manager.entries[0].suspended);

    manager.entries[0].def.drivewire.enabled = false;
    manager.apply_live_drivewire(0);

    assert!(vm(&manager).machine.bus.drivewire.is_some());
    assert_eq!(vm(&manager).dw_paths[0], Some(disk));
}
