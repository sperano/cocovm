//! `manager::lifecycle` tests for the per-VM stats this module maintains:
//! `start_vm` recording a boot count and seeding a fresh VM's live runtime
//! total from the persisted one, and `suspend_vm`/`stop_vm` folding that
//! live total back into the persisted total. Boots a real machine
//! via `crate::launch_machine`, which reads the real `roms/coco3.rom`
//! (git-ignored, local-only) the same way `launch_test.rs` does — no special
//! setup needed, since ROM lookup resolves through `CARGO_MANIFEST_DIR` at
//! compile time, not a runtime-relative path.

use std::fs;

use coco_core::MachineConfig;

use super::*;
use crate::machine_def::tests::TempDir;

/// A minimal CoCo 3 definition, mirroring `launch_test.rs`'s `base_def`.
fn base_def() -> machine_def::MachineDef {
    machine_def::MachineDef::from_config(
        "Lifecycle Test".to_string(),
        None,
        &MachineConfig::default(),
    )
}

/// A `ManagerApp` with one entry (`slug`/`def`), `machines_dir`/
/// `artifacts_root` pointed at temp dirs so nothing touches the real
/// per-user directories.
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

/// `stop_vm` folds whatever `total_runtime` had accumulated into the
/// persisted total; a second stop with nothing newly accumulated (zero
/// elapsed) is a no-op that leaves the total untouched.
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
    // Deterministic accumulated time instead of sleeping for real —
    // `CocoApp::total_runtime` is `pub(crate)`.
    manager.entries[0].vm.as_mut().unwrap().total_runtime = std::time::Duration::from_secs(125);
    manager.stop_vm(0);
    assert_eq!(manager.entries[0].def.stats.runtime_secs, 125);

    let loaded = machine_def::load_all(machines_dir.path()).expect("reload should succeed");
    assert_eq!(loaded[0].1.stats.runtime_secs, 125);

    // Start again (this legitimately rewrites the file — a fresh start
    // bumps `starts`), accumulate nothing this session, then poke a
    // sentinel comment onto the end of the on-disk file — something
    // `machine_def::save`'s serializer would never itself produce, so a
    // spurious rewrite is caught even where the round-tripped *content*
    // would otherwise match byte-for-byte.
    manager.start_vm(0);
    let toml_path = machines_dir.path().join("lifecycle-stop.toml");
    let mut sentinel_contents = fs::read_to_string(&toml_path).expect("file should exist");
    sentinel_contents.push_str("\n# sentinel: a zero-elapsed fold must not rewrite this file\n");
    fs::write(&toml_path, &sentinel_contents).expect("sentinel write should succeed");

    // Stop with nothing newly accumulated: folding a zero-elapsed session
    // must not touch the total, or the file at all.
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

/// `fold_runtime_into_def` is an idempotent assignment from the live VM's
/// `total_runtime` (truncated to whole seconds) into the persisted total —
/// folding twice with nothing new accrued between the calls must leave the
/// def, and the live field itself, exactly as they were after the first
/// fold (`fold_runtime_into_def`'s doc).
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

    // Poke a sentinel onto the on-disk file — something `machine_def::save`'s
    // serializer would never itself produce — so a spurious re-save by the
    // second, no-op fold below is caught even where the round-tripped
    // *content* would otherwise match byte-for-byte.
    let toml_path = machines_dir.path().join("lifecycle-fold-idempotent.toml");
    let mut sentinel_contents = fs::read_to_string(&toml_path).expect("file should exist");
    sentinel_contents.push_str("\n# sentinel: an idempotent fold must not rewrite this file\n");
    fs::write(&toml_path, &sentinel_contents).expect("sentinel write should succeed");

    // Fold again with the VM untouched: idempotent, and folding never
    // touches the live field, only reads it.
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

/// The review's finding this test guards: `launch_machine` must seed a
/// freshly launched VM's `total_runtime` from the def's persisted
/// `[stats].runtime_secs`, so a machine that already has accrued time shows
/// it immediately rather than starting back at zero.
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
/// `starts == 1`, not 2 — Resume must never count as a fresh start. A
/// positive assertion about what the cycle produces, not an absence test
/// (project convention).
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

/// The cold-resume path `resume_does_not_add_a_second_start` doesn't reach:
/// the VM object gone (window closed, or the app quit and relaunched —
/// `entry.suspended` rehydrates from the on-disk `.ccstate` either way)
/// forces `resume_vm` down its relaunch branch, which must go through
/// `launch_vm` and not `start_vm` — a regression here means a plain
/// suspend/resume cycle silently inflates the boot count.
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

    // Simulate the window having been closed (or the app quit and
    // relaunched): the VM object is gone, but the entry is still Suspended.
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
