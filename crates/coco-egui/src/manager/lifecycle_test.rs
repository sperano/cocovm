//! `manager::lifecycle` tests for the per-VM stats this module maintains:
//! `start_vm` recording a boot count and seeding a fresh VM's live runtime
//! total from the persisted one, and `suspend_vm`/`stop_vm` folding that
//! live total back into the persisted total. Boots a real machine
//! through `crate::launch_machine`, which reads the real `coco3.rom` from the
//! cocovm XDG data directory the same way `launch_test.rs` does.

use std::fs;

use coco_core::MachineConfig;

use super::*;
use crate::machine_def::tests::TempDir;
#[cfg(unix)]
use crate::save_state::tests::ReadOnly;

/// A minimal CoCo 3 definition, mirroring `launch_test.rs`'s `base_def`.
fn base_def() -> machine_def::MachineDef {
    machine_def::MachineDef::from_config(
        "Lifecycle Test".to_string(),
        None,
        &MachineConfig::default(),
    )
}

/// A `ManagerApp` with one entry, `machines_dir`/`artifacts_root` pointed at
/// temp dirs so nothing touches the real per-user directories.
fn test_manager(
    machines_dir: &std::path::Path,
    artifacts_root: &std::path::Path,
    slug: &str,
    def: machine_def::MachineDef,
) -> ManagerApp {
    ManagerApp::new(
        None,
        Some(machines_dir.to_path_buf()),
        Some(artifacts_root.to_path_buf()),
        vec![MachineEntry::new(slug.to_string(), def)],
    )
}

#[test]
fn start_vm_increments_starts_and_persists() {
    let machines_dir = TempDir::new("lifecycle-start-machines");
    let artifacts_root = TempDir::new("lifecycle-start-artifacts");
    let mut manager = test_manager(
        machines_dir.path(),
        artifacts_root.path(),
        "lifecycle-test",
        base_def(),
    );
    assert_eq!(manager.entries[0].def.stats.starts, 0);

    manager.start_vm(0);
    assert!(
        manager.entries[0].vm.is_some(),
        "launch should succeed: {:?}",
        manager.entries[0].launch_error
    );
    assert_eq!(manager.entries[0].def.stats.starts, 1);

    let loaded = machine_def::load_all(machines_dir.path()).expect("reload should succeed");
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        loaded[0].1.stats.starts, 1,
        "the boot count must be visible on disk, not just in memory"
    );
}

/// `stop_vm` folds accumulated `total_runtime` into the persisted total; a
/// second stop with nothing newly accumulated is a no-op.
#[test]
fn stop_vm_folds_runtime_and_zero_elapsed_is_a_noop() {
    let machines_dir = TempDir::new("lifecycle-stop-machines");
    let artifacts_root = TempDir::new("lifecycle-stop-artifacts");
    let mut manager = test_manager(
        machines_dir.path(),
        artifacts_root.path(),
        "lifecycle-stop",
        base_def(),
    );

    manager.start_vm(0);
    assert!(
        manager.entries[0].vm.is_some(),
        "launch should succeed: {:?}",
        manager.entries[0].launch_error
    );
    // Deterministic accumulated time instead of sleeping for real.
    manager.entries[0].vm.as_mut().unwrap().total_runtime = std::time::Duration::from_secs(125);
    manager.stop_vm(0);
    assert_eq!(manager.entries[0].def.stats.runtime_secs, 125);

    let loaded = machine_def::load_all(machines_dir.path()).expect("reload should succeed");
    assert_eq!(loaded[0].1.stats.runtime_secs, 125);

    // Start again, accumulate nothing, then append a sentinel the
    // serializer would never produce, to catch a spurious rewrite.
    manager.start_vm(0);
    let toml_path = machines_dir.path().join("lifecycle-stop.toml");
    let mut sentinel_contents = fs::read_to_string(&toml_path).expect("file should exist");
    sentinel_contents.push_str("\n# sentinel: a zero-elapsed fold must not rewrite this file\n");
    fs::write(&toml_path, &sentinel_contents).expect("sentinel write should succeed");

    // Stop with nothing newly accumulated must not touch the total or the file at all.
    manager.stop_vm(0);
    assert_eq!(
        manager.entries[0].def.stats.runtime_secs, 125,
        "a zero-elapsed fold must not change the persisted total"
    );
    assert_eq!(
        fs::read_to_string(&toml_path).expect("file should still exist"),
        sentinel_contents,
        "a zero-elapsed fold must not touch the file at all — the sentinel would be dropped \
         by any real save (it isn't part of the schema)"
    );
}

