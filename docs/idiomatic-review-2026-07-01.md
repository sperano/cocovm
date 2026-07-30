# Idiomatic Rust review — cocovm (2026-07-01)

Produced by an automated review pass over the whole workspace (not a diff review).
Clippy was clean on `--all-targets` at the time of review.

## 1. Overall verdict

This is genuinely idiomatic, well-above-average Rust for an emulator. The
architecture is the right shape: a dependency-light `Bus` trait as the
CPU/machine seam, disjoint-field borrows instead of `Rc<RefCell>`, namespaced
bitmask constant modules (`cc`, `init0`, `vmode`, `vres`, `cr`), hardware tables
as `const` arrays, and clean per-instruction TDD suites. The findings below are
refinements, not rescues — mostly a few needless allocations, one fully-redundant
`Default` impl, and some magic bit literals that violate the project's own
"named constants" convention.

## 2. Top findings (ordered by importance)

### F1. `GIME`'s hand-written `Default` is entirely redundant — derive it
`crates/coco-core/src/gime.rs:190-216`. Every field defaults to `0`, `false`, or
a zero-filled array (`[[0; 8]; 2]`, `[0; 16]` both implement `Default`). The
27-line manual impl duplicates what `#[derive(Default)]` produces and is a
maintenance hazard: add a field and the manual impl silently drifts. Replace the
whole block with `#[derive(Default)]` on the struct (it already derives
`Debug, Clone, Serialize, Deserialize`; `new()` calling `Self::default()` keeps
working). Contrast with `PIAPort` (`pia.rs:41`), whose manual `Default` is
*justified* because `input` starts at `0xFF` — that one should stay.

### F2. Per-field heap allocations in the graphics render paths
`crates/coco-core/src/video.rs:184` `vdg_palette_indices` returns `Vec<usize>`
for 2–4 elements, and the sole caller (`lib.rs:231`) immediately does
`.iter().map(...).collect::<Vec<[u8;4]>>()` — two heap allocations on every VDG
graphics field (~60/s). `render_coco_graphics` also allocates
`data: vec![0u8; ...]` per field (`lib.rs:237`). The palette indices are a
compile-time-bounded set; return a small fixed array or borrow a
`&'static [usize]` table, and fold the color resolution into a stack
`[[u8;4]; 4]`. This is a hot path — worth removing the allocations even though
correctness is unaffected.

### F3. Magic bit literals where the project convention is named constants
The PSH/PUL register-mask bits are raw hex: `mask & 0x80`, `0x40`, `0x20`, ...
`0x01` (`crates/mc6809/src/lib.rs:1085-1092` and `1108-1120`), and the TFR/EXG
register-selector codes `0x0..0xB` (`reg_read`/`reg_write`, `lib.rs:1006-1035`;
`tfr_value`, `1041-1053`). These are exactly the kind of hardware-encoding
constants that belong in a `psh` / `regsel` module next to `cc` and `postbyte`.
(Raw *opcode* hex in the `step` match is standard emulator practice and fine to
leave; this is specifically about the reusable field encodings.)

### F4. The DRY of the giant `step` match is carried by copy-paste, not abstraction
`crates/mc6809/src/lib.rs:225-636`. The dispatch is a ~400-line match where
addressing-mode variants repeat the same skeleton dozens of times, e.g.
`let (ea, ic) = self.ea_indexed(bus); let v = bus.read_u16(ea); self.x = v;
self.set_nz16(v); 5 + ic`. It reads clearly and is a legitimate emulator style,
but the LD/ST/CMP families across imm/dir/idx/ext are prime candidates for a
small declarative macro (`ld16!(reg, cycles)`) or an addressing-mode helper
returning `(value, extra_cycles)`. Not required, but it's the single biggest
maintainability lever in the codebase and would shrink the match substantially.
Relatedly, the catch-all `_ => 2` (lines 294, 314, 635) silently NOPs
unimplemented opcodes — during bring-up a `debug_assert!`/trace hook would
surface decode gaps instead of hiding them.

### F5. Repeated flag-toggle idiom lacks the obvious shared helper
`set_carry` and `set_overflow` (`lib.rs:819-833`) each expand the
`if on { cc |= m } else { cc &= !m }` pattern, and that same pattern is
open-coded in `set_z16` (`lib.rs:1184`). A single
`fn set_flag(&mut self, mask: u8, on: bool)` would collapse all three and read
better at call sites. Minor DRY, but this pattern recurs.

### F6. `ctx.input(|i| (i.events.clone(), i.modifiers))` clones the event vector every frame
`crates/coco-egui/src/main.rs:150`. The clone sidesteps egui's input-lock
borrow, but it copies the whole `Vec<Event>` each repaint. You can do the event
scanning *inside* the `input` closure, or clone only what you branch on. Low
impact (small vector, 60 Hz), noted for completeness.

## 3. What the codebase does well

- **Architecture / API design**: the `Bus` trait with `&mut self` reads
  (side-effecting registers) and default `read_u16`/`write_u16` is exactly
  right; `Machine` splitting `cpu` and `bus` into disjoint fields to get two
  `&mut` borrows without interior mutability is the idiomatic 6809 solution and
  is documented as such.
- **Constants**: hardware register layouts as namespaced `pub mod` constant
  blocks (`cc`, `postbyte`, `init0`, `init1`, `vmode`, `vres`, `hoff`, `cr`)
  and lookup tables as `const [_; N]` (`LPF_LINES`, `LPR_LINES`, `TEXT_COLS`,
  `GFX_BPP`) — clean, greppable, and matches the project convention.
- **Match ergonomics**: nibble-dispatched `branch_taken` (`lib.rs:973`) and
  `rmw_apply` (`lib.rs:930`), and the tuple-match in `blit_semigraphics4`
  (`video.rs:103`) are textbook.
- **Naming**: getters correctly drop `get_` (`d()`, `data()`, `irq()`),
  `const fn` used where possible (`config.rs`),
  `#[allow(clippy::upper_case_acronyms)]` to honor the chip-naming rule without
  fighting the linter.
- **Iterators**: `chunks_exact_mut` for border fills, `iter_mut().zip()` in
  `resolve_colors`, `.get().copied().unwrap_or(...)` for bounds-safe fetches —
  allocation-free and idiomatic.
- **Tests**: per-instruction-group integration suites (`alu.rs`, `branches.rs`,
  `indexed.rs`, `stack.rs`, ...) with a shared `Sys` harness carrying a
  documented `#[allow(dead_code)]`; unit tests co-located under `#[cfg(test)]`.
  Good separation of headless `coco-core` from the `eframe` frontend so the
  core is testable without a window.
