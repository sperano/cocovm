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

/// Drives the real installed asset bundle; skips when its `cartridges/`
/// directory doesn't hold the probe image.
#[test]
fn known_cartridges_submenu_picks_bundled_image() {
    const PROBE_FILE: &str = "Androne (1983) (26-3096) (Tandy).ccc";
    let installed = crate::paths::cartridges_dir().filter(|dir| dir.join(PROBE_FILE).is_file());
    if installed.is_none() {
        eprintln!(
            "skipping known_cartridges_submenu_picks_bundled_image: assets/cartridges not present"
        );
        return;
    }

    let dir = TempDir::new("known-carts");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    open_combo_at(&mut harness, "None", 1);
    assert_eq!(
        harness.query_all_by_label("Cartridge ROM…").count(),
        1,
        "combo popup not open"
    );
    assert_eq!(
        harness.query_all_by_label("Known Cartridges ⏵").count(),
        1,
        "submenu entry missing"
    );

    click(&mut harness, "Known Cartridges ⏵");
    harness.step();
    assert!(
        label_exists(&harness, "Androne"),
        "known cartridge rows missing from the submenu"
    );

    click(&mut harness, "Androne");
    harness.step();
    assert_eq!(
        harness.query_all_by_label("Known Cartridges ⏵").count(),
        0,
        "combo popup should have closed after the pick"
    );

    let form = harness.state_mut().edit_form_mut().expect("form present");
    match &form.cartridge {
        new_vm::CartridgeChoice::Image(image) => {
            assert!(image.path.ends_with(PROBE_FILE), "picked {:?}", image.path);
            assert!(image.hardware_detected);
            assert_eq!(image.hardware, CartridgeHardware::RomPak);
        }
        other => panic!("expected an Image choice, got {other:?}"),
    }
}
