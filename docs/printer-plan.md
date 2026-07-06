# Printer Emulation Plan — Tandy DMP-105 on the Bit-Banger Port

Goal: Virtual ][-grade printer emulation. The CoCo bit-bangs printer bytes out
the 4-pin serial port; we decode that stream cycle-accurately, interpret it as
a Tandy DMP-105 (architected as "DMP family" so the DMP-130 superset and an
Epson FX dialect can slot in later), and render onto continuous fanfold paper
drawn with the period-correct tractor-feed strips — sprocket holes down both
edges — with PNG/PDF export and a plain text-capture mode.

Agent/model legend (use Fable only where judgment or visual taste is the
bottleneck):

| Executor | Model | Use for |
|---|---|---|
| `hw-verify` | Sonnet | All hardware/protocol fact-finding before code |
| `coco-impl` | Sonnet | All routine implementation with a verified spec |
| `quick-check` | Haiku | Build/test/clippy sweeps after each task |
| `idiomatic-rust` | Sonnet | Review pass after each phase lands |
| `trace-debug` | Opus | Contingency only: serial-timing bugs that resist diagnosis |
| Fable (main session) | Fable | Spec-writing from verification reports, acceptance testing, paper-visual design sign-off |

## Phase 0 — Verification (before any code)

- [x] **V1: Bit-banger port facts** — `hw-verify` → `docs/bitbanger-spec.md`
  Which PIA1 line is serial TX and which input is the printer BUSY/CD
  handshake; $FF20-$FF23 involvement; Color BASIC `PRINT #-2:` framing
  (start/stop bits, data bits, parity), default 600 baud, the baud POKE
  (address 150) value table; whether BASIC waits on the handshake line.
  Sources: CoCo 3 Service Manual + Color BASIC Unravelled (both in `./docs`),
  MAME `coco` bit-banger implementation. Deliverable: cited register/timing
  spec for the decoder.
- [x] **V2: DMP-105 protocol** — `hw-verify` → `docs/dmp105-protocol.md`
  Control/escape code set (CR/LF/FF semantics, pitches, underline, elongate),
  bit-image graphics mode entry/format, line spacing commands, printable
  width (80 col @ 10 cpi), buffer/handshake behavior, serial settings the
  printer accepts. Sources: DMP-105 owner's manual (web), Tandy printer
  reference; flag every code we can't confirm. Deliverable: the interpreter
  spec, marked verified vs inferred.
- [ ] **V3 (later, with Phase 3): DMP-130 extensions + Epson FX-80 core set** —
  `hw-verify`, deferred until Phase 3 starts.

Fable writes the implementation specs from V1/V2 findings (same pattern as
VHD/MPI/composite).

## Phase 1 — Serial decode + text capture (headless value first)

- [x] **T1: `bitbanger.rs` in coco-core** — `coco-impl`
  Cycle-timed sampler on the verified PIA1 TX line (fed from the machine's
  per-instruction tick like the cassette/FDC), async frame decoder
  (configurable baud/framing from V1), BUSY handshake line driven back into
  the PIA input so BASIC paces itself. Emits a byte stream to a pluggable
  sink. Unit tests with synthetic edge timings, incl. framing-error cases.
- [x] **T2: Text-capture sink + frontend plumbing** — `coco-impl`
  "Print to text file" mode: File menu (Start/Stop Capture, path picker),
  `--print-capture <path>` CLI. Headless test: boot real ROM, `LLIST` a
  program, assert the captured text (fdc.rs boot-test pattern).
- [x] **T3: NitrOS-9 `/p` end-to-end** — `coco-impl` (found: NitrOS-9's driver
  compensates for the speed poke, holds true 600 baud = 2972 cycles/bit;
  Color BASIC's doesn't — see bitbanger-spec.md)
  EOU boot test printing via `/p` (e.g. `dir >/p`), assert capture. If the
  decode garbles under OS-9's interrupt load and the cause isn't obvious,
  escalate to `trace-debug` (Opus) rather than guessing.
- [x] **Q1: sweep** — `quick-check` (370 tests green, clippy clean); review —
  `idiomatic-rust` (applied: FileSink BufWriter, PB0 constant derivation,
  `PrinterSink::write_byte` rename; declined: `cart_error` field rename —
  the UI treats it as a single error banner by design).

## Phase 2 — DMP-105 interpreter + virtual paper

- [x] **T4: DMP-105 state machine in coco-core** — `coco-impl` (done:
  printer.rs paper model + dmp105.rs interpreter + 9x7 font [artistic
  approximation]; open: European $A0-$BF glyphs TODO, graphics-LF step-unit
  contradiction deferred to V3 — see dmp105-protocol.md §5)
  Escape parser from the V2 spec; text pipeline: current pitch/style → dot
  columns from a 9(or 7, per V2)-pin dot font → line raster; CR/LF/FF and
  line-spacing handling; page model (66 lines @ 6 lpi on 11" fanfold).
  Output = abstract "paper" raster pages (dots, not pixels), so rendering
  style is a frontend concern. Golden tests: byte streams → expected dot
  rasters.
- [x] **T5: Fanfold paper window in coco-egui** — `coco-impl` for the window/
  scroll/export plumbing; **Fable for the visual spec + sign-off** (this is
  the Virtual ][ moment and the user's explicit ask) — DONE: paper_render.rs
  (pure rasterizer) + paper_view.rs (window, page-texture cache, auto-follow);
  screenshots pixel-verified (hole pitch/phase) and signed off 2026-07-06:
  - Continuous fanfold sheet, scrollable, fills in live as the head prints.
  - **Tractor-feed strips on BOTH edges: the detachable perforated margins
    with the sprocket/pin-feed holes** (round holes at the standard 1/2"
    pitch down each strip, dotted perforation line separating strip from
    printable area) — the defining look of period printer paper.
  - Horizontal page perforation lines every 11"; subtle paper tint; dot-
    matrix impressions rendered as discrete slightly-bled dots, not vector
    text. Optional: faint green-bar banding toggle.
  - Fable reviews a rendered screenshot before this task closes.
- [x] **T6: Export** — `coco-impl` (done: `paper_export.rs` — PNG via `image`
  [promoted to a regular dependency], hand-rolled minimal PDF [one page per
  fanfold page, `FlateDecode`-compressed `DeviceRGB` image XObjects via
  `flate2`, already resolved transitively so no new crate] with two menu
  items for the 9.5×11-with-strips vs. 8.5×11-trimmed variants; Tear Off
  wired up in `paper_view.rs` behind a confirm/cancel dialog)
  Save paper as PNG (per page and full roll) and PDF (one page per fanfold
  page, strips optionally cropped). Tear-off (clear) action, with confirm.
- [x] **Q2: sweep** — `quick-check` (417 tests green, clippy clean); review —
  `idiomatic-rust`. Applied: CRITICAL `1C 1C 1C` unbounded-recursion fix
  (repeat expands via mode dispatchers, never re-enters `feed`), saturating
  head-position arithmetic + 8" print-zone mark clamp, blank-roll tear-off
  count/disable, dirty-invalidation widened by the dot-bleed pad. Declined
  (perf/cosmetic lows): extent() O(rows) dot-count sum, blank-fill loop,
  rgb-helper DRY. Reviewer verified the hand-rolled PDF xref bookkeeping
  correct.

## Phase 3 — Graphics + family extensions (appetite-driven)

- [ ] **T7: DMP-105 bit-image graphics** — `coco-impl` (test with a CoCo
  screen-dump utility from the EOU disk).
- [ ] **T8: DMP-130 superset** — `hw-verify` (V3) then `coco-impl`.
- [ ] **T9: Epson FX-80 dialect behind the same sink** — `coco-impl`, optional.
- [ ] **T10: CGP-115 plotter** — stretch goal, separate mini-plan if wanted.

## Acceptance

1. `LLIST` from Disk BASIC produces correct text both in capture mode and on
   the virtual paper. 2. EOU `dir >/p` prints under NitrOS-9. 3. The paper
   window shows fanfold stock with sprocket-hole tractor strips on both
   edges and page perforations; PNG/PDF export matches. 4. A screen-dump
   utility prints a recognizable graphics image (Phase 3). 5. Workspace
   tests/clippy stay green; every hardware claim in code cites V1/V2.

## Risks / notes

- Bit-banger timing under IRQ load is the likely trap (BASIC bit-bangs with
  interrupts masked; OS-9 may not) — hence the trace-debug contingency.
- DMP-105 manual coverage of bit-image mode may be thin; V2 must say what's
  verified vs inferred, and T7 only implements the verified part.
- Serde/save-state debt grows with each new device; unchanged policy (defer).
