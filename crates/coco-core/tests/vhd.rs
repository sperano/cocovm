//! VHD (virtual hard disk, NitrOS-9 `emudsk`) register-window coverage:
//! status lifecycle, sector read/write semantics (short/EOF reads,
//! zero-extend-on-write), drive-select independence and deselection,
//! MMU-translated bus transfers, and the command-write reentrancy guard.
//!
//! No CPU stepping or ROM booting is needed for any of this — registers are
//! driven directly through `mc6809::Bus::read`/`write` on a constructed
//! `Machine`, matching `tests/fdc.rs`'s controller-level test style.

use coco_core::config::BLOCK_SIZE;
use coco_core::gime::DISABLED_MMU_BASE;
use coco_core::vhd::{command, status, VHDImage, SECTOR_SIZE};
use coco_core::{Machine, MachineConfig};
use mc6809::Bus;

// ---- register addresses ($FF80-$FF86; see DESIGN.md and crate::bus) -------

const LRN_HI: u16 = 0xFF80;
const LRN_MID: u16 = 0xFF81;
const LRN_LO: u16 = 0xFF82;
const COMMAND_STATUS: u16 = 0xFF83;
const BUFFER_HI: u16 = 0xFF84;
const BUFFER_LO: u16 = 0xFF85;
const SELECT: u16 = 0xFF86;

/// Value that deselects both drives: anything other than 0 or 1.
const DESELECT: u8 = 5;
/// Open-bus readback for a deselected/unanswered register.
const OPEN_BUS: u8 = 0xFF;

fn load_rom(name: &str) -> Box<[u8]> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display())).into_boxed_slice()
}

fn boot_machine() -> Machine {
    Machine::new(MachineConfig::default(), load_rom("coco3.rom"))
}

/// Select `drive` (0 or 1) via `$FF86`.
fn select(m: &mut Machine, drive: u8) {
    m.bus.write(SELECT, drive);
}

/// Set the 24-bit LRN via `$FF80-$FF82` (big-endian).
fn set_lrn(m: &mut Machine, lrn: u32) {
    m.bus.write(LRN_HI, ((lrn >> 16) & 0xFF) as u8);
    m.bus.write(LRN_MID, ((lrn >> 8) & 0xFF) as u8);
    m.bus.write(LRN_LO, (lrn & 0xFF) as u8);
}

/// Set the 16-bit CPU buffer address via `$FF84-$FF85` (big-endian).
fn set_buffer(m: &mut Machine, addr: u16) {
    m.bus.write(BUFFER_HI, (addr >> 8) as u8);
    m.bus.write(BUFFER_LO, (addr & 0xFF) as u8);
}

/// Read back `len` bytes from the CPU's logical address space starting at
/// `addr`, through the MMU exactly like the VHD transfer itself.
fn read_ram(m: &mut Machine, addr: u16, len: usize) -> Vec<u8> {
    (0..len as u16).map(|i| m.bus.read(addr.wrapping_add(i))).collect()
}

// ============================================================================
// Status lifecycle
// ============================================================================

#[test]
fn unmounted_drive_reports_no_vhd() {
    let mut m = boot_machine();
    select(&mut m, 0);
    assert_eq!(m.bus.read(COMMAND_STATUS), status::NO_VHD);
}

#[test]
fn insert_sets_power_on_status_before_any_command() {
    let mut m = boot_machine();
    m.bus.vhd.insert(0, VHDImage::Memory(vec![0u8; SECTOR_SIZE]));
    select(&mut m, 0);
    assert_eq!(m.bus.read(COMMAND_STATUS), status::POWER_ON);
}

// ============================================================================
// Full read
// ============================================================================

#[test]
fn full_read_transfers_sector_to_buffer() {
    let mut m = boot_machine();
    let pattern: Vec<u8> = (0..256).map(|i| i as u8).collect();
    let mut image = vec![0u8; 6 * SECTOR_SIZE];
    image[5 * SECTOR_SIZE..6 * SECTOR_SIZE].copy_from_slice(&pattern);
    m.bus.vhd.insert(0, VHDImage::Memory(image));

    select(&mut m, 0);
    set_lrn(&mut m, 5);
    let buffer = 0x2000;
    set_buffer(&mut m, buffer);
    m.bus.write(COMMAND_STATUS, command::READ);

    assert_eq!(m.bus.read(COMMAND_STATUS), status::OK);
    assert_eq!(read_ram(&mut m, buffer, SECTOR_SIZE), pattern);
}

