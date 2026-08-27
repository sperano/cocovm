use std::path::Path;

use super::*;
use crate::machine_def::tests::TempDir;
use crate::rom_load::COCO3_ROM_FILE;
use crate::save_state::tests::ReadOnly;
use crate::{AppParams, ROMSource, installed_roms_dir};
use coco_core::{MachineConfig, fdc};
use eframe::App;

impl ManagerApp {
    /// The detail pane's current draft name, when one is shown.
    pub(crate) fn detail_name(&self) -> Option<&str> {
        self.edit.as_ref().map(|e| e.name.as_str())
    }

    /// Mutable access to the detail pane's edit form — needed because the
    /// "ROM Pak…" combo opens native file dialogs a headless test harness
    /// can't drive.
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

/// Quit folds every running VM's runtime into its persisted total, same as
/// Stop, both in memory and on disk.
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

/// A name committed while its VM is running leaves slug migration pending.
/// Quit must flush and drop the VM before moving both slug-keyed filesystem
/// objects, or the rename is lost when the in-memory flag disappears.
#[test]
fn on_exit_migrates_a_rename_deferred_while_running() {
    const OLD_SLUG: &str = "before-rename";
    const NEW_SLUG: &str = "after-rename";
    const ARTIFACT_FILE: &str = "disk.img";

    let machines_dir = TempDir::new("manager-on-exit-rename-machines");
    let artifacts_root = TempDir::new("manager-on-exit-rename-artifacts");
    let def = machine_def::MachineDef::from_config(
        "Before Rename".to_string(),
        None,
        &MachineConfig::default(),
    );
    machine_def::save(machines_dir.path(), OLD_SLUG, &def).expect("seed definition");
    let old_artifacts = artifacts_root.path().join(OLD_SLUG);
    fs::create_dir(&old_artifacts).expect("seed artifact directory");
    fs::write(old_artifacts.join(ARTIFACT_FILE), b"artifact").expect("seed artifact");
    let mut manager = ManagerApp::new(
        None,
        Some(machines_dir.path().to_path_buf()),
        Some(artifacts_root.path().to_path_buf()),
        vec![MachineEntry::new(OLD_SLUG.to_string(), def)],
    );

    manager.start_vm(0);
    assert!(manager.entries[0].vm.is_some(), "VM should launch");
    manager.entries[0].def.name = "After Rename".to_string();
    machine_def::save(machines_dir.path(), OLD_SLUG, &manager.entries[0].def)
        .expect("commit renamed definition under old slug");
    manager.entries[0].rename_pending = true;

    manager.on_exit(None);

    assert_eq!(manager.entries[0].slug, NEW_SLUG);
    assert!(manager.entries[0].vm.is_none());
    assert!(
        !machines_dir
            .path()
            .join(format!("{OLD_SLUG}.toml"))
            .exists()
    );
    assert!(
        machines_dir
            .path()
            .join(format!("{NEW_SLUG}.toml"))
            .is_file()
    );
    assert!(!artifacts_root.path().join(OLD_SLUG).exists());
    assert_eq!(
        fs::read(artifacts_root.path().join(NEW_SLUG).join(ARTIFACT_FILE)).unwrap(),
        b"artifact"
    );
}

/// A booted machine with a dirty floppy mounted in drive 0. The caller wraps
/// [`ReadOnly`] around `disk_path` separately, so the initial write-back
/// runs while the file is still writable.
fn vm_with_dirty_disk(disk_path: &Path) -> Box<CocoApp> {
    let roms_dir = installed_roms_dir();
    let rom_path = roms_dir.join(COCO3_ROM_FILE);
    let rom = std::fs::read(&rom_path)
        .expect("installed coco3.rom is required (ensure_assets)")
        .into_boxed_slice();
    let mut vm = CocoApp::new(
        MachineConfig::default(),
        rom,
        ROMSource::File(rom_path),
        AppParams::default(),
    );
    vm.insert_disk_controller()
        .unwrap_or_else(|e| panic!("test fixture FD-502 install: {e}"));
    vm.insert_disk(0, disk_path.to_path_buf());
    assert!(
        vm.cart_error.is_none(),
        "mounting the disk: {:?}",
        vm.cart_error
    );
    vm.machine
        .bus
        .cart
        .as_disk_cart()
        .expect("FD-502 just inserted")
        .disk_mut(0)
        .expect("drive 0 mounted")
        .write_byte(0, 0xAA);
    Box::new(vm)
}

/// A failed suspend (dirty disk that can't be written back) must leave the
/// entry unchanged: not suspended, VM still running, error surfaced, and no
/// `suspended.ccstate` written.
#[test]
fn suspend_fails_and_leaves_the_machine_running_when_disk_write_back_fails() {
    let artifacts = TempDir::new("suspend-flush-failure");
    let disk_path = artifacts.path().join("dirty.dsk");
    let sector_size = 128usize << fdc::DEFAULT_SECTOR_SIZE_CODE;
    let one_track = fdc::DEFAULT_SECTORS_PER_TRACK * sector_size * fdc::DEFAULT_SIDES;
    std::fs::write(&disk_path, vec![0u8; one_track]).expect("write disk fixture");

    let vm = vm_with_dirty_disk(&disk_path);
    let def = machine_def::MachineDef::from_config(
        "Dirty Disk".to_string(),
        None,
        &MachineConfig::default(),
    );
    let mut entry = MachineEntry::new("dirty-disk".to_string(), def);
    entry.vm = Some(vm);
    let mut app = ManagerApp::new(
        None,
        None,
        Some(artifacts.path().to_path_buf()),
        vec![entry],
    );

    let state_file = suspend_state_path(artifacts.path(), "dirty-disk");
    {
        let _ro = ReadOnly::new(&disk_path);
        app.suspend_vm(0);
    }

    let entry = &app.entries[0];
    assert!(
        !entry.suspended,
        "a failed suspend must not mark the entry Suspended"
    );
    let vm = entry
        .vm
        .as_ref()
        .expect("a failed suspend must not drop the VM");
    assert!(
        vm.is_running(),
        "a failed suspend must leave the machine Running"
    );
    assert!(
        entry.launch_error.is_some(),
        "the flush failure must be reported"
    );
    assert!(
        !state_file.exists(),
        "no suspended.ccstate must be written on a failed suspend"
    );
}
