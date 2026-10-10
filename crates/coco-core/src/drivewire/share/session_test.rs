use std::fs;

use super::*;
use crate::drivewire::host::HostExecutor;
use crate::drivewire::share::tests::{ScratchDir, table};

const QUEUE_CAPACITY: usize = 2;

fn session(
    scratch: &ScratchDir,
    access: ShareAccess,
    registry: &Arc<AccessRegistry>,
) -> ShareSession {
    ShareSession::new(
        table(&[("games", scratch.path(), access)]),
        LeaseOwner::new(),
        Arc::clone(registry),
    )
}

fn op_path(op: fn(Vec<u8>) -> ShareOp, path: &str) -> ShareOp {
    op(path.as_bytes().to_vec())
}

fn list(path: &str) -> ShareOp {
    ShareOp::List {
        path: path.as_bytes().to_vec(),
        start: 0,
    }
}

fn cd(path: Vec<u8>) -> ShareOp {
    ShareOp::ChangeDir { path }
}

fn open_read(path: Vec<u8>) -> ShareOp {
    ShareOp::OpenRead { path }
}

fn create(path: Vec<u8>) -> ShareOp {
    ShareOp::Create { path }
}

fn handle_of(reply: Vec<u8>) -> ShareHandle {
    assert_eq!(reply.len(), 1, "an open replies with one handle byte");
    ShareHandle::from_byte(reply[0])
}

fn read_all(session: &ShareSession, handle: ShareHandle) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let chunk = session
            .execute(ShareOp::Read {
                handle,
                max: MAX_HOST_RESPONSE_BYTES,
            })
            .unwrap();
        if chunk.is_empty() {
            return bytes;
        }
        bytes.extend(chunk);
    }
}

#[test]
fn two_sessions_on_one_root_keep_their_own_directory_and_handles() {
    let scratch = ScratchDir::new("session-isolation");
    scratch.file("a/one.txt", b"from a");
    scratch.file("b/two.txt", b"from b");
    let registry = Arc::new(AccessRegistry::new());
    let vm_a = session(&scratch, ShareAccess::ReadOnly, &registry);
    let vm_b = session(&scratch, ShareAccess::ReadOnly, &registry);

    vm_a.execute(op_path(cd, "games/a")).unwrap();
    vm_b.execute(op_path(cd, "games/b")).unwrap();
    assert_eq!(vm_a.execute(list("")).unwrap(), b"one.txt\n");
    assert_eq!(vm_b.execute(list("")).unwrap(), b"two.txt\n");

    let a = handle_of(vm_a.execute(op_path(open_read, "one.txt")).unwrap());
    let b = handle_of(vm_b.execute(op_path(open_read, "two.txt")).unwrap());
    assert_eq!(a, b, "handle numbers are per session");
    assert_eq!(read_all(&vm_a, a), b"from a");
    assert_eq!(read_all(&vm_b, b), b"from b");
    vm_a.execute(ShareOp::Close { handle: a }).unwrap();
    assert_eq!(vm_b.status().open_handles, 1);
    assert_eq!(
        (vm_a.status().cwd, vm_b.status().cwd),
        ("/games/a".into(), "/games/b".into())
    );
}

#[test]
fn two_sessions_can_use_different_roots_under_one_name() {
    let first = ScratchDir::new("session-root-a");
    let second = ScratchDir::new("session-root-b");
    first.file("only-a.txt", b"");
    second.file("only-b.txt", b"");
    let registry = Arc::new(AccessRegistry::new());
    let vm_a = session(&first, ShareAccess::ReadOnly, &registry);
    let vm_b = session(&second, ShareAccess::ReadOnly, &registry);
    assert_eq!(vm_a.execute(list("games")).unwrap(), b"only-a.txt\n");
    assert_eq!(vm_b.execute(list("games")).unwrap(), b"only-b.txt\n");
}

#[test]
fn listings_continue_from_an_entry_index_within_the_response_limit() {
    let scratch = ScratchDir::new("session-chunks");
    let long = "n".repeat(200);
    let count = 40;
    for i in 0..count {
        scratch.file(&format!("{long}{i:02}"), b"");
    }
    let registry = Arc::new(AccessRegistry::new());
    let vm = session(&scratch, ShareAccess::ReadOnly, &registry);
    let mut start = 0;
    let mut seen = 0;
    loop {
        let chunk = vm
            .execute(ShareOp::List {
                path: b"games".to_vec(),
                start,
            })
            .unwrap();
        if chunk.is_empty() {
            break;
        }
        assert!(chunk.len() <= MAX_HOST_RESPONSE_BYTES);
        let lines = chunk.iter().filter(|&&b| b == LINE_END).count();
        start += lines;
        seen += lines;
    }
    assert_eq!(seen, count);
}

