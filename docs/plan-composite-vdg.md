# Plan: composite/TV-look rendering for the CoCo 1/2 VDG

Goal: make `--machine coco1/coco2` look like a CoCo on a TV set, not like
MAME's idealized RGB — richer green, near-black text, true-black border — and
give RG6 (PMODE 4 / SCREEN 1,1) its NTSC artifact colors, which many games
require. The user is the perceptual reference for the "TV look"; MAME is the
behavioral reference for artifacting.

Verified 2026-07-07 against: MAME `mc6847.cpp` (BSD-3-Clause, Nathan Woods —
same license basis as our existing `font6847.rs`/GIME-composite transcriptions,
see NOTICE.md), the CoCo 2 NTSC Service Manual §4.5/§4 video chain, Lomont's
CoCo hardware notes, XRoar's published manual (architecture only — XRoar is
GPL, no code was read), and this repo's CoCo 3 composite implementation on
`initial-dev` (52db939). Contradictions and dead ends are flagged inline; don't
re-derive.

## Verified reference

### Why the current output looks "off"

Our `VDG_FIXED_PALETTE` (video.rs) is a byte-identical transcription of MAME's
`s_palette` — and MAME renders the boot screen pixel-identically to us
(confirmed side-by-side 2026-07-07 with `mame coco`). MAME's palette models
the VDG's output as idealized RGB and applies **no composite/TV processing to
VDG text or CG modes at all** (verified: no monitor-type machinery exists in
`mc6847.cpp` beyond a black-and-white flag; the `coco` driver has none either).
What a real CoCo 1/2 showed went VDG → MC1372 RF modulator → TV NTSC decoder
→ CRT, which is where the remembered look comes from. So matching MAME was
correct for accuracy of the *chip*, and is exactly the thing to move beyond
for the *TV*.

### The real video chain (CoCo 2 Service Manual)

- SAM divides the 14.31818 MHz crystal by 4 → 3.579545 MHz color reference to
  both the VDG and the MC1372 (SM §4).
- VDG outputs Y (luma+sync), Phase A, Phase B, and a chroma bias reference
  into the MC1372, whose balanced modulators generate NTSC chroma. The only
  numeric phase fact the SM gives: Phase A lags Phase B by ~100° — a relative
  modulator offset, **not** per-color hue angles.
- SM §4.5 "Artifacting": RG6 "is designed to be a black and white mode" and
  normally produces no color burst; a dedicated one-shot circuit "forces the
  MC1372 to generate the burst signal in the high resolution mode, contrary to
  the original design," with burst phase chosen "to produce a desired set of
  hues."
- **SM prose trap**: §4.5 says the artifact circuit fires when "GM0 and CSS
  are high" — but GM0 is high in all four RG modes. MAME's gate and Lomont
  both restrict artifacting to RG6 (all of AG+GM2+GM1+GM0 set), and Lomont
  confirms it's a 256×192-mode phenomenon. Trust MAME+Lomont: **RG6 only**.
  (CSS selects the black/green vs black/buff base pair; it does not gate.)
- **Random phase on power-up is real hardware behavior**: the burst/dot phase
  relationship comes up in one of two states, which is why period games say
  "press RESET until the title screen is orange" (Lomont). MAME models this
  as a Standard/Reverse config toggle rather than actual randomness.

### MAME's artifacter (the RG6 mechanism to reproduce)

`mc6847.cpp` `artifacter` — a sliding-window pattern LUT, not a signal
simulation:

- Gate: `(mode & (AG|GM2|GM1|GM0)) == (AG|GM2|GM1|GM0)` and artifacting ≠ off.
- `update_colors(c0, c1)`: builds a 16-entry blend table — entry 0 = c0,
  15 = c1, entries 1–14 interpolate a hardcoded 14×3 table of blend factors
  (dk-purple, red `0.000,0.500,1.000`, blue `1.000,0.500,0.000`, etc.) via
  `mix_color(factor) = c0·(1−factor) + c1·factor` per channel.
- A 128-entry `artifact_correction` LUT maps a 6-bit neighborhood code (which
  of 6 horizontally adjacent pixels are c1) to one of those 16 colors; per
  line, each output pixel *pair* is overwritten by the LUT result for its
  ±2-pixel window.
- Standard/Reverse (default Standard) XORs the blend-table index low bit —
  the red/blue swap, i.e. the two power-on phase states.
- MAME exposes this as a per-machine config item, always available; it is the
  **only** TV-look concession MAME makes for the VDG.
- (Ignore `process_artifacts_pal` — a separate PAL cross-color mechanism for
  PAL VDG variants; NTSC CoCo irrelevant.)

License: `mc6847.cpp` is BSD-3-Clause — transcribing the two tables follows
the established NOTICE.md practice (font tables, GIME composite tables). Add
the attribution entry.

### XRoar's architecture (for the tier ladder, not for code)

XRoar's `-ccr` renderer ladder, per its manual: `none` (fixed palette — what
we and MAME do today) → `simple`/`5bit` (pattern LUTs for B&W artifacting —
MAME-equivalent idea) → `partial`/`simulated` (real per-pixel NTSC composite
encode→decode; produces the TV palette *and* artifact colors from the one
mechanism, no separate tables). This confirms the end-state architecture if we
ever want full fidelity, and that LUT-based artifacting is a respectable
intermediate tier, not a hack of our own invention.

### What we could NOT verify (blocks Tier 2's "derive, don't tune")

