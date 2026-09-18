use super::{CartridgeDTO, DosRom, MachineDef, SlotDTO};
use coco_core::{MachineConfig, MachineVariant, MemorySize};

fn controller(dos_rom: DosRom, mpi: bool) -> CartridgeDTO {
    if mpi {
        let mut slots = std::array::from_fn(|_| SlotDTO::Empty);
        slots[crate::DEFAULT_MPI_SWITCH_SLOT] = SlotDTO::FD502 { dos_rom };
        CartridgeDTO::MPI {
            slots,
            switch: crate::DEFAULT_MPI_SWITCH_SLOT + 1,
        }
    } else {
        CartridgeDTO::FD502 { dos_rom }
    }
}

#[test]
fn absent_dos_rom_defaults_to_disk_basic_direct_and_in_mpi() {
    for mpi in [false, true] {
        let expected = controller(DosRom::DiskBasic, mpi);
        let serialized = toml::to_string(&expected).unwrap();
        let legacy = serialized.replace("dos_rom = \"disk_basic\"\n", "");
        assert!(!legacy.contains("dos_rom"));
        let actual: CartridgeDTO = toml::from_str(&legacy).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual.dos_rom(), Some(DosRom::DiskBasic));
    }
}

#[test]
fn both_dos_rom_choices_round_trip_direct_and_in_mpi() {
    for mpi in [false, true] {
        for dos_rom in [DosRom::DiskBasic, DosRom::HdbDosDw3] {
            let expected = controller(dos_rom, mpi);
            let actual: CartridgeDTO =
                toml::from_str(&toml::to_string(&expected).unwrap()).unwrap();
            assert_eq!(actual, expected);
            assert_eq!(actual.uses_hdbdos(), dos_rom == DosRom::HdbDosDw3);
        }
    }
}

#[test]
fn hdbdos_requires_coco3_even_when_drivewire_is_disabled() {
    for variant in [MachineVariant::Coco1, MachineVariant::Coco2] {
        let config = MachineConfig {
            variant,
            memory: MemorySize::K64,
            monitor: None,
            ..MachineConfig::default()
        };
        for mpi in [false, true] {
            let mut def = MachineDef::from_config("DOS".into(), None, &config);
            def.peripherals.cartridge = controller(DosRom::HdbDosDw3, mpi);
            assert_eq!(
                def.to_machine_config().unwrap_err(),
                "HDB-DOS (DriveWire, CoCo 3) requires a CoCo 3"
            );
            def.peripherals.cartridge = controller(DosRom::DiskBasic, mpi);
            assert!(def.to_machine_config().is_ok());
        }
    }
}
