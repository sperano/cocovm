use std::fs::File;
use std::sync::Arc;

use super::*;
use crate::drivewire::share::tests::ScratchDir;

fn open(path: &Path) -> File {
    File::open(path).expect("open scratch file")
}

fn lease(
    registry: &Arc<AccessRegistry>,
    path: &Path,
    owner: LeaseOwner,
    mode: AccessMode,
) -> Result<Lease, ShareError> {
    registry.acquire(&open(path), path, owner, mode)
}

#[test]
fn many_owners_may_read_one_file() {
    let scratch = ScratchDir::new("access-read");
    let path = scratch.file("disk.dsk", b"x");
    let registry = Arc::new(AccessRegistry::new());
    let first = lease(&registry, &path, LeaseOwner::new(), AccessMode::Read).unwrap();
    let second = lease(&registry, &path, LeaseOwner::new(), AccessMode::Read).unwrap();
    assert_eq!(registry.leased_files(), 1);
    drop((first, second));
    assert_eq!(registry.leased_files(), 0);
}

#[test]
fn a_writer_excludes_other_owners_both_ways() {
    let scratch = ScratchDir::new("access-write");
    let path = scratch.file("disk.dsk", b"x");
    let registry = Arc::new(AccessRegistry::new());
    let (vm_a, vm_b) = (LeaseOwner::new(), LeaseOwner::new());

    let writer = lease(&registry, &path, vm_a, AccessMode::Write).unwrap();
    for mode in [AccessMode::Read, AccessMode::Write] {
        assert_eq!(
            lease(&registry, &path, vm_b, mode).unwrap_err(),
            ShareError::Busy
        );
    }
    drop(writer);

    let reader = lease(&registry, &path, vm_b, AccessMode::Read).unwrap();
    assert_eq!(
        lease(&registry, &path, vm_a, AccessMode::Write).unwrap_err(),
        ShareError::Busy
    );
    drop(reader);
    assert!(lease(&registry, &path, vm_a, AccessMode::Write).is_ok());
}

#[test]
fn one_owner_never_conflicts_with_itself() {
    let scratch = ScratchDir::new("access-self");
    let path = scratch.file("disk.dsk", b"x");
    let registry = Arc::new(AccessRegistry::new());
    let owner = LeaseOwner::new();
    let write = lease(&registry, &path, owner, AccessMode::Write).unwrap();
    let again = lease(&registry, &path, owner, AccessMode::Write).unwrap();
    let read = lease(&registry, &path, owner, AccessMode::Read).unwrap();
    drop(write);
    assert_eq!(
        lease(&registry, &path, LeaseOwner::new(), AccessMode::Read).unwrap_err(),
        ShareError::Busy,
        "the remaining write lease still holds the file"
    );
    drop((again, read));
    assert_eq!(registry.leased_files(), 0);
}

#[test]
fn different_spellings_of_one_path_share_a_lease() {
    let scratch = ScratchDir::new("access-spelling");
    let path = scratch.file("sub/disk.dsk", b"x");
    let detour = scratch.path().join("sub/../sub/./disk.dsk");
    let registry = Arc::new(AccessRegistry::new());
    let _writer = lease(&registry, &path, LeaseOwner::new(), AccessMode::Write).unwrap();
    assert_eq!(
        lease(&registry, &detour, LeaseOwner::new(), AccessMode::Read).unwrap_err(),
        ShareError::Busy
    );
}

#[cfg(unix)]
#[test]
fn hard_links_share_a_lease_on_unix() {
    let scratch = ScratchDir::new("access-hardlink");
    let path = scratch.file("disk.dsk", b"x");
    let link = scratch.path().join("link.dsk");
    std::fs::hard_link(&path, &link).unwrap();
    let registry = Arc::new(AccessRegistry::new());
    let _writer = lease(&registry, &path, LeaseOwner::new(), AccessMode::Write).unwrap();
    assert_eq!(
        lease(&registry, &link, LeaseOwner::new(), AccessMode::Read).unwrap_err(),
        ShareError::Busy
    );
}

#[test]
fn separate_registries_do_not_coordinate() {
    let scratch = ScratchDir::new("access-registries");
    let path = scratch.file("disk.dsk", b"x");
    let first = Arc::new(AccessRegistry::new());
    let second = Arc::new(AccessRegistry::new());
    let _a = lease(&first, &path, LeaseOwner::new(), AccessMode::Write).unwrap();
    assert!(lease(&second, &path, LeaseOwner::new(), AccessMode::Write).is_ok());
}
