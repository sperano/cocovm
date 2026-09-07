//! 7. Phase-5 review: hostile-payload validation
//!
//! `Cassette::bit`, `SSC::tms_budget`, `SSC::tms`'s nested `TMS7040`
//! (`timer1.phase`, `cycles`), `WD1773::Transfer::index`,
//! `WD1773::Transfer::offset` plus `total`, and `Machine`'s scanline
//! scheduler fields (`line`, `line_cycles_spent`, `line_budget`) have no
//! public setter that can create an out-of-range value. Normal protocol
//! dispatch also keeps them in range. Unlike the RAM-length tamper test,
//! these tests mutate the CBOR bytes of a valid save, as a hex editor could,
//! using [`mutate_cbor`] and [`rewrap_container`] that follows.

use std::io::{Read, Write};
use std::path::PathBuf;

use ciborium::Value;
use coco_core::cart::MultiPak;
use coco_core::fdc::{DiskCart, JVCDisk, dskreg};
use coco_core::snapshot::{self, MediaRef, MediaRefs, MediaSources, SnapshotError};
use coco_core::ssc::SoundSpeechCartridge;
use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize, VDGVariant};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use mc6809::Bus;

use super::common::{boot_machine, expect_err, load_rom, system_rom_only_media};

/// Header length ([`snapshot::CONTAINER_MAGIC`] + version byte + `u32`
/// schema) — mirrors the `schema_offset` calculation in
/// `future_schema_is_reported_as_schema_too_new`; `snapshot::HEADER_LEN` itself isn't
/// public.
fn header_len() -> usize {
    snapshot::CONTAINER_MAGIC.len() + 1 + 4
}

/// Split a real `.ccstate` container (from [`snapshot::save`]) into its raw
/// (ungzipped) CBOR payload bytes, discarding the header.
fn cbor_body_of(container: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    GzDecoder::new(&container[header_len()..])
        .read_to_end(&mut out)
        .expect("gunzip");
    out
}

/// Decodes `cbor` and walks `path` as a chain of map keys. Every
/// `#[derive(Serialize)]` struct/enum in this crate's tree serializes as a
/// CBOR map keyed by field/variant name — the module doc's "CBOR carries
/// field names" claim, which this test relies on). It overwrites the leaf with
/// `new_value` and re-encodes the payload.
fn mutate_cbor(cbor: &[u8], path: &[&str], new_value: Value) -> Vec<u8> {
    let mut root: Value = ciborium::from_reader(cbor).expect("decode cbor");
    *navigate(&mut root, path) = new_value;
    let mut out = Vec::new();
    ciborium::into_writer(&root, &mut out).expect("encode cbor");
    out
}

/// Walks `path` as a chain of CBOR map keys from `root`, returning the leaf.
fn navigate<'a>(root: &'a mut Value, path: &[&str]) -> &'a mut Value {
    let mut cursor = root;
    for key in path {
        let map = cursor
            .as_map_mut()
            .unwrap_or_else(|| panic!("expected a map navigating to {key:?}"));
        cursor = &mut map
            .iter_mut()
            .find(|(k, _)| k.as_text() == Some(*key))
            .unwrap_or_else(|| panic!("missing CBOR map key {key:?} (path {path:?})"))
            .1;
    }
    cursor
}

/// Gzips `cbor` and prepends a container header at `schema` — the
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

/// A minimal machine with an empty system ROM and an
/// [`SoundSpeechCartridge`] plugged directly into the cartridge port.
fn machine_with_ssc() -> Machine {
    let mut machine = Machine::new(MachineConfig::default(), Box::new([]));
    let blank_firmware = [0; tms7000::ROM_SIZE];
    let blank_speech_rom = [0; coco_core::sp0256::ROM_SIZE];
    machine.insert_cartridge(
        SoundSpeechCartridge::new(&blank_firmware, &blank_speech_rom).expect("blank ROMs"),
    );
    machine
}