#[test]
fn change_dir_rejects_files_and_missing_directories_without_moving() {
    let scratch = ScratchDir::new("session-cd");
    scratch.file("file.txt", b"");
    let registry = Arc::new(AccessRegistry::new());
    let vm = session(&scratch, ShareAccess::ReadOnly, &registry);
    vm.execute(op_path(cd, "games")).unwrap();
    assert_eq!(
        vm.execute(op_path(cd, "file.txt")),
        Err(ShareError::NotADirectory)
    );
    assert_eq!(vm.execute(op_path(cd, "nope")), Err(ShareError::NotFound));
    assert_eq!(
        vm.execute(op_path(cd, "/nope")),
        Err(ShareError::UnknownShare)
    );
    assert_eq!(vm.status().cwd, "/games");
    assert_eq!(vm.status().last_error, Some(ShareError::UnknownShare));
    vm.execute(op_path(cd, "..")).unwrap();
    assert_eq!(vm.status().cwd, "/");
}

#[test]
fn read_only_shares_refuse_writes() {
    let scratch = ScratchDir::new("session-ro");
    let original = scratch.file("disk.dsk", b"original");
    let registry = Arc::new(AccessRegistry::new());
    let vm = session(&scratch, ShareAccess::ReadOnly, &registry);
    assert_eq!(
        vm.execute(op_path(create, "games/disk.dsk")),
        Err(ShareError::ReadOnly)
    );
    assert_eq!(
        vm.execute(op_path(create, "games/new.bin")),
        Err(ShareError::ReadOnly)
    );
    assert!(!scratch.path().join("new.bin").exists());

    let handle = handle_of(vm.execute(op_path(open_read, "games/disk.dsk")).unwrap());
    assert_eq!(
        vm.execute(ShareOp::Write {
            handle,
            data: b"x".to_vec()
        }),
        Err(ShareError::ReadOnly)
    );

    let image = vm.open_image(b"games/disk.dsk").unwrap();
    assert!(!image.writable);
    assert!((&image.file).write_all(b"clobber").is_err());
    assert_eq!(fs::read(original).unwrap(), b"original");
}

#[test]
fn read_write_shares_create_truncate_and_write() {
    let scratch = ScratchDir::new("session-rw");
    let target = scratch.file("out.bin", b"old contents");
    let registry = Arc::new(AccessRegistry::new());
    let vm = session(&scratch, ShareAccess::ReadWrite, &registry);
    let handle = handle_of(vm.execute(op_path(create, "games/out.bin")).unwrap());
    vm.execute(ShareOp::Write {
        handle,
        data: b"new".to_vec(),
    })
    .unwrap();
    vm.execute(ShareOp::Close { handle }).unwrap();
    assert_eq!(fs::read(target).unwrap(), b"new");
    assert_eq!(
        vm.execute(ShareOp::Write {
            handle,
            data: vec![0; MAX_HOST_RESPONSE_BYTES + 1]
        }),
        Err(ShareError::TooLarge)
    );
    assert!(vm.open_image(b"games/out.bin").unwrap().writable);
}

#[test]
fn concurrent_access_from_two_vms_follows_the_lease_rules() {
    let scratch = ScratchDir::new("session-concurrent");
    let shared = scratch.file("shared.dsk", b"keep me");
    let registry = Arc::new(AccessRegistry::new());
    let vm_a = session(&scratch, ShareAccess::ReadWrite, &registry);
    let vm_b = session(&scratch, ShareAccess::ReadWrite, &registry);

    let reader_a = handle_of(
        vm_a.execute(op_path(open_read, "games/shared.dsk"))
            .unwrap(),
    );
    let reader_b = handle_of(
        vm_b.execute(op_path(open_read, "games/shared.dsk"))
            .unwrap(),
    );
    assert_eq!(
        vm_b.execute(op_path(create, "games/shared.dsk")),
        Err(ShareError::Busy)
    );
    assert_eq!(
        fs::read(&shared).unwrap(),
        b"keep me",
        "a refused create must not truncate"
    );
    vm_a.execute(ShareOp::Close { handle: reader_a }).unwrap();
    vm_b.execute(ShareOp::Close { handle: reader_b }).unwrap();

    let image = vm_a.open_image(b"games/shared.dsk").unwrap();
    assert_eq!(
        vm_b.open_image(b"games/shared.dsk").unwrap_err(),
        ShareError::Busy
    );
    assert_eq!(
        vm_b.execute(op_path(open_read, "games/shared.dsk")),
        Err(ShareError::Busy)
    );
    assert_eq!(vm_b.status().last_error, Some(ShareError::Busy));
    drop(image);
    assert!(vm_b.open_image(b"games/shared.dsk").is_ok());
}

