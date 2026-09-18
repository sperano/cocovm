//! Manager DriveWire definition and cold-launch UI regressions.

use std::fs;
use std::path::{Path, PathBuf};

use coco_core::rom_db::CartridgeHardware;
use egui_kittest::kittest::{NodeT, Queryable};

use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;

const CONFLICT_ERROR: &str =
    "DriveWire Becker port conflicts with the Games Master Cartridge at $FF41";

fn seed_manager(name: &str) -> (TempDir, manager::MachineEntry, PathBuf) {
    let dir = TempDir::new(name);
    let entry = sample_entry("drivewire", "DriveWire CoCo");
    machine_def::save(dir.path(), &entry.slug, &entry.def).expect("seed definition");
    let file = dir.path().join("drivewire.toml");
    (dir, entry, file)
}

fn select_drivewire(harness: &mut ManagerHarness) {
    click(harness, "DriveWire CoCo");
    harness.get_by_label("DriveWire");
}

fn write_disk(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, vec![0_u8; coco_core::drivewire::SECTOR_SIZE]).expect("write DW disk");
    path
}

fn saved_def(file: &Path) -> machine_def::MachineDef {
    toml::from_str(&fs::read_to_string(file).expect("read definition")).expect("parse definition")
}

#[test]
fn drivewire_controls_persist_and_eject_the_startup_disk() {
    let (dir, entry, file) = seed_manager("ui-drivewire-persist");
    let disk = write_disk(dir.path(), "dw0.dsk");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![entry]);
    select_drivewire(&mut harness);

    click(&mut harness, "Enable DriveWire");
    click(&mut harness, "HDB-DOS mode");
    harness.state_mut().edit_form_mut().unwrap().drivewire.disk0 = Some(disk.display().to_string());
    harness.step();
    harness.step();

    let saved = saved_def(&file);
    assert!(saved.drivewire.enabled);
    assert!(saved.drivewire.hdbdos_mode);
    assert_eq!(saved.drivewire.disk0.as_deref(), disk.to_str());
    harness.get_by_label(
        "DriveWire changes apply at the next start from power off. Resume keeps the saved session.",
    );

    drop(harness);
    let reopened = manager::MachineEntry::new("drivewire".to_string(), saved);
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![reopened]);
    select_drivewire(&mut harness);
    harness.get_by_label("dw0.dsk");
    click(&mut harness, "Eject DW0");
    assert!(saved_def(&file).drivewire.disk0.is_none());
}

#[test]
fn cold_start_mounts_configured_drivewire_mode_and_disk() {
    let (dir, mut entry, _) = seed_manager("ui-drivewire-launch");
    let disk = write_disk(dir.path(), "startup.dsk");
    entry.def.drivewire.enabled = true;
    entry.def.drivewire.hdbdos_mode = true;
    entry.def.drivewire.disk0 = Some(disk.display().to_string());
    machine_def::save(dir.path(), &entry.slug, &entry.def).unwrap();
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![entry]);

    click(&mut harness, "DriveWire CoCo");
    click(&mut harness, "Start");

    let vm = harness.state().entries[0].vm.as_ref().expect("VM launched");
    let dw = vm.machine.bus.drivewire.as_ref().expect("Becker enabled");
    assert!(dw.hdbdos_mode());
    assert!(dw.is_mounted(0));
    assert_eq!(vm.dw_paths[0].as_deref(), Some(disk.as_path()));
}

#[test]
fn missing_startup_disk_surfaces_as_launch_error() {
    let (dir, mut entry, _) = seed_manager("ui-drivewire-missing");
    entry.def.drivewire.enabled = true;
    entry.def.drivewire.disk0 = Some(dir.path().join("missing.dsk").display().to_string());
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![entry]);

    click(&mut harness, "DriveWire CoCo");
    click(&mut harness, "Start");

    let error = harness.state().entries[0]
        .launch_error
        .as_deref()
        .expect("bad startup media must fail launch");
    assert!(error.contains("missing.dsk"), "unexpected error: {error}");
    assert!(harness.state().entries[0].vm.is_none());
}

