//! JVC image geometry (jvc_dsk.cpp) and OS-9 LSN0 geometry sniffing for
//! headerless images (os9_dsk.cpp find_size).

use std::path::PathBuf;

use coco_core::fdc::{JVCDisk, JVCError};

use super::common::ONE_TRACK_BYTES;

#[test]
fn headerless_image_uses_all_defaults() {
    // 35 tracks x 18 spt x 1 side x 256B, no header (file_len is an exact
    // multiple of 256).
    let bytes = vec![0u8; 35 * ONE_TRACK_BYTES];
    let disk = JVCDisk::from_bytes(bytes).unwrap();
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
    let disk = JVCDisk::from_bytes(bytes).unwrap();
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
    let disk = JVCDisk::from_bytes(bytes).unwrap();
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
    let disk = JVCDisk::from_bytes(bytes).unwrap();
    let cases = [
        (0u8, 0u8, 0u8), // track0 side0 -> marker 0
        (0, 1, 1),       // track0 side1 -> marker 1
        (1, 0, 2),       // track1 side0 -> marker 2
        (1, 1, 3),       // track1 side1 -> marker 3
    ];
    for (track, side, marker) in cases {
        let off = disk.sector_offset(track, side, 1).unwrap();
        assert_eq!(
            disk.read_bytes(off, 1)[0],
            marker,
            "track{track} side{side}"
        );
    }
}

#[test]
fn sector_offset_rejects_out_of_range_track_side_and_sector() {
    let disk = JVCDisk::from_bytes(vec![0u8; ONE_TRACK_BYTES]).unwrap(); // 1 track
    assert_eq!(
        disk.sector_offset(1, 0, 1),
        None,
        "track beyond track_count"
    );
    assert_eq!(
        disk.sector_offset(0, 1, 1),
        None,
        "side beyond sides (single-sided)"
    );
    assert_eq!(
        disk.sector_offset(0, 0, 0),
        None,
        "sector below first_sector_id"
    );
    assert_eq!(
        disk.sector_offset(0, 0, 19),
        None,
        "sector beyond sectors_per_track"
    );
    assert!(disk.sector_offset(0, 0, 1).is_some());
    assert!(disk.sector_offset(0, 0, 18).is_some());
}

#[test]
fn invalid_geometry_is_rejected() {
    // Headerless (multiple of 256) but not a whole number of default-geometry
    // tracks (4608 bytes/track).
    let bytes = vec![0u8; 5120];
    assert_eq!(bytes.len() % 256, 0);
    let err = JVCDisk::from_bytes(bytes).unwrap_err();
    assert!(matches!(err, JVCError::InvalidGeometry { .. }));
}

#[test]
fn nonzero_attribute_flag_is_rejected() {
    // 5-byte header (spt=18, sides=1, size code=1, first id=1, attr flag=1),
    // one otherwise-valid track of data.
    let mut bytes = vec![18u8, 1, 1, 1, 1];
    bytes.extend(vec![0u8; ONE_TRACK_BYTES]);
    assert_eq!(
        JVCDisk::from_bytes(bytes).unwrap_err(),
        JVCError::AttributeBytesUnsupported
    );
}

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

    let disk = JVCDisk::from_bytes(bytes).unwrap();
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
    let disk = JVCDisk::from_bytes(bytes).unwrap();
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
    let disk = JVCDisk::from_bytes(bytes).unwrap();
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
        eprintln!(
            "skipping real_nitros9_40_track_disk_parses_as_40_tracks_2_sides: {} not present",
            path.display()
        );
        return;
    };
    let disk = JVCDisk::from_bytes(bytes).unwrap();
    assert_eq!(disk.track_count(), 40);
    assert_eq!(disk.sides(), 2);
}
