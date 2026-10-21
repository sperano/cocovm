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
fn manager_with_artifacts(machines_dir: &Path, artifacts_root: &Path) -> ManagerApp {
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
fn artifact_dir_refuses_slugs_other_than_one_file_name() {
    let root = Path::new("artifacts");
    for slug in [
        "",
        ".",
        "..",
        "running-delete/..",
        "running-delete/.",
        "/tmp",
    ] {
        assert!(
            artifact_dir(root, slug).is_err(),
            "slug {slug:?} must be refused"
        );
    }
    assert_eq!(artifact_dir(root, TEST_SLUG), Ok(root.join(TEST_SLUG)));
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

/// Slug of a machine that mounts a disk from [`TEST_SLUG`]'s data folder.
const BORROWER_SLUG: &str = "borrower";

/// [`manager_with_artifacts`] plus a [`BORROWER_SLUG`] machine whose drive 0
/// is `disk0`, a path naming [`TEST_SLUG`]'s disk.
fn manager_with_borrower(machines_dir: &Path, artifacts_root: &Path, disk0: String) -> ManagerApp {
    let mut manager = manager_with_artifacts(machines_dir, artifacts_root);
    let mut borrower = test_def();
    borrower.name = "Borrower".to_string();
    borrower.media.disk0 = Some(disk0);
    machine_def::save(machines_dir, BORROWER_SLUG, &borrower).expect("save borrower");
    manager
        .entries
        .push(MachineEntry::new(BORROWER_SLUG.to_string(), borrower));
    manager
}

/// Deleting a machine whose data folder holds a disk another machine
/// mounts, by absolute path or by a relative one climbing out with `..`, is
/// refused before anything is removed.
#[test]
fn deleting_machine_whose_files_another_machine_uses_is_refused() {
    for absolute in [true, false] {
        let label = if absolute { "absolute" } else { "relative" };
        let machines_dir = TempDir::new("delete-shared-defs");
        let artifacts_root = TempDir::new("delete-shared-root");
        let shared_disk = artifacts_root.path().join(TEST_SLUG).join(DISK_FILE);
        let disk0 = if absolute {
            shared_disk.display().to_string()
        } else {
            format!("../{TEST_SLUG}/./{DISK_FILE}")
        };
        let mut manager = manager_with_borrower(machines_dir.path(), artifacts_root.path(), disk0);

        let error = manager
            .delete_machine(0)
            .expect_err(&format!("{label}: the delete must be refused"));

        assert!(error.contains("“Borrower”"), "{label}: {error}");
        assert_eq!(manager.entries.len(), 2, "{label}");
        assert!(machine_def::def_path(machines_dir.path(), TEST_SLUG).is_file());
        assert_eq!(fs::read(&shared_disk).expect("disk survives"), DISK_BYTES);
    }
}

/// A file another machine names only counts when it is inside the data
/// folder: a sibling folder sharing the slug as a name prefix doesn't.
#[test]
fn deleting_machine_ignores_paths_beside_its_data_folder() {
    let machines_dir = TempDir::new("delete-prefix-defs");
    let artifacts_root = TempDir::new("delete-prefix-root");
    let beside = artifacts_root
        .path()
        .join(format!("{TEST_SLUG}-2"))
        .join(DISK_FILE);
    let mut manager = manager_with_borrower(
        machines_dir.path(),
        artifacts_root.path(),
        beside.display().to_string(),
    );

    manager.delete_machine(0).expect("delete should succeed");

    assert_eq!(manager.entries.len(), 1);
    assert!(!artifacts_root.path().join(TEST_SLUG).exists());
}

/// Deleting both machines together goes ahead: the one using the other's
/// files is going too.
#[test]
fn deleting_machine_with_its_borrower_succeeds() {
    let machines_dir = TempDir::new("delete-batch-defs");
    let artifacts_root = TempDir::new("delete-batch-root");
    let shared_disk = artifacts_root.path().join(TEST_SLUG).join(DISK_FILE);
    let mut manager = manager_with_borrower(
        machines_dir.path(),
        artifacts_root.path(),
        shared_disk.display().to_string(),
    );
    manager.pending_delete = vec![TEST_SLUG.to_string(), BORROWER_SLUG.to_string()];

    manager.commit_pending_delete();

    assert_eq!(manager.delete_error, None);
    assert!(manager.entries.is_empty());
    assert!(manager.pending_delete.is_empty());
    assert!(!artifacts_root.path().join(TEST_SLUG).exists());
}

/// A running machine counts the media its VM has mounted now, not only what
/// its definition names: a disk inserted at runtime from the other machine's
/// data folder refuses the delete too.
#[test]
fn deleting_machine_whose_files_a_running_machine_mounted_is_refused() {
    let machines_dir = TempDir::new("delete-live-defs");
    let artifacts_root = TempDir::new("delete-live-root");
    let shared_disk = artifacts_root.path().join(TEST_SLUG).join(DISK_FILE);
    let mut manager = manager_with_artifacts(machines_dir.path(), artifacts_root.path());
    let mut borrower = test_def();
    borrower.name = "Borrower".to_string();
    manager
        .entries
        .push(MachineEntry::new(BORROWER_SLUG.to_string(), borrower));
    manager.start_vm(1);
    assert!(
        manager.entries[1].vm.is_some(),
        "launch should succeed: {:?}",
        manager.entries[1].launch_error
    );
    manager.entries[1].vm.as_mut().unwrap().disk_paths[1] = Some(shared_disk.clone());

    let error = manager
        .delete_machine(0)
        .expect_err("the delete must be refused");

    assert!(error.contains("“Borrower”"), "{error}");
    assert_eq!(manager.entries.len(), 2);
    assert_eq!(fs::read(&shared_disk).expect("disk survives"), DISK_BYTES);
}

/// Slug of a machine outside the batch that mounts a disk from
/// [`BORROWER_SLUG`]'s data folder.
const OUTSIDER_SLUG: &str = "outsider";

/// Writes [`DISK_FILE`] into `slug`'s data folder under `artifacts_root`
/// and returns its path.
fn write_disk(artifacts_root: &Path, slug: &str) -> PathBuf {
    let disk = artifacts_root.join(slug).join(DISK_FILE);
    fs::create_dir_all(disk.parent().unwrap()).expect("create artifact dir");
    fs::write(&disk, DISK_BYTES).expect("write disk");
    disk
}

/// A batch deletes a borrower before the machine whose files it uses, so a
/// borrower that fails to go stops the batch with those files intact.
#[cfg(unix)]
#[test]
fn failed_borrower_delete_keeps_the_folder_it_uses() {
    let machines_dir = TempDir::new("delete-order-defs");
    let artifacts_root = TempDir::new("delete-order-root");
    let shared_disk = artifacts_root.path().join(TEST_SLUG).join(DISK_FILE);
    let mut manager = manager_with_borrower(
        machines_dir.path(),
        artifacts_root.path(),
        shared_disk.display().to_string(),
    );
    let borrower_dir = write_disk(artifacts_root.path(), BORROWER_SLUG)
        .parent()
        .unwrap()
        .to_path_buf();
    manager.pending_delete = vec![TEST_SLUG.to_string(), BORROWER_SLUG.to_string()];

    {
        let _read_only = ReadOnly::new(&borrower_dir);
        manager.commit_pending_delete();
    }

    assert!(
        manager.delete_error.is_some(),
        "the failure must be reported"
    );
    assert_eq!(manager.entries.len(), 2);
    assert_eq!(fs::read(&shared_disk).expect("disk survives"), DISK_BYTES);
    assert_eq!(
        manager.pending_delete,
        [BORROWER_SLUG.to_string(), TEST_SLUG.to_string()]
    );
}

/// When two batch members use each other's folders, no order protects
/// both, so a member whose folder a machine outside the batch uses refuses
/// the whole batch before anything is removed.
#[test]
fn batch_with_a_folder_used_outside_it_deletes_nothing() {
    let machines_dir = TempDir::new("delete-cycle-defs");
    let artifacts_root = TempDir::new("delete-cycle-root");
    let shared_disk = artifacts_root.path().join(TEST_SLUG).join(DISK_FILE);
    let mut manager = manager_with_borrower(
        machines_dir.path(),
        artifacts_root.path(),
        shared_disk.display().to_string(),
    );
    let borrower_disk = write_disk(artifacts_root.path(), BORROWER_SLUG);
    manager.entries[0].def.media.disk1 = Some(borrower_disk.display().to_string());
    let mut outsider = test_def();
    outsider.name = "Outsider".to_string();
    outsider.media.disk0 = Some(borrower_disk.display().to_string());
    manager
        .entries
        .push(MachineEntry::new(OUTSIDER_SLUG.to_string(), outsider));
    manager.pending_delete = vec![TEST_SLUG.to_string(), BORROWER_SLUG.to_string()];

    manager.commit_pending_delete();

    let error = manager
        .delete_error
        .as_deref()
        .expect("the batch is refused");
    assert!(error.contains("“Outsider”"), "{error}");
    assert_eq!(manager.entries.len(), 3);
    assert_eq!(fs::read(&shared_disk).expect("disk survives"), DISK_BYTES);
    assert_eq!(fs::read(&borrower_disk).expect("disk survives"), DISK_BYTES);
}
