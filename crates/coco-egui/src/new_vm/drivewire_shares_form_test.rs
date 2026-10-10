use std::fs;

use super::*;
use crate::machine_def::tests::TempDir;

fn share(name: &str, path: &str) -> DriveWireShareDTO {
    DriveWireShareDTO {
        name: name.into(),
        path: path.into(),
        access: ShareAccessDTO::ReadOnly,
    }
}

/// `share`'s problem when it is the only share.
fn lone_problem(share: &DriveWireShareDTO, roots: &mut ShareRootChecks) -> Option<String> {
    share_problem(share, std::slice::from_ref(share), "slug", roots)
}

#[test]
fn new_share_names_skip_names_in_use_ignoring_case() {
    assert_eq!(unused_share_name(&[]), "share1");
    let taken = [share("SHARE1", ""), share("share2", "")];
    assert_eq!(unused_share_name(&taken), "share3");
}

#[test]
fn problems_report_names_before_folders() {
    let dir = TempDir::new("share-form-problems");
    let folder = dir.path().display().to_string();
    let mut roots = ShareRootChecks::default();
    let bad = share("a b", &folder);
    assert!(
        lone_problem(&bad, &mut roots)
            .unwrap()
            .contains("letters, digits")
    );
    let all = [share("games", &folder), share("Games", &folder)];
    assert!(
        share_problem(&all[0], &all, "slug", &mut roots)
            .unwrap()
            .contains("used twice")
    );
    let empty = share("games", " ");
    assert_eq!(
        lone_problem(&empty, &mut roots).as_deref(),
        Some(NO_FOLDER_WARNING)
    );
    let fine = share("games", &folder);
    assert_eq!(lone_problem(&fine, &mut roots), None);
}

#[test]
fn missing_folders_and_files_are_reported() {
    let dir = TempDir::new("share-form-roots");
    let file = dir.path().join("file.txt");
    fs::write(&file, b"").unwrap();
    let missing = dir.path().join("missing");
    assert!(root_problem(&file).unwrap().contains("is not a folder"));
    assert!(root_problem(&missing).unwrap().contains("not found"));
    assert_eq!(root_problem(dir.path()), None);
}

#[test]
fn folder_checks_are_cached_until_they_expire() {
    let dir = TempDir::new("share-form-cache");
    let root = dir.path().join("root");
    fs::create_dir(&root).unwrap();
    let mut checks = ShareRootChecks::default();
    assert_eq!(checks.problem(&root), None);
    fs::remove_dir(&root).unwrap();
    assert_eq!(checks.problem(&root), None, "a fresh check is reused");

    let stale = Instant::now() - ROOT_RECHECK_INTERVAL;
    checks.checked.get_mut(&root).unwrap().0 = stale;
    assert!(checks.problem(&root).is_some());
    checks.retain(&[]);
    assert!(checks.checked.is_empty());
}
