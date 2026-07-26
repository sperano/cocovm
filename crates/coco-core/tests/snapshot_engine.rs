//! Phase-2 tests for the save-state snapshot *engine*
//! (`crates/coco-core/src/snapshot.rs`): the `.ccstate` container format,
//! media-reference handling, and the restore flow — everything phase 1's
//! `snapshot_roundtrip.rs` deliberately left for "a later phase" once the
//! CBOR payload got wrapped in the real container.

use std::io::{Read, Write};
use std::path::PathBuf;

use ciborium::Value;
use coco_core::cart::{Cart, Cartridge, MultiPak, RomPak};
use coco_core::fdc::{DiskCart, JvcDisk, dskreg};
use coco_core::snapshot::{
    self, MediaCheck, MediaRef, MediaRefs, MediaSources, SlotRomRef, SnapshotError, SnapshotPayload,
};
use coco_core::ssc::{Ssc, cmd as ssc_cmd, reg as ssc_reg};
use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use mc6809::{Bus, MC6809, State};

fn rom_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/coco3.rom")
}

fn load_rom() -> Box<[u8]> {
    std::fs::read(rom_path())
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", rom_path().display()))
        .into_boxed_slice()
}

fn boot_machine() -> Machine {
    Machine::new(MachineConfig::default(), load_rom())
}

/// Same CPU-trace-identity snapshot as `snapshot_roundtrip.rs`'s
/// `CpuSnapshot` (not exported from that test binary, so duplicated here —
/// see that file's doc comment for the rationale).
#[derive(Debug, PartialEq)]
struct CpuSnapshot {
    a: u8,
    b: u8,
    x: u16,
    y: u16,
    u: u16,
    s: u16,
    pc: u16,
    dp: u8,
    cc: u8,
    cycles: u64,
    state: State,
}

impl CpuSnapshot {
    fn of(cpu: &MC6809) -> Self {
        Self {
            a: cpu.a,
            b: cpu.b,
            x: cpu.x,
            y: cpu.y,
            u: cpu.u,
            s: cpu.s,
            pc: cpu.pc,
            dp: cpu.dp,
            cc: cpu.cc,
            cycles: cpu.cycles,
            state: cpu.state,
        }
    }
}

const WARMUP_STEPS: u32 = 200_000;
/// The plan's acceptance bar (`docs/plan-save-states.md` "Acceptance"):
/// "a 1M-instruction trace.rs-style log from the restore point is
/// identical to an unsnapshotted run."
const LOCKSTEP_STEPS: u32 = 1_000_000;

/// `Result::unwrap_err` requires `T: Debug`, which `SnapshotPayload`/
/// `RestoredMachine` deliberately don't implement (they carry a whole
/// `Machine`, RAM included — a `Debug` dump of that isn't useful and isn't
/// worth deriving just for tests). This is the same "match instead of
/// unwrap_err" workaround, named for clarity at each call site.
fn expect_err<T>(result: Result<T, SnapshotError>) -> SnapshotError {
    match result {
        Ok(_) => panic!("expected an error, got Ok"),
        Err(e) => e,
    }
}

/// A `MediaRefs` recording just the real system ROM, for tests that need a
/// save/restore round trip but no other media.
fn system_rom_only_media() -> MediaRefs {
    MediaRefs {
        system_rom: Some(MediaRef {
            path: rom_path(),
            sha256: snapshot::sha256_file(&rom_path()).expect("hash roms/coco3.rom"),
        }),
        ..MediaRefs::default()
    }
}

// ---- 1. Full engine round-trip -------------------------------------------

