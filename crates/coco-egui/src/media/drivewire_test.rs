//! DriveWire image leases and share installation on a running `CocoApp`.

use std::fs;
use std::path::{Path, PathBuf};

use coco_core::drivewire::share::{AccessMode, ShareAccess, ShareSpec, ShareTable};

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

/// `SS.Open`/`SS.Close`, the SCF statcodes `scdwv` sends with
/// `OP_SERSETSTAT` when a path opens or closes on a channel (`scf.d`).
const SS_OPEN: u8 = 0x29;
const SS_CLOSE: u8 = 0x2A;
/// The virtual channel the simulated guest uses.
const CHANNEL: u8 = 1;
/// Generous bound for the host worker to finish a small job.
const WORKER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn rw_shares(root: &Path) -> ShareTable {
    ShareTable::new(vec![ShareSpec {
        name: "games".to_string(),
        root: root.to_path_buf(),
        access: ShareAccess::ReadWrite,
    }])
    .unwrap()
}

/// Runs `line` as the guest's `dw` utility would, waiting for the hangup.
pub(crate) fn guest_command(app: &mut CocoApp, line: &str) {
    use drivewire::opcode;
    let dw = app.machine.bus.drivewire.as_mut().unwrap();
    let mut request = vec![opcode::SERSETSTAT, CHANNEL, SS_OPEN];
    let line = format!("{line}\r");
    request.extend([opcode::SERWRITEM, CHANNEL, line.len() as u8]);
    request.extend(line.bytes());
    for byte in request {
        dw.data_write(byte, 0);
    }
    let started = std::time::Instant::now();
    while !dw.channel_info(CHANNEL).unwrap().closing {
        assert!(
            started.elapsed() < WORKER_TIMEOUT,
            "{line:?} did not finish"
        );
        dw.poll_host();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    for byte in [opcode::SERSETSTAT, CHANNEL, SS_CLOSE] {
        dw.data_write(byte, 0);
    }
    app.poll_drivewire_host();
}

#[test]
fn a_guest_insert_is_session_media_that_unchanged_settings_keep() {
    let dir = TempDir::new("dw-guest-insert");
    let startup = write_disk(dir.path(), "startup.dsk");
    let guest = write_disk(dir.path(), "guest.dsk");
    let mut app = boot_app();
    app.apply_drivewire_settings(settings(Some(startup.clone())))
        .unwrap();
    app.set_drivewire_shares(rw_shares(dir.path()));

    guest_command(&mut app, "dw disk insert 0 games/guest.dsk");
    assert_eq!(app.dw_paths[0], Some(guest.clone()));
    assert!(
        app.dw_leases[0].is_none(),
        "the server holds the guest lease"
    );
    assert_eq!(app.dw_startup[0], Some(startup.clone()));

    app.apply_drivewire_settings(settings(Some(startup.clone())))
        .unwrap();
    assert_eq!(
        app.dw_paths[0],
        Some(guest),
        "an unchanged setting keeps it"
    );
    let other = write_disk(dir.path(), "other.dsk");
    app.apply_drivewire_settings(settings(Some(other.clone())))
        .unwrap();
    assert_eq!(
        app.dw_paths[0],
        Some(other),
        "a changed setting replaces it"
    );
}

#[test]
fn a_guest_eject_releases_the_startup_image() {
    let dir = TempDir::new("dw-guest-eject");
    let startup = write_disk(dir.path(), "startup.dsk");
    let mut app = boot_app();
    app.apply_drivewire_settings(settings(Some(startup.clone())))
        .unwrap();

    guest_command(&mut app, "dw disk eject 0");
    assert_eq!(app.dw_paths[0], None);
    assert!(app.dw_leases[0].is_none());
    let mut second = boot_app();
    second
        .apply_drivewire_settings(settings(Some(startup.clone())))
        .unwrap();
    app.apply_drivewire_settings(settings(Some(startup)))
        .unwrap();
    assert_eq!(app.dw_paths[0], None, "the guest's eject lasts the session");
}

#[test]
fn a_restored_read_only_guest_image_takes_a_read_lease() {
    let dir = TempDir::new("dw-guest-restore");
    write_disk(dir.path(), "ro.dsk");
    let mut app = boot_app();
    app.apply_drivewire_settings(settings(None)).unwrap();
    app.set_drivewire_shares(shares(dir.path()));
    guest_command(&mut app, "dw disk insert 2 games/ro.dsk");
    let dw = app.machine.bus.drivewire.as_ref();
    assert_eq!(super::restored_lease_mode(dw, 2), AccessMode::Read);
    assert_eq!(super::restored_lease_mode(dw, 0), AccessMode::Write);
    assert_eq!(
        super::guest_media_note(dw.unwrap(), 2),
        "\nInserted by the guest, read-only"
    );
}
