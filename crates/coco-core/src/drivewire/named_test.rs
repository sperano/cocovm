use std::fs;
use std::path::Path;

use super::*;
use crate::drivewire::command::tests::Guest;
use crate::drivewire::share::tests::{ScratchDir, table};
use crate::drivewire::share::{LeaseOwner, ShareAccess};
use crate::drivewire::{
    HDBDOS_SECTORS_PER_DISK, SECTOR_SIZE, TRANSACTION_TIMEOUT_CYCLES, checksum_of, error, opcode,
};

const SHARE: &str = "games";
/// Bytes past one whole sector, so the object ends in a partial sector.
const PARTIAL_TAIL: usize = 44;
const OBJECT_LEN: usize = SECTOR_SIZE + PARTIAL_TAIL;
/// A drive the HDB-DOS remapping would reach for an LSN on "disk" 255.
const HDBDOS_DISK_255_LSN: u64 = 255 * HDBDOS_SECTORS_PER_DISK;
/// `OP_TIME`'s reply: year, month, day, hour, minute, second.
const TIME_REPLY_LEN: usize = 6;

fn object_bytes(seed: u8, len: usize) -> Vec<u8> {
    (0..len).map(|i| seed.wrapping_add(i as u8)).collect()
}

/// A guest whose VM serves named objects from `root` with `access`.
fn guest(root: &Path, access: ShareAccess) -> Guest {
    guest_owned_by(root, access, LeaseOwner::new())
}

fn guest_owned_by(root: &Path, access: ShareAccess, owner: LeaseOwner) -> Guest {
    let mut guest = Guest::new();
    let shares = table(&[(SHARE, root, access)]).with_named_objects(Some(SHARE.to_string()));
    guest.server.set_shares(shares, owner);
    guest
}

fn call(guest: &mut Guest, op: u8, name: &str) -> u8 {
    let len = u8::try_from(name.len()).expect("test names fit a length byte");
    let mut request = vec![op, len];
    request.extend_from_slice(name.as_bytes());
    let mut reply = guest.feed(&request);
    guest.pump();
    reply.extend(guest.feed(&[]));
    assert_eq!(reply.len(), 1, "a named-object call replies with one byte");
    reply[0]
}

fn mount(guest: &mut Guest, name: &str) -> u8 {
    call(guest, opcode::NAMEOBJ_MOUNT, name)
}

fn create(guest: &mut Guest, name: &str) -> u8 {
    call(guest, opcode::NAMEOBJ_CREATE, name)
}

fn lsn_bytes(lsn: u64) -> [u8; 3] {
    let [.., high, mid, low] = lsn.to_be_bytes();
    [high, mid, low]
}

/// `OP_READ`: the sector, or the error status.
fn read(guest: &mut Guest, drive: u8, lsn: u64) -> Result<Vec<u8>, u8> {
    let mut request = vec![opcode::READ, drive];
    request.extend(lsn_bytes(lsn));
    let mut reply = guest.feed(&request);
    guest.pump();
    reply.extend(guest.feed(&[]));
    match reply[0] {
        error::OK => {
            assert_eq!(reply.len(), 1 + SECTOR_SIZE + 2);
            Ok(reply[1..=SECTOR_SIZE].to_vec())
        }
        status => Err(status),
    }
}

/// `OP_WRITE`: the status byte.
fn write(guest: &mut Guest, drive: u8, lsn: u64, sector: &[u8]) -> u8 {
    let mut request = vec![opcode::WRITE, drive];
    request.extend(lsn_bytes(lsn));
    request.extend_from_slice(sector);
    request.extend(checksum_of(sector).to_be_bytes());
    let mut reply = guest.feed(&request);
    guest.pump();
    reply.extend(guest.feed(&[]));
    assert_eq!(reply.len(), 1);
    reply[0]
}

fn leased(guest: &Guest) -> Option<u8> {
    guest
        .server
        .named_objects()
        .iter()
        .find(|object| object.leased)
        .map(|object| object.drive)
}

