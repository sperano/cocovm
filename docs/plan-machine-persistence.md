# Plan: Machine persistence (definitions for the CocoVM manager)

The manager window (worktree-vm-manager) lists multiple machine
configurations at once, VirtualBox-style. That needs machines to exist as
*files*, not CLI invocations. This plan covers the **cold layer** — machine
definitions — and how it composes with the existing hot layer
(`plan-save-states.md`, unchanged by this plan).

## Two layers, two lifetimes

| Layer | What | Format | Where | Plan |
|---|---|---|---|---|
| **Definition** (cold) | name + hardware config + attached media + peripherals | TOML, human-editable | `config_dir()/machines/<slug>.toml` | this plan |
| **Snapshot** (hot) | frozen CPU/RAM/device tree mid-execution | binary (`bincode`/`postcard`) | `data_dir()/machines/<slug>/` | `plan-save-states.md` |

Rationale for the split location:
- The user chose XDG on macOS *specifically* so config stays where terminal
  users can edit and version it (`paths.rs` module doc). Definitions are
  exactly that kind of file: one small TOML per machine, `git`-able,
  hand-editable. This finally uses `paths::config_dir()` (dead-code since
  PR #6, which named "config-file loading" as the next step).
- Snapshots and machine-owned artifacts (blank disks created for a machine,
  exported media) are big/binary → `data_dir()/machines/<slug>/`, mirroring
  roms/ and images/ already living under data.

## Definition schema (v1)

New frontend module `coco-egui/src/machine_def.rs` (core stays IO-free; the
frontend already owns media mounting and rfd/CLI paths):

```toml
schema = 1                    # integer, bump on breaking change
name = "Dev CoCo 3"           # display name (list row title)
created = "2026-07-16"        # informational

[hardware]
variant = "coco3"             # coco1 | coco2 | coco3
ram = "512k"                  # 4k…2048k, validated by MachineConfig::validate
video = "ntsc"                # ntsc | pal
monitor = "rgb"               # rgb | composite
vdg = "mc6847"                # mc6847 | mc6847t1 (meaningful on coco2 only)
rom = "/path/custom.rom"      # optional; default = rom_db composition

[media]                       # every key optional
cart = "/paks/arkanoid.ccc"
disk0 = "dev.dsk"             # relative ⇒ resolved against data_dir()/machines/<slug>/
disk1 = "/shared/utils.dsk"
vhd0 = "/vhd/68SDC.VHD"
tape = "session.cas"

[peripherals]
mpi = false
rtc = false                   # later: explicit MPI slot table replaces these
# [peripherals.mpi_slots] — v2, when the manager edits slots

[ui]
aspect_correct = true
kb_mode = "positional"        # positional | symbolic
```

Decisions:
- **Versioned DTO, not raw `MachineConfig`.** A `MachineDefV1` struct with its
  own serde derives converts *to* `MachineConfig` + mount calls. Internal
  refactors then never silently change the file format. Unknown TOML keys are
  warned about, not fatal (forward compat); `schema > known` is fatal.
- **Identity = slug** (file stem, derived from name at creation, uniquified).
  Rename via the manager = file rename + artifact-dir rename. No UUIDs — this
  is a per-user local registry, and slugs keep the config dir greppable.
- **Media by reference, never embedded** (same rule as snapshots). Absolute
  paths stored as given; relative paths mean "this machine's artifact dir",
  making machine-owned blank disks portable. The definition does NOT store
  content hashes — that's snapshot-consistency business, not definition
  business.
- **Runtime status (running/paused/stopped) is never persisted.** It's a
  property of the live process. Later, "stopped but has a saved state" is
  derived from the artifact dir containing a snapshot file.
- Validation on load = `MachineConfig::validate` + file-existence checks on
  media; failures mark the list row with an error badge instead of refusing
  to start the manager.

## Manager wiring (implementation order)

1. `machine_def.rs`: `MachineDefV1` + `load_all() -> Vec<(slug, Result<MachineDef>)>`
   + `save(slug, &def)`. Deps: `serde` (already transitive) + `toml` in
   coco-egui. Unit tests round-trip a full and a minimal TOML, unknown-key
   warning, schema-too-new error, slug uniquification.
2. Manager list rows from `load_all()` (name, variant/RAM subtitle, status
   dot — all stopped for now). Selection state in `ManagerApp`.
3. "New…" toolbar button drives the existing `new_vm.rs` dialog, but the
   Create action writes a definition file and inserts a row (does NOT boot).
4. Selected row → right pane becomes the detail/edit form (photo stays for
   empty selection, per the mock); Save writes the TOML.
5. Launch = build `MachineConfig` from the def, mount media, run — the same
   code path `main()`'s CLI branch uses today (refactored to be callable from
   both).
