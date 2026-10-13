//! Restoring a snapshot re-leases its DriveWire images and reinstalls shares.

use std::fs;
use std::path::{Path, PathBuf};

use coco_core::drivewire::share::{ShareAccess, ShareSpec, ShareTable};
use coco_core::drivewire::{PrintFormat, PrintOutput};

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

#[test]
fn restoring_its_own_snapshot_keeps_the_printer_output() {
    let dir = TempDir::new("restore-dw-printer");
    let image = write_disk(dir.path());
    let state = dir.path().join("printer.ccstate");
    let mut app = app_with_image(&image);
    let folder = dir.path().join("prints");
    app.set_drivewire_printer(PrintOutput::Folder {
        dir: folder.clone(),
        format: PrintFormat::Text,
    });
    app.save_state_to(&state).unwrap();

    let notes = app.restore_state_from(&state).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(
        app.machine.bus.drivewire.as_ref().unwrap().print_output(),
        &PrintOutput::Folder {
            dir: folder,
            format: PrintFormat::Text,
        }
    );
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

/// Longest wait for the DriveWire host worker in these tests.
const NAMED_WORKER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Mounts the named object `name` as the guest would; returns the reply.
fn named_mount(app: &mut CocoApp, name: &str) -> u8 {
    let dw = app.machine.bus.drivewire.as_mut().unwrap();
    let mut request = vec![drivewire::opcode::NAMEOBJ_MOUNT, name.len() as u8];
    request.extend(name.bytes());
    for byte in request {
        dw.data_write(byte, 0);
    }
    let started = std::time::Instant::now();
    while dw.status_read() == 0 {
        assert!(started.elapsed() < NAMED_WORKER_TIMEOUT, "no reply");
        dw.poll_host();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    dw.data_read()
}

fn app_with_named_objects(dir: &Path) -> CocoApp {
    let mut app = app_with_image(&write_disk(dir));
    let shares = ShareTable::new(vec![ShareSpec {
        name: "objects".to_string(),
        root: dir.to_path_buf(),
        access: ShareAccess::ReadOnly,
    }])
    .unwrap()
    .with_named_objects(Some("objects".to_string()));
    app.set_drivewire_shares(shares);
    app
}

#[test]
fn restoring_reopens_named_objects_and_notes_the_missing_ones() {
    let dir = TempDir::new("restore-dw-named");
    fs::write(dir.path().join("kept.bin"), b"kept").unwrap();
    let gone = dir.path().join("gone.bin");
    fs::write(&gone, b"gone").unwrap();
    let state = dir.path().join("named.ccstate");
    let mut app = app_with_named_objects(dir.path());
    let kept = named_mount(&mut app, "kept.bin");
    let removed = named_mount(&mut app, "gone.bin");
    assert_ne!(kept, drivewire::NAMED_FAILURE);
    assert_ne!(removed, drivewire::NAMED_FAILURE);
    app.save_state_to(&state).unwrap();
    fs::remove_file(&gone).unwrap();

    let notes = app.restore_state_from(&state).unwrap();
    assert_eq!(
        notes,
        [format!(
            "DriveWire named object \"gone.bin\" (DW{removed}) not reopened: not found"
        )]
    );
    let objects = app.machine.bus.drivewire.as_ref().unwrap().named_objects();
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].drive, kept);
    assert!(objects[0].attached && objects[0].write_protected);
    assert!(!objects[0].leased, "the lease was on the dropped object");
}
