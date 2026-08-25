use super::*;
use crate::save_state::tests::{
    DIRTY_BYTE, ReadOnly, boot_app, ensure_writable, is_dirty, mount_and_dirty,
    write_one_track_disk,
};

/// Scratch directory holding only the fixture files a given test writes into it, under
/// `target/` (git-ignored).
fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp-test-media-disk")
        .join(name);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// `insert_disk` over a dirty disk whose backing file has gone read-only must abort the
/// mount entirely — the old disk stays mounted, dirty, and tracked; the
/// new image is never installed.
#[test]
fn insert_disk_fails_and_preserves_dirty_old_disk_when_write_back_fails() {
    let dir = scratch_dir("insert-write-back-failure");
    let old_path = dir.join("old.dsk");
    ensure_writable(&old_path);
    write_one_track_disk(&old_path);
    let new_path = dir.join("new.dsk");
    write_one_track_disk(&new_path);

    let mut app = boot_app();
    mount_and_dirty(&mut app, 0, &old_path);

    let _ro = ReadOnly::new(&old_path);
    app.insert_disk(0, new_path.clone());
    let err = app
        .cart_error
        .clone()
        .expect("a read-only backing file must fail the insert");
    assert!(
        err.contains(&old_path.display().to_string()),
        "error should name the unwritable old path: {err}"
    );
    assert!(
        is_dirty(&mut app, 0),
        "the old disk must stay mounted and dirty"
    );
    assert_eq!(app.disk_paths[0], Some(old_path.clone()));

    let disk = app
        .machine
        .bus
        .cart
        .as_disk_cart()
        .unwrap()
        .disk(0)
        .unwrap();
    assert_eq!(
        disk.bytes()[0],
        DIRTY_BYTE,
        "the old (unwritten-back) disk contents must still be mounted"
    );
}

/// `new_blank_disk` over a dirty old disk with a read-only backing file must abort before
/// creating the new file at all — no stray empty image lands, and the old disk is preserved.
#[test]
fn new_blank_disk_fails_and_creates_no_file_when_write_back_fails() {
    let dir = scratch_dir("new-blank-write-back-failure");
    let old_path = dir.join("old.dsk");
    ensure_writable(&old_path);
    write_one_track_disk(&old_path);
    let new_path = dir.join("blank.dsk");
    let _ = std::fs::remove_file(&new_path); // a prior run's leftover

    let mut app = boot_app();
    mount_and_dirty(&mut app, 0, &old_path);

    let _ro = ReadOnly::new(&old_path);
    app.new_blank_disk(0, new_path.clone());
    let err = app
        .cart_error
        .clone()
        .expect("a read-only backing file must fail new_blank_disk");
    assert!(
        err.contains(&old_path.display().to_string()),
        "error should name the unwritable old path: {err}"
    );
    assert!(
        !new_path.exists(),
        "the new blank image must not be created on disk"
    );
    assert!(
        is_dirty(&mut app, 0),
        "the old disk must stay mounted and dirty"
    );
    assert_eq!(app.disk_paths[0], Some(old_path.clone()));
}

/// `eject_disk` with a read-only backing file must abort, leaving the disk mounted, dirty,
/// and tracked. Once write access is restored, a retried eject succeeds and empties the drive.
#[test]
fn eject_disk_fails_then_succeeds_after_write_access_is_restored() {
    let dir = scratch_dir("eject-write-back-failure");
    let disk_path = dir.join("dirty.dsk");
    ensure_writable(&disk_path);
    write_one_track_disk(&disk_path);

    let mut app = boot_app();
    mount_and_dirty(&mut app, 0, &disk_path);

    {
        let _ro = ReadOnly::new(&disk_path);
        app.eject_disk(0);
        let err = app
            .cart_error
            .clone()
            .expect("a read-only backing file must fail the eject");
        assert!(
            err.contains(&disk_path.display().to_string()),
            "error should name the unwritable path: {err}"
        );
        assert!(
            app.machine
                .bus
                .cart
                .as_disk_cart()
                .unwrap()
                .disk(0)
                .is_some(),
            "the disk must stay mounted"
        );
        assert!(is_dirty(&mut app, 0), "the disk must stay dirty");
        assert_eq!(app.disk_paths[0], Some(disk_path.clone()));
    }

    // `cart_error` is sticky (not auto-cleared), so reset it here to
    // observe the retry's own outcome.
    app.cart_error = None;
    app.eject_disk(0);
    assert!(
        app.cart_error.is_none(),
        "the retried eject must succeed: {:?}",
        app.cart_error
    );
    assert!(
        app.machine
            .bus
            .cart
            .as_disk_cart()
            .unwrap()
            .disk(0)
            .is_none(),
        "the drive must be empty after a successful eject"
    );
    assert_eq!(app.disk_paths[0], None);
    let saved = std::fs::read(&disk_path).expect("read saved disk");
    assert_eq!(
        saved[0], DIRTY_BYTE,
        "the write-back must have actually landed"
    );
}
