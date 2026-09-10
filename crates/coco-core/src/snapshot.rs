//! Save-state snapshot engine: the `.ccstate` container format, media
//! references, and the restore flow that turns a decoded payload plus
//! resolved media bytes back into a running [`Machine`](crate::Machine).
//!
//! ## Compatibility
//!
//! The snapshot format is in development and has no backward compatibility
//! guarantee. Only the current schema is accepted; unreadable snapshots
//! return a load error.
//!
//! ## Container format
//!
//! ```text
//! magic "CCSTATE" (7 bytes) | container_version: u8 | schema: u32 LE | gzip(CBOR payload)
//! ```
//!
//! [`CONTAINER_VERSION`] is the container/header layout itself (this module's
//! own framing); [`SCHEMA_VERSION`] identifies the serialized machine state.
//! Recommended file extension: `.ccstate` (a frontend concern; this module
//! works on plain bytes and never touches a file itself).
//!
//! ## Media: references, not content
//!
//! ROM/disk/VHD/DriveWire/tape bytes never travel inside the payload — they
//! can be copyrighted commercial software. [`MediaRefs`] records where the
//! frontend found each one (path) and a SHA-256 of its contents at save time;
//! [`load`] decodes the payload but does no file I/O, and [`restore`] takes
//! already-resolved [`MediaSources`] bytes/handles rather than opening
//! anything itself — the caller (a real frontend, or a test injecting bytes
//! directly) owns every filesystem access. The caller must flush dirty media
//! (unsaved floppy/tape changes) BEFORE calling [`sha256_file`]/[`save`], so
//! the recorded hash actually describes what's on disk — this module has no
//! flush hook of its own.
//!
//! The "no media bytes embedded" rule earlier is about WHOLE media images —
//! disk/VHD/DriveWire/tape files, ROM images. It does NOT extend to a
//! device's own in-flight I/O buffers: a snapshot taken mid-sector-transfer
//! carries that sector's bytes in the WD1773's `Transfer.buf` (see
//! `crate::wd1773`), the same way RAM carries whatever a program loaded into
//! it. Those bytes are as much machine state as a CPU register — only the
//! backing media file itself is excluded.

mod codec;
mod error;
mod hash;
mod payload;
pub(crate) mod restore;

pub use codec::{load, save};
pub use error::SnapshotError;
pub use hash::{MediaCheck, sha256_file, sha256_hex};
pub use payload::{
    CartROMRole, CartROMSource, MediaRef, MediaRefs, MediaSources, RestoreNote, RestoredMachine,
    SlotROMRef, SnapshotPayload,
};
pub use restore::restore;

/// Container magic bytes: the first 7 bytes of every `.ccstate` file.
pub const CONTAINER_MAGIC: &[u8; 7] = b"CCSTATE";
/// Container/header layout version (this module's own framing) — distinct
/// from [`SCHEMA_VERSION`], which versions the machine tree the container
/// carries.
pub const CONTAINER_VERSION: u8 = 1;
/// Version of the serialized machine state.
pub const SCHEMA_VERSION: u32 = 1;

/// Byte length of the container header: magic + version byte + schema `u32`.
const HEADER_LEN: usize = CONTAINER_MAGIC.len() + 1 + 4;
