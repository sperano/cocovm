//! FD-502 disk controller coverage: JVC image geometry, DSKREG decode and the
//! HALT*/NMI control-line recomputation, the WD1773 command state machine, and
//! an end-to-end boot-and-`DIR` integration test against the real
//! `roms/disk11.rom`.

use std::path::PathBuf;

use coco_core::cart::Cartridge;
use coco_core::fdc::{dskreg, DiskCart, JvcDisk, JvcError};
use coco_core::wd1773::{status, WD1773};
use coco_core::{Machine, MachineConfig};
use mc6809::Bus;

// ============================================================================
// JVC image geometry (jvc_dsk.cpp)
// ============================================================================

/// Default geometry, one 18-sector/256-byte/1-side track's worth of bytes.
const ONE_TRACK_BYTES: usize = 18 * 256;

#[test]
fn headerless_image_uses_all_defaults() {
    // 35 tracks x 18 spt x 1 side x 256B, no header (file_len is an exact
    // multiple of 256).
    let bytes = vec![0u8; 35 * ONE_TRACK_BYTES];
    let disk = JvcDisk::from_bytes(bytes).unwrap();
    assert_eq!(disk.track_count(), 35);
    assert_eq!(disk.sectors_per_track(), 18);
    assert_eq!(disk.sides(), 1);
    assert_eq!(disk.sector_size(), 256);
    assert_eq!(disk.first_sector_id(), 1);
}

#[test]
fn two_byte_header_sets_spt_and_sides_defaults_the_rest() {
    // Header = [spt=18, sides=2]; size code/first-id/attribute byte all absent
    // -> defaults. 40 tracks of 18 spt x 2 sides x 256B.
    const TRACKS: usize = 40;
    let mut bytes = vec![18u8, 2u8];
    bytes.extend(vec![0u8; TRACKS * ONE_TRACK_BYTES * 2]);
    // file_len % 256 must equal the 2-byte header for this to parse as
    // intended; ONE_TRACK_BYTES*2 is a multiple of 256, so this holds for any
    // TRACKS.
    assert_eq!(bytes.len() % 256, 2);
    let disk = JvcDisk::from_bytes(bytes).unwrap();
    assert_eq!(disk.sectors_per_track(), 18);
    assert_eq!(disk.sides(), 2);
    assert_eq!(disk.sector_size(), 256);
    assert_eq!(disk.first_sector_id(), 1);
    assert_eq!(disk.track_count(), TRACKS);
}

#[test]
fn sector_offset_matches_the_spec_formula_single_sided() {
    // Explicit 5-byte header: spt=2, sides=1, size code=0 (128B), first id=1,
    // attribute flag=0. 3 tracks, each sector's 128 bytes filled with a marker
    // identifying (track, sector) so the offset math can be checked by content.
    const SPT: usize = 2;
    const SIDES: usize = 1;
    const SECTOR_SIZE: usize = 128;
    const TRACKS: usize = 3;
    let mut bytes = vec![SPT as u8, SIDES as u8, 0, 1, 0];
    for row in 0..TRACKS * SIDES {
        for sector_index in 0..SPT {
            let marker = (row * SPT + sector_index) as u8;
            bytes.extend(std::iter::repeat_n(marker, SECTOR_SIZE));
        }
    }
    let disk = JvcDisk::from_bytes(bytes).unwrap();
    assert_eq!(disk.track_count(), TRACKS);

    for track in 0..TRACKS as u8 {
        for sector_id in 1..=SPT as u8 {
            let off = disk.sector_offset(track, 0, sector_id).unwrap();
            let expected_marker = (track as usize) * SPT + (sector_id as usize - 1);
            assert_eq!(disk.read_bytes(off, 1)[0], expected_marker as u8);
        }
    }
}

#[test]
fn two_sided_image_interleaves_track0_side0_track0_side1_track1_side0() {
    // 1 spt, 2 sides, 128B sectors, 2 tracks: markers 0,1,2,3 laid out in
    // (track*sides+side) row order.
    let mut bytes = vec![1u8, 2, 0, 1, 0];
    for marker in 0u8..4 {
        bytes.extend(std::iter::repeat_n(marker, 128));
    }
    let disk = JvcDisk::from_bytes(bytes).unwrap();
    let cases = [
        (0u8, 0u8, 0u8), // track0 side0 -> marker 0
        (0, 1, 1),       // track0 side1 -> marker 1
        (1, 0, 2),       // track1 side0 -> marker 2
        (1, 1, 3),       // track1 side1 -> marker 3
    ];
    for (track, side, marker) in cases {
        let off = disk.sector_offset(track, side, 1).unwrap();
        assert_eq!(disk.read_bytes(off, 1)[0], marker, "track{track} side{side}");
    }
}

#[test]
fn sector_offset_rejects_out_of_range_track_side_and_sector() {
    let disk = JvcDisk::from_bytes(vec![0u8; ONE_TRACK_BYTES]).unwrap(); // 1 track
    assert_eq!(disk.sector_offset(1, 0, 1), None, "track beyond track_count");
    assert_eq!(disk.sector_offset(0, 1, 1), None, "side beyond sides (single-sided)");
    assert_eq!(disk.sector_offset(0, 0, 0), None, "sector below first_sector_id");
    assert_eq!(disk.sector_offset(0, 0, 19), None, "sector beyond sectors_per_track");
    assert!(disk.sector_offset(0, 0, 1).is_some());
    assert!(disk.sector_offset(0, 0, 18).is_some());
}

#[test]
fn invalid_geometry_is_rejected() {
    // Headerless (multiple of 256) but not a whole number of default-geometry
    // tracks (4608 bytes/track).
    let bytes = vec![0u8; 5120];
    assert_eq!(bytes.len() % 256, 0);
    let err = JvcDisk::from_bytes(bytes).unwrap_err();
    assert!(matches!(err, JvcError::InvalidGeometry { .. }));
}

#[test]
fn nonzero_attribute_flag_is_rejected() {
    // 5-byte header (spt=18, sides=1, size code=1, first id=1, attr flag=1),
    // one otherwise-valid track of data.
    let mut bytes = vec![18u8, 1, 1, 1, 1];
    bytes.extend(vec![0u8; ONE_TRACK_BYTES]);
    assert_eq!(JvcDisk::from_bytes(bytes).unwrap_err(), JvcError::AttributeBytesUnsupported);
}

// ============================================================================
// OS-9 LSN0 geometry sniffing for headerless images (os9_dsk.cpp find_size)
// ============================================================================

/// JVC default sectors/track and sector size, as assumed for any headerless
/// image (the sniff only ever trusts LSN0 against these).
const OS9_SPT: usize = 18;
const OS9_SECTOR_SIZE: usize = 256;
/// Byte offset within a sector safely clear of every LSN0 field the sniff
/// reads (0x00-0x02, 0x10-0x12), used to stamp per-(track,side) markers
/// without corrupting the identification sector under test.
const MARKER_OFFSET: usize = 0x50;

