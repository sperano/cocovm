//! Save-state coverage for the trickiest device to snapshot correctly: the
//! FD-502 disk controller mid-sector-transfer, with HALT* actually asserted
//! at the moment of the snapshot — standalone and nested behind a
//! Multi-Pak Interface ("Acceptance": "Snapshot
//! mid-disk-read (HALT asserted) restores without corrupting the
//! transfer"; phase 3 spec items 2-3).
//!
//! Register pokes mirror `tests/fdc.rs`'s direct-DSKREG style; disk/MPI
//! construction mirrors `tests/mpi.rs`. Unlike those files, transfers here
//! are driven through a full [`Machine`] (not a bare `DiskCart`/`SystemBus`),
//! via [`Machine::step_instruction`] — the only stepping primitive that
//! ticks the cartridge (`Machine::step_cpu_raw` used elsewhere in the
//! snapshot test suite is CPU-only and never advances FDC timing at all).

use std::path::PathBuf;

use coco_core::cart::{Cartridge, MultiPak, ROMPak};
use coco_core::fdc::{DiskCart, JVCDisk, dskreg};
use coco_core::snapshot::{self, MediaRef, MediaRefs, MediaSources, SlotROMRef};
use coco_core::wd1773::status;
use coco_core::{Machine, MachineConfig};
use mc6809::{Bus, MC6809, State};
use test_assets::rom::{COCO3, DISK11};

/// Same CPU-trace-identity snapshot as the other snapshot test files (see
/// `snapshot_roundtrip.rs`'s doc comment for the rationale; duplicated here
/// rather than shared since these are separate test binaries).
#[derive(Debug, PartialEq)]
struct CPUSnapshot {
    a: u8,
    b: u8,
    x: u16,
    y: u16,
    u: u16,
    s: u16,
    pc: u16,
    dp: u8,
    cc: u8,
    cycles: u64,
    state: State,
}

impl CPUSnapshot {
    fn of(cpu: &MC6809) -> Self {
        Self {
            a: cpu.a,
            b: cpu.b,
            x: cpu.x,
            y: cpu.y,
            u: cpu.u,
            s: cpu.s,
            pc: cpu.pc,
            dp: cpu.dp,
            cc: cpu.cc,
            cycles: cpu.cycles,
            state: cpu.state,
        }
    }
}

fn load_rom(name: &str) -> Box<[u8]> {
    let path = test_assets::rom(name);
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .into_boxed_slice()
}

fn boot_machine() -> Machine {
    Machine::new(MachineConfig::default(), load_rom(COCO3))
}

/// One headerless track (18 sectors x 256B), every sector filled with the
/// `i as u8` index pattern -- same trick as `tests/fdc.rs`'s
/// `index_pattern_disk`, so a successful Read Sector's bytes double as proof
/// the transfer wasn't corrupted.
const ONE_TRACK_BYTES: usize = 18 * 256;
fn index_pattern_disk_bytes() -> Vec<u8> {
    (0..ONE_TRACK_BYTES).map(|i| i as u8).collect()
}

const DSKREG: u16 = 0xFF40;
const STATUS_COMMAND_REG: u16 = 0xFF48;
const TRACK_REG: u16 = 0xFF49;
const SECTOR_REG: u16 = 0xFF4A;
const DATA_REG: u16 = 0xFF4B;

/// Fields run before touching the FDC directly, so the machine carries real,
/// varied CPU/GIME state by the time it's snapshotted (same rationale as
/// `snapshot_roundtrip.rs`'s `WARMUP_STEPS`) -- deliberately short: idle
/// BASIC never touches the disk registers, so this is just for variety, not
/// to reach a particular banner.
const WARMUP_FIELDS: usize = 60;

const SECTOR_BYTES: usize = 256;
/// Generous cap against an infinite loop if a regression stops the transfer
/// from ever completing (comfortably more than the FDC's own ~8,700-cycle
/// search-latency + 256-byte + CRC-trailer budget, since most of that budget
/// is spent one cycle per `step_instruction` call while HALT* is asserted).
const MAX_TRANSFER_STEPS: usize = 20_000;

