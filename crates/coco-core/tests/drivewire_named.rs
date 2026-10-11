//! Guest acceptance for DriveWire named objects (`coco_core::drivewire`'s
//! `named` module) under HDB-DOS, the way DWLOAD-style loaders use them.
//!
//! The client is a small 6809 routine run with `EXEC` from the HDB-DOS
//! prompt. Like the FujiNet DWLOAD clone (`fujinet-config`
//! `src/coco/cfgload/dwload_cmoc.c`), it sends `OP_NAMEOBJ_MOUNT` or
//! `OP_NAMEOBJ_CREATE` and then sector requests for the returned drive
//! through the DriveWire routines HDB-DOS exports: `[$D941]` (DWWrite: X =
//! bytes, Y = count) and `[$D93F]` (DWRead: X = buffer, Y = count, returns
//! the byte sum in Y). In the installed `hdbdw3bc3.rom` these vectors hold
//! `$DB15` and `$DB24`. HDB-DOS mode stays on and drive 0 keeps serving
//! HDB-DOS's own disk, so the test also covers coexistence: object drives
//! bypass the LSN/630 remapping while `DIR` still reaches drive 0.
//!
//! CoCoBoot, the client the DriveWire specification names, is not among
//! the installed assets, so this routine stands in for it. Skips when the
//! ROMs or `tests/blank02.dsk` are absent.

use std::path::{Path, PathBuf};

use coco_core::cart::ROMPak;
use coco_core::drivewire::share::{LeaseOwner, ShareAccess, ShareSpec, ShareTable};
use coco_core::drivewire::{DWImage, NAMED_FAILURE, SECTOR_SIZE, error, opcode};
use coco_core::{Machine, MachineConfig};
use mc6809::Bus;
use test_assets::{
    disk::BLANK02,
    rom::{COCO3, HDBDW3BC3},
};

/// Where the client routine and its data live: free RAM well above BASIC.
const CODE: u16 = 0x4000;
/// Request length (2 bytes, big-endian), then the reply byte and status.
const REQUEST_LEN: u16 = 0x40F0;
const RESULT: u16 = 0x40F2;
const STATUS: u16 = 0x40F3;
/// Sector packet length (2 bytes).
const PACKET_LEN: u16 = 0x40F4;
/// The client's checksum echo for `OP_READEX` (2 bytes).
const CHECKSUM: u16 = 0x40F6;
const REQUEST: u16 = 0x4100;
/// The sector packet: opcode, drive (filled in from [`RESULT`]), LSN, and
/// for a write the data and checksum.
const PACKET: u16 = 0x4200;
const SECTOR: u16 = 0x4400;
/// [`STATUS`] before the routine runs, so a skipped status read shows.
const UNSET: u8 = 0xEE;

/// The client. Branch offsets are relative to the next instruction.
const CLIENT: &[u8] = &[
    0x8E, 0x41, 0x00, // LDX #REQUEST
    0x10, 0xBE, 0x40, 0xF0, // LDY REQUEST_LEN
    0xAD, 0x9F, 0xD9, 0x41, // JSR [$D941]   DWWrite
    0x8E, 0x40, 0xF2, // LDX #RESULT
    0x10, 0x8E, 0x00, 0x01, // LDY #1
    0xAD, 0x9F, 0xD9, 0x3F, // JSR [$D93F]   DWRead
    0xB6, 0x40, 0xF2, // LDA RESULT
    0x27, 0x3A, // BEQ DONE        (0 = no object)
    0xB7, 0x42, 0x01, // STA PACKET+1   the drive
    0x8E, 0x42, 0x00, // LDX #PACKET
    0x10, 0xBE, 0x40, 0xF4, // LDY PACKET_LEN
    0xAD, 0x9F, 0xD9, 0x41, // JSR [$D941]
    0xB6, 0x42, 0x00, // LDA PACKET
    0x81, 0xD2, // CMPA #OP_READEX
    0x26, 0x1A, // BNE STAT
    0x8E, 0x44, 0x00, // LDX #SECTOR
    0x10, 0x8E, 0x01, 0x00, // LDY #256
    0xAD, 0x9F, 0xD9, 0x3F, // JSR [$D93F]   Y = byte sum
    0x10, 0xBF, 0x40, 0xF6, // STY CHECKSUM
    0x8E, 0x40, 0xF6, // LDX #CHECKSUM
    0x10, 0x8E, 0x00, 0x02, // LDY #2
    0xAD, 0x9F, 0xD9, 0x41, // JSR [$D941]
    // STAT:
    0x8E, 0x40, 0xF3, // LDX #STATUS
    0x10, 0x8E, 0x00, 0x01, // LDY #1
    0xAD, 0x9F, 0xD9, 0x3F, // JSR [$D93F]
    // DONE:
    0x39, // RTS
];

