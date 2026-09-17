# CoCo 3 Emulator — Architecture Design

This is design-only (no code on disk yet). It goes top-down: the workspace, the
load-bearing abstractions, then each subsystem, then a build order. Rust-specific
concerns (the borrow checker fights you'll hit) are called out inline because they
shape the design more than they would in C.

---

## 1. Workspace layout

Split the pure emulator from the UI. The core must be headless so it can be
unit-tested and run CPU conformance suites without a window.

```
cocovm/                 (workspace root)
├─ crates/
│  ├─ mc6809/            6809E CPU core. No deps. Generic over a Bus trait.
│  ├─ coco-core/         The machine: bus, GIME, MMU, PIAs, timing. Headless.
│  └─ coco-egui/         eframe frontend: video texture, input, audio, debugger.
└─ Cargo.toml            [workspace]
```

Why three crates, not one:

- **`mc6809` standalone** — keeping the CPU dependency-free and bus-generic means
  it can be tested against a flat 64K array with zero machine baggage. (Note:
  unlike the 6502/68000/Z80, the 6809 has **no** TomHarte/SingleStepTests-style
  per-instruction JSON suite — see §5 for the validation strategy that replaces it.)
- **`coco-core` headless** — lets you boot the real ROM to the BASIC prompt in an
  integration test and assert on the framebuffer, no GPU needed.
- **`coco-egui`** — the "Virtual ][ niceness" lives here and changes fastest;
  isolating it keeps recompiles cheap.

Drop `test01/` (it has served its purpose proving eframe builds) — fold it into
`coco-egui`.

---

## 2. The two load-bearing abstractions

Everything hinges on (a) how the CPU touches the world, and (b) how you satisfy
the borrow checker while the CPU mutates devices that also need to read RAM.

### 2a. The `Bus` trait

```rust
pub trait Bus {
    fn read(&mut self, addr: u16) -> u8;   // &mut: reads have side effects
    fn write(&mut self, addr: u16, val: u8);
}
```

`read` takes `&mut self` deliberately — reading a PIA data register clears its
interrupt flag, reading a GIME IRQ-status register clears pending bits.
Pretending reads are pure will bite you. The CPU is generic:
`fn step(&mut self, bus: &mut impl Bus) -> u32` returning cycles consumed. Tests
use a `FlatBus([u8; 65536])`; the machine uses `SystemBus`. Monomorphized, so
zero cost.

### 2b. The borrow-checker strategy (the part people get wrong)

The CPU mutably borrows the bus for a whole instruction. The GIME needs to mutate
itself (timer, counters) *and* read RAM during scanout. If `Machine` owns `cpu`
and everything else flat, you can't write `self.cpu.step(&mut self)`.

The clean fix: **CPU is one field, the bus is everything else.**

```rust
pub struct Machine {
    cpu: Mc6809,
    bus: SystemBus,        // disjoint from cpu → borrow checker is happy
}

pub struct SystemBus {
    ram: Box<[u8]>,           // size chosen at construction: 128K / 512K / 2048K
    rom: Box<[u8]>,           // Super Extended Color BASIC, 32K
    gime: Gime,               // MMU table, video regs, timer, irq state
    pia0: Mc6821,
    pia1: Mc6821,
    cart: Cartridge,          // trait object: FDC, ROM pack, Multi-Pak
    // keyboard matrix, joystick axes, audio sink…
}

impl Machine {
    fn step(&mut self) -> u32 { self.cpu.step(&mut self.bus) }  // two disjoint fields
}
```

For scanout, the GIME reads RAM that lives *next to* it in `SystemBus`.
Destructure to get disjoint borrows:

```rust
let SystemBus { gime, ram, .. } = &mut self.bus;
gime.render_scanline(line, ram, &mut framebuffer);
```

This pattern (split a struct's fields by destructuring) is how you avoid
`Rc<RefCell<…>>` everywhere. **Recommendation: no `Rc`/`RefCell` in the core.**
They're a code smell here and they cost you `serde` save-states later.

---

## 3. Address decoding (`impl Bus for SystemBus`)

The GIME MMU sits between CPU addresses and physical RAM, but the top page is
fixed I/O. Decode order:

```
read/write(addr):
  if addr >= 0xFF00 && io_enabled:   // I/O page is not MMU-translated
      decode device (below)
  else:
      phys = gime.translate(addr)    // 8K-block MMU, or ROM, or default map
      ram/rom[phys]
```

I/O page map (`$FF00–$FFFF`):

| Range         | Device                                                         |
|---------------|----------------------------------------------------------------|
| `$FF00–$FF03` | PIA0 (keyboard, joystick comparator)                           |
| `$FF20–$FF23` | PIA1 (DAC sound, cassette, VDG-legacy mode bits)               |
| `$FF40–$FF5F` | Cartridge / FDC control                                        |
| `$FF7F`       | Multi-Pak Interface select register (SCS/CTS slot, when an MPI is inserted) |
| `$FF80–$FF86` | VHD (virtual hard disk, NitrOS-9 `emudsk`)                     |
| `$FF90–$FF9F` | GIME control (INIT0/1, IRQ/FIRQ enable, timer, video, border…) |
| `$FFA0–$FFAF` | MMU task registers (8 blocks × 2 tasks, INIT1 selects task)    |
| `$FFB0–$FFBF` | 16 palette registers                                           |
| `$FFC0–$FFDF` | SAM-compatibility registers (legacy video/memory bits)         |
| `$FFE0–$FFFF` | ROM / vectors when mapped                                      |

The MMU: each task register holds a physical block number; `phys = (block << 13)
| (addr & 0x1FFF)`. The GIME's MMU addresses **up to 2 MB** — 512K was only Tandy's
shipped maximum, not a chip limit. The valid block range scales with installed RAM:

| RAM    | 8K blocks | Valid bank range | Block-number bits | Notes                          |
|--------|-----------|------------------|-------------------|--------------------------------|
| 128K   | 16        | high blocks ¹    | (see ¹)           | RAM sits at the *top* of space |
| 512K   | 64        | `0x00–0x3F`      | 6                 | Tandy's shipped maximum        |
| 1024K  | 128       | `0x00–0x7F`      | 7                 | confirmed real config          |
| 2048K  | 256       | `0x00–0xFF`      | 8 (full register) | confirmed real config ²        |

Two real hardware behaviours the emulator must model — both verified against the
[Sock GIME reference](https://www.6809.org.uk/twilight/sock/gime.html) /
[cococommunity reference](https://www.cococommunity.net/socks-gime-register-reference/)
and corroborated by an owner of a 2 MB machine:

- **Writes use the full 8 bits** (banks `0–255` → 2 MB). **Reads return only the
  low 6 bits reliably** — on most machines the upper 2 bits read back as *bus
  bleedover*, not the stored value. Some memory upgrades fix the readback; most
  don't, so software can't rely on those 2 bits. Model: store 8 bits on write;
  on read return `stored & 0x3F | (bus_garbage & 0xC0)` unless configured as a
  "fixed-readback" machine. Sizing is therefore *not* a simple `block_mask & ` on
  every access — it's a write-width / read-width asymmetry.
- ¹ **Smaller machines map RAM into the *high* blocks**, not `0..N`. A 128K machine
  doesn't use banks `0x00–0x0F`; its RAM lives at the top of the block space
  (default mapping puts usable RAM where the ROM/BASIC bring-up expects it). The
  exact 128K valid range is **not yet pinned** here — verify against the Super
  Extended BASIC Unravelled docs and a reference emulator before coding it. Do not
  assume a low-bit mask.
- ² 2 MB on a stock GIME is real (owner-confirmed). 8 MB exists too via further
  banking (e.g. CoCoZilla) but is out of scope.

`MemorySize` thus carries both the byte count and the valid-bank predicate; keep
the read/write asymmetry in the MMU model from day one.

When the MMU is disabled (INIT0 bit), use the fixed power-on map. ROM mapping
(INIT0 ROM bits) overlays the top 16K/32K — handle that *before* the MMU
translation for the affected range.

---

## 4. Timing model — scanline-driven

Two viable models; the recommendation is **scanline-stepped** as the driver
because it maps cleanly to both the GIME's sync interrupts and the host's 60 fps.

```
run_field():
  for line in 0..lines_per_field:           // NTSC 262 @ 60Hz, PAL 312 @ 50Hz
      run CPU for ~CYCLES_PER_LINE cycles    (≈ 57 @ 0.895 MHz; ~same per-line both standards)
      gime.tick_timer(by cycles, line)       // 12-bit countdown
      if line in active area: gime.render_scanline(line, ram, fb)
      gime.hsync()    // may raise HBORD interrupt
  gime.vsync()        // may raise VBORD interrupt; present framebuffer
```

**NTSC vs PAL** is a `VideoStandard` enum chosen at construction, collapsing to a
small constants table — don't scatter `if pal` checks through the code:

| Standard | Lines/field | Field rate | Active lines    | Selected by         |
|----------|-------------|------------|-----------------|---------------------|
| NTSC     | 262         | ~60 Hz     | 192 / 200 / 225 | default             |
| PAL      | 312         | ~50 Hz     | 192 / 200 / 225 | GIME 50/60Hz bit ¹  |

¹ The GIME has a 50/60 Hz video-mode bit (`$FF98`), but it only retimes the
display within the host standard — the *machine* is physically NTSC or PAL (master
crystal). Model the crystal as the construction-time `VideoStandard`; treat the
GIME bit as a mode flag on top of it. The horizontal rate (~15.7 kHz) is nearly
identical for both, so `CYCLES_PER_LINE` barely moves; the difference is almost
entirely lines-per-field and thus field rate, which drives the host pacing period.

- **CPU speed switch**: the "high-speed poke" (`$FFD7`/`$FFD9` SAM bits) doubles
  to ~1.79 MHz — that's just a different `CYCLES_PER_LINE`.
- **GIME timer**: 12-bit (counts 0–4095, per Sock's GIME reference), clocked from
  one of two sources selected by INIT1 bit 5 (TINS). On underflow it reloads and
  raises a timer interrupt (IRQ and/or FIRQ per the enable regs). Decrement it by
  elapsed cycles inside the line loop.
  - **Caution — sources disagree on the fast clock.** Sock's reference gives the
    two periods as **279.365 ns (≈3.58 MHz, the NTSC colour clock) fast** and
    **63.695 µs (≈15.7 kHz, the horizontal line rate) slow**. The cococommunity
    register reference instead lists **70 ns (≈14.3 MHz dot clock)** for the fast
    source. This must be pinned empirically against a reference emulator (XRoar /
    MAME `gime.cpp`) during implementation — do **not** hard-code a number on
    authority alone. The slow = horizontal-line-rate value is consistent across
    sources and is the safer one to rely on first.
- **Interrupt aggregation**: the GIME ORs its sources (timer, HBORD, VBORD,
  keyboard, serial, cartridge) into the 6809's IRQ and FIRQ lines, gated by
  `$FF92`/`$FF93` enables. The PIAs *also* drive IRQ/FIRQ (legacy path). So the
  CPU sees `irq = gime.irq() | pia0.irq() | …`. Reading the GIME status reg
  (`$FF92`) returns and clears pending bits — model that as a side-effecting read.
  - **Implementation note (2026-07, verified against the real ROM):** the stock
    CoCo 3 sits at the BASIC prompt with **INIT0 IEN=0/FEN=0**, i.e. the *legacy
    PIA path*, not the GIME interrupt block. The 60 Hz field sync is wired to
    **PIA0 CB1** (`$FF03`) and horizontal sync to **PIA0 CA1** (`$FF01`); both
    drive IRQ. That field-sync IRQ is what breaks the ROM out of its `BRA *` idle
    loop. `coco-core` implements this path first (`SystemBus::{hsync,vsync}` +
    `Machine::run_field`); GIME-sourced IRQ ORs in later.

**Host pacing**: don't trust egui's repaint cadence for emulation timing. Use a
real-time accumulator — accumulate wall-clock delta, run whole emulated fields
while `accumulator >= field_period`, present the latest framebuffer. Start
single-threaded inside `update()`; if audio underruns, *then* move the core to its
own thread with a triple-buffered framebuffer and an audio ring. Don't build the
thread split up front.

---

## 5. The 6809 core (biggest chunk)

Registers: `A B` (=`D`), `X Y U S PC DP CC` where CC = `E F H I N Z V C`. Two
extended opcode pages (prefix `$10`, `$11`).

The hard parts, in order of pain:

1. **Indexed addressing** — one postbyte encodes ~a dozen sub-modes: constant
   offsets (5/8/16-bit), accumulator offsets (A/B/D), auto inc/dec by 1 or 2,
   PC-relative (8/16), extended-indirect, and indirect variants of most. Build one
   `fn ea_indexed(&mut self, bus) -> (u16 addr, u32 extra_cycles)` and get it
   bulletproof — a large fraction of all instructions route through it.
2. **`PSH`/`PUL`/`TFR`/`EXG`** — register-mask and register-pair encodings.
3. **Interrupts** — NMI/IRQ/FIRQ/SWI/SWI2/SWI3, with FIRQ stacking only CC+PC
   (the `E` flag records which). `CWAI` and `SYNC` halt states.

Structure: dispatch via `match opcode`, with addressing-mode helpers and a
per-opcode cycle table. Don't try to be cycle-*exact* mid-instruction at first;
instruction-granular cycle counts are enough to get the ROM booting and sync
interrupts roughly right. Tighten later only if a game needs it.

**Test it for real.** There is **no** per-instruction JSON suite for the 6809 (the
TomHarte/SingleStepTests project covers the 68000, 65x02, Z80, 8088, etc. — but no
6800-family 8-bit). Hand-verifying a 6809 by eye is a trap, so the validation
strategy is:

1. **Trace-diff against a reference emulator.** Boot the real ROM and compare a
   per-instruction CPU trace (PC, opcode, registers, cycles) against
   [XRoar](https://github.com/sixxie/xroar) or MAME's `m6809` debugger trace from
   the same reset vector. The first divergent line is the bug.
2. **Self-checking exerciser ROMs** — run a 6809 assembly test program that
   executes each instruction, captures the condition codes, and compares against
   expected results, flagging mismatches. A concrete, verified-on-real-silicon one:
   [`flexemu`'s `cputest.txt`](https://github.com/aladur/flexemu/blob/master/src/tools/cputest.txt)
   by W. Schwotzer (tested on an SGS-Thomson EF6809P) — covers the arithmetic/logic
   instructions, `TFR`/`EXG`, and the full range of addressing modes (indexed
   pre/post inc-dec, indirect, PC-relative, extended). Assemble it and run it
   headless in `mc6809`; a clean pass is strong evidence the core is correct.
3. **Hand-written unit tests for the corners** — indexed-mode postbytes, `TFR`/
   `EXG` register encodings, FIRQ-vs-IRQ stacking, `CWAI`/`SYNC`.

(Design for the Hitachi 6309 later as a feature-flagged superset, but don't pay for
it now — ask before adding that scope.)

---

## 6. GIME video

This is where "start with the GIME" actually means a lot of surface area. Split it:

- **Native CoCo 3 modes** (do these first): graphics with 1/2/4 bits-per-pixel at
  160/256/320/640 width; text at 32/40/64/80 columns with attributes (8 colors
  fg/bg, blink, underline). 16 palette registers, 6-bit values interpreted as RGB
  or composite per the monitor-type bit.
  - **DONE (2026-07, `coco-core::gime_video`):** both native paths render from
    *physical* RAM at the $FF9D/$FF9E vertical offset ×8 (MMU bypassed), honour
    $FF9A border, $FF9B video bank (>512K), $FF9C smooth scroll, and $FF9F
    HVEN/X-offset with the 256-byte seam wrap. Text uses the GIME internal font
    (MAME `hires_font`, see `NOTICE.md`), attribute fg regs 8–15 / bg regs 0–7,
    blink (provisional field-count phase until the GIME timer lands), and the
    LPR-dependent underline line. Geometry verified against SEB Unravelled II
    and MAME `gime.cpp` (text HRES ignores bit 1; LPR is 1/1/2/8/9/10/11).
    Composite palette interpretation and per-scanline mode changes (Option B
    canonical raster) are still TODO.
- **Legacy VDG modes** (defer — large compat surface): the CoCo 1/2
  semigraphics/VDG modes selected through the SAM-compat and PIA mode bits. Needed
  to run old software, but not to boot CoCo 3 BASIC. Flag this as explicit deferred
  scope.
  - **Correction (2026-07, verified against the real ROM):** the power-on BASIC
    prompt is drawn in the **VDG-compatible 32×16 alphanumeric text mode** (COCO
    bit set), *not* a GIME native text mode — native 40/80-col text only appears
    with `WIDTH 40/80`. So the first visible-prompt milestone required the VDG
    text path ahead of "native first". `coco-core::video` implements it: 32×16
    from logical `$0400`, MC6847 glyphs. **Colours are data-driven from the GIME
    palette registers** the ROM programmed (not hardcoded): text bg = palette reg
    12, fg = reg 13, converted via `GIME::rgb_color` (6-bit `RGBrgb`, ×0x55, per
    MAME `gime.cpp`); legacy text border is black. **Bit 6 of each screen byte is
    inverse video** (swaps fg/bg); the stock BASIC screen stores every character
    with bit 6 set, so the prompt is **black glyphs on green** (`#00FF00`) with a
    black border — verified against a MAME screenshot. CSS orange set, semigraphics
    (bit 7), and native GIME text are TODO. FONT LICENSING: `src/font6847.rs` is
    MAME's `vdg_t1_fontdata8x12` (BSD-3-Clause, Nathan Woods) — see `NOTICE.md`.

Render to an RGBA `framebuffer: Vec<u8>` sized to the max active area (border
included). Per-scanline write into it; at VSYNC upload as an `egui::ColorImage` →
`TextureHandle`, drawn in the `CentralPanel` integer-scaled to the right aspect
ratio. Geometry to get right: active area vs. border (`$FF9A` border color), and
the vertical/horizontal offset+scroll registers (`$FF9C–$FF9F`) that set where in
physical RAM the raster reads from.

---

## 7. I/O

- **PIAs (MC6821 ×2)** — one reusable `Mc6821` type: ports A/B each with data reg,
  DDR, control reg (CRA/CRB) including the DDR-access bit and the CA1/CA2/CB1/CB2
  line/interrupt logic. PIA0 = keyboard + joystick comparator; PIA1 = 6-bit DAC
  sound, cassette, and the legacy VDG mode bits. Reading a data register clears the
  corresponding interrupt flag — that's why `Bus::read` is `&mut`.
- **Keyboard** — the CoCo matrix is scanned by driving column strobes on PIA0 PB
  and reading rows on PIA0 PA. Host side: translate `egui::Event::Key` into a
  matrix bitfield; the PIA read returns the live matrix state. (Map host layout →
  CoCo positions; this needs a thought-through key table.)
- **Joystick** — successive-approximation via comparator: software ramps the DAC,
  the pot voltage vs. DAC appears as a bit in PIA0 PA. Emulate by storing analog
  axis values and computing the comparator bit from the current DAC level.
- **Cartridge/FDC** — make `Cartridge` a trait so a WD1773 floppy controller, plain
  ROM packs, and the Multi-Pak slot all plug in. Defer the FDC; design the seam now.
- **Audio** — accumulate the 6-bit DAC by cycle, downsample to 48 kHz, feed `cpal`.
  Defer until video+CPU work; just leave the sink interface.

---

## 8. Frontend & the "Virtual ][" niceness

egui is genuinely good for this — the debugger is where it shines:

- **Video panel**: the texture, integer-scaled, correct aspect.
- **Controls**: load ROM/disk/cart, reset, pause, speed toggle (0.89/1.79/turbo).
- **Debugger** (the premium feel): live CPU register view, disassembler, memory hex
  viewer, breakpoints, single-step, **GIME/MMU register inspector**, palette
  swatches. These are cheap in egui and make the project feel like a real tool
  early.

---

## 9. Save states — decide now, not later

Make every core struct `#[derive(Serialize, Deserialize)]` from the start. A
snapshot is just `RAM + all device state`. It's nearly free if you avoid
`Rc`/`RefCell`/raw pointers (see §2b) and miserable to retrofit. This is a big QoL
win for an emulator and a strong reason to keep the core a plain owned tree.

---

## 10. Recommended build order

Each milestone is independently verifiable — that's the point.

1. **`mc6809` + flat bus + JSON tests passing.** No machine yet. Pure, fast
   feedback.
2. **`SystemBus` skeleton**: RAM + ROM + MMU translation + I/O stubs. Boot the real
   Super Extended Color BASIC ROM; step the CPU; confirm it runs into the I/O page
   (reads keyboard, writes video RAM) without crashing.
3. **GIME text mode + scanline loop + sync interrupts.** Now you *see* the BASIC
   prompt in egui. This is the first "it's alive" moment.
4. **PIA0 keyboard.** Type into BASIC.
5. **GIME native graphics modes + palette.** Run graphics demos.
6. **Debugger panels.** Cash in on the headless core.
7. **Audio (DAC), then FDC/disk, then legacy VDG modes.** The long tail.

The GIME doesn't produce anything visible until step 3, and only after the CPU
(step 1) and bus (step 2) exist — which is why starting *at* the GIME is starting
in the middle. The GIME is still the *centerpiece*; it's just not the *entry point*.

---

## Key decisions (recommendations — push back where you disagree)

- **Bus-generic CPU, no `Rc`/`RefCell` in core, split-field borrow pattern** (§2).
- **Scanline-driven timing, instruction-granular cycle counts** to start (§4–5).
- **Native GIME modes first, legacy VDG deferred** (§6) — an explicit scope cut.
- **serde-able core from day one** (§9).
- **6309 and Multi-Pak designed-for but not built** — don't add that scope without
  asking.

### Settled decisions

- **Both NTSC and PAL in the core.** Chosen at construction via a `VideoStandard`
  enum that feeds a constants table (lines-per-field, field rate, pacing period).
  The frontend does not offer PAL: the CoCo 3 PAL ROM isn't wired up and the PAL
  field-sync edges are unverified, so only a hand-edited machine definition can
  select it. See §4.
- **User-selectable memory: 128K / 512K / 2048K** (1024K trivially follows). Single
  `Box<[u8]>` sized at construction. The MMU models the real **write-8-bits /
  read-low-6-bits asymmetry** rather than a single mask, and smaller machines map
  RAM into the high blocks. 2 MB on a stock GIME is real hardware. See §2b and §3.

Both become fields of a `MachineConfig` passed to `Machine::new` — the frontend
exposes memory (not the video standard) in the machine form, and both are part of
the serialized save-state header so a snapshot restores into a matching machine.