#[test]
fn mount_returns_a_drive_whose_sectors_are_the_object_with_a_padded_tail() {
    let scratch = ScratchDir::new("named-mount");
    let bytes = object_bytes(1, OBJECT_LEN);
    scratch.file("boot/os9.bin", &bytes);
    let mut guest = guest(scratch.path(), ShareAccess::ReadOnly);

    let drive = mount(&mut guest, "boot/os9.bin");
    assert_eq!(drive, u8::MAX, "the first object takes drive 255");
    assert_eq!(read(&mut guest, drive, 0).unwrap(), bytes[..SECTOR_SIZE]);
    let mut tail = bytes[SECTOR_SIZE..].to_vec();
    tail.resize(SECTOR_SIZE, 0);
    assert_eq!(read(&mut guest, drive, 1).unwrap(), tail);
    assert_eq!(read(&mut guest, drive, 2), Err(error::READ));
    assert_eq!(
        guest.server.named_objects(),
        [NamedObjectInfo {
            drive,
            name: "boot/os9.bin".to_string(),
            guest_path: "/games/boot/os9.bin".to_string(),
            write_protected: true,
            leased: true,
            attached: true,
        }]
    );
}

#[test]
fn create_makes_a_new_object_that_takes_sector_writes() {
    let scratch = ScratchDir::new("named-create");
    let mut guest = guest(scratch.path(), ShareAccess::ReadWrite);

    let drive = create(&mut guest, "SAVE.BIN");
    assert!(drive >= NAMED_DRIVE_FIRST);
    assert_eq!(read(&mut guest, drive, 0), Err(error::READ), "empty object");
    let sector = object_bytes(7, SECTOR_SIZE);
    assert_eq!(write(&mut guest, drive, 1, &sector), error::OK);
    assert_eq!(write(&mut guest, drive, 0, &sector), error::OK);

    let host = fs::read(scratch.path().join("SAVE.BIN")).unwrap();
    assert_eq!(host, [sector.clone(), sector.clone()].concat());
    assert_eq!(read(&mut guest, drive, 1).unwrap(), sector);
    assert_eq!(guest.server.sectors_written(), 2);
    assert_eq!(guest.server.drive_ops(0), 0, "drive 0 saw no activity");
}

#[test]
fn create_fails_for_an_existing_object_and_mount_for_a_missing_one() {
    let scratch = ScratchDir::new("named-exists");
    let bytes = object_bytes(3, OBJECT_LEN);
    scratch.file("hello.bin", &bytes);
    let mut guest = guest(scratch.path(), ShareAccess::ReadWrite);

    assert_eq!(create(&mut guest, "hello.bin"), NAMED_FAILURE);
    assert_eq!(fs::read(scratch.path().join("hello.bin")).unwrap(), bytes);
    assert_eq!(mount(&mut guest, "missing.bin"), NAMED_FAILURE);
    assert!(!scratch.path().join("missing.bin").exists());
    assert!(guest.server.named_objects().is_empty());
}

#[test]
fn a_read_only_share_refuses_object_writes_and_creates() {
    let scratch = ScratchDir::new("named-readonly");
    let bytes = object_bytes(5, OBJECT_LEN);
    scratch.file("hello.bin", &bytes);
    let mut guest = guest(scratch.path(), ShareAccess::ReadOnly);

    let drive = mount(&mut guest, "hello.bin");
    let sector = object_bytes(9, SECTOR_SIZE);
    assert_eq!(write(&mut guest, drive, 0, &sector), error::WRITE);
    assert_eq!(fs::read(scratch.path().join("hello.bin")).unwrap(), bytes);
    assert_eq!(create(&mut guest, "new.bin"), NAMED_FAILURE);
    assert!(!scratch.path().join("new.bin").exists());
}

#[test]
fn repeated_names_return_the_drive_already_holding_the_object() {
    let scratch = ScratchDir::new("named-repeat");
    scratch.file("a.bin", &object_bytes(1, SECTOR_SIZE));
    scratch.file("b.bin", &object_bytes(2, SECTOR_SIZE));
    let mut guest = guest(scratch.path(), ShareAccess::ReadWrite);

    let a = mount(&mut guest, "a.bin");
    let b = mount(&mut guest, "b.bin");
    assert_ne!(a, b);
    assert_eq!(mount(&mut guest, "a.bin"), a);
    assert_eq!(
        mount(&mut guest, "./sub/../a.bin"),
        a,
        "same file, other spelling"
    );
    assert_eq!(guest.server.named_objects().len(), 2);
    assert_eq!(leased(&guest), Some(a));
}

