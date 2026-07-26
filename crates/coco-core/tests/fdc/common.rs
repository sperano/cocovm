use std::path::PathBuf;

use coco_core::cart::Cartridge;
use coco_core::fdc::{DiskCart, JvcDisk};
use coco_core::{Machine, MachineConfig};
use mc6809::Bus;

/// Default geometry, one 18-sector/256-byte/1-side track's worth of bytes.
pub const ONE_TRACK_BYTES: usize = 18 * 256;

pub fn load_rom(name: &str) -> Box<[u8]> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display())).into_boxed_slice()
}

pub fn disk_cart() -> DiskCart {
    DiskCart::new(load_rom("disk11.rom"))
}

/// One track, one sector, every byte the given marker — enough to identify
/// which drive/side answered a Read Sector without caring about pacing.
pub fn marker_disk(marker: u8) -> JvcDisk {
    JvcDisk::from_bytes(vec![marker; ONE_TRACK_BYTES]).unwrap()
}

/// One track, 2 sides, 1 sector/side, 128B — for side-select coverage.
pub fn two_sided_marker_disk(side0: u8, side1: u8) -> JvcDisk {
    let mut bytes = vec![1u8, 2, 0, 1, 0]; // spt=1, sides=2, size 128, first id 1
    bytes.extend(vec![side0; 128]);
    bytes.extend(vec![side1; 128]);
    JvcDisk::from_bytes(bytes).unwrap()
}

/// Comfortably more than the implementation's first-byte search latency (~30
/// byte times: the ID-field-to-data-field span a Read Sector spends before the
/// first byte lands), so one sector read's first byte is ready.
pub const ONE_DRQ_INTERVAL: u32 = 1200;

/// Read sector 1 of track 0 through the currently-selected drive/side and
/// return the byte delivered. Force-Interrupts first so a previous call's
/// still-in-flight command (this only ticks long enough for the first byte,
/// not a whole 256-byte sector) doesn't cause the new command to be silently
/// ignored (spec: a command written while busy is ignored).
pub fn read_marker_byte(cart: &mut DiskCart) -> u8 {
    cart.write(0xFF48, 0xD0); // Force Interrupt, cancel only
    cart.write(0xFF49, 0); // track register
    cart.write(0xFF4A, 1); // sector register
    cart.write(0xFF48, 0x80); // Read Sector, single
    cart.tick(ONE_DRQ_INTERVAL);
    cart.read(0xFF4B)
}

/// A disk whose track 0, sector 1 is filled with `i as u8` for `i` in
/// `0..256` — lets a read-sector test confirm both delivery order and value.
pub fn index_pattern_disk() -> JvcDisk {
    let bytes: Vec<u8> = (0..ONE_TRACK_BYTES).map(|i| i as u8).collect();
    JvcDisk::from_bytes(bytes).unwrap()
}

/// Comfortably longer than the implementation's fixed Type I/RNF settle delay.
pub const SETTLE: u32 = 200;
/// The implementation's DRQ pacing interval, mirrored here so tests can step
/// exactly one byte at a time.
pub const DRQ_INTERVAL: u32 = 30;
/// The implementation's first-byte search latency (30 byte times): a Read/Write
/// Sector command's first DRQ trails the command by the ID-field-to-data-field
/// span, not one DRQ interval.
pub const FIRST_BYTE_LATENCY: u32 = 30 * DRQ_INTERVAL;
/// Mirrors the WD1773's CRC trailer: the gap between a read's final data-byte
/// DRQ and command completion (2 byte times).
pub const CRC_TRAILER: u32 = 2 * DRQ_INTERVAL;

pub fn boot_machine() -> Machine {
    Machine::new(MachineConfig::default(), load_rom("coco3.rom"))
}

pub fn screen_row(m: &mut Machine, row: u16) -> String {
    (0..32)
        .map(|c| {
            let code = m.bus.read(0x0400 + row * 32 + c) & 0x3F;
            if code < 0x20 { (b'@' + code) as char } else { (b' ' + (code - 0x20)) as char }
        })
        .collect()
}

pub fn tap(m: &mut Machine, pos: (u8, u8)) {
    for _ in 0..3 {
        m.bus.keyboard.set(pos, true);
        m.run_field();
    }
    m.bus.keyboard.set(pos, false);
    for _ in 0..3 {
        m.run_field();
    }
}

pub fn tap_char(m: &mut Machine, c: char) {
    let (pos, shift) = coco_core::keyboard::char_key(c).unwrap_or_else(|| panic!("no key for {c:?}"));
    if shift {
        m.bus.keyboard.set(coco_core::keyboard::SHIFT, true);
    }
    tap(m, pos);
    if shift {
        m.bus.keyboard.set(coco_core::keyboard::SHIFT, false);
    }
}

pub fn type_str(m: &mut Machine, s: &str) {
    for c in s.chars() {
        tap_char(m, c);
    }
}

/// Directory track (RS-DOS): track 17, 0-indexed physical track (the 18th
/// track on a 35-track disk).
pub const DIR_TRACK: u8 = 17;
/// GAT (granule allocation table) sector.
pub const GAT_SECTOR: u8 = 2;
/// First directory-entry sector; entries run through sector 11.
pub const DIR_FIRST_SECTOR: u8 = 3;
pub const DIR_LAST_SECTOR: u8 = 11;
pub const SECTOR_SIZE: usize = 256;
pub const DIR_ENTRY_SIZE: usize = 32;

/// Synthesize a headerless 35-track/18-spt/1-side/256B RS-DOS disk with one
/// file, "HELLO.BAS", occupying granule 0 (track 0, both granules — i.e. the
/// first 2 tracks worth of granules; RS-DOS granules are half-tracks, 2 per
/// track, 9 sectors each on a 18-spt disk).
pub fn synthesized_rsdos_disk(filename8: &str, ext3: &str) -> JvcDisk {
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

/// Like [`load_rom`] but returns `None` instead of panicking when the
/// (git-ignored) ROM image isn't present, so the LOADM regression below skips
/// gracefully in an asset-less checkout.
pub fn try_load_rom(name: &str) -> Option<Box<[u8]>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../roms")
        .join(name);
    std::fs::read(&path).ok().map(Vec::into_boxed_slice)
}

/// Synthesize a 35-track RS-DOS disk holding one machine-language file
/// (`name8`.BIN) whose `data` loads at `load_addr`. The file (a single DECB
/// binary load segment plus the exec trailer) occupies granule 0.
pub fn synthesized_ml_disk(name8: &str, load_addr: u16, data: &[u8]) -> JvcDisk {
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

/// Like [`try_load_rom`], but for a disk image under the git-ignored `disks/`.
pub fn try_load_disk(name: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../disks")
        .join(name);
    std::fs::read(&path).ok()
}
