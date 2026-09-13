//! Manager UI coverage for cartridge ROM detection and unknown-ROM fallback.

use std::path::PathBuf;

use coco_core::rom_db::CartridgeHardware;
use egui_kittest::kittest::Queryable;

use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;

#[test]
fn cartridge_dropdown_has_one_rom_picker() {
    let dir = TempDir::new("cartridge-picker");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    open_combo_at(&mut harness, "None", 1);

    assert_eq!(harness.query_all_by_label("Cartridge ROM…").count(), 1);
    assert_eq!(harness.query_all_by_label("ROM Pak…").count(), 0);
    assert_eq!(harness.query_all_by_label("Banked ROM Pak…").count(), 0);
    assert_eq!(harness.query_all_by_label("Games Master…").count(), 0);
}

#[test]
fn unknown_rom_hardware_fallback_can_be_changed() {
    let dir = TempDir::new("cartridge-fallback");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    let form = harness
        .state_mut()
        .edit_form_mut()
        .expect("pane form seeded");
    form.cartridge = new_vm::CartridgeChoice::Image(cartridge_image_choice(
        PathBuf::from("/paks/unknown.rom"),
        CartridgeHardware::RomPak,
    ));
    harness.step();

    select_combo_at(
        &mut harness,
        "ROM Pak (fallback)",
        0,
        "Games Master Cartridge",
    );

    assert_eq!(
        harness.state().entries[0].def.peripherals.cartridge,
        machine_def::CartridgeDTO::GamesMaster {
            path: "/paks/unknown.rom".to_string(),
            autostart: true,
        }
    );
}