#[test]
fn each_call_ends_the_previous_lease_even_when_it_fails() {
    let scratch = ScratchDir::new("named-lease");
    scratch.file("a.bin", &object_bytes(1, SECTOR_SIZE));
    let mut guest = guest(scratch.path(), ShareAccess::ReadOnly);

    let a = mount(&mut guest, "a.bin");
    assert_eq!(leased(&guest), Some(a));
    assert_eq!(mount(&mut guest, "missing.bin"), NAMED_FAILURE);
    assert_eq!(leased(&guest), None);
    // The object stays mounted; only the guarantee ended.
    assert!(read(&mut guest, a, 0).is_ok());
}

#[test]
fn a_full_table_replaces_the_least_recently_returned_object() {
    let scratch = ScratchDir::new("named-full");
    let names: Vec<String> = (0..=MAX_NAMED_OBJECTS)
        .map(|i| format!("o{i}.bin"))
        .collect();
    for (i, name) in names.iter().enumerate() {
        scratch.file(name, &object_bytes(i as u8, SECTOR_SIZE));
    }
    let mut guest = guest(scratch.path(), ShareAccess::ReadOnly);
    guest
        .server
        .mount(0, DWImage::Memory(vec![0xD0; SECTOR_SIZE]));

    let drives: Vec<u8> = names[..MAX_NAMED_OBJECTS]
        .iter()
        .map(|name| mount(&mut guest, name))
        .collect();
    let expected: Vec<u8> = (0..MAX_NAMED_OBJECTS).map(|i| u8::MAX - i as u8).collect();
    assert_eq!(drives, expected, "drives count down from 255");
    // Returning o0 again makes o1 the least recently returned.
    assert_eq!(mount(&mut guest, &names[0]), drives[0]);

    let newest = mount(&mut guest, &names[MAX_NAMED_OBJECTS]);
    assert!(
        !drives.contains(&newest),
        "a replaced drive is not reused at once"
    );
    assert_eq!(guest.server.named_objects().len(), MAX_NAMED_OBJECTS);
    assert_eq!(read(&mut guest, drives[1], 0), Err(error::NOT_READY));
    assert_eq!(
        read(&mut guest, drives[0], 0).unwrap(),
        object_bytes(0, SECTOR_SIZE)
    );
    assert_eq!(
        read(&mut guest, 0, 0).unwrap(),
        [0xD0; SECTOR_SIZE],
        "configured drives are never taken"
    );
}

#[test]
fn drive_numbers_wrap_after_the_lowest_object_drive() {
    let scratch = ScratchDir::new("named-wrap");
    scratch.file("a.bin", &object_bytes(1, SECTOR_SIZE));
    let mut objects = NamedObjects {
        cursor: NAMED_DRIVE_FIRST,
        ..NamedObjects::default()
    };
    assert_eq!(objects.allocate(), NAMED_DRIVE_FIRST);
    assert_eq!(objects.cursor, u8::MAX);

    let mut guest = guest(scratch.path(), ShareAccess::ReadOnly);
    guest.server.named.cursor = NAMED_DRIVE_FIRST;
    assert_eq!(mount(&mut guest, "a.bin"), NAMED_DRIVE_FIRST);
}

#[test]
fn replacing_the_host_file_during_a_lease_keeps_io_on_the_leased_object() {
    let scratch = ScratchDir::new("named-replace");
    let original = object_bytes(1, SECTOR_SIZE);
    let replacement = object_bytes(2, SECTOR_SIZE);
    let path = scratch.file("a.bin", &original);
    let staged = scratch.file("staged.bin", &replacement);
    let mut guest = guest(scratch.path(), ShareAccess::ReadWrite);

    let drive = mount(&mut guest, "a.bin");
    fs::rename(&staged, &path).unwrap();
    assert_eq!(read(&mut guest, drive, 0).unwrap(), original);

    let renamed = mount(&mut guest, "a.bin");
    assert_ne!(renamed, drive, "the name now resolves to another file");
    assert_eq!(read(&mut guest, renamed, 0).unwrap(), replacement);
}

