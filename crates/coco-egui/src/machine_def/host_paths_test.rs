use crate::machine_def::tests::full_def;
use crate::machine_def::{CartridgeDTO, PrintOutputDTO, SlotDTO};

/// A relative cartridge ROM path, resolved against the artifact dir.
const RELATIVE_ROM: &str = "paks/game.ccc";
/// An absolute cartridge ROM path.
const ABSOLUTE_ROM: &str = "/paks/game.ccc";

/// Every path-bearing field is listed, in field order; empty and unused
/// MPI slots add nothing.
#[test]
fn host_paths_lists_every_path_field() {
    let def = full_def();

    assert_eq!(
        def.host_paths(),
        [
            "dev.dsk",
            "/shared/utils.dsk",
            "/vhd/68SDC.VHD",
            "session.cas",
            "dw0.dsk",
            "/shared/dw2.dsk",
            "/shared/games",
            "prints",
            "/paks/arkanoid.ccc",
            "/path/custom.rom",
        ]
    );
}

/// A relative system ROM resolves against the working directory, never the
/// artifact dir, so it isn't listed; blank paths aren't either.
#[test]
fn host_paths_skips_relative_rom_and_blank_paths() {
    let mut def = full_def();
    def.hardware.rom = Some("custom.rom".to_string());
    def.drivewire.printer.folder = " ".to_string();

    let paths = def.host_paths();

    assert!(!paths.contains(&"custom.rom"), "{paths:?}");
    assert!(!paths.contains(&" "), "{paths:?}");
}

/// The print folder counts only when print jobs go there: launch ignores
/// it for any other output.
#[test]
fn host_paths_skips_print_folder_unless_output_is_folder() {
    for output in [PrintOutputDTO::Off, PrintOutputDTO::PrinterPort] {
        let mut def = full_def();
        def.drivewire.printer.output = output;

        let paths = def.host_paths();

        assert!(!paths.contains(&"prints"), "{output:?}: {paths:?}");
    }
}

/// Every path-bearing cartridge, at the top level or in a MultiPak slot,
/// lists its ROM image, relative or absolute.
#[test]
fn host_paths_lists_every_cartridge_rom_path() {
    let top_level: [fn(String) -> CartridgeDTO; 3] = [
        |path| CartridgeDTO::ROMPak { path },
        |path| CartridgeDTO::BankedROMPak { path },
        |path| CartridgeDTO::GamesMaster { path },
    ];
    let in_slot: [fn(String) -> SlotDTO; 3] = [
        |path| SlotDTO::ROMPak { path },
        |path| SlotDTO::BankedROMPak { path },
        |path| SlotDTO::GamesMaster { path },
    ];
    for rom in [RELATIVE_ROM, ABSOLUTE_ROM] {
        let mpi_slots = in_slot.iter().map(|slot| CartridgeDTO::MPI {
            slots: [
                slot(rom.to_string()),
                SlotDTO::Empty,
                SlotDTO::Empty,
                SlotDTO::RTC,
            ],
            switch: 1,
        });
        for cartridge in top_level
            .iter()
            .map(|cart| cart(rom.to_string()))
            .chain(mpi_slots)
        {
            let mut def = full_def();
            def.peripherals.cartridge = cartridge.clone();

            let paths = def.host_paths();

            assert!(paths.contains(&rom), "{cartridge:?}: {paths:?}");
        }
    }
}