/// Build a headerless image of `tracks` x `sides` x 18 x 256B with a
/// consistent OS-9 LSN0 (DD.TOT/DD.FMT/DD.SPT) stamped into its first sector.
fn os9_synthetic_disk(tracks: usize, sides: usize) -> Vec<u8> {
    let total_sectors = tracks * sides * OS9_SPT;
    let mut bytes = vec![0u8; total_sectors * OS9_SECTOR_SIZE];
    bytes[0] = ((total_sectors >> 16) & 0xFF) as u8;
    bytes[1] = ((total_sectors >> 8) & 0xFF) as u8;
    bytes[2] = (total_sectors & 0xFF) as u8;
    bytes[0x10] = if sides == 2 { 1 } else { 0 };
    bytes[0x11] = ((OS9_SPT >> 8) & 0xFF) as u8;
    bytes[0x12] = (OS9_SPT & 0xFF) as u8;
    bytes
}

#[test]
fn os9_lsn0_sniff_adopts_two_sides_and_halves_track_count() {
    // Mirrors the real 368,640-byte NitrOS-9 40-track/2-side disk: a bare JVC
    // default parse would see 80 tracks x 1 side, but a consistent LSN0
    // declaring 2 sides must flip that to 40 x 2.
    const TRACKS: usize = 40;
    const SIDES: usize = 2;
    let mut bytes = os9_synthetic_disk(TRACKS, SIDES);
    // Stamp a marker identifying (track, side) into every sector-1, at an
    // offset clear of the LSN0 fields, to check the adopted geometry's
    // interleaving (track-major, side-interleaved — same row order `JvcDisk`
    // already uses for explicit 2-sided headers).
    for track in 0..TRACKS {
        for side in 0..SIDES {
            let row = track * SIDES + side;
            let sector_off = row * OS9_SPT * OS9_SECTOR_SIZE;
            bytes[sector_off + MARKER_OFFSET] = (row % 256) as u8;
        }
    }

    let disk = JvcDisk::from_bytes(bytes).unwrap();
    assert_eq!(disk.track_count(), TRACKS);
    assert_eq!(disk.sides(), SIDES);
    assert_eq!(disk.sectors_per_track(), OS9_SPT);

    for track in 0..TRACKS as u8 {
        for side in 0..SIDES as u8 {
            let off = disk.sector_offset(track, side, 1).unwrap();
            let row = track as usize * SIDES + side as usize;
            let expected = (row % 256) as u8;
            assert_eq!(
                disk.read_bytes(off + MARKER_OFFSET, 1)[0],
                expected,
                "track{track} side{side}"
            );
        }
    }
}

#[test]
fn os9_lsn0_with_mismatched_tot_keeps_naive_defaults() {
    // LSN0 claims 2 sides (DD.FMT bit 0 set) but DD.TOT is corrupted so it no
    // longer matches file_len -- the sniff must reject it outright and the
    // naive JVC-default geometry (18 spt, 1 side) must be kept exactly.
    const TRACKS: usize = 10;
    let mut bytes = os9_synthetic_disk(TRACKS, 1);
    bytes[0x10] = 1; // claims 2 sides
    bytes[2] = bytes[2].wrapping_add(1); // DD.TOT now wrong
    let disk = JvcDisk::from_bytes(bytes).unwrap();
    assert_eq!(disk.sides(), 1);
    assert_eq!(disk.track_count(), TRACKS);
    assert_eq!(disk.sectors_per_track(), OS9_SPT);
}

#[test]
fn headerless_disk_with_no_os9_signature_is_unaffected_by_the_sniff() {
    // All-zero LSN0 (DD.TOT bytes all zero, no OS-9 signature): DD.SPT reads
    // as 0 != 18, so the sniff must reject it and the plain RS-DOS 35-track
    // default parse (already covered by `headerless_image_uses_all_defaults`)
    // is unaffected. Restated here to pin the sniff-rejection path directly.
    let bytes = vec![0u8; 35 * ONE_TRACK_BYTES];
    let disk = JvcDisk::from_bytes(bytes).unwrap();
    assert_eq!(disk.track_count(), 35);
    assert_eq!(disk.sides(), 1);
}

/// The real NitrOS-9 Level 2 CoCo3 40-track disk image, if present
/// (git-ignored, local-only asset — see `CLAUDE.md`). Skips gracefully when
/// absent, following `load_rom`'s pattern below.
#[test]
fn real_nitros9_40_track_disk_parses_as_40_tracks_2_sides() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../disks/NOS9_6809_L2_v030300_coco3_40d_1.dsk");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("skipping real_nitros9_40_track_disk_parses_as_40_tracks_2_sides: {} not present", path.display());
        return;
    };
    let disk = JvcDisk::from_bytes(bytes).unwrap();
    assert_eq!(disk.track_count(), 40);
    assert_eq!(disk.sides(), 2);
}

// ============================================================================
// DSKREG decode + update_lines (coco_fdc.cpp)
// ============================================================================

fn load_rom(name: &str) -> Box<[u8]> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display())).into_boxed_slice()
}

fn disk_cart() -> DiskCart {
    DiskCart::new(load_rom("disk11.rom"))
}

/// One track, one sector, every byte the given marker — enough to identify
/// which drive/side answered a Read Sector without caring about pacing.
fn marker_disk(marker: u8) -> JvcDisk {
    JvcDisk::from_bytes(vec![marker; ONE_TRACK_BYTES]).unwrap()
}

/// One track, 2 sides, 1 sector/side, 128B — for side-select coverage.
fn two_sided_marker_disk(side0: u8, side1: u8) -> JvcDisk {
    let mut bytes = vec![1u8, 2, 0, 1, 0]; // spt=1, sides=2, size 128, first id 1
    bytes.extend(vec![side0; 128]);
    bytes.extend(vec![side1; 128]);
    JvcDisk::from_bytes(bytes).unwrap()
}

/// Comfortably more than the implementation's first-byte search latency (~30
/// byte times: the ID-field-to-data-field span a Read Sector spends before the
/// first byte lands), so one sector read's first byte is ready.
const ONE_DRQ_INTERVAL: u32 = 1200;

/// Read sector 1 of track 0 through the currently-selected drive/side and
/// return the byte delivered. Force-Interrupts first so a previous call's
/// still-in-flight command (this only ticks long enough for the first byte,
/// not a whole 256-byte sector) doesn't cause the new command to be silently
/// ignored (spec: a command written while busy is ignored).
fn read_marker_byte(cart: &mut DiskCart) -> u8 {
    cart.write(0xFF48, 0xD0); // Force Interrupt, cancel only
    cart.write(0xFF49, 0); // track register
    cart.write(0xFF4A, 1); // sector register
    cart.write(0xFF48, 0x80); // Read Sector, single
    cart.tick(ONE_DRQ_INTERVAL);
    cart.read(0xFF4B)
}

#[test]
fn reset_state_has_drq_set_so_halt_never_spuriously_asserts() {
    let cart = disk_cart();
    // dskreg=0 out of the gate, so halt-enable is clear regardless of drq —
    // this is the "reset state drq=true" fact under direct test.
    assert!(!cart.halt_asserted());
}

#[test]
fn halt_line_is_not_drq_and_halt_enable() {
    let mut cart = disk_cart();
    cart.write(0xFF40, dskreg::HALT_ENABLE);
    // drq is still true (nothing has cleared it yet): HALT must not assert.
    assert!(!cart.halt_asserted());
    cart.read(0xFF4B); // clears DRQ as a side effect
    assert!(cart.halt_asserted(), "HALT must assert once DRQ clears with halt-enable set");
}

