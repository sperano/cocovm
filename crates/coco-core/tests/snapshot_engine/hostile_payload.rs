//! 7. Phase-5 review: hostile-payload validation
//!
//! `Cassette::bit`/`SSC::Load::cap`/`WD1773::Transfer::index`/`WD1773::Transfer::
//! offset+total` have no `pub` setter that can reach an out-of-range value —
//! by design, the normal protocol dispatch that reaches these fields never
//! produces one. So unlike #6's `ram`-length tamper (`bus.ram` is `pub`),
//! these tests hand-mutate the actual CBOR bytes of a valid save, the same
//! way a hex editor on a real `.ccstate` file would, via [`mutate_cbor`]/
//! [`rewrap_container`] below.

use std::io::{Read, Write};
use std::path::PathBuf;

use ciborium::Value;
use coco_core::cart::MultiPak;
use coco_core::fdc::{dskreg, DiskCart, JVCDisk};
use coco_core::snapshot::{self, MediaRef, MediaRefs, MediaSources, SnapshotError};
use coco_core::ssc::{cmd as ssc_cmd, reg as ssc_reg, SoundSpeechCartridge};
use coco_core::{Machine, MachineConfig};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use mc6809::Bus;

use super::common::expect_err;

/// Header length ([`snapshot::CONTAINER_MAGIC`] + version byte + `u32`
/// schema) — mirrors `future_schema_is_reported_as_schema_too_new`'s own
/// `schema_offset` computation above; `snapshot::HEADER_LEN` itself isn't
/// public.
fn header_len() -> usize {
    snapshot::CONTAINER_MAGIC.len() + 1 + 4
}

/// Split a real `.ccstate` container (from [`snapshot::save`]) into its raw
/// (ungzipped) CBOR payload bytes, discarding the header.
fn cbor_body_of(container: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    GzDecoder::new(&container[header_len()..]).read_to_end(&mut out).expect("gunzip");
    out
}

/// Decode `cbor`, walk `path` as a chain of map keys (every
/// `#[derive(Serialize)]` struct/enum in this crate's tree serializes as a
/// CBOR map keyed by field/variant name — the module doc's "CBOR carries
/// field names" claim, load-bearing here), overwrite the leaf with
/// `new_value`, and re-encode.
fn mutate_cbor(cbor: &[u8], path: &[&str], new_value: Value) -> Vec<u8> {
    let mut root: Value = ciborium::from_reader(cbor).expect("decode cbor");
    let mut cursor = &mut root;
    for key in path {
        let map = cursor.as_map_mut().unwrap_or_else(|| panic!("expected a map navigating to {key:?}"));
        cursor = &mut map
            .iter_mut()
            .find(|(k, _)| k.as_text() == Some(*key))
            .unwrap_or_else(|| panic!("missing CBOR map key {key:?} (path {path:?})"))
            .1;
    }
    *cursor = new_value;
    let mut out = Vec::new();
    ciborium::into_writer(&root, &mut out).expect("encode cbor");
    out
}

/// Gzip `cbor` and prepend a real container header at `schema` — the
/// re-assembly half of the hand-mutate round trip, mirroring
/// [`snapshot::save`]'s own tail (that function isn't reusable directly:
/// it takes a `&Machine`, not raw CBOR bytes).
fn rewrap_container(cbor: &[u8], schema: u32) -> Vec<u8> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(cbor).expect("gzip cbor");
    let compressed = gz.finish().expect("finish gzip");
    let mut out = Vec::new();
    out.extend_from_slice(snapshot::CONTAINER_MAGIC);
    out.push(snapshot::CONTAINER_VERSION);
    out.extend_from_slice(&schema.to_le_bytes());
    out.extend_from_slice(&compressed);
    out
}

/// A minimal machine (empty system ROM — nothing here executes any CPU
/// instructions) with an [`SoundSpeechCartridge`] plugged directly into the cartridge port.
fn machine_with_ssc() -> Machine {
    let mut machine = Machine::new(MachineConfig::default(), Box::new([]));
    machine.insert_cartridge(SoundSpeechCartridge::new());
    machine
}

