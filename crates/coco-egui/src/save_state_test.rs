use std::path::Path;

use super::*;
use crate::rom_load::COCO3_ROM_FILE;
use crate::{ROMSource, disk_basic_rom_path, installed_roms_dir};
use coco_core::cassette::test_support::{SPINUP_BURN_CYCLES, record_bytes_fsk, tape_block};
use coco_core::snapshot;
use coco_core::{MachineConfig, fdc};

/// Leader byte (Service Manual §5.10, `cassette-verified-facts`) — a plain
/// unstructured tape prefix, matching `ui_tests::vm_window_tape`'s own use of
/// it to get the demodulator to lock on before a framed block.
const LEADER: u8 = 0x55;

/// Add back the owner-write bit on `path` if it's missing — defensive
/// cleanup so a test killed mid-assertion (leaving its fixture read-only)
/// doesn't wedge the *next* run of the same test, which starts by
/// overwriting that fixture. Flips the one bit via `PermissionsExt` rather
/// than `Permissions::set_readonly(false)`, which on Unix clears every
/// write-protect bit and leaves the file world-writable (clippy
/// `permissions_set_readonly_false`).
///
/// `pub(crate)`: `disk_test.rs`/`tape_test.rs` reuse it for the
/// insert/new/eject write-back-preservation coverage, the same
/// cross-test-module sharing as [`ReadOnly`] itself.
#[cfg(unix)]
pub(crate) fn ensure_writable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        if perms.mode() & 0o200 == 0 {
            perms.set_mode(perms.mode() | 0o200);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

/// Non-Unix: `Permissions::set_readonly(false)` would be the only option,
/// and [`ReadOnly`]'s drop already restores the exact original permissions —
/// nothing extra to defend against, so the defensive cleanup is a no-op.
#[cfg(not(unix))]
pub(crate) fn ensure_writable(_path: &Path) {}

/// Make `path` read-only for the duration of the value — used to simulate a
/// write-back failure (`EACCES`/`EPERM`) without needing actual filesystem
/// permission loss. Restores the exact original permissions on drop, panic
/// included, so a failed assertion never leaves a permanently read-only
/// fixture behind (and, unlike `set_readonly(false)`, never widens them
/// past what they started as).
///
/// `pub(crate)`: `manager_test.rs` reuses it for the suspend half of the
/// same regression coverage — the same cross-test-module sharing as
/// `machine_def_test.rs`'s `TempDir`.
pub(crate) struct ReadOnly<'a> {
    path: &'a Path,
    original: std::fs::Permissions,
}

impl<'a> ReadOnly<'a> {
    pub(crate) fn new(path: &'a Path) -> Self {
        let original = std::fs::metadata(path)
            .expect("fixture must exist before it's made read-only")
            .permissions();
        let mut perms = original.clone();
        perms.set_readonly(true);
        std::fs::set_permissions(path, perms).expect("set read-only");
        Self { path, original }
    }
}

impl Drop for ReadOnly<'_> {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(self.path, self.original.clone());
    }
}

/// Scratch directory holding only the fixture files a given test writes
/// into it, under `target/` (git-ignored, unlike the installed
/// [`installed_roms_dir`] this test also reads from for the system ROM and
/// the FD-502's `disk11.rom` — both real dumps installed by
/// `ensure_assets`, same as every other test that mounts an FD-502).
fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp-test-save-state")
        .join(name);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// A machine with a ROM pak (direct fixture) and a floppy (direct
