//! The NitrOS-9 Ease of Use (EOU) images the NitrOS-9 EOU machine template
//! (`manager::presets`) copies into each new machine. They aren't part of
//! the app's asset bundle: the first-run asset dialog (`manager::assets`)
//! fetches L. Curtis Boyle's EOU 1.0.1 release archive next to the bundle,
//! keeps only the two emulator images, and caches them under
//! `paths::eou_dir`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::download::{self, Progress};

/// The EOU release [`EOU_URL`] serves, as a literal so `concat!` can build
/// the names that carry it.
macro_rules! eou_version {
    () => {
        "1.0.1"
    };
}
pub(crate) use eou_version;

/// The release's name, as the asset dialog shows it.
pub(crate) const EOU_TITLE: &str = concat!("NitrOS-9 Ease of Use ", eou_version!());

/// Name of `paths::eou_dir` under the assets directory. Versioned, so a
/// later EOU release lands in a fresh directory and gets downloaded again.
pub(crate) const EOU_DIR_NAME: &str = concat!("nitros9-eou-", eou_version!());

/// L. Curtis Boyle's EOU release for the 6809 (the CoCo 3's stock CPU),
/// where `--eou-url` (`COCOVM_EOU_URL`) points unless overridden. Must
/// never contain `"` or `\` — `config::default_config_template`
/// interpolates it unescaped into a quoted TOML string.
pub(crate) const EOU_URL: &str =
    "https://www.lcurtisboyle.com/nitros9/EOU_Version1_0_1_(6809_ONLY)_04-07-2024.zip";

/// Size of the archive at [`EOU_URL`], as its `Content-Length` reported on
/// 2026-10-22; the asset dialog quotes it before the download starts.
pub(crate) const EOU_ARCHIVE_BYTES: u64 = 58_226_559;

/// SHA-256 of the archive at [`EOU_URL`] ([`EOU_ARCHIVE_BYTES`], served with
/// `Last-Modified: Wed, 02 Oct 2024 17:22:45 GMT`). An archive that doesn't
/// match installs nothing, so a changed or truncated download never ends up
/// in a machine.
pub(crate) const EOU_SHA256: &str =
    "4a1394c86186c943f66ed99046609dbed97a31650ee9e828c3c6e5a0a9c73184";

/// The 180K boot floppy: its OS9Boot carries the emulator hard-disk driver
/// and the `/h0` descriptors, so it boots from Disk BASIC's `DOS`.
pub(crate) const EOU_BOOT_DISK: &str = "68EMU.dsk";

/// The 128 MB system disk the boot floppy's startup runs from, on the
/// emulator VHD interface.
pub(crate) const EOU_SYSTEM_VHD: &str = "68SDC.VHD";

/// Every file [`unpack_eou`] keeps from the archive. The rest (release
/// notes and manuals) is skipped.
pub(crate) const EOU_IMAGES: [&str; 2] = [EOU_BOOT_DISK, EOU_SYSTEM_VHD];

/// Appended to an image's name while it is written, so a failed or
/// unverified download never leaves a file under its real name.
const PARTIAL_SUFFIX: &str = ".part";

/// The file under the EOU directory whose lock [`download_eou`] holds, so
/// two app instances never write the same [`PARTIAL_SUFFIX`] files at once.
const LOCK_FILE: &str = ".download.lock";

/// The [`EOU_IMAGES`] not present as files under `dir`.
pub(crate) fn missing_eou_images(dir: &Path) -> Vec<&'static str> {
    EOU_IMAGES
        .into_iter()
        .filter(|name| !dir.join(name).is_file())
        .collect()
}

/// Download the archive at `url` into `dest`, reporting into `progress`,
/// and keep its [`EOU_IMAGES`] once its digest matches [`EOU_SHA256`].
/// Holds `dest`'s [`lock_dest`] throughout: an instance that waited on it
/// finds the images another one installed meanwhile and downloads nothing.
pub(crate) fn download_eou(
    url: &str,
    dest: &Path,
    progress: Arc<Progress>,
) -> Result<(), Box<dyn std::error::Error>> {
    let _lock = lock_dest(dest)?;
    if missing_eou_images(dest).is_empty() {
        return Ok(());
    }
    unpack_eou(download::open(url, progress)?, dest, EOU_SHA256)
}

