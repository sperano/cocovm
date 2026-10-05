//! `stop_vm`/`suspend_vm` actions against real VMs launched through
//! `ManagerApp::start_vm`, with temp `machines_dir`/`artifacts_root`
//! directories so suspend has somewhere to write its state. The dirty-media
//! cases reuse `save_state`'s FD-502 fixtures.

use std::path::{Path, PathBuf};

use super::*;
use crate::machine_def::{self, tests::TempDir};
use crate::manager::{MachineEntry, suspend_state_path};
use crate::save_state::tests::{DIRTY_BYTE, mount_and_dirty, write_one_track_disk};
use crate::{CocoApp, MachineConfig};

const SLUG: &str = "lifecycle-action";

/// A manager whose one Powered Off entry stores its definition and
/// artifacts under the given temp directories.
fn manager_in(machines_dir: &Path, artifacts_root: &Path) -> ManagerApp {
    let def =
        machine_def::MachineDef::from_config(SLUG.to_string(), None, &MachineConfig::default());
    ManagerApp::new(
        None,
        Some(machines_dir.to_path_buf()),
        Some(artifacts_root.to_path_buf()),
        vec![MachineEntry::new(SLUG.to_string(), def)],
        None,
    )
}

/// The temp directories one test's manager lives in; dropped (and deleted)
/// with the test.
struct Dirs {
    machines: TempDir,
    artifacts: TempDir,
}

impl Dirs {
    fn new(tag: &str) -> Self {
        Self {
            machines: TempDir::new(&format!("{tag}-machines")),
            artifacts: TempDir::new(&format!("{tag}-artifacts")),
        }
    }

    /// A manager with its entry started.
    fn running_manager(&self) -> ManagerApp {
        let mut manager = manager_in(self.machines.path(), self.artifacts.path());
        manager
            .start_vm_action(&slug())
            .expect("launching the test machine");
        manager
    }

    /// A one-track floppy fixture inside the artifacts directory.
    fn disk(&self) -> PathBuf {
        let path = self.artifacts.path().join("dirty.dsk");
        write_one_track_disk(&path);
        path
    }

    fn state_file(&self) -> PathBuf {
        suspend_state_path(self.artifacts.path(), SLUG)
    }
}

fn slug() -> Option<String> {
    Some(SLUG.to_string())
}

fn live_vm(manager: &mut ManagerApp) -> &mut CocoApp {
    manager.entries[0].vm.as_mut().expect("the VM is live")
}

fn first_byte(path: &Path) -> u8 {
    std::fs::read(path).expect("read the disk fixture")[0]
}

#[test]
fn stop_writes_dirty_media_back_and_powers_off() {
    let dirs = Dirs::new("stop-action-flush");
    let mut manager = dirs.running_manager();
    let disk = dirs.disk();
    mount_and_dirty(live_vm(&mut manager), 0, &disk);

    let reply = manager.stop_vm_action(&slug()).expect("stop succeeds");

    assert_eq!(reply, Reply::Done);
    assert!(!manager.entries[0].is_alive());
    assert_eq!(first_byte(&disk), DIRTY_BYTE, "the write-back must land");
}

#[test]
fn stop_is_a_no_op_on_a_powered_off_vm() {
    let dirs = Dirs::new("stop-action-off");
    let mut manager = manager_in(dirs.machines.path(), dirs.artifacts.path());

    assert_eq!(manager.stop_vm_action(&slug()), Ok(Reply::Done));
    assert!(!manager.entries[0].is_alive());
}

#[test]
fn stop_discards_a_suspended_vms_state() {
    let dirs = Dirs::new("stop-action-suspended");
    let mut manager = dirs.running_manager();
    manager
        .suspend_vm_action(&slug())
        .expect("suspend succeeds");
    assert!(dirs.state_file().exists());

    manager.stop_vm_action(&slug()).expect("stop succeeds");

    assert!(!manager.entries[0].is_alive());
    assert!(!dirs.state_file().exists(), "Stop discards the saved state");
}

#[test]
fn suspend_saves_state_and_leaves_the_vm_suspended() {
    let dirs = Dirs::new("suspend-action");
    let mut manager = dirs.running_manager();
    let disk = dirs.disk();
    mount_and_dirty(live_vm(&mut manager), 0, &disk);

    let reply = manager
        .suspend_vm_action(&slug())
        .expect("suspend succeeds");

    assert_eq!(reply, Reply::Done);
    assert!(manager.entries[0].suspended);
    assert!(dirs.state_file().exists());
    assert_eq!(first_byte(&disk), DIRTY_BYTE, "the write-back must land");
}

#[test]
fn suspend_is_a_no_op_when_already_suspended() {
    let dirs = Dirs::new("suspend-action-twice");
    let mut manager = dirs.running_manager();
    manager.suspend_vm_action(&slug()).expect("first suspend");

    assert_eq!(manager.suspend_vm_action(&slug()), Ok(Reply::Done));
    assert!(manager.entries[0].suspended);
}

#[test]
fn suspend_rejects_a_powered_off_vm() {
    let dirs = Dirs::new("suspend-action-off");
    let mut manager = manager_in(dirs.machines.path(), dirs.artifacts.path());

    let error = manager
        .suspend_vm_action(&slug())
        .expect_err("nothing to suspend");

    assert!(error.contains("powered off"), "{error}");
    assert!(!manager.entries[0].is_alive());
}

#[cfg(unix)]
mod write_back_failures {
    use super::*;
    use crate::save_state::tests::ReadOnly;

    #[test]
    fn stop_powers_off_and_reports_a_failed_write_back() {
        let dirs = Dirs::new("stop-action-read-only");
        let mut manager = dirs.running_manager();
        let disk = dirs.disk();
        mount_and_dirty(live_vm(&mut manager), 0, &disk);
        let _read_only = ReadOnly::new(&disk);

        let error = manager
            .stop_vm_action(&slug())
            .expect_err("the write-back fails");

        assert!(error.contains(&disk.display().to_string()), "{error}");
        assert!(error.ends_with("is now powered_off."), "{error}");
        assert!(!manager.entries[0].is_alive(), "Stop powers off regardless");
        assert!(
            manager.entries[0].launch_error.is_some(),
            "the manager row keeps showing the failure"
        );
    }

    #[test]
    fn suspend_keeps_the_vm_running_when_a_write_back_fails() {
        let dirs = Dirs::new("suspend-action-read-only");
        let mut manager = dirs.running_manager();
        let disk = dirs.disk();
        mount_and_dirty(live_vm(&mut manager), 0, &disk);
        let _read_only = ReadOnly::new(&disk);

        let error = manager
            .suspend_vm_action(&slug())
            .expect_err("the write-back fails");

        assert!(error.contains(&disk.display().to_string()), "{error}");
        assert!(error.ends_with("is now running."), "{error}");
        assert!(manager.entries[0].is_running());
        assert!(!dirs.state_file().exists(), "no state is saved");
    }
}
