//! Payload types: everything a snapshot needs besides resolved media bytes,
//! and the restore-time media references/sources/notes that shuttle around
//! them.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::drivewire::DWImage;
use crate::vhd::VHDImage;
use crate::{Machine, drivewire, fdc, vhd};

/// Everything a snapshot needs besides resolved media bytes: the machine
/// tree (config travels inside `machine.config`) plus where its media came
/// from.
#[derive(Serialize, Deserialize)]
pub struct SnapshotPayload {
    pub media: MediaRefs,
    pub machine: Machine,
}

/// Borrowing twin of [`SnapshotPayload`] with identical field names/layout,
/// so [`super::save`] can CBOR-encode by reference instead of cloning the whole
/// machine tree to pass it to `ciborium::into_writer`.
#[derive(Serialize)]
pub(super) struct SnapshotPayloadRef<'a> {
    pub(super) media: &'a MediaRefs,
    pub(super) machine: &'a Machine,
}

/// Where one media file lived and what it hashed to, at save time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaRef {
    /// As the frontend knew it at save time — absolute or relative, whatever
    /// the frontend itself used; this module never resolves or interprets
    /// it, only carries it.
    pub path: PathBuf,
    /// Lowercase hex SHA-256 of the file's contents at save time (see
    /// [`super::sha256_file`]).
    pub sha256: String,
}

/// A ROM-bearing cartridge's image reference, located by where it plugs in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotROMRef {
    /// `None` = the machine's own cartridge port; `Some(i)` = Multi-Pak slot
    /// `i` (0-3).
    pub mpi_slot: Option<u8>,
    pub rom: MediaRef,
}

/// Every media reference a snapshot might carry. Every field is
/// `#[serde(default)]` per evolution rule 2 — a future field added here must
/// still load an older snapshot as "this media slot was never used".
///
/// `disks`/`vhds`/`drivewire` are `Vec`, not `[Option<MediaRef>;
/// N::DRIVE_COUNT]`: a fixed-size array bakes today's `DRIVE_COUNT` into the
/// serialized shape, so a future change to it would fail to deserialize (or
/// silently truncate) every snapshot written before the change — the
/// evolution contract earlier forbids that. [`super::restore`] matches these up
/// against the machine's actual drive count itself (zip-style: a short `Vec`
/// leaves trailing drives as "never mounted"; a `Vec` longer than the current
/// build's `DRIVE_COUNT` is [`super::SnapshotError::InvalidPayload`], naming the
/// slot).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaRefs {
    #[serde(default)]
    pub system_rom: Option<MediaRef>,
    /// ROM-bearing carts, keyed by where they sit. Covers ROMPak/
    /// BankedROMPak/GamesMasterCartridge/DiskCart/Orch90 images, the
    /// SoundSpeechCartridge's SP0256-AL2 allophone ROM, and the
    /// DeluxeRS232 EPROM — one entry per ROM-bearing cart that actually has
    /// an image (the DeluxeRS232 is the one cart in this list that can
    /// legitimately run without one; see [`super::restore`]'s cart-ROM
    /// step).
    #[serde(default)]
    pub cart_roms: Vec<SlotROMRef>,
    /// FD-502 JVC drives, indexed by drive number.
    #[serde(default)]
    pub disks: Vec<Option<MediaRef>>,
    /// VHD drives, indexed by drive number.
    #[serde(default)]
    pub vhds: Vec<Option<MediaRef>>,
    /// DriveWire drives, indexed by drive number.
    #[serde(default)]
    pub drivewire: Vec<Option<MediaRef>>,
    #[serde(default)]
    pub tape: Option<MediaRef>,
}

/// Resolved media bytes/handles for [`super::restore`], produced by the caller from
/// a [`MediaRefs`] (a real frontend re-reads each `path`; tests inject bytes
/// directly). Never touched by [`super::load`] — only [`super::restore`] consumes this.
#[derive(Default)]
pub struct MediaSources {
    pub system_rom: Option<Box<[u8]>>,
    /// `(mpi_slot, bytes)` pairs, matched against the deserialized cart
    /// tree's own `(mpi_slot, ..)` positions — see [`SlotROMRef`].
    pub cart_roms: Vec<(Option<u8>, Vec<u8>)>,
    pub disks: [Option<Vec<u8>>; fdc::DRIVE_COUNT],
    pub vhds: [Option<VHDImage>; vhd::DRIVE_COUNT],
    pub drivewire: [Option<DWImage>; drivewire::DRIVE_COUNT],
    pub tape: Option<Vec<u8>>,
}

/// The result of a successful [`super::restore`]: the live machine plus any
/// non-fatal notes that the frontend should surface, such as a toast. Hash
/// verification is caller-side (see [`MediaRef::verify`]) — a mismatch
/// warning is built there, not here.
pub struct RestoredMachine {
    pub machine: Machine,
    pub notes: Vec<RestoreNote>,
}

/// A non-fatal condition [`super::restore`] leaves for the caller to surface:
/// state that came back in a documented placeholder form rather than fully
/// restored. Typed rather than raw strings so a caller can react to a
/// specific condition programmatically. For example, after `restore` returns,
/// the egui frontend re-injects the Disto RTC's host time source and can omit
/// [`RestoreNote::RTCPlaceholderTime`] before showing the remaining notes as a
/// toast. The note remains relevant to callers that do not immediately
/// resynchronize it, such as a headless tool or test; matching on message text
/// could not identify it safely across future wording changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreNote {
    /// Print capture was active when this snapshot was saved; capture is
    /// stopped until restarted from the frontend.
    PrintCaptureStopped,
    /// The Deluxe RS-232 host connection restored as loopback; the real
    /// endpoint needs to be re-plugged from the frontend.
    RS232EndpointLoopback,
    /// The Disto real-time clock restored to a placeholder time
    /// (1970-01-01); true only for a caller that doesn't itself re-sync it
    /// from a live time source right after restoring — see this type's own
    /// doc comment.
    RTCPlaceholderTime,
}

impl fmt::Display for RestoreNote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RestoreNote::PrintCaptureStopped => write!(
                f,
                "print capture was active when this snapshot was saved; capture is stopped until \
                 restarted from the Machine menu"
            ),
            RestoreNote::RS232EndpointLoopback => write!(
                f,
                "Deluxe RS-232 host connection restored as loopback; re-plug the real endpoint from \
                 the frontend"
            ),
            RestoreNote::RTCPlaceholderTime => write!(
                f,
                "Disto real-time clock restored to a placeholder time (1970-01-01); re-sync it from \
                 the frontend"
            ),
        }
    }
}
