//! 2. Header checks

use coco_core::snapshot::{self, MediaRefs, SnapshotError};
use coco_core::{Machine, MachineConfig};

use super::common::expect_err;

/// A valid container's bytes, cheap to build (no ROM execution needed — the
/// header tests only ever patch/truncate bytes, never decode the payload).
fn a_valid_save() -> Vec<u8> {
    let machine = Machine::new(MachineConfig::default(), Box::new([]));
    snapshot::save(&machine, &MediaRefs::default()).expect("save")
}

#[test]
fn truncated_file_is_not_a_snapshot() {
    let bytes = a_valid_save();
    let err = expect_err(snapshot::load(&bytes[..3]));
    assert!(matches!(err, SnapshotError::NotASnapshot), "{err:?}");
}

#[test]
fn wrong_magic_is_not_a_snapshot() {
    let mut bytes = a_valid_save();
    bytes[0] = b'X';
    let err = expect_err(snapshot::load(&bytes));
    assert!(matches!(err, SnapshotError::NotASnapshot), "{err:?}");
}

#[test]
fn future_container_version_is_unsupported() {
    let mut bytes = a_valid_save();
    let version_offset = snapshot::CONTAINER_MAGIC.len();
    bytes[version_offset] = snapshot::CONTAINER_VERSION + 1;
    let err = expect_err(snapshot::load(&bytes));
    match err {
        SnapshotError::UnsupportedContainer { found, supported } => {
            assert_eq!(found, snapshot::CONTAINER_VERSION + 1);
            assert_eq!(supported, snapshot::CONTAINER_VERSION);
        }
        other => panic!("expected UnsupportedContainer, got {other:?}"),
    }
}

#[test]
fn future_schema_is_reported_as_schema_too_new() {
    let mut bytes = a_valid_save();
    let schema_offset = snapshot::CONTAINER_MAGIC.len() + 1;
    let future = snapshot::SCHEMA_VERSION + 1;
    bytes[schema_offset..schema_offset + 4].copy_from_slice(&future.to_le_bytes());
    let err = expect_err(snapshot::load(&bytes));
    match err {
        SnapshotError::SchemaTooNew { found, current } => {
            assert_eq!(found, future);
            assert_eq!(current, snapshot::SCHEMA_VERSION);
        }
        other => panic!("expected SchemaTooNew, got {other:?}"),
    }
}