#[test]
fn ssc_tms_budget_outside_one_step_is_invalid_payload_not_a_panic() {
    let machine = machine_with_ssc();

    let bytes = snapshot::save(&machine, &MediaRefs::default()).expect("save");
    let cbor = cbor_body_of(&bytes);
    // A positive budget would have `Cartridge::tick` run the firmware for
    // as many instructions as the number says before the host gets a turn;
    // `SoundSpeechCartridge::validate_restored` bounds it to one step's debt.
    let tampered = mutate_cbor(
        &cbor,
        &[
            "machine",
            "bus",
            "cart",
            "SoundSpeechCartridge",
            "tms_budget",
        ],
        Value::Integer(999_999.into()),
    );
    let bytes = rewrap_container(&tampered, snapshot::SCHEMA_VERSION);

    let payload = snapshot::load(&bytes).expect("load (schema/magic still valid)");
    let err = expect_err(snapshot::restore(payload, MediaSources::default()));
    assert!(matches!(err, SnapshotError::InvalidPayload(_)), "{err:?}");
}

#[test]
fn ssc_tms_timer1_phase_past_period_is_invalid_payload_not_a_panic() {
    let machine = machine_with_ssc();

    let bytes = snapshot::save(&machine, &MediaRefs::default()).expect("save");
    let cbor = cbor_body_of(&bytes);
    // `Timer1::tick`'s `while phase >= period` loop assumes `phase` starts
    // below the period; a huge `phase` would spin it `phase / period` times
    // instead of the 0 a live chip ever needs. `cycles` near `u64::MAX` is
    // tampered alongside it (its own hazard is an overflow panic in
    // `TMS7040::step`, made saturating) to confirm the timer check rejects
    // the whole payload before either ever reaches `step`.
    let cbor = mutate_cbor(
        &cbor,
        &[
            "machine",
            "bus",
            "cart",
            "SoundSpeechCartridge",
            "tms",
            "timer1",
            "phase",
        ],
        Value::Integer(u32::MAX.into()),
    );
    let tampered = mutate_cbor(
        &cbor,
        &[
            "machine",
            "bus",
            "cart",
            "SoundSpeechCartridge",
            "tms",
            "cycles",
        ],
        Value::Integer(u64::MAX.into()),
    );
    let bytes = rewrap_container(&tampered, snapshot::SCHEMA_VERSION);

    let payload = snapshot::load(&bytes).expect("load (schema/magic still valid)");
    let err = expect_err(snapshot::restore(payload, MediaSources::default()));
    assert!(matches!(err, SnapshotError::InvalidPayload(_)), "{err:?}");
}