#[test]
fn intrq_high_clears_dskreg_halt_enable() {
    let mut cart = disk_cart();
    cart.write(0xFF40, dskreg::HALT_ENABLE);
    cart.read(0xFF4B); // drq now false -> halt asserted
    assert!(cart.halt_asserted());
    cart.write(0xFF48, 0xD8); // Force Interrupt, I3 set -> INTRQ high
    assert!(!cart.halt_asserted(), "a high INTRQ must clear DSKREG's halt-enable bit");
}

#[test]
fn nmi_edge_fires_only_when_density_nmi_enable_bit_is_set() {
    let mut cart = disk_cart();
    cart.write(0xFF40, 0); // bit5 clear
    cart.write(0xFF48, 0xD8); // Force Interrupt, I3 -> INTRQ high
    assert!(!cart.take_nmi(), "NMI must not fire when DSKREG bit5 is clear");

    let mut cart = disk_cart();
    cart.write(0xFF40, dskreg::DENSITY_AND_NMI_ENABLE);
    cart.write(0xFF48, 0xD8);
    assert!(cart.take_nmi(), "NMI must fire on the rising edge of intrq && bit5");
    assert!(!cart.take_nmi(), "the edge must not repeat once consumed");
}

#[test]
fn dskreg_reads_are_open_bus() {
    let mut cart = disk_cart();
    cart.write(0xFF40, 0xFF);
    for addr in 0xFF40u16..=0xFF47 {
        assert_eq!(cart.read(addr), coco_core::cart::IO_OPEN_BUS);
    }
}

#[test]
fn drive_select_priority_bit2_then_bit1_then_bit0_then_bit6() {
    let mut cart = disk_cart();
    cart.insert_disk(0, marker_disk(10));
    cart.insert_disk(1, marker_disk(11));
    cart.insert_disk(2, marker_disk(12));
    cart.insert_disk(3, marker_disk(13));

    let cases: [(u8, u8); 4] = [
        (dskreg::DRIVE2 | dskreg::DRIVE1 | dskreg::DRIVE0, 12), // bit2 wins
        (dskreg::DRIVE1 | dskreg::DRIVE0, 11),                  // bit1 wins (no bit2)
        (dskreg::DRIVE0, 10),                                   // bit0 wins
        (dskreg::DRIVE3_OR_SIDE, 13),                           // only bit6 -> drive 3
    ];
    for (select_bits, expect_marker) in cases {
        cart.write(0xFF40, dskreg::MOTOR_ON | select_bits);
        assert_eq!(read_marker_byte(&mut cart), expect_marker, "select bits {select_bits:#04x}");
    }
}

#[test]
fn side_select_is_bit6_unless_it_is_selecting_drive3() {
    let mut cart = disk_cart();
    cart.insert_disk(0, two_sided_marker_disk(20, 21));

    cart.write(0xFF40, dskreg::MOTOR_ON | dskreg::DRIVE0);
    assert_eq!(read_marker_byte(&mut cart), 20, "bit6 clear -> side 0");

    cart.write(0xFF40, dskreg::MOTOR_ON | dskreg::DRIVE0 | dskreg::DRIVE3_OR_SIDE);
    assert_eq!(read_marker_byte(&mut cart), 21, "bit6 set with drive0 selected -> side 1");
}

#[test]
fn not_ready_status_bit_reflects_missing_disk_or_motor_off() {
    let mut cart = disk_cart();
    cart.insert_disk(0, marker_disk(1));

    cart.write(0xFF40, dskreg::MOTOR_ON | dskreg::DRIVE0);
    assert_eq!(cart.read(0xFF48) & status::NOT_READY, 0, "mounted + motor on -> ready");

    cart.write(0xFF40, dskreg::DRIVE0); // motor off
    assert_eq!(cart.read(0xFF48) & status::NOT_READY, status::NOT_READY, "motor off -> not ready");

    cart.write(0xFF40, dskreg::MOTOR_ON | dskreg::DRIVE1); // unmounted drive
    assert_eq!(cart.read(0xFF48) & status::NOT_READY, status::NOT_READY, "unmounted drive -> not ready");
}

// ============================================================================
// WD1773 command state machine
// ============================================================================

/// A disk whose track 0, sector 1 is filled with `i as u8` for `i` in
/// `0..256` — lets a read-sector test confirm both delivery order and value.
fn index_pattern_disk() -> JvcDisk {
    let bytes: Vec<u8> = (0..ONE_TRACK_BYTES).map(|i| i as u8).collect();
    JvcDisk::from_bytes(bytes).unwrap()
}

/// Comfortably longer than the implementation's fixed Type I/RNF settle delay.
const SETTLE: u32 = 200;
/// The implementation's DRQ pacing interval, mirrored here so tests can step
/// exactly one byte at a time.
const DRQ_INTERVAL: u32 = 30;
/// The implementation's first-byte search latency (30 byte times): a Read/Write
/// Sector command's first DRQ trails the command by the ID-field-to-data-field
/// span, not one DRQ interval.
const FIRST_BYTE_LATENCY: u32 = 30 * DRQ_INTERVAL;
/// Mirrors the WD1773's CRC trailer: the gap between a read's final data-byte
/// DRQ and command completion (2 byte times).
const CRC_TRAILER: u32 = 2 * DRQ_INTERVAL;

#[test]
fn restore_zeroes_track_register_and_sets_track0_plus_intrq() {
    let mut wd = WD1773::new();
    let mut disk = index_pattern_disk();
    wd.track = 5;
    wd.write_command(0x00, Some(&mut disk), 0); // Restore, no verify
    assert!(wd.busy);
    wd.tick(SETTLE, Some(&mut disk), 0);
    assert!(!wd.busy);
    assert!(wd.intrq);
    assert_eq!(wd.track, 0);
    let s = wd.read_status(true, true);
    assert_eq!(s & status::TRACK0, status::TRACK0);
    assert!(!wd.intrq, "reading status must clear INTRQ");
}

#[test]
fn seek_moves_to_the_data_register_value() {
    let mut wd = WD1773::new();
    let mut disk = JvcDisk::from_bytes(vec![0u8; 10 * ONE_TRACK_BYTES]).unwrap(); // 10 tracks
    wd.data = 5;
    wd.write_command(0x10, Some(&mut disk), 0); // Seek, no verify
    wd.tick(SETTLE, Some(&mut disk), 0);
    assert!(!wd.busy);
    assert!(wd.intrq);
    assert_eq!(wd.track, 5);
    assert_eq!(wd.read_status(true, true) & status::TRACK0, 0, "track 5 is not track 0");
}

#[test]
fn verify_sets_rnf_when_the_target_track_is_beyond_the_image() {
    let mut wd = WD1773::new();
    let mut disk = JvcDisk::from_bytes(vec![0u8; ONE_TRACK_BYTES]).unwrap(); // 1 track only
    wd.data = 5; // beyond the single mounted track
    wd.write_command(0x14, Some(&mut disk), 0); // Seek with Verify (V bit set)
    wd.tick(SETTLE, Some(&mut disk), 0);
    assert!(!wd.busy);
    assert!(wd.intrq, "RNF still completes with INTRQ");
    assert_eq!(wd.read_status(true, true) & status::RECORD_NOT_FOUND, status::RECORD_NOT_FOUND);
}