/// Fields to boot to the `OK` prompt; observed well under one poll batch.
const MAX_BOOT_FIELDS: usize = 6_000;
/// Fields to wait for the client or `DIR` to finish.
const MAX_RUN_FIELDS: usize = 3_000;
/// Fields per screen check.
const POLL_FIELDS: usize = 60;
/// Key frames per press and release.
const KEY_FIELDS: usize = 3;
/// Printed once the client returns; typed as `"NAM"+"ED"` so the command
/// line itself does not match.
const DONE_MARKER: &str = "NAMED";
const RUN_CLIENT: &str = "EXEC &H4000:PRINT \"NAM\"+\"ED\"";
/// A whole sector plus a partial one, so the object ends mid-sector.
const OBJECT_LEN: usize = SECTOR_SIZE + 44;
const SHARE: &str = "objects";

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("cocovm-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create scratch folder");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tap(m: &mut Machine, c: char) {
    let (pos, shift) =
        coco_core::keyboard::char_key(c).unwrap_or_else(|| panic!("no key for {c:?}"));
    m.bus.keyboard.set(coco_core::keyboard::SHIFT, shift);
    m.bus.keyboard.set(pos, true);
    for _ in 0..KEY_FIELDS {
        m.run_field();
    }
    m.bus.keyboard.set(pos, false);
    m.bus.keyboard.set(coco_core::keyboard::SHIFT, false);
    for _ in 0..KEY_FIELDS {
        m.run_field();
    }
}

fn enter(m: &mut Machine, line: &str) {
    line.chars().chain(['\r']).for_each(|c| tap(m, c));
}

fn wait_for(m: &mut Machine, marker: &str, max_fields: usize) -> String {
    let mut fields = 0;
    loop {
        for _ in 0..POLL_FIELDS {
            m.run_field();
        }
        fields += POLL_FIELDS;
        let screen = m.text_screen_lines().join("\n");
        if screen.contains(marker) {
            return screen;
        }
        assert!(
            fields < max_fields,
            "never saw {marker:?} after {fields} fields; screen:\n{screen}"
        );
    }
}

/// HDB-DOS at its prompt, in HDB-DOS mode, with a scratch copy of
/// `blank02.dsk` in drive 0 and `objects` serving named objects.
fn boot(scratch: &Scratch, objects: &Path, access: ShareAccess) -> Option<Machine> {
    let (Ok(coco), Ok(hdbdos)) = (
        std::fs::read(test_assets::rom(COCO3)),
        std::fs::read(test_assets::rom(HDBDW3BC3)),
    ) else {
        eprintln!("skipping named-object test: roms/coco3.rom or roms/hdbdw3bc3.rom missing");
        return None;
    };
    let blank = test_assets::disk(BLANK02);
    if !blank.exists() {
        eprintln!("skipping named-object test: {} missing", blank.display());
        return None;
    }
    let disk_path = scratch.path().join("drive0.dsk");
    std::fs::copy(&blank, &disk_path).expect("copy blank02.dsk");
    let disk = std::fs::File::options()
        .read(true)
        .write(true)
        .open(&disk_path)
        .expect("open scratch disk");

    let mut m = Machine::new(MachineConfig::default(), coco.into_boxed_slice());
    m.insert_cartridge(ROMPak::from_bytes(&hdbdos, false).unwrap());
    m.bus.enable_drivewire();
    let dw = m.bus.drivewire.as_mut().unwrap();
    dw.set_hdbdos_mode(true);
    dw.mount(0, DWImage::File(disk));
    let share = ShareSpec {
        name: SHARE.to_string(),
        root: objects.to_path_buf(),
        access,
    };
    let shares = ShareTable::new(vec![share])
        .unwrap()
        .with_named_objects(Some(SHARE.to_string()));
    dw.set_shares(shares, LeaseOwner::new());
    m.reset();
    wait_for(&mut m, "OK", MAX_BOOT_FIELDS);
    Some(m)
}

fn poke(m: &mut Machine, addr: u16, bytes: &[u8]) {
    for (offset, &byte) in bytes.iter().enumerate() {
        m.bus.write(addr + offset as u16, byte);
    }
}

fn peek(m: &mut Machine, addr: u16, len: usize) -> Vec<u8> {
    (0..len)
        .map(|offset| m.bus.read(addr + offset as u16))
        .collect()
}

fn checksum(sector: &[u8]) -> u16 {
    sector.iter().map(|&byte| u16::from(byte)).sum()
}

