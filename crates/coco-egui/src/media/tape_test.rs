use super::*;
use crate::save_state::tests::{ReadOnly, boot_app, ensure_writable};
use coco_core::cassette::test_support::{SPINUP_BURN_CYCLES, record_bytes_fsk, tape_block};

/// Leader byte (Service Manual §5.10, `cassette-verified-facts`), matching
/// `save_state_test.rs`'s use of it to get the demodulator to lock on before
/// a framed block.
const LEADER: u8 = 0x55;

/// Scratch directory holding only the fixture files a given test writes
/// into it, under `target/` (git-ignored), mirroring
/// `save_state_test.rs`'s `scratch_dir`.
fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp-test-media-tape")
        .join(name);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Record one framed block onto the mounted tape and finalize it, dirtying
/// the cassette (mirrors `save_state_test.rs`'s
/// `save_state_to_fails_when_tape_write_back_fails`).
fn dirty_tape(app: &mut CocoApp) {
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
}

/// `insert_tape` over a dirty tape whose backing `.cas` has gone read-only
/// must abort the mount entirely: the old tape stays mounted and dirty, and
/// `tape_path` still names the old file.
#[test]
fn insert_tape_fails_and_preserves_dirty_old_tape_when_write_back_fails() {
    let dir = scratch_dir("insert-write-back-failure");
    let old_path = dir.join("old.cas");
    ensure_writable(&old_path);
    let _ = std::fs::remove_file(&old_path);
    let new_path = dir.join("new.cas");
    std::fs::write(&new_path, Vec::<u8>::new()).expect("write new tape fixture");

    let mut app = boot_app();
    app.new_tape(old_path.clone());
    assert!(
        app.cart_error.is_none(),
        "creating the old blank tape: {:?}",
        app.cart_error
    );
    dirty_tape(&mut app);

    let _ro = ReadOnly::new(&old_path);
    app.insert_tape(new_path.clone());
    let err = app
        .cart_error
        .clone()
        .expect("a read-only backing file must fail the insert");
    assert!(
        err.contains(&old_path.display().to_string()),
        "error should name the unwritable old path: {err}"
    );
    assert!(
        app.machine.bus.cassette.dirty(),
        "the old tape must stay mounted and dirty"
    );
    assert_eq!(app.tape_path, Some(old_path.clone()));
}

/// `new_tape` with a dirty old tape and a read-only old `.cas` must abort
/// before creating the new file at all: no stray empty file lands on disk,
/// and the old tape is preserved exactly like the `insert_tape` case.
#[test]
fn new_tape_fails_and_creates_no_file_when_write_back_fails() {
    let dir = scratch_dir("new-write-back-failure");
    let old_path = dir.join("old.cas");
    ensure_writable(&old_path);
    let _ = std::fs::remove_file(&old_path);
    let new_path = dir.join("blank.cas");
    let _ = std::fs::remove_file(&new_path); // a prior run's leftover

    let mut app = boot_app();
    app.new_tape(old_path.clone());
    assert!(
        app.cart_error.is_none(),
        "creating the old blank tape: {:?}",
        app.cart_error
    );
    dirty_tape(&mut app);

    let _ro = ReadOnly::new(&old_path);
    app.new_tape(new_path.clone());
    let err = app
        .cart_error
        .clone()
        .expect("a read-only backing file must fail new_tape");
    assert!(
        err.contains(&old_path.display().to_string()),
        "error should name the unwritable old path: {err}"
    );
    assert!(
        !new_path.exists(),
        "the new blank tape must not be created on disk"
    );
    assert!(
        app.machine.bus.cassette.dirty(),
        "the old tape must stay mounted and dirty"
    );
    assert_eq!(app.tape_path, Some(old_path.clone()));
}

