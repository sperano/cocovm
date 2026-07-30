//! `ROMPak` cartridge coverage: the MAME-compatible mirror-fill, and end-to-end
//! integration against the real `roms/coco3.rom` / `roms/disk11.rom` for the
//! two ways a pak reaches the CPU — the autostart FIRQ boot path (fact 1/2)
//! and BASIC's cold-start `DK` disk-controller probe (`docs/cartridges.md`).

use std::path::PathBuf;

use coco_core::cart::{Cartridge, ROM_PAK_MAX_LEN, ROMPak, ROMPakError};
use coco_core::{Machine, MachineConfig};
use mc6809::Bus;

// ---- ROMPak::from_bytes: rejection and mirror-fill ----------------------------

#[test]
fn rejects_empty_image() {
    assert_eq!(
        ROMPak::from_bytes(&[], false).unwrap_err(),
        ROMPakError::Empty
    );
}

#[test]
fn rejects_oversized_image() {
    let bytes = vec![0u8; ROM_PAK_MAX_LEN + 1];
    assert_eq!(
        ROMPak::from_bytes(&bytes, false).unwrap_err(),
        ROMPakError::TooLarge {
            len: ROM_PAK_MAX_LEN + 1
        }
    );
}

#[test]
fn full_32k_image_maps_cts_half_first() {
    // Pak dumps are CTS-window-first: file offset 0 is the byte at $C000, and
    // the file's second half lands at $8000-$BFFF (GIME bank swap — MAME
    // gime.cpp `((bank & 3) ^ 2) * 0x2000`). Arkanoid's entry code at file
    // offset 0 must be fetched by BASIC's JMP $C000.
    let bytes: Vec<u8> = (0..ROM_PAK_MAX_LEN).map(|i| i as u8).collect();
    let mut pak = ROMPak::from_bytes(&bytes, false).unwrap();
    assert_eq!(pak.rom_read(0xC000), bytes[0x0000]);
    assert_eq!(pak.rom_read(0xE000), bytes[0x2000]);
    assert_eq!(pak.rom_read(0x8000), bytes[0x4000]);
    assert_eq!(pak.rom_read(0xA000), bytes[0x6000]);
    assert_eq!(pak.rom_read(0xFDFF), bytes[0x3DFF]);
}

#[test]
fn undersized_image_mirrors_at_the_c000_and_e000_windows() {
    const LEN: usize = 8 * 1024;
    let bytes: Vec<u8> = (0..LEN).map(|i| i as u8).collect();
    let mut pak = ROMPak::from_bytes(&bytes, false).unwrap();
    // An 8K image mirror-fills to 4 copies across the 32K buffer, so $8000,
    // $A000, $C000, and $E000 (each LEN bytes apart) must all read identically.
    for off in 0..LEN as u16 {
        let want = bytes[off as usize];
        assert_eq!(
            pak.rom_read(0x8000 + off),
            want,
            "offset {off:#06x} at $8000"
        );
        assert_eq!(
            pak.rom_read(0xC000 + off),
            want,
            "offset {off:#06x} at $C000"
        );
        assert_eq!(
            pak.rom_read(0xE000 + off),
            want,
            "offset {off:#06x} at $E000"
        );
    }
}

#[test]
fn mirror_fill_equals_plain_repetition_for_any_size() {
    // A deliberately non-power-of-two size. MAME's doubling loop always lands
    // each copy at a multiple of the image length and copies a prefix of an
    // already-periodic buffer, so its output is byte-identical to plain
    // repetition (`image[i % len]`) for every image size — assert exactly
    // that, byte for byte, across the whole 32K buffer (through the 16K
    // half-swap `rom_read` applies on top of the filled image).
    const LEN: usize = 5000;
    let bytes: Vec<u8> = (0..LEN).map(|i| (i % 256) as u8).collect();
    let mut pak = ROMPak::from_bytes(&bytes, false).unwrap();
    for i in 0..ROM_PAK_MAX_LEN {
        assert_eq!(
            pak.rom_read(0x8000 + i as u16),
            bytes[(i ^ 0x4000) % LEN],
            "mismatch at offset {i:#06x}"
        );
    }
}

// ---- Real-ROM integration ------------------------------------------------------

