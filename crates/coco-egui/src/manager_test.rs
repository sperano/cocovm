use coco_core::MachineConfig;
use eframe::App;

use super::*;
use crate::machine_def::tests::TempDir;

impl ManagerApp {
    /// The detail pane's current draft name, when one is shown — `ui_tests.rs`
    /// checks that selecting a row seeds the right draft without depending on
    /// how `egui::TextEdit` exposes its value to the accessibility tree.
    pub(crate) fn detail_name(&self) -> Option<&str> {
        self.edit.as_ref().map(|e| e.name.as_str())
    }

    /// Mutable access to the detail pane's edit form — `ui_tests.rs` seeds
    /// ROM Pak picks directly, since the "ROM Pak…" combo items open native
    /// file dialogs a headless harness cannot drive.
    pub(crate) fn edit_form_mut(&mut self) -> Option<&mut new_vm::MachineForm> {
        self.edit.as_mut().map(|e| &mut e.form)
    }
}

/// A tiny non-black RGBA frame (2×2, opaque red).
const RED_FRAME: [u8; 16] = [
    0xFF, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF,
];
/// Same geometry, uniformly black — the frame [`write_thumbnail_png`]'s
/// blank-screen heuristic guards against.
const BLACK_FRAME: [u8; 16] = [0, 0, 0, 0xFF, 0, 0, 0, 0xFF, 0, 0, 0, 0xFF, 0, 0, 0, 0xFF];

#[test]
fn write_thumbnail_png_round_trips_and_leaves_no_tmp() {
    let dir = TempDir::new("thumb-roundtrip");
    write_thumbnail_png(dir.path(), &RED_FRAME, 2, 2).expect("write succeeds");

    assert!(!dir.path().join(format!("{THUMBNAIL_FILE}.tmp")).exists());
    let image = image::open(dir.path().join(THUMBNAIL_FILE)).expect("decodable PNG");
    assert_eq!((image.width(), image.height()), (2, 2));
}

#[test]
fn uniformly_black_frame_keeps_the_previous_thumbnail() {
    let dir = TempDir::new("thumb-black-skip");
    write_thumbnail_png(dir.path(), &RED_FRAME, 2, 2).unwrap();
    let before = fs::read(dir.path().join(THUMBNAIL_FILE)).unwrap();

    write_thumbnail_png(dir.path(), &BLACK_FRAME, 2, 2).unwrap();
    let after = fs::read(dir.path().join(THUMBNAIL_FILE)).unwrap();
    assert_eq!(
        before, after,
        "a blank screen must not clobber a useful preview"
    );
}

#[test]
fn black_frame_is_still_written_when_no_previous_thumbnail_exists() {
    let dir = TempDir::new("thumb-black-first");
    write_thumbnail_png(dir.path(), &BLACK_FRAME, 2, 2).unwrap();
    assert!(dir.path().join(THUMBNAIL_FILE).exists());
}

/// Quit is the power switch for every running VM at once
/// (`ManagerApp::on_exit`'s doc): each live VM's session runtime must fold
/// into its persisted total the same way `Self::stop_vm` does, both in
/// memory and on disk. Boots a real machine via `crate::launch_machine`,
/// same as `manager::lifecycle`'s own tests.
#[test]
fn on_exit_folds_live_runtime_into_the_persisted_total() {
    let machines_dir = TempDir::new("manager-on-exit-machines");
    let artifacts_root = TempDir::new("manager-on-exit-artifacts");
    let def = machine_def::MachineDef::from_config(
        "On Exit Test".to_string(),
        None,
        &MachineConfig::default(),
    );
    let mut manager = ManagerApp::new(
        None,
        Some(machines_dir.path().to_path_buf()),
        Some(artifacts_root.path().to_path_buf()),
        vec![MachineEntry::new("on-exit-test".to_string(), def)],
    );

    manager.start_vm(0);
    assert!(
        manager.entries[0].vm.is_some(),
        "launch should succeed: {:?}",
        manager.entries[0].launch_error
    );
    manager.entries[0].vm.as_mut().unwrap().total_runtime = std::time::Duration::from_secs(77);

    manager.on_exit(None);

    assert_eq!(manager.entries[0].def.stats.runtime_secs, 77);
    let loaded = machine_def::load_all(machines_dir.path()).expect("reload should succeed");
    assert_eq!(loaded[0].1.stats.runtime_secs, 77);
}