#[test]
fn ssc_load_cap_past_ram_size_is_invalid_payload_not_a_panic() {
    let mut machine = machine_with_ssc();
    // `$98` = LOAD_SOUND_INDIVIDUAL_START buffer 0: a legitimate `$FF7E`
    // write that leaves the SSC mid `Mode::Loading(Load { cursor: 0, cap:
    // 64, .. })` — see `SoundSpeechCartridge::start_load_individual`.
    machine.bus.cart.write(ssc_reg::DATA, ssc_cmd::LOAD_SOUND_INDIVIDUAL_START);

    let bytes = snapshot::save(&machine, &MediaRefs::default()).expect("save");
    let cbor = cbor_body_of(&bytes);
    // Past `ram::SIZE` (512): `SoundSpeechCartridge::feed_load` would index `self.ram[cursor]`
    // for any `cursor` up to (but not including) `cap` with no bounds check
    // against `ram::SIZE` of its own.
    let tampered = mutate_cbor(
        &cbor,
        &["machine", "bus", "cart", "SoundSpeechCartridge", "mode", "Loading", "cap"],
        Value::Integer(999_999.into()),
    );
    let bytes = rewrap_container(&tampered, snapshot::SCHEMA_VERSION);

    let payload = snapshot::load(&bytes).expect("load (schema/magic still valid)");
    let err = expect_err(snapshot::restore(payload, MediaSources::default()));
    assert!(matches!(err, SnapshotError::InvalidPayload(_)), "{err:?}");
}

/// A minimal machine with an FD-502 holding a single blank (synthetic, not
/// `roms/disk11.rom`) headerless disk in drive 0 — no CPU stepping needed,
/// register writes reach the controller directly through the fixed I/O
/// page.
fn machine_with_disk_in_read_transfer() -> Machine {
    let mut machine = Machine::new(MachineConfig::default(), Box::new([]));
    // Any nonempty bytes: `DiskCart::new` only rejects an EMPTY image
    // (`ROMPak::from_bytes`) — this never needs to be a real Disk BASIC ROM
    // since nothing here executes a CPU instruction.
    let mut cart = DiskCart::new(vec![0u8; 16].into_boxed_slice());
    const ONE_TRACK_BYTES: usize = 18 * 256;
    cart.insert_disk(0, JVCDisk::from_bytes(vec![0u8; ONE_TRACK_BYTES]).expect("build disk"));
    machine.insert_cartridge(cart);

    const DSKREG: u16 = 0xFF40;
    const SECTOR_REG: u16 = 0xFF4A;
    const STATUS_COMMAND_REG: u16 = 0xFF48;
    machine.bus.write(DSKREG, dskreg::MOTOR_ON | dskreg::DRIVE0);
    // `JvcDisk`'s default `first_sector_id` is 1 — sector 0 doesn't exist
    // (`JvcDisk::sector_offset`'s `checked_sub` underflows and the command
    // would settle as not-found instead of starting a transfer).
    machine.bus.write(SECTOR_REG, 1);
    machine.bus.write(STATUS_COMMAND_REG, 0x80); // Read Sector, single, track 0 sector 1
    machine
}

#[test]
fn wd1773_transfer_index_past_buf_len_is_invalid_payload_not_a_panic() {
    let machine = machine_with_disk_in_read_transfer();

    let bytes = snapshot::save(&machine, &MediaRefs::default()).expect("save");
    let cbor = cbor_body_of(&bytes);
    // Past `Transfer.buf`'s real length (one sector, 256 bytes): once the
    // transfer resumes, `WD1773::advance_transfer` would index `t.buf[t.index]`
    // once `t.index < t.total` — both of which a hostile payload also
    // controls, so nothing else stops this from running off the end of
    // `buf`.
    let tampered = mutate_cbor(
        &cbor,
        &["machine", "bus", "cart", "DiskCart", "fdc", "op", "Transfer", "index"],
        Value::Integer(999_999.into()),
    );
    let bytes = rewrap_container(&tampered, snapshot::SCHEMA_VERSION);

    let payload = snapshot::load(&bytes).expect("load (schema/magic still valid)");
    let err = expect_err(snapshot::restore(payload, MediaSources::default()));
    assert!(matches!(err, SnapshotError::InvalidPayload(_)), "{err:?}");
}

