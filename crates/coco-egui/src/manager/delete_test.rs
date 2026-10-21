//! Regression tests for confirmed machine deletion.

use std::time::Duration;

use coco_core::MachineConfig;

use super::*;
use crate::machine_def::{self, tests::TempDir};
use crate::manager::{MachineEntry, suspend_state_path};
#[cfg(unix)]
use crate::save_state::tests::ReadOnly;

const TEST_SLUG: &str = "running-delete";
const LIVE_RUNTIME_SECS: u64 = 37;

const DISK_FILE: &str = "disk0.dsk";
const DISK_BYTES: &[u8] = b"deleted machine's files";

fn test_def() -> machine_def::MachineDef {
    machine_def::MachineDef::from_config(
        "Running Delete Test".to_string(),
        None,
        &MachineConfig::default(),
    )
}

fn test_manager(machines_dir: &std::path::Path) -> ManagerApp {
    ManagerApp::new(
        None,
        Some(machines_dir.to_path_buf()),
        None,
        vec![MachineEntry::new(TEST_SLUG.to_string(), test_def())],
        None,
    )
}

/// A manager over a saved [`TEST_SLUG`] whose artifact dir under
/// `artifacts_root` holds a disk image and a suspend state.
fn manager_with_artifacts(
    machines_dir: &std::path::Path,
    artifacts_root: &std::path::Path,
) -> ManagerApp {
    machine_def::save(machines_dir, TEST_SLUG, &test_def()).expect("save definition");
    let artifact_dir = artifacts_root.join(TEST_SLUG);
    fs::create_dir_all(&artifact_dir).expect("create artifact dir");
    fs::write(artifact_dir.join(DISK_FILE), DISK_BYTES).expect("write disk");
    fs::write(suspend_state_path(artifacts_root, TEST_SLUG), DISK_BYTES)
        .expect("write suspend state");
    ManagerApp::new(
        None,
        Some(machines_dir.to_path_buf()),
        Some(artifacts_root.to_path_buf()),
        vec![MachineEntry::new(TEST_SLUG.to_string(), test_def())],
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

#[test]
fn deleting_machine_removes_its_artifact_dir_and_definition() {
    let machines_dir = TempDir::new("delete-artifacts-defs");
    let artifacts_root = TempDir::new("delete-artifacts-root");
    let mut manager = manager_with_artifacts(machines_dir.path(), artifacts_root.path());
    assert!(manager.entries[0].suspended);

    manager.delete_machine(0).expect("delete should succeed");

    assert!(manager.entries.is_empty());
    assert!(!machine_def::def_path(machines_dir.path(), TEST_SLUG).exists());
    assert!(!artifacts_root.path().join(TEST_SLUG).exists());
    assert!(
        !machine_def::slug_on_disk(
            Some(machines_dir.path()),
            Some(artifacts_root.path()),
            TEST_SLUG
        ),
        "a new machine can take the deleted machine's slug"
    );
}

#[test]
fn deleting_machine_keeps_other_machines_artifact_dirs() {
    const OTHER_SLUG: &str = "other-machine";
    let machines_dir = TempDir::new("delete-keeps-other-defs");
    let artifacts_root = TempDir::new("delete-keeps-other-root");
    let other_disk = artifacts_root.path().join(OTHER_SLUG).join(DISK_FILE);
    fs::create_dir_all(other_disk.parent().unwrap()).expect("create other artifact dir");
    fs::write(&other_disk, DISK_BYTES).expect("write other disk");
    let mut manager = manager_with_artifacts(machines_dir.path(), artifacts_root.path());

    manager.delete_machine(0).expect("delete should succeed");

    assert_eq!(
        fs::read(&other_disk).expect("other disk survives"),
        DISK_BYTES
    );
}

#[test]
fn deleting_machine_without_artifact_dir_succeeds() {
    let machines_dir = TempDir::new("delete-no-artifacts-defs");
    let artifacts_root = TempDir::new("delete-no-artifacts-root");
    machine_def::save(machines_dir.path(), TEST_SLUG, &test_def()).expect("save definition");
    let mut manager = ManagerApp::new(
        None,
        Some(machines_dir.path().to_path_buf()),
        Some(artifacts_root.path().to_path_buf()),
        vec![MachineEntry::new(TEST_SLUG.to_string(), test_def())],
        None,
    );

    manager.delete_machine(0).expect("delete should succeed");

    assert!(manager.entries.is_empty());
    assert!(!machine_def::def_path(machines_dir.path(), TEST_SLUG).exists());
}

#[test]
fn artifact_dir_removal_refuses_slugs_outside_one_component() {
    let artifacts_root = TempDir::new("delete-bad-slug-root");
    let disk = artifacts_root.path().join(TEST_SLUG).join(DISK_FILE);
    fs::create_dir_all(disk.parent().unwrap()).expect("create artifact dir");
    fs::write(&disk, DISK_BYTES).expect("write disk");

    for slug in ["", ".", "..", "running-delete/..", "/tmp"] {
        assert!(
            remove_artifact_dir(artifacts_root.path(), slug).is_err(),
            "slug {slug:?} must be refused"
        );
    }
    assert_eq!(fs::read(&disk).expect("disk survives"), DISK_BYTES);
}

/// An artifact dir that can't be removed fails the delete before the
/// definition goes, so the machine stays listed for the modal's retry
/// instead of vanishing with its data still on disk.
#[cfg(unix)]
#[test]
fn failed_artifact_dir_removal_keeps_machine_and_definition() {
    let machines_dir = TempDir::new("delete-fail-defs");
    let artifacts_root = TempDir::new("delete-fail-root");
    let mut manager = manager_with_artifacts(machines_dir.path(), artifacts_root.path());
    let artifact_dir = artifacts_root.path().join(TEST_SLUG);

    let result = {
        let _read_only = ReadOnly::new(&artifact_dir);
        manager.delete_machine(0)
    };

    assert!(result.is_err(), "the removal failure must be reported");
    assert_eq!(manager.entries.len(), 1);
    assert!(machine_def::def_path(machines_dir.path(), TEST_SLUG).is_file());

    manager.delete_machine(0).expect("the retry should succeed");
    assert!(manager.entries.is_empty());
    assert!(!artifact_dir.exists());
}
