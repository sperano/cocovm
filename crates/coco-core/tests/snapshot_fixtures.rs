//! Golden-fixture gate: "snapshots load forever"
//! (`docs/plan-machine-persistence.md` "Golden-fixture gate", phase 3 spec
//! item 5). Every fixture committed under `tests/fixtures/snapshots/` must
//! still load, restore, and continue trace-identically in every future
//! build. This file is both the generator (run once by hand, `#[ignore]`d)
//! and the CI gate that replays every committed fixture.
//!
//! ## Why a synthetic ROM, not `roms/coco3.rom`
//!
//! A machine booted from the real Super Extended Color BASIC ROM copies that
//! 32K image into RAM during its cold-start (`snapshot_roundtrip.rs`'s doc
//! comment documents exactly this), so its snapshot's `bus.ram` would carry
//! copyrighted bytes -- unsafe to commit. [`synthetic_rom`] is a small
//! hand-assembled 6809 program with no such content, so the `.ccstate`
//! fixture it boots is safe to commit alongside it.
//!
//! ## Fixture trio
//!
//! Each fixture is three files sharing a stem under
//! `tests/fixtures/snapshots/`:
//! - `<stem>.ccstate` -- the snapshot itself;
//! - `<stem>.rom` -- the synthetic system ROM it was booted from;
//! - `<stem>.trace` -- the expected continuation: one line per step for
//!   [`TRACE_STEPS`] steps after the snapshot point, in the format
//!   `pc,cc,a,b,x,y,u,s,dp,cycles` (register fields uppercase hex, no
//!   `0x`/`$` prefix, zero-padded to their natural width; `cycles` decimal),
//!   comma-separated, one line per step, `\n`-terminated -- see
//!   [`trace_line`].

use std::path::{Path, PathBuf};

use coco_core::snapshot::{self, MediaRef, MediaRefs, MediaSources};
use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/snapshots")
}

// ---- Synthetic ROM ----------------------------------------------------

const ROM_SIZE: usize = 32 * 1024;
/// RAM address the ROM's loop increments every iteration.
const COUNTER_ADDR: u16 = 0x0400;
/// GIME palette register 0 (`$FFB0-$FFBF`, `gime::PALETTE_LEN` entries) --
/// the loop mirrors its counter's low 6 bits here every iteration, so the
/// fixture also round-trips nontrivial GIME state, not just CPU/RAM.
const PALETTE_REG0: u16 = 0xFFB0;

/// Hand-assembled 6809 program, 32K, mapped to `$8000-$FFFF`:
/// ```text
/// $8000  10 CE 5E FF   LDS  #$5EFF
/// $8004  86 01         LDA  #$01
/// $8006  1F 8B         TFR  A,DP
/// loop:
/// $8008  7C 04 00      INC  $0400
/// $800B  B6 04 00      LDA  $0400
/// $800E  84 3F         ANDA #$3F
/// $8010  B7 FF B0      STA  $FFB0
/// $8013  20 F3         BRA  loop
/// $FFFE  80 00         (RESET vector -> $8000)
/// ```
/// Sets a stack pointer and a nonzero direct page (so both round-trip
/// meaningfully, not just their power-on-zero default), then loops forever:
/// increments a RAM counter and mirrors its low 6 bits into GIME palette
/// register 0. No interrupts used -- no NMI/IRQ/FIRQ vectors are set. Fully
/// deterministic and infinite, so any warmup/trace step count is safe.
fn synthetic_rom() -> Box<[u8]> {
    let mut rom = vec![0u8; ROM_SIZE];
    let [counter_hi, counter_lo] = COUNTER_ADDR.to_be_bytes();
    let [palette_hi, palette_lo] = PALETTE_REG0.to_be_bytes();
    rom[0x0000..0x0008].copy_from_slice(&[
        0x10, 0xCE, 0x5E, 0xFF, // LDS #$5EFF
        0x86, 0x01, // LDA #$01
        0x1F, 0x8B, // TFR A,DP
    ]);
    rom[0x0008..0x0015].copy_from_slice(&[
        0x7C, counter_hi, counter_lo, // INC $0400
        0xB6, counter_hi, counter_lo, // LDA $0400
        0x84, 0x3F, // ANDA #$3F
        0xB7, palette_hi, palette_lo, // STA $FFB0
        0x20, 0xF3, // BRA loop ($8008)
    ]);
    rom[0x7FFE..0x8000].copy_from_slice(&[0x80, 0x00]); // RESET vector -> $8000
    rom.into_boxed_slice()
}

fn synthetic_machine(rom: Box<[u8]>) -> Machine {
    let config =
        MachineConfig { variant: MachineVariant::Coco3, memory: MemorySize::K128, ..MachineConfig::default() };
    Machine::new(config, rom)
}

// ---- Trace format -------------------------------------------------------

/// One `pc,cc,a,b,x,y,u,s,dp,cycles` line for `m`'s current CPU state -- see
/// this file's module doc for the exact field order/format.
fn trace_line(m: &Machine) -> String {
    let cpu = &m.cpu;
    format!(
        "{:04X},{:02X},{:02X},{:02X},{:04X},{:04X},{:04X},{:04X},{:02X},{}",
        cpu.pc, cpu.cc, cpu.a, cpu.b, cpu.x, cpu.y, cpu.u, cpu.s, cpu.dp, cpu.cycles
    )
}

