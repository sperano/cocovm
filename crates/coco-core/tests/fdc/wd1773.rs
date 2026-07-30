//! WD1773 command state machine

use coco_core::fdc::JVCDisk;
use coco_core::wd1773::{status, WD1773};

use super::common::{index_pattern_disk, CRC_TRAILER, DRQ_INTERVAL, FIRST_BYTE_LATENCY, ONE_TRACK_BYTES, SETTLE};

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
    let mut disk = JVCDisk::from_bytes(vec![0u8; 10 * ONE_TRACK_BYTES]).unwrap(); // 10 tracks
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
    let mut disk = JVCDisk::from_bytes(vec![0u8; ONE_TRACK_BYTES]).unwrap(); // 1 track only
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
    let mut disk = JVCDisk::from_bytes(header).unwrap();
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
    let mut disk = JVCDisk::from_bytes(vec![0u8; ONE_TRACK_BYTES]).unwrap();
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
    let mut disk = JVCDisk::from_bytes(vec![0xAAu8; ONE_TRACK_BYTES]).unwrap();
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
    let mut disk = JVCDisk::from_bytes(vec![0u8; ONE_TRACK_BYTES]).unwrap(); // spt=18
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
    let mut disk = JVCDisk::from_bytes(vec![0u8; ONE_TRACK_BYTES]).unwrap();
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
    let mut disk = JVCDisk::from_bytes(bytes).unwrap();
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