#[test]
fn malformed_names_fail_without_ending_framing() {
    let scratch = ScratchDir::new("named-malformed");
    scratch.file("a.bin", &object_bytes(1, SECTOR_SIZE));
    let mut guest = guest(scratch.path(), ShareAccess::ReadWrite);

    assert_eq!(guest.feed(&[opcode::NAMEOBJ_MOUNT, 0]), [NAMED_FAILURE]);
    for name in ["a*b", "../a.bin", "/games/a.bin", " a.bin", "a\\b"] {
        assert_eq!(create(&mut guest, name), NAMED_FAILURE, "{name:?}");
    }
    let mut high_bit = vec![opcode::NAMEOBJ_MOUNT, 2, b'a', 0xAE];
    let mut reply = guest.feed(&high_bit);
    guest.pump();
    reply.extend(guest.feed(&[]));
    assert_eq!(reply, [NAMED_FAILURE]);
    assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 1);

    // A name cut short by the transaction timeout gets no reply and leaves
    // the lease alone; the next byte is an opcode again.
    let drive = mount(&mut guest, "a.bin");
    high_bit.truncate(3);
    assert!(guest.feed(&high_bit).is_empty());
    guest
        .server
        .data_write(opcode::TIME, TRANSACTION_TIMEOUT_CYCLES * 4);
    assert_eq!(guest.feed(&[]).len(), TIME_REPLY_LEN);
    assert_eq!(leased(&guest), Some(drive));
}

#[test]
fn hdbdos_mode_addresses_object_drives_directly_and_never_remaps_into_them() {
    let scratch = ScratchDir::new("named-hdbdos");
    let bytes = object_bytes(4, OBJECT_LEN);
    scratch.file("prog.bin", &bytes);
    let mut guest = guest(scratch.path(), ShareAccess::ReadOnly);
    guest.server.set_hdbdos_mode(true);
    let disk: Vec<u8> = (0..2 * SECTOR_SIZE)
        .map(|i| (i / SECTOR_SIZE) as u8)
        .collect();
    guest.server.mount(0, DWImage::Memory(disk));

    let drive = mount(&mut guest, "prog.bin");
    assert_eq!(drive, u8::MAX);
    assert_eq!(read(&mut guest, drive, 0).unwrap(), bytes[..SECTOR_SIZE]);
    assert_eq!(read(&mut guest, 0, 1).unwrap(), [1; SECTOR_SIZE]);
    assert_eq!(
        read(&mut guest, 0, HDBDOS_DISK_255_LSN),
        Err(error::NOT_READY),
        "an HDB-DOS drive number is not an object drive"
    );
}

#[test]
fn restore_reopens_objects_by_name_and_drops_those_that_are_gone() {
    let scratch = ScratchDir::new("named-restore");
    let a = object_bytes(1, SECTOR_SIZE);
    scratch.file("a.bin", &a);
    let b_path = scratch.file("b.bin", &object_bytes(2, SECTOR_SIZE));
    let mut guest = guest(scratch.path(), ShareAccess::ReadWrite);
    let drive_a = mount(&mut guest, "a.bin");
    let drive_b = mount(&mut guest, "b.bin");

    let mut cbor = Vec::new();
    ciborium::into_writer(&guest.server, &mut cbor).unwrap();
    guest.server.stop_host();
    fs::remove_file(&b_path).unwrap();
    let mut restored = guest_owned_by(scratch.path(), ShareAccess::ReadWrite, LeaseOwner::new());
    let shares = restored.server.share_session().table().clone();
    let mut server: DWServer = ciborium::from_reader(cbor.as_slice()).unwrap();
    // Keep the harness's manual executor, as the frontend keeps its host.
    std::mem::swap(&mut server.host, &mut restored.server.host);
    restored.server = server;
    restored.server.after_restore();
    restored.server.set_shares(shares, LeaseOwner::new());

    assert!(restored.server.named_objects().iter().all(|o| !o.attached));
    assert_eq!(read(&mut restored, drive_a, 0), Err(error::NOT_READY));

    let failures = restored.server.reattach_named_objects();
    assert_eq!(
        failures,
        [NamedRestoreFailure {
            drive: drive_b,
            name: "b.bin".to_string(),
            error: NamedRestoreError::Share(ShareError::NotFound),
        }]
    );
    assert_eq!(read(&mut restored, drive_a, 0).unwrap(), a);
    assert_eq!(read(&mut restored, drive_b, 0), Err(error::NOT_READY));
    assert_eq!(leased(&restored), None, "b held the lease and is gone");
}

