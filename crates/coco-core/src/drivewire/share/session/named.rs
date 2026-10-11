//! Named-object resolution: the names DriveWire guests send with
//! `OP_NAMEOBJ_MOUNT` and `OP_NAMEOBJ_CREATE` (see `drivewire::named`).
//!
//! A name is a relative path inside the share the VM selected for named
//! objects ([`ShareTable::named_objects`]), the way the DriveWire 4 Java
//! server appends a name to its `NamedObjectDir`. Names follow the share
//! path grammar ([`path`]): `dir/file` reaches a subfolder, `..` cannot
//! leave the share, and the current directory of the `dw` commands plays no
//! part. A leading `/` is refused: an absolute name would address the share
//! list rather than an object.

use std::fs::OpenOptions;

use super::super::access::AccessMode;
use super::super::path::{self, GuestPath};
use super::super::{ShareAccess, ShareError};
use super::{ShareImage, ShareSession, lock};

const SEPARATOR: u8 = b'/';

impl ShareSession {
    /// Opens the existing object `name` for a named-object mount, leased
    /// for writing when its share is read/write and for reading otherwise.
    /// Blocks on the host filesystem, so call it from a host job or a
    /// restore, never inside a Becker register access.
    pub fn open_named(&self, name: &[u8]) -> Result<ShareImage, ShareError> {
        let result = self
            .named_path(name)
            .and_then(|guest| self.open_guest_image(guest));
        self.record_named(result.as_ref().err().copied());
        result
    }

    /// Creates the object `name`, which must not exist yet, in a read/write
    /// share, and opens it empty and write-leased. Blocks like
    /// [`Self::open_named`].
    pub fn create_named(&self, name: &[u8]) -> Result<ShareImage, ShareError> {
        let result = self
            .named_path(name)
            .and_then(|guest| self.create_guest_image(guest));
        self.record_named(result.as_ref().err().copied());
        result
    }

    fn create_guest_image(&self, guest: GuestPath) -> Result<ShareImage, ShareError> {
        let resolved = path::resolve_new(&guest, &self.table)?;
        if resolved.access == ShareAccess::ReadOnly {
            return Err(ShareError::ReadOnly);
        }
        // `create_new` refuses any existing entry, a symlink included.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&resolved.host)?;
        let lease = self
            .registry
            .acquire(&file, &resolved.host, self.owner, AccessMode::Write)?;
        Ok(ShareImage {
            file,
            writable: true,
            lease,
            guest_path: guest.to_string(),
            host_path: resolved.host,
        })
    }

    /// `name` as a path inside the named-object share.
    fn named_path(&self, name: &[u8]) -> Result<GuestPath, ShareError> {
        let share = self
            .table
            .named_objects()
            .and_then(|share| self.table.get(share))
            .ok_or(ShareError::UnknownShare)?;
        if name.is_empty() || name.first() == Some(&SEPARATOR) {
            return Err(ShareError::InvalidPath);
        }
        let root = GuestPath::parse(share.name.as_bytes(), &GuestPath::top(), &self.table)?;
        let guest = GuestPath::parse(name, &root, &self.table)?;
        if guest.share() != Some(share.name.as_str()) || guest.components().is_empty() {
            return Err(ShareError::Escape);
        }
        Ok(guest)
    }

    /// Records a named-object failure for the settings UI, without waiting
    /// on a directory or handle change.
    fn record_named(&self, error: Option<ShareError>) {
        let state = lock(&self.state);
        self.record(&state, error);
    }
}

#[cfg(test)]
#[path = "named_test.rs"]
mod tests;
