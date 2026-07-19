# Plan: 3D world frontend ("the desk")

A diegetic 3D frontend: the VM is presented as a physical 1986 desk — CoCo 3
case, cartridge slot, FD-502, disks, and a CRT monitor — rendered in OpenGL
inside the existing egui app. Configuration actions become physical gestures
(seat a pak in the slot, flip the power switch, plug the monitor cable) that
call the exact same core APIs the menus call today. The emulator's video output
is texture-mapped onto the CRT's curved glass with a shader stack. Virtual]['s
2.5D aesthetic taken to real 3D, for one lovingly-detailed machine.

Experimental — lives in worktree `worktree-3d-experiments` until the incremental
path proves out.

## Current state (verified in-repo)

- `coco-egui` is on `eframe = "0.33"` with the **default glow (OpenGL)
  backend** — egui's `PaintCallback` escape hatch hands us the raw `glow`
  context inside any panel, so a 3D viewport coexists with all existing egui
  UI (menus, debugger, dialogs). No engine migration, no second window.
- Framebuffer upload already exists: `main.rs:1122` builds an
  `egui::ColorImage::from_rgba_unmultiplied` and `load_texture("coco-fb", …,
  NEAREST)` per frame (manager thumbnails do the same). The 3D path reuses this
  texture — same bytes, mapped onto a mesh instead of an `Image` widget.
- The core API surface the physical gestures need already exists and is
  frontend-agnostic (`coco-core/src/lib.rs`): `reset()` (:164),
  `power_cycle()` (:176), `insert_cartridge()` (:196), `eject_cartridge()`
  (:202); MPI slot insert at `cart.rs:303`; disk/VHD/cassette attach likewise.
- `MachineConfig.monitor: MonitorType` (config.rs:168) already selects RGB vs
  composite decode — the doc comment literally says "which monitor cable is
  plugged in". The 3D world makes this physically true (see decision 4).
- coco-core stays headless and untouched. The 3D world is a *view*, exactly
  like the manager's live thumbnails.

## Design decisions

