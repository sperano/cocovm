//! 1. Full engine round-trip

use coco_core::snapshot::{self, MediaSources};
use mc6809::{MC6809, State};

use super::common::{boot_machine, load_rom, system_rom_only_media};

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
    let sources = MediaSources {
        system_rom: Some(load_rom()),
        ..MediaSources::default()
    };
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