/// `suspend_vm` folds runtime the same way `stop_vm` does, on a *successful*
/// suspend.
#[test]
fn suspend_vm_folds_runtime() {
    let machines_dir = TempDir::new("lifecycle-suspend-machines");
    let artifacts_root = TempDir::new("lifecycle-suspend-artifacts");
    let mut manager = test_manager(
        machines_dir.path(),
        artifacts_root.path(),
        "lifecycle-suspend",
        base_def(),
    );

    manager.start_vm(0);
    assert!(
        manager.entries[0].vm.is_some(),
        "launch should succeed: {:?}",
        manager.entries[0].launch_error
    );
    manager.entries[0].vm.as_mut().unwrap().total_runtime = std::time::Duration::from_secs(42);
    manager.suspend_vm(0);
    assert!(
        manager.entries[0].launch_error.is_none(),
        "suspend should succeed: {:?}",
        manager.entries[0].launch_error
    );
    assert_eq!(manager.entries[0].def.stats.runtime_secs, 42);

    let loaded = machine_def::load_all(machines_dir.path()).expect("reload should succeed");
    assert_eq!(loaded[0].1.stats.runtime_secs, 42);
}

/// `fold_runtime_into_def` is idempotent: folding twice with nothing new
/// accrued between calls must leave the def and the live field unchanged.
#[test]
fn fold_runtime_into_def_is_idempotent() {
    let machines_dir = TempDir::new("lifecycle-fold-idempotent-machines");
    let artifacts_root = TempDir::new("lifecycle-fold-idempotent-artifacts");
    let mut manager = test_manager(
        machines_dir.path(),
        artifacts_root.path(),
        "lifecycle-fold-idempotent",
        base_def(),
    );

    manager.start_vm(0);
    assert!(
        manager.entries[0].vm.is_some(),
        "launch should succeed: {:?}",
        manager.entries[0].launch_error
    );
    manager.entries[0].vm.as_mut().unwrap().total_runtime =
        std::time::Duration::from_secs_f64(105.4);

    manager.fold_runtime_into_def(0);
    assert_eq!(
        manager.entries[0].def.stats.runtime_secs, 105,
        "truncated to whole seconds"
    );

    // Append a sentinel the serializer would never produce, so a spurious
    // re-save by the no-op fold that follows is caught.
    let toml_path = machines_dir.path().join("lifecycle-fold-idempotent.toml");
    let mut sentinel_contents = fs::read_to_string(&toml_path).expect("file should exist");
    sentinel_contents.push_str("\n# sentinel: an idempotent fold must not rewrite this file\n");
    fs::write(&toml_path, &sentinel_contents).expect("sentinel write should succeed");

    // Fold again with the VM untouched: idempotent, and folding only reads the live field.
    manager.fold_runtime_into_def(0);
    assert_eq!(
        manager.entries[0].def.stats.runtime_secs, 105,
        "a second fold with nothing newly accrued must not change the total"
    );
    assert_eq!(
        fs::read_to_string(&toml_path).expect("file should still exist"),
        sentinel_contents,
        "a no-op fold must not touch the file at all"
    );
    assert_eq!(
        manager.entries[0].vm.as_ref().unwrap().total_runtime,
        std::time::Duration::from_secs_f64(105.4),
        "folding never touches the live VM's own total_runtime"
    );
}

/// `launch_machine` must seed a freshly launched VM's `total_runtime` from
/// the def's persisted `[stats].runtime_secs`, not start it back at zero.
#[test]
fn start_vm_seeds_total_runtime_from_the_persisted_total() {
    let machines_dir = TempDir::new("lifecycle-seed-machines");
    let artifacts_root = TempDir::new("lifecycle-seed-artifacts");
    let mut def = base_def();
    def.stats.runtime_secs = 4242;
    let mut manager = test_manager(
        machines_dir.path(),
        artifacts_root.path(),
        "lifecycle-seed",
        def,
    );

    manager.start_vm(0);

    assert_eq!(
        manager.entries[0]
            .vm
            .as_ref()
            .expect("launch should succeed")
            .total_runtime
            .as_secs(),
        4242,
        "the launch-time seed must carry the persisted total into the live VM"
    );
}

