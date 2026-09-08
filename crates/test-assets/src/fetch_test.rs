//! Unit tests for the bundle installer, without a network.
use std::path::PathBuf;

use super::*;

/// A gzipped tar shaped like the test bundle: every entry under `tests/`.
fn bundle(files: &[(&str, &[u8])]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tarball = tar::Builder::new(gz);
    for (name, contents) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tarball
            .append_data(&mut header, format!("{TESTS_KIND}/{name}"), *contents)
            .unwrap();
    }
    tarball.into_inner().unwrap().finish().unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp-test-assets/fetch")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[test]
fn install_bundle_lands_entries_in_dest_and_removes_staging() {
    let assets = scratch("install");
    let dest = assets.join(TESTS_KIND);
    let bytes = bundle(&[("a.dsk", b"\x01\x02"), ("b.VHD", b"\x03")]);

    install_bundle(&bytes[..], &dest).unwrap();

    assert_eq!(fs::read(dest.join("a.dsk")).unwrap(), b"\x01\x02");
    assert_eq!(fs::read(dest.join("b.VHD")).unwrap(), b"\x03");
    let leftovers: Vec<_> = fs::read_dir(&assets)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, vec![std::ffi::OsString::from(TESTS_KIND)]);
}

#[test]
fn install_bundle_overwrites_existing_files_and_keeps_others() {
    let dest = scratch("overwrite").join(TESTS_KIND);
    install_bundle(&bundle(&[("a.dsk", b"old"), ("keep.dsk", b"k")])[..], &dest).unwrap();

    install_bundle(&bundle(&[("a.dsk", b"new")])[..], &dest).unwrap();

    assert_eq!(fs::read(dest.join("a.dsk")).unwrap(), b"new");
    assert_eq!(fs::read(dest.join("keep.dsk")).unwrap(), b"k");
}

#[test]
fn install_bundle_sweeps_staging_left_by_a_killed_run() {
    let assets = scratch("stale");
    let stale = assets.join(staging_name(999_999));
    fs::create_dir_all(stale.join(TESTS_KIND)).unwrap();
    fs::write(stale.join(TESTS_KIND).join("partial.VHD"), b"trunc").unwrap();

    install_bundle(&bundle(&[("a.dsk", b"a")])[..], &assets.join(TESTS_KIND)).unwrap();

    assert!(!stale.exists());
    assert!(assets.join(TESTS_KIND).join("a.dsk").exists());
}

#[test]
fn install_bundle_rejects_garbage_without_touching_dest() {
    let dest = scratch("garbage").join(TESTS_KIND);
    assert!(install_bundle(&b"not a tarball"[..], &dest).is_err());
    assert!(!dest.exists());
}
