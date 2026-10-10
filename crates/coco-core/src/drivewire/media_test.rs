use std::fs;

use crate::drivewire::command::tests::Guest;
use crate::drivewire::share::ShareAccess;
use crate::drivewire::share::tests::ScratchDir;
use crate::drivewire::{
    DWImage, DWServer, DriveMedia, GuestMediaChange, MediaOrigin, SECTOR_SIZE, checksum_of, error,
    opcode,
};

/// Sectors in the test images.
const IMAGE_SECTORS: usize = 4;
const DRIVE: usize = 1;
const INSERTED: &str = "OK command successful\n\rDisk inserted in drive 1.\r\n";

/// An image whose sector `n` is filled with byte `fill + n`.
fn image(fill: u8) -> Vec<u8> {
    (0..IMAGE_SECTORS)
        .flat_map(|sector| vec![fill + sector as u8; SECTOR_SIZE])
        .collect()
}

fn sharing(name: &str, access: ShareAccess) -> (ScratchDir, Guest) {
    let dir = ScratchDir::new(name);
    dir.file("disks/a.dsk", &image(0x10));
    dir.file("disks/My Disk.dsk", &image(0x40));
    let guest = Guest::sharing(&[("games", dir.path(), access)]);
    (dir, guest)
}

fn header(drive: usize, lsn: u8) -> [u8; 4] {
    [drive as u8, 0, 0, lsn]
}

/// `OP_READ` through the host worker: the status, then the sector.
fn read_sector(guest: &mut Guest, drive: usize, lsn: u8) -> (u8, Vec<u8>) {
    let mut reply = guest.feed(&[&[opcode::READ][..], &header(drive, lsn)].concat());
    guest.pump();
    reply.extend(guest.feed(&[]));
    let data = reply.get(1..=SECTOR_SIZE).map(<[u8]>::to_vec);
    (reply[0], data.unwrap_or_default())
}

/// `OP_WRITE` of a sector filled with `fill`; returns the status.
fn write_sector(guest: &mut Guest, drive: usize, lsn: u8, fill: u8) -> u8 {
    let sector = [fill; SECTOR_SIZE];
    let checksum = checksum_of(&sector).to_be_bytes();
    let request = [
        &[opcode::WRITE][..],
        &header(drive, lsn),
        &sector,
        &checksum,
    ]
    .concat();
    let mut reply = guest.feed(&request);
    guest.pump();
    reply.extend(guest.feed(&[]));
    reply[0]
}

#[test]
fn insert_mounts_an_image_for_the_session() {
    let (dir, mut guest) = sharing("insert", ShareAccess::ReadWrite);
    assert_eq!(
        guest.run_text("dw disk insert 1 games/disks/a.dsk"),
        INSERTED
    );
    assert_eq!(
        read_sector(&mut guest, DRIVE, 2),
        (error::OK, vec![0x12; SECTOR_SIZE])
    );
    assert_eq!(write_sector(&mut guest, DRIVE, 3, 0xEE), error::OK);
    assert_eq!(
        fs::read(dir.path().join("disks/a.dsk")).unwrap()[3 * SECTOR_SIZE],
        0xEE
    );
    assert_eq!(
        guest.server.drive_media(DRIVE),
        Some(&DriveMedia {
            name: Some("/games/disks/a.dsk".to_string()),
            origin: MediaOrigin::Guest,
            write_protected: false,
        })
    );
    assert_eq!(
        guest.server.take_guest_media_changes(),
        vec![GuestMediaChange {
            drive: DRIVE,
            host_path: Some(dir.path().join("disks/a.dsk")),
        }]
    );
    assert!(guest.server.take_guest_media_changes().is_empty());
}

#[test]
fn an_image_from_a_read_only_share_is_write_protected() {
    let (dir, mut guest) = sharing("insert-ro", ShareAccess::ReadOnly);
    assert_eq!(
        guest.run_text("dw disk insert 1 games/disks/My Disk.dsk"),
        INSERTED
    );
    assert_eq!(
        read_sector(&mut guest, DRIVE, 0),
        (error::OK, vec![0x40; SECTOR_SIZE])
    );
    assert_eq!(write_sector(&mut guest, DRIVE, 0, 0xEE), error::WRITE);
    assert_eq!(
        fs::read(dir.path().join("disks/My Disk.dsk")).unwrap(),
        image(0x40)
    );
    assert_eq!(
        guest.run_text("dw disk show"),
        "OK command successful\n\r\r\nCurrent DriveWire disks:\r\n\r\nX1  */games/disks/My Disk.dsk\r\n"
    );
}

#[test]
fn a_failed_insert_keeps_the_mounted_image() {
    let (_dir, mut guest) = sharing("insert-fail", ShareAccess::ReadWrite);
    guest.server.mount(DRIVE, DWImage::Memory(image(0x70)));
    for (line, reply) in [
        (
            "dw disk insert 1 games/disks/none.dsk",
            "FAIL 203 games/disks/none.dsk: not found\n\r",
        ),
        (
            "dw disk insert 1 games/disks",
            "FAIL 201 games/disks: is a directory\n\r",
        ),
        (
            "dw disk insert 9 games/disks/a.dsk",
            "FAIL 101 There is no drive 9. Valid drive numbers are 0 - 3\n\r",
        ),
    ] {
        assert_eq!(guest.run_text(line), reply, "{line}");
    }
    assert_eq!(
        read_sector(&mut guest, DRIVE, 1),
        (error::OK, vec![0x71; SECTOR_SIZE])
    );
    assert_eq!(
        guest.server.drive_media(DRIVE).unwrap().origin,
        MediaOrigin::Host
    );
    assert!(guest.server.take_guest_media_changes().is_empty());
}