#[test]
fn ssc_tms_io_control_flag_without_a_source_is_invalid_payload_not_a_panic() {
    let machine = machine_with_ssc();

    let bytes = snapshot::save(&machine, &MediaRefs::default()).expect("save");
    let cbor = cbor_body_of(&bytes);
    // io_control's INT1 flag bit (0x02) set with neither pulse_latch nor
    // int_line true: no live path can produce this, and check_interrupts
    // would dispatch a spurious INT1 on the next step if it were allowed.
    let tampered = mutate_cbor(
        &cbor,
        &[
            "machine",
            "bus",
            "cart",
            "SoundSpeechCartridge",
            "tms",
            "io_control",
        ],
        Value::Integer(0x02.into()),
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
    cart.insert_disk(
        0,
        JVCDisk::from_bytes(vec![0u8; ONE_TRACK_BYTES]).expect("build disk"),
    );
    machine.insert_cartridge(cart);

    // Unit-level bus pokes, no booting (module doc): state the SCS-window
    // precondition (INIT0 MC2) explicitly so the following writes actually reach
    // the controller instead of being dropped by the closed gate.
    machine.bus.gime.write_init0(coco_core::gime::init0::MC2);

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
        &[
            "machine", "bus", "cart", "DiskCart", "fdc", "op", "Transfer", "index",
        ],
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
        tape: Some(MediaRef {
            path: PathBuf::from("tape.cas"),
            sha256: snapshot::sha256_hex(&tape_bytes),
        }),
        ..MediaRefs::default()
    };

    let bytes = snapshot::save(&machine, &media).expect("save");
    let cbor = cbor_body_of(&bytes);
    // `Cassette::current_bit_is_one` shifts a tape byte right by `bit` with
    // no bounds check of its own: 8 (or higher) shift-overflow-panics in a
    // debug build and is unspecified in release.
    let tampered = mutate_cbor(
        &cbor,
        &["machine", "bus", "cassette", "bit"],
        Value::Integer(9.into()),
    );
    let bytes = rewrap_container(&tampered, snapshot::SCHEMA_VERSION);

    let payload = snapshot::load(&bytes).expect("load (schema/magic still valid)");
    let sources = MediaSources {
        tape: Some(tape_bytes),
        ..MediaSources::default()
    };
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

/// Pins a rejection to the scheduler check that produced it, not just the variant.
fn assert_invalid_payload_mentions(err: SnapshotError, needle: &str) {
    match err {
        SnapshotError::InvalidPayload(msg) => {
            assert!(msg.contains(needle), "message: {msg:?}");
        }
        other => panic!("expected InvalidPayload, got {other:?}"),
    }
}

#[test]
fn scanline_past_lines_per_field_is_invalid_payload_not_a_panic() {
    let machine = Machine::new(MachineConfig::default(), Box::new([]));

    let bytes = snapshot::save(&machine, &MediaRefs::default()).expect("save");
    let cbor = cbor_body_of(&bytes);
    // Default config is Coco3/NTSC: 262 lines per field, so line 262 itself
    // is already outside the `0..lines_per_field` range `end_of_line` keeps
    // `Machine::line` in.
    let tampered = mutate_cbor(&cbor, &["machine", "line"], Value::Integer(262.into()));
    let bytes = rewrap_container(&tampered, snapshot::SCHEMA_VERSION);

    let payload = snapshot::load(&bytes).expect("load (schema/magic still valid)");
    let err = expect_err(snapshot::restore(payload, MediaSources::default()));
    assert_invalid_payload_mentions(err, "scanline");
}

#[test]
fn line_budget_past_max_speed_ceiling_is_invalid_payload_not_a_panic() {
    let machine = Machine::new(MachineConfig::default(), Box::new([]));

    let bytes = snapshot::save(&machine, &MediaRefs::default()).expect("save");
    let cbor = cbor_body_of(&bytes);
    // No variant/standard/speed-poke combination's cycles_per_field()/lines
    // reaches anywhere near this — see `Machine::max_line_budget`.
    let tampered = mutate_cbor(
        &cbor,
        &["machine", "line_budget"],
        Value::Integer(1_000_000.into()),
    );
    let bytes = rewrap_container(&tampered, snapshot::SCHEMA_VERSION);

    let payload = snapshot::load(&bytes).expect("load (schema/magic still valid)");
    let err = expect_err(snapshot::restore(payload, MediaSources::default()));
    assert_invalid_payload_mentions(err, "line budget");
}

#[test]
fn line_cycles_spent_near_u32_max_is_invalid_payload_not_a_panic() {
    let machine = Machine::new(MachineConfig::default(), Box::new([]));

    let bytes = snapshot::save(&machine, &MediaRefs::default()).expect("save");
    let cbor = cbor_body_of(&bytes);
    // A plausible budget, so the spent/budget check (not the ceiling) rejects
    // the value that would overflow `step_instruction`'s accumulation.
    let cbor = mutate_cbor(
        &cbor,
        &["machine", "line_budget"],
        Value::Integer(50.into()),
    );
    let tampered = mutate_cbor(
        &cbor,
        &["machine", "line_cycles_spent"],
        Value::Integer(u32::MAX.into()),
    );
    let bytes = rewrap_container(&tampered, snapshot::SCHEMA_VERSION);

    let payload = snapshot::load(&bytes).expect("load (schema/magic still valid)");
    let err = expect_err(snapshot::restore(payload, MediaSources::default()));
    assert_invalid_payload_mentions(err, "line_cycles_spent");
}

#[test]
fn line_cycles_spent_not_less_than_budget_is_invalid_payload_not_a_panic() {
    let machine = Machine::new(MachineConfig::default(), Box::new([]));

    let bytes = snapshot::save(&machine, &MediaRefs::default()).expect("save");
    let cbor = cbor_body_of(&bytes);
    // Both nonzero and equal: violates the documented `line_cycles_spent <
    // line_budget` invariant without tripping either check above on its own.
    let cbor = mutate_cbor(
        &cbor,
        &["machine", "line_budget"],
        Value::Integer(100.into()),
    );
    let tampered = mutate_cbor(
        &cbor,
        &["machine", "line_cycles_spent"],
        Value::Integer(100.into()),
    );
    let bytes = rewrap_container(&tampered, snapshot::SCHEMA_VERSION);

    let payload = snapshot::load(&bytes).expect("load (schema/magic still valid)");
    let err = expect_err(snapshot::restore(payload, MediaSources::default()));
    assert_invalid_payload_mentions(err, "line_cycles_spent");
}

/// Not hostile: a mid-line snapshot whose `line_budget` sits in the
/// double-speed range even though this machine never poked double speed —
/// legitimate when the poke happened earlier in the same scanline the
/// snapshot was taken on (`Machine`'s `line_budget` doc comment). Must still
/// restore, proving the scheduler checks don't false-positive on it.
#[test]
fn mid_line_scheduler_state_survives_restore() {
    let machine = boot_machine();

    let media = system_rom_only_media();
    let bytes = snapshot::save(&machine, &media).expect("save");
    let cbor = cbor_body_of(&bytes);
    let cbor = mutate_cbor(&cbor, &["machine", "line"], Value::Integer(10.into()));
    let cbor = mutate_cbor(
        &cbor,
        &["machine", "line_budget"],
        Value::Integer(100.into()),
    );
    let tampered = mutate_cbor(
        &cbor,
        &["machine", "line_cycles_spent"],
        Value::Integer(50.into()),
    );
    let bytes = rewrap_container(&tampered, snapshot::SCHEMA_VERSION);

    let payload = snapshot::load(&bytes).expect("load (schema/magic still valid)");
    let sources = MediaSources {
        system_rom: Some(load_rom()),
        ..MediaSources::default()
    };
    snapshot::restore(payload, sources).expect("restore");
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
    gz.write_all(&vec![0u8; OVERSIZED_PAYLOAD_LEN])
        .expect("gzip zeros");
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

/// Like [`mutate_cbor`] but inserts `key` into the map at `path`, replacing
/// it if present — for keys today's `save` no longer writes (a legacy field
/// a pre-change snapshot still carries).
fn insert_cbor_key(cbor: &[u8], path: &[&str], key: &str, new_value: Value) -> Vec<u8> {
    let mut root: Value = ciborium::from_reader(cbor).expect("decode cbor");
    let map = navigate(&mut root, path)
        .as_map_mut()
        .expect("leaf is a map");
    map.retain(|(k, _)| k.as_text() != Some(key));
    map.push((Value::Text(key.to_string()), new_value));
    let mut out = Vec::new();
    ciborium::into_writer(&root, &mut out).expect("encode cbor");
    out
}

/// A CoCo 1 config that passes `MachineConfig::validate` (64K, NTSC, no
/// monitor, plain MC6847) — the default config is a CoCo 3.
fn coco1_config() -> MachineConfig {
    MachineConfig {
        variant: MachineVariant::Coco1,
        memory: MemorySize::K64,
        monitor: None,
        vdg: Some(VDGVariant::MC6847),
        ..MachineConfig::default()
    }
}

#[test]
fn bus_variant_is_rederived_from_config_even_when_a_legacy_key_disagrees() {
    // `SystemBus::variant` used to be serialized alongside `config.variant`;
    // an old or hand-edited `.ccstate` can carry a `bus.variant` that
    // contradicts the config. The config is the only source of truth: the
    // restored bus decodes for the machine the config names, in both
    // directions.
    for (config, stale) in [
        (MachineConfig::default(), "Coco1"),
        (coco1_config(), "Coco3"),
    ] {
        let machine = Machine::new(config, Box::new([]));
        let bytes = snapshot::save(&machine, &MediaRefs::default()).expect("save");
        let cbor = cbor_body_of(&bytes);
        let tampered = insert_cbor_key(
            &cbor,
            &["machine", "bus"],
            "variant",
            Value::Text(stale.to_string()),
        );
        let bytes = rewrap_container(&tampered, snapshot::SCHEMA_VERSION);

        let payload = snapshot::load(&bytes).expect("load");
        let sources = MediaSources {
            system_rom: Some(Box::new([])),
            ..MediaSources::default()
        };
        let restored = snapshot::restore(payload, sources).expect("restore");
        assert_eq!(restored.machine.bus.variant, config.variant);
        assert_eq!(restored.machine.config.variant, config.variant);
    }
}