#[test]
fn missing_and_renamed_files() {
    let scratch = ScratchDir::new("session-rename");
    scratch.file("before.txt", b"contents");
    let registry = Arc::new(AccessRegistry::new());
    let vm = session(&scratch, ShareAccess::ReadOnly, &registry);
    assert_eq!(
        vm.execute(op_path(open_read, "games/missing")),
        Err(ShareError::NotFound)
    );
    assert_eq!(
        vm.open_image(b"games/missing").unwrap_err(),
        ShareError::NotFound
    );

    let handle = handle_of(vm.execute(op_path(open_read, "games/before.txt")).unwrap());
    fs::rename(
        scratch.path().join("before.txt"),
        scratch.path().join("after.txt"),
    )
    .unwrap();
    assert_eq!(
        vm.execute(op_path(open_read, "games/before.txt")),
        Err(ShareError::NotFound)
    );
    assert_eq!(
        read_all(&vm, handle),
        b"contents",
        "handles follow the open file"
    );
    assert_eq!(vm.execute(list("games")).unwrap(), b"after.txt\n");
}

#[test]
fn a_removed_current_directory_fails_later_requests() {
    let scratch = ScratchDir::new("session-removed-cwd");
    let sub = scratch.dir("sub");
    let registry = Arc::new(AccessRegistry::new());
    let vm = session(&scratch, ShareAccess::ReadOnly, &registry);
    vm.execute(op_path(cd, "games/sub")).unwrap();
    fs::remove_dir(sub).unwrap();
    assert_eq!(vm.execute(list("")), Err(ShareError::NotFound));
    vm.execute(op_path(cd, "/")).unwrap();
}

#[test]
fn opens_require_a_file_below_a_share() {
    let scratch = ScratchDir::new("session-open-dir");
    scratch.dir("sub");
    let registry = Arc::new(AccessRegistry::new());
    let vm = session(&scratch, ShareAccess::ReadWrite, &registry);
    assert_eq!(
        vm.execute(op_path(open_read, "")),
        Err(ShareError::InvalidPath)
    );
    assert_eq!(
        vm.execute(op_path(open_read, "games")),
        Err(ShareError::IsADirectory)
    );
    assert_eq!(
        vm.execute(op_path(open_read, "/")),
        Err(ShareError::IsADirectory)
    );
    assert!(matches!(
        vm.execute(op_path(open_read, "games/sub")),
        Err(ShareError::IsADirectory)
    ));
    assert_eq!(
        vm.execute(op_path(create, "games/sub")),
        Err(ShareError::IsADirectory)
    );
}

#[test]
fn handle_slots_are_bounded_and_validated() {
    let scratch = ScratchDir::new("session-handles");
    scratch.file("f.txt", b"");
    let registry = Arc::new(AccessRegistry::new());
    let vm = session(&scratch, ShareAccess::ReadOnly, &registry);
    let handles: Vec<ShareHandle> = (0..MAX_OPEN_HANDLES)
        .map(|_| handle_of(vm.execute(op_path(open_read, "games/f.txt")).unwrap()))
        .collect();
    assert_eq!(
        vm.execute(op_path(open_read, "games/f.txt")),
        Err(ShareError::TooManyHandles)
    );
    assert_eq!(vm.status().open_handles, MAX_OPEN_HANDLES);
    let closed = handles[0];
    vm.execute(ShareOp::Close { handle: closed }).unwrap();
    assert_eq!(
        vm.execute(ShareOp::Close { handle: closed }),
        Err(ShareError::BadHandle)
    );
    for byte in [0, u8::try_from(MAX_OPEN_HANDLES + 1).unwrap(), u8::MAX] {
        let handle = ShareHandle::from_byte(byte);
        assert_eq!(
            vm.execute(ShareOp::Read { handle, max: 1 }),
            Err(ShareError::BadHandle)
        );
    }
}

#[test]
fn reset_closes_handles_and_releases_leases() {
    let scratch = ScratchDir::new("session-reset");
    scratch.file("f.txt", b"");
    let registry = Arc::new(AccessRegistry::new());
    let mut vm = session(&scratch, ShareAccess::ReadWrite, &registry);
    vm.execute(op_path(create, "games/f.txt")).unwrap();
    assert_eq!(registry.leased_files(), 1);
    vm.reset();
    assert_eq!(registry.leased_files(), 0);
    assert_eq!(
        vm.status(),
        ShareStatus {
            cwd: "/".into(),
            ..ShareStatus::default()
        }
    );
}

#[test]
fn jobs_report_share_errors_through_the_host_executor() {
    let scratch = ScratchDir::new("session-job");
    let registry = Arc::new(AccessRegistry::new());
    let vm = session(&scratch, ShareAccess::ReadOnly, &registry);
    let (mut executor, manual) = HostExecutor::manual(QUEUE_CAPACITY);
    executor
        .submit(vm.job(op_path(open_read, "games/none")))
        .unwrap();
    manual
        .complete(manual.take_request().unwrap().run())
        .unwrap();
    assert_eq!(
        executor.poll().unwrap().result,
        Err(HostError::Share(ShareError::NotFound))
    );
}
