# Plan: Save states

Freeze/restore the whole machine — the Virtual][-class QoL feature DESIGN.md §9
told us to "decide now, not later". The core is already a plain owned tree (no
`Rc`/`RefCell`, per §2b), so this is mostly mechanical **except** for one
structural blocker and a handful of things that must *not* be serialized.

## Current state (verified in-repo)

- `cart.rs:1-5` names the blocker: `SystemBus.cart: Box<dyn Cartridge>` is not
  `Serialize`, so neither `SystemBus` nor `Machine` can derive serde.
- `MachineConfig` already derives Serialize/Deserialize (config.rs) — the only
  core type that does today.
- Device inventory that must snapshot (all owned, no interior mutability):
  `MC6809` (regs + `State` enum + cycles), `SystemBus` { ram, gime (incl. MMU,
  palette, timer, IRQ latches, sam_page, monitor type), pia0/pia1, keyboard,
  joysticks, cassette, bitbanger (+ DMP-105 interpreter/paper model behind it),
  vhd ×2, cart }, and `Machine`'s own loop state (`prev_halted`, audio buffer,
  graphics scratch — plus the `line`/`cycles_left_in_line` fields the debugger
  refactor adds, see `plan-debugger.md`).
- Host-side resources that must NOT be inside the snapshot, only *referenced*:
  ROM images (copyrighted), disk/VHD/cassette backing files, printer sinks
  (`Box<dyn PrinterSink>` holds an open host file), egui textures/audio.

## Design decisions

### 1. Cartridge serialization: **closed enum, not typetag**
Replace `Box<dyn Cartridge>` with an enum:
```rust
pub enum Cart { Empty, RomPak(RomPak), Disk(DiskCart), MultiPak(Box<MultiPak>), /* grows */ }
```
- The cartridge set is closed and in-crate — an enum is honest, faster (static
  dispatch), removes the `as_disk_cart`/`as_multipak` downcast hacks (become
  `match`/accessors), and derives serde directly. `MultiPak.slots` becomes
  `[Cart; 4]` (boxed for size), which is also self-recursive-safe.
- typetag would keep the open trait but adds a dyn-registry dependency for a
  set of types we fully control. Not worth it. (If out-of-crate carts ever
  matter, revisit.)
- Keep a slim `Cartridge` trait *internally* if it reduces duplication (enum
  delegates to it), but the stored type is the enum.

### 2. Snapshot = state + references, not content
```rust
pub struct Snapshot {
    version: u32,                  // bump on breaking layout change
    config: MachineConfig,
    machine: MachineState,         // the full serde tree
    media: MediaRefs,              // paths + content hashes of ROM/disks/VHD/tape
}
```
- ROM/disk/VHD/tape are stored as **path + SHA-256** (and image dirty state
  flushed before snapshot, like `flush_dirty_disks` on exit). On restore,
  verify hashes; mismatch → load with a warning, absent → error listing what
  to re-attach. Never embed copyrighted images.
- Media *positions* (FDC track/sector state, cassette sample cursor, VHD LSN
  regs) ARE machine state and serialize normally.
- Printer capture: serialize the DMP-105/paper state; the active `PrinterSink`
  file handle is frontend-owned — on restore, capture is simply "stopped"
  (document it).
- Format: `bincode` (or `postcard`) inside a small header — compact, fast; a
  `serde_json` debug dump behind a flag is handy for diffing two snapshots.

### 3. Non-serializable / skip fields
`#[serde(skip)]` + rebuild-on-restore for: `Machine.audio_buffer` (drop),
`graphics_scratch` (reallocate), any cached host handles. Every skip needs a
`fn after_restore(&mut self)` hook chain that re-derives them — make this a
convention, not ad-hoc.

### 4. Frontend UX
- Menu: Save State / Load State (rfd file dialog, `.ccstate`), plus N quick
  slots with keyboard chords; status-bar toast on save/load.
- Snapshot while paused OR running (take at a field boundary — after
  `run_field` returns; with the debugger refactor, any instruction boundary
  works).
- CLI: `--state <file>` to boot straight into a snapshot.

## Task breakdown & model assignment

| # | Task | Model | Rationale |
|---|------|-------|-----------|
| 1 | **`Cart` enum migration**: replace `Box<dyn Cartridge>` everywhere (bus, Machine insert/eject, MultiPak slots, frontend menus), delete downcasts, keep all 400+ tests green | **Opus** | Cross-cutting surgery through bus/frontend with MPI recursion; regression risk is the whole cartridge subsystem (FDC HALT/NMI timing must not change). |
| 2 | **Derive pass**: serde on every core struct (mc6809 + coco-core), `#[serde(skip)]` audit, `after_restore` hooks, version field | **Sonnet** | Broad but mechanical once the enum exists; the skip-audit checklist comes from this plan. |
| 3 | **Snapshot/restore engine**: `Snapshot` container, media refs + hashing, dirty-flush ordering, bincode format + version gate | **Sonnet** | Contained, well-specified. |
| 4 | **Frontend**: menu items, quick slots, toasts, `--state` CLI | **Haiku** | Mechanical UI following existing menu patterns. |
| 5 | **Tests**: round-trip equality (snapshot → restore → identical `trace.rs` continuation for N instructions — THE acceptance test), mid-FDC-transfer snapshot, MPI-with-FD-502 snapshot, missing-media restore error, version mismatch | **Sonnet** | The trace-continuation test is the strong gate and cheap to write against the existing trace harness. |

Sequence: 1 → 2 → 3 → (4 ∥ 5).

## Acceptance
- Snapshot mid-BASIC-program, restart the app, restore: execution continues and
  a 1M-instruction `trace.rs`-style log from the restore point is **identical**
  to an unsnapshotted run.
- Snapshot during a NitrOS-9 EOU session on VHD; restore works, `dir /dd` still
  correct.
- Snapshot mid-disk-read (HALT asserted) restores without corrupting the
  transfer.
- No ROM/disk bytes inside the `.ccstate` file, in the sense that matters:
  no *whole media image* (a full ROM/disk/VHD/tape file) is ever embedded
  (inspect). A mid-sector-transfer snapshot legitimately carries that one
  sector's bytes in the WD1773's `Transfer.buf` — the same way RAM carries
  loaded content — since that's in-flight device state, not a media file
  (`coco_core::snapshot`'s module doc).

## Risks
- **Cart enum blast radius** (task 1) — biggest single change; do it before any
  new peripherals land (every new cart makes it worse).
- **Hidden non-determinism**: any state accidentally left out shows up as
  trace divergence after restore — the trace-continuation test catches it;
  budget time to chase these.
- **Version churn** while other features are in flight — keep `version` cheap
  to bump and don't promise cross-version compat yet.
