use super::*;
use crate::save_state::tests::{
    DIRTY_BYTE, ReadOnly, boot_app, ensure_writable, is_dirty, mount_and_dirty,
    write_one_track_disk,
};

/// Size of the throwaway ROM pak fixture below — [`ROMPak::from_bytes`] just
/// needs a well-formed load, not real game content (mirrors
/// `save_state_test.rs`'s `build_media_refs_hashes_match_the_mounted_files`
/// fixture).
const ROM_PAK_SIZE: usize = 0x4000;

/// MPI slot the FD-502 occupies in the tests below (mirrors
/// `save_state_test.rs`'s `build_media_refs_hashes_match_the_mounted_files`,
/// which plugs a ROM pak into slot 0 and the FD-502 into slot 3).
const FD502_SLOT: usize = 3;

/// An MPI slot distinct from [`FD502_SLOT`], used to prove an unrelated
/// slot's insert isn't blocked by a failing disk elsewhere in the MPI.
const OTHER_SLOT: usize = 0;

fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp-test-media-cart")
        .join(name);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write_rom_pak_fixture(path: &Path) {
    std::fs::write(path, vec![0x11u8; ROM_PAK_SIZE]).expect("write ROM pak fixture");
}

/// `insert_cartridge` must abort entirely when a dirty disk's backing file is read-only —
/// nothing swapped, disk left dirty — then succeed once write access is restored.
#[test]
fn insert_cartridge_fails_and_preserves_dirty_disk_when_write_back_fails() {
    let dir = scratch_dir("insert-cartridge-write-back-failure");
    let disk_path = dir.join("dirty.dsk");
    ensure_writable(&disk_path);
    write_one_track_disk(&disk_path);
    let rom_path = dir.join("game.rom");
    write_rom_pak_fixture(&rom_path);

    let mut app = boot_app();
    mount_and_dirty(&mut app, 0, &disk_path);

    {
        let _ro = ReadOnly::new(&disk_path);
        app.insert_cartridge(rom_path.clone(), true);
        let err = app
            .cart_error
            .clone()
            .expect("a read-only disk backing file must fail the swap");
        assert!(
            err.contains(&disk_path.display().to_string()),
            "error should name the unwritable disk path: {err}"
        );
        assert!(
            app.machine.bus.cart.as_disk_cart().is_some(),
            "the FD-502 must stay in the port"
        );
        assert!(is_dirty(&mut app, 0), "the disk must stay dirty");
        assert_eq!(app.disk_paths[0].as_deref(), Some(disk_path.as_path()));
        assert!(
            app.cart_path.is_none(),
            "no ROM pak must have been inserted"
        );
    }

    // `_ro` dropped above restores write access; retry must now succeed.
    app.cart_error = None;
    app.insert_cartridge(rom_path.clone(), true);
    assert!(
        app.cart_error.is_none(),
        "the retried insert must succeed: {:?}",
        app.cart_error
    );
    assert_eq!(app.cart_path, Some(rom_path));
    assert_eq!(app.disk_paths, [None, None]);
    let saved = std::fs::read(&disk_path).expect("read saved disk");
    assert_eq!(
        saved[0], DIRTY_BYTE,
        "the write-back must have actually landed"
    );
}

