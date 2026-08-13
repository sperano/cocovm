use super::*;
use crate::save_state::tests::{ReadOnly, boot_app, ensure_writable, write_one_track_disk};

/// The byte every test below writes to dirty a mounted disk, and checks for
/// in the saved-back file afterward.
const DIRTY_BYTE: u8 = 0xAA;

/// Scratch directory holding only the fixture files a given test writes
/// into it, under `target/` (git-ignored), mirroring
/// `save_state_test.rs`'s `scratch_dir`.
fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp-test-media-disk")
        .join(name);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn is_dirty(app: &mut CocoApp, drive: usize) -> bool {
    app.machine
        .bus
        .cart
        .as_disk_cart()
        .expect("FD-502 mounted")
        .disk(drive)
        .expect("drive mounted")
        .dirty()
}

fn write_dirty_byte(app: &mut CocoApp, drive: usize) {
    app.machine
        .bus
        .cart
        .as_disk_cart()
        .expect("FD-502 mounted")
        .disk_mut(drive)
        .expect("drive mounted")
        .write_byte(0, DIRTY_BYTE);
}

/// Mount `disk_path` in `drive` and dirty byte 0, asserting each step
/// succeeds — the common setup shared by every test below.
fn mount_and_dirty(app: &mut CocoApp, drive: usize, disk_path: &Path) {
    app.insert_disk(drive, disk_path.to_path_buf());
    assert!(
        app.cart_error.is_none(),
        "mounting the disk: {:?}",
        app.cart_error
    );
    write_dirty_byte(app, drive);
    assert!(is_dirty(app, drive), "writing a byte must dirty the disk");
}

/// `insert_disk` over a dirty disk whose backing file has gone read-only
/// must abort the mount entirely: the old disk stays mounted and dirty (its
/// written byte still present), `disk_paths[drive]` still names the old
/// file, and the new image is never installed.
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

/// `new_blank_disk` over a dirty old disk with a read-only backing file must
/// abort before creating the new file at all: no stray empty image lands on
/// disk, and the old disk is preserved exactly like the `insert_disk` case.
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

/// `eject_disk` with a read-only backing file must abort the eject: the
/// disk stays mounted, dirty, and tracked at its path. Once write access is
/// restored, a retried eject succeeds, writes the pending byte through, and
/// leaves the drive empty.
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

    // `_ro` dropped above restores write access; retry must now succeed.
    // `cart_error` is a sticky UI field (cleared by the error dialog, not by
    // the action that set it), so clear it here first to observe the retry's
    // own outcome rather than the previous failure's leftover message.
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
