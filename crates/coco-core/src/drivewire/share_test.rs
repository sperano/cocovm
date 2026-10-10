use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::drivewire::host::{HostError, HostExecutor, HostState};

const QUEUE_CAPACITY: usize = 4;

static NEXT_SCRATCH: AtomicUsize = AtomicUsize::new(0);

/// A temporary folder removed on drop, shared by the share test modules.
pub(in crate::drivewire::share) struct ScratchDir(PathBuf);

impl ScratchDir {
    pub(in crate::drivewire::share) fn new(name: &str) -> Self {
        let id = NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("cocovm-share-{name}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create scratch folder");
        Self(path)
    }

    pub(in crate::drivewire::share) fn path(&self) -> &Path {
        &self.0
    }

    /// Creates `relative` (and its parents) holding `bytes`.
    pub(in crate::drivewire::share) fn file(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().expect("file has a parent")).expect("create parents");
        fs::write(&path, bytes).expect("write scratch file");
        path
    }

    pub(in crate::drivewire::share) fn dir(&self, relative: &str) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(&path).expect("create scratch subfolder");
        path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A table of `(name, root, access)` shares.
pub(in crate::drivewire::share) fn table(shares: &[(&str, &Path, ShareAccess)]) -> ShareTable {
    ShareTable::new(
        shares
            .iter()
            .map(|&(name, root, access)| ShareSpec {
                name: name.to_string(),
                root: root.to_path_buf(),
                access,
            })
            .collect(),
    )
    .expect("valid test shares")
}

fn spec(name: &str) -> ShareSpec {
    ShareSpec {
        name: name.to_string(),
        root: PathBuf::from("/srv/coco"),
        access: ShareAccess::ReadOnly,
    }
}

#[test]
fn share_names_accept_letters_digits_hyphens_and_underscores() {
    for name in [
        "games",
        "OS9-disks",
        "a",
        "x_1",
        &"n".repeat(MAX_SHARE_NAME_LEN),
    ] {
        assert_eq!(validate_share_name(name), Ok(()), "{name:?}");
    }
}

#[test]
fn share_names_reject_separators_dots_spaces_and_overlong_names() {
    let too_long = "n".repeat(MAX_SHARE_NAME_LEN + 1);
    for name in [
        "",
        ".",
        "..",
        "a/b",
        "a b",
        "a.b",
        "日本",
        "C:",
        too_long.as_str(),
    ] {
        assert_eq!(
            validate_share_name(name),
            Err(ShareConfigError::InvalidName(name.to_string())),
            "{name:?}"
        );
    }
}

#[test]
fn table_rejects_duplicate_names_ignoring_case() {
    assert_eq!(
        ShareTable::new(vec![spec("Games"), spec("games")]),
        Err(ShareConfigError::DuplicateName("games".to_string()))
    );
}

#[test]
fn table_rejects_an_empty_root_and_too_many_shares() {
    let mut empty = spec("games");
    empty.root = PathBuf::new();
    assert_eq!(
        ShareTable::new(vec![empty]),
        Err(ShareConfigError::EmptyRoot("games".to_string()))
    );
    let many = (0..=MAX_SHARES).map(|i| spec(&format!("s{i}"))).collect();
    assert_eq!(ShareTable::new(many), Err(ShareConfigError::TooMany));
}

#[test]
fn table_lookup_ignores_ascii_case() {
    let table = ShareTable::new(vec![spec("Games")]).unwrap();
    assert_eq!(table.get("GAMES").map(|s| s.name.as_str()), Some("Games"));
    assert!(table.get("other").is_none());
}

#[test]
fn errors_map_to_dw_command_codes() {
    for error in [
        ShareError::InvalidPath,
        ShareError::UnknownShare,
        ShareError::Escape,
        ShareError::RootUnavailable,
        ShareError::NotFound,
        ShareError::NotADirectory,
        ShareError::IsADirectory,
        ShareError::ReadOnly,
        ShareError::PermissionDenied,
    ] {
        assert_eq!(error.command_code(), command_code::RESOLUTION, "{error:?}");
    }
    for error in [
        ShareError::Busy,
        ShareError::DirectoryTooLarge,
        ShareError::TooManyHandles,
        ShareError::BadHandle,
        ShareError::TooLarge,
        ShareError::Io(std::io::ErrorKind::Other),
    ] {
        assert_eq!(error.command_code(), command_code::HOST_IO, "{error:?}");
    }
}

