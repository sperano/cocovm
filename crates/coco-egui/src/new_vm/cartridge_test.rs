use std::fs;
use std::path::{Path, PathBuf};

use crate::machine_def::tests::TempDir;

use super::*;

const STANDARD_CARTRIDGE_IMAGE_SIZE: usize = 0x2000;
const MIND_ROLL_IMAGE_SIZE: usize = 0x8000;
const COLOR_BASEBALL_CRC_SUFFIX: [u8; 4] = [0x11, 0x35, 0xa2, 0xf7];
const MIND_ROLL_CRC_SUFFIX: [u8; 4] = [0xe0, 0x24, 0x53, 0x1e];
const CYD_GMC_CRC_SUFFIX: [u8; 4] = [0x17, 0x60, 0x50, 0x9b];

#[test]
fn terminal_hardware_names_are_specific() {
    assert_eq!(hardware_name(CartridgeHardware::RomPak), "ROM Pak");
    assert_eq!(
        hardware_name(CartridgeHardware::BankedRomPak),
        "Banked ROM Pak"
    );
    assert_eq!(
        hardware_name(CartridgeHardware::GamesMaster),
        "Games Master Cartridge"
    );
}

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

fn assert_direct_image(
    choice: CartridgeChoice,
    path: &Path,
    hardware: CartridgeHardware,
    hardware_detected: bool,
) {
    assert_eq!(
        choice,
        CartridgeChoice::Image(CartridgeImageChoice {
            path: path.to_path_buf(),
            autostart: DEFAULT_AUTOSTART,
            hardware,
            hardware_detected,
        })
    );
}

fn assert_slot_image(
    choice: SlotChoice,
    path: &Path,
    hardware: CartridgeHardware,
    hardware_detected: bool,
) {
    assert_eq!(
        choice,
        SlotChoice::Image(CartridgeImageChoice {
            path: path.to_path_buf(),
            autostart: DEFAULT_AUTOSTART,
            hardware,
            hardware_detected,
        })
    );
}

#[test]
fn known_rompak_is_detected_for_direct_and_mpi_use() {
    let dir = TempDir::new("known-rompak");
    let path = write_crc_image(
        &dir,
        "color-baseball.rom",
        STANDARD_CARTRIDGE_IMAGE_SIZE,
        COLOR_BASEBALL_CRC_SUFFIX,
    );

    assert_direct_image(
        cartridge_rom(path.clone()),
        &path,
        CartridgeHardware::RomPak,
        true,
    );
    assert_slot_image(
        slot_cartridge_rom(path.clone()),
        &path,
        CartridgeHardware::RomPak,
        true,
    );
}

#[test]
fn cyd_gmc_is_detected_for_direct_and_mpi_use() {
    let dir = TempDir::new("cyd-gmc");
    let path = write_crc_image(
        &dir,
        "cyd_gmc.ccc",
        STANDARD_CARTRIDGE_IMAGE_SIZE,
        CYD_GMC_CRC_SUFFIX,
    );

    assert_direct_image(
        cartridge_rom(path.clone()),
        &path,
        CartridgeHardware::GamesMaster,
        true,
    );
    assert_slot_image(
        slot_cartridge_rom(path.clone()),
        &path,
        CartridgeHardware::GamesMaster,
        true,
    );
}

#[test]
fn mind_roll_is_detected_as_banked_without_gmc_sound() {
    let dir = TempDir::new("mind-roll-banked");
    let path = write_crc_image(
        &dir,
        "mind-roll.rom",
        MIND_ROLL_IMAGE_SIZE,
        MIND_ROLL_CRC_SUFFIX,
    );

    assert_direct_image(
        cartridge_rom(path.clone()),
        &path,
        CartridgeHardware::BankedRomPak,
        true,
    );
    assert_slot_image(
        slot_cartridge_rom(path.clone()),
        &path,
        CartridgeHardware::BankedRomPak,
        true,
    );
}

#[test]
fn unknown_images_use_an_overridable_rompak_fallback() {
    let dir = TempDir::new("unknown");
    let path = write_image(&dir, "unknown.rom", b"unknown cartridge");

    assert_direct_image(
        cartridge_rom(path.clone()),
        &path,
        CartridgeHardware::RomPak,
        false,
    );
    assert_slot_image(
        slot_cartridge_rom(path.clone()),
        &path,
        CartridgeHardware::RomPak,
        false,
    );
}

#[test]
fn unreadable_images_use_an_overridable_rompak_fallback() {
    let dir = TempDir::new("unreadable");
    let path = dir.path().join("missing.rom");

    assert_direct_image(
        cartridge_rom(path.clone()),
        &path,
        CartridgeHardware::RomPak,
        false,
    );
    assert_slot_image(
        slot_cartridge_rom(path.clone()),
        &path,
        CartridgeHardware::RomPak,
        false,
    );
}

#[test]
fn manual_unknown_hardware_is_persisted() {
    let image = CartridgeImageChoice {
        path: PathBuf::from("unknown.rom"),
        autostart: false,
        hardware: CartridgeHardware::GamesMaster,
        hardware_detected: false,
    };

    assert_eq!(
        cartridge_dto_for_image(&image),
        CartridgeDTO::GamesMaster {
            path: "unknown.rom".to_string(),
            autostart: false,
        }
    );
    assert_eq!(
        slot_dto_for_image(&image),
        SlotDTO::GamesMaster {
            path: "unknown.rom".to_string(),
            autostart: false,
        }
    );
}