/// THE acceptance gate (`docs/plan-save-states.md` "Acceptance", phase 3
/// spec item 1): boot on the real ROM, warm up well into BASIC's idle loop,
/// save through the engine, restore into a fresh `Machine`, then run
/// [`LOCKSTEP_STEPS`] (1M) instructions on both the original and the
/// restored machine side by side, comparing every CPU register/flag/cycle
/// count after every single step. Any device left out of the serde tree, or
/// restored into the wrong state, shows up here as a divergence.
///
/// Deliberately driven by `Machine::step_instruction` — the full per-scanline
/// pipeline (GIME timer ticks, PIA field-sync IRQs, audio-event flushing,
/// cartridge ticking, and the resumable `line`/`line_cycles_spent` state) —
/// not the bare CPU-only `Machine::step`. The snapshot lands mid-field at an
/// arbitrary instruction boundary, which is exactly what the frontend's
/// save-while-running does, and any of that loop state left out of the serde
/// tree diverges the IRQ timing within a field or two.
#[test]
fn full_round_trip_continues_trace_identically() {
    let mut original = boot_machine();
    for _ in 0..WARMUP_STEPS {
        original.step_instruction();
    }

    let media = system_rom_only_media();
    let bytes = snapshot::save(&original, &media).expect("save");

    let payload = snapshot::load(&bytes).expect("load");
    let sources = MediaSources { system_rom: Some(load_rom()), ..MediaSources::default() };
    let restored = snapshot::restore(payload, sources).expect("restore");
    let mut restored = restored.machine;

    for i in 0..LOCKSTEP_STEPS {
        let orig_event = original.step_instruction();
        let rest_event = restored.step_instruction();
        assert_eq!(
            orig_event, rest_event,
            "step event diverged at lockstep instruction {i}"
        );
        assert_eq!(
            CpuSnapshot::of(&original.cpu),
            CpuSnapshot::of(&restored.cpu),
            "CPU state diverged at lockstep instruction {i}"
        );
        assert_eq!(
            original.current_scanline(),
            restored.current_scanline(),
            "scanline position diverged at lockstep instruction {i}"
        );
    }
    assert_eq!(
        original.bus.ram, restored.bus.ram,
        "RAM contents diverged after {LOCKSTEP_STEPS} lockstep instructions"
    );
}

// ---- 2. Header checks ------------------------------------------------------

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

// ---- 3. Missing system ROM source -----------------------------------------

#[test]
fn missing_system_rom_source_is_missing_media() {
    let original = boot_machine();
    let media = system_rom_only_media();
    let bytes = snapshot::save(&original, &media).expect("save");
    let payload = snapshot::load(&bytes).expect("load");

    let err = expect_err(snapshot::restore(payload, MediaSources::default()));
    match err {
        SnapshotError::MissingMedia { descriptions } => {
            assert!(
                descriptions.iter().any(|d| d.contains("system ROM")),
                "descriptions {descriptions:?} should mention the system ROM"
            );
        }
        other => panic!("expected MissingMedia, got {other:?}"),
    }
}

// ---- 4. MediaRef::verify ----------------------------------------------------

/// A scratch file path under the OS temp dir, unique to this test process —
/// cleaned up by the caller (`RAII` would be nicer, but a bare helper keeps
/// this file's dependency list unchanged).
fn scratch_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("coco_snapshot_engine_test_{}_{name}", std::process::id()))
}

#[test]
fn media_ref_verify_reports_ok_mismatch_and_missing() {
    let path = scratch_path("verify");
    std::fs::write(&path, b"hello coco").unwrap();
    let media_ref = MediaRef { path: path.clone(), sha256: snapshot::sha256_hex(b"hello coco") };
    assert_eq!(media_ref.verify(), MediaCheck::Ok);

    std::fs::write(&path, b"goodbye coco").unwrap();
    match media_ref.verify() {
        MediaCheck::Mismatch { actual } => {
            assert_eq!(actual, snapshot::sha256_hex(b"goodbye coco"));
        }
        other => panic!("expected Mismatch, got {other:?}"),
    }

    std::fs::remove_file(&path).unwrap();
    assert_eq!(media_ref.verify(), MediaCheck::Missing);
}

// ---- 5. No-ROM-bytes gate (structural) -------------------------------------

