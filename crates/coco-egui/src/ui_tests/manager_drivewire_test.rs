//! Manager DriveWire definition and cold-launch UI regressions.

use std::fs;
use std::path::{Path, PathBuf};

use coco_core::{drivewire::DRIVE_COUNT, rom_db::CartridgeHardware};
use egui_kittest::kittest::{NodeT, Queryable};

use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;

const CONFLICT_ERROR: &str =
    "DriveWire Becker port conflicts with the Games Master Cartridge at $FF41";
const STARTUP_PATHS: [&str; DRIVE_COUNT] = [
    "/images/Boot Disk.dsk",
    "relative.os9",
    "disks/日本語.img",
    " disk.vhd ",
];

fn seed_manager(name: &str) -> (TempDir, manager::MachineEntry, PathBuf) {
    let dir = TempDir::new(name);
    let entry = sample_entry("drivewire", "DriveWire CoCo");
    machine_def::save(dir.path(), &entry.slug, &entry.def).expect("seed definition");
    let file = dir.path().join("drivewire.toml");
    (dir, entry, file)
}

fn select_drivewire(harness: &mut ManagerHarness) {
    click(harness, "DriveWire CoCo");
    click(harness, "DriveWire");
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
fn drivewire_paths_persist_while_typing_and_reopen_for_clearing() {
    let (dir, entry, file) = seed_manager("ui-drivewire-persist");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![entry]);
    select_drivewire(&mut harness);

    click(&mut harness, "Enable DriveWire");
    click(&mut harness, "HDB-DOS mode");
    for (drive, path) in STARTUP_PATHS.iter().enumerate() {
        let label = format!("DW{drive} disk image");
        assert!(
            harness
                .query_by_label(&format!("Clear DW{drive}"))
                .is_none()
        );
        harness.get_by_label(&label).focus();
        harness.step();
        harness.get_by_label(&label).type_text(path);
        harness.step();
        assert_eq!(saved_def(&file).drivewire.disk_paths()[drive], Some(*path));
    }

    let saved = saved_def(&file);
    assert!(saved.drivewire.enabled);
    assert!(saved.drivewire.hdbdos_mode);
    harness.get_by_label(
        "DriveWire changes reach a running machine immediately. Resume keeps the saved session.",
    );

    drop(harness);
    let reopened = manager::MachineEntry::new("drivewire".to_string(), saved);
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![reopened]);
    select_drivewire(&mut harness);
    assert_reopened_paths_and_clear(&mut harness, &file);
}

fn assert_reopened_paths_and_clear(harness: &mut ManagerHarness, file: &Path) {
    for (drive, path) in STARTUP_PATHS.iter().enumerate() {
        assert_eq!(
            harness
                .get_by_label(&format!("DW{drive} disk image"))
                .accesskit_node()
                .value()
                .as_deref(),
            Some(*path)
        );
        click(harness, &format!("Clear DW{drive}"));
        assert!(saved_def(file).drivewire.disk_paths()[drive].is_none());
        assert!(
            harness
                .query_by_label(&format!("Clear DW{drive}"))
                .is_none()
        );
    }
}

#[test]
fn deleting_path_text_leaves_the_startup_drive_empty() {
    let (dir, mut entry, file) = seed_manager("ui-drivewire-delete-text");
    entry.def.drivewire.enabled = true;
    entry.def.drivewire.disk0 = Some("startup.dsk".to_owned());
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![entry]);
    select_drivewire(&mut harness);
    harness.get_by_label("DW0 disk image").focus();
    harness.step();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.key_press(egui::Key::Backspace);
    harness.step();
    harness.step();
    assert!(saved_def(&file).drivewire.disk0.is_none());
    assert!(harness.query_by_label("Clear DW0").is_none());
}

#[test]
fn disabled_drivewire_paths_keep_their_contents_and_disable_all_controls() {
    let (dir, mut entry, file) = seed_manager("ui-drivewire-disabled");
    entry.def.drivewire.disk0 = Some("startup.dsk".to_owned());
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![entry]);
    select_drivewire(&mut harness);
    for drive in 0..DRIVE_COUNT {
        for label in [
            format!("DW{drive} disk image"),
            format!("Browse DW{drive}…"),
        ] {
            assert!(harness.get_by_label(&label).accesskit_node().is_disabled());
        }
    }
    assert!(
        harness
            .get_by_label("Clear DW0")
            .accesskit_node()
            .is_disabled()
    );
    click(&mut harness, "Enable DriveWire");
    click(&mut harness, "Enable DriveWire");
    assert_eq!(
        saved_def(&file).drivewire.disk0.as_deref(),
        Some("startup.dsk")
    );
}