### 1. Rendering: glow `PaintCallback` inside egui, custom mini-renderer
- One central egui panel owns a `PaintCallback`; inside it, our own GL code:
  a forward renderer, one directional + one ambient light, glTF-loaded meshes,
  ~30 draw calls. No bevy/three-d dependency — the scene is two dozen static
  props, not a game world. (If scope ever explodes, the clean upgrade is
  eframe's wgpu backend + the same callback mechanism — revisit then, not now.)
- Existing egui UI stays fully functional and overlays the viewport. The 3D
  desk is *optional per VM window* — a view mode toggled alongside the normal
  flat view, never a replacement.

### 2. Scene & interaction: tiny purpose-built scene graph + canned animations
- Flat `Vec<Prop>` scene: mesh, transform, AABB, `PropKind` enum (Case,
  PowerSwitch, CartSlot, Cartridge(id), DiskDrive, Disk(id), Monitor(kind),
  Cable(kind), …). No ECS.
- Mouse picking: ray from camera through cursor, ray–AABB first, ray–triangle
  refine. Hover → emissive highlight; click/drag → canned animation
  (cart slides in over ~300 ms). **Animation completion fires the core call**
  (`insert_cartridge`, `power_cycle`, …) so emulator state changes exactly
  once, at a well-defined instant — the physical layer is a veneer over the
  same operations the menus use.
- Camera: orbit around the desk, clamped; double-click monitor → dolly to
  fullscreen framing (and back). Keyboard input keeps going to the emulated
  keyboard as in flat view.

### 3. CRT shader stack (the payoff)
Composable fragment-shader passes on the screen mesh, each independently
toggleable: barrel distortion matched to tube curvature → shadow-mask /
aperture-grille pattern → scanline profile → phosphor persistence (previous-
frame blend) → bloom lighting the bezel → faint room reflection on the glass.
NEAREST-sampled source texture stays the ground truth underneath.

### 4. Monitor choice = decode path, physically
Which monitor sits on the desk *and which cable runs to it* sets
`MonitorType`: RGB cable to the CM-8 → RGB palette; RF/composite cable to a
TV → the existing MAME-derived composite tables, and artifact-color tricks
appear. Same GIME state, different glass. This is the correct causal model of
the hardware and makes an existing subtle feature discoverable. Swapping the
cable while running is legal (it is on real hardware) and just flips the
config field.
- **hw-verify findings (task 5, done 2026-07-19)**: CM-8 (Cat. 26-3215) is
  the CoCo 3's analog RGB monitor — 10-pin flat cable to a header on the
  case **bottom** (routed through a molded groove), not a rear-panel DIN;
  the CM-8 has no composite input. CM-5 is a Tandy 1000 TTL-RGBI/CGA
  monitor, incompatible — **dropped from the desk roster**. Composite roster:
  generic 13" composite CRT via the RCA jack, or TV via the RF switch box
  (no Tandy monitor was marketed specifically as the CoCo composite one).
  Thexder shipped on **cartridge** (Cat. 26-3072, 1987) — the demo script's
  "insert Thexder pak" is period-accurate.

### 5. Assets: git-ignored, first-run-download pattern
- Geometry: modeled in Blender from reference photos (user owns a real 2048K
  CoCo 3), exported glTF. Label art / scans are copyrighted → git-ignored
  `assets3d/` beside `roms/` and `docs/`; anything redistributable ships via
  the PR #6 per-user asset-dir + first-run download mechanism.
- The renderer must degrade gracefully: missing model → gray placeholder box
  with the prop name, so the code path is testable without the asset pack.

## Incremental path (each step demoable)

1. GL triangle in a `PaintCallback` panel next to the running flat view.
2. `coco-fb` texture on a flat quad in 3D space, orbit camera.
3. CRT shader stack on that quad.
4. Static monitor + case meshes (placeholder boxes acceptable).
5. Picking + power switch → `power_cycle()`.
6. Cartridge drag-to-slot → `insert_cartridge()`; eject.
7. Cable/monitor swap → `MonitorType`; FD-502 + disk insertion; rest of props.

## Task breakdown & model assignment

| # | Task | Model | Rationale |
|---|------|-------|-----------|
| 1 | **GL foundation**: PaintCallback viewport, glow renderer, camera, glTF load, placeholder-box fallback (steps 1–2) | **Opus** | Raw-GL-inside-egui lifetime/context handling is fiddly and load-bearing for everything after. |
| 2 | **CRT shader stack**: the 6 passes, per-pass toggles, params in a debug panel (step 3) | **Sonnet** | Well-specified shader work, iterated visually. |
| 3 | **Scene graph + picking + animations**: props, ray casting, highlight, canned insert/switch animations firing core calls (steps 4–6) | **Sonnet** | Contained; the core API is already there. |
| 4 | **Physical config mapping**: cable/monitor ↔ `MonitorType`, cart/disk/MPI mapping, view-mode toggle & persistence in machine defs | **Sonnet** | Mechanical once 3 lands. |
| 5 | **hw-verify pass**: CM-5 identity/compat, monitor roster, cart-vs-disk catalog facts for the props we model | **hw-verify agent** | Cited findings before any period claim is baked into assets/UI. |
| 6 | **Blender assets**: case, CM-8, FD-502, paks, disks, desk | **human (Eric)** | The long pole; code never blocks on it thanks to placeholder boxes. |

Sequence: 1 → 2 ∥ (3 → 4), 5 anytime before 4, 6 fully parallel.

## Acceptance

- Flat view and 3D view render the same VM simultaneously with no divergence
  (same texture), and egui_kittest's existing 15-test suite still passes —
  the 3D layer adds picking-level tests only.
- Power-switch click boots to the BASIC prompt on the 3D CRT; drag FD-502 in,
  insert an OS-9 disk, type DOS on the real keyboard → boot text visible on
  the curved glass.
- Swapping RF↔RGB cable flips artifact colors live (the existing composite
  palette, now physically motivated).
- App runs and all interactions work with zero assets present (placeholder
  boxes).

## Risks

- **Scope creep toward a game engine** — the flat `Vec<Prop>` + canned
  animations design is the fence; anything needing physics/ECS is out.
- **glow context misuse** (resources created on the wrong context / dropped
  late) — classic egui-PaintCallback pitfall; task 1 must nail the resource
  lifecycle pattern before shaders pile on.
- **Asset stall** — mitigated by placeholder-box degradation; the feature is
  mergeable ugly.
- **Perf** — a second render path per VM window; keep the 3D view per-window
  optional and skip rendering when the panel is hidden. Thumbnails already
  proved multi-VM texture upload is fine.
