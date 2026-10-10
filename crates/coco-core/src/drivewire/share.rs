//! Host folder shares for DriveWire guest services.
//!
//! A VM's settings name host folders ("shares") and grant each one
//! read-only or read/write access. Guest services — `dw server dir`,
//! `dw server list`, `dw disk insert`, named objects, or a later filesystem
//! service — reach host files only through these shares, using one
//! resolution path ([`path`]) and one set of access rules ([`access`]).
//!
//! Guest paths name a virtual tree. Its top level (`/`) lists the VM's
//! shares; `SHARE/dir/file` descends into one. Host absolute paths are never
//! interpreted, `.` and `..` are resolved lexically and cannot climb above
//! the top level, and symlinks are followed only while their target stays
//! inside the share's root. Each VM's [`ShareSession`] keeps its own current
//! directory and open handles, even when two VMs name the same host folder.
//! The guest-facing wire behavior these shares serve is the NitrOS-9 `dw`
//! command contract described in the DriveWire capability notes; see
//! [`command_code`] for its result codes.

pub mod access;
pub mod path;
mod session;

use std::fmt;
use std::io;
use std::path::PathBuf;

pub use access::{AccessMode, AccessRegistry, Lease, LeaseOwner};
pub use session::{MAX_OPEN_HANDLES, ShareHandle, ShareImage, ShareOp, ShareSession, ShareStatus};

use super::DWServer;
use super::host::{RequestId, SubmitError};

/// Longest share name, in bytes.
pub const MAX_SHARE_NAME_LEN: usize = 32;

/// Most shares one VM can configure.
pub const MAX_SHARES: usize = 16;

/// `dw` command result codes for share failures, from the DriveWire 4 Java
/// server's `DWCmdServerDir`/`DWCmdServerList`, as recorded in the guest
/// compatibility contract (`docs/drivewire-guest-contract.md` before commit
/// `185ac1f`).
pub mod command_code {
    /// The share or path cannot be resolved or accessed.
    pub const RESOLUTION: u16 = 201;
    /// The host refused or failed the operation.
    pub const HOST_IO: u16 = 202;
}

/// Access a share grants its guests.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ShareAccess {
    #[default]
    ReadOnly,
    ReadWrite,
}

/// One configured share: a guest-visible name for a host folder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShareSpec {
    pub name: String,
    /// Host folder, already resolved by the frontend. It need not exist:
    /// a missing root fails each request with [`ShareError::RootUnavailable`].
    pub root: PathBuf,
    pub access: ShareAccess,
}

/// A VM's validated share list.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShareTable {
    shares: Vec<ShareSpec>,
}

impl ShareTable {
    /// Validates names and roots; see [`ShareConfigError`].
    pub fn new(shares: Vec<ShareSpec>) -> Result<Self, ShareConfigError> {
        validate_share_names(shares.iter().map(|share| share.name.as_str()))?;
        if let Some(share) = shares
            .iter()
            .find(|share| share.root.as_os_str().is_empty())
        {
            return Err(ShareConfigError::EmptyRoot(share.name.clone()));
        }
        Ok(Self { shares })
    }

    /// The share named `name`, ignoring ASCII case.
    pub fn get(&self, name: &str) -> Option<&ShareSpec> {
        self.shares
            .iter()
            .find(|share| share.name.eq_ignore_ascii_case(name))
    }

    pub fn shares(&self) -> &[ShareSpec] {
        &self.shares
    }

    pub fn is_empty(&self) -> bool {
        self.shares.is_empty()
    }
}

/// Checks share names on their own, so settings can report a bad name
/// before its folder is chosen.
pub fn validate_share_names<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> Result<(), ShareConfigError> {
    let mut seen: Vec<&str> = Vec::new();
    for name in names {
        validate_share_name(name)?;
        if seen.iter().any(|other| other.eq_ignore_ascii_case(name)) {
            return Err(ShareConfigError::DuplicateName(name.to_string()));
        }
        seen.push(name);
    }
    if seen.len() > MAX_SHARES {
        return Err(ShareConfigError::TooMany);
    }
    Ok(())
}

/// 1–[`MAX_SHARE_NAME_LEN`] ASCII letters, digits, `-`, or `_`. The guest
/// types these names, so they avoid path separators, dots, and spaces.
pub fn validate_share_name(name: &str) -> Result<(), ShareConfigError> {
    let valid = !name.is_empty()
        && name.len() <= MAX_SHARE_NAME_LEN
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if valid {
        Ok(())
    } else {
        Err(ShareConfigError::InvalidName(name.to_string()))
    }
}

/// Why a share list cannot be used.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShareConfigError {
    InvalidName(String),
    DuplicateName(String),
    EmptyRoot(String),
    TooMany,
}