No MC6847 or MC1372 datasheet exists in `./docs` (the SM explicitly defers to
"the VDG data sheet" it doesn't include), and no numeric per-color luma/phase
table was found in any non-GPL source reachable during verification. The
GIME's `COMPOSITE_PALETTE` in `gime.rs` is hand-measured **from a GIME** — a
different encoder chip; it is not evidence for VDG colors. Consequence: a
first-principles "TV palette" cannot be derived today without guessing.
**Action item (user): drop the Motorola MC6847 datasheet (and ideally MC1372)
into `./docs`** — then hw-verify extracts the luma levels + phase angles and
Tier 2 becomes derivation instead of taste.

## Integration surface & merge dependency

- This worktree owns `ColorSource { GimePalette(&[..;16]), VdgFixed }` in
  `video.rs` — the seam a composite path plugs into.
- `initial-dev` (52db939, after this worktree's base) added
  `gime::MonitorType { Rgb, Composite }`, `MachineConfig.monitor`, `--monitor`
  CLI + View-menu radio, and refactored `video.rs` so CoCo-compatible renders
  always resolve through `GIME::color()` (fine on a CoCo 3 — the ROM programs
  the palette registers). It does **not** know about `ColorSource`.
- **Merge reconciliation (do this first)**: keep `ColorSource`. The CoCo 3 arm
  feeds it the `GIME::color()`-resolved 16-entry table (which automatically
  inherits CoCo 3 composite mode for legacy screens); the CoCo 1/2 arm keeps
  `VdgFixed`. Do NOT route CoCo 1/2 colors through `gime::MonitorType`'s
  tables — a real CoCo 1/2 has no GIME, and the GIME composite tables are the
  wrong chip's measurements.
- After the merge, `MachineConfig.monitor` (one `--monitor` knob) drives both
  machines: on CoCo 3 it selects the GIME decode as today; on CoCo 1/2 it
  selects `VdgFixed` (Rgb — today's idealized look) vs the new TV path
  (Composite). For a CoCo 1/2, "Composite" is arguably the *authentic* setting
  (the machine only had RF/composite out); keep Rgb the default until Tier 2
  exists, then consider flipping the CoCo 1/2 default to Composite.

## Tiers

### Tier 1 — RG6 artifact colors (unblocked now; biggest game-visible win)

Transcribe MAME's 14×3 blend-factor table + 128-entry correction LUT into
`video.rs` (or a new `vdg_artifact.rs`), cited + NOTICE.md entry. Apply in the
RG6 render path as a per-line post-pass over the decoded 2-color pixels,
gated on:

- mode is RG6 (AG+GM2:GM1:GM0 = 111 — reuse the existing decode's mode), and
- artifact setting ≠ Off.

Config: `ArtifactPhase { Off, Standard, Reverse }` on `MachineConfig`
(CoCo 1/2 only; default Standard — on real hardware artifacting is always
present on the TV), `--artifact` CLI + View-menu radio. Deterministic
Standard/Reverse instead of hardware's power-on randomness (tests need
determinism; a "randomize on reset" flavor toggle is a possible follow-up,
default off). CSS keeps selecting the base pair (black/green vs black/buff);
both pairs artifact, matching MAME.

Tests: alternating-bit line → red/blue runs that swap under Reverse; solid
0x00/0xFF lines stay pure c0/c1; RGB-monitor CoCo 3 output byte-identical
(artifacter never touches the GIME paths); a boot test running a PMODE 4
program asserting non-{black,buff} pixels appear.

Acceptance: side-by-side screenshot vs `mame coco` (default = Standard) on the
same PMODE 4 pattern.

### Tier 2 — derived "TV" palette for all VDG modes (blocked on datasheet)

Once the MC6847 datasheet is in `./docs`: extract per-color luma + chroma
phase, run a standard NTSC decode (offline or const-eval — a documented
script/test, not hand-tuned numbers) to produce `VDG_COMPOSITE_PALETTE`
(same 16-entry layout as `VDG_FIXED_PALETTE`). `ColorSource` gains the
monitor dimension: `VdgFixed` (Rgb) vs `VdgComposite` (Composite). Everything
downstream (text, SG4, all CG/RG modes, borders) picks it up through the
existing resolve path.

Sanity anchors: bright green should move toward the remembered deeper green;
alpha dark green toward near-black; "black" border toward true black. If the
derived values don't visibly land there, stop and re-verify rather than
tweaking to taste — then let the user eyeball A/B against memory (they're the
reference for "TV", MAME can't help here).

### Tier 3 — full composite encode/decode (defer; appetite-driven)

XRoar-`partial`-equivalent: encode each scanline to an NTSC luma+chroma
signal (from the same datasheet numbers) and decode per-pixel. Subsumes
Tiers 1–2 (artifacts and palette fall out of the physics), adds cross-color
bleed on text edges, costs real CPU per field. Implement independently (no
GPL code), only when someone wants demo-accurate fringing. The per-scanline
scanout work (Option B, plan-per-scanline-video.md on initial-dev) is a
natural prerequisite/companion.

### Tier 4 — CRT cosmetics (orthogonal, frontend-only)

Scanlines/phosphor/soft-glow as an egui shader/texture pass, independent of
color correctness and of machine variant. Not specced here; note it exists so
nobody bundles it into the color work.

## Order of work

1. Merge `initial-dev` ↔ this branch; do the `ColorSource`/`MonitorType`
   reconciliation above (its own reviewed commit).
2. Tier 1 artifacting (unblocked, self-contained).
3. User drops MC6847 (+MC1372) datasheet into `./docs` → hw-verify extracts
   the color table → Tier 2 palette.
4. Tiers 3–4 by appetite, later.
