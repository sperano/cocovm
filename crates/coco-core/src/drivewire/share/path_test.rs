use std::fs;

use super::*;
use crate::drivewire::share::ShareAccess;
use crate::drivewire::share::tests::{ScratchDir, table};

const NEVER_CANCELLED: &dyn Fn() -> bool = &|| false;

fn parse(input: &[u8], cwd: &GuestPath, table: &ShareTable) -> Result<String, ShareError> {
    GuestPath::parse(input, cwd, table).map(|path| path.to_string())
}

fn games_table(scratch: &ScratchDir) -> ShareTable {
    table(&[("Games", scratch.path(), ShareAccess::ReadOnly)])
}

fn names(entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| format!("{}{}", entry.name, if entry.is_dir { "/" } else { "" }))
        .collect()
}

#[test]
fn relative_paths_start_at_the_current_directory() {
    let scratch = ScratchDir::new("parse-rel");
    let table = games_table(&scratch);
    let cwd = GuestPath::parse(b"/games/sub", &GuestPath::top(), &table).unwrap();
    assert_eq!(
        parse(b"a/b.dsk", &cwd, &table).unwrap(),
        "/Games/sub/a/b.dsk"
    );
    assert_eq!(parse(b"", &cwd, &table).unwrap(), "/Games/sub");
    assert_eq!(parse(b"/", &cwd, &table).unwrap(), "/");
    assert_eq!(
        parse(b"/GAMES//x/./y/", &cwd, &table).unwrap(),
        "/Games/x/y"
    );
}

#[test]
fn parent_components_stop_at_the_top_level() {
    let scratch = ScratchDir::new("parse-parent");
    let table = games_table(&scratch);
    let top = GuestPath::top();
    assert_eq!(parse(b"games/a/..", &top, &table).unwrap(), "/Games");
    assert_eq!(parse(b"games/..", &top, &table).unwrap(), "/");
    assert_eq!(parse(b"../../..", &top, &table).unwrap(), "/");
    assert_eq!(
        parse(b"games/../../etc/passwd", &top, &table),
        Err(ShareError::UnknownShare)
    );
}

#[test]
fn host_absolute_paths_are_not_interpreted() {
    let scratch = ScratchDir::new("parse-abs");
    let table = games_table(&scratch);
    let top = GuestPath::top();
    let root = scratch.path().to_str().unwrap().as_bytes().to_vec();
    assert_eq!(parse(&root, &top, &table), Err(ShareError::UnknownShare));
    assert_eq!(
        parse(b"/etc/passwd", &top, &table),
        Err(ShareError::UnknownShare)
    );
    assert_eq!(
        parse(b"C:\\Windows", &top, &table),
        Err(ShareError::InvalidPath)
    );
    assert_eq!(
        parse(b"games/C:x", &top, &table),
        Err(ShareError::InvalidPath)
    );
}

#[test]
fn components_outside_the_guest_alphabet_are_rejected() {
    let scratch = ScratchDir::new("parse-bytes");
    let table = games_table(&scratch);
    let top = GuestPath::top();
    for bad in [
        &b"games/a\\b"[..],
        b"games/\x00",
        b"games/tab\there",
        b"games/\x80semigraphic",
        b"games/ lead",
        b"games/trail ",
        b"games/wild*",
        b"games/q?",
        "games/é".as_bytes(),
    ] {
        assert_eq!(
            parse(bad, &top, &table),
            Err(ShareError::InvalidPath),
            "{bad:?}"
        );
    }
    assert_eq!(
        parse(b"games/My Disk.dsk", &top, &table).unwrap(),
        "/Games/My Disk.dsk"
    );
}

#[test]
fn overlong_paths_are_rejected() {
    let scratch = ScratchDir::new("parse-long");
    let table = games_table(&scratch);
    let long = format!("games/{}", "a".repeat(MAX_GUEST_PATH_LEN));
    assert_eq!(
        parse(long.as_bytes(), &GuestPath::top(), &table),
        Err(ShareError::InvalidPath)
    );
}

#[test]
fn existing_paths_resolve_inside_the_canonical_root() {
    let scratch = ScratchDir::new("resolve");
    let file = scratch.file("dir/disk.dsk", b"x");
    let table = games_table(&scratch);
    let guest = GuestPath::parse(b"games/dir/disk.dsk", &GuestPath::top(), &table).unwrap();
    let resolved = resolve_existing(&guest, &table).unwrap();
    assert_eq!(resolved.host, fs::canonicalize(file).unwrap());
    assert_eq!(resolved.access, ShareAccess::ReadOnly);
}