/// A full Stop → Start → Suspend → Resume → Stop cycle ends with
/// `starts == 1`, not 2 — Resume must never count as a fresh start.
#[test]
fn resume_does_not_add_a_second_start() {
    let machines_dir = TempDir::new("lifecycle-resume-machines");
    let artifacts_root = TempDir::new("lifecycle-resume-artifacts");
    let mut manager = test_manager(
        machines_dir.path(),
        artifacts_root.path(),
        "lifecycle-resume",
        base_def(),
    );

    manager.start_vm(0);
    assert!(
        manager.entries[0].vm.is_some(),
        "launch should succeed: {:?}",
        manager.entries[0].launch_error
    );
    manager.suspend_vm(0);
    assert!(
        manager.entries[0].suspended,
        "suspend should succeed: {:?}",
        manager.entries[0].launch_error
    );
    manager.resume_vm(0);
    assert!(
        !manager.entries[0].suspended,
        "resume should succeed: {:?}",
        manager.entries[0].launch_error
    );
    manager.stop_vm(0);

    assert_eq!(
        manager.entries[0].def.stats.starts, 1,
        "resume must not be counted as a fresh start"
    );
}

/// The cold-resume path the previous test doesn't reach: no VM object
/// forces `resume_vm` through its relaunch branch, which must not count as
/// a fresh start.
#[test]
fn cold_resume_does_not_add_a_second_start() {
    let machines_dir = TempDir::new("lifecycle-cold-resume-machines");
    let artifacts_root = TempDir::new("lifecycle-cold-resume-artifacts");
    let mut manager = test_manager(
        machines_dir.path(),
        artifacts_root.path(),
        "lifecycle-cold-resume",
        base_def(),
    );

    manager.start_vm(0);
    assert!(
        manager.entries[0].vm.is_some(),
        "launch should succeed: {:?}",
        manager.entries[0].launch_error
    );
    manager.suspend_vm(0);
    assert!(
        manager.entries[0].suspended,
        "suspend should succeed: {:?}",
        manager.entries[0].launch_error
    );

    // Simulate the window having been closed: the VM object is gone, but
    // the entry is still Suspended.
    manager.entries[0].vm = None;

    manager.resume_vm(0);
    assert!(
        manager.entries[0].vm.is_some(),
        "resume should relaunch: {:?}",
        manager.entries[0].launch_error
    );
    assert!(
        !manager.entries[0].suspended,
        "resume should succeed: {:?}",
        manager.entries[0].launch_error
    );

    assert_eq!(
        manager.entries[0].def.stats.starts, 1,
        "a cold resume (relaunch + restore) must not be counted as a fresh start"
    );
}

/// `resume_vm` must consume `suspended.ccstate` before declaring the
/// machine Running: with deletion injected to fail, the resume itself must
/// fail and leave the entry Suspended, its VM paused, and the checkpoint
/// intact — surviving even a simulated restart, until the directory is
/// writable again.
#[cfg(unix)]
#[test]
fn failed_checkpoint_cleanup_fails_the_resume_and_survives_restart() {
    let machines_dir = TempDir::new("lifecycle-cleanup-fail-machines");
    let artifacts_root = TempDir::new("lifecycle-cleanup-fail-artifacts");
    let slug = "lifecycle-cleanup-fail";
    let mut manager = test_manager(machines_dir.path(), artifacts_root.path(), slug, base_def());

    manager.start_vm(0);
    assert!(
        manager.entries[0].vm.is_some(),
        "launch should succeed: {:?}",
        manager.entries[0].launch_error
    );
    manager.suspend_vm(0);
    assert!(
        manager.entries[0].suspended,
        "suspend should succeed: {:?}",
        manager.entries[0].launch_error
    );

    let state_file = suspend_state_path(artifacts_root.path(), slug);
    let artifact_dir = artifacts_root.path().join(slug);
    {
        let _read_only = ReadOnly::new(&artifact_dir);
        manager.resume_vm(0);
    }

    let entry = &manager.entries[0];
    assert!(
        entry.suspended,
        "a resume that cannot consume the checkpoint must fail and stay Suspended"
    );
    assert!(
        !entry.is_running(),
        "the entry must not read Running while the checkpoint file persists"
    );
    assert!(
        !entry
            .vm
            .as_ref()
            .expect("a failed warm resume keeps the paused VM")
            .is_running(),
        "the VM itself must stay paused"
    );
    assert!(
        entry.launch_error.is_some(),
        "the cleanup failure must be reported"
    );
    assert!(
        state_file.is_file(),
        "the checkpoint must survive the failed resume — it is still the truth"
    );

    // Simulated restart: a fresh manager re-seeds Suspended from the still-present file.
    let (loaded_slug, loaded_def) = machine_def::load_all(machines_dir.path())
        .expect("reload should succeed")
        .remove(0);
    let mut restarted = ManagerApp::new(
        None,
        Some(machines_dir.path().to_path_buf()),
        Some(artifacts_root.path().to_path_buf()),
        vec![MachineEntry::new(loaded_slug, loaded_def)],
    );
    assert!(
        restarted.entries[0].suspended,
        "restart must re-seed Suspended from the still-present checkpoint"
    );

    restarted.resume_vm(0);
    assert!(
        !restarted.entries[0].suspended,
        "resume should succeed once the checkpoint can be consumed: {:?}",
        restarted.entries[0].launch_error
    );
    assert!(
        restarted.entries[0].is_running(),
        "the resumed machine should be Running"
    );
    assert!(
        !state_file.exists(),
        "a successful resume consumes the checkpoint"
    );
}