#[test]
fn restored_payload_carries_no_rom_bytes_before_reattachment() {
    let mut machine = boot_machine();
    let pak_image = vec![0xA5u8; 4096];
    machine.insert_cartridge(RomPak::from_bytes(&pak_image, false).expect("build pak"));

    let bytes = snapshot::save(&machine, &MediaRefs::default()).expect("save");
    let payload: SnapshotPayload = snapshot::load(&bytes).expect("load");

    assert!(
        payload.machine.bus.rom.is_empty(),
        "deserialized-but-not-yet-reattached system ROM must be empty"
    );
    let cart_debug = format!("{:?}", payload.machine.bus.cart);
    assert!(
        matches!(payload.machine.bus.cart, Cart::RomPak(_)),
        "expected a RomPak cart, got {cart_debug}"
    );
    assert!(
        cart_debug.contains("image_len: 0"),
        "deserialized-but-not-yet-reattached pak image must be empty, got {cart_debug}"
    );
}

// ---- Cart-ROM reattachment (not one of the spec's numbered tests, but the
// only thing exercising `restore_cart_roms`/`require_cart_rom` end to end —
// every test above either has no cartridge inserted or never calls
// `restore`, so without this the whole cart-ROM reattachment path would be
// untested code). ------------------------------------------------------------

#[test]
fn direct_port_cart_rom_is_reattached_through_a_full_restore() {
    let mut machine = boot_machine();
    // A uniform fill so any byte read back proves the *mirrored* image
    // (not just offset 0) survived reattachment, without needing to work
    // out `RomPak`'s half-swap indexing by hand.
    let pak_image = vec![0x42u8; 1024];
    machine.insert_cartridge(RomPak::from_bytes(&pak_image, false).expect("build pak"));

    let media = MediaRefs {
        system_rom: Some(MediaRef {
            path: rom_path(),
            sha256: snapshot::sha256_file(&rom_path()).expect("hash roms/coco3.rom"),
        }),
        cart_roms: vec![SlotRomRef {
            mpi_slot: None,
            rom: MediaRef {
                path: PathBuf::from("pak.rom"),
                sha256: snapshot::sha256_hex(&pak_image),
            },
        }],
        ..MediaRefs::default()
    };
    let bytes = snapshot::save(&machine, &media).expect("save");
    let payload = snapshot::load(&bytes).expect("load");
    let sources = MediaSources {
        system_rom: Some(load_rom()),
        cart_roms: vec![(None, pak_image)],
        ..MediaSources::default()
    };
    let restored = snapshot::restore(payload, sources).expect("restore");

    match restored.machine.bus.cart {
        Cart::RomPak(pak) => assert_eq!(pak.rom_peek(0x8000), 0x42),
        other => panic!("expected RomPak, got {other:?}"),
    }
}

#[test]
fn missing_cart_rom_source_is_missing_media() {
    let mut machine = boot_machine();
    let pak_image = vec![0x99u8; 1024];
    machine.insert_cartridge(RomPak::from_bytes(&pak_image, false).expect("build pak"));

    let media = MediaRefs {
        system_rom: Some(MediaRef {
            path: rom_path(),
            sha256: snapshot::sha256_file(&rom_path()).expect("hash roms/coco3.rom"),
        }),
        cart_roms: vec![SlotRomRef {
            mpi_slot: None,
            rom: MediaRef {
                path: PathBuf::from("pak.rom"),
                sha256: snapshot::sha256_hex(&pak_image),
            },
        }],
        ..MediaRefs::default()
    };
    let bytes = snapshot::save(&machine, &media).expect("save");
    let payload = snapshot::load(&bytes).expect("load");
    // No `cart_roms` entry in `sources`: the pak's image is unresolved.
    let sources = MediaSources { system_rom: Some(load_rom()), ..MediaSources::default() };

    let err = expect_err(snapshot::restore(payload, sources));
    match err {
        SnapshotError::MissingMedia { descriptions } => {
            assert!(
                descriptions.iter().any(|d| d.contains("RomPak")),
                "descriptions {descriptions:?} should mention the missing RomPak ROM"
            );
        }
        other => panic!("expected MissingMedia, got {other:?}"),
    }
}