/// Create `dest`, open its [`LOCK_FILE`], and wait for an exclusive lock on
/// it; dropping the file releases the lock, and so does a crash. When the
/// lock can't be taken (a platform or NFS mount without locks), the
/// download warns and goes ahead unlocked, as starter-machine seeding does
/// (`manager::presets::lock_seeding`): the race needs two instances
/// downloading at once.
fn lock_dest(dest: &Path) -> Result<File, Box<dyn std::error::Error>> {
    fs::create_dir_all(dest).map_err(|e| path_error(dest, e))?;
    let path = dest.join(LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| path_error(&path, e))?;
    if let Err(e) = file.lock() {
        eprintln!(
            "coco: {}: locking: {e}; downloading NitrOS-9 EOU unlocked",
            path.display()
        );
    }
    Ok(file)
}

/// Stream the zip archive in `reader` and write its [`EOU_IMAGES`] into
/// `dest`, but only once the whole stream's SHA-256 matches `sha256`
/// (lowercase hex). No image takes its real name before the whole archive
/// checks out, and a failure removes the partial files. Split from the
/// download so it can be tested without a network; the caller serializes
/// calls for one `dest` ([`download_eou`]'s lock), since they share the
/// partial files.
pub(crate) fn unpack_eou(
    reader: impl Read,
    dest: &Path,
    sha256: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(dest)?;
    let mut reader = HashingReader::new(reader);
    let verified = extract_images(&mut reader, dest).and_then(|()| {
        // The central directory follows the last entry; hash it too.
        io::copy(&mut reader, &mut io::sink())?;
        let digest = reader.into_hex_digest();
        if digest == sha256 {
            Ok(())
        } else {
            Err(format!("unexpected NitrOS-9 EOU archive (SHA-256 {digest})").into())
        }
    });
    let installed = verified.and_then(|()| {
        EOU_IMAGES.into_iter().try_for_each(|name| {
            let target = dest.join(name);
            fs::rename(partial_path(dest, name), &target).map_err(|e| path_error(&target, e))
        })
    });
    if installed.is_err() {
        for name in EOU_IMAGES {
            // Best effort: an image that was never written is already gone.
            let _ = fs::remove_file(partial_path(dest, name));
        }
    }
    installed
}

/// Write each [`EOU_IMAGES`] entry of the zip stream in `reader` to its
/// [`partial_path`] under `dest`, and fail when one is absent.
fn extract_images(reader: &mut impl Read, dest: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut found = Vec::with_capacity(EOU_IMAGES.len());
    while let Some(mut entry) = zip::read::read_zipfile_from_stream(reader)? {
        let wanted = {
            let entry_name = entry.name()?;
            EOU_IMAGES.into_iter().find(|&name| name == entry_name)
        };
        if let Some(name) = wanted {
            let path = partial_path(dest, name);
            File::create(&path)
                .and_then(|mut file| io::copy(&mut entry, &mut file))
                .map_err(|e| path_error(&path, e))?;
            found.push(name);
        }
        // Dropping `entry` skips whatever of it is left unread.
    }
    match EOU_IMAGES.into_iter().find(|name| !found.contains(name)) {
        Some(name) => Err(format!("the NitrOS-9 EOU archive has no {name}").into()),
        None => Ok(()),
    }
}

/// `error` from an operation on `path`, naming the path.
fn path_error(path: &Path, error: io::Error) -> Box<dyn std::error::Error> {
    format!("{}: {error}", path.display()).into()
}

/// Where `name` is written under `dest` until the archive checks out.
fn partial_path(dest: &Path, name: &str) -> PathBuf {
    dest.join(format!("{name}{PARTIAL_SUFFIX}"))
}

/// A reader that feeds every byte read from `inner` to a SHA-256 hasher.
struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
}

impl<R> HashingReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    /// Lowercase hex SHA-256 of everything read.
    fn into_hex_digest(self) -> String {
        format!("{:x}", self.hasher.finalize())
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
}

#[cfg(test)]
#[path = "nitros9_test.rs"]
mod tests;
