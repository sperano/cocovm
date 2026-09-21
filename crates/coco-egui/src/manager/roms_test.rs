//! `rom_rows`: which images a definition lists, where they resolve, and
//! what status each gets from the roms directory's contents.

use std::path::Path;

use super::*;
use crate::machine_def::tests::TempDir;
use crate::machine_def::{MachineDef, RS232EndpointDTO};
use coco_core::{MachineConfig, MachineVariant, MemorySize, VideoStandard};

const COCO3_DESC: &str = "Super Extended Color BASIC 2.0 (CoCo 3 NTSC)";

fn coco3_def() -> MachineDef {
    MachineDef::from_config("Alpha".to_string(), None, &MachineConfig::default())
}

fn coco2_def() -> MachineDef {
    MachineDef::from_config(
        "Beta".to_string(),
        None,
        &MachineConfig {
            variant: MachineVariant::Coco2,
            video: VideoStandard::NTSC,
            memory: MemorySize::K64,
            monitor: None,
            vdg: None,
        },
    )
}

/// `(role, file name, status)` per row — the shape the assertions compare.
fn summary(rows: &[ROMRow]) -> Vec<(&str, String, &ROMStatus)> {
    rows.iter()
        .map(|row| (row.role.as_str(), file_name(&row.path), &row.status))
        .collect()
}

#[test]
fn coco3_lists_its_system_rom_as_missing_without_a_roms_dir() {
    let rows = rom_rows(&coco3_def(), "alpha", None);
    assert_eq!(
        summary(&rows),
        vec![("System ROM", "coco3.rom".to_string(), &ROMStatus::Missing)]
    );
}

#[test]
fn a_fake_coco3_rom_is_reported_as_differing_from_the_known_dump() {
    let dir = TempDir::new("roms-fake-coco3");
    std::fs::write(dir.path().join("coco3.rom"), [0u8; 16]).unwrap();
    let rows = rom_rows(&coco3_def(), "alpha", Some(dir.path()));
    assert_eq!(rows[0].path, dir.path().join("coco3.rom"));
    assert_eq!(rows[0].status, ROMStatus::Mismatch(COCO3_DESC));
}

#[test]
fn an_explicit_system_rom_is_listed_at_its_own_path() {
    let dir = TempDir::new("roms-explicit");
    let custom = dir.path().join("custom.rom");
    std::fs::write(&custom, [0u8; 32]).unwrap();
    let mut def = coco3_def();
    def.hardware.rom = Some(custom.display().to_string());

    let rows = rom_rows(&def, "alpha", Some(dir.path()));
    assert_eq!(
        summary(&rows),
        vec![(
            "System ROM",
            "custom.rom".to_string(),
            &ROMStatus::Unrecognized(32)
        )]
    );
}

#[test]
fn coco2_lists_color_basic_and_an_optional_extended_basic() {
    let dir = TempDir::new("roms-coco2");
    // Only the older dump is installed: the row must name the file that
    // exists, not the preferred candidate.
    std::fs::write(dir.path().join("bas11.rom"), [0u8; 8]).unwrap();
    let rows = rom_rows(&coco2_def(), "beta", Some(dir.path()));
    assert_eq!(
        summary(&rows),
        vec![
            (
                "Color BASIC",
                "bas11.rom".to_string(),
                &ROMStatus::Mismatch("Color BASIC 1.1 (CoCo 1/2)")
            ),
            (
                "Extended Color BASIC",
                "extbas11.rom".to_string(),
                &ROMStatus::OptionalAbsent
            ),
        ]
    );
}

#[test]
fn mpi_slots_prefix_their_rows_and_skip_romless_occupants() {
    let dir = TempDir::new("roms-mpi");
    let mut def = coco3_def();
    def.peripherals.cartridge = CartridgeDTO::MPI {
        slots: [
            SlotDTO::FD502 {
                dos_rom: Default::default(),
            },
            SlotDTO::RS232 {
                endpoint: RS232EndpointDTO::default(),
            },
            SlotDTO::RTC,
            SlotDTO::SoundSpeech,
        ],
        switch: 1,
    };

    let rows = rom_rows(&def, "alpha", Some(dir.path()));
    let roles: Vec<&str> = rows.iter().map(|row| row.role.as_str()).collect();
    assert_eq!(
        roles,
        [
            "System ROM",
            "Slot 1: Disk BASIC 1.1",
            "Slot 2: RS-232 Pak EPROM",
            "Slot 4: Speech/Sound allophones",
            "Slot 4: Speech/Sound firmware",
        ]
    );
    assert_eq!(rows[1].status, ROMStatus::Missing);
    assert_eq!(rows[2].status, ROMStatus::OptionalAbsent);
    assert_eq!(rows[3].path, dir.path().join(rom_load::SP0256_ROM));
}

#[test]
fn a_rom_pak_in_the_bare_port_resolves_its_own_image() {
    let dir = TempDir::new("roms-rompak");
    let pak = dir.path().join("game.ccc");
    std::fs::write(&pak, [0u8; 64]).unwrap();
    let mut def = coco3_def();
    def.peripherals.cartridge = CartridgeDTO::ROMPak {
        path: pak.display().to_string(),
    };

    let rows = rom_rows(&def, "alpha", None);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].role, "ROM Pak");
    assert_eq!(rows[1].path, Path::new(&pak));
    assert_eq!(rows[1].status, ROMStatus::Unrecognized(64));
}

#[test]
fn orch90_in_the_bare_port_resolves_its_stock_rom() {
    let dir = TempDir::new("roms-orch90");
    let mut def = coco3_def();
    def.peripherals.cartridge = CartridgeDTO::Orch90;

    let rows = rom_rows(&def, "alpha", Some(dir.path()));
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].role, "Orchestra-90 ROM");
    assert_eq!(rows[1].path, dir.path().join(rom_load::ORCH90_ROM));
    assert_eq!(rows[1].status, ROMStatus::Missing);
}

#[test]
fn hdbdos_rom_is_listed_direct_and_in_mpi() {
    for mpi in [false, true] {
        let mut def = coco3_def();
        let dos_rom = DosRom::HdbDosDw3;
        def.peripherals.cartridge = if mpi {
            let mut slots = std::array::from_fn(|_| SlotDTO::Empty);
            slots[crate::DEFAULT_MPI_SWITCH_SLOT] = SlotDTO::FD502 { dos_rom };
            CartridgeDTO::MPI {
                slots,
                switch: crate::DEFAULT_MPI_SWITCH_SLOT + 1,
            }
        } else {
            CartridgeDTO::FD502 { dos_rom }
        };
        let rows = rom_rows(&def, "alpha", None);
        let row = rows.last().unwrap();
        assert!(row.role.ends_with(dos_rom.label()));
        assert_eq!(file_name(&row.path), dos_rom.filename());
        assert_eq!(row.status, ROMStatus::Missing);
    }
}
