//! What each DriveWire drive holds besides its image bytes: who mounted it,
//! the name the guest sees, and whether writes are refused.
//!
//! The host mounts images from VM settings ([`DWServer::mount`]); the guest
//! mounts and ejects them with `dw disk insert`/`dw disk eject` (see
//! `drivewire::command`). Guest changes last for the session only: they are
//! never written to the VM definition, so a cold start mounts the
//! configured images again. The frontend mirrors them into its session
//! media through [`DWServer::take_guest_media_changes`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::share::Lease;
use super::{DRIVE_COUNT, DWImage, DWServer};

/// Who mounted a drive's current image.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MediaOrigin {
    /// The host: VM settings, a snapshot restore, or a test.
    Host,
    /// The guest, with `dw disk insert`, for this session only.
    Guest,
}

/// A mounted drive, as `dw disk show` and the settings UI describe it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DriveMedia {
    /// Guest-visible name: the normalized share path of a guest mount, or
    /// the file name the host set with [`DWServer::set_drive_name`].
    pub name: Option<String>,
    pub origin: MediaOrigin,
    /// Writes fail with a DriveWire write error: the guest mounted the
    /// image from a read-only share.
    pub write_protected: bool,
}

impl DriveMedia {
    fn host() -> Self {
        Self {
            name: None,
            origin: MediaOrigin::Host,
            write_protected: false,
        }
    }
}

/// A guest mount change for the frontend to mirror into its session media.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuestMediaChange {
    pub drive: usize,
    /// The host file now mounted, or `None` after an eject.
    pub host_path: Option<PathBuf>,
}

/// Per-drive media state. Only the descriptions enter snapshots: leases and
/// pending changes belong to the running host.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct MediaTable {
    drives: [Option<DriveMedia>; DRIVE_COUNT],
    /// The guest emptied the drive with `dw disk eject`.
    #[serde(default)]
    guest_ejected: [bool; DRIVE_COUNT],
    /// Leases on guest-mounted images; host mounts keep their own.
    #[serde(skip)]
    leases: [Option<Lease>; DRIVE_COUNT],
    /// The latest unreported guest change per drive.
    #[serde(skip)]
    changes: [Option<Option<PathBuf>>; DRIVE_COUNT],
}

impl MediaTable {
    pub(super) fn write_protected(&self, drive: usize) -> bool {
        self.drives
            .get(drive)
            .and_then(Option::as_ref)
            .is_some_and(|media| media.write_protected)
    }
}

impl DWServer {
    /// How `drive`'s image was mounted, or `None` when it is empty.
    pub fn drive_media(&self, drive: usize) -> Option<&DriveMedia> {
        self.drives[drive].as_ref()?;
        self.media.drives[drive].as_ref()
    }

    /// Names a host-mounted image for `dw disk show`, typically its file
    /// name. Ignored for an empty drive.
    pub fn set_drive_name(&mut self, drive: usize, name: impl Into<String>) {
        if let Some(media) = self.media.drives[drive].as_mut() {
            media.name = Some(name.into());
        }
    }

    /// Writes to `drive` fail: the guest mounted it from a read-only share.
    /// Unlike [`Self::drive_media`], this holds before a restored server's
    /// images are reattached, so the frontend can reopen them read-only.
    pub fn drive_write_protected(&self, drive: usize) -> bool {
        self.media.write_protected(drive)
    }

    /// Whether the guest's `dw disk insert` or `dw disk eject` decided what
    /// `drive` holds now. Holds across snapshots.
    pub fn guest_changed(&self, drive: usize) -> bool {
        self.media.guest_ejected[drive]
            || self.media.drives[drive]
                .as_ref()
                .is_some_and(|media| media.origin == MediaOrigin::Guest)
    }

    /// Guest mounts and ejects since the last call, latest per drive.
    pub fn take_guest_media_changes(&mut self) -> Vec<GuestMediaChange> {
        self.media
            .changes
            .iter_mut()
            .enumerate()
            .filter_map(|(drive, change)| {
                let host_path = change.take()?;
                Some(GuestMediaChange { drive, host_path })
            })
            .collect()
    }

    pub(super) fn record_host_mount(&mut self, drive: usize) {
        self.media.drives[drive] = Some(DriveMedia::host());
        self.media.leases[drive] = None;
        self.media.guest_ejected[drive] = false;
    }

    pub(super) fn record_host_eject(&mut self, drive: usize) {
        self.media.drives[drive] = None;
        self.media.leases[drive] = None;
        self.media.guest_ejected[drive] = false;
    }

    /// A restored image keeps its snapshot description; a snapshot from
    /// before descriptions existed gets a host one.
    pub(super) fn record_reattach(&mut self, drive: usize) {
        self.media.drives[drive].get_or_insert_with(DriveMedia::host);
        self.media.leases[drive] = None;
    }

    /// `dw disk insert`: replaces whatever `drive` held.
    pub(super) fn mount_guest_image(&mut self, drive: usize, image: super::share::ShareImage) {
        self.mount(drive, DWImage::File(image.file));
        self.media.drives[drive] = Some(DriveMedia {
            name: Some(image.guest_path),
            origin: MediaOrigin::Guest,
            write_protected: !image.writable,
        });
        self.media.leases[drive] = Some(image.lease);
        self.media.changes[drive] = Some(Some(image.host_path));
    }

    /// `dw disk eject`: returns whether `drive` held an image.
    pub(super) fn eject_for_guest(&mut self, drive: usize) -> bool {
        if !self.is_mounted(drive) {
            return false;
        }
        self.eject(drive);
        self.media.guest_ejected[drive] = true;
        self.media.changes[drive] = Some(None);
        true
    }
}

#[cfg(test)]
#[path = "media_test.rs"]
mod tests;
