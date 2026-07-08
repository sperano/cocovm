# Plan: Debugger / Monitor

A first-class interactive debugger — the force-multiplier for everything else
on the roadmap (HD6309 flag/cycle bugs, GIME raster work, cartridge bring-up).
Goal: Virtual][-class ergonomics, not a bolted-on afterthought.

## Current state (verified in-repo)

- `crates/coco-egui/src/main.rs:12` — "the debugger panels are still TODO".
- `main.rs:128` `running: bool` already gates emulation in `update()`
  (main.rs:917): pause exists, but only at field granularity.
- `Machine::step()` (lib.rs:150) exists but bypasses ALL peripheral ticking,
  interrupt servicing, hsync/vsync, and HALT handling — it's `cpu.step` raw.
  Fine for unit tests, wrong for a debugger (stepping would freeze the GIME
  timer, FDC, cassette, and never deliver interrupts).
- The real loop is `Machine::run_field()` (lib.rs:212): per-scanline
  `run_cycles()` → `hsync()` → audio sample, with field-sync edges at fixed
  lines. All loop state (current line, cycle budget) is **local** — the machine
  cannot stop mid-field and resume. This is the core refactor.
- No disassembler exists anywhere in the workspace.
- `examples/trace.rs` duplicates `run_field`'s structure by hand (and its
  consts) — a symptom of the non-resumable loop; the refactor should let it
  collapse onto the real loop.

## Design

### 1. `mc6809` crate: disassembler (`disasm.rs`)
Pure function, no CPU state:
```rust
pub struct Insn { pub len: u8, pub mnemonic: &'static str, pub operand: String }
pub fn disassemble(read: &mut impl FnMut(u16) -> u8, pc: u16) -> Insn
```
- Full opcode map incl. `$10`/`$11` pages and every indexed postbyte form
  (5/8/16-bit offsets, auto inc/dec, A/B/D offsets, PCR, indirect, extended
  indirect) — mirrors what the executing core already implements; the core *is*
  the authority (it boots BASIC), cross-checked against
  `docs/6x09_Instruction_Sets.pdf` for mnemonics/operand syntax.
- Byte reader is a closure so the UI can disassemble through the live bus
  **without side effects** (see "side-effect-free reads" below).
- Designed to grow HD6309 mnemonics later (same table-driven shape).

### 2. `coco-core`: resumable machine loop + debug core (`debug.rs`)
**The load-bearing refactor.** Move `run_field`'s loop state into `Machine`
fields (`line: u32`, `cycles_left_in_line: u32`) so execution is resumable at
instruction granularity:
- `Machine::step_instruction() -> StepEvent` — exactly one instruction (or one
  burned HALT cycle) with **full fidelity**: peripheral ticks, NMI/FIRQ/IRQ
  servicing, hsync/audio/field-sync when the line budget crosses, field wrap.
- `run_field()` becomes `while !field_boundary { step_instruction() }` —
  **behaviorally identical**; the whole existing test suite (202 tests, boot
  traces) is the regression harness and must stay green with zero diffs.
- `Debugger` struct owned by the frontend, passed by `&mut`:
  - PC breakpoints (`HashSet<u16>`), enable/disable, hit counts.
  - Memory watchpoints (read/write/access on CPU address) — checked via a
    cheap `Option<&WatchSet>` hook in `SystemBus` read/write; zero cost when
    `None` (the common case).
  - `run_until(&mut self, m: &mut Machine, budget) -> StopReason`
    (`Breakpoint(pc)`, `Watchpoint{addr, kind}`, `FieldComplete`, `Step`).
  - Instruction **trace ring buffer** (last N PCs + register snapshots) for
    "how did I get here", exportable in `trace.rs` format for MAME diffing.
- **Side-effect-free bus reads** for UI: `SystemBus::peek(addr) -> u8` that
  routes like `read` but never mutates (no PIA flag clears, no FDC status
  side effects, no GIME IRQ acks). Needed by disasm view, memory view, stack
  view. Devices get a `peek` path or the bus returns last-latched values.

### 3. `coco-egui`: debugger UI
Toggled with **F11** (F9/F10/F12 taken). One `egui::Window` cluster
(dockable panels), matching the existing `window_title` style:
- **Controls**: Run/Pause, Step In, Step Over (temp breakpoint after
  JSR/BSR/LBSR — falls back to Step In otherwise), Step Out (run until S
  rises past current frame), Step Scanline, Step Field, Run-to-cursor.
- **Registers**: A B D X Y U S PC DP CC (CC as EFHINZVC flag toggles),
  editable while paused; cycle counter; current scanline.
- **Disassembly**: scrollback around PC, breakpoint gutter (click to toggle),
  current-PC highlight, follows execution or free-navigates (address box).
- **Memory**: hex+ASCII grid, editable while paused; view either **CPU
  logical** (through the live MMU) or **physical RAM** (bank picker); goto
  address; highlight watchpoints.
- **Stack**: S-relative walk with return-address annotation.
- **Hardware state**: GIME regs ($FF90-$FF9F decoded — IRQ/FIRQ enables +
  pending, timer, video mode via `video_mode_summary`), MMU task/bank map,
  PIA0/PIA1 ports (DDR/output/input/control, CB1/CA1 flag state), cart line
  states (HALT/NMI/CART).
- Pause also freezes audio cleanly (drop owed fields — main.rs:44 comment
  already anticipates "debugger pause").

### Explicitly deferred (keep v1 shippable)
Conditional breakpoints/expressions, symbol files, source-level OS-9 module
awareness, GIME palette/video visualizers, watch expressions. The
`Debugger`/`StopReason` shapes leave room for all of these.

## Task breakdown & model assignment

| # | Task | Model | Rationale |
|---|------|-------|-----------|
| 1 | **Disassembler** in `mc6809` + tests (known byte sequences from the ROM + every addressing mode) | **Sonnet** | Table-driven, fully specified by the existing core; voluminous but mechanical. |
| 2 | **Resumable loop refactor + debug core**: `step_instruction`, `Machine` line-state fields, `Debugger`/`run_until`, watch hooks, `peek`, trace ring | **Opus** | Touches the machine's timing heart; a subtle regression (interrupt order, hsync placement, HALT edge) breaks everything downstream. Zero-diff test gate. |
| 3 | **Debugger UI** in coco-egui: panels, stepping controls, F11 toggle, editable state | **Sonnet** | Large but well-bounded egui work following existing window patterns. |
| 4 | **`trace.rs` collapse** onto the resumable loop + trace-ring export | **Haiku** | Deletion + thin glue once #2 lands. |
| 5 | **Verification**: full suite green, boot-trace diff vs pre-refactor byte-identical, manual step-through of the BASIC keyboard poll loop | **quick-check + trace-debug** | The refactor's acceptance gate. |

Sequence: 1 ∥ 2 → 3 → 4, 5 continuous.

## Acceptance
- Pause mid-BASIC, step instruction-by-instruction through the keyboard ROM
  loop with correct disassembly, registers, and GIME/PIA panels live.
- Breakpoint at `$A000`-area ROM routine hits; run resumes cleanly; audio
  doesn't glitch on pause/resume.
- Watchpoint on a text-screen byte fires on a `PRINT`.
- Pre/post-refactor `trace.rs` output byte-identical for 2M instructions,
  with and without a cart.
