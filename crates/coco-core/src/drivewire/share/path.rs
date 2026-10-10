//! Guest path grammar and host resolution, shared by every share service.
//!
//! A [`GuestPath`] is a normalized position in the guest's virtual tree:
//! the top level, or a share plus the components below its root. Parsing is
//! purely lexical: `.` is dropped, `..` removes one component (from a share
//! root it returns to the top level, and at the top level it stays there),
//! and empty components collapse. Lexical `..` therefore never reaches a host
//! folder above a share.
//!
//! Host resolution then canonicalizes the share root and the target, and
//! rejects a target outside the canonical root with [`ShareError::Escape`].
//! Symlinks whose targets stay inside the root are followed. The check runs
//! in the same host job as the open that follows it. A host process that swaps
//! a symlink between the two can still redirect the open. Guests cannot create
//! symlinks through these services.
//!
//! Filename encoding: guest components are printable ASCII (`0x20`–`0x7E`)
//! without `/`, the bytes in [`RESERVED_BYTES`], or leading/trailing spaces.
//! High-bit CoCo characters have no agreed host spelling, so they are
//! rejected. Host names are matched byte for byte, with the host's own case
//! rules. Host names that are not in this alphabet are left out of listings,
//! since the guest could not name them back.

use std::fs;
use std::path::{Path, PathBuf};

use super::{ShareAccess, ShareError, ShareTable};

/// Longest guest path or component, in bytes.
pub const MAX_GUEST_PATH_LEN: usize = 255;

/// Most entries one directory listing reads.
pub const MAX_DIR_ENTRIES: usize = 1024;

/// Bytes a guest component may not contain besides the separator: Windows
/// reserves them in filenames, and `*`/`?` are wildcards on most hosts.
pub const RESERVED_BYTES: &[u8] = b"\\:*?\"<>|";

const SEPARATOR: u8 = b'/';
const FIRST_PRINTABLE: u8 = 0x20;
const LAST_PRINTABLE: u8 = 0x7E;
const SPACE: u8 = b' ';
const CURRENT: &str = ".";
const PARENT: &str = "..";

/// A normalized position in the guest's virtual tree.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GuestPath {
    /// Configured share name, in its configured case. `None` is the top level.
    share: Option<String>,
    components: Vec<String>,
}

impl GuestPath {
    /// The top level, which lists the shares.
    pub fn top() -> Self {
        Self::default()
    }

    /// Parses `input` relative to `cwd`. A leading `/` starts at the top
    /// level; an empty input names `cwd` itself.
    pub fn parse(input: &[u8], cwd: &Self, table: &ShareTable) -> Result<Self, ShareError> {
        if input.len() > MAX_GUEST_PATH_LEN {
            return Err(ShareError::InvalidPath);
        }
        let mut path = if input.first() == Some(&SEPARATOR) {
            Self::top()
        } else {
            cwd.clone()
        };
        for component in input.split(|&byte| byte == SEPARATOR) {
            path.push(component, table)?;
        }
        Ok(path)
    }

    pub fn share(&self) -> Option<&str> {
        self.share.as_deref()
    }

    pub fn components(&self) -> &[String] {
        &self.components
    }

    pub fn is_top(&self) -> bool {
        self.share.is_none()
    }

    fn push(&mut self, component: &[u8], table: &ShareTable) -> Result<(), ShareError> {
        let Ok(component) = std::str::from_utf8(component) else {
            return Err(ShareError::InvalidPath);
        };
        match component {
            "" | CURRENT => {}
            PARENT => {
                if self.components.pop().is_none() {
                    self.share = None;
                }
            }
            _ if !is_guest_component(component) => return Err(ShareError::InvalidPath),
            _ if self.share.is_none() => {
                let share = table.get(component).ok_or(ShareError::UnknownShare)?;
                self.share = Some(share.name.clone());
            }
            _ => self.components.push(component.to_string()),
        }
        Ok(())
    }
}

impl std::fmt::Display for GuestPath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Some(share) = &self.share else {
            return formatter.write_str("/");
        };
        write!(formatter, "/{share}")?;
        self.components
            .iter()
            .try_for_each(|component| write!(formatter, "/{component}"))
    }
}

/// Whether `name` is in the guest filename alphabet (see the module doc).
pub fn is_guest_component(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_GUEST_PATH_LEN
        && name != CURRENT
        && name != PARENT
        && bytes.first() != Some(&SPACE)
        && bytes.last() != Some(&SPACE)
        && bytes.iter().all(|&byte| {
            (FIRST_PRINTABLE..=LAST_PRINTABLE).contains(&byte)
                && byte != SEPARATOR
                && !RESERVED_BYTES.contains(&byte)
        })
}

