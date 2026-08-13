//! Unit tests for the candidate-path join logic and the empty-directory
//! fallback rule. These deliberately avoid touching the real home dir or the
//! real repo-root `roms/`/`disks/`: the private `candidates` helper takes
//! `repo_root` as a parameter precisely so its join order can be checked
//! against a fake root, and `resolve_dir`/`resolve_path` (which `roms_dir`/
//! `disks_dir`/`rom`/`disk` are thin wrappers over) are checked with
//! kind/name strings that are most unlikely to exist anywhere on disk.
use super::*;

/// `candidates` must always put the repo-root candidate first, joined
/// exactly as `repo_root/<kind>` — this is the "prefer the repo-root copy"
/// contract callers rely on.
#[test]
fn candidates_puts_repo_root_candidate_first() {
    let fake_root = PathBuf::from("/fake/repo/root/for/test-assets");
    let list = candidates(&fake_root, "roms");
    assert_eq!(list[0], fake_root.join("roms"));
}

/// Every candidate — repo-root or XDG — must end in `<kind>`, never some
/// other directory name.
#[test]
fn candidates_all_end_with_kind() {
    let fake_root = PathBuf::from("/fake/repo/root/for/test-assets");
    let list = candidates(&fake_root, "disks");
    for candidate in &list {
        assert_eq!(candidate.file_name(), Some(std::ffi::OsStr::new("disks")));
    }
}

/// A `kind` that doesn't exist under either candidate root falls back to
/// the repo-root candidate, not the XDG one — so panic/skip messages keep
/// naming a path under the repo root.
#[test]
fn roms_dir_falls_back_to_repo_root_candidate_when_kind_is_absent_everywhere() {
    let kind = format!("nonexistent-kind-{}", std::process::id());
    let resolved = resolve_dir(&kind);
    assert_eq!(resolved, repo_root().join(&kind));
}

/// Same fallback contract as `resolve_dir`, but for a specific file under a
/// `kind` that doesn't exist anywhere.
#[test]
fn resolve_path_falls_back_to_repo_root_candidate_when_file_is_absent_everywhere() {
    let kind = format!("nonexistent-kind-{}", std::process::id());
    let name = "nonexistent-file.bin";
    let resolved = resolve_path(&kind, name);
    assert_eq!(resolved, repo_root().join(&kind).join(name));
}

/// The empty-dir shadowing bug the directory resolver used to have: a
/// directory that exists but holds nothing must not "qualify" as a
/// candidate, so it can't shadow a fully-populated one further down the
/// preference order.
#[test]
fn is_populated_dir_rejects_an_existing_empty_directory() {
    let dir = std::env::temp_dir().join(format!("test-assets-empty-dir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create empty test dir");

    let populated = is_populated_dir(&dir);

    let _ = std::fs::remove_dir(&dir);
    assert!(!populated, "an empty existing directory must not qualify");
}

/// The counterpart to the above: a directory holding at least one entry
/// does qualify.
#[test]
fn is_populated_dir_accepts_a_directory_with_an_entry() {
    let dir = std::env::temp_dir().join(format!("test-assets-nonempty-dir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create test dir");
    std::fs::write(dir.join("marker"), b"x").expect("write marker file");

    let populated = is_populated_dir(&dir);

    let _ = std::fs::remove_dir_all(&dir);
    assert!(populated, "a directory with an entry must qualify");
}

/// A directory that doesn't exist at all is, like an empty one, not
/// populated — [`is_populated_dir`] must not panic or otherwise treat a
/// missing path differently from an empty one.
#[test]
fn is_populated_dir_rejects_a_missing_directory() {
    let dir = std::env::temp_dir().join(format!("test-assets-missing-dir-{}", std::process::id()));
    assert!(!is_populated_dir(&dir));
}