/// Dispatch a Read Sector (track 0, sector 1, drive 0) on the currently
/// selected FD-502 through `m`'s full bus -- DSKREG halt-enable set, so this
/// leaves HALT* asserted the instant the command is written (DRQ clears,
/// halt-enable is set, per `tests/fdc.rs`'s `halt_line_is_not_drq_and_halt_enable`).
fn dispatch_read_sector(m: &mut Machine) {
    m.bus.write(DSKREG, dskreg::MOTOR_ON | dskreg::DRIVE0);
    m.bus.write(STATUS_COMMAND_REG, 0xD0); // Force Interrupt, cancel only: clean slate
    m.bus.write(TRACK_REG, 0);
    m.bus.write(SECTOR_REG, 1);
    m.bus.write(
        DSKREG,
        dskreg::MOTOR_ON | dskreg::DRIVE0 | dskreg::HALT_ENABLE,
    );
    m.bus.write(STATUS_COMMAND_REG, 0x80); // Read Sector, single
    while !m.bus.halt_asserted() {
        m.step_instruction();
    }
}

/// Drive `m` forward one `step_instruction` at a time, polling the status
/// register for DRQ and draining a byte the instant it's ready, until a full
/// sector (`SECTOR_BYTES`) has been collected. Because the poll happens right
/// after every single step, it always catches DRQ before the CPU gets a
/// chance to run past it -- so this reaches sector completion in lockstep
/// with the FDC's own byte pacing, deterministically, without depending on
/// what code (if any) the CPU is otherwise executing. Returns the drained
/// bytes and a full per-step CPU trace, for the lockstep comparison between
/// the original and restored machine.
fn drain_sector(m: &mut Machine) -> (Vec<u8>, Vec<CPUSnapshot>) {
    let mut bytes = Vec::with_capacity(SECTOR_BYTES);
    let mut trace = Vec::new();
    let mut steps = 0usize;
    while bytes.len() < SECTOR_BYTES {
        assert!(
            steps < MAX_TRANSFER_STEPS,
            "sector transfer never completed"
        );
        m.step_instruction();
        trace.push(CPUSnapshot::of(&m.cpu));
        steps += 1;
        if m.bus.read(STATUS_COMMAND_REG) & status::DRQ != 0 {
            bytes.push(m.bus.read(DATA_REG));
        }
    }
    (bytes, trace)
}

fn expected_sector_bytes() -> Vec<u8> {
    (0..SECTOR_BYTES as u32).map(|i| i as u8).collect()
}

/// A [`MediaRef`] naming `path` and hashing `bytes` -- collapses the
/// two-field struct literal that would otherwise repeat at every media
/// reference below.
fn media_ref(path: &str, bytes: &[u8]) -> MediaRef {
    MediaRef {
        path: PathBuf::from(path),
        sha256: snapshot::sha256_hex(bytes),
    }
}

/// `save` -> `load` -> `restore` through the engine, panicking (with the
/// engine's own message) on any step's failure -- both tests below only care
/// about the happy path, `snapshot_engine.rs` and `snapshot_media.rs` cover
/// the error paths.
fn save_and_restore(original: &Machine, media: MediaRefs, sources: MediaSources) -> Machine {
    let bytes = snapshot::save(original, &media).expect("save");
    let payload = snapshot::load(&bytes).expect("load");
    snapshot::restore(payload, sources)
        .expect("restore")
        .machine
}

/// Drain the sector on both machines post-restore and assert the whole
/// transfer -- CPU trace, delivered bytes, and RAM -- came back identical,
/// and that the delivered bytes are themselves uncorrupted.
fn assert_transfer_completes_identically(original: &mut Machine, restored: &mut Machine) {
    let (original_bytes, original_trace) = drain_sector(original);
    let (restored_bytes, restored_trace) = drain_sector(restored);

    assert_eq!(
        original_trace, restored_trace,
        "CPU trace diverged draining the sector post-restore"
    );
    assert_eq!(
        original_bytes, restored_bytes,
        "drained sector bytes diverged post-restore"
    );
    assert_eq!(
        original_bytes,
        expected_sector_bytes(),
        "the transfer must not corrupt the sector's data"
    );
    assert_eq!(
        original.bus.ram, restored.bus.ram,
        "RAM diverged after completing the transfer"
    );
}

// ============================================================================
// 2. Standalone FD-502: mid-transfer snapshot (HALT asserted).
// ============================================================================

