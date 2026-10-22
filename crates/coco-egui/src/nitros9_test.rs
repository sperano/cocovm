use std::fs;
use std::io::{Cursor, Write};
use std::thread;
use std::time::Duration;

use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;

use super::*;
use crate::machine_def::tests::TempDir;

const BOOT_BYTES: &[u8] = b"boot floppy";
const VHD_BYTES: &[u8] = b"system disk";
const NOTES: &str = "NitrOS9_Ease_of_Use-VERSION1.0.1_UPDATE_NOTES.rtf";

/// A URL no download can fetch, for calls that must not reach the network.
const NO_NETWORK_URL: &str = "unreachable://eou.invalid/archive.zip";

/// How long the lock test gives a blocked download to (wrongly) finish.
const LOCK_WAIT: Duration = Duration::from_millis(100);

/// A directory already holding both installed images.
fn installed_dir(name: &str) -> TempDir {
    let dir = TempDir::new(name);
    fs::write(dir.path().join(EOU_BOOT_DISK), BOOT_BYTES).unwrap();
    fs::write(dir.path().join(EOU_SYSTEM_VHD), VHD_BYTES).unwrap();
    dir
}

/// A stored (uncompressed) zip holding `entries` in order.
fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// The release's layout: both images among the documentation.
fn eou_zip() -> Vec<u8> {
    zip_of(&[
        (EOU_BOOT_DISK, BOOT_BYTES),
        (EOU_SYSTEM_VHD, VHD_BYTES),
        (NOTES, b"notes"),
    ])
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Every file `dir` holds, by name, sorted.
fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// A matching archive installs exactly the two images, byte for byte, and
/// leaves the documentation out.
#[test]
fn unpack_keeps_only_the_images() {
    let dest = TempDir::new("eou-unpack");
    let archive = eou_zip();

    unpack_eou(archive.as_slice(), dest.path(), &sha256_hex(&archive)).unwrap();

    assert_eq!(file_names(dest.path()), [EOU_BOOT_DISK, EOU_SYSTEM_VHD]);
    assert_eq!(
        fs::read(dest.path().join(EOU_BOOT_DISK)).unwrap(),
        BOOT_BYTES
    );
    assert_eq!(
        fs::read(dest.path().join(EOU_SYSTEM_VHD)).unwrap(),
        VHD_BYTES
    );
    assert!(missing_eou_images(dest.path()).is_empty());
}

/// A digest mismatch installs nothing and cleans up the partial files.
#[test]
fn unpack_rejects_an_unexpected_archive() {
    let dest = TempDir::new("eou-unpack-digest");
    let archive = eou_zip();

    let error = unpack_eou(archive.as_slice(), dest.path(), &sha256_hex(b"other")).unwrap_err();

    assert!(error.to_string().contains("unexpected"), "{error}");
    assert!(file_names(dest.path()).is_empty());
}

/// An archive without one of the images fails and names the absent one.
#[test]
fn unpack_requires_both_images() {
    let dest = TempDir::new("eou-unpack-missing");
    let archive = zip_of(&[(EOU_BOOT_DISK, BOOT_BYTES)]);

    let error = unpack_eou(archive.as_slice(), dest.path(), &sha256_hex(&archive)).unwrap_err();

    assert!(error.to_string().contains(EOU_SYSTEM_VHD), "{error}");
    assert!(file_names(dest.path()).is_empty());
}

/// A download cut short is not a zip that ends cleanly.
#[test]
fn unpack_rejects_a_truncated_archive() {
    let dest = TempDir::new("eou-unpack-truncated");
    let archive = eou_zip();
    let truncated = &archive[..archive.len() / 2];

    unpack_eou(truncated, dest.path(), &sha256_hex(&archive)).unwrap_err();

    assert!(file_names(dest.path()).is_empty());
}

/// Only absent images count as missing.
#[test]
fn missing_images_lists_absent_files() {
    let dir = TempDir::new("eou-missing");
    assert_eq!(missing_eou_images(dir.path()), EOU_IMAGES);

    fs::write(dir.path().join(EOU_BOOT_DISK), BOOT_BYTES).unwrap();
    assert_eq!(missing_eou_images(dir.path()), [EOU_SYSTEM_VHD]);
}

/// With both images already installed (by another instance that held the
/// lock first), the download fetches nothing.
#[test]
fn download_skips_installed_images() {
    let dest = installed_dir("eou-installed");
    download_eou(NO_NETWORK_URL, dest.path(), Arc::default()).unwrap();
    assert_eq!(
        fs::read(dest.path().join(EOU_SYSTEM_VHD)).unwrap(),
        VHD_BYTES
    );
}

/// A download waits while another holds the directory's lock.
#[test]
fn download_waits_for_the_lock() {
    let dest = installed_dir("eou-locked");
    let held = lock_dest(dest.path()).unwrap();
    let path = dest.path().to_path_buf();

    let waiting = thread::spawn(move || {
        download_eou(NO_NETWORK_URL, &path, Arc::default()).map_err(|e| e.to_string())
    });
    thread::sleep(LOCK_WAIT);
    assert!(!waiting.is_finished(), "must wait for the lock");

    drop(held);
    assert_eq!(waiting.join().unwrap(), Ok(()));
}