#[test]
fn full_read_supports_lrn_beyond_16_bits() {
    let mut m = boot_machine();
    let lrn: u32 = 0x01_2345;
    let pattern: Vec<u8> = (0..256).map(|i| (i as u8).wrapping_mul(3)).collect();
    let offset = lrn as usize * SECTOR_SIZE;
    let mut image = vec![0u8; offset + SECTOR_SIZE];
    image[offset..offset + SECTOR_SIZE].copy_from_slice(&pattern);
    m.bus.vhd.insert(0, VHDImage::Memory(image));

    select(&mut m, 0);
    set_lrn(&mut m, lrn);
    let buffer = 0x2000;
    set_buffer(&mut m, buffer);
    m.bus.write(COMMAND_STATUS, command::READ);

    assert_eq!(m.bus.read(COMMAND_STATUS), status::OK);
    assert_eq!(read_ram(&mut m, buffer, SECTOR_SIZE), pattern);
}

#[test]
fn read_past_eof_entirely_zero_fills() {
    let mut m = boot_machine();
    // Only 2 sectors in the image; LRN 10 is well past the end.
    m.bus.vhd.insert(0, VHDImage::Memory(vec![0xAAu8; 2 * SECTOR_SIZE]));

    select(&mut m, 0);
    set_lrn(&mut m, 10);
    let buffer = 0x2000;
    set_buffer(&mut m, buffer);
    // Pre-fill the buffer with a marker so the zero-fill is actually observed.
    for i in 0..SECTOR_SIZE as u16 {
        m.bus.write(buffer.wrapping_add(i), 0xFF);
    }
    m.bus.write(COMMAND_STATUS, command::READ);

    assert_eq!(m.bus.read(COMMAND_STATUS), status::OK);
    assert_eq!(read_ram(&mut m, buffer, SECTOR_SIZE), vec![0u8; SECTOR_SIZE]);
}

#[test]
fn short_tail_read_zero_pads_remainder() {
    let mut m = boot_machine();
    const TAIL_LEN: usize = 100;
    let mut image = vec![0u8; 5 * SECTOR_SIZE + TAIL_LEN];
    // Nonzero marker in the tail so it's distinguishable from the zero-pad.
    for (i, b) in image[5 * SECTOR_SIZE..].iter_mut().enumerate() {
        *b = (i as u8).wrapping_add(1);
    }
    m.bus.vhd.insert(0, VHDImage::Memory(image));

    select(&mut m, 0);
    set_lrn(&mut m, 5); // the last, partial sector
    let buffer = 0x2000;
    set_buffer(&mut m, buffer);
    m.bus.write(COMMAND_STATUS, command::READ);

    assert_eq!(m.bus.read(COMMAND_STATUS), status::OK);
    let got = read_ram(&mut m, buffer, SECTOR_SIZE);
    for (i, &byte) in got.iter().enumerate().take(TAIL_LEN) {
        assert_eq!(byte, (i as u8).wrapping_add(1), "tail byte {i}");
    }
    for (i, &byte) in got.iter().enumerate().skip(TAIL_LEN) {
        assert_eq!(byte, 0, "zero-pad byte {i}");
    }
}

// ============================================================================
// Full write
// ============================================================================

#[test]
fn write_past_eof_zero_extends_then_writes() {
    let mut m = boot_machine();
    // 2 sectors of 0xAA; LRN 4 is 2 sectors past the current end.
    m.bus.vhd.insert(0, VHDImage::Memory(vec![0xAAu8; 2 * SECTOR_SIZE]));

    select(&mut m, 0);
    set_lrn(&mut m, 4);
    let buffer: u16 = 0x3000;
    let pattern: Vec<u8> = (0..256).map(|i| (i as u8) ^ 0x5A).collect();
    for (i, b) in pattern.iter().enumerate() {
        m.bus.write(buffer.wrapping_add(i as u16), *b);
    }
    set_buffer(&mut m, buffer);
    m.bus.write(COMMAND_STATUS, command::WRITE);

    assert_eq!(m.bus.read(COMMAND_STATUS), status::OK);
    let image = m.bus.vhd.image(0).unwrap().as_memory().unwrap();
    assert_eq!(image.len(), 5 * SECTOR_SIZE, "image extended exactly to LRN 4's sector end");
    assert!(
        image[2 * SECTOR_SIZE..4 * SECTOR_SIZE].iter().all(|&b| b == 0),
        "the zero-extended gap (sectors 2-3) must be zero, not left as old/garbage bytes"
    );
    assert_eq!(&image[4 * SECTOR_SIZE..5 * SECTOR_SIZE], pattern.as_slice());
}