/// Runs the client with a named-object request and a sector packet, and
/// returns the reply byte and the sector status.
fn run_client(m: &mut Machine, op: u8, name: &str, packet: &[u8]) -> (u8, u8) {
    let mut request = vec![op, u8::try_from(name.len()).unwrap()];
    request.extend_from_slice(name.as_bytes());
    poke(m, CODE, CLIENT);
    poke(m, REQUEST, &request);
    poke(m, REQUEST_LEN, &(request.len() as u16).to_be_bytes());
    poke(m, PACKET, packet);
    poke(m, PACKET_LEN, &(packet.len() as u16).to_be_bytes());
    poke(m, STATUS, &[UNSET]);
    // Clear the screen so the marker below comes from this run.
    enter(m, "CLS");
    enter(m, RUN_CLIENT);
    let screen = wait_for(m, DONE_MARKER, MAX_RUN_FIELDS);
    assert!(!screen.contains("ERROR"), "screen:\n{screen}");
    (peek(m, RESULT, 1)[0], peek(m, STATUS, 1)[0])
}

fn read_packet(lsn: u8) -> Vec<u8> {
    vec![opcode::READEX, 0, 0, 0, lsn]
}

#[test]
fn a_guest_mounts_an_existing_object_and_reads_it_through_hdbdos_routines() {
    let scratch = Scratch::new("named-mount");
    let objects = scratch.path().join("objects");
    std::fs::create_dir(&objects).unwrap();
    let bytes: Vec<u8> = (0..OBJECT_LEN).map(|i| (i * 7) as u8).collect();
    std::fs::write(objects.join("HELLO.BIN"), &bytes).unwrap();
    let Some(mut m) = boot(&scratch, &objects, ShareAccess::ReadOnly) else {
        return;
    };

    let (drive, status) = run_client(&mut m, opcode::NAMEOBJ_MOUNT, "HELLO.BIN", &read_packet(0));
    assert_eq!(drive, u8::MAX, "the first object takes drive 255");
    assert_eq!(status, error::OK);
    assert_eq!(peek(&mut m, SECTOR, SECTOR_SIZE), bytes[..SECTOR_SIZE]);
    let echoed = peek(&mut m, CHECKSUM, 2);
    assert_eq!(echoed, checksum(&bytes[..SECTOR_SIZE]).to_be_bytes());

    let (again, status) = run_client(&mut m, opcode::NAMEOBJ_MOUNT, "HELLO.BIN", &read_packet(1));
    assert_eq!(again, drive, "a mounted object keeps its drive");
    assert_eq!(status, error::OK);
    let mut tail = bytes[SECTOR_SIZE..].to_vec();
    tail.resize(SECTOR_SIZE, 0);
    assert_eq!(peek(&mut m, SECTOR, SECTOR_SIZE), tail, "zero-padded tail");

    let (missing, status) = run_client(
        &mut m,
        opcode::NAMEOBJ_MOUNT,
        "MISSING.BIN",
        &read_packet(0),
    );
    assert_eq!(missing, NAMED_FAILURE);
    assert_eq!(status, UNSET, "no sector request follows a failure");

    // HDB-DOS still reaches its own disk in drive 0.
    enter(&mut m, "DIR");
    let screen = wait_for(&mut m, "SALUT", MAX_RUN_FIELDS);
    assert!(!screen.contains("ERROR"), "screen:\n{screen}");
    assert_eq!(m.bus.drivewire.as_ref().unwrap().unknown_opcodes(), 0);
}

#[test]
fn a_guest_creates_and_writes_a_new_object_in_a_read_write_share() {
    let scratch = Scratch::new("named-create");
    let objects = scratch.path().join("objects");
    std::fs::create_dir(&objects).unwrap();
    std::fs::write(objects.join("TAKEN.BIN"), b"keep").unwrap();
    let Some(mut m) = boot(&scratch, &objects, ShareAccess::ReadWrite) else {
        return;
    };
    let disk_before = std::fs::read(scratch.path().join("drive0.dsk")).unwrap();

    let sector: Vec<u8> = (0..SECTOR_SIZE).map(|i| (255 - i) as u8).collect();
    let mut packet = vec![opcode::WRITE, 0, 0, 0, 0];
    packet.extend_from_slice(&sector);
    packet.extend(checksum(&sector).to_be_bytes());

    let (drive, status) = run_client(&mut m, opcode::NAMEOBJ_CREATE, "SAVE.BIN", &packet);
    assert_eq!(drive, u8::MAX);
    assert_eq!(status, error::OK);
    assert_eq!(std::fs::read(objects.join("SAVE.BIN")).unwrap(), sector);

    let (exists, status) = run_client(&mut m, opcode::NAMEOBJ_CREATE, "TAKEN.BIN", &packet);
    assert_eq!(exists, NAMED_FAILURE, "create refuses an existing object");
    assert_eq!(status, UNSET);
    assert_eq!(std::fs::read(objects.join("TAKEN.BIN")).unwrap(), b"keep");

    let disk_after = std::fs::read(scratch.path().join("drive0.dsk")).unwrap();
    assert_eq!(disk_after, disk_before, "object writes never reach drive 0");
}