#[test]
fn missing_files_and_file_parents_fail_to_resolve() {
    let scratch = ScratchDir::new("resolve-missing");
    scratch.file("plain", b"x");
    let table = games_table(&scratch);
    let top = GuestPath::top();
    let missing = GuestPath::parse(b"games/nope", &top, &table).unwrap();
    assert_eq!(
        resolve_existing(&missing, &table).unwrap_err(),
        ShareError::NotFound
    );
    let below_file = GuestPath::parse(b"games/plain/x", &top, &table).unwrap();
    assert!(matches!(
        resolve_existing(&below_file, &table),
        Err(ShareError::NotADirectory | ShareError::NotFound)
    ));
}

#[test]
fn a_removed_renamed_or_replaced_root_is_unavailable() {
    let scratch = ScratchDir::new("resolve-root");
    let root = scratch.dir("root");
    let table = table(&[("games", &root, ShareAccess::ReadOnly)]);
    let guest = GuestPath::parse(b"games", &GuestPath::top(), &table).unwrap();
    assert!(resolve_existing(&guest, &table).is_ok());

    fs::rename(&root, scratch.path().join("renamed")).unwrap();
    assert_eq!(
        resolve_existing(&guest, &table).unwrap_err(),
        ShareError::RootUnavailable
    );
    fs::write(&root, b"now a file").unwrap();
    assert_eq!(
        resolve_existing(&guest, &table).unwrap_err(),
        ShareError::RootUnavailable
    );
    assert_eq!(
        list(&guest, &table, NEVER_CANCELLED).unwrap_err(),
        ShareError::RootUnavailable
    );
}

#[test]
fn new_files_need_an_existing_parent_inside_the_share() {
    let scratch = ScratchDir::new("resolve-new");
    scratch.dir("sub");
    let table = games_table(&scratch);
    let top = GuestPath::top();
    let new = GuestPath::parse(b"games/sub/new.bin", &top, &table).unwrap();
    assert_eq!(
        resolve_new(&new, &table).unwrap().host,
        fs::canonicalize(scratch.path().join("sub"))
            .unwrap()
            .join("new.bin")
    );
    let orphan = GuestPath::parse(b"games/none/new.bin", &top, &table).unwrap();
    assert_eq!(
        resolve_new(&orphan, &table).unwrap_err(),
        ShareError::NotFound
    );
    let dir = GuestPath::parse(b"games/sub", &top, &table).unwrap();
    assert_eq!(
        resolve_new(&dir, &table).unwrap_err(),
        ShareError::IsADirectory
    );
    let root = GuestPath::parse(b"games", &top, &table).unwrap();
    assert_eq!(
        resolve_new(&root, &table).unwrap_err(),
        ShareError::IsADirectory
    );
}

#[cfg(unix)]
mod symlinks {
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn links_inside_the_root_are_followed() {
        let scratch = ScratchDir::new("link-inside");
        let target = scratch.file("real/disk.dsk", b"x");
        symlink(scratch.path().join("real"), scratch.path().join("alias")).unwrap();
        let table = games_table(&scratch);
        let guest = GuestPath::parse(b"games/alias/disk.dsk", &GuestPath::top(), &table).unwrap();
        assert_eq!(
            resolve_existing(&guest, &table).unwrap().host,
            fs::canonicalize(target).unwrap()
        );
    }

    #[test]
    fn links_that_leave_the_root_are_escapes() {
        let outside = ScratchDir::new("link-outside-target");
        outside.file("secret.txt", b"secret");
        let scratch = ScratchDir::new("link-outside");
        let root = scratch.dir("root");
        symlink(outside.path(), root.join("out")).unwrap();
        symlink(outside.path().join("secret.txt"), root.join("secret")).unwrap();
        let table = table(&[("games", &root, ShareAccess::ReadWrite)]);
        let top = GuestPath::top();
        for path in [&b"games/out/secret.txt"[..], b"games/secret", b"games/out"] {
            let guest = GuestPath::parse(path, &top, &table).unwrap();
            assert_eq!(
                resolve_existing(&guest, &table).unwrap_err(),
                ShareError::Escape
            );
        }
        let through_dir = GuestPath::parse(b"games/out/new.txt", &top, &table).unwrap();
        assert_eq!(
            resolve_new(&through_dir, &table).unwrap_err(),
            ShareError::Escape
        );
        let over_link = GuestPath::parse(b"games/secret", &top, &table).unwrap();
        assert_eq!(
            resolve_new(&over_link, &table).unwrap_err(),
            ShareError::Escape
        );
    }