// ============================================================================
// Command dispatch
// ============================================================================

#[test]
fn unknown_command_reports_unknown_command_status() {
    let mut m = boot_machine();
    m.bus.vhd.insert(0, VHDImage::Memory(vec![0u8; SECTOR_SIZE]));
    select(&mut m, 0);
    const UNKNOWN: u8 = 3;
    m.bus.write(COMMAND_STATUS, UNKNOWN);
    assert_eq!(m.bus.read(COMMAND_STATUS), status::UNKNOWN_COMMAND);
}

#[test]
fn flush_command_reports_ok() {
    let mut m = boot_machine();
    m.bus.vhd.insert(0, VHDImage::Memory(vec![0u8; SECTOR_SIZE]));
    select(&mut m, 0);
    m.bus.write(COMMAND_STATUS, command::FLUSH);
    assert_eq!(m.bus.read(COMMAND_STATUS), status::OK);
}

#[test]
fn read_bumps_the_drive_access_count() {
    let mut m = boot_machine();
    m.bus.vhd.insert(0, VHDImage::Memory(vec![0u8; SECTOR_SIZE]));
    select(&mut m, 0);
    assert_eq!(m.bus.vhd.access_count(0), 0);
    m.bus.write(COMMAND_STATUS, command::READ);
    assert_eq!(m.bus.vhd.access_count(0), 1);
    m.bus.write(COMMAND_STATUS, command::READ);
    assert_eq!(m.bus.vhd.access_count(0), 2, "each dispatched command bumps it again");
}

#[test]
fn unknown_command_does_not_bump_the_access_count() {
    let mut m = boot_machine();
    m.bus.vhd.insert(0, VHDImage::Memory(vec![0u8; SECTOR_SIZE]));
    select(&mut m, 0);
    const UNKNOWN: u8 = 3;
    m.bus.write(COMMAND_STATUS, UNKNOWN);
    assert_eq!(m.bus.vhd.access_count(0), 0);
}

#[test]
fn unmounted_drive_reports_no_vhd_regardless_of_command() {
    // Spec: the unmounted check happens before dispatch, uniformly for every
    // command value.
    let mut m = boot_machine();
    select(&mut m, 0);
    for cmd in [command::READ, command::WRITE, command::FLUSH, 3] {
        m.bus.write(COMMAND_STATUS, cmd);
        assert_eq!(m.bus.read(COMMAND_STATUS), status::NO_VHD, "command {cmd}");
    }
}

// ============================================================================
// Drive-select independence and deselection
// ============================================================================

#[test]
fn per_drive_state_is_independent() {
    let mut m = boot_machine();
    let pattern0 = vec![0x11u8; SECTOR_SIZE];
    let pattern1 = vec![0x22u8; SECTOR_SIZE];
    let mut image0 = vec![0u8; 3 * SECTOR_SIZE];
    image0[2 * SECTOR_SIZE..3 * SECTOR_SIZE].copy_from_slice(&pattern0);
    let mut image1 = vec![0u8; 3 * SECTOR_SIZE];
    image1[SECTOR_SIZE..2 * SECTOR_SIZE].copy_from_slice(&pattern1);
    m.bus.vhd.insert(0, VHDImage::Memory(image0));
    m.bus.vhd.insert(1, VHDImage::Memory(image1));

    select(&mut m, 0);
    set_lrn(&mut m, 2); // drive 0's LRN

    select(&mut m, 1);
    set_lrn(&mut m, 1); // drive 1's LRN -- must not disturb drive 0's

    select(&mut m, 0); // back to drive 0
    let buffer = 0x2000;
    set_buffer(&mut m, buffer);
    m.bus.write(COMMAND_STATUS, command::READ);

    assert_eq!(m.bus.read(COMMAND_STATUS), status::OK);
    // If drive 1's LRN had clobbered drive 0's, this would read pattern1 (or
    // zero, from image0's short length) instead of pattern0.
    assert_eq!(read_ram(&mut m, buffer, SECTOR_SIZE), pattern0);
}

