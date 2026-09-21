use coco_core::{MachineConfig, snapshot};

use crate::machine_def::{CartridgeDTO, DosRom, MachineDef, SlotDTO};

const FD502_SLOT: usize = crate::DEFAULT_MPI_SWITCH_SLOT;

fn definition(dos_rom: DosRom, slotted: bool) -> MachineDef {
    let mut def = MachineDef::from_config("DOS ROM test".into(), None, &MachineConfig::default());
    def.peripherals.cartridge = if slotted {
        let mut slots = std::array::from_fn(|_| SlotDTO::Empty);
        slots[FD502_SLOT] = SlotDTO::FD502 { dos_rom };
        CartridgeDTO::MPI {
            slots,
            switch: FD502_SLOT + 1,
        }
    } else {
        CartridgeDTO::FD502 { dos_rom }
    };
    def
}

#[test]
fn selected_dos_rom_survives_save_restore_and_resave() {
    let dir = crate::machine_def::tests::TempDir::new("dos-rom-state");
    for dos_rom in [DosRom::DiskBasic, DosRom::HdbDosDw3] {
        for slotted in [false, true] {
            let def = definition(dos_rom, slotted);
            let mut app = crate::launch_machine(&def, "dos-rom-test").unwrap();
            let path = crate::rom_load::dos_rom_path(dos_rom);
            assert_eq!(app.disk_rom_path.as_ref(), Some(&path));
            let media = app.build_media_refs().unwrap();
            assert_eq!(media.cart_roms.len(), 1);
            let reference = &media.cart_roms[0];
            assert_eq!(reference.mpi_slot, slotted.then_some(FD502_SLOT as u8));
            assert_eq!(reference.rom.path, path);
            assert_eq!(reference.rom.sha256, snapshot::sha256_file(&path).unwrap());
            let state_path = dir.path().join("dos.ccstate");
            app.save_state_to(&state_path).unwrap();
            let mut restored = super::tests::boot_app();
            restored.load_state_from(&state_path).unwrap();
            assert_eq!(restored.disk_rom_path.as_ref(), Some(&path));
            let resaved = restored.build_media_refs().unwrap();
            assert_eq!(resaved.cart_roms[0].rom.path, path);
            assert_eq!(resaved.cart_roms[0].rom.sha256, reference.rom.sha256);
        }
    }
}

#[test]
fn hdbdos_controller_launches_with_the_saved_drivewire_settings() {
    for slotted in [false, true] {
        for (enabled, hdbdos_mode) in [(false, false), (true, false), (true, true)] {
            let mut def = definition(DosRom::HdbDosDw3, slotted);
            def.drivewire.enabled = enabled;
            def.drivewire.hdbdos_mode = hdbdos_mode;
            let mut app = crate::launch_machine(&def, "hdbdos-drivewire").unwrap();
            let dw = app.machine.bus.drivewire.as_ref();
            assert_eq!(
                dw.map(|dw| dw.hdbdos_mode()),
                enabled.then_some(hdbdos_mode)
            );
            assert!(app.machine.bus.cart.as_disk_cart().is_some());
        }
    }
}