#[test]
fn a_snapshot_from_before_named_objects_restores_with_none() {
    let mut guest = Guest::new();
    let mut value: ciborium::Value = {
        let mut cbor = Vec::new();
        ciborium::into_writer(&guest.server, &mut cbor).unwrap();
        ciborium::from_reader(cbor.as_slice()).unwrap()
    };
    if let ciborium::Value::Map(fields) = &mut value {
        fields.retain(|(key, _)| key.as_text() != Some("named"));
    }
    let mut cbor = Vec::new();
    ciborium::into_writer(&value, &mut cbor).unwrap();
    guest.server = ciborium::from_reader(cbor.as_slice()).unwrap();
    assert!(guest.server.named_objects().is_empty());
    assert_eq!(guest.server.named.cursor, u8::MAX);
}

#[test]
fn suspend_and_early_opcodes_answer_a_pending_call_with_failure() {
    let scratch = ScratchDir::new("named-pending");
    scratch.file("a.bin", &object_bytes(1, SECTOR_SIZE));
    let mut guest = guest(scratch.path(), ShareAccess::ReadWrite);
    let request = [&[opcode::NAMEOBJ_MOUNT, 5][..], b"a.bin"].concat();

    assert!(guest.feed(&request).is_empty(), "waits for the host");
    assert!(!guest.server.host_is_idle());
    guest.server.suspend_host();
    assert_eq!(guest.feed(&[]), [NAMED_FAILURE]);
    guest.pump();
    assert!(guest.server.named_objects().is_empty());
    guest.server.resume_host();

    assert!(guest.feed(&request).is_empty());
    assert!(guest.feed(&[opcode::NOP]).is_empty(), "abandoned, no reply");
    guest.pump();
    assert!(guest.server.named_objects().is_empty());
    assert_eq!(mount(&mut guest, "a.bin"), u8::MAX);
}

#[test]
fn objects_survive_resets_and_close_when_the_host_stops() {
    let scratch = ScratchDir::new("named-reset");
    let bytes = object_bytes(1, SECTOR_SIZE);
    scratch.file("a.bin", &bytes);
    let mut vm_a = guest(scratch.path(), ShareAccess::ReadWrite);
    let mut vm_b = guest(scratch.path(), ShareAccess::ReadWrite);

    let drive = mount(&mut vm_a, "a.bin");
    assert_eq!(mount(&mut vm_b, "a.bin"), NAMED_FAILURE, "VM A writes it");
    vm_a.server.reset_session();
    vm_a.feed(&[opcode::RESET1, opcode::DWINIT, 1]);
    assert_eq!(read(&mut vm_a, drive, 0).unwrap(), bytes);

    vm_a.server.stop_host();
    assert!(vm_a.server.named_objects().is_empty());
    assert_eq!(mount(&mut vm_b, "a.bin"), u8::MAX);
}

#[test]
fn restore_drops_objects_whose_name_now_resolves_in_another_share() {
    let (games, work) = (
        ScratchDir::new("named-games"),
        ScratchDir::new("named-work"),
    );
    games.file("os9.dsk", &object_bytes(1, SECTOR_SIZE));
    work.file("os9.dsk", &object_bytes(2, SECTOR_SIZE));
    let mut guest = guest(games.path(), ShareAccess::ReadWrite);
    let drive = mount(&mut guest, "os9.dsk");
    let mut cbor = Vec::new();
    ciborium::into_writer(&guest.server, &mut cbor).unwrap();
    guest.server.stop_host();

    let mut server: DWServer = ciborium::from_reader(cbor.as_slice()).unwrap();
    std::mem::swap(&mut server.host, &mut guest.server.host);
    guest.server = server;
    guest.server.after_restore();
    let shares = table(&[
        (SHARE, games.path(), ShareAccess::ReadWrite),
        ("work", work.path(), ShareAccess::ReadWrite),
    ])
    .with_named_objects(Some("work".to_string()));
    guest.server.set_shares(shares, LeaseOwner::new());

    let failures = guest.server.reattach_named_objects();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].error, NamedRestoreError::OtherShare);
    assert_eq!(read(&mut guest, drive, 0), Err(error::NOT_READY));
    assert!(guest.server.named_objects().is_empty());
}