#[test]
fn cassette_bit_out_of_range_is_invalid_payload_not_a_panic() {
    let mut machine = Machine::new(MachineConfig::default(), Box::new([]));
    let tape_bytes = vec![0u8; 10];
    machine.bus.cassette.insert_tape(tape_bytes.clone());
    // `media.tape` must be recorded for `restore_tape` to call
    // `reattach_tape` at all (an unrecorded tape is skipped entirely) — the
    // path/hash themselves don't matter to this test.
    let media = MediaRefs {
        tape: Some(MediaRef { path: PathBuf::from("tape.cas"), sha256: snapshot::sha256_hex(&tape_bytes) }),
        ..MediaRefs::default()
    };

    let bytes = snapshot::save(&machine, &media).expect("save");
    let cbor = cbor_body_of(&bytes);
    // `Cassette::current_bit_is_one` shifts a tape byte right by `bit` with
    // no bounds check of its own: 8 (or higher) shift-overflow-panics in a
    // debug build and is unspecified in release.
    let tampered =
        mutate_cbor(&cbor, &["machine", "bus", "cassette", "bit"], Value::Integer(9.into()));
    let bytes = rewrap_container(&tampered, snapshot::SCHEMA_VERSION);

    let payload = snapshot::load(&bytes).expect("load (schema/magic still valid)");
    let sources = MediaSources { tape: Some(tape_bytes), ..MediaSources::default() };
    let err = expect_err(snapshot::restore(payload, sources));
    match err {
        SnapshotError::MediaShape { role, detail } => {
            assert_eq!(role, "tape");
            assert!(detail.contains("bit index"), "detail: {detail:?}");
        }
        other => panic!("expected MediaShape, got {other:?}"),
    }
}

#[test]
fn nested_multipak_is_invalid_payload_not_a_panic() {
    // 100% public API: `Cart: From<MultiPak>` makes a `MultiPak` itself
    // `impl Into<Cart>`, so nothing stops one MPI slot holding another —
    // `MultiPak::insert`'s signature has no way to reject it. Real hardware
    // can't build this (an MPI slot is a passive backplane connector, not
    // another MPI), so only a payload gets here.
    let mut machine = Machine::new(MachineConfig::default(), Box::new([]));
    let mut outer = MultiPak::new(0);
    outer.insert(0, MultiPak::new(0));
    machine.insert_cartridge(outer);

    let bytes = snapshot::save(&machine, &MediaRefs::default()).expect("save");
    let payload = snapshot::load(&bytes).expect("load");
    let err = expect_err(snapshot::restore(payload, MediaSources::default()));
    match err {
        SnapshotError::InvalidPayload(msg) => {
            assert!(msg.contains("Multi-Pak"), "message: {msg:?}");
        }
        other => panic!("expected InvalidPayload, got {other:?}"),
    }
}

/// Cap on the zero-fill this test inflates through a REAL gzip stream (not a
/// hand-crafted header) — comfortably past [`snapshot::MAX_PAYLOAD_BYTES`]
/// (64 MiB) so the cap trips before this test's own input is exhausted, and
/// small enough that compressing/writing it stays fast (all-zero input
/// compresses to a few KB either way).
const OVERSIZED_PAYLOAD_LEN: usize = 70 * 1024 * 1024;

#[test]
fn oversized_gzip_payload_is_rejected_without_allocating_it() {
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    gz.write_all(&vec![0u8; OVERSIZED_PAYLOAD_LEN]).expect("gzip zeros");
    let compressed = gz.finish().expect("finish gzip");

    let mut bytes = Vec::new();
    bytes.extend_from_slice(snapshot::CONTAINER_MAGIC);
    bytes.push(snapshot::CONTAINER_VERSION);
    bytes.extend_from_slice(&snapshot::SCHEMA_VERSION.to_le_bytes());
    bytes.extend_from_slice(&compressed);

    let err = expect_err(snapshot::load(&bytes));
    match err {
        SnapshotError::InvalidPayload(msg) => {
            assert!(msg.contains("MAX_PAYLOAD_BYTES"), "message: {msg:?}");
        }
        other => panic!("expected InvalidPayload, got {other:?}"),
    }
}