#[test]
fn games_master_disables_drivewire_and_rejects_a_later_conflict() {
    let (dir, mut entry, file) = seed_manager("ui-drivewire-gmc");
    entry.def.peripherals.cartridge = machine_def::CartridgeDTO::GamesMaster {
        path: "gmc.rom".to_string(),
        autostart: false,
    };
    machine_def::save(dir.path(), &entry.slug, &entry.def).unwrap();
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![entry]);
    select_drivewire(&mut harness);

    assert!(
        harness
            .get_by_label("Enable DriveWire")
            .accesskit_node()
            .is_disabled()
    );

    harness.state_mut().edit_form_mut().unwrap().cartridge = new_vm::CartridgeChoice::None;
    harness.step();
    click(&mut harness, "Enable DriveWire");
    harness.state_mut().edit_form_mut().unwrap().cartridge =
        new_vm::CartridgeChoice::Image(new_vm::CartridgeImageChoice {
            path: dir.path().join("gmc.rom"),
            autostart: false,
            hardware: CartridgeHardware::GamesMaster,
            hardware_detected: true,
        });
    harness.step();
    harness.step();

    harness.get_by_label(CONFLICT_ERROR);
    assert!(saved_def(&file).drivewire.enabled);
    assert!(
        !saved_def(&file)
            .peripherals
            .cartridge
            .contains_games_master()
    );
}

#[test]
fn edits_leave_the_active_drivewire_session_unchanged() {
    let (dir, mut entry, file) = seed_manager("ui-drivewire-live");
    const CLIENT_VERSION: u8 = 1;
    const SERVER_VERSION: u8 = 4;
    const REPLY_AVAILABLE: u8 = 2;
    const FIRST_CYCLE: u64 = 1;
    const SECOND_CYCLE: u64 = 2;

    let startup = write_disk(dir.path(), "startup.dsk");
    let runtime = write_disk(dir.path(), "runtime.dsk");
    let next_startup = write_disk(dir.path(), "next-startup.dsk");
    entry.def.drivewire.enabled = true;
    entry.def.drivewire.disk0 = Some(startup.display().to_string());
    machine_def::save(dir.path(), &entry.slug, &entry.def).unwrap();
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![entry]);
    select_drivewire(&mut harness);
    click(&mut harness, "Start");

    let vm = harness.state_mut().entries[0].vm.as_mut().unwrap();
    vm.set_running(false);
    vm.insert_dw_disk(0, runtime.clone());
    let dw = vm.machine.bus.drivewire.as_mut().unwrap();
    dw.data_write(coco_core::drivewire::opcode::DWINIT, FIRST_CYCLE);
    dw.data_write(CLIENT_VERSION, SECOND_CYCLE);
    assert_eq!(dw.status_read(), REPLY_AVAILABLE);

    click(&mut harness, "4:3 aspect correction");
    harness.state_mut().edit_form_mut().unwrap().drivewire.disk0 =
        Some(next_startup.display().to_string());
    harness
        .state_mut()
        .edit_form_mut()
        .unwrap()
        .drivewire
        .hdbdos_mode = true;
    harness.step();
    harness.step();

    let vm = harness.state().entries[0]
        .vm
        .as_ref()
        .expect("session remains alive");
    assert!(!vm.is_running());
    assert_eq!(vm.dw_paths[0].as_deref(), Some(runtime.as_path()));
    let dw = vm.machine.bus.drivewire.as_ref().unwrap();
    assert!(!dw.hdbdos_mode());
    assert_eq!(dw.status_read(), REPLY_AVAILABLE);
    assert_eq!(
        harness.state_mut().entries[0]
            .vm
            .as_mut()
            .unwrap()
            .machine
            .bus
            .drivewire
            .as_mut()
            .unwrap()
            .data_read(),
        SERVER_VERSION
    );
    let saved = saved_def(&file);
    assert_eq!(saved.drivewire.disk0.as_deref(), next_startup.to_str());
    assert!(saved.drivewire.hdbdos_mode);
}