impl fmt::Display for ShareConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(name) => write!(
                formatter,
                "DriveWire share name \"{name}\" must be 1–{MAX_SHARE_NAME_LEN} letters, \
                 digits, hyphens, or underscores"
            ),
            Self::DuplicateName(name) => {
                write!(formatter, "DriveWire share name \"{name}\" is used twice")
            }
            Self::EmptyRoot(name) => write!(formatter, "DriveWire share \"{name}\" has no folder"),
            Self::TooMany => write!(formatter, "a VM can define at most {MAX_SHARES} shares"),
        }
    }
}

impl std::error::Error for ShareConfigError {}

/// A failed share request. Every variant is safe to show the guest: none
/// carries a host path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShareError {
    /// The guest path is empty where a file is required, too long, or
    /// contains a byte outside the guest filename alphabet.
    InvalidPath,
    /// The first path component names no configured share.
    UnknownShare,
    /// A symlink resolves outside the share's root.
    Escape,
    /// The share's folder is missing, renamed, or not a folder.
    RootUnavailable,
    NotFound,
    NotADirectory,
    IsADirectory,
    /// A write to a read-only share, or through a read-only handle.
    ReadOnly,
    /// The host denied access.
    PermissionDenied,
    /// Another VM holds the file in a conflicting mode.
    Busy,
    /// The directory has more than [`path::MAX_DIR_ENTRIES`] entries.
    DirectoryTooLarge,
    /// Every handle slot in the session is in use.
    TooManyHandles,
    /// The handle is not open in this session.
    BadHandle,
    /// The request exceeds a bounded transfer size.
    TooLarge,
    Io(io::ErrorKind),
}

impl ShareError {
    /// Maps an operating-system error met while resolving or opening a path.
    pub fn from_io(error: &io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::NotADirectory => Self::NotADirectory,
            io::ErrorKind::IsADirectory => Self::IsADirectory,
            io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            io::ErrorKind::ReadOnlyFilesystem => Self::ReadOnly,
            kind => Self::Io(kind),
        }
    }

    /// The `dw` command result code for this failure (see [`command_code`]).
    pub fn command_code(self) -> u16 {
        match self {
            Self::InvalidPath
            | Self::UnknownShare
            | Self::Escape
            | Self::RootUnavailable
            | Self::NotFound
            | Self::NotADirectory
            | Self::IsADirectory
            | Self::ReadOnly
            | Self::PermissionDenied => command_code::RESOLUTION,
            Self::Busy
            | Self::DirectoryTooLarge
            | Self::TooManyHandles
            | Self::BadHandle
            | Self::TooLarge
            | Self::Io(_) => command_code::HOST_IO,
        }
    }

    /// Short ASCII explanation for a guest status line and the settings UI.
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidPath => "invalid path",
            Self::UnknownShare => "no such share",
            Self::Escape => "path leaves the share",
            Self::RootUnavailable => "share folder unavailable",
            Self::NotFound => "not found",
            Self::NotADirectory => "not a directory",
            Self::IsADirectory => "is a directory",
            Self::ReadOnly => "read-only",
            Self::PermissionDenied => "permission denied",
            Self::Busy => "in use by another VM",
            Self::DirectoryTooLarge => "directory too large",
            Self::TooManyHandles => "too many open files",
            Self::BadHandle => "file not open",
            Self::TooLarge => "request too large",
            Self::Io(_) => "host I/O error",
        }
    }
}

impl fmt::Display for ShareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(kind) => write!(formatter, "{} ({kind})", self.message()),
            _ => formatter.write_str(self.message()),
        }
    }
}

impl std::error::Error for ShareError {}

impl DWServer {
    /// Installs `table` for guest services, owned by `owner` for cross-VM
    /// access rules. An identical table and owner keep the current session;
    /// any change starts a fresh one, closing its handles and returning the
    /// current directory to the top level.
    pub fn set_shares(&mut self, table: ShareTable, owner: LeaseOwner) {
        if self.shares.table() != &table || self.shares.owner() != owner {
            self.shares = ShareSession::new(table, owner, AccessRegistry::global());
        }
    }

    pub fn share_session(&self) -> &ShareSession {
        &self.shares
    }

    /// Queues `op` on the host executor; its reply arrives through
    /// [`Self::take_host_completion`] (see [`ShareOp`] for each payload).
    pub fn submit_share_op(&mut self, op: ShareOp) -> Result<RequestId, SubmitError> {
        let job = self.shares.job(op);
        self.submit_host_service(job)
    }
}

#[cfg(test)]
#[path = "share_test.rs"]
mod tests;