/// fixture) both mounted through a MultiPak — the FD-502 and a plain
/// cartridge share the single cartridge port on real hardware, so an MPI
/// is the only way to combine them — produces [`MediaRefs`] whose hashes
/// match [`snapshot::sha256_file`] of every file involved: the two
/// fixtures this test wrote, the installed `coco3.rom` system ROM, and
/// the installed `disk11.rom` the FD-502 always loads
/// ([`disk_basic_rom_path`]).
#[test]
fn build_media_refs_hashes_match_the_mounted_files() {
    let roms_dir = installed_roms_dir();
    let rom_path = roms_dir.join(COCO3_ROM_FILE);
    let rom = std::fs::read(&rom_path)
        .expect("installed coco3.rom is required (ensure_assets)")
        .into_boxed_slice();
    assert!(
        disk_basic_rom_path().is_file(),
        "installed disk11.rom is required (ensure_assets)"
    );

    let dir = scratch_dir("media-refs");
    let cart_path = dir.join("game.rom");
    std::fs::write(&cart_path, vec![0x11u8; 0x4000]).expect("write cart fixture");
    let disk_path = dir.join("disk0.dsk");
    std::fs::write(&disk_path, Vec::<u8>::new()).expect("write disk fixture");

    let mut app = CocoApp::new(
        MachineConfig::default(),
        rom,
        ROMSource::File(rom_path.clone()),
        None,
        [None, None],
        [None, None],
        std::array::from_fn(|_| None),
        false,
        false,
        false,
    );
    app.insert_multipak();
    app.mpi_insert_rompak(0, cart_path.clone());
    app.mpi_insert_fd502(3);
    assert!(
        app.cart_error.is_none(),
        "mounting the MPI slots: {:?}",
        app.cart_error
    );
    app.insert_disk(0, disk_path.clone());
    assert!(
        app.cart_error.is_none(),
        "mounting the disk: {:?}",
        app.cart_error
    );

    let media = app
        .build_media_refs()
        .expect("build_media_refs should succeed");

    let system_rom = media
        .system_rom
        .as_ref()
        .expect("system ROM must be recorded");
    assert_eq!(system_rom.path, rom_path);
    // Hashes the boot-time bytes the running machine actually has
    // (`Self::system_rom_media_ref`'s fix — phase-5 review item 13), not
    // a fresh re-read of the file; they agree here only because the file
    // hasn't changed since boot, which is exactly what this asserts.
    assert_eq!(
        system_rom.sha256,
        snapshot::sha256_hex(&app.machine.bus.rom)
    );

    assert_eq!(
        media.cart_roms.len(),
        2,
        "the ROM pak and the FD-502's own ROM"
    );
    let cart_ref = media
        .cart_roms
        .iter()
        .find(|r| r.mpi_slot == Some(0))
        .expect("the ROM pak's slot must be recorded");
    assert_eq!(cart_ref.rom.path, cart_path);
    assert_eq!(
        cart_ref.rom.sha256,
        snapshot::sha256_file(&cart_path).unwrap()
    );
    let fd502_ref = media
        .cart_roms
        .iter()
        .find(|r| r.mpi_slot == Some(3))
        .expect("the FD-502's slot must be recorded");
    assert_eq!(fd502_ref.rom.path, disk_basic_rom_path());
    assert_eq!(
        fd502_ref.rom.sha256,
        snapshot::sha256_file(&disk_basic_rom_path()).unwrap()
    );

    let disk0 = media.disks[0].as_ref().expect("drive 0 must be recorded");
    assert_eq!(disk0.path, disk_path);
    assert_eq!(disk0.sha256, snapshot::sha256_file(&disk_path).unwrap());
    assert!(media.disks[1].is_none());
}

/// A bare booted machine — no cart, no media mounted yet — for the flush-
/// failure tests below, which each mount exactly one piece of media
/// themselves.
///
/// `pub(crate)`: `disk_test.rs`/`tape_test.rs` reuse it, like
/// [`ensure_writable`] and [`ReadOnly`].
pub(crate) fn boot_app() -> CocoApp {
    let roms_dir = installed_roms_dir();
    let rom_path = roms_dir.join(COCO3_ROM_FILE);
    let rom = std::fs::read(&rom_path)
        .expect("installed coco3.rom is required (ensure_assets)")
        .into_boxed_slice();
    CocoApp::new(
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
    )
}

/// A one-track JVC image, the minimal `write_byte`-able fixture — every
/// FD-502 write-back test needs at least one track to dirty.
///
/// `pub(crate)`: `disk_test.rs` reuses it, like [`boot_app`].
pub(crate) fn write_one_track_disk(path: &Path) {
    let sector_size = 128usize << fdc::DEFAULT_SECTOR_SIZE_CODE;
    let one_track = fdc::DEFAULT_SECTORS_PER_TRACK * sector_size * fdc::DEFAULT_SIDES;
    std::fs::write(path, vec![0u8; one_track]).expect("write disk fixture");
}