#[test]
fn drivewire_paths_and_browse_buttons_align_at_narrow_manager_widths() {
    const WINDOW_WIDTHS: [f32; 2] = [1080.0, 700.0];
    const WINDOW_HEIGHT: f32 = 720.0;
    let (dir, mut entry, _) = seed_manager("ui-drivewire-layout");
    entry.def.drivewire.enabled = true;
    entry.def.drivewire.disk0 = Some("/images/a long disk image filename.dsk".to_owned());
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![entry]);
    select_drivewire(&mut harness);
    for width in WINDOW_WIDTHS {
        harness.set_size(egui::vec2(width, WINDOW_HEIGHT));
        harness.step();
        harness.step();
        let first = harness.get_by_label("DW0 disk image").rect();
        let first_browse = harness.get_by_label("Browse DW0…").rect();
        for drive in 0..DRIVE_COUNT {
            let input = harness
                .get_by_label(&format!("DW{drive} disk image"))
                .rect();
            let browse = harness.get_by_label(&format!("Browse DW{drive}…")).rect();
            assert_eq!(input.x_range(), first.x_range());
            assert_eq!(browse.x_range(), first_browse.x_range());
            assert!(input.right() <= browse.left());
            assert!(browse.right() <= width);
        }
        let clear = harness.get_by_label("Clear DW0").rect();
        assert!(first.contains_rect(clear));
    }
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
fn edits_reach_the_running_session_without_resetting_its_protocol() {
    let (dir, mut entry, file) = seed_manager("ui-drivewire-live");
    const CLIENT_VERSION: u8 = 1;
    const SERVER_VERSION: u8 = 4;
    const REPLY_AVAILABLE: u8 = 2;
    const FIRST_CYCLE: u64 = 1;
    const SECOND_CYCLE: u64 = 2;

    let startup = write_disk(dir.path(), "startup.dsk");
    let next_startup = write_disk(dir.path(), "next-startup.dsk");
    entry.def.drivewire.enabled = true;
    entry.def.drivewire.disk0 = Some(startup.display().to_string());
    machine_def::save(dir.path(), &entry.slug, &entry.def).unwrap();
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![entry]);
    select_drivewire(&mut harness);
    click(&mut harness, "Start");

    let vm = harness.state_mut().entries[0].vm.as_mut().unwrap();
    vm.set_running(false);
    let dw = vm.machine.bus.drivewire.as_mut().unwrap();
    dw.data_write(coco_core::drivewire::opcode::DWINIT, FIRST_CYCLE);
    dw.data_write(CLIENT_VERSION, SECOND_CYCLE);
    assert_eq!(dw.status_read(), REPLY_AVAILABLE);

    click(&mut harness, "Input");
    click(&mut harness, "Symbolic");
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
    assert_eq!(vm.dw_paths[0].as_deref(), Some(next_startup.as_path()));
    let dw = vm.machine.bus.drivewire.as_ref().unwrap();
    assert!(dw.hdbdos_mode());
    assert!(dw.is_mounted(0));
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

#[test]
fn host_shares_are_added_edited_and_removed_through_the_drivewire_tab() {
    let (dir, mut entry, file) = seed_manager("ui-drivewire-shares");
    entry.def.drivewire.enabled = true;
    let folder = dir.path().join("games");
    fs::create_dir_all(&folder).unwrap();
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![entry]);
    select_drivewire(&mut harness);

    click(&mut harness, "Add share");
    let saved = saved_def(&file).drivewire.shares;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "share1");
    assert_eq!(saved[0].access, machine_def::ShareAccessDTO::ReadOnly);
    harness.get_by_label("No folder selected; this share is inactive.");

    harness.get_by_label("Share 1 folder").focus();
    harness.step();
    harness
        .get_by_label("Share 1 folder")
        .type_text(&folder.display().to_string());
    harness.step();
    harness.step();
    assert_eq!(
        saved_def(&file).drivewire.shares[0].path,
        folder.display().to_string()
    );
    assert!(
        harness
            .query_by_label("No folder selected; this share is inactive.")
            .is_none()
    );

    click(&mut harness, "Share 1 access");
    click(&mut harness, "Read/write");
    assert_eq!(
        saved_def(&file).drivewire.shares[0].access,
        machine_def::ShareAccessDTO::ReadWrite
    );

    click(&mut harness, "Add share");
    harness.get_by_label("Share 2 name").focus();
    harness.step();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.get_by_label("Share 2 name").type_text("SHARE1");
    harness.step();
    harness.step();
    // Shown under the row and as the auto-save error that keeps the file unchanged.
    let duplicate = "DriveWire share name \"SHARE1\" is used twice";
    assert_eq!(harness.query_all_by_label(duplicate).count(), 2);
    assert_eq!(saved_def(&file).drivewire.shares[1].name, "share2");

    click(&mut harness, "Remove share 2");
    click(&mut harness, "Remove share 1");
    assert!(saved_def(&file).drivewire.shares.is_empty());
}

#[test]
fn host_share_controls_follow_the_drivewire_switch() {
    let (dir, mut entry, _) = seed_manager("ui-drivewire-shares-disabled");
    entry.def.drivewire.shares = vec![machine_def::DriveWireShareDTO {
        name: "games".to_string(),
        path: dir.path().join("missing").display().to_string(),
        access: machine_def::ShareAccessDTO::ReadOnly,
    }];
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![entry]);
    select_drivewire(&mut harness);
    for label in [
        "Add share",
        "Share 1 name",
        "Share 1 folder",
        "Remove share 1",
    ] {
        assert!(
            harness.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
    click(&mut harness, "Enable DriveWire");
    assert!(
        !harness
            .get_by_label("Add share")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        harness
            .query_all_by_label_contains("not found")
            .next()
            .is_some()
    );
}