#[test]
fn io_errors_map_to_share_errors() {
    use std::io::{Error, ErrorKind};
    let cases = [
        (ErrorKind::NotFound, ShareError::NotFound),
        (ErrorKind::NotADirectory, ShareError::NotADirectory),
        (ErrorKind::IsADirectory, ShareError::IsADirectory),
        (ErrorKind::PermissionDenied, ShareError::PermissionDenied),
        (ErrorKind::ReadOnlyFilesystem, ShareError::ReadOnly),
        (
            ErrorKind::StorageFull,
            ShareError::Io(ErrorKind::StorageFull),
        ),
    ];
    for (kind, expected) in cases {
        assert_eq!(ShareError::from(Error::from(kind)), expected);
    }
}

#[test]
fn guest_messages_are_printable_ascii() {
    let error = ShareError::Io(std::io::ErrorKind::Other);
    for error in [ShareError::Escape, ShareError::Busy, error] {
        assert!(error.message().bytes().all(|b| (0x20..=0x7E).contains(&b)));
    }
}

#[test]
fn same_table_and_owner_keep_the_session() {
    let scratch = ScratchDir::new("keep");
    scratch.dir("sub");
    let shares = table(&[("games", scratch.path(), ShareAccess::ReadOnly)]);
    let owner = LeaseOwner::new();
    let mut server = DWServer::new();
    server.set_shares(shares.clone(), owner);
    server
        .share_session()
        .execute(ShareOp::ChangeDir {
            path: b"games/sub".to_vec(),
        })
        .unwrap();
    server.set_shares(shares.clone(), owner);
    assert_eq!(server.share_session().status().cwd, "/games/sub");

    server.set_shares(shares, LeaseOwner::new());
    assert_eq!(server.share_session().status().cwd, "/");
}

#[test]
fn a_changed_table_starts_a_fresh_session() {
    let scratch = ScratchDir::new("change");
    let owner = LeaseOwner::new();
    let mut server = DWServer::new();
    server.set_shares(
        table(&[("games", scratch.path(), ShareAccess::ReadOnly)]),
        owner,
    );
    server
        .share_session()
        .execute(ShareOp::ChangeDir {
            path: b"games".to_vec(),
        })
        .unwrap();
    server.set_shares(
        table(&[("games", scratch.path(), ShareAccess::ReadWrite)]),
        owner,
    );
    assert_eq!(server.share_session().status().cwd, "/");
}

#[test]
fn share_ops_run_on_the_host_executor_and_report_through_service_completions() {
    let scratch = ScratchDir::new("submit");
    scratch.file("hello.txt", b"hi");
    let (host, manual) = HostExecutor::manual(QUEUE_CAPACITY);
    let mut server = DWServer::with_host_executor(host);
    server.set_shares(
        table(&[("games", scratch.path(), ShareAccess::ReadOnly)]),
        LeaseOwner::new(),
    );
    let id = server
        .submit_share_op(ShareOp::List {
            path: b"games".to_vec(),
            start: 0,
        })
        .unwrap();
    let missing = server
        .submit_share_op(ShareOp::OpenRead {
            path: b"games/missing".to_vec(),
        })
        .unwrap();
    assert!(server.take_host_completion().is_none());

    while let Some(request) = manual.take_request() {
        manual.complete(request.run()).unwrap();
    }
    server.poll_host();
    server.poll_host();
    let listed = server.take_host_completion().unwrap();
    assert_eq!(listed.id, id);
    assert_eq!(listed.result.unwrap(), b"hello.txt\n");
    let failed = server.take_host_completion().unwrap();
    assert_eq!(failed.id, missing);
    assert_eq!(failed.result, Err(HostError::Share(ShareError::NotFound)));
    assert_eq!(
        server.host_diagnostics().last_error,
        Some(HostError::Share(ShareError::NotFound))
    );
}

#[test]
fn stopping_the_host_rejects_share_ops() {
    let mut server = DWServer::new();
    server.stop_host();
    assert_eq!(server.host_diagnostics().state, HostState::Stopped);
    assert!(
        server
            .submit_share_op(ShareOp::List {
                path: Vec::new(),
                start: 0
            })
            .is_err()
    );
}

#[test]
fn machine_reset_returns_the_share_session_to_the_top_level() {
    let scratch = ScratchDir::new("reset");
    scratch.file("a.bin", b"a");
    let mut server = DWServer::new();
    server.set_shares(
        table(&[("games", scratch.path(), ShareAccess::ReadOnly)]),
        LeaseOwner::new(),
    );
    let session = server.share_session().clone();
    session
        .execute(ShareOp::ChangeDir {
            path: b"games".to_vec(),
        })
        .unwrap();
    session
        .execute(ShareOp::OpenRead {
            path: b"a.bin".to_vec(),
        })
        .unwrap();
    server.reset_session();
    let status = server.share_session().status();
    assert_eq!((status.cwd.as_str(), status.open_handles), ("/", 0));
    assert_eq!(
        server.share_session().execute(ShareOp::Read {
            handle: ShareHandle::from_byte(1),
            max: 1
        }),
        Err(ShareError::BadHandle)
    );
}
