# Plan: CoCo 1 / CoCo 2 machine support

Goal: boot real Color BASIC / Extended Color BASIC on emulated CoCo 1 and CoCo 2
machines (SAM MC6883 + VDG MC6847, no GIME), alongside the existing CoCo 3, with
a `--machine` selector in the frontend. ROMs are provided by the user (see
"ROM files" below).

Verified against: MAME `6883sam.cpp` / `mc6847.cpp` / `coco12.cpp` /
`coco12_m.cpp`, `docs/Color Computer 2 NTSC Service Manual (26-3026 & 26-3027)
(Tandy).pdf` (SAM register map pp. 8–10), Bob Russell's memory map, and Color
BASIC Unravelled. Contradictions found during verification are called out
inline — do not re-derive.

## What we already have

The CoCo 3 emulates the CoCo 1/2, so most of the hard parts exist and just need
to be reached without a GIME:

- **VDG rendering is done**: `video.rs` renders alphanumeric/SG4 text
  (`render_text`) and all CG/RG graphics modes (`decode_vdg_graphics`,
  `render_graphics`) from PIA1 $FF22 mode bits, including per-byte
  semigraphics/inverse selection (data bits 7/6). `font6847.rs` is the MC6847
  character generator. Framebuffer geometry (288×224) matches.
- **SAM strobes are modeled** — but inside the GIME (`gime.rs::write_sam`):
  V0–V2, F0–F6, R1, TY already work as a CoCo 3 compatibility overlay.
- **Everything peripheral is machine-neutral**: `mc6809` crate, `pia.rs`,
  `keyboard.rs` (same 7×8 matrix), `cassette.rs`, `joystick.rs`, `cart.rs` /
  `fdc.rs` / `wd1773.rs`, sound mux/DAC, joystick comparator.

What's missing: a machine-variant concept, a *primary* (not overlay) SAM memory
map, VDG-native fixed colors instead of GIME palette registers, VDG field-sync
timing, and per-variant ROM loading.

## Verified hardware reference

### SAM MC6883 registers ($FFC0–$FFDF)

No data lines to the SAM: writing **any** value to an even address clears the
bit, odd sets it (service manual p. 8; MAME `6883sam.h` `alter_sam_state`).

| Bit  | Clear / Set   | Meaning |
|------|---------------|---------|
| V0   | $FFC0 / $FFC1 | VDG-counter mode (with V1, V2) |
| V1   | $FFC2 / $FFC3 | |
| V2   | $FFC4 / $FFC5 | |
| F0–F6| $FFC6…$FFD3   | Display offset, ×512 bytes (BASIC sets F2 → $0400) |
| P1   | $FFD4 / $FFD5 | Page #1 — banks upper 32K RAM into $0000–$7FFF; only effective when TY=0 and 64K; unused by BASIC, "should be cleared" (service manual p. 8) |
| R0   | $FFD6 / $FFD7 | CPU rate (see "Speed poke") |
| R1   | $FFD8 / $FFD9 | CPU rate |
| M0   | $FFDA / $FFDB | Memory size (4K / 16K / 32K–64K) |
| M1   | $FFDC / $FFDD | |
| TY   | $FFDE / $FFDF | Map type: **0 = ROM map, 1 = all-RAM** |

