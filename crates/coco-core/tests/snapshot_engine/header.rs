//! 2. Header checks

use std::io::Write;

use coco_core::snapshot::{self, MediaRefs, SnapshotError};
use coco_core::{Machine, MachineConfig};
use flate2::Compression;
use flate2::write::GzEncoder;

use super::common::expect_err;

const SCHEMA_OFFSET: usize = snapshot::CONTAINER_MAGIC.len() + size_of::<u8>();
const HEADER_LEN: usize = SCHEMA_OFFSET + size_of::<u32>();

/// A valid container's bytes, cheap to build without ROM execution.
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
fn mismatched_schema_is_rejected_before_decompression() {
    for schema in [snapshot::SCHEMA_VERSION - 1, snapshot::SCHEMA_VERSION + 1] {
        let mut bytes = a_valid_save();
        bytes[SCHEMA_OFFSET..HEADER_LEN].copy_from_slice(&schema.to_le_bytes());
        bytes.truncate(HEADER_LEN);
        bytes.extend_from_slice(b"invalid gzip");
        let err = expect_err(snapshot::load(&bytes));
        match err {
            SnapshotError::UnsupportedSchema { found, supported } => {
                assert_eq!(found, schema);
                assert_eq!(supported, snapshot::SCHEMA_VERSION);
            }
            other => panic!("expected UnsupportedSchema, got {other:?}"),
        }
    }
}

#[test]
fn malformed_gzip_returns_decode_error() {
    let mut bytes = a_valid_save();
    bytes.truncate(HEADER_LEN);
    bytes.extend_from_slice(b"invalid gzip");
    let err = expect_err(snapshot::load(&bytes));
    assert!(matches!(err, SnapshotError::Decode(_)), "{err:?}");
}

#[test]
fn malformed_cbor_returns_decode_error() {
    const UNEXPECTED_CBOR_BREAK: u8 = 0xff;
    let mut bytes = a_valid_save();
    bytes.truncate(HEADER_LEN);
    let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
    gzip.write_all(&[UNEXPECTED_CBOR_BREAK]).expect("gzip CBOR");
    bytes.extend_from_slice(&gzip.finish().expect("finish gzip"));
    let err = expect_err(snapshot::load(&bytes));
    assert!(matches!(err, SnapshotError::Decode(_)), "{err:?}");
}
