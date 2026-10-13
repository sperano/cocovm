//! Resume restores the saved DriveWire session independently of startup edits.

use super::*;
use crate::machine_def::tests::TempDir;

const TEST_SLUG: &str = "drivewire-resume";

fn manager_with_disk(dir: &std::path::Path, enabled: bool) -> (ManagerApp, PathBuf) {
    let image = dir.join("snapshot.dsk");
    fs::write(&image, vec![0_u8; coco_core::drivewire::SECTOR_SIZE]).unwrap();
    let mut def = machine_def::MachineDef::from_config(
        "DriveWire resume".to_string(),
        None,
        &MachineConfig::default(),
    );
    def.drivewire.enabled = enabled;
    def.drivewire.hdbdos_mode = true;
    def.drivewire.disk0 = Some(image.display().to_string());
    let manager = ManagerApp::new(
        None,
        Some(dir.join("machines")),
        Some(dir.join("artifacts")),
        vec![MachineEntry::new(TEST_SLUG.to_string(), def)],
        None,
    );
    (manager, image)
}

#[test]
fn cold_resume_ignores_changed_startup_media_and_restores_saved_drivewire() {
    for enabled in [false, true] {
        let dir = TempDir::new(if enabled {
            "resume-dw-on"
        } else {
            "resume-dw-off"
        });
        let (mut manager, image) = manager_with_disk(dir.path(), enabled);
        manager.start_vm(0);
        assert!(manager.entries[0].vm.is_some());
        manager.suspend_vm(0);
        assert!(manager.entries[0].suspended);
        if enabled {
            let diagnostics = manager.entries[0]
                .vm
                .as_ref()
                .unwrap()
                .machine
                .bus
                .drivewire
                .as_ref()
                .unwrap()
                .host_diagnostics();
            assert_eq!(
                diagnostics.state,
                coco_core::drivewire::host::HostState::Suspended
            );
        }
        manager.entries[0].vm = None;
        let next = &mut manager.entries[0].def.drivewire;
        next.enabled = true;
        next.hdbdos_mode = false;
        next.disk0 = Some(dir.path().join("missing.dsk").display().to_string());
        let startup = next.clone();

        manager.resume_vm(0);

        let entry = &manager.entries[0];
        assert!(entry.launch_error.is_none(), "{:?}", entry.launch_error);
        assert!(!entry.suspended);
        assert_eq!(entry.def.drivewire, startup);
        let vm = entry.vm.as_ref().expect("saved session resumed");
        assert_eq!(vm.machine.bus.drivewire.is_some(), enabled);
        if let Some(dw) = &vm.machine.bus.drivewire {
            assert_eq!(
                dw.host_diagnostics().state,
                coco_core::drivewire::host::HostState::Running
            );
            assert!(dw.hdbdos_mode());
            assert!(dw.is_mounted(0));
            assert_eq!(vm.dw_paths[0], Some(image));
        }
    }
}

#[test]
fn cold_resume_keeps_the_saved_shares() {
    let dir = TempDir::new("resume-dw-shares");
    let (mut manager, _) = manager_with_disk(dir.path(), true);
    manager.entries[0].def.drivewire.shares = vec![machine_def::DriveWireShareDTO {
        name: "games".to_string(),
        path: dir.path().display().to_string(),
        access: machine_def::ShareAccessDTO::ReadOnly,
    }];
    manager.entries[0].def.drivewire.named_objects = Some("games".to_string());
    manager.start_vm(0);
    manager.suspend_vm(0);
    manager.entries[0].vm = None;

    manager.resume_vm(0);

    let entry = &manager.entries[0];
    assert!(entry.launch_error.is_none(), "{:?}", entry.launch_error);
    let dw = entry
        .vm
        .as_ref()
        .unwrap()
        .machine
        .bus
        .drivewire
        .as_ref()
        .unwrap();
    assert!(dw.share_session().table().get("games").is_some());
    assert_eq!(dw.share_session().table().named_objects(), Some("games"));
}

/// Starts a DriveWire VM printing into `prints` and sends it half a job.
fn manager_mid_print_job(dir: &std::path::Path, prints: &std::path::Path) -> ManagerApp {
    let (mut manager, _) = manager_with_disk(dir, true);
    let printer = &mut manager.entries[0].def.drivewire.printer;
    printer.output = machine_def::PrintOutputDTO::Folder;
    printer.folder = prints.display().to_string();
    manager.start_vm(0);
    let vm = manager.entries[0].vm.as_mut().unwrap();
    let dw = vm.machine.bus.drivewire.as_mut().unwrap();
    for byte in *b"HALF" {
        dw.data_write(coco_core::drivewire::opcode::PRINT, 0);
        dw.data_write(byte, 0);
    }
    manager
}

/// Waits for the print writer thread, which outlives a stopped VM.
fn wait_for_print_files(prints: &std::path::Path) -> Vec<String> {
    const WAIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);
    const WAIT_STEP: std::time::Duration = std::time::Duration::from_millis(2);
    let start = std::time::Instant::now();
    loop {
        let names: Vec<String> = fs::read_dir(prints)
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                    .collect()
            })
            .unwrap_or_default();
        if !names.is_empty() || start.elapsed() > WAIT_LIMIT {
            return names;
        }
        std::thread::sleep(WAIT_STEP);
    }
}

#[test]
fn stopping_a_suspended_vm_writes_its_open_print_job() {
    let dir = TempDir::new("stop-suspended-print");
    let prints = dir.path().join("prints");
    let mut manager = manager_mid_print_job(dir.path(), &prints);
    manager.suspend_vm(0);
    assert!(manager.entries[0].suspended);

    manager.stop_vm(0);

    let names = wait_for_print_files(&prints);
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(names[0].ends_with("-job0001-incomplete.txt"), "{names:?}");
    assert_eq!(fs::read(prints.join(&names[0])).unwrap(), b"HALF");
}

#[test]
fn closing_a_suspended_vm_leaves_its_open_print_job_to_the_state_file() {
    let dir = TempDir::new("close-suspended-print");
    let prints = dir.path().join("prints");
    let mut manager = manager_mid_print_job(dir.path(), &prints);
    manager.suspend_vm(0);
    manager.entries[0]
        .vm
        .as_mut()
        .unwrap()
        .stop_drivewire_host();
    manager.entries[0].vm = None;

    manager.resume_vm(0);

    let vm = manager.entries[0].vm.as_ref().unwrap();
    let dw = vm.machine.bus.drivewire.as_ref().unwrap();
    assert_eq!(dw.print_diagnostics().open_bytes, b"HALF".len());
    assert!(!prints.exists());
}
