//! Saved `[drivewire]` edits reconcile into a Running VM; Suspended ones wait.

use std::fs;
use std::path::{Path, PathBuf};

use coco_core::MachineConfig;
use coco_core::drivewire::share::ShareOp;

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

/// Writes one zeroed sector to DW0 through the protocol, leaving the drive dirty.
fn write_sector_to_drive0(dw: &mut coco_core::drivewire::DWServer) {
    const HOST_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
    const DRIVE_AND_LSN: [u8; 4] = [0; 4];
    const ZERO_SECTOR_CHECKSUM: [u8; 2] = [0; 2];

    let mut request = vec![coco_core::drivewire::opcode::WRITE];
    request.extend(DRIVE_AND_LSN);
    request.extend([0_u8; coco_core::drivewire::SECTOR_SIZE]);
    request.extend(ZERO_SECTOR_CHECKSUM);
    for (cycle, byte) in request.into_iter().enumerate() {
        dw.data_write(byte, cycle as u64);
    }
    let deadline = std::time::Instant::now() + HOST_WRITE_TIMEOUT;
    while dw.status_read() == 0 {
        assert!(std::time::Instant::now() < deadline, "host write timed out");
        dw.poll_host();
        std::thread::yield_now();
    }
    assert_eq!(dw.data_read(), coco_core::drivewire::error::OK);
}

#[test]
fn a_mode_change_leaves_mounted_drives_untouched() {
    let dir = TempDir::new("live-dw-untouched");
    let disk = write_disk(dir.path(), "live.dsk");
    let mut manager = running_manager(dir.path(), Some(&disk));
    let app = manager.entries[0].vm.as_mut().unwrap();
    app.set_running(false);
    write_sector_to_drive0(app.machine.bus.drivewire.as_mut().unwrap());

    manager.entries[0].def.drivewire.hdbdos_mode = true;
    manager.apply_live_drivewire(0);

    let dw = vm(&manager).machine.bus.drivewire.as_ref().unwrap();
    assert!(dw.hdbdos_mode());
    // A remount would have cleared the dirty flag.
    assert!(dw.dirty(0));
}

fn share(
    name: &str,
    root: &Path,
    access: machine_def::ShareAccessDTO,
) -> machine_def::DriveWireShareDTO {
    machine_def::DriveWireShareDTO {
        name: name.to_string(),
        path: root.display().to_string(),
        access,
    }
}

fn share_cwd(manager: &ManagerApp, index: usize) -> String {
    manager.entries[index]
        .share_status()
        .expect("share session")
        .cwd
}

fn change_dir(manager: &ManagerApp, index: usize, path: &str) {
    let dw = manager.entries[index]
        .vm
        .as_ref()
        .unwrap()
        .machine
        .bus
        .drivewire
        .as_ref();
    dw.unwrap()
        .share_session()
        .execute(ShareOp::ChangeDir {
            path: path.as_bytes().to_vec(),
        })
        .unwrap();
}

#[test]
fn saved_shares_reach_the_running_vm_and_unrelated_saves_keep_its_session() {
    let dir = TempDir::new("live-dw-shares");
    let root = dir.path().join("games");
    fs::create_dir_all(root.join("sub")).unwrap();
    let mut manager = running_manager(dir.path(), None);
    let drivewire = &mut manager.entries[0].def.drivewire;
    drivewire.enabled = true;
    drivewire.shares = vec![share("games", &root, machine_def::ShareAccessDTO::ReadOnly)];
    manager.apply_live_drivewire(0);
    assert!(manager.entries[0].launch_error.is_none());
    change_dir(&manager, 0, "games/sub");

    manager.entries[0].def.drivewire.hdbdos_mode = true;
    manager.apply_live_drivewire(0);
    assert_eq!(share_cwd(&manager, 0), "/games/sub");

    manager.entries[0].def.drivewire.shares[0].access = machine_def::ShareAccessDTO::ReadWrite;
    manager.apply_live_drivewire(0);
    assert_eq!(
        share_cwd(&manager, 0),
        "/",
        "a changed share starts a fresh session"
    );
}

#[test]
fn an_invalid_share_list_is_reported_without_touching_the_session() {
    let dir = TempDir::new("live-dw-shares-invalid");
    let mut manager = running_manager(dir.path(), None);
    let drivewire = &mut manager.entries[0].def.drivewire;
    drivewire.enabled = true;
    drivewire.shares = vec![
        share("games", dir.path(), machine_def::ShareAccessDTO::ReadOnly),
        share("GAMES", dir.path(), machine_def::ShareAccessDTO::ReadOnly),
    ];
    manager.apply_live_drivewire(0);
    let error = manager.entries[0].launch_error.as_deref().unwrap();
    assert!(error.contains("used twice"), "{error}");
}

#[test]
fn two_running_vms_share_a_root_or_use_their_own() {
    let dir = TempDir::new("live-dw-two-vms");
    let common = dir.path().join("common");
    let private = dir.path().join("private");
    fs::create_dir_all(common.join("a")).unwrap();
    fs::create_dir_all(common.join("b")).unwrap();
    fs::create_dir_all(&private).unwrap();
    let mut defs = Vec::new();
    for (name, extra) in [("vm-a", None), ("vm-b", Some(&private))] {
        let mut def =
            machine_def::MachineDef::from_config(name.to_string(), None, &MachineConfig::default());
        def.drivewire.enabled = true;
        def.drivewire.shares = vec![share(
            "common",
            &common,
            machine_def::ShareAccessDTO::ReadOnly,
        )];
        if let Some(root) = extra {
            def.drivewire
                .shares
                .push(share("mine", root, machine_def::ShareAccessDTO::ReadWrite));
        }
        defs.push(MachineEntry::new(name.to_string(), def));
    }
    let mut manager = ManagerApp::new(
        None,
        Some(dir.path().join("machines")),
        Some(dir.path().join("artifacts")),
        defs,
        None,
    );
    manager.start_vm(0);
    manager.start_vm(1);

    change_dir(&manager, 0, "common/a");
    change_dir(&manager, 1, "common/b");
    assert_eq!(share_cwd(&manager, 0), "/common/a");
    assert_eq!(share_cwd(&manager, 1), "/common/b");
    let table = |index: usize| {
        let dw = manager.entries[index]
            .vm
            .as_ref()
            .unwrap()
            .machine
            .bus
            .drivewire
            .as_ref();
        dw.unwrap().share_session().table().clone()
    };
    assert!(table(0).get("mine").is_none());
    assert!(table(1).get("mine").is_some());
}

#[test]
fn a_second_vm_cannot_start_with_an_image_the_first_has_mounted() {
    let dir = TempDir::new("live-dw-image-conflict");
    let disk = write_disk(dir.path(), "shared.dsk");
    let mut manager = running_manager(dir.path(), Some(&disk));
    let mut def = manager.entries[0].def.clone();
    def.name = "Second".to_string();
    manager
        .entries
        .push(MachineEntry::new("second".to_string(), def));

    manager.start_vm(1);
    let error = manager.entries[1].launch_error.as_deref().unwrap();
    assert!(error.contains("in use by another running VM"), "{error}");
    assert!(manager.entries[1].vm.is_none());
}
