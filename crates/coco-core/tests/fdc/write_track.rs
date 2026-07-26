//! Write Track (format) MFM stream parsing

use coco_core::fdc::JvcDisk;
use coco_core::wd1773::{status, WD1773};

use super::common::{DRQ_INTERVAL, ONE_TRACK_BYTES};

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