#[test]
fn read_sector_delivers_256_correct_bytes_paced_by_drq_then_intrq() {
    let mut wd = WD1773::new();
    let mut disk = index_pattern_disk();
    wd.track = 0;
    wd.sector = 1;
    wd.write_command(0x80, Some(&mut disk), 0); // Read Sector, no multiple
    assert!(wd.busy);
    for expected in 0..256u32 {
        // The first byte waits out the sector-search latency; the rest pace at
        // one DRQ interval each.
        let step = if expected == 0 { FIRST_BYTE_LATENCY } else { DRQ_INTERVAL };
        wd.tick(step, Some(&mut disk), 0);
        assert!(wd.drq, "DRQ must be asserted for byte {expected}");
        // INTRQ must trail the final byte's DRQ by the CRC-read time: if it
        // rose together with it, the FD-502's NMI would preempt the halt
        // loop's collection of the last byte of every sector.
        assert!(!wd.intrq, "INTRQ before byte {expected} was collected");
        assert_eq!(wd.read_data(), expected as u8, "byte {expected}");
    }
    wd.tick(CRC_TRAILER, Some(&mut disk), 0);
    assert!(!wd.busy, "busy must clear once all 256 bytes are delivered");
    assert!(wd.intrq);
    assert_eq!(
        wd.read_status(true, true) & status::LOST_DATA,
        0,
        "a fully-serviced read must not report LOST DATA"
    );
}

/// Regression: a Read Sector's first data byte must not arrive during the short
/// setup window between the command write and the driver arming its transfer
/// loop. NitrOS-9 Level 2's `boot_1773` issues Read Sector, runs a ~54-cycle
/// `Delay2`, THEN enables HALT and enters its `LDA DATAREG` loop; if the first
/// byte lands inside that delay the driver never collects it and the transfer
/// trips LOST DATA, which its NMI handler reads as `E$Read` (the boot prints
/// FAILED). Pacing the first byte at one DRQ interval — the pre-fix behaviour —
/// put it squarely inside the window.
#[test]
fn read_sector_first_byte_waits_out_the_driver_setup_delay() {
    /// Comfortably longer than boot_1773's ~54-cycle post-command `Delay2`.
    const DRIVER_SETUP_DELAY: u32 = 70;

    let mut wd = WD1773::new();
    let mut disk = index_pattern_disk();
    wd.track = 0;
    wd.sector = 1;
    wd.write_command(0x80, Some(&mut disk), 0); // Read Sector, no multiple

    // The driver hasn't started collecting bytes yet: no DRQ may fire (and thus
    // no byte can be lost) during its post-command setup delay.
    wd.tick(DRIVER_SETUP_DELAY, Some(&mut disk), 0);
    assert!(!wd.drq, "no DRQ may fire during the driver's post-command setup delay");

    // Now collect all 256 bytes the way the HALT loop does: spin one byte-time
    // at a time until each DRQ, then take the byte.
    for expected in 0..256u32 {
        while !wd.drq {
            wd.tick(DRQ_INTERVAL, Some(&mut disk), 0);
        }
        assert_eq!(wd.read_data(), expected as u8, "byte {expected}");
    }
    wd.tick(CRC_TRAILER, Some(&mut disk), 0);
    assert!(!wd.busy);
    assert!(wd.intrq);
    assert_eq!(
        wd.read_status(true, true) & status::LOST_DATA,
        0,
        "the first byte must survive the setup delay — no LOST DATA"
    );
}

/// Regression: a Read Sector's data field is fetched from whichever side the
/// head sits over when the field *streams* (after the ID-address-mark search),
/// not from the side selected when the command was written. The WD1773 has no
/// side input — head select is the external DSKREG bit, sampled continuously.
///
/// NitrOS-9 Level 2's RBF driver relies on this when a sequential read crosses a
/// side boundary: it writes the Read Sector command with the *old* side still
/// latched in DSKREG, then flips DSKREG to the new side before its (halting)
/// `LDA DATAREG` loop collects the first byte. Sampling the side at command
/// dispatch instead reads the wrong physical side — off by one full track's
/// worth of sectors — silently corrupting every module whose body straddles a
/// side boundary (e.g. `rb1773`), which wedges the boot at "NITROS9 BOOT".
#[test]
fn read_sector_samples_side_when_the_data_field_streams_not_at_dispatch() {
    // Two-sided default-geometry image; mark (track 0, sector 1) distinctly on
    // each side so the delivered byte reveals which side was actually read.
    const SIDE0_MARK: u8 = 0xAA;
    const SIDE1_MARK: u8 = 0x55;
    let mut header = vec![18u8, 2u8]; // spt=18, sides=2; rest defaults (256B)
    header.extend(vec![0u8; ONE_TRACK_BYTES * 2]); // one 2-sided track
    let mut disk = JvcDisk::from_bytes(header).unwrap();
    assert_eq!(disk.sides(), 2);
    let off0 = disk.sector_offset(0, 0, 1).unwrap();
    let off1 = disk.sector_offset(0, 1, 1).unwrap();
    for i in 0..disk.sector_size() {
        disk.write_byte(off0 + i, SIDE0_MARK);
        disk.write_byte(off1 + i, SIDE1_MARK);
    }

    let mut wd = WD1773::new();
    wd.track = 0;
    wd.sector = 1;
    // Command written while DSKREG still selects side 0 (the previous sector's
    // side).
    wd.write_command(0x80, Some(&mut disk), 0); // Read Sector, single
    // DSKREG flips to side 1 during the ID-search latency, before the data
    // field streams — modelled by ticking the first-byte latency with side 1.
    wd.tick(FIRST_BYTE_LATENCY, Some(&mut disk), 1);
    assert!(wd.drq, "first byte must be ready after the search latency");
    assert_eq!(
        wd.read_data(),
        SIDE1_MARK,
        "the data field must come from the side selected when it streams (side 1), \
         not the side latched at command dispatch (side 0)"
    );
}

#[test]
fn write_sector_round_trips_into_the_image() {
    let mut wd = WD1773::new();
    let mut disk = JvcDisk::from_bytes(vec![0u8; ONE_TRACK_BYTES]).unwrap();
    wd.track = 0;
    wd.sector = 1;
    wd.write_command(0xA0, Some(&mut disk), 0); // Write Sector, no multiple
    assert!(wd.busy);
    for expected in 0..256u32 {
        // The first byte request waits out the sector-search latency.
        let step = if expected == 0 { FIRST_BYTE_LATENCY } else { DRQ_INTERVAL };
        wd.tick(step, Some(&mut disk), 0);
        assert!(wd.drq, "DRQ must request byte {expected}");
        wd.write_data(expected as u8, Some(&mut disk), 0);
    }
    assert!(!wd.busy);
    assert!(wd.intrq);
    let off = disk.sector_offset(0, 0, 1).unwrap();
    let written = disk.read_bytes(off, 256);
    let expected: Vec<u8> = (0..256u32).map(|i| i as u8).collect();
    assert_eq!(written, expected.as_slice());
}

#[test]
fn write_sector_to_a_write_protected_image_sets_status_and_does_not_transfer() {
    let mut wd = WD1773::new();
    let mut disk = JvcDisk::from_bytes(vec![0xAAu8; ONE_TRACK_BYTES]).unwrap();
    disk.set_write_protected(true);
    wd.track = 0;
    wd.sector = 1;
    wd.write_command(0xA0, Some(&mut disk), 0);
    assert!(!wd.busy, "write-protected write must not transfer");
    assert!(wd.intrq);
    assert_eq!(wd.read_status(true, true) & status::WRITE_PROTECT, status::WRITE_PROTECT);
    let off = disk.sector_offset(0, 0, 1).unwrap();
    assert_eq!(disk.read_bytes(off, 1)[0], 0xAA, "image must be untouched");
}