/// `save_state_to` must fail — before writing any `.ccstate`, and without
/// clearing the disk's dirty flag — when a floppy's write-back can't reach
/// its file, then succeed and clear dirty once the file is writable again
/// (the "later retry can succeed" contract `write_back_disk`'s doc promises).
/// Regression coverage for Vikunja #175: `save_state_to` used to call the
/// then-infallible `flush_media`, hash whatever stale bytes were already on
/// disk, and report success even though the dirty disk was never saved.
#[test]
fn save_state_to_fails_and_leaves_disk_dirty_when_write_back_fails() {
    let dir = scratch_dir("disk-flush-failure");
    let disk_path = dir.join("dirty.dsk");
    ensure_writable(&disk_path);
    write_one_track_disk(&disk_path);

    let mut app = boot_app();
    app.insert_disk(0, disk_path.clone());
    assert!(
        app.cart_error.is_none(),
        "mounting the disk: {:?}",
        app.cart_error
    );

    app.machine
        .bus
        .cart
        .as_disk_cart()
        .expect("FD-502 just inserted")
        .disk_mut(0)
        .expect("drive 0 mounted")
        .write_byte(0, 0xAA);
    let is_dirty = |app: &mut CocoApp| {
        app.machine
            .bus
            .cart
            .as_disk_cart()
            .unwrap()
            .disk(0)
            .unwrap()
            .dirty()
    };
    assert!(is_dirty(&mut app), "writing a byte must dirty the disk");

    let ccstate_path = dir.join("state.ccstate");
    let _ = std::fs::remove_file(&ccstate_path); // a prior run's leftover
    {
        let _ro = ReadOnly::new(&disk_path);
        let err = app
            .save_state_to(&ccstate_path)
            .expect_err("a read-only backing file must fail the save");
        assert!(
            err.contains(&disk_path.display().to_string()),
            "error should name the unwritable path: {err}"
        );
        assert!(
            !ccstate_path.exists(),
            "no .ccstate must be written on a flush failure"
        );
        assert!(
            is_dirty(&mut app),
            "the disk must stay dirty for a later retry"
        );
    }

    // `_ro` dropped above restores write access; retry must now succeed.
    app.save_state_to(&ccstate_path)
        .expect("retry after the file is writable again must succeed");
    assert!(ccstate_path.exists());
    assert!(
        !is_dirty(&mut app),
        "a successful save-back must clear dirty"
    );
    let saved = std::fs::read(&disk_path).expect("read saved disk");
    assert_eq!(saved[0], 0xAA, "the write-back must have actually landed");
}

/// [`save_state_to_fails_and_leaves_disk_dirty_when_write_back_fails`]'s tape
/// counterpart: a landed recording that can't be written back to its `.cas`
/// file fails the whole save instead of silently snapshotting stale media.
#[test]
fn save_state_to_fails_when_tape_write_back_fails() {
    let dir = scratch_dir("tape-flush-failure");
    let tape_path = dir.join("untitled.cas");
    let _ = std::fs::remove_file(&tape_path); // `new_tape` refuses to overwrite

    let mut app = boot_app();
    app.new_tape(tape_path.clone());
    assert!(
        app.cart_error.is_none(),
        "creating the blank tape: {:?}",
        app.cart_error
    );

    let mut block = vec![LEADER; 16];
    block.extend(tape_block(0x01, b"X"));
    {
        let cassette = &mut app.machine.bus.cassette;
        cassette.tick(SPINUP_BURN_CYCLES, true); // drain motor spin-up
        record_bytes_fsk(cassette, &block);
    }
    app.machine.bus.cassette.finalize_recording();
    assert!(
        app.machine.bus.cassette.dirty(),
        "a framed recording must land and dirty the tape"
    );

    let ccstate_path = dir.join("state.ccstate");
    let _ro = ReadOnly::new(&tape_path);
    let err = app
        .save_state_to(&ccstate_path)
        .expect_err("a read-only .cas target must fail the save");
    assert!(
        err.contains(&tape_path.display().to_string()),
        "error should name the unwritable path: {err}"
    );
    assert!(
        !ccstate_path.exists(),
        "no .ccstate must be written on a flush failure"
    );
    assert!(
        app.machine.bus.cassette.dirty(),
        "the tape must stay dirty for a later retry"
    );
}