// ---- 6. RAM-length tamper ---------------------------------------------------

#[test]
fn ram_length_mismatch_is_an_invalid_payload_error_not_a_panic() {
    let config = MachineConfig {
        variant: MachineVariant::Coco3,
        memory: MemorySize::K128,
        ..MachineConfig::default()
    };
    let mut machine = Machine::new(config, Box::new([]));
    // Hand-tamper the RAM length so it no longer matches `config.memory`.
    machine.bus.ram = vec![0u8; 1].into_boxed_slice();

    let payload = SnapshotPayload { media: MediaRefs::default(), machine };
    let err = expect_err(snapshot::restore(payload, MediaSources::default()));
    assert!(matches!(err, SnapshotError::InvalidPayload(_)), "{err:?}");
}

/// Cheap companion to #5/#6: confirms the streamed [`snapshot::sha256_file`]
/// agrees with the in-memory [`snapshot::sha256_hex`] on the same bytes.
#[test]
fn sha256_file_matches_sha256_hex_of_the_same_bytes() {
    let path = scratch_path("hash");
    let content = b"the quick brown fox jumps over the lazy dog";
    std::fs::write(&path, content).unwrap();
    assert_eq!(snapshot::sha256_file(&path).unwrap(), snapshot::sha256_hex(content));
    std::fs::remove_file(&path).unwrap();
}

// ---- 7. Phase-5 review: hostile-payload validation -------------------------
//
// `Cassette::bit`/`SSC::Load::cap`/`WD1773::Transfer::index`/`WD1773::Transfer::
// offset+total` have no `pub` setter that can reach an out-of-range value —
// by design, the normal protocol dispatch that reaches these fields never
// produces one. So unlike #6's `ram`-length tamper (`bus.ram` is `pub`),
// these tests hand-mutate the actual CBOR bytes of a valid save, the same
// way a hex editor on a real `.ccstate` file would, via [`mutate_cbor`]/
// [`rewrap_container`] below.

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
/// instructions) with an [`Ssc`] plugged directly into the cartridge port.
fn machine_with_ssc() -> Machine {
    let mut machine = Machine::new(MachineConfig::default(), Box::new([]));
    machine.insert_cartridge(Ssc::new());
    machine
}

#[test]
fn ssc_load_cap_past_ram_size_is_invalid_payload_not_a_panic() {
    let mut machine = machine_with_ssc();
    // `$98` = LOAD_SOUND_INDIVIDUAL_START buffer 0: a legitimate `$FF7E`
    // write that leaves the SSC mid `Mode::Loading(Load { cursor: 0, cap:
    // 64, .. })` — see `Ssc::start_load_individual`.
    machine.bus.cart.write(ssc_reg::DATA, ssc_cmd::LOAD_SOUND_INDIVIDUAL_START);

    let bytes = snapshot::save(&machine, &MediaRefs::default()).expect("save");
    let cbor = cbor_body_of(&bytes);
    // Past `ram::SIZE` (512): `Ssc::feed_load` would index `self.ram[cursor]`
    // for any `cursor` up to (but not including) `cap` with no bounds check
    // against `ram::SIZE` of its own.
    let tampered = mutate_cbor(
        &cbor,
        &["machine", "bus", "cart", "Ssc", "mode", "Loading", "cap"],
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
    // (`RomPak::from_bytes`) — this never needs to be a real Disk BASIC ROM
    // since nothing here executes a CPU instruction.
    let mut cart = DiskCart::new(vec![0u8; 16].into_boxed_slice());
    const ONE_TRACK_BYTES: usize = 18 * 256;
    cart.insert_disk(0, JvcDisk::from_bytes(vec![0u8; ONE_TRACK_BYTES]).expect("build disk"));
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