#[test]
fn read_sector_sets_rnf_when_the_sector_is_missing() {
    let mut wd = WD1773::new();
    let mut disk = JvcDisk::from_bytes(vec![0u8; ONE_TRACK_BYTES]).unwrap(); // spt=18
    wd.track = 0;
    wd.sector = 99; // out of range
    wd.write_command(0x80, Some(&mut disk), 0);
    wd.tick(SETTLE, Some(&mut disk), 0);
    assert!(!wd.busy);
    assert!(wd.intrq);
    assert_eq!(wd.read_status(true, true) & status::RECORD_NOT_FOUND, status::RECORD_NOT_FOUND);
}

#[test]
fn force_interrupt_cancels_a_pending_command() {
    let mut wd = WD1773::new();
    let mut disk = JvcDisk::from_bytes(vec![0u8; ONE_TRACK_BYTES]).unwrap();
    wd.track = 0;
    wd.sector = 1;
    wd.write_command(0xA0, Some(&mut disk), 0); // Write Sector: busy, awaiting DRQ
    assert!(wd.busy);
    wd.write_command(0xD0, None, 0); // Force Interrupt, low nibble 0: cancel only
    assert!(!wd.busy);
    assert!(!wd.intrq, "low nibble 0 must not raise INTRQ");
    // The cancelled write must never complete on its own.
    wd.tick(10_000, Some(&mut disk), 0);
    assert!(!wd.intrq);
    assert!(!wd.busy);
}

#[test]
fn force_interrupt_with_i3_sets_intrq_even_while_idle() {
    let mut wd = WD1773::new();
    assert!(!wd.busy);
    wd.write_command(0xD8, None, 0); // Force Interrupt, I3 set
    assert!(wd.intrq);
}

#[test]
fn status_read_clears_intrq() {
    let mut wd = WD1773::new();
    wd.write_command(0xD8, None, 0);
    assert!(wd.intrq);
    wd.read_status(true, true);
    assert!(!wd.intrq);
}

#[test]
fn multiple_read_increments_the_sector_register_then_rnf_past_the_last_sector() {
    // spt=3 so the run-off-the-end case is reachable quickly.
    let mut bytes = vec![3u8, 1, 1, 1, 0]; // spt=3, sides=1, size 256, first id 1
    for sector in 1u8..=3 {
        bytes.extend(vec![sector; 256]);
    }
    let mut disk = JvcDisk::from_bytes(bytes).unwrap();
    let mut wd = WD1773::new();
    wd.track = 0;
    wd.sector = 1;
    wd.write_command(0x90, Some(&mut disk), 0); // Read Sector, multiple

    for sector in 1u8..=3 {
        assert_eq!(wd.sector, sector, "sector register before reading sector {sector}");
        for byte in 0..256 {
            // Only the command's very first byte waits out the sector-search
            // latency; multiple-sector continuations roll on at one DRQ interval.
            let step = if sector == 1 && byte == 0 { FIRST_BYTE_LATENCY } else { DRQ_INTERVAL };
            wd.tick(step, Some(&mut disk), 0);
            assert_eq!(wd.read_data(), sector, "sector {sector}");
        }
        // The CRC trailer after the sector's last byte doubles as the
        // inter-sector gap: once it elapses, the register rolls onto the next
        // sector — or, past the end of the track, the lookup fails with RNF.
        wd.tick(CRC_TRAILER, Some(&mut disk), 0);
    }
    assert!(!wd.busy);
    assert!(wd.intrq);
    assert_eq!(wd.read_status(true, true) & status::RECORD_NOT_FOUND, status::RECORD_NOT_FOUND);
}

// ----------------------------------------------------------------------------
// Write Track (format) MFM stream parsing
// ----------------------------------------------------------------------------

/// Verified DSKINI 1.1 write-track byte layout (disk11.rom's format template
/// at $D6D4), broken into named field lengths: gap1, then per sector 8x$00
/// sync-lead-in, 3x$F5, $FE (ID AM), 4 literal ID bytes, $F7, gap2, 12x$00
/// sync-lead-in, 3x$F5, $FB (data AM), sector-size data bytes, $F7, gap3; a
/// final 200x$4E gap4 closes the track (not reproduced verbatim by the test
/// below — see [`write_track_parses_a_synthetic_dskini_stream_into_the_image`]).
const DSKINI_GAP1: usize = 32;
const DSKINI_SYNC_LEAD_IN: usize = 8;
const DSKINI_SYNC_COUNT: usize = 3;
const DSKINI_GAP2: usize = 22;
const DSKINI_SYNC_LEAD_IN2: usize = 12;
const DSKINI_GAP3: usize = 24;

/// Append one DSKINI-style formatted sector's MFM stream bytes (ID field +
/// data field) to `stream`. `literal_side` is the ID field's own side byte —
/// per spec the WD1773's Write Track never derives side from it (side comes
/// from the hardware side-select parameter instead), so it's fine for this
/// to be garbage in the test.
fn push_formatted_sector(
    stream: &mut Vec<u8>,
    track: u8,
    literal_side: u8,
    sector: u8,
    size_code: u8,
    fill: u8,
) {
    stream.extend(std::iter::repeat_n(0x00, DSKINI_SYNC_LEAD_IN));
    stream.extend(std::iter::repeat_n(0xF5, DSKINI_SYNC_COUNT));
    stream.push(0xFE); // ID address mark
    stream.push(track);
    stream.push(literal_side);
    stream.push(sector);
    stream.push(size_code);
    stream.push(0xF7); // "write CRC": terminates the ID field
    stream.extend(std::iter::repeat_n(0x4E, DSKINI_GAP2));
    stream.extend(std::iter::repeat_n(0x00, DSKINI_SYNC_LEAD_IN2));
    stream.extend(std::iter::repeat_n(0xF5, DSKINI_SYNC_COUNT));
    stream.push(0xFB); // data address mark
    stream.extend(std::iter::repeat_n(fill, 128usize << size_code));
    stream.push(0xF7); // "write CRC": terminates the data field, lays the sector
    stream.extend(std::iter::repeat_n(0x4E, DSKINI_GAP3));
}