/// `eject_tape` with a read-only backing `.cas` must abort the eject: the
/// tape stays mounted, dirty, and tracked at its path. Once write access is
/// restored, a retried eject succeeds and clears both the deck and the path.
#[test]
fn eject_tape_fails_then_succeeds_after_write_access_is_restored() {
    let dir = scratch_dir("eject-write-back-failure");
    let tape_path = dir.join("dirty.cas");
    ensure_writable(&tape_path);
    let _ = std::fs::remove_file(&tape_path);

    let mut app = boot_app();
    app.new_tape(tape_path.clone());
    assert!(
        app.cart_error.is_none(),
        "creating the blank tape: {:?}",
        app.cart_error
    );
    dirty_tape(&mut app);

    {
        let _ro = ReadOnly::new(&tape_path);
        app.eject_tape();
        let err = app
            .cart_error
            .clone()
            .expect("a read-only backing file must fail the eject");
        assert!(
            err.contains(&tape_path.display().to_string()),
            "error should name the unwritable path: {err}"
        );
        assert!(app.machine.bus.cassette.dirty(), "the tape must stay dirty");
        assert_eq!(app.tape_path, Some(tape_path.clone()));
    }

    // `_ro` dropped above restores write access; retry must now succeed.
    // `cart_error` is a sticky UI field (cleared by the error dialog, not by
    // the action that set it), so clear it here first to observe the retry's
    // own outcome rather than the previous failure's leftover message.
    app.cart_error = None;
    app.eject_tape();
    assert!(
        app.cart_error.is_none(),
        "the retried eject must succeed: {:?}",
        app.cart_error
    );
    assert_eq!(app.tape_path, None);
}

/// `eject_tape` when only the *optional* `.wav` sibling ([`CocoApp::save_tape_wav`])
/// fails to write must NOT abort: by the time that write is attempted the
/// canonical `.cas` has already landed and [`coco_core::cassette::Cassette::mark_saved`]
/// has already run, so the tape is clean and there is nothing left for an
/// abort to preserve — see [`CocoApp::save_tape_bytes`]'s doc comment. The
/// eject must still proceed (cassette unmounted, `tape_path` cleared) with
/// the `.wav` failure surfaced through [`CocoApp::cart_error`], and the
/// `.cas` on disk must hold the recording that did land.
#[test]
fn eject_tape_proceeds_when_only_the_wav_sibling_write_fails() {
    let dir = scratch_dir("wav-sibling-write-back-failure");
    let tape_path = dir.join("wav-edge.cas");
    ensure_writable(&tape_path);
    let _ = std::fs::remove_file(&tape_path);
    let wav_path = tape_path.with_extension("wav");
    ensure_writable(&wav_path);

    let mut app = boot_app();
    app.save_tape_wav = true;
    app.new_tape(tape_path.clone());
    assert!(
        app.cart_error.is_none(),
        "creating the blank tape: {:?}",
        app.cart_error
    );
    dirty_tape(&mut app);
    let expected_cas_bytes = app.machine.bus.cassette.tape_bytes().to_vec();

    // The `.wav` sibling must already exist to be made read-only: `.cas`
    // writes happen first and always succeed here, so only the second,
    // `.wav` write in `save_tape_bytes` is made to fail.
    std::fs::write(&wav_path, Vec::<u8>::new()).expect("create wav fixture");
    let _ro = ReadOnly::new(&wav_path);

    app.eject_tape();
    let err = app
        .cart_error
        .clone()
        .expect("a read-only .wav sibling must still report an error");
    assert!(
        err.contains(&wav_path.display().to_string()),
        "error should name the unwritable .wav path: {err}"
    );
    assert!(
        !app.machine.bus.cassette.has_tape(),
        "the eject must still proceed despite the .wav failure"
    );
    assert_eq!(app.tape_path, None);
    let saved_cas = std::fs::read(&tape_path).expect("read saved .cas");
    assert_eq!(
        saved_cas, expected_cas_bytes,
        "the canonical .cas write-back must have landed before the .wav failure"
    );
}