#[test]
fn mid_fdc_transfer_snapshot_restores_without_corrupting_the_transfer() {
    let mut original = boot_machine();
    let disk_rom = load_rom(DISK11);
    let disk_bytes = index_pattern_disk_bytes();
    let mut cart = DiskCart::new(disk_rom.clone());
    cart.insert_disk(
        0,
        JVCDisk::from_bytes(disk_bytes.clone()).expect("build disk"),
    );
    original.insert_cartridge(cart);
    original.reset();
    for _ in 0..WARMUP_FIELDS {
        original.run_field();
    }

    dispatch_read_sector(&mut original);
    assert!(
        original.bus.halt_asserted(),
        "HALT* must be asserted: transfer dispatched, no bytes drained yet"
    );

    let media = MediaRefs {
        system_rom: Some(media_ref(COCO3, &load_rom(COCO3))),
        cart_roms: vec![SlotROMRef {
            mpi_slot: None,
            rom: media_ref(DISK11, &disk_rom),
        }],
        disks: vec![Some(media_ref("test.jvc", &disk_bytes)), None, None, None],
        ..MediaRefs::default()
    };
    let sources = MediaSources {
        system_rom: Some(load_rom(COCO3)),
        cart_roms: vec![(None, disk_rom.to_vec())],
        disks: [Some(disk_bytes.clone()), None, None, None],
        ..MediaSources::default()
    };
    let mut restored = save_and_restore(&original, media, sources);
    assert!(
        restored.bus.halt_asserted(),
        "restored machine must come back mid-transfer, still HALT*-asserted"
    );

    assert_transfer_completes_identically(&mut original, &mut restored);
}

// ============================================================================
// 3. FD-502 nested in a Multi-Pak Interface slot.
// ============================================================================

/// Physical slot the FD-502 sits in (and the front-panel switch points at,
/// so `Machine::reset` routes both SCS and CTS to it with no extra `$FF7F`
/// write needed) -- mirrors `tests/mpi.rs`'s `SWITCH_SLOT4`.
const FDC_SLOT: usize = 3;
/// Physical slot the plain ROM pak sits in.
const ROMPAK_SLOT: usize = 0;

#[test]
fn mpi_with_fd502_snapshot_restores_without_corrupting_the_transfer() {
    let mut original = boot_machine();
    let disk_rom = load_rom(DISK11);
    let disk_bytes = index_pattern_disk_bytes();
    let mut disk_cart = DiskCart::new(disk_rom.clone());
    disk_cart.insert_disk(
        0,
        JVCDisk::from_bytes(disk_bytes.clone()).expect("build disk"),
    );

    let pak_image = vec![0x77u8; 1024];
    let pak = ROMPak::from_bytes(&pak_image, false).expect("build pak");

    let mut mp = MultiPak::new(FDC_SLOT);
    mp.insert(FDC_SLOT, disk_cart);
    mp.insert(ROMPAK_SLOT, pak);
    original.insert_cartridge(mp);
    original.reset();
    for _ in 0..WARMUP_FIELDS {
        original.run_field();
    }

    let original_control = original
        .bus
        .cart
        .as_multipak()
        .expect("a MultiPak is inserted")
        .control_read();

    dispatch_read_sector(&mut original);
    assert!(
        original.bus.halt_asserted(),
        "HALT* must be asserted: transfer dispatched through the MPI, no bytes drained yet"
    );

    let media = MediaRefs {
        system_rom: Some(media_ref(COCO3, &load_rom(COCO3))),
        cart_roms: vec![
            SlotROMRef {
                mpi_slot: Some(ROMPAK_SLOT as u8),
                rom: media_ref("pak.rom", &pak_image),
            },
            SlotROMRef {
                mpi_slot: Some(FDC_SLOT as u8),
                rom: media_ref(DISK11, &disk_rom),
            },
        ],
        disks: vec![Some(media_ref("test.jvc", &disk_bytes)), None, None, None],
        ..MediaRefs::default()
    };
    let sources = MediaSources {
        system_rom: Some(load_rom(COCO3)),
        cart_roms: vec![
            (Some(ROMPAK_SLOT as u8), pak_image.clone()),
            (Some(FDC_SLOT as u8), disk_rom.to_vec()),
        ],
        disks: [Some(disk_bytes.clone()), None, None, None],
        ..MediaSources::default()
    };
    let mut restored = save_and_restore(&original, media, sources);

    let restored_control = restored
        .bus
        .cart
        .as_multipak()
        .expect("a MultiPak came back")
        .control_read();
    assert_eq!(
        original_control, restored_control,
        "MPI slot-select register must round-trip"
    );
    assert!(
        restored.bus.halt_asserted(),
        "restored machine must come back mid-transfer, still HALT*-asserted"
    );

    assert_transfer_completes_identically(&mut original, &mut restored);
}
