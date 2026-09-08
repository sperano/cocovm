//! On-demand download of the test-image bundle into `<xdg>/assets/tests`.
//! The app never fetches these: end users have no use for 40 MB of NitrOS-9
//! images, so the tests pull their own bundle the first time one is missing.

use std::path::Path;
use std::sync::OnceLock;
use std::{fs, io};

use crate::{TESTS_KIND, tests_dir};

/// Where the test bundle is fetched from unless [`TEST_ASSETS_URL_ENV`] says
/// otherwise. The tarball's entries all live under `tests/`.
pub const DEFAULT_TEST_ASSETS_URL: &str =
    "https://assets.spe.quebec/cocovm/cocovm-test-assets-v1.tgz";
/// Environment variable overriding [`DEFAULT_TEST_ASSETS_URL`]; set it to an
/// empty string to disable fetching altogether (offline CI).
pub const TEST_ASSETS_URL_ENV: &str = "COCOVM_TEST_ASSETS_URL";

/// Fetch the bundle if `path` is absent — at most once per process, so a
/// failure costs one attempt, not one per test. Never panics: on failure it
/// warns on stderr and leaves the callers to skip as they always have.
pub(crate) fn ensure_present(path: &Path) {
    if path.exists() {
        return;
    }
    static FETCHED: OnceLock<()> = OnceLock::new();
    FETCHED.get_or_init(|| {
        if let Err(e) = fetch_bundle() {
            eprintln!("test-assets: could not fetch the test bundle ({e}); disk tests will skip");
        }
    });
}

fn fetch_bundle() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var(TEST_ASSETS_URL_ENV).unwrap_or_else(|_| DEFAULT_TEST_ASSETS_URL.into());
    if url.is_empty() {
        return Ok(());
    }
    let dest = tests_dir();
    eprintln!("test-assets: downloading {url} into {}", dest.display());
    let response = ureq::get(&url).call()?;
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
    let staging = parent.join(format!(".{TESTS_KIND}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    let result = unpack_then_move(reader, &staging, dest);
    let _ = fs::remove_dir_all(&staging);
    result
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
