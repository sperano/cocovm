use std::path::Path;

use super::*;
use crate::machine_def::tests::TempDir;
use crate::save_state::tests::ReadOnly;
use crate::{ROMSource, dev_roms_dir};
use coco_core::{MachineConfig, fdc};

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

/// A booted machine with a dirty floppy mounted in drive 0 (not yet
/// unwritable — the caller wraps [`ReadOnly`] around `disk_path` separately,
/// so `insert_disk`'s own write-back of "whatever was in the drive first"
/// runs while the file is still writable).
fn vm_with_dirty_disk(disk_path: &Path) -> Box<CocoApp> {
    let roms_dir = dev_roms_dir();
    let rom_path = roms_dir.join("coco3.rom");
    let rom = std::fs::read(&rom_path)
        .expect("roms/coco3.rom is required (git-ignored, local-only)")
        .into_boxed_slice();
    let mut vm = CocoApp::new(
        MachineConfig::default(),
        rom,
        ROMSource::File(rom_path),
        None,
        [None, None],
        [None, None],
        std::array::from_fn(|_| None),
        false,
        false,
        false,
    );
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

/// Suspend (`ManagerApp::suspend_vm`) calls `CocoApp::save_state_to`, which
/// now fails the whole save when a dirty disk can't be written back
/// (Vikunja #175 — it used to flush unconditionally, silently discard the
/// unsaved bytes, and mark the entry Suspended anyway). A failed suspend
/// must leave the entry exactly where it started: not suspended, its VM
/// alive and still running, the error surfaced in `launch_error`, and no
/// `suspended.ccstate` written — mirrors
/// `save_state_test.rs`'s `save_state_to_fails_and_leaves_disk_dirty_when_write_back_fails`
/// one level up, through the manager's own suspend path instead of calling
/// `save_state_to` directly.
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
