//! Regression tests for confirmed machine deletion.

use std::time::Duration;

use coco_core::MachineConfig;

use super::*;
use crate::machine_def::{self, tests::TempDir};
use crate::manager::MachineEntry;

const TEST_SLUG: &str = "running-delete";
const LIVE_RUNTIME_SECS: u64 = 37;

fn test_manager(machines_dir: &std::path::Path) -> ManagerApp {
    let def = machine_def::MachineDef::from_config(
        "Running Delete Test".to_string(),
        None,
        &MachineConfig::default(),
    );
    ManagerApp::new(
        None,
        Some(machines_dir.to_path_buf()),
        None,
        vec![MachineEntry::new(TEST_SLUG.to_string(), def)],
        None,
    )
}

#[test]
fn deleting_running_machine_does_not_recreate_definition() {
    let machines_dir = TempDir::new("delete-running-machine");
    let mut manager = test_manager(machines_dir.path());
    let definition_path = machines_dir.path().join(format!("{TEST_SLUG}.toml"));

    manager.start_vm(0);
    assert!(
        manager.entries[0].vm.is_some(),
        "launch should succeed: {:?}",
        manager.entries[0].launch_error
    );
    assert!(definition_path.is_file());
    manager.entries[0].vm.as_mut().unwrap().total_runtime = Duration::from_secs(LIVE_RUNTIME_SECS);

    manager.delete_machine(0).expect("delete should succeed");

    assert!(manager.entries.is_empty());
    assert!(
        !definition_path.exists(),
        "stopping the VM must not recreate its deleted definition"
    );
}