    #[test]
    fn dangling_links_cannot_create_files() {
        let scratch = ScratchDir::new("link-dangling");
        let root = scratch.dir("root");
        symlink(scratch.path().join("elsewhere.bin"), root.join("dangling")).unwrap();
        let table = table(&[("games", &root, ShareAccess::ReadWrite)]);
        let guest = GuestPath::parse(b"games/dangling", &GuestPath::top(), &table).unwrap();
        assert_eq!(
            resolve_existing(&guest, &table).unwrap_err(),
            ShareError::NotFound
        );
        assert_eq!(resolve_new(&guest, &table).unwrap_err(), ShareError::Escape);
        assert!(!scratch.path().join("elsewhere.bin").exists());
    }

    #[test]
    fn listings_omit_escaping_and_dangling_links() {
        let outside = ScratchDir::new("list-links-target");
        let scratch = ScratchDir::new("list-links");
        let root = scratch.dir("root");
        scratch.file("root/inside.txt", b"x");
        symlink(root.join("inside.txt"), root.join("alias.txt")).unwrap();
        symlink(outside.path(), root.join("escape")).unwrap();
        symlink(root.join("missing"), root.join("dangling")).unwrap();
        let table = table(&[("games", &root, ShareAccess::ReadOnly)]);
        let guest = GuestPath::parse(b"games", &GuestPath::top(), &table).unwrap();
        let entries = list(&guest, &table, NEVER_CANCELLED).unwrap();
        assert_eq!(names(&entries), ["alias.txt", "inside.txt"]);
    }

    #[test]
    fn listings_omit_names_the_guest_cannot_spell() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let scratch = ScratchDir::new("list-names");
        scratch.file("ok.txt", b"x");
        scratch.file("café.txt", b"x");
        scratch.file("colon:name", b"x");
        let raw = OsStr::from_bytes(b"latin1-\xE9.txt");
        fs::write(scratch.path().join(raw), b"x").unwrap();
        let table = games_table(&scratch);
        let guest = GuestPath::parse(b"games", &GuestPath::top(), &table).unwrap();
        assert_eq!(
            names(&list(&guest, &table, NEVER_CANCELLED).unwrap()),
            ["ok.txt"]
        );
    }
}

#[test]
fn the_top_level_lists_the_shares_as_directories() {
    let scratch = ScratchDir::new("list-top");
    let table = table(&[
        ("zeta", scratch.path(), ShareAccess::ReadOnly),
        ("alpha", scratch.path(), ShareAccess::ReadWrite),
    ]);
    let entries = list(&GuestPath::top(), &table, NEVER_CANCELLED).unwrap();
    assert_eq!(names(&entries), ["alpha/", "zeta/"]);
}

#[test]
fn listings_are_sorted_and_mark_directories() {
    let scratch = ScratchDir::new("list-sort");
    scratch.file("b.dsk", b"x");
    scratch.file("a.dsk", b"x");
    scratch.dir("Cdir");
    let table = games_table(&scratch);
    let guest = GuestPath::parse(b"games", &GuestPath::top(), &table).unwrap();
    let entries = list(&guest, &table, NEVER_CANCELLED).unwrap();
    assert_eq!(names(&entries), ["Cdir/", "a.dsk", "b.dsk"]);
}

#[test]
fn oversized_and_cancelled_listings_stop() {
    let scratch = ScratchDir::new("list-big");
    for i in 0..=MAX_DIR_ENTRIES {
        scratch.file(&format!("f{i}"), b"");
    }
    let table = games_table(&scratch);
    let guest = GuestPath::parse(b"games", &GuestPath::top(), &table).unwrap();
    assert_eq!(
        list(&guest, &table, NEVER_CANCELLED).unwrap_err(),
        ShareError::DirectoryTooLarge
    );
    assert_eq!(
        list(&guest, &table, &|| true).unwrap_err(),
        ShareError::Io(std::io::ErrorKind::Interrupted)
    );
}