/// Pins the MPI conditional-flush rule: inserting into the FD-502's own slot aborts on a
/// failed write-back, but the same failing disk must not block an insert into another slot.
#[test]
fn mpi_insert_rompak_only_blocks_on_the_fd502s_own_slot() {
    let dir = scratch_dir("mpi-rompak-conditional-flush");
    let disk_path = dir.join("dirty.dsk");
    ensure_writable(&disk_path);
    write_one_track_disk(&disk_path);
    let rom_path = dir.join("game.rom");
    write_rom_pak_fixture(&rom_path);

    let mut app = boot_app();
    app.insert_multipak();
    app.mpi_insert_fd502(FD502_SLOT);
    assert!(
        app.cart_error.is_none(),
        "mounting the FD-502: {:?}",
        app.cart_error
    );
    mount_and_dirty(&mut app, 0, &disk_path);

    let _ro = ReadOnly::new(&disk_path);

    // Replacing the FD-502's own slot must abort.
    app.mpi_insert_rompak(FD502_SLOT, rom_path.clone(), true);
    let err = app
        .cart_error
        .clone()
        .expect("replacing the FD-502's own slot must fail");
    assert!(
        err.contains(&disk_path.display().to_string()),
        "error should name the unwritable disk path: {err}"
    );
    assert!(
        matches!(app.mpi.as_ref().unwrap().slots[FD502_SLOT], MPISlot::FD502),
        "the FD-502 must stay in its slot"
    );
    assert!(is_dirty(&mut app, 0), "the disk must stay dirty");
    assert_eq!(app.disk_paths[0].as_deref(), Some(disk_path.as_path()));

    // The same failing disk must not block an unrelated slot's insert.
    app.cart_error = None;
    app.mpi_insert_rompak(OTHER_SLOT, rom_path, true);
    assert!(
        app.cart_error.is_none(),
        "an unrelated slot's insert must not be blocked by a failing disk: {:?}",
        app.cart_error
    );
    assert!(
        matches!(
            app.mpi.as_ref().unwrap().slots[OTHER_SLOT],
            MPISlot::ROMPak(_)
        ),
        "the ROM pak must be inserted into the other slot"
    );
    assert!(
        matches!(app.mpi.as_ref().unwrap().slots[FD502_SLOT], MPISlot::FD502),
        "the FD-502 must still be in its own slot"
    );
    assert!(
        is_dirty(&mut app, 0),
        "the disk must still be dirty — it was never flushed"
    );
    assert_eq!(app.disk_paths[0].as_deref(), Some(disk_path.as_path()));
}

/// `mpi_insert_rompak` into the FD-502's own slot on a *writable* dirty disk must succeed,
/// flushing before the swap and clearing `disk_paths` — the success path
/// the abort tests above never exercise.
#[test]
fn mpi_insert_rompak_flushes_and_clears_disk_paths_on_success() {
    let dir = scratch_dir("mpi-rompak-success-flush");
    let disk_path = dir.join("dirty.dsk");
    ensure_writable(&disk_path);
    write_one_track_disk(&disk_path);
    let rom_path = dir.join("game.rom");
    write_rom_pak_fixture(&rom_path);

    let mut app = boot_app();
    app.insert_multipak();
    app.mpi_insert_fd502(FD502_SLOT);
    assert!(
        app.cart_error.is_none(),
        "mounting the FD-502: {:?}",
        app.cart_error
    );
    mount_and_dirty(&mut app, 0, &disk_path);

    app.mpi_insert_rompak(FD502_SLOT, rom_path, true);
    assert!(
        app.cart_error.is_none(),
        "the insert must succeed: {:?}",
        app.cart_error
    );
    assert!(
        matches!(
            app.mpi.as_ref().unwrap().slots[FD502_SLOT],
            MPISlot::ROMPak(_)
        ),
        "the ROM pak must replace the FD-502 in its slot"
    );
    assert_eq!(app.disk_paths, [None, None]);
    let saved = std::fs::read(&disk_path).expect("read saved disk");
    assert_eq!(
        saved[0], DIRTY_BYTE,
        "the flush must have actually landed before the FD-502 was replaced"
    );
}

/// [`mpi_insert_rompak_only_blocks_on_the_fd502s_own_slot`]'s counterpart for `mpi_insert_rtc`.
#[test]
fn mpi_insert_rtc_aborts_when_the_fd502s_own_slot_disk_write_back_fails() {
    let dir = scratch_dir("mpi-rtc-conditional-flush");
    let disk_path = dir.join("dirty.dsk");
    ensure_writable(&disk_path);
    write_one_track_disk(&disk_path);

    let mut app = boot_app();
    app.insert_multipak();
    app.mpi_insert_fd502(FD502_SLOT);
    assert!(
        app.cart_error.is_none(),
        "mounting the FD-502: {:?}",
        app.cart_error
    );
    mount_and_dirty(&mut app, 0, &disk_path);

    let _ro = ReadOnly::new(&disk_path);
    app.mpi_insert_rtc(FD502_SLOT);
    let err = app
        .cart_error
        .clone()
        .expect("replacing the FD-502's own slot must fail");
    assert!(
        err.contains(&disk_path.display().to_string()),
        "error should name the unwritable disk path: {err}"
    );
    assert!(
        matches!(app.mpi.as_ref().unwrap().slots[FD502_SLOT], MPISlot::FD502),
        "the FD-502 must stay in its slot"
    );
    assert!(is_dirty(&mut app, 0), "the disk must stay dirty");
    assert_eq!(app.disk_paths[0].as_deref(), Some(disk_path.as_path()));
}