6. **Live row thumbnails** (user, 2026-07-16: the mock's per-row image is a
   miniature *live* view of that VM's screen):
   - *Running/paused VM*: draw the VM's existing framebuffer
     `TextureHandle` in the row at thumbnail size — the texture is already
     uploaded every field for the VM window, and one egui Context serves all
     viewports, so this costs one extra quad, zero extra uploads. Paused VMs
     naturally freeze on their last frame.
   - *Stopped machine*: on stop/app-exit, write `thumbnail.png` (last
     framebuffer, downscaled) into the machine's artifact dir and show that;
     placeholder art (model badge) when none exists yet.
   - Keep the row image widget aspect-correct against variable framebuffer
     sizes (GIME modes change dimensions mid-session).

## DECIDED (user, 2026-07-16): in-process, one native window per running VM

Target scenario: manager window + a launched CoCo 3 + a newly created,
launched CoCo 2 = **three OS windows, two VMs emulating concurrently**.

- Each running VM is an **immediate viewport** driven from
  `ManagerApp::update` — the proven paper-window pattern. All VM state stays
  on the main thread; each viewport's child ctx delivers that window's own
  keyboard/mouse input, so focus routing comes from egui for free.
- `CocoApp` stops being *the* `eframe::App` and becomes a per-VM window
  struct (working name `VmWindow`) owned by the manager:
  `Vec<MachineEntry { slug, def, vm: Option<VmWindow> }>`. Its `update(ctx)`
  body is reused nearly verbatim inside the viewport closure (menu bar,
  toolbar, status bar and all); `running` maps to the list's paused state.
- Per-VM `field_debt` pacing already lives in the VM struct; N VMs pace
  independently inside one repaint loop. Audio: one `cpal` stream per VM
  (already per-`AudioOutput`). gilrs is process-wide: route gamepad events to
  the focused VM only.
- Lifetime rule (accepted trade-off of in-process): the manager window is the
  root viewport — closing it closes every VM (flush dirty disks exactly like
  today's `on_exit`, confirm if any VM is running). A VM window's close
  button stops that VM (flush + drop), it does not hide it.
- Direct-boot CLI (`coco --cart …`) keeps working: it builds a manager whose
  list has one transient entry pre-launched, or (simpler, v1) bypasses the
  manager entirely as today. Revisit only if the two paths drift.
- Process-per-VM (VirtualBox model) is rejected for now: IPC for
  status/pause/stop and duplicated asset loading buy isolation we don't need
  yet. Re-open only if in-process audio/timing across VMs proves flaky.

## Snapshot compatibility contract (user requirement, 2026-07-16)

**Requirement: a snapshot written today must load in every future version.**
This AMENDS one decision in `plan-save-states.md` (its "Format: bincode (or
postcard)" line) — everything else there stands.

- **Self-describing payload, not positional.** bincode/postcard encode
  fields by position: adding one field breaks every old file unless each
  struct is hand-versioned. Use **CBOR** (`ciborium`) instead: field names
  travel with the data, so serde's evolution tools work — new fields load
  from old files via `#[serde(default)]`, removed fields are ignored,
  renames keep `#[serde(alias = "old_name")]`. RAM/framebuffer blobs stay
  compact via `serde_bytes` (CBOR has a native bytes type; this is why not
  JSON — 2MB of RAM as base64 is silly). Whole payload gzipped with the
  already-present `flate2`.
- **Container**: `magic "CCSTATE" | container_version u8 | schema u32 |
  gzip(CBOR payload)`. `schema` is the *machine-tree* schema; bump ONLY on a
  semantic break that serde evolution can't express. Loader dispatch:
  `schema == current` → plain load; `schema < current` with a registered
  migration → migrate; otherwise a precise error naming both versions.
- **Evolution rules** (enforced in review, documented next to the types):
  1. never remove or rename a serialized field without `alias`/migration;
  2. every added field carries `#[serde(default = ...)]` whose default
     reproduces the *old* behaviour;
  3. never change the meaning or units of an existing field — add a new
     field and migrate;
  4. enum variants may be added, never repurposed.
- **Golden-fixture gate** (what actually guarantees "always works"): every
  time `schema` bumps or a release is cut, commit a real snapshot fixture
  (small RAM machine, mid-BASIC-program) under `crates/coco-core/tests/
  fixtures/snapshots/`. A test loads *every* committed fixture and runs the
  trace-continuation check. Old snapshots then can't silently rot — CI says
  so. (Fixtures contain no ROM/disk bytes by construction, so committing
  them is licensing-safe.)
- The machine-definition TOML follows the same philosophy at lower stakes:
  `schema` integer, unknown keys warn, missing keys default.

## Relationship to plan-save-states.md

- The Cart-enum refactor (its task 1) is still the blocker for snapshots and
  is untouched by this plan — definitions don't serialize `Machine` at all.
- Its format decision is amended by the compatibility contract above
  (CBOR + gzip replaces bincode; the debug-dump-as-JSON idea stays).
- A definition's `[media]` table is the source for `Snapshot.MediaRefs`
  path+hash entries when snapshots land.
- Snapshot files live in the machine's artifact dir, giving the manager
  per-machine state slots for free — and the golden-fixture gate slots into
  its task 5 test list.
