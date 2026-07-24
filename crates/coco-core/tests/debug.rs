//! Debug core coverage: the resumable `step_instruction`/`run_field`
//! equivalence, side-effect-free `peek`, and the `Debugger`'s breakpoints,
//! watchpoints, and trace ring (`docs/plan-debugger.md` §2).

use std::path::PathBuf;

use coco_core::debug::{Debugger, StopReason, TraceEntry, WatchKind, WatchTable};
use coco_core::pia::cr;
use coco_core::{Machine, MachineConfig, StepKind};
use mc6809::{Bus, MC6809};

/// CoCo 3 legacy text-screen base (SAM page): cold-start BASIC clears it, so a
/// write watch here trips deterministically during boot.
const TEXT_SCREEN_BASE: u16 = 0x0400;

fn load_rom() -> Box<[u8]> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/coco3.rom");
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .into_boxed_slice()
}

fn boot_machine() -> Machine {
    Machine::new(MachineConfig::default(), load_rom())
}

/// A field driven one `step_instruction` at a time must land in exactly the
/// same state as one `run_field` — the zero-diff refactor's core promise,
/// exercised on real booting ROM code across several fields (so interrupts,
/// hsync, and the GIME timer are all in play).
#[test]
fn step_instruction_loop_matches_run_field() {
    let mut via_run = boot_machine();
    let mut via_step = boot_machine();

    for _ in 0..3 {
        via_run.run_field();
    }
    for _ in 0..3 {
        while !via_step.step_instruction().field_complete {}
    }

    assert_eq!(via_run.cpu.pc, via_step.cpu.pc, "PC diverged");
    assert_eq!(
        via_run.cpu.cycles, via_step.cpu.cycles,
        "cycle count diverged"
    );
    assert_eq!(via_run.cpu.s, via_step.cpu.s, "S diverged");
    assert_eq!(via_run.cpu.a, via_step.cpu.a);
    assert_eq!(via_run.cpu.b, via_step.cpu.b);
    assert_eq!(via_run.cpu.x, via_step.cpu.x);
    assert_eq!(via_run.cpu.y, via_step.cpu.y);
    assert_eq!(via_run.cpu.u, via_step.cpu.u);
    assert_eq!(via_run.cpu.cc, via_step.cpu.cc);
    assert_eq!(
        via_run.framebuffer, via_step.framebuffer,
        "framebuffer diverged"
    );
}

/// `step_instruction` on a fresh machine retires a real instruction (the
/// cold-start code is running, HALT* is not asserted) and reports its cost.
#[test]
fn step_instruction_retires_one_instruction() {
    let mut m = boot_machine();
    let pc_before = m.cpu.pc;
    let cycles_before = m.cpu.cycles;
    let event = m.step_instruction();
    match event.kind {
        StepKind::Instruction { cycles } => {
            assert!(cycles > 0);
            assert_eq!(m.cpu.cycles, cycles_before + cycles as u64);
        }
        StepKind::HaltCycle => panic!("cold start should not be halted"),
    }
    assert_ne!(m.cpu.pc, pc_before, "PC should advance");
}

/// A PC breakpoint stops `run_until` before the instruction executes, and its
/// hit count increments. A disabled breakpoint is ignored.
///
/// `run_until` also stops at each `FieldComplete`, so the breakpoint target is
/// chosen well within the first field (cold start is deterministic before any
/// interrupt, so a probe machine reproduces the same PC stream).
#[test]
fn breakpoint_stops_before_execution() {
    // Find a real, reachable PC ~40 instructions into the cold start.
    const PROBE_STEPS: usize = 40;
    let start_pc = boot_machine().cpu.pc;
    let target = {
        let mut probe = boot_machine();
        for _ in 0..PROBE_STEPS {
            probe.step_instruction();
        }
        probe.cpu.pc
    };
    assert_ne!(target, start_pc, "target must differ from the parked PC");

    let mut m = boot_machine();
    let mut dbg = Debugger::new();
    dbg.add_breakpoint(target);

    let reason = dbg.run_until(&mut m, 100_000);
    assert_eq!(reason, StopReason::Breakpoint(target));
    assert_eq!(m.cpu.pc, target, "must stop at the breakpoint, not past it");
    assert_eq!(dbg.breakpoint(target).unwrap().hits, 1);

    // Disabled: the same run makes progress without stopping here again.
    dbg.set_breakpoint_enabled(target, false);
    let reason = dbg.run_until(&mut m, 100);
    assert_ne!(reason, StopReason::Breakpoint(target));
    assert_ne!(m.cpu.pc, target);
}

/// `run_until` resumes cleanly off a breakpoint it is currently parked on
/// (doesn't immediately re-trigger the same address).
#[test]
fn run_until_resumes_off_parked_breakpoint() {
    let mut m = boot_machine();
    let mut dbg = Debugger::new();
    let parked = m.cpu.pc;
    dbg.add_breakpoint(parked);
    let reason = dbg.run_until(&mut m, 50);
    assert_ne!(reason, StopReason::Breakpoint(parked));
    assert_ne!(m.cpu.pc, parked);
}

