use std::fs;
use std::io::Read;
use std::sync::Arc;

use super::super::super::access::{AccessRegistry, LeaseOwner};
use super::*;
use crate::drivewire::share::tests::{ScratchDir, table};

const OBJECT_BYTES: &[u8] = b"object bytes";

/// A session with `games` (`access`) serving named objects and a second,
/// read/write share `other` that does not.
fn session(scratch: &ScratchDir, other: &ScratchDir, access: ShareAccess) -> ShareSession {
    let shares = table(&[
        ("games", scratch.path(), access),
        ("other", other.path(), ShareAccess::ReadWrite),
    ])
    .with_named_objects(Some("GAMES".to_string()));
    ShareSession::new(shares, LeaseOwner::new(), Arc::new(AccessRegistry::new()))
}

fn contents(image: &mut ShareImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    image.file.read_to_end(&mut bytes).unwrap();
    bytes
}

#[test]
fn mount_opens_an_existing_object_by_its_name_inside_the_share() {
    let (scratch, other) = (ScratchDir::new("named-open"), ScratchDir::new("named-o"));
    scratch.file("boot/hello.bin", OBJECT_BYTES);
    let session = session(&scratch, &other, ShareAccess::ReadWrite);

    let mut image = session.open_named(b"boot/hello.bin").unwrap();
    assert_eq!(contents(&mut image), OBJECT_BYTES);
    assert!(image.writable);
    assert_eq!(image.guest_path, "/games/boot/hello.bin");
}

#[test]
fn a_read_only_share_yields_read_only_objects_and_refuses_create() {
    let (scratch, other) = (ScratchDir::new("named-ro"), ScratchDir::new("named-ro-o"));
    scratch.file("hello.bin", OBJECT_BYTES);
    let session = session(&scratch, &other, ShareAccess::ReadOnly);

    assert!(!session.open_named(b"hello.bin").unwrap().writable);
    assert_eq!(
        session.create_named(b"new.bin").unwrap_err(),
        ShareError::ReadOnly
    );
    assert!(!scratch.path().join("new.bin").exists());
}

#[test]
fn mount_fails_for_a_missing_object_and_create_for_an_existing_one() {
    let (scratch, other) = (
        ScratchDir::new("named-exist"),
        ScratchDir::new("named-ex-o"),
    );
    scratch.file("hello.bin", OBJECT_BYTES);
    let session = session(&scratch, &other, ShareAccess::ReadWrite);

    assert_eq!(
        session.open_named(b"missing.bin").unwrap_err(),
        ShareError::NotFound
    );
    assert_eq!(
        session.create_named(b"hello.bin").unwrap_err(),
        ShareError::Exists
    );
    assert_eq!(
        fs::read(scratch.path().join("hello.bin")).unwrap(),
        OBJECT_BYTES
    );
    assert_eq!(session.status().last_error, Some(ShareError::Exists));
}

#[test]
fn create_makes_an_empty_writable_object() {
    let (scratch, other) = (
        ScratchDir::new("named-create"),
        ScratchDir::new("named-c-o"),
    );
    let session = session(&scratch, &other, ShareAccess::ReadWrite);

    let mut image = session.create_named(b"new.bin").unwrap();
    assert!(image.writable);
    assert!(contents(&mut image).is_empty());
    assert_eq!(fs::read(scratch.path().join("new.bin")).unwrap(), b"");
}

#[test]
fn names_cannot_leave_the_named_object_share() {
    let (scratch, other) = (
        ScratchDir::new("named-escape"),
        ScratchDir::new("named-e-o"),
    );
    other.file("secret.bin", OBJECT_BYTES);
    let session = session(&scratch, &other, ShareAccess::ReadWrite);

    for name in [
        &b"../other/secret.bin"[..],
        b"/other/secret.bin",
        b"..",
        b".",
    ] {
        let error = session.open_named(name).unwrap_err();
        assert!(
            matches!(error, ShareError::Escape | ShareError::InvalidPath),
            "{:?} gave {error:?}",
            String::from_utf8_lossy(name)
        );
    }
    assert_eq!(
        session.create_named(b"../other/new.bin").unwrap_err(),
        ShareError::Escape
    );
    assert!(!other.path().join("new.bin").exists());
}

#[test]
fn malformed_names_are_refused() {
    let (scratch, other) = (ScratchDir::new("named-bad"), ScratchDir::new("named-b-o"));
    let session = session(&scratch, &other, ShareAccess::ReadWrite);

    for name in [
        &b""[..],
        b"a*b",
        b"bad\x80",
        b"a\\b",
        b" lead",
        &[b'x'; 256],
    ] {
        assert_eq!(
            session.create_named(name).unwrap_err(),
            ShareError::InvalidPath,
            "{:?}",
            String::from_utf8_lossy(name)
        );
    }
    assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 0);
}

#[test]
fn without_a_named_object_share_every_name_fails() {
    let scratch = ScratchDir::new("named-none");
    scratch.file("hello.bin", OBJECT_BYTES);
    let shares = table(&[("games", scratch.path(), ShareAccess::ReadWrite)]);
    let session = ShareSession::new(shares, LeaseOwner::new(), Arc::new(AccessRegistry::new()));
    assert_eq!(
        session.open_named(b"hello.bin").unwrap_err(),
        ShareError::UnknownShare
    );

    // A selected share without a folder is not in the table.
    let shares = table(&[("games", scratch.path(), ShareAccess::ReadWrite)])
        .with_named_objects(Some("inactive".to_string()));
    let session = ShareSession::new(shares, LeaseOwner::new(), Arc::new(AccessRegistry::new()));
    assert_eq!(
        session.create_named(b"new.bin").unwrap_err(),
        ShareError::UnknownShare
    );
}

#[test]
fn another_vm_writing_the_object_makes_it_busy() {
    let (scratch, other) = (
        ScratchDir::new("named-busy"),
        ScratchDir::new("named-busy-o"),
    );
    scratch.file("hello.bin", OBJECT_BYTES);
    let registry = Arc::new(AccessRegistry::new());
    let shares = table(&[
        ("games", scratch.path(), ShareAccess::ReadWrite),
        ("other", other.path(), ShareAccess::ReadWrite),
    ])
    .with_named_objects(Some("games".to_string()));
    let vm_a = ShareSession::new(shares.clone(), LeaseOwner::new(), Arc::clone(&registry));
    let vm_b = ShareSession::new(shares, LeaseOwner::new(), registry);

    let held = vm_a.open_named(b"hello.bin").unwrap();
    assert_eq!(vm_b.open_named(b"hello.bin").unwrap_err(), ShareError::Busy);
    drop(held);
    assert!(vm_b.open_named(b"hello.bin").is_ok());
}
