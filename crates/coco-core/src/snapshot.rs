//! Save-state snapshot engine: the `.ccstate` container format, media
//! references, and the restore flow that turns a decoded payload plus
//! resolved media bytes back into a running [`Machine`](crate::Machine)
//! (`docs/plan-save-states.md`, `docs/plan-machine-persistence.md`).
//!
//! ## Compatibility contract
//!
//! **A snapshot written today must load in every future version**
//! (`docs/plan-machine-persistence.md` "Snapshot compatibility contract",
//! user requirement 2026-07-16). The payload is CBOR (`ciborium`), not a
//! positional format like bincode/postcard: CBOR carries field names with
//! the data, so serde's evolution tools (`#[serde(default)]`/`alias`) work
//! across versions instead of every struct needing hand-rolled versioning.
//! RAM and other big buffers stay compact via `serde_bytes`/
//! [`crate::serde_util::byte_array`] rather than base64-in-JSON. The whole
//! payload is gzipped with `flate2`.
//!
//! Four evolution rules govern every change to a type that lives inside
//! [`SnapshotPayload`] (enforced in review, not by the compiler):
//!
//! 1. never remove or rename a serialized field without `#[serde(alias =
//!    "old_name")]` or a migration;
//! 2. every added field carries `#[serde(default = "...")]` whose default
//!    reproduces the *old* behaviour (a snapshot from before the field
//!    existed must load as if the field had always held that value);
//! 3. never change the meaning or units of an existing field — add a new
//!    field and migrate instead;
//! 4. enum variants may be added, never repurposed.
//!
//! What actually *guarantees* rule compliance, per the plan, is the
//! golden-fixture gate: every time [`SCHEMA_VERSION`] bumps, or a release is
//! cut, a real snapshot fixture (small RAM, mid-BASIC-program) is committed
//! under `crates/coco-core/tests/fixtures/snapshots/`, and a test loads every
//! committed fixture and runs the trace-continuation check from it. That test
//! (and its first fixture) is phase 3's job — this module only builds the
//! engine the gate exercises.
//!
//! ## Container format
//!
//! ```text
//! magic "CCSTATE" (7 bytes) | container_version: u8 | schema: u32 LE | gzip(CBOR payload)
//! ```
//!
//! [`CONTAINER_VERSION`] is the container/header layout itself (this module's
//! own framing); [`SCHEMA_VERSION`] is the *machine-tree* schema and is
//! bumped only on a semantic break serde's evolution tools can't express —
//! everything the four rules above can absorb should NOT bump it. Recommended
//! file extension: `.ccstate` (a frontend concern; this module works on plain
//! bytes and never touches a file itself).
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
//! The "no media bytes embedded" rule above is about WHOLE media images —
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
    MediaRef, MediaRefs, MediaSources, RestoreNote, RestoredMachine, SlotROMRef, SnapshotPayload,
};
pub use restore::restore;

/// Container magic bytes: the first 7 bytes of every `.ccstate` file.
pub const CONTAINER_MAGIC: &[u8; 7] = b"CCSTATE";
/// Container/header layout version (this module's own framing) — distinct
/// from [`SCHEMA_VERSION`], which versions the machine tree the container
/// carries.
pub const CONTAINER_VERSION: u8 = 1;
/// Machine-tree schema version. Bump ONLY on a semantic break the four
/// evolution rules in the module doc can't express; every other change
/// (added/renamed/removed fields, new enum variants) stays on the current
/// schema.
pub const SCHEMA_VERSION: u32 = 1;

/// Byte length of the container header: magic + version byte + schema `u32`.
const HEADER_LEN: usize = CONTAINER_MAGIC.len() + 1 + 4;