/// Drives a Write Track (`$F0`) command through the MFM format-stream parser
/// with a synthetic 2-sector DSKINI-style stream against a blank (0-track)
/// image. Confirms both sectors land at the right offset with the right
/// content, the image grows to include the newly-formatted track, and the ID
/// field's literal side byte (deliberately garbage here) is ignored in favor
/// of the hardware side parameter — this is what makes in-emulator DSKINI
/// possible (previously the byte stream was just discarded).
#[test]
fn write_track_parses_a_synthetic_dskini_stream_into_the_image() {
    const TRACK: u8 = 5;
    const HW_SIDE: u8 = 0;
    const LITERAL_SIDE: u8 = 0x99; // garbage: must be ignored, see doc comment
    const SIZE_CODE: u8 = 1; // 256B, matches the blank image's default geometry

    let mut wd = WD1773::new();
    wd.set_double_density(true); // exercise the MFM parser explicitly, not just the default
    let mut disk = JvcDisk::from_bytes(Vec::new()).unwrap();
    assert_eq!(disk.track_count(), 0, "starting from a blank image");

    let mut stream = Vec::new();
    stream.extend(std::iter::repeat_n(0x4E, DSKINI_GAP1));
    push_formatted_sector(&mut stream, TRACK, LITERAL_SIDE, 1, SIZE_CODE, 0x7A);
    push_formatted_sector(&mut stream, TRACK, LITERAL_SIDE, 2, SIZE_CODE, 0x7B);

    wd.write_command(0xF0, Some(&mut disk), HW_SIDE); // Write Track, no options
    assert!(wd.busy);

    let mut sent = 0usize;
    for &b in &stream {
        wd.tick(DRQ_INTERVAL, Some(&mut disk), HW_SIDE);
        wd.write_data(b, Some(&mut disk), HW_SIDE);
        sent += 1;
    }
    // Drain the rest of the command's fixed byte budget (gap4-style filler)
    // until INTRQ; the exact budget is an internal implementation constant,
    // so loop until done rather than hard-coding it, with a generous safety
    // cap against an infinite loop if that regresses.
    const SAFETY_CAP: usize = 8_000;
    while wd.busy {
        assert!(sent < SAFETY_CAP, "Write Track never completed");
        wd.tick(DRQ_INTERVAL, Some(&mut disk), HW_SIDE);
        wd.write_data(0x4E, Some(&mut disk), HW_SIDE);
        sent += 1;
    }
    assert!(wd.intrq);

    assert_eq!(
        disk.track_count(),
        TRACK as usize + 1,
        "image must grow to include the formatted track"
    );
    for (sector, marker) in [(1u8, 0x7Au8), (2u8, 0x7Bu8)] {
        let off = disk
            .sector_offset(TRACK, HW_SIDE, sector)
            .expect("formatted sector must be present");
        let bytes = disk.read_bytes(off, 256);
        assert!(
            bytes.iter().all(|&b| b == marker),
            "sector {sector} data must be the fill byte {marker:#04X}"
        );
    }
}

/// Write Track's write-protect check mirrors Write Sector's: fires
/// immediately (no DRQ pacing, no transfer started) and leaves the image
/// completely untouched — there's no target sector to fail to find, so
/// (unlike Write Sector's `None` arm) an *unmounted* drive would just proceed
/// with the transfer; only write-protect short-circuits.
#[test]
fn write_track_to_a_write_protected_image_sets_status_and_does_not_transfer() {
    let mut wd = WD1773::new();
    wd.set_double_density(true);
    let mut disk = JvcDisk::from_bytes(vec![0xAAu8; ONE_TRACK_BYTES]).unwrap();
    disk.set_write_protected(true);
    let before = disk.bytes().to_vec();

    wd.write_command(0xF0, Some(&mut disk), 0); // Write Track, no options
    assert!(!wd.busy, "write-protected Write Track must not transfer");
    assert!(wd.intrq);
    assert_eq!(wd.read_status(true, true) & status::WRITE_PROTECT, status::WRITE_PROTECT);
    assert_eq!(disk.track_count(), 1, "image must not grow");
    assert_eq!(disk.bytes(), before.as_slice(), "image must be completely unchanged");
}

// ============================================================================
// Integration: boot Disk Extended Color BASIC and read a synthesized RS-DOS
// directory via DIR.
// ============================================================================

fn boot_machine() -> Machine {
    Machine::new(MachineConfig::default(), load_rom("coco3.rom"))
}

fn screen_row(m: &mut Machine, row: u16) -> String {
    (0..32)
        .map(|c| {
            let code = m.bus.read(0x0400 + row * 32 + c) & 0x3F;
            if code < 0x20 { (b'@' + code) as char } else { (b' ' + (code - 0x20)) as char }
        })
        .collect()
}

fn tap(m: &mut Machine, pos: (u8, u8)) {
    for _ in 0..3 {
        m.bus.keyboard.set(pos, true);
        m.run_field();
    }
    m.bus.keyboard.set(pos, false);
    for _ in 0..3 {
        m.run_field();
    }
}

fn tap_char(m: &mut Machine, c: char) {
    let (pos, shift) = coco_core::keyboard::char_key(c).unwrap_or_else(|| panic!("no key for {c:?}"));
    if shift {
        m.bus.keyboard.set(coco_core::keyboard::SHIFT, true);
    }
    tap(m, pos);
    if shift {
        m.bus.keyboard.set(coco_core::keyboard::SHIFT, false);
    }
}

fn type_str(m: &mut Machine, s: &str) {
    for c in s.chars() {
        tap_char(m, c);
    }
}

/// Directory track (RS-DOS): track 17, 0-indexed physical track (the 18th
/// track on a 35-track disk).
const DIR_TRACK: u8 = 17;
/// GAT (granule allocation table) sector.
const GAT_SECTOR: u8 = 2;
/// First directory-entry sector; entries run through sector 11.
const DIR_FIRST_SECTOR: u8 = 3;
const DIR_LAST_SECTOR: u8 = 11;
const SECTOR_SIZE: usize = 256;
const DIR_ENTRY_SIZE: usize = 32;

/// Synthesize a headerless 35-track/18-spt/1-side/256B RS-DOS disk with one
/// file, "HELLO.BAS", occupying granule 0 (track 0, both granules — i.e. the
/// first 2 tracks worth of granules; RS-DOS granules are half-tracks, 2 per
/// track, 9 sectors each on a 18-spt disk).
fn synthesized_rsdos_disk(filename8: &str, ext3: &str) -> JvcDisk {
    const TRACKS: usize = 35;
    let mut bytes = vec![0u8; TRACKS * ONE_TRACK_BYTES];

    let track_offset = |track: usize, sector: u8| -> usize {
        (track * 18 + (sector as usize - 1)) * SECTOR_SIZE
    };

    // GAT: 68 granules, $FF = free, except granule 0 = $C1 (last granule of
    // the file, 1 sector used in its last sector).
    let gat_off = track_offset(DIR_TRACK as usize, GAT_SECTOR);
    bytes[gat_off..gat_off + 68].fill(0xFF);
    bytes[gat_off] = 0xC1;

    // Directory entries: sectors 3..=11, 8 x 32-byte entries per sector,
    // unused entries start with $FF. First entry = our file.
    for sector in DIR_FIRST_SECTOR..=DIR_LAST_SECTOR {
        let off = track_offset(DIR_TRACK as usize, sector);
        bytes[off..off + SECTOR_SIZE].fill(0xFF);
    }
    let entry_off = track_offset(DIR_TRACK as usize, DIR_FIRST_SECTOR);
    let mut name = [b' '; 8];
    for (i, b) in filename8.bytes().enumerate() {
        name[i] = b;
    }
    let mut ext = [b' '; 3];
    for (i, b) in ext3.bytes().enumerate() {
        ext[i] = b;
    }
    bytes[entry_off..entry_off + 8].copy_from_slice(&name);
    bytes[entry_off + 8..entry_off + 11].copy_from_slice(&ext);
    bytes[entry_off + 11] = 0x00; // file type: BASIC program
    bytes[entry_off + 12] = 0xFF; // ASCII flag (not used for BAS, but harmless)
    bytes[entry_off + 13] = 0x00; // first granule
    bytes[entry_off + 14] = 0x00; // bytes in last sector (MSB)
    bytes[entry_off + 15] = 0x01; // bytes in last sector (LSB) - 1 byte used
    for b in &mut bytes[entry_off + 16..entry_off + DIR_ENTRY_SIZE] {
        *b = 0;
    }

    JvcDisk::from_bytes(bytes).unwrap()
}