#[test]
fn deselected_state_reads_open_bus_and_drops_writes() {
    let mut m = boot_machine();
    let mut image = vec![0u8; 8 * SECTOR_SIZE];
    image[7 * SECTOR_SIZE..8 * SECTOR_SIZE].fill(0x77);
    m.bus.vhd.insert(0, VHDImage::Memory(image));

    select(&mut m, 0);
    set_lrn(&mut m, 7);

    m.bus.write(SELECT, DESELECT);
    for addr in LRN_HI..=BUFFER_LO {
        assert_eq!(m.bus.read(addr), OPEN_BUS, "addr {addr:#06X} while deselected");
    }

    // Attempts while deselected are silently dropped -- neither of these may
    // take effect.
    set_lrn(&mut m, 99); // would clobber the LRN if not dropped
    m.bus.write(COMMAND_STATUS, command::READ); // would run a command if not dropped

    select(&mut m, 0); // reselect
    assert_eq!(
        m.bus.read(COMMAND_STATUS),
        status::POWER_ON,
        "status must be untouched by the dropped command write"
    );

    let buffer = 0x2000;
    set_buffer(&mut m, buffer);
    m.bus.write(COMMAND_STATUS, command::READ);
    assert_eq!(m.bus.read(COMMAND_STATUS), status::OK);
    assert_eq!(
        read_ram(&mut m, buffer, SECTOR_SIZE),
        vec![0x77u8; SECTOR_SIZE],
        "LRN must still be 7, not clobbered to 99 by the dropped write"
    );
}

// ============================================================================
// MMU translation
// ============================================================================

#[test]
fn transfer_honors_mmu_translation() {
    let mut m = boot_machine();
    let pattern: Vec<u8> = (0..256).map(|i| i as u8).collect();
    m.bus.vhd.insert(0, VHDImage::Memory(pattern.clone()));

    select(&mut m, 0);
    set_lrn(&mut m, 0);

    let buffer: u16 = 0x2000;
    let slot = buffer as usize / BLOCK_SIZE;
    const BLOCK: u8 = 5; // arbitrary physical block, distinct from the identity map
    m.bus.gime.mmu_enabled = true;
    m.bus.gime.task = 0;
    m.bus.gime.mmu[0][slot] = BLOCK;

    set_buffer(&mut m, buffer);
    m.bus.write(COMMAND_STATUS, command::READ);
    assert_eq!(m.bus.read(COMMAND_STATUS), status::OK);

    let mmu_phys = BLOCK as usize * BLOCK_SIZE + (buffer as usize % BLOCK_SIZE);
    let identity_phys = DISABLED_MMU_BASE | buffer as usize;
    assert_ne!(mmu_phys, identity_phys, "test setup must pick translations that actually differ");

    assert_eq!(&m.bus.ram[mmu_phys..mmu_phys + SECTOR_SIZE], pattern.as_slice());
    assert_eq!(
        &m.bus.ram[identity_phys..identity_phys + SECTOR_SIZE],
        vec![0u8; SECTOR_SIZE].as_slice(),
        "the naive identity-mapped location must NOT have received the transfer"
    );
}

// ============================================================================
// Reentrancy guard
// ============================================================================

#[test]
fn reentrant_command_write_is_dropped_and_outer_result_stands() {
    let mut m = boot_machine();
    let pattern: Vec<u8> = (0..256).map(|i| i as u8).collect();
    m.bus.vhd.insert(0, VHDImage::Memory(pattern));

    select(&mut m, 0);
    set_lrn(&mut m, 0);
    // Buffer address lands the transfer's very first byte write on $FF83
    // itself -- the loop's own Bus::write recurses into the command-register
    // decoder mid-command. Later loop iterations also land on $FF84-$FF86
    // (live register writes, not guarded) and beyond, including $FF86 the
    // drive-select latch -- deselecting the drive as an incidental side
    // effect of which pattern byte lands there. That's expected (per spec);
    // reselect afterward to check the outer command's real result.
    set_buffer(&mut m, COMMAND_STATUS);

    m.bus.write(COMMAND_STATUS, command::READ); // must not hang or panic

    select(&mut m, 0);
    assert_eq!(
        m.bus.read(COMMAND_STATUS),
        status::OK,
        "outer read command's real outcome must stand, not a dropped reentrant attempt's"
    );
}
