//! Restoring a snapshot re-leases its DriveWire images and reinstalls shares.

use std::fs;
use std::path::{Path, PathBuf};

use coco_core::drivewire::share::{ShareAccess, ShareSpec, ShareTable};

use crate::app::DriveWireLaunch;
use crate::machine_def::tests::TempDir;
use crate::save_state::tests::boot_app;
use crate::*;

fn write_disk(dir: &Path) -> PathBuf {
    let path = dir.join("snapshot.dsk");
    fs::write(&path, vec![0_u8; drivewire::SECTOR_SIZE]).unwrap();
    path
}

fn app_with_image(image: &Path) -> CocoApp {
    let mut app = boot_app();
    app.apply_drivewire_settings(Some(DriveWireLaunch {
        hdbdos_mode: false,
        disk_paths: [Some(image.to_path_buf()), None, None, None],
    }))
    .unwrap();
    app
}

#[test]
fn restoring_onto_an_image_another_vm_holds_ejects_it_with_a_note() {
    let dir = TempDir::new("restore-dw-busy");
    let image = write_disk(dir.path());
    let state = dir.path().join("busy.ccstate");
    let mut saver = app_with_image(&image);
    saver.save_state_to(&state).unwrap();

    let mut restorer = boot_app();
    let notes = restorer.restore_state_from(&state).unwrap();
    assert!(
        notes
            .iter()
            .any(|note| note.contains("in use by another running VM")),
        "{notes:?}"
    );
    assert_eq!(restorer.dw_paths[0], None);
    assert!(
        !restorer
            .machine
            .bus
            .drivewire
            .as_ref()
            .unwrap()
            .is_mounted(0)
    );
}

#[test]
fn restoring_its_own_snapshot_keeps_the_image_and_the_shares() {
    let dir = TempDir::new("restore-dw-own");
    let image = write_disk(dir.path());
    let state = dir.path().join("own.ccstate");
    let mut app = app_with_image(&image);
    let shares = ShareTable::new(vec![ShareSpec {
        name: "games".to_string(),
        root: dir.path().to_path_buf(),
        access: ShareAccess::ReadWrite,
    }])
    .unwrap();
    app.set_drivewire_shares(shares.clone());
    app.save_state_to(&state).unwrap();

    let notes = app.restore_state_from(&state).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(app.dw_paths[0], Some(image));
    assert!(app.dw_leases[0].is_some());
    let dw = app.machine.bus.drivewire.as_ref().unwrap();
    assert!(dw.is_mounted(0));
    assert_eq!(dw.share_session().table(), &shares);
}

#[cfg(unix)]
#[test]
fn a_restored_guest_image_survives_an_unrelated_settings_save() {
    use std::os::unix::fs::PermissionsExt;

    use crate::media::drivewire::tests::guest_command;

    let dir = TempDir::new("restore-dw-guest");
    let startup = write_disk(dir.path());
    let pristine = dir.path().join("pristine.dsk");
    fs::write(&pristine, vec![0_u8; drivewire::SECTOR_SIZE]).unwrap();
    // The host itself cannot write this image: only a read-only reopen works.
    fs::set_permissions(&pristine, fs::Permissions::from_mode(0o444)).unwrap();
    let state = dir.path().join("guest.ccstate");
    let mut app = app_with_image(&startup);
    let shares = ShareTable::new(vec![ShareSpec {
        name: "games".to_string(),
        root: dir.path().to_path_buf(),
        access: ShareAccess::ReadOnly,
    }])
    .unwrap();
    app.set_drivewire_shares(shares);
    guest_command(&mut app, "dw disk insert 0 games/pristine.dsk");
    app.save_state_to(&state).unwrap();

    let notes = app.restore_state_from(&state).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(app.dw_paths[0], Some(pristine.clone()));
    let dw = app.machine.bus.drivewire.as_ref().unwrap();
    assert!(dw.drive_write_protected(0));
    assert!(dw.guest_changed(0));

    // Saving the unchanged startup path, as a shares edit does, keeps it.
    app.apply_drivewire_settings(Some(DriveWireLaunch {
        hdbdos_mode: false,
        disk_paths: [Some(startup), None, None, None],
    }))
    .unwrap();
    assert_eq!(app.dw_paths[0], Some(pristine));
}
