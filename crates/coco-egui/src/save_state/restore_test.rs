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
