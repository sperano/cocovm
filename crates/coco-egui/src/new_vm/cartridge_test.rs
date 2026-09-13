use std::fs;
use std::path::{Path, PathBuf};

use crate::machine_def::tests::TempDir;

use super::*;

const STANDARD_CARTRIDGE_IMAGE_SIZE: usize = 0x2000;
const MIND_ROLL_IMAGE_SIZE: usize = 0x8000;
const COLOR_BASEBALL_CRC_SUFFIX: [u8; 4] = [0x11, 0x35, 0xa2, 0xf7];
const MIND_ROLL_CRC_SUFFIX: [u8; 4] = [0xe0, 0x24, 0x53, 0x1e];
const CYD_GMC_CRC_SUFFIX: [u8; 4] = [0x17, 0x60, 0x50, 0x9b];

fn write_image(dir: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, bytes).expect("write cartridge test image");
    path
}

fn write_crc_image(dir: &TempDir, name: &str, size: usize, suffix: [u8; 4]) -> PathBuf {
    let mut bytes = vec![0; size];
    let suffix_start = bytes.len() - suffix.len();
    bytes[suffix_start..].copy_from_slice(&suffix);
    write_image(dir, name, &bytes)
}

fn assert_banked_rompak(choice: CartridgeChoice, path: &Path) {
    assert_eq!(
        choice,
        CartridgeChoice::BankedROMPak {
            path: path.to_path_buf(),
            autostart: DEFAULT_AUTOSTART,
        }
    );
}

fn assert_rompak(choice: CartridgeChoice, path: &Path) {
    assert_eq!(
        choice,
        CartridgeChoice::ROMPak {
            path: path.to_path_buf(),
            autostart: DEFAULT_AUTOSTART,
        }
    );
}

fn assert_games_master(choice: CartridgeChoice, path: &Path) {
    assert_eq!(
        choice,
        CartridgeChoice::GamesMaster {
            path: path.to_path_buf(),
            autostart: DEFAULT_AUTOSTART,
        }
    );
}

#[test]
fn known_rompak_overrides_direct_games_master_fallback() {
    let dir = TempDir::new("known-rompak-direct");
    let path = write_crc_image(
        &dir,
        "color-baseball.rom",
        STANDARD_CARTRIDGE_IMAGE_SIZE,
        COLOR_BASEBALL_CRC_SUFFIX,
    );

    assert_rompak(games_master(path.clone()), &path);
}

#[test]
fn cyd_gmc_overrides_direct_rompak_fallback() {
    let dir = TempDir::new("cyd-gmc-direct");
    let path = write_crc_image(
        &dir,
        "cyd_gmc.rom",
        STANDARD_CARTRIDGE_IMAGE_SIZE,
        CYD_GMC_CRC_SUFFIX,
    );

    assert_games_master(rompak(path.clone()), &path);
}

#[test]
fn known_hardware_overrides_mpi_fallbacks() {
    let dir = TempDir::new("known-mpi");
    let rompak_path = write_crc_image(
        &dir,
        "rompak.rom",
        STANDARD_CARTRIDGE_IMAGE_SIZE,
        COLOR_BASEBALL_CRC_SUFFIX,
    );
    let gmc_path = write_crc_image(
        &dir,
        "cyd_gmc.rom",
        STANDARD_CARTRIDGE_IMAGE_SIZE,
        CYD_GMC_CRC_SUFFIX,
    );

    assert_eq!(
        slot_games_master(rompak_path.clone()),
        SlotChoice::ROMPak {
            path: rompak_path,
            autostart: DEFAULT_AUTOSTART,
        }
    );
    assert_eq!(
        slot_rompak(gmc_path.clone()),
        SlotChoice::GamesMaster {
            path: gmc_path,
            autostart: DEFAULT_AUTOSTART,
        }
    );
}

#[test]
fn mind_roll_selects_banked_rompak_without_gmc_sound() {
    let dir = TempDir::new("mind-roll-banked");
    let path = write_crc_image(
        &dir,
        "mind-roll.rom",
        MIND_ROLL_IMAGE_SIZE,
        MIND_ROLL_CRC_SUFFIX,
    );

    assert_banked_rompak(rompak(path.clone()), &path);
    assert!(matches!(
        slot_games_master(path),
        SlotChoice::BankedROMPak { .. }
    ));
}

#[test]
fn unknown_direct_image_preserves_selected_fallback() {
    let dir = TempDir::new("unknown-direct");
    let path = write_image(&dir, "unknown.rom", b"unknown cartridge");

    assert_rompak(rompak(path.clone()), &path);
    assert_games_master(games_master(path.clone()), &path);
}

#[test]
fn unknown_mpi_image_preserves_selected_fallback() {
    let dir = TempDir::new("unknown-mpi");
    let path = write_image(&dir, "unknown.rom", b"unknown cartridge");

    assert!(matches!(
        slot_rompak(path.clone()),
        SlotChoice::ROMPak { .. }
    ));
    assert!(matches!(
        slot_games_master(path),
        SlotChoice::GamesMaster { .. }
    ));
}

#[test]
fn unreadable_images_preserve_direct_and_mpi_fallbacks() {
    let dir = TempDir::new("unreadable");
    let path = dir.path().join("missing.rom");

    assert_rompak(rompak(path.clone()), &path);
    assert_games_master(games_master(path.clone()), &path);
    assert!(matches!(
        slot_rompak(path.clone()),
        SlotChoice::ROMPak { .. }
    ));
    assert!(matches!(
        slot_games_master(path),
        SlotChoice::GamesMaster { .. }
    ));
}