#[test]
fn eject_empties_a_drive_once() {
    let (_dir, mut guest) = sharing("eject", ShareAccess::ReadWrite);
    guest.server.mount(DRIVE, DWImage::Memory(image(0)));
    assert_eq!(
        guest.run_text("dw disk eject 1"),
        "OK command successful\n\rDisk ejected from drive 1.\r\n"
    );
    assert_eq!(read_sector(&mut guest, DRIVE, 0).0, error::NOT_READY);
    assert!(guest.server.guest_changed(DRIVE));
    let saved = serde_json::to_string(&guest.server).unwrap();
    let restored: DWServer = serde_json::from_str(&saved).unwrap();
    assert!(
        restored.guest_changed(DRIVE),
        "the eject survives a snapshot"
    );
    assert_eq!(
        guest.run_text("dw disk eject 1"),
        "FAIL 102 There is no disk in drive 1\n\r"
    );
    assert_eq!(
        guest.server.take_guest_media_changes(),
        vec![GuestMediaChange {
            drive: DRIVE,
            host_path: None,
        }]
    );
}

#[test]
fn eject_all_empties_every_drive() {
    let (_dir, mut guest) = sharing("eject-all", ShareAccess::ReadWrite);
    guest.server.mount(0, DWImage::Memory(image(0)));
    guest.run("dw disk insert 1 games/disks/a.dsk");
    assert_eq!(
        guest.run_text("dw disk eject all"),
        "OK command successful\n\rEjected all disks.\r\n"
    );
    assert!((0..crate::drivewire::DRIVE_COUNT).all(|drive| !guest.server.is_mounted(drive)));
    let changes = guest.server.take_guest_media_changes();
    assert_eq!(changes.len(), 2);
    assert!(changes.iter().all(|change| change.host_path.is_none()));
}

#[test]
fn show_describes_one_drive() {
    let (_dir, mut guest) = sharing("show", ShareAccess::ReadWrite);
    guest.server.mount(0, DWImage::Memory(image(0)));
    guest.server.set_drive_name(0, "system.dsk");
    assert_eq!(
        guest.run_text("dw disk show 0"),
        "OK command successful\n\rDetails for disk in drive #0:\r\n\r\nsystem.dsk\r\n\r\n\
         Mounted by: VM settings\r\nAccess: read/write\r\n"
    );
    assert_eq!(
        guest.run_text("dw disk show 2"),
        "FAIL 102 There is no disk in drive 2\n\r"
    );
}

#[test]
fn a_host_mount_replaces_a_guest_mount_and_its_lease() {
    let (dir, mut guest) = sharing("host-over-guest", ShareAccess::ReadWrite);
    guest.run("dw disk insert 1 games/disks/a.dsk");
    let mut other = Guest::sharing(&[("games", dir.path(), ShareAccess::ReadWrite)]);
    assert_eq!(
        other.run_text("dw disk insert 0 games/disks/a.dsk"),
        "FAIL 202 games/disks/a.dsk: in use by another VM\n\r"
    );
    guest.server.mount(DRIVE, DWImage::Memory(image(0)));
    assert_eq!(
        guest.server.drive_media(DRIVE).unwrap().origin,
        MediaOrigin::Host
    );
    assert!(!guest.server.guest_changed(DRIVE));
    assert_eq!(
        other.run_text("dw disk insert 0 games/disks/a.dsk"),
        "OK command successful\n\rDisk inserted in drive 0.\r\n"
    );
}

#[test]
fn two_vms_can_insert_the_same_read_only_image() {
    let (dir, mut first) = sharing("shared-ro", ShareAccess::ReadOnly);
    let mut second = Guest::sharing(&[("games", dir.path(), ShareAccess::ReadOnly)]);
    assert_eq!(
        first.run_text("dw disk insert 1 games/disks/a.dsk"),
        INSERTED
    );
    assert_eq!(
        second.run_text("dw disk insert 1 games/disks/a.dsk"),
        INSERTED
    );
    second.run("dw disk eject 1");
    assert!(first.server.is_mounted(DRIVE));
}

#[test]
fn a_snapshot_keeps_the_description_and_write_protection() {
    let (dir, mut guest) = sharing("snapshot", ShareAccess::ReadOnly);
    guest.run("dw disk insert 1 games/disks/a.dsk");
    let saved = serde_json::to_string(&guest.server).unwrap();
    let mut restored: DWServer = serde_json::from_str(&saved).unwrap();
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.path().join("disks/a.dsk"))
        .unwrap();
    restored.reattach(DRIVE, DWImage::File(file));
    let media = restored.drive_media(DRIVE).unwrap();
    assert_eq!(media.origin, MediaOrigin::Guest);
    assert!(media.write_protected);
    assert!(restored.guest_changed(DRIVE));
    assert!(restored.take_guest_media_changes().is_empty());
}

#[test]
fn an_old_snapshot_reattaches_as_a_host_mount() {
    let mut server = DWServer::new();
    server.reattach(0, DWImage::Memory(image(0)));
    assert_eq!(server.drive_media(0).unwrap().origin, MediaOrigin::Host);
    server.eject(0);
    assert_eq!(server.drive_media(0), None);
}