/// The cold-relaunch shape of the same guard: with the VM object gone, a
/// resume that restores but can't consume the checkpoint must drop the VM
/// again and stay Suspended.
#[cfg(unix)]
#[test]
fn cold_resume_with_failed_cleanup_drops_the_vm_and_stays_suspended() {
    let machines_dir = TempDir::new("lifecycle-cold-cleanup-fail-machines");
    let artifacts_root = TempDir::new("lifecycle-cold-cleanup-fail-artifacts");
    let slug = "lifecycle-cold-cleanup-fail";
    let mut manager = test_manager(machines_dir.path(), artifacts_root.path(), slug, base_def());

    manager.start_vm(0);
    assert!(
        manager.entries[0].vm.is_some(),
        "launch should succeed: {:?}",
        manager.entries[0].launch_error
    );
    manager.suspend_vm(0);
    assert!(
        manager.entries[0].suspended,
        "suspend should succeed: {:?}",
        manager.entries[0].launch_error
    );
    manager.entries[0].vm = None; // window closed while suspended

    let state_file = suspend_state_path(artifacts_root.path(), slug);
    let artifact_dir = artifacts_root.path().join(slug);
    {
        // Read stays allowed (the restore must succeed first); only the delete fails.
        let _read_only = ReadOnly::new(&artifact_dir);
        manager.resume_vm(0);
    }

    let entry = &manager.entries[0];
    assert!(
        entry.suspended,
        "a cold resume that cannot consume the checkpoint must stay Suspended"
    );
    assert!(
        entry.vm.is_none(),
        "the just-restored VM must be dropped again, back to window-closed Suspended"
    );
    assert!(
        entry.launch_error.is_some(),
        "the cleanup failure must be reported"
    );
    assert!(
        state_file.is_file(),
        "the checkpoint must survive the failed resume"
    );
}

/// Stop on a Suspended machine discards the checkpoint; when the discard
/// itself fails, the entry must stay Suspended with the failure surfaced,
/// keeping its preview until a retried Stop succeeds.
#[cfg(unix)]
#[test]
fn stop_that_cannot_discard_the_checkpoint_stays_suspended() {
    let machines_dir = TempDir::new("lifecycle-stop-discard-fail-machines");
    let artifacts_root = TempDir::new("lifecycle-stop-discard-fail-artifacts");
    let slug = "lifecycle-stop-discard-fail";
    let mut manager = test_manager(machines_dir.path(), artifacts_root.path(), slug, base_def());

    manager.start_vm(0);
    assert!(
        manager.entries[0].vm.is_some(),
        "launch should succeed: {:?}",
        manager.entries[0].launch_error
    );
    manager.suspend_vm(0);
    assert!(
        manager.entries[0].suspended,
        "suspend should succeed: {:?}",
        manager.entries[0].launch_error
    );

    let state_file = suspend_state_path(artifacts_root.path(), slug);
    let artifact_dir = artifacts_root.path().join(slug);
    {
        let _read_only = ReadOnly::new(&artifact_dir);
        manager.stop_vm(0);
    }

    let thumbnail_file = artifact_dir.join("thumbnail.png");
    let entry = &manager.entries[0];
    assert!(
        entry.suspended,
        "a stop that cannot discard the checkpoint must leave the entry Suspended"
    );
    assert!(entry.vm.is_none(), "stop still drops the VM object");
    assert!(
        entry.launch_error.is_some(),
        "the discard failure must be reported"
    );
    assert!(
        state_file.is_file(),
        "the checkpoint must survive the failed discard"
    );
    assert!(
        thumbnail_file.is_file(),
        "a row that stays Suspended keeps its suspend-time preview"
    );

    // With the directory writable again the same Stop powers off cleanly.
    manager.stop_vm(0);
    let entry = &manager.entries[0];
    assert!(!entry.suspended, "the retried stop should power off");
    assert!(
        entry.launch_error.is_none(),
        "a fully successful stop clears the error, like the other transports"
    );
    assert!(
        !state_file.exists() && !thumbnail_file.exists(),
        "powering off discards both artifact files"
    );
}
