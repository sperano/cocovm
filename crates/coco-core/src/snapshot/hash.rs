//! Hash helpers (file IO allowed here only): SHA-256 of bytes or a file, and
//! [`MediaRef`] verification against the filesystem.

use std::fmt::Write;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::payload::MediaRef;

/// Read buffer size for [`sha256_file`]'s streamed hash — large media files
/// (VHDs can run to hundreds of MB) must never be read whole into memory
/// just to hash them.
const SHA256_READ_BUF_LEN: usize = 8 * 1024;

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_lower(&hasher.finalize())
}

/// Lowercase hex SHA-256 of the file at `path`, streamed in
/// [`SHA256_READ_BUF_LEN`]-byte chunks rather than read whole into memory —
/// VHD images can run to hundreds of MB.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; SHA256_READ_BUF_LEN];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Outcome of checking a [`MediaRef`] against the file it names, right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaCheck {
    /// The file exists and still hashes to `sha256`.
    Ok,
    /// The file exists but hashes to something else — the frontend should
    /// offer "load with warning".
    Mismatch { actual: String },
    /// The file doesn't exist, or couldn't be read for any other reason —
    /// the frontend should treat this as an error (prompt to re-locate it).
    Missing,
}

impl MediaRef {
    /// Check this reference against the file it names on the current
    /// filesystem. Any read failure (missing file, permission error, ...) is
    /// reported as [`MediaCheck::Missing`] — from the caller's point of view
    /// "can't verify this" and "it's not there" call for the same response.
    pub fn verify(&self) -> MediaCheck {
        match sha256_file(&self.path) {
            Ok(actual) if actual == self.sha256 => MediaCheck::Ok,
            Ok(actual) => MediaCheck::Mismatch { actual },
            Err(_) => MediaCheck::Missing,
        }
    }
}