fn load_rom(name: &str) -> Box<[u8]> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../roms")
        .join(name);
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .into_boxed_slice()
}

fn boot_machine() -> Machine {
    Machine::new(MachineConfig::default(), load_rom("coco3.rom"))
}

/// Decode a text-screen row to ASCII (same VDG alphanumeric decode as
/// `tests/alive.rs`/`tests/keyboard.rs`).
fn screen_row(m: &mut Machine, row: u16) -> String {
    (0..32)
        .map(|c| {
            let code = m.bus.read(0x0400 + row * 32 + c) & 0x3F;
            if code < 0x20 {
                (b'@' + code) as char
            } else {
                (b' ' + (code - 0x20)) as char
            }
        })
        .collect()
}

/// RAM address the test pak's cart code writes a marker byte to, and the byte
/// it writes.
const MARKER_ADDR: u16 = 0x0400;
const MARKER_BYTE: u8 = 0xA5;
/// Offset of the external ROM base ($C000) within a pak's 32K image: pak
/// dumps are CTS-window-first, so $C000 is file offset 0.
const CART_ENTRY_OFFSET: usize = 0x0000;

/// A pak whose code at $C000 writes [`MARKER_BYTE`] to [`MARKER_ADDR`] and
/// loops forever: `LDA #$A5 ; STA $0400 ; BRA *` (bytes verified by hand:
/// `86 A5 B7 04 00 20 FE`).
fn marker_pak(autostart: bool) -> ROMPak {
    let mut image = vec![0u8; ROM_PAK_MAX_LEN];
    let program = [0x86, 0xA5, 0xB7, 0x04, 0x00, 0x20, 0xFE];
    image[CART_ENTRY_OFFSET..CART_ENTRY_OFFSET + program.len()].copy_from_slice(&program);
    ROMPak::from_bytes(&image, autostart).unwrap()
}

#[test]
fn autostart_pak_runs_its_cart_code_via_the_firq_boot_path() {
    // Bounded so a regression (CB1 never pulses, FIRQ never fires) fails the
    // test instead of hanging.
    const MAX_FIELDS: usize = 400;
    let mut m = boot_machine();
    m.insert_cartridge(marker_pak(true));
    m.reset();

    let mut fired = false;
    for _ in 0..MAX_FIELDS {
        m.run_field();
        if m.bus.read(MARKER_ADDR) == MARKER_BYTE {
            fired = true;
            break;
        }
    }
    assert!(
        fired,
        "cart code at $C000 never ran: CART*->PIA1 CB1->FIRQ->$8C28->$C000 path didn't fire"
    );
}

#[test]
fn non_autostart_pak_boots_to_normal_basic_and_never_runs_cart_code() {
    const FIELDS: usize = 400;
    let mut m = boot_machine();
    m.insert_cartridge(marker_pak(false));
    m.reset();
    for _ in 0..FIELDS {
        m.run_field();
    }
    assert_ne!(
        m.bus.read(MARKER_ADDR),
        MARKER_BYTE,
        "cart code ran despite autostart=false: the CART line must stay idle"
    );
    let ok_prompt = (0..16).any(|r| screen_row(&mut m, r).trim_start().starts_with("OK"));
    assert!(ok_prompt, "expected BASIC to boot to its normal OK prompt");
}

#[test]
fn disk_basic_pak_integrates_at_cold_start() {
    // Non-autostart: Disk BASIC ROM Paks don't tie CART* to Q (fact 6). BASIC's
    // cold start finds it instead via the "DK" signature probe at $C000/$C001
    // (`docs/cartridges.md`).
    const FIELDS: usize = 400;
    let mut m = boot_machine();
    m.insert_cartridge(
        ROMPak::from_bytes(&load_rom("disk11.rom"), false).unwrap(),
    );
    m.reset();
    for _ in 0..FIELDS {
        m.run_field();
    }
    let banner = (0..16).any(|r| screen_row(&mut m, r).contains("DISK EXTENDED COLOR BASIC"));
    assert!(
        banner,
        "expected \"DISK EXTENDED COLOR BASIC\" after inserting disk11.rom; row0 = {:?}",
        screen_row(&mut m, 0)
    );
}