/// Steps after the snapshot point every fixture's `.trace` file records.
const TRACE_STEPS: u32 = 2_000;

/// Driven by `Machine::step_instruction` (the full per-scanline pipeline),
/// not the bare CPU-only `Machine::step`, so the fixture payload carries
/// live mid-field state — `line`/`line_cycles_spent`, the latched
/// `field_scan`, GIME timer/IRQ latches — and format rot in any of those
/// fields is caught by this gate instead of hiding behind their defaults.
fn continuation_trace(m: &mut Machine) -> Vec<String> {
    (0..TRACE_STEPS)
        .map(|_| {
            m.step_instruction();
            trace_line(m)
        })
        .collect()
}

// ---- Generator (run once by hand; produces the committed fixture files) --

/// Steps run before the snapshot point. Arbitrary but deterministic -- the
/// synthetic ROM's loop never terminates or diverges, so any value works;
/// this is well past several palette-register cycles.
const WARMUP_STEPS: u32 = 5_000;

/// Not a CI test: writes the three `v1-synthetic.*` files under
/// `tests/fixtures/snapshots/`, to be committed alongside this branch. Run
/// once by hand: `cargo test -p coco-core -- --ignored generate_golden_fixture`.
#[test]
#[ignore]
fn generate_golden_fixture() {
    let rom = synthetic_rom();
    let mut m = synthetic_machine(rom.clone());
    for _ in 0..WARMUP_STEPS {
        m.step_instruction();
    }

    let media = MediaRefs {
        system_rom: Some(MediaRef {
            path: PathBuf::from("v1-synthetic.rom"),
            sha256: snapshot::sha256_hex(&rom),
        }),
        ..MediaRefs::default()
    };
    let ccstate = snapshot::save(&m, &media).expect("save");
    let trace = continuation_trace(&mut m);

    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir).expect("create tests/fixtures/snapshots");
    std::fs::write(dir.join("v1-synthetic.ccstate"), &ccstate).expect("write .ccstate");
    std::fs::write(dir.join("v1-synthetic.rom"), &rom).expect("write .rom");
    std::fs::write(dir.join("v1-synthetic.trace"), trace.join("\n") + "\n").expect("write .trace");
}

// ---- Gate: every committed fixture must still load and continue ---------

/// A committed fixture's gzipped-CBOR payload must stay small: 128K of
/// mostly-repetitive RAM gzips to a few KB. A fixture anywhere near this cap
/// means something started embedding real media (disk/VHD/tape bytes) into
/// a snapshot fixture -- exactly what [`MediaRefs`] (path+hash, not content)
/// exists to avoid.
const FIXTURE_MAX_BYTES: u64 = 200 * 1024;

#[test]
fn all_committed_fixtures_still_load() {
    let dir = fixtures_dir();
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.expect("fixtures dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("ccstate") {
            continue;
        }
        check_fixture(&path);
        checked += 1;
    }
    assert!(checked > 0, "no *.ccstate fixtures found under {}", dir.display());
}

/// Load, restore, and trace-continue one fixture, comparing against its
/// committed `.trace` -- future fixtures (`<stem>.ccstate` + `<stem>.rom` +
/// `<stem>.trace`) join this gate automatically via [`all_committed_fixtures_still_load`]'s
/// directory scan.
fn check_fixture(ccstate_path: &Path) {
    let stem = ccstate_path.file_stem().and_then(|s| s.to_str()).expect("fixture file stem");
    let dir = ccstate_path.parent().expect("fixture parent dir");
    let rom_path = dir.join(format!("{stem}.rom"));
    let trace_path = dir.join(format!("{stem}.trace"));

    let ccstate =
        std::fs::read(ccstate_path).unwrap_or_else(|e| panic!("read {}: {e}", ccstate_path.display()));
    assert!(
        ccstate.len() as u64 <= FIXTURE_MAX_BYTES,
        "fixture {stem} is {} bytes, over the {FIXTURE_MAX_BYTES}-byte cap -- did something \
         start embedding media bytes into a snapshot fixture?",
        ccstate.len()
    );
    let rom = std::fs::read(&rom_path).unwrap_or_else(|e| panic!("read {}: {e}", rom_path.display()));
    let expected_trace =
        std::fs::read_to_string(&trace_path).unwrap_or_else(|e| panic!("read {}: {e}", trace_path.display()));

    let payload = snapshot::load(&ccstate).unwrap_or_else(|e| {
        panic!(
            "fixture {stem} failed to load -- the snapshot compatibility contract \
             (`coco_core::snapshot`'s module doc: a snapshot written today must load in every \
             future version) was broken: {e}"
        )
    });
    let sources = MediaSources { system_rom: Some(rom.into_boxed_slice()), ..MediaSources::default() };
    let mut restored = snapshot::restore(payload, sources)
        .unwrap_or_else(|e| panic!("fixture {stem} failed to restore: {e}"))
        .machine;

    let actual_trace = continuation_trace(&mut restored).join("\n") + "\n";
    assert_eq!(
        actual_trace, expected_trace,
        "fixture {stem}'s continuation trace no longer matches its committed .trace -- the \
         snapshot compatibility contract was broken (`coco_core::snapshot`'s module doc, \
         evolution rules 1-4; see also this file's own module doc)"
    );
}
