//! Phase-2 tests for the save-state snapshot *engine*
//! (`crates/coco-core/src/snapshot.rs`): the `.ccstate` container format,
//! media-reference handling, and the restore flow. These tests cover the
//! container and restore behavior that `snapshot_roundtrip.rs` leaves out.

#[path = "snapshot_engine/common.rs"]
mod common;

#[path = "snapshot_engine/header.rs"]
mod header;
#[path = "snapshot_engine/hostile_payload.rs"]
mod hostile_payload;
#[path = "snapshot_engine/lockstep.rs"]
mod lockstep;
#[path = "snapshot_engine/media.rs"]
mod media;
