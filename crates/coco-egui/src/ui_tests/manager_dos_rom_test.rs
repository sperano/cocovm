use std::fs;
use egui_kittest::kittest::{NodeT, Queryable};

use crate::machine_def::tests::TempDir;
use crate::machine_def::{DosRom, MachineDef};
use crate::new_vm::{CartridgeChoice, SlotChoice};

use super::harness::*;

fn saved_definition(dir: &TempDir) -> MachineDef {
    toml::from_str(&fs::read_to_string(dir.path().join("coco-3.toml")).unwrap()).unwrap()
}

#[test]
fn new_machine_dos_rom_selection_saves_drivewire_settings_direct_and_in_mpi() {
    for mpi in [false, true] {
        let dir = TempDir::new(&format!("ui-dos-rom-{mpi}"));
        let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());
        click_containing(&mut harness, "New");
        if mpi {
            select_combo_at(&mut harness, "None", 1, "MultiPak Interface");
            select_combo_at(&mut harness, "Empty", 0, "FD-502");
        } else {
            select_combo_at(&mut harness, "None", 1, "FD-502");
        }
        select_combo_at(&mut harness, "Disk BASIC 1.1", 0, DosRom::HdbDosDw3.label());
        let saved = saved_definition(&dir);
        assert_eq!(
            saved.peripherals.cartridge.dos_rom(),
            Some(DosRom::HdbDosDw3)
        );
        assert!(saved.drivewire.enabled);
        assert!(saved.drivewire.hdbdos_mode);
        for label in ["Enable DriveWire", "HDB-DOS mode"] {
            assert!(harness.get_by_label(label).accesskit_node().is_disabled());
        }
        let form = harness.state_mut().edit_form_mut().unwrap();
        form.drivewire.enabled = false;
        form.drivewire.hdbdos_mode = false;
        harness.step();
        harness.step();
        let saved = saved_definition(&dir);
        assert!(saved.drivewire.enabled);
        assert!(saved.drivewire.hdbdos_mode);
    }
}

#[test]
fn changing_model_resets_hdbdos_direct_and_in_mpi() {
    for mpi in [false, true] {
        let dir = TempDir::new(&format!("ui-dos-model-{mpi}"));
        let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());
        click_containing(&mut harness, "New");
        let form = harness.state_mut().edit_form_mut().unwrap();
        let dos_rom = DosRom::HdbDosDw3;
        if mpi {
            form.cartridge = CartridgeChoice::MPI;
            form.mpi_slots[crate::DEFAULT_MPI_SWITCH_SLOT] = SlotChoice::FD502 { dos_rom };
        } else {
            form.cartridge = CartridgeChoice::FD502 { dos_rom };
        }
        harness.step();
        for click_model in [false, true] {
            let combo = harness.get_all_by_value("CoCo 3")
                .find(|node| node.accesskit_node().role() == eframe::egui::accesskit::Role::ComboBox)
                .unwrap();
            if click_model { combo.click(); } else { combo.hover(); }
            harness.step();
        }
        harness.step();
        click(&mut harness, "CoCo 2");
        assert_eq!(
            saved_definition(&dir).peripherals.cartridge.dos_rom(),
            Some(DosRom::DiskBasic)
        );
        open_combo_at(&mut harness, "Disk BASIC 1.1", 0);
        assert!(
            harness
                .get_by_label(DosRom::HdbDosDw3.label())
                .accesskit_node()
                .is_disabled()
        );
    }
}