/// The bus watch hook records the first matching access only while a table is
/// installed, respects the read/write direction, and stays silent otherwise
/// (the hot path when no watchpoints exist).
#[test]
fn watch_hook_traps_only_installed_matching_accesses() {
    let mut m = boot_machine();

    // No table installed: nothing recorded (the None fast path).
    m.bus.write(TEXT_SCREEN_BASE, 0x55);
    assert!(m.bus.take_watch_hit().is_none());

    // Write-only watch on the address.
    let mut table = WatchTable::default();
    table.watch(TEXT_SCREEN_BASE, false, true);
    m.bus.install_watches(table);

    // A read does not trip a write-only watch.
    m.bus.clear_watch_hit();
    let _ = m.bus.read(TEXT_SCREEN_BASE);
    assert!(m.bus.take_watch_hit().is_none());

    // A write does, and only the first hit is kept.
    m.bus.clear_watch_hit();
    m.bus.write(TEXT_SCREEN_BASE, 0x11);
    m.bus.write(TEXT_SCREEN_BASE, 0x22);
    let hit = m.bus.take_watch_hit().expect("write should trip the watch");
    assert_eq!(hit.addr, TEXT_SCREEN_BASE);
    assert_eq!(hit.kind, WatchKind::Write);

    // An empty table installs as the None fast path again.
    m.bus.install_watches(WatchTable::default());
    m.bus.write(TEXT_SCREEN_BASE, 0x33);
    assert!(m.bus.take_watch_hit().is_none());
}

/// End-to-end: `run_until` stops with `Watchpoint` when the booting ROM writes
/// a watched address (BASIC clears the text screen at cold start), and bumps
/// the watchpoint's hit count. `run_until` also returns at each field boundary,
/// so the caller loops across `FieldComplete` until the watch trips (as the UI
/// would, rendering a field each pass).
#[test]
fn run_until_stops_on_watchpoint() {
    let mut m = boot_machine();
    let mut dbg = Debugger::new();
    dbg.add_watchpoint(TEXT_SCREEN_BASE, false, true);

    // Cap the number of fields so a miss fails fast instead of hanging.
    const MAX_FIELDS: usize = 4000;
    let mut reason = StopReason::FieldComplete;
    for _ in 0..MAX_FIELDS {
        reason = dbg.run_until(&mut m, 20_000_000);
        if reason != StopReason::FieldComplete {
            break;
        }
    }
    assert_eq!(
        reason,
        StopReason::Watchpoint {
            addr: TEXT_SCREEN_BASE,
            kind: WatchKind::Write
        }
    );
    assert_eq!(dbg.watchpoint(TEXT_SCREEN_BASE).unwrap().hits, 1);
}

/// `peek` never clears a PIA Cx1 interrupt flag; a real `read` does.
#[test]
fn peek_does_not_clear_pia_flags() {
    let mut m = boot_machine();
    // PIA0 side A: select the data register (falling-edge C1), then latch CA1.
    m.bus.pia0.write(1, cr::DDR_ACCESS);
    m.bus.pia0.a.set_c1(false); // high->low sets C1_FLAG
    assert_ne!(m.bus.pia0.a.control & cr::C1_FLAG, 0, "flag should be set");

    let _ = m.bus.peek(0xFF00);
    let _ = m.bus.peek(0xFF00);
    assert_ne!(
        m.bus.pia0.a.control & cr::C1_FLAG,
        0,
        "peek must not clear the flag"
    );

    let _ = m.bus.read(0xFF00);
    assert_eq!(
        m.bus.pia0.a.control & cr::C1_FLAG,
        0,
        "read must clear the flag"
    );
}

/// `peek` returns the same bytes as `read` for the pure regions (RAM and
/// internal ROM), so the disassembly/memory views match what executes.
#[test]
fn peek_matches_read_for_ram_and_rom() {
    let mut m = boot_machine();
    for addr in [0x8000u16, 0x8C1B, 0xA000, 0xFFFE] {
        assert_eq!(
            m.bus.peek(addr),
            m.bus.read(addr),
            "ROM peek/read at {addr:04X}"
        );
    }
    m.bus.write(0x1234, 0xAB);
    assert_eq!(m.bus.peek(0x1234), 0xAB);
    assert_eq!(m.bus.peek(0x1234), m.bus.read(0x1234));
}

/// The trace ring records one entry per retired instruction, in the exact
/// `examples/trace.rs` text format, and is bounded by its capacity.
#[test]
fn trace_ring_records_and_formats() {
    let mut m = boot_machine();
    let mut dbg = Debugger::with_trace_capacity(4);
    dbg.trace_enabled = true;

    dbg.run_until(&mut m, 32);
    assert_eq!(dbg.trace().count(), 4, "ring must be capped at capacity");

    let text = dbg.export_trace();
    assert_eq!(text.lines().count(), 4);
    for line in text.lines() {
        assert!(
            line.contains(":  A="),
            "line not in trace.rs format: {line}"
        );
        assert!(line.contains(" CC="));
    }
}

/// `TraceEntry::format` matches `examples/trace.rs`'s `log_state` byte-for-byte.
#[test]
fn trace_entry_format_is_exact() {
    let mut cpu = MC6809::new();
    cpu.pc = 0x8C1B;
    cpu.a = 0x12;
    cpu.b = 0x34;
    cpu.x = 0x5678;
    cpu.y = 0x9ABC;
    cpu.u = 0xDEF0;
    cpu.s = 0x1357;
    cpu.dp = 0x24;
    cpu.cc = 0x68;
    let entry = TraceEntry::capture(&cpu);
    assert_eq!(
        entry.format(),
        "8C1B:  A=12 B=34 X=5678 Y=9ABC U=DEF0 S=1357 DP=24 CC=68"
    );
}
