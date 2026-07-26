//! Everything that can go wrong across [`super::save`]/[`super::load`]/
//! [`super::restore`].

use std::fmt;

/// Everything that can go wrong across [`super::save`]/[`super::load`]/[`super::restore`], with a
/// [`Display`](fmt::Display) message precise enough to show a user directly.
#[derive(Debug)]
pub enum SnapshotError {
    /// The bytes don't start with [`super::CONTAINER_MAGIC`], or are too short to
    /// contain a full header at all.
    NotASnapshot,
    /// The container's own framing version isn't one this build understands.
    UnsupportedContainer { found: u8, supported: u8 },
    /// The machine-tree schema is newer than this build knows how to read.
    SchemaTooNew { found: u32, current: u32 },
    /// The machine-tree schema is older than current, and no migration is
    /// registered for it (see `super::migrate`).
    NoMigration { found: u32, current: u32 },
    /// CBOR encoding failed (besides the dedicated
    /// [`SnapshotError::CustomCartNotSnapshotable`] case).
    Encode(String),
    /// Gzip or CBOR decoding failed.
    Decode(String),
    /// The machine being saved has a [`crate::cart::Cart::Custom`] test double inserted,
    /// which has no serializable shape.
    CustomCartNotSnapshotable,
    /// One or more media sources needed by [`super::restore`] weren't provided;
    /// `descriptions` names every one collected, so a caller can prompt for
    /// all of them at once.
    MissingMedia { descriptions: Vec<String> },
    /// A provided media source doesn't fit the shape the snapshot recorded
    /// (wrong floppy geometry, oversized ROM image, ...) — retrying with the
    /// *same* file wouldn't help, unlike [`SnapshotError::MissingMedia`].
    MediaShape { role: String, detail: String },
    /// The decoded payload is internally invalid (bad config, RAM length
    /// mismatch, self-contradictory media state, ...) — never reached from
    /// bytes this module itself produced, only from corrupted or
    /// hand-edited ones.
    InvalidPayload(String),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnapshotError::NotASnapshot => write!(f, "not a CoCo save state"),
            SnapshotError::UnsupportedContainer { found, supported } => write!(
                f,
                "unsupported save-state container version {found} (this build supports {supported})"
            ),
            SnapshotError::SchemaTooNew { found, current } => write!(
                f,
                "this save state was written by a newer version (schema {found}); this build \
                 understands up to schema {current}"
            ),
            SnapshotError::NoMigration { found, current } => write!(
                f,
                "this save state is schema {found}; this build is schema {current} and has no \
                 migration path from {found}"
            ),
            SnapshotError::Encode(msg) => write!(f, "failed to encode save state: {msg}"),
            SnapshotError::Decode(msg) => write!(f, "failed to decode save state: {msg}"),
            SnapshotError::CustomCartNotSnapshotable => {
                write!(f, "cannot save: an out-of-crate test cartridge is inserted")
            }
            SnapshotError::MissingMedia { descriptions } => {
                write!(f, "missing media needed to restore this save state:")?;
                for desc in descriptions {
                    write!(f, "\n  - {desc}")?;
                }
                Ok(())
            }
            SnapshotError::MediaShape { role, detail } => {
                write!(f, "{role} doesn't match this save state: {detail}")
            }
            SnapshotError::InvalidPayload(msg) => write!(f, "invalid save state: {msg}"),
        }
    }
}

impl std::error::Error for SnapshotError {}