#[test]
fn boots_to_disk_basic_and_dir_lists_the_synthesized_file() {
    const FIELDS: usize = 400;
    let mut m = boot_machine();
    let mut cart = DiskCart::new(load_rom("disk11.rom"));
    cart.insert_disk(0, synthesized_rsdos_disk("HELLO", "BAS"));
    m.insert_cartridge(Box::new(cart));
    m.reset();
    for _ in 0..FIELDS {
        m.run_field();
    }
    let banner = (0..16).any(|r| screen_row(&mut m, r).contains("DISK EXTENDED COLOR BASIC"));
    assert!(banner, "expected the Disk BASIC banner; row0 = {:?}", screen_row(&mut m, 0));

    type_str(&mut m, "DIR");
    tap_char(&mut m, '\r');
    for _ in 0..FIELDS {
        m.run_field();
    }

    let found = (0..16).any(|r| screen_row(&mut m, r).contains("HELLO"));
    assert!(
        found,
        "expected DIR to list HELLO.BAS; screen:\n{}",
        (0..16).map(|r| screen_row(&mut m, r)).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn boots_to_disk_basic_without_a_disk_inserted() {
    const FIELDS: usize = 400;
    let mut m = boot_machine();
    m.insert_cartridge(Box::new(DiskCart::new(load_rom("disk11.rom"))));
    m.reset();
    for _ in 0..FIELDS {
        m.run_field();
    }
    let banner = (0..16).any(|r| screen_row(&mut m, r).contains("DISK EXTENDED COLOR BASIC"));
    assert!(banner, "expected the Disk BASIC banner even with no disk mounted (no spurious HALT)");
}

/// Hot-plugging the controller into a machine already sitting at the BASIC
/// prompt must power-cycle to take effect: the DK probe that links Disk
/// BASIC only runs on the ROM's cold-start path, and a warm reset skips it
/// because BASIC's warm-start magic is still in RAM (the frontend's
/// Insert/New Disk menu path relies on this).
#[test]
fn controller_added_mid_session_boots_disk_basic_after_power_cycle() {
    const FIELDS: usize = 400;
    let mut m = boot_machine();
    m.reset();
    for _ in 0..FIELDS {
        m.run_field();
    }
    let plain = (0..16).any(|r| screen_row(&mut m, r).contains("COLOR BASIC"))
        && !(0..16).any(|r| screen_row(&mut m, r).contains("DISK"));
    assert!(plain, "precondition: booted to non-disk BASIC");

    m.insert_cartridge(Box::new(DiskCart::new(load_rom("disk11.rom"))));
    m.power_cycle();
    for _ in 0..FIELDS {
        m.run_field();
    }
    let banner = (0..16).any(|r| screen_row(&mut m, r).contains("DISK EXTENDED COLOR BASIC"));
    assert!(banner, "expected Disk BASIC after mid-session insert + power cycle");
}

/// Like [`load_rom`] but returns `None` instead of panicking when the
/// (git-ignored) ROM image isn't present, so the LOADM regression below skips
/// gracefully in an asset-less checkout.
fn try_load_rom(name: &str) -> Option<Box<[u8]>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../roms")
        .join(name);
    std::fs::read(&path).ok().map(Vec::into_boxed_slice)
}

/// Synthesize a 35-track RS-DOS disk holding one machine-language file
/// (`name8`.BIN) whose `data` loads at `load_addr`. The file (a single DECB
/// binary load segment plus the exec trailer) occupies granule 0.
fn synthesized_ml_disk(name8: &str, load_addr: u16, data: &[u8]) -> JvcDisk {
    const TRACKS: usize = 35;
    let mut bytes = vec![0u8; TRACKS * ONE_TRACK_BYTES];
    let track_offset =
        |track: usize, sector: u8| -> usize { (track * 18 + (sector as usize - 1)) * SECTOR_SIZE };

    // DECB binary file image: one load segment ($00 len addr data) then the
    // exec trailer ($FF $0000 exec-addr).
    let len = data.len() as u16;
    let mut file = vec![
        0x00,
        (len >> 8) as u8,
        len as u8,
        (load_addr >> 8) as u8,
        load_addr as u8,
    ];
    file.extend_from_slice(data);
    file.extend_from_slice(&[0xFF, 0x00, 0x00, (load_addr >> 8) as u8, load_addr as u8]);

    // Lay the file into granule 0 (track 0, sectors 1..), a 9-sector granule.
    let sectors_used = file.len().div_ceil(SECTOR_SIZE);
    assert!(sectors_used <= 9, "test file must fit one granule");
    bytes[0..file.len()].copy_from_slice(&file);

    // GAT: granule 0 is the file's only (hence last) granule.
    let gat_off = track_offset(DIR_TRACK as usize, GAT_SECTOR);
    bytes[gat_off..gat_off + 68].fill(0xFF);
    bytes[gat_off] = 0xC0 | sectors_used as u8;

    // Directory: entry 0 = our file, the rest free ($FF).
    for sector in DIR_FIRST_SECTOR..=DIR_LAST_SECTOR {
        let off = track_offset(DIR_TRACK as usize, sector);
        bytes[off..off + SECTOR_SIZE].fill(0xFF);
    }
    let entry_off = track_offset(DIR_TRACK as usize, DIR_FIRST_SECTOR);
    let mut name = [b' '; 8];
    for (i, b) in name8.bytes().enumerate() {
        name[i] = b;
    }
    bytes[entry_off..entry_off + 8].copy_from_slice(&name);
    bytes[entry_off + 8..entry_off + 11].copy_from_slice(b"BIN");
    bytes[entry_off + 11] = 0x02; // file type: machine language
    bytes[entry_off + 12] = 0x00; // binary (not ASCII)
    bytes[entry_off + 13] = 0x00; // first granule
    let last_sector_bytes = (file.len() - (sectors_used - 1) * SECTOR_SIZE) as u16;
    bytes[entry_off + 14] = (last_sector_bytes >> 8) as u8;
    bytes[entry_off + 15] = last_sector_bytes as u8;

    JvcDisk::from_bytes(bytes).unwrap()
}

/// Regression: the FD-502 halt/NMI handshake must not drop a sector's final
/// byte. Before the fix, the sector-completion NMI preempted DSKCON's
/// `STB ,X+` for the last byte of every 256-byte sector — the MC6809
/// recognizes interrupts only at instruction-end boundaries, so the
/// instruction that HALT* released must run before the NMI is acknowledged.
/// One byte per sector was loading as $00. The payload here fills three
/// sectors with a non-zero marker; every byte, including each sector's 256th,
/// must survive LOADM.
#[test]
fn loadm_preserves_every_sector_byte_across_the_halt_nmi_handshake() {
    const LOAD_ADDR: u16 = 0x3F00;
    const DATA_LEN: usize = 600; // spans 3 sectors -> 2 sector-boundary bytes in payload
    const FILL: u8 = 0xE5;
    const FIELDS: usize = 400;

    let (Some(coco), Some(disk_rom)) = (try_load_rom("coco3.rom"), try_load_rom("disk11.rom"))
    else {
        eprintln!("skipping loadm_preserves_every_sector_byte: roms/ assets not present");
        return;
    };

    let data = vec![FILL; DATA_LEN];
    let mut m = Machine::new(MachineConfig::default(), coco);
    let mut cart = DiskCart::new(disk_rom);
    cart.insert_disk(0, synthesized_ml_disk("TESTML", LOAD_ADDR, &data));
    m.insert_cartridge(Box::new(cart));
    m.reset();
    for _ in 0..FIELDS {
        m.run_field();
    }
    assert!(
        (0..16).any(|r| screen_row(&mut m, r).contains("DISK EXTENDED COLOR BASIC")),
        "expected the Disk BASIC banner before LOADM"
    );

    type_str(&mut m, "LOADM\"TESTML\"");
    tap_char(&mut m, '\r');
    for _ in 0..FIELDS {
        m.run_field();
    }

    let loaded: Vec<u8> = (0..DATA_LEN as u16)
        .map(|i| m.bus.read(LOAD_ADDR + i))
        .collect();
    if let Some(i) = loaded.iter().position(|&b| b != FILL) {
        panic!(
            "byte at {:#06X} loaded as {:#04X}, expected {:#04X} — a sector's final byte was dropped",
            LOAD_ADDR + i as u16,
            loaded[i],
            FILL
        );
    }
}

/// End-to-end: boot Disk BASIC with a completely blank (0-byte) image
/// mounted, run `DSKINI0` to format it in-emulator (exercising the WD1773's
/// new Write Track MFM parser through the real ROM, not a synthetic stream),
/// then confirm the image came out as a full 35-track RS-DOS disk filled with
/// DSKINI's $FF fill byte, and that a subsequent `DIR` doesn't choke on it.
#[test]
fn dskini_formats_a_blank_disk_and_dir_reports_no_io_error() {
    /// DSKINI formats all 35 tracks (18 sectors each); this needs far more
    /// DRQ-paced field budget than the ~400-field boot/DIR tests above.
    const FORMAT_FIELDS: usize = 4000;
    const BOOT_FIELDS: usize = 400;
    const DIR_FIELDS: usize = 400;

    let (Some(coco), Some(disk_rom)) = (try_load_rom("coco3.rom"), try_load_rom("disk11.rom"))
    else {
        eprintln!("skipping dskini_formats_a_blank_disk_and_dir_reports_no_io_error: roms/ assets not present");
        return;
    };

    let mut m = Machine::new(MachineConfig::default(), coco);
    let mut cart = DiskCart::new(disk_rom);
    cart.insert_disk(0, JvcDisk::from_bytes(Vec::new()).unwrap()); // blank, 0 tracks
    m.insert_cartridge(Box::new(cart));
    m.reset();
    for _ in 0..BOOT_FIELDS {
        m.run_field();
    }
    assert!(
        (0..16).any(|r| screen_row(&mut m, r).contains("DISK EXTENDED COLOR BASIC")),
        "expected the Disk BASIC banner before DSKINI"
    );

    type_str(&mut m, "DSKINI0");
    tap_char(&mut m, '\r');
    for _ in 0..FORMAT_FIELDS {
        m.run_field();
    }

    const EXPECTED_LEN: usize = 35 * 18 * SECTOR_SIZE;
    {
        let cart = m.bus.cart.as_disk_cart().expect("disk controller still inserted");
        let disk = cart.disk(0).expect("drive 0 still mounted");
        assert_eq!(disk.bytes().len(), EXPECTED_LEN, "formatted image must be a full 35-track disk");

        // Sample track 5 sector 1's data region: DSKINI fills every sector
        // with $FF.
        let off = disk.sector_offset(5, 0, 1).expect("track 5 sector 1 must exist after formatting");
        let sample = disk.read_bytes(off, SECTOR_SIZE);
        assert!(sample.iter().all(|&b| b == 0xFF), "DSKINI must fill every sector with $FF");
    }

    type_str(&mut m, "DIR");
    tap_char(&mut m, '\r');
    for _ in 0..DIR_FIELDS {
        m.run_field();
    }
    let screen: Vec<String> = (0..16).map(|r| screen_row(&mut m, r)).collect();
    assert!(
        !screen.iter().any(|row| row.contains("?IO ERROR")),
        "DIR must not report an IO error after DSKINI; screen:\n{}",
        screen.join("\n")
    );
}

/// Like [`try_load_rom`], but for a disk image under the git-ignored `disks/`.
fn try_load_disk(name: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../disks")
        .join(name);
    std::fs::read(&path).ok()
}

/// End-to-end regression that NitrOS-9 Level 2 boots all the way to its shell
/// prompt on the real 40-track two-sided image, booted via Disk BASIC's `DOS`
/// command. Two FD-502 bugs each stalled this boot:
///
/// - Read Sector first-byte latency: `boot_1773` arms its HALT/NMI collection
///   loop only after a short post-command `Delay2`; a first byte paced at one
///   DRQ interval landed inside that delay, tripped LOST DATA, and printed
///   FAILED after the "NITROS9 BOOT" banner.
/// - Read Sector side-at-dispatch: the RBF driver flips DSKREG to the next side
///   *after* writing the Read Sector command, so sampling the side at dispatch
///   read the wrong physical side across every side boundary and corrupted
///   `rb1773`'s body — the console (`/Term`) attach then wedged the boot at the
///   "NITROS9 BOOT" banner forever (see
///   `read_sector_samples_side_when_the_data_field_streams_not_at_dispatch`).
///
/// Reaching the shell's `Time ?` startup prompt proves the whole boot module
/// set loaded and linked and the console attach completed. Skips when the
/// git-ignored ROM or disk assets aren't present.
#[test]
fn nitros9_l2_boot_reaches_shell_prompt() {
    const BOOT_FIELDS: usize = 300;
    const SETTLE_FIELDS: usize = 1500;
    const DISK: &str = "NOS9_6809_L2_v030300_coco3_40d_1.dsk";

    let (Some(coco), Some(disk_rom), Some(dsk)) = (
        try_load_rom("coco3.rom"),
        try_load_rom("disk11.rom"),
        try_load_disk(DISK),
    ) else {
        eprintln!("skipping nitros9_l2_boot_reaches_shell_prompt: roms/ or disks/ assets not present");
        return;
    };

    let mut m = Machine::new(MachineConfig::default(), coco);
    let mut cart = DiskCart::new(disk_rom);
    cart.insert_disk(0, JvcDisk::from_bytes(dsk).unwrap());
    m.insert_cartridge(Box::new(cart));
    m.reset();
    for _ in 0..BOOT_FIELDS {
        m.run_field();
    }
    type_str(&mut m, "DOS");
    tap_char(&mut m, '\r');
    for _ in 0..SETTLE_FIELDS {
        m.run_field();
    }

    // OS-9 switches to the GIME hi-res text screen; text_screen_lines() reads it.
    let screen = m.text_screen_lines().join("\n");
    assert!(
        !screen.contains("FAILED"),
        "NitrOS-9 boot must not report FAILED; screen:\n{screen}"
    );
    assert!(
        screen.contains("NitrOS-9") && screen.contains("Level 2"),
        "expected the NitrOS-9 Level 2 banner (console attach completed); screen:\n{screen}"
    );
    assert!(
        screen.contains("Time ?"),
        "expected the shell startup script's 'Time ?' prompt (full boot to shell); screen:\n{screen}"
    );
}