/// A guest path resolved to a host path inside its share.
#[derive(Debug)]
pub(super) struct Resolved {
    pub access: ShareAccess,
    /// Canonical share root.
    pub root: PathBuf,
    pub host: PathBuf,
}

/// One directory entry, as the guest sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

/// Resolves a path that must exist: the share root itself or something
/// below it.
pub(super) fn resolve_existing(
    guest: &GuestPath,
    table: &ShareTable,
) -> Result<Resolved, ShareError> {
    let (access, root) = share_root(guest, table)?;
    let candidate = guest
        .components
        .iter()
        .fold(root.clone(), |path, component| path.join(component));
    let host = fs::canonicalize(&candidate).map_err(|error| ShareError::from_io(&error))?;
    contain(&root, &host)?;
    Ok(Resolved { access, root, host })
}

/// Resolves a file to create or replace: its parent must exist inside the
/// share, and an existing symlink at the target must stay inside it too.
pub(super) fn resolve_new(guest: &GuestPath, table: &ShareTable) -> Result<Resolved, ShareError> {
    let Some((name, parents)) = guest.components.split_last() else {
        return Err(ShareError::IsADirectory);
    };
    let (access, root) = share_root(guest, table)?;
    let parent = parents
        .iter()
        .fold(root.clone(), |path, component| path.join(component));
    let parent = fs::canonicalize(&parent).map_err(|error| ShareError::from_io(&error))?;
    contain(&root, &parent)?;
    let mut host = parent.join(name);
    if let Ok(metadata) = fs::symlink_metadata(&host) {
        if metadata.file_type().is_symlink() {
            // A dangling link would create its target wherever it points.
            host = fs::canonicalize(&host).map_err(|_| ShareError::Escape)?;
            contain(&root, &host)?;
        }
        if fs::metadata(&host).is_ok_and(|metadata| metadata.is_dir()) {
            return Err(ShareError::IsADirectory);
        }
    }
    Ok(Resolved { access, root, host })
}

/// Lists `guest`, sorted by name. The top level lists the shares.
/// `is_cancelled` is polled per entry so a large scan stops early.
pub(super) fn list(
    guest: &GuestPath,
    table: &ShareTable,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Entry>, ShareError> {
    if guest.is_top() {
        let mut entries: Vec<Entry> = table
            .shares()
            .iter()
            .map(|share| Entry {
                name: share.name.clone(),
                is_dir: true,
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        return Ok(entries);
    }
    let Resolved { root, host, .. } = resolve_existing(guest, table)?;
    let mut entries = Vec::new();
    let reader = fs::read_dir(&host).map_err(|error| ShareError::from_io(&error))?;
    for (scanned, item) in reader.enumerate() {
        if scanned >= MAX_DIR_ENTRIES {
            return Err(ShareError::DirectoryTooLarge);
        }
        if is_cancelled() {
            return Err(ShareError::Io(std::io::ErrorKind::Interrupted));
        }
        let item = item.map_err(|error| ShareError::from_io(&error))?;
        if let Some(entry) = guest_entry(&root, &item) {
            entries.push(entry);
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// The guest view of `item`, or `None` when the guest could not name it or
/// it is a symlink that leaves the share or dangles.
fn guest_entry(root: &Path, item: &fs::DirEntry) -> Option<Entry> {
    let name = item.file_name().to_str()?.to_string();
    if !is_guest_component(&name) {
        return None;
    }
    let file_type = item.file_type().ok()?;
    let is_dir = if file_type.is_symlink() {
        let target = fs::canonicalize(item.path()).ok()?;
        contain(root, &target).ok()?;
        target.is_dir()
    } else {
        file_type.is_dir()
    };
    Some(Entry { name, is_dir })
}

/// The share's access and canonical root. The root is resolved on every
/// request, so a removed, renamed, or replaced folder is noticed at once.
fn share_root(guest: &GuestPath, table: &ShareTable) -> Result<(ShareAccess, PathBuf), ShareError> {
    let name = guest.share.as_deref().ok_or(ShareError::IsADirectory)?;
    let share = table.get(name).ok_or(ShareError::UnknownShare)?;
    let root = fs::canonicalize(&share.root).map_err(|_| ShareError::RootUnavailable)?;
    if !root.is_dir() {
        return Err(ShareError::RootUnavailable);
    }
    Ok((share.access, root))
}

fn contain(root: &Path, host: &Path) -> Result<(), ShareError> {
    if host.starts_with(root) {
        Ok(())
    } else {
        Err(ShareError::Escape)
    }
}

#[cfg(test)]
#[path = "path_test.rs"]
mod tests;