**TY polarity trap**: Color BASIC Unravelled's appendix labels $FFDE
"ROM DISABLED" / $FFDF "ROM ENABLED" — that is **wrong**. MAME, Bob Russell,
and the CoCo 2 service manual (p. 8: "If this bit is set, the ROMs 'disappear'…
all 64K (less the top 256) locations are available for RAM") all agree:
$FFDF (set) = all-RAM.

### CoCo 1/2 memory map (TY=0)

| Range | Contents |
|---|---|
| $0000–$7FFF | RAM (4K/16K machines mirror/shrink per M0–M1; start with real sizes, no mirroring, and verify BASIC's RAM sizing) |
| $8000–$9FFF | Extended Color BASIC ROM (absent on non-ECB machines → open bus) |
| $A000–$BFFF | Color BASIC ROM |
| $C000–$FEFF | Cartridge (CTS) |
| $FF00–$FF1F | PIA0 · $FF20–$FF3F PIA1 · $FF40–$FF5F cart SCS (FDC) |
| $FF60–$FFBF | Open bus / cart passthrough — **no GIME registers decoded** |
| $FFC0–$FFDF | SAM strobes (write-only in effect; reads fall through) |
| $FFE0–$FFFF | Mirror of $BFE0–$BFFF (Color BASIC ROM top) |

- **Vector mirror width**: the service manual documents $FFF2–$FFFF → $BFF2–$BFFF;
  MAME's decode is the full 32 bytes $FFE0–$FFFF → $BFE0–$BFFF (`6883sam.cpp`
  read/write paths: `offset >= 0xffe0` selects ROM slot 1 with
  `offset & 0x1fff`). Follow MAME's 32-byte mirror; it's consistent with the
  manual ("top 256" of the map never becomes RAM).
- **TY=1 (all-RAM)**: requires M1 set (64K); RAM decode extends $0000–$FEFF.
  While TY=0, writes to $8000–$FEFF do **not** write through to the RAM
  underneath (MAME gates write-through on TY). So the classic ROM→RAM copy is:
  read ROM into a low-RAM buffer or flip TY around each byte — Disk BASIC's
  `POKE &HFFDF` shadow trick reads ROM with TY=0 and writes with TY=1.
- **P1** only matters when TY=0 ∧ 64K: it ORs $8000 into RAM addresses for CPU
  accesses in $0000–$7FFF. Implement (it's two lines), test lightly.

### VDG MC6847 + SAM video counter

Mode pins come from **PIA1 port B** (written at $FF22):

| PB | VDG pin |
|----|---------|
| PB7 | A/G (0 = alpha/semigraphics, 1 = graphics) |
| PB6 | GM2 |
| PB5 | GM1 |
| PB4 | GM0 **and** INT/EXT (shared line) |
| PB3 | CSS |
| PB2 | RAMSZ input (memory-size sense — see below) |
| PB1 | single-bit sound |
| PB0 | RS-232 RX (bit-banger) |

`AS` and `INV` are **not** register-driven: the VDG latches them from bits 7
and 6 of each fetched data byte (MAME `coco12_m.cpp` `pia1_pb_changed` /
per-byte `as_w`/`inv_w`). `video.rs::render_text` already does this per byte.

SAM V0–V2 do **not** select the display mode — they set the video counter's
x/y clock division and must merely be *consistent* with the PIA mode bits
(service manual p. 8: "the display mode control registers (V0–V2) and the PIA
controlling the VDG must all be set to the proper mode"). Division table
(MAME `6883sam.cpp`):

| V2:V1:V0 | matches | y-division (rows per RAM row) |
|---|---|---|
| 0 | alpha/SG4/SG6 | 12 |
| 1 | CG1 | 3 (x÷3) |
| 2 | RG1 | 3 |
| 3 | CG2 | 2 (x÷2) |
| 4 | RG2 | 2 |
| 5 | CG3 | 1 |
| 6 | CG6/RG3/RG6 | 1 |
| 7 | DMA (counter free-runs) | — |

`video.rs` already encodes the practical consequence of this
(`LEGACY_GFX_LINES_PER_ROW` keyed on V-bits), so the renderer needs no change
here — the *authoritative* mode remains PIA bits, with V-bits picking vertical
cadence, exactly as today.

**VDG colors are fixed** (green/black and orange/black text sets via CSS; fixed
CG/RG color sets). Today `video.rs` resolves colors through GIME palette
registers (`vdg_palette_indices` → palette regs 0–11, `TEXT_FG/BG_INDEX` =
regs 13/12) — correct for CoCo 3 because its ROM initializes those registers to
the VDG defaults, but a CoCo 1/2 has no palette registers. The renderers need a
color-source parameter: GIME-palette lookup (CoCo 3) vs hardwired VDG RGB
(CoCo 1/2).

### Timing / interrupts

- CPU clock: same 894,886 Hz (14.31818 MHz / 16) — existing `CPU_HZ` is right.
- HSYNC → PIA0 CA1, FS (field sync) → PIA0 CB1 — already wired in
  `bus.rs::hsync` / `fs_falling` / `fs_rising`.
- **FS edge lines differ**: plain MC6847 FS falls at scanline 216 (NTSC), not
  the GIME's 244 — `config.rs` already documents this in the
  `fs_falling_line()` comment. Make these per-variant.
- No GIME timer, no GIME IRQ/FIRQ sources. Only PIA interrupts exist. The
  existing wire-OR in `bus.rs::irq_asserted`/`firq_asserted` is already inert
  when GIME enables are clear.
- CART detect: cartridge Q-line → PIA1 CB1, same as today.

### Speed poke (R0/R1)

R1 set doubles the master rate but the VDG can't fetch fast enough — real
hardware shows garbage while R1 is set. R0 is the "address-dependent" speed-up
(ROM fast / RAM slow); **MAME does not model the address dependence** (its own
TODO) and just doubles the clock for either bit. Match MAME for now: either bit
set → double clock; add a `KNOWN GAP` comment. Do not attempt address-dependent
timing without a better source (Lomont flags it unresolved too).

### RAMSZ (PIA1 PB2)

BASIC senses the RAM configuration partly via PB2. Tie it per configured
memory size (MAME `coco12.cpp` does the same). Verify against Color BASIC's
sizing routine during Phase 6 boot tests; a wrong PB2 shows up as a wrong
"MEMORY SIZE" at boot.

### CoCo 1 vs CoCo 2 vs CoCo 2B

- **CoCo 1 and CoCo 2 are software-identical** — MAME uses one driver (`coco`)
  with BIOS variants; same SAM, same MC6847, same PIA wiring. We model them as
  one core behavior; the variant only picks default RAM size and ROM set.
- **CoCo 2B** (later CoCo 2) has the **MC6847T1**, which is real new behavior,
  not a font swap (MAME `mc6847.cpp` `is_mc6847t1` paths):
  - GM0 (with INV clear) becomes a **lowercase-select** in internal alpha mode;
    GM1 becomes a second inverse toggle.
  - **SG6 and "stripe" semigraphics are removed**.
  - INT/EXT is ignored for the SG4 decision.
  - Ships with Color BASIC 1.3.
  Defer T1 to a follow-up phase; it's contained entirely in the text renderer.

## ROM files

Expected in git-ignored `roms/` (MAME romset names; any of these versions work,
listed = preferred):

| File | Content | Size | Machine |
|---|---|---|---|
| `bas12.rom` | Color BASIC 1.2 | 8K → $A000 | coco1/coco2 |
| `extbas11.rom` | Extended Color BASIC 1.1 | 8K → $8000 | coco1/coco2 |
| `bas13.rom` | Color BASIC 1.3 | 8K → $A000 | coco2b (later) |

Also accepted: `bas10`/`bas11`, `extbas10`. Loader should take Color BASIC
alone (16K non-ECB machine: $8000–$9FFF open bus) or both. Internally compose
them into the existing flat ROM image (32K window, $8000-based): extbas at
offset 0, bas at offset $2000, unused regions = open-bus filler.

## Implementation phases

Each phase compiles and keeps the CoCo 3 test suite green.

### Phase 1 — `MachineVariant` in config, threaded to the bus

- `config.rs`: `MachineVariant { Coco1, Coco2, Coco3 }` (Coco2B later) on
  `MachineConfig`; extend `MemorySize` with `K4`/`K16`/`K32`/`K64`
  (CoCo 1/2) — validate variant/size combinations (CoCo 3 keeps
  128/512/2048).
- Per-variant `fs_falling_line()`/`fs_rising_line()` (216 vs 244 family).
- `Machine::new` / `SystemBus::new` take the variant. Default stays Coco3 —
  zero behavior change this phase.

### Phase 2 — standalone `sam.rs` (MC6883) as the primary memory map

New `Sam` struct owning the 16-bit SAM state + address decode. **Do not**
refactor the GIME to share it — the GIME's SAM-compat overlay (`write_sam`)
stays as-is; the small duplication buys zero risk to CoCo 3.

- `write_strobe(addr)` for $FFC0–$FFDF (even/odd bit-pair).
- `map(addr) -> SamTarget` where `SamTarget ∈ {Ram(phys), RomBas(off),
  RomExt(off), Cart(off), Io, OpenBus}` implementing the TY/M1/P1 rules and
  the $FFE0–$FFFF → $BFE0–$BFFF mirror.
- `display_base()` (F-bits × 512), `v_bits()`, `cpu_fast()` (R0|R1).
- Bus: branch on variant in `read`/`write`/`phys`/`io_read`/`io_write` —
  GIME path untouched; SAM path decodes only PIA0/PIA1/cart-SCS/SAM, with
  $FF60–$FFBF open bus. Keep the branch at the top of the four bus entry
  points rather than a trait object; two concrete paths are easier to keep
  cycle-honest.
- Open-bus reads: return the conventional $FF for now (constant named, one
  place) — revisit if a test program cares about floating-bus behavior.

Tests: SAM strobe unit tests (mirroring existing `sam_video.rs` style),
TY=0 write-no-through / TY=1 all-RAM, vector mirror reads, P1 banking.

### Phase 3 — VDG-native color source for the existing renderers

- Introduce a color-lookup enum/param for `video.rs`: `GimePalette` (current
  behavior) vs `VdgFixed` (hardwired RGB for the 8 VDG colors + black/buff,
  text green/orange sets via CSS).
- `Machine::video_mode()`: for SAM variants, skip INIT0 entirely — always the
  `CocoText`/`CocoGraphics` branches, mode from PIA1 $FF22 + SAM V-bits,
  display base from `sam.display_base()`, fetched **through `sam.map`** (RAM
  only — a display base pointing at ROM shows open-bus, don't crash).
- No `gime_video.rs` involvement.

Tests: golden-ish render tests per mode (text, SG4, CG1, RG2, RG6/PMODE4)
asserting exact fixed colors, mirroring `render.rs`/`render_graphics.rs`.

### Phase 4 — field loop and interrupts per variant

- `run_field`: skip GIME timer tick and GIME HBORD/EI0/EI1 raises for SAM
  variants (gate the calls in `bus.rs::hsync`/`fs_*` on variant); PIA0
  CA1/CB1 pulses stay.
- FS lines from the per-variant config (Phase 1).
- Speed poke: `sam.cpu_fast()` feeds the existing `cycles_per_field()`
  doubling, with the `KNOWN GAP` note on R0 address-dependence.

Tests: PIA sync-edge test at line 216 (clone of `pia_sync.rs`), 60 Hz IRQ
cadence drives BASIC's TIMER.

### Phase 5 — frontend

- `--machine coco1|coco2|coco3` (default coco3), per-variant default RAM size
  (coco1: 32K? pick 64K — it's what most surviving machines have — and allow
  override), per-variant ROM resolution (`roms/bas12.rom` +
  `roms/extbas11.rom`, error message listing accepted names).
- Framebuffer path unchanged (288×224 already exists); window title shows the
  machine.
- Cassette/printer/joystick UI works as-is; hide CoCo 3-only UI (palette
  debug, hi-res) for SAM variants if any is exposed.

### Phase 6 — boot acceptance tests

Mirroring `alive.rs`/`boot.rs` (skip-if-ROM-missing pattern):

- coco2 + bas12+extbas11 boots to `EXTENDED COLOR BASIC 1.1` copyright screen,
  cursor blinking, `PRINT 2+2` → `4`.
- `PMODE 4:SCREEN 1,1` renders; `POKE 65495`-family speed pokes don't crash.
- 64K test: `POKE &HFFDF` all-RAM flip + execution from low RAM.
- Cassette `CLOAD` smoke test on a `.cas` (infrastructure is machine-neutral).
- Disk: defer — FD-502 under CoCo 1/2 needs Disk BASIC ROM ($C000 cart) which
  the existing `RomPak` path should already serve; verify, don't build new.

### Deferred / follow-ups (explicitly not dropped)

- **CoCo 2B / MC6847T1** (lowercase, SG6 removal) — text-renderer-contained.
- **NTSC artifact colors for PMODE4** — many CoCo 1/2 games depend on them;
  reuse the composite-palette work from initial-dev when it lands.
- External-ROM (INT/EXT) character generator support.
- Address-dependent R0 speedup (blocked on a trustworthy source).
- 4K/16K RAM mirroring subtleties, if BASIC's sizing misbehaves with plain
  truncated RAM.

## Branch note

This worktree branches from committed HEAD (a43248b). `initial-dev` carries
uncommitted work touching `bus.rs`/`gime.rs`/`config.rs` (bit-banger, printer,
VHD, MPI, composite). Expect a merge when it lands; the Phase 2 decision to
leave the GIME untouched and add a parallel SAM path exists precisely to keep
that merge small.
