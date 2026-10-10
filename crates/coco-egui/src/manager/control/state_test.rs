use super::*;
use crate::machine_def;
use crate::machine_def::tests::TempDir;
use crate::manager::MachineEntry;
use crate::save_state::tests::{ReadOnly, is_dirty, mount_and_dirty, write_one_track_disk};
use crate::{AppParams, CocoApp, MachineConfig, ROMSource};

fn running_entry(slug: &str) -> MachineEntry {
    let config = MachineConfig::default();
    let def = machine_def::MachineDef::from_config(slug.to_string(), None, &config);
    let rom_path = crate::installed_roms_dir().join(crate::rom_load::COCO3_ROM_FILE);
    let rom = std::fs::read(&rom_path)
        .expect("installed coco3.rom is required (first-run asset download)")
        .into_boxed_slice();
    let vm = CocoApp::new(
        config,
        rom,
        ROMSource::File(rom_path),
        AppParams::default(),
        crate::joy::SharedGamepad::without_backend(),
    );
    let mut entry = MachineEntry::new(slug.to_string(), def);
    entry.vm = Some(Box::new(vm));
    entry
}

fn manager(entries: Vec<MachineEntry>) -> ManagerApp {
    ManagerApp::new(None, None, None, entries, None)
}

#[test]
fn path_save_and_load_round_trip_machine_state() {
    let dir = TempDir::new("mcp-state-path-round-trip");
    let path = dir.path().join("state.ccstate");
    let mut manager = manager(vec![running_entry("live")]);
    manager.entries[0].vm.as_mut().unwrap().machine.cpu.pc = 0x1234;

    manager
        .save_state_action(&Some("live".into()), StateTarget::Path(path.clone()))
        .expect("save state");
    manager.entries[0].vm.as_mut().unwrap().machine.cpu.pc = 0x5678;
    manager
        .load_state_action(&Some("live".into()), StateTarget::Path(path))
        .expect("load state");

    assert_eq!(
        manager.entries[0].vm.as_ref().unwrap().machine.cpu.pc,
        0x1234
    );
}

#[test]
fn slot_save_creates_the_directory_and_slot_load_round_trips() {
    let dir = TempDir::new("mcp-state-slot-round-trip");
    let quick_dir = dir.path().join("nested").join("quick");
    let mut manager = manager(vec![running_entry("live")]);
    let vm = manager.entries[0].vm.as_mut().unwrap();
    vm.quick_state_dir = Some(quick_dir.clone());
    vm.machine.cpu.pc = 0x2345;

    manager
        .save_state_action(&None, StateTarget::Slot(1))
        .expect("save quick state");
    assert!(quick_dir.join("slot-2.ccstate").is_file());
    manager.entries[0].vm.as_mut().unwrap().machine.cpu.pc = 0x6789;
    manager
        .load_state_action(&None, StateTarget::Slot(1))
        .expect("load quick state");

    assert_eq!(
        manager.entries[0].vm.as_ref().unwrap().machine.cpu.pc,
        0x2345
    );
}

#[test]
fn missing_slot_is_reported() {
    let dir = TempDir::new("mcp-state-missing-slot");
    let mut manager = manager(vec![running_entry("live")]);
    manager.entries[0].vm.as_mut().unwrap().quick_state_dir = Some(dir.path().to_path_buf());

    let error = manager
        .load_state_action(&None, StateTarget::Slot(0))
        .expect_err("missing slot must fail");

    assert!(error.contains("could not read"), "{error}");
    assert!(error.contains("slot-1.ccstate"), "{error}");
}

#[test]
fn state_actions_require_a_running_vm() {
    let mut manager = manager(vec![running_entry("paused")]);
    manager.entries[0].suspended = true;

    for save in [true, false] {
        let result = if save {
            manager.save_state_action(&Some("paused".into()), StateTarget::Slot(0))
        } else {
            manager.load_state_action(&Some("paused".into()), StateTarget::Slot(0))
        };
        assert!(result.unwrap_err().contains("not running"));
    }
}

#[test]
fn save_surfaces_dirty_disk_write_back_failure() {
    let dir = TempDir::new("mcp-state-dirty-disk");
    let disk_path = dir.path().join("dirty.dsk");
    let state_path = dir.path().join("state.ccstate");
    write_one_track_disk(&disk_path);
    let mut manager = manager(vec![running_entry("live")]);
    mount_and_dirty(manager.entries[0].vm.as_mut().unwrap(), 0, &disk_path);

    let _read_only = ReadOnly::new(&disk_path);
    let error = manager
        .save_state_action(&None, StateTarget::Path(state_path.clone()))
        .expect_err("dirty disk write-back must fail the tool action");

    assert!(error.contains(&disk_path.display().to_string()), "{error}");
    assert!(!state_path.exists());
    assert!(is_dirty(manager.entries[0].vm.as_mut().unwrap(), 0));
}
