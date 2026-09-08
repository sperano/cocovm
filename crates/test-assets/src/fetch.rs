//! On-demand download of the test-image bundle into `<xdg>/assets/tests`.
//! The app never fetches these: end users have no use for 40 MB of NitrOS-9
//! images, so the tests pull their own bundle the first time one is missing.

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;
use std::{fs, io};

use crate::{TESTS_KIND, tests_dir};

/// Where the test bundle is fetched from unless [`TEST_ASSETS_URL_ENV`] says
/// otherwise. The tarball's entries all live under `tests/`.
pub const DEFAULT_TEST_ASSETS_URL: &str =
    "https://assets.spe.quebec/cocovm/cocovm-test-assets-v1.tgz";
/// Environment variable overriding [`DEFAULT_TEST_ASSETS_URL`]; set it to an
/// empty string to disable fetching altogether (offline CI).
pub const TEST_ASSETS_URL_ENV: &str = "COCOVM_TEST_ASSETS_URL";

/// Advisory lock file beside the tests directory, serializing concurrent
/// test processes (nextest runs one per test) so only the first downloads.
const LOCK_FILE: &str = ".tests.lock";
/// Give up on an unreachable host quickly rather than hanging the suite.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Upper bound on the whole download — 40 MB on a slow link, not forever.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Fetch the bundle if `path` is absent — at most once per process, so a
/// failure costs one attempt, not one per test. Never panics: on failure it
/// warns on stderr and leaves the callers to skip as they always have.
pub(crate) fn ensure_present(path: &Path) {
    if path.exists() {
        return;
    }
    static FETCHED: OnceLock<()> = OnceLock::new();
    FETCHED.get_or_init(|| {
        if let Err(e) = fetch_bundle_locked(path) {
            eprintln!("test-assets: could not fetch the test bundle ({e}); disk tests will skip");
        }
    });
}

/// Take the process-wide lock, then re-check: a process that waited on the
/// lock usually finds the file installed by the one that held it.
fn fetch_bundle_locked(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var(TEST_ASSETS_URL_ENV).unwrap_or_else(|_| DEFAULT_TEST_ASSETS_URL.into());
    if url.is_empty() {
        return Ok(());
    }
    let dest = tests_dir();
    let parent = dest.parent().ok_or("tests dir has no parent")?;
    fs::create_dir_all(parent)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(parent.join(LOCK_FILE))?;
    lock.lock()?;
    if path.exists() {
        return Ok(());
    }
    eprintln!("test-assets: downloading {url} into {}", dest.display());
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_global(Some(FETCH_TIMEOUT))
            .build(),
    );
    let response = agent.get(&url).call()?;
    install_bundle(response.into_body().into_reader(), &dest)?;
    Ok(())
}

/// Unpack a gzipped tar whose entries live under `tests/` so that they end
/// up directly in `dest`. Unpacks into a sibling staging directory first and
/// renames file by file, so an interrupted download never leaves a
/// truncated image that the next run would mistake for a good one.
pub(crate) fn install_bundle(reader: impl io::Read, dest: &Path) -> io::Result<()> {
    let parent = dest
        .parent()
        .ok_or_else(|| io::Error::other("tests dir has no parent"))?;
    fs::create_dir_all(parent)?;
    remove_stale_staging(parent)?;
    let staging = parent.join(staging_name(std::process::id()));
    let result = unpack_then_move(reader, &staging, dest);
    let _ = fs::remove_dir_all(&staging);
    result
}

/// `.tests-`: what every staging directory name starts with.
fn staging_prefix() -> String {
    format!(".{TESTS_KIND}-")
}

/// `.tests-<pid>`: the staging directory a fetching process unpacks into.
fn staging_name(pid: u32) -> String {
    format!("{}{pid}", staging_prefix())
}

/// Delete staging directories left by runs that were killed mid-download —
/// fetches are serialized, so any that exist are dead.
fn remove_stale_staging(parent: &Path) -> io::Result<()> {
    let prefix = staging_prefix();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

fn unpack_then_move(reader: impl io::Read, staging: &Path, dest: &Path) -> io::Result<()> {
    tar::Archive::new(flate2::read::GzDecoder::new(reader)).unpack(staging)?;
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(staging.join(TESTS_KIND))? {
        let entry = entry?;
        fs::rename(entry.path(), dest.join(entry.file_name()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "fetch_test.rs"]
mod tests;
