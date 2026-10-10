//! DriveWire image leases and share installation on a running `CocoApp`.

use std::fs;
use std::path::{Path, PathBuf};

use coco_core::drivewire::share::{ShareAccess, ShareSpec, ShareTable};

use crate::machine_def::tests::TempDir;
use crate::save_state::tests::boot_app;
use crate::*;

fn write_disk(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, vec![0_u8; drivewire::SECTOR_SIZE]).unwrap();
    path
}

fn settings(disk0: Option<PathBuf>) -> Option<app::DriveWireLaunch> {
    Some(app::DriveWireLaunch {
        hdbdos_mode: false,
        disk_paths: [disk0, None, None, None],
    })
}

fn shares(root: &Path) -> ShareTable {
    ShareTable::new(vec![ShareSpec {
        name: "games".to_string(),
        root: root.to_path_buf(),
        access: ShareAccess::ReadOnly,
    }])
    .unwrap()
}

#[test]
fn a_second_vm_cannot_mount_an_image_the_first_has_mounted() {
    let dir = TempDir::new("dw-lease-conflict");
    let image = write_disk(dir.path(), "shared.dsk");
    let mut first = boot_app();
    let mut second = boot_app();
    first
        .apply_drivewire_settings(settings(Some(image.clone())))
        .unwrap();

    let error = second
        .apply_drivewire_settings(settings(Some(image.clone())))
        .unwrap_err();
    assert!(error.contains("in use by another running VM"), "{error}");
    assert_eq!(second.dw_paths[0], None);

    first.apply_drivewire_settings(settings(None)).unwrap();
    second
        .apply_drivewire_settings(settings(Some(image.clone())))
        .unwrap();
    assert_eq!(second.dw_paths[0], Some(image));
}

#[test]
fn one_vm_may_remount_its_own_image_and_disabling_releases_it() {
    let dir = TempDir::new("dw-lease-self");
    let image = write_disk(dir.path(), "own.dsk");
    let mut first = boot_app();
    first
        .apply_drivewire_settings(settings(Some(image.clone())))
        .unwrap();
    first
        .apply_drivewire_settings(Some(app::DriveWireLaunch {
            hdbdos_mode: false,
            disk_paths: [Some(image.clone()), Some(image.clone()), None, None],
        }))
        .unwrap();
    assert!(first.dw_leases.iter().take(2).all(Option::is_some));

    first.apply_drivewire_settings(None).unwrap();
    assert!(first.dw_leases.iter().all(Option::is_none));
    let mut second = boot_app();
    second
        .apply_drivewire_settings(settings(Some(image)))
        .unwrap();
}

#[test]
fn dropping_a_vm_releases_its_images() {
    let dir = TempDir::new("dw-lease-drop");
    let image = write_disk(dir.path(), "dropped.dsk");
    let mut first = boot_app();
    first
        .apply_drivewire_settings(settings(Some(image.clone())))
        .unwrap();
    drop(first);
    let mut second = boot_app();
    second
        .apply_drivewire_settings(settings(Some(image)))
        .unwrap();
}

#[test]
fn shares_wait_for_drivewire_and_install_when_it_is_enabled() {
    let dir = TempDir::new("dw-shares-install");
    let mut app = boot_app();
    app.set_drivewire_shares(shares(dir.path()));
    assert!(app.machine.bus.drivewire.is_none());

    app.apply_drivewire_settings(settings(None)).unwrap();
    let dw = app.machine.bus.drivewire.as_ref().unwrap();
    assert_eq!(dw.share_session().table(), &shares(dir.path()));
    assert_eq!(dw.share_session().owner(), app.lease_owner);
}
