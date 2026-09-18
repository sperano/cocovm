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
            assert!(dw.hdbdos_mode());
            assert!(dw.is_mounted(0));
            assert_eq!(vm.dw_paths[0], Some(image));
        }
    }
}
