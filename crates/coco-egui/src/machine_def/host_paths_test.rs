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

/// The front-panel switch of the MultiPaks [`cartridges_with_rom`] builds.
const MPI_SWITCH: usize = 1;

/// Every cartridge that names a ROM image, set to `rom`: each path-bearing
/// variant at the top level, then each in a MultiPak slot.
fn cartridges_with_rom(rom: &str) -> Vec<CartridgeDTO> {
    let path = || rom.to_string();
    let in_slot = |slot| CartridgeDTO::MPI {
        slots: [slot, SlotDTO::Empty, SlotDTO::Empty, SlotDTO::RTC],
        switch: MPI_SWITCH,
    };
    vec![
        CartridgeDTO::ROMPak { path: path() },
        CartridgeDTO::BankedROMPak { path: path() },
        CartridgeDTO::GamesMaster { path: path() },
        in_slot(SlotDTO::ROMPak { path: path() }),
        in_slot(SlotDTO::BankedROMPak { path: path() }),
        in_slot(SlotDTO::GamesMaster { path: path() }),
    ]
}

/// Every path-bearing cartridge, at the top level or in a MultiPak slot,
/// lists its ROM image, relative or absolute.
#[test]
fn host_paths_lists_every_cartridge_rom_path() {
    for rom in [RELATIVE_ROM, ABSOLUTE_ROM] {
        for cartridge in cartridges_with_rom(rom) {
            let mut def = full_def();
            def.peripherals.cartridge = cartridge;

            let paths = def.host_paths();

            assert!(
                paths.contains(&rom),
                "{:?}: {paths:?}",
                def.peripherals.cartridge
            );
        }
    }
}
