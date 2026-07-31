# Writing a CoCo Emulator — A One-Semester Course

**Reference codebase:** this repository (`cocovm`) — a Tandy Color Computer 1/2/3
emulator in Rust (~52k lines: `mc6809`, `coco-core`, `coco-egui`).

**Who this course is for.** Experienced programmers who know Rust well (but not
necessarily its advanced corners), know 6809 assembly reasonably well, and are
familiar with what a CoCo 3 *does* without knowing all the details of how the
GIME, VDG, SAM, and PIAs do it — and who have never written an emulator or done
modern graphics programming. Every chapter assumes exactly that background.

**How the course works.** One class per week, 16 weeks, six parts. Each week
has required reading (specific source files and tests — the tests *are* the
textbook exercises with answers), a lecture outline, and exercises. The
codebase is complete and working, so the mode of study is *archaeology, then
surgery*: read a subsystem, run its tests, break something on purpose, watch
which test catches it, then extend it.

**Standing tools.** `cargo test -p <crate>` per chapter;
`crates/coco-core/examples/` for visual demos (they write PPM files — no GPU
needed); [`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) is the architecture document the whole course hangs off.
Real ROMs in `./roms/` and reference PDFs in `./docs/` (git-ignored, and
present only on a machine whose owner has obtained them independently).

**Chapters written so far:**

1. [The Shape of the Machine](ch01-the-shape-of-the-machine.md)
2. [CPU core I: registers, flags, dispatch, simple addressing](ch02-registers-flags-dispatch.md)
3. [The Indexed Postbyte, Stacks, and the Disassembler](ch03-the-indexed-postbyte.md)
4. [CPU core III: interrupts, halt states, and how to test a CPU with no test suite](ch04-interrupts-and-how-to-test-a-cpu.md)
5. [The Bus: Memory Maps, the SAM, and the GIME MMU](ch05-the-bus.md)
6. [Time and the Scanline Loop](ch06-time-and-the-scanline-loop.md)
7. [How a Raster Works + the Legacy VDG Text Mode](ch07-raster-and-vdg-text.md)
8. [GIME Native Modes: Registers, Text Attributes, Graphics, Palette](ch08-gime-native-modes.md)
9. [Advanced Video: Composite, Mid-Frame Splits, and PMODEs](ch09-splits-composite-and-pmodes.md)
10. [The PIAs, the Keyboard Matrix, and the Joystick ADC-by-Comparator](ch10-pias-keyboard-joystick.md)
11. [Sound: from a 6-bit DAC to your speakers](ch11-sound.md)
12. [The Cassette: FSK Modems, 1980 Edition](ch12-cassette.md)
13. [Disks: the WD1773 State Machine, and Three Ways to Store Bytes](ch13-disks.md)
14. [Serial: Bit-Banging, a Real UART, a Printer — and the Cartridge System](ch14-serial-printers-carts.md)
15. [The egui Frontend: Pixels, Keys, and Real Time](ch15-the-egui-frontend.md)
16. [The Debugger and Save States: the Payoff of Every Earlier Decision](ch16-debugger-and-save-states.md)

Plus the self-study [Appendices A–D](appendices.md): emulating without complete
documentation, ROM licensing and provenance, deferred scope, and the tooling
lab bench.

---

## Part I — The CPU (weeks 1–4)

Everything in an emulator is downstream of one function:
`cpu.step(&mut bus) -> cycles`. We start where the design started.

### Week 1 — What an emulator is, and the two load-bearing abstractions

*Goal: understand the whole machine's shape before touching any chip.*

- What "emulation" means concretely: a `struct` full of registers, a loop, and
  a memory-access seam. Interpreters vs JIT (we interpret; why that's the
  right call at 0.895 MHz).
- Tour of the CoCo 3 as hardware: 6809E, SAM legacy, GIME (MMU + video +
  interrupts), two 6821 PIAs, the cartridge port. Block diagram from the
  Service Manual (`docs/`).
- **The `Bus` trait** ([`crates/mc6809/src/lib.rs:29`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L29)): why `read` takes
  `&mut self` (reading a PIA data register clears an interrupt flag — reads
  have side effects). Why the CPU is generic over the bus
  (monomorphization = zero cost, testable against a flat 64K array).
- **The borrow-checker strategy** ([DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §2b): `Machine { cpu, bus }` as
  two disjoint fields; destructuring `SystemBus` to get disjoint borrows
  during scanout. Why there is no `Rc<RefCell<…>>` in the machine's state tree, and
  what that buys later (save states, week 16).
- Workspace layout: why three crates ([DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §1).

**Reading:** [`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) (all of it — it's the course's map),
[`crates/mc6809/src/lib.rs:29-159`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L29-L159), [`crates/coco-core/src/machine.rs:61-135`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine.rs#L61-L135).
**Exercise:** write a toy `FlatBus` and a "CPU" that only executes `NOP` and
`JMP extended`; run a 3-instruction loop. (~50 lines; the point is feeling
the fetch-execute rhythm before the real thing.)

### Week 2 — CPU core I: registers, flags, dispatch, simple addressing

*Goal: read `MC6809::step()` and know where every opcode goes.*

- The register file ([`lib.rs:139`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L139)): A/B/D, X/Y/U/S, PC, DP, CC. The CC bit
  layout `E F H I N Z V C` ([`lib.rs:47`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L47)).
- Dispatch: there is **no opcode table** — the `match` in [`exec.rs:30`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L30) *is*
  the table, split into family functions (`exec_alu8`, `exec_logic8`,
  `exec_rmw`, …, in [`exec/exec_data.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec/exec_data.rs)). Trade-off vs classic table-driven
  6502 cores: transparency over compactness.
- Addressing modes: immediate, direct (`DP:byte` — why DP exists), extended
  ([`addressing.rs:7-28`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L7-L28)).
- Flag computation as shared primitives ([`alu.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs)): `add8`/`sub8` serve
  ADD/ADC/SUB/SBC/CMP; half-carry (bit 3) for DAA; signed overflow as
  "operands same sign, result differs". The RMW dispatcher `rmw_apply`
  ([`alu.rs:156`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs#L156)) keyed on the opcode's low nibble — and the TST exception
  (flags but no write-back).
- Cycle counting: per-instruction granularity, accumulated in `cpu.cycles`.
  Why instruction-level granularity is enough to boot BASIC ([DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5).

**Reading:** [`exec.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs), [`exec/exec_data.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec/exec_data.rs), [`alu.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs), tests [`loads.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/loads.rs),
[`alu.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/alu.rs), [`logic_rmw.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/logic_rmw.rs).
**Exercises:** (1) trace `ADCA #$7F` with C=1, A=$80 by hand, predict all five
flags, and verify against a test you write; (2) delete the `V` computation from
`asl` and find which existing test fails; (3) implement one currently-missing
illegal-opcode behavior of your choice as a no-op with correct byte count.

### Week 3 — CPU core II: the indexed postbyte, stacks, and the disassembler

*Goal: master the single hardest 200 lines in the CPU.*

- The indexed postbyte (`1 rr i mmmm`, [`lib.rs:76`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L76), decoder
  [`addressing.rs:58-171`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L58-L171)): 5-bit offsets, A/B/D accumulator offsets, auto
  inc/dec by 1/2, PC-relative, extended-indirect — and the indirect bit that
  adds a second memory fetch. A large fraction of all instructions route
  through `ea_indexed`; it must be bulletproof.
- PSH/PUL register masks and the push order (PC first, CC last —
  [`stack.rs:26`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/stack.rs#L26)); TFR/EXG nibble encodings and the 8↔16-bit size-mismatch
  rules ([`regs.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/regs.rs)).
- Page prefixes `$10`/`$11` as part of the opcode, not a modifier
  ([`exec.rs:125`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L125), [`exec.rs:176`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L176)).
- The disassembler ([`disasm.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm.rs), [`disasm/tables.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm/tables.rs), [`disasm/indexed.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm/indexed.rs)):
  table-driven, mirrors the executor byte-for-byte, renders illegal opcodes
  as `???` with correct length so a scrolling view never desyncs. This is
  the first piece of the debugger we'll meet again in week 16.

**Reading:** [`addressing.rs:58-171`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L58-L171), [`stack.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/stack.rs), [`regs.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/regs.rs), [`disasm/`](https://github.com/sperano/cocovm/tree/main/crates/mc6809/src/disasm);
tests [`indexed.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/indexed.rs), [`stack.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/stack.rs), [`disasm_indexed.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/disasm_indexed.rs).
**Exercises:** (1) hand-decode postbytes `$8B`, `$F4`, `$9F` into syntax and
cycle cost, then check against [`disasm/indexed.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm/indexed.rs); (2) write a test for
`LEAX ,--Y` asserting both the EA and Y's side effect; (3) explain why the
5-bit-offset form cannot be indirect (look at the bit layout).

### Week 4 — CPU core III: interrupts, halt states, and how to test a CPU with no test suite

*Goal: know the interrupt frames cold, plus the validation strategy.*

- IRQ vs FIRQ vs NMI: full 12-byte frame vs CC+PC, the E flag telling RTI
  which frame to unwind ([`lib.rs:193-254`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L193-L254)). SWI/SWI2/SWI3. Why NMI is ignored
  until S is first loaded (`nmi_armed`).
- SYNC and CWAI as CPU *states* (`State` enum, [`lib.rs:123`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L123)) — the CPU is a
  small state machine around the instruction loop, not just a loop.
- **The 6809 testing problem**: unlike the Z80/6502/68000, there is no
  TomHarte-style per-instruction JSON suite ([DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5). The three-legged
  strategy used here: (1) trace-diff against XRoar/MAME from the same reset
  vector ([`examples/trace.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/trace.rs), the trace ring in [`coco-core/src/debug.rs:186`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/debug.rs#L186));
  (2) self-checking exerciser ROMs run headless; (3) hand-written corner
  tests. First divergent trace line = the bug.

**Reading:** [`lib.rs:123-254`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L123-L254), tests [`interrupts.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/interrupts.rs); [DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5;
[`crates/coco-core/examples/trace.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/trace.rs).
**Exercises:** (1) draw both stack frames (IRQ and FIRQ) byte-by-byte with
addresses; (2) write a test where FIRQ arrives during SYNC with F masked,
and describe what happens; (3) produce a 50-instruction trace of the real
ROM from reset and annotate the first 10 lines against the SEB Unravelled
listing.

---

## Part II — The Machine (weeks 5–6)

### Week 5 — The bus: memory maps, the SAM, and the GIME MMU

*Goal: given any address on any CoCo, say what the CPU touches.*

- Decode order ([`coco-core/src/bus.rs:267`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L267)): hardwired vectors
  `$FFE0–$FFFF` first, then the I/O page `$FF00–$FFBF`, then the ROM window,
  then MMU-translated RAM. The full I/O page map ([DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §3) — memorize
  the big landmarks: `$FF00` PIA0, `$FF20` PIA1, `$FF40` cartridge,
  `$FF90` GIME, `$FFA0` MMU, `$FFB0` palette, `$FFC0` SAM strobes.
- **Two machines, two paths**: the CoCo 3 GIME path vs the CoCo 1/2 SAM path
  ([`bus/sam_path.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sam_path.rs)), branched once at entry. The SAM's write-only strobe
  registers (write to an even address clears a bit, odd sets it) — the
  course's first "weird hardware" moment.
- The GIME MMU ([`gime.rs:235`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L235)): 8K slots, two task sets, `phys = block<<13 |
  addr&0x1FFF`; the write-8-bits/read-6-bits asymmetry ([DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §3).
- ROM composition per variant (`config.rs`, `rom_db.rs` CRC validation);
  reset: `Machine::new` fetches `$FFFE` and lands in ROM.

**Reading:** [`bus.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs), [`bus/io.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs), [`sam.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/sam.rs), [`bus/sam_path.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sam_path.rs), [`config.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/config.rs);
tests [`bus_map.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bus_map.rs) (the single best teaching test file in the repo),
[`sam.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/sam.rs), [`boot.rs:27`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/boot.rs#L27).
**Exercises:** (1) compute the physical address of logical `$4321` given MMU
slot 2 = block `$3A`; (2) write a `bus_map.rs`-style test for the MC3
constant-page behavior; (3) explain why `$FFFE` must bypass the MMU even in
all-RAM mode (what would happen on reset otherwise?).

### Week 6 — Time: the scanline loop, sync pulses, and "it's alive"

*Goal: understand `run_field()` — the heartbeat every other chapter plugs into.*

- The scanline-driven model ([DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §4, [`machine/run.rs:20`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs#L20)): 262 lines
  (NTSC) per field, 56 CPU cycles per line ([DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md)'s ~57 sketch loses to
  two truncating divisions — week 6 derives it), `end_of_line()` fires hsync,
  renders the scanline, flushes audio, ticks the GIME timer. The
  double-speed poke = a different line budget ([`run.rs:185`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs#L185)).
- Interrupt plumbing ([`bus/sync.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sync.rs)): wired-OR of PIA and GIME sources into
  IRQ/FIRQ; the crucial historical detail that stock BASIC idles on the
  **PIA path** (field sync on PIA0 CB1), not the GIME interrupt block — the
  60 Hz IRQ is what breaks BASIC out of its `BRA *` idle loop.
- Interrupts are recognized at instruction boundaries only
  ([`run.rs:174`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs#L174)) — and why the FD-502 disk handshake forced that exact
  ordering (a preview of week 13).
- Lab: [`tests/coco1_boot.rs:93`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/coco1_boot.rs#L93) boots real Color BASIC headless, types
  `PRINT 2+2`, and reads " 4" off the framebuffer. Step through it.

**Reading:** [`machine/run.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs), [`bus/sync.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sync.rs), [`machine.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine.rs); tests
[`coco1_boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/coco1_boot.rs), [`pia_sync.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/pia_sync.rs), [`speed.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/speed.rs).
**Exercises:** (1) compute cycles per field for NTSC at both CPU speeds and
check against `config.rs`; (2) disable `fs_falling` and describe precisely
how the boot test fails; (3) instrument `run_field` to histogram
instructions-per-line during boot.

---

## Part III — Video: the GIME at last (weeks 7–9)

The GIME is the obvious chip to want to start with — here is why it arrives in
week 7, not week 1: nothing the GIME does is visible until a CPU (weeks 2–4)
executes ROM code over a bus (week 5) on a clock (week 6). Now it pays off all
at once.

### Week 7 — How a raster works + the legacy VDG text mode

*Goal: from "TV scans lines" to the green BASIC prompt, with zero GPU.*

- Raster fundamentals for the graphics-shy: fields, active area vs border,
  why everything in weeks 6–9 is per-scanline. The canonical 640×240 RGBA
  canvas ([`raster.rs:16`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/raster.rs#L16)) that every mode renders into; the frontend only
  scales it (that separation is what makes the core headless-testable).
- The surprise the codebase documents ([DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §6 correction): the CoCo 3
  BASIC prompt is drawn in **VDG-compatible 32×16 text mode**, not a GIME
  native mode. So legacy text came first here too.
- VDG text rendering ([`video/text.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs)): screen RAM at `$0400`, MC6847 glyphs
  ([`font6847.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font6847.rs)), bit 6 = inverse video (the stock screen sets it on every
  character — black-on-green), bit 7 = semigraphics-4 (2×2 blocks, color in
  bits 6–4). Fonts as data: MC6847 vs T1 vs GIME ([`font_gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font_gime.rs)) and the
  T1's sneaky true-lowercase rule.
- Where colors come from even in legacy mode: the GIME palette registers
  the ROM programmed (bg=reg 12, fg=reg 13), 6-bit RGB decoded ×0x55.

**Reading:** [`video.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video.rs), [`video/text.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs), [`font6847.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font6847.rs), [`raster.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/raster.rs),
[`machine/video_mode.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/video_mode.rs); tests [`render.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render.rs), [`render_coco12/`](https://github.com/sperano/cocovm/tree/main/crates/coco-core/tests/render_coco12).
**Exercises:** (1) hand-render the byte `$C1` ('A' with bit 6 set) into an
8×12 pixel grid; (2) write a test asserting the border is black while text
bg is green after boot; (3) add a fantasy font variant and boot with it.

### Week 8 — GIME native modes: registers, text attributes, graphics, palette

*Goal: read a `$FF90–$FF9F` register dump and describe the exact screen.*

- The register file as a video mode description: `$FF98` VMODE (BP graphics/
  text, LPR lines-per-row), `$FF99` VRES (LPF, HRES, CRES), `$FF9A` border,
  `$FF9D/9E` video base (×8, **physical** address — MMU bypassed!),
  `$FF9C` smooth scroll, `$FF9F` horizontal offset + the 256-byte seam wrap
  ([`gime.rs:42-160`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L42-L160), [`gime_video.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs)).
- Text with attributes: fg from palette regs 8–15, bg from 0–7, blink driven
  by the GIME timer, LPR-dependent underline row.
- Graphics: 1/2/4 bpp unpacking, MSB first, at 160/256/320/640 widths
  (`paint_graphics_row`, [`gime_video.rs:352`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L352)).
- The palette ([`gime/palette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs)): 16 registers, 6-bit `RGBrgb`, RGB decode;
  what `WIDTH 80` and `HSCREEN 2` actually write (test `gime_modes.rs`
  replays the real ROM's register sequence — read it side-by-side with the
  SEB Unravelled listing).

**Reading:** [`gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs), [`gime_video.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs), [`gime/palette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs); tests
[`render_gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_gime.rs), [`gime_modes.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/gime_modes.rs); run [`examples/gime_demo.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/gime_demo.rs) and
[`examples/palette_trace.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/palette_trace.rs).
**Exercises:** (1) given a register dump, compute bytes-per-row and total
screen memory for HSCREEN 4; (2) modify `gime_demo.rs` to render your name
in attribute text with blink; (3) why must the video base be physical, not
logical? What CoCo 3 trick does that enable? (Hint: video pages > 64K.)

### Week 9 — Advanced video: composite artifacts, mid-frame splits, PMODE graphics

*Goal: the effects that made demos possible — and the emulation policy
questions they force.*

- Composite vs RGB monitors as a *palette interpretation*, not a different
  renderer: the two hand-measured 64-entry tables, BPI hue rotation, MOCH
  grayscale ([`gime/palette.rs:19`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs#L19); test `composite.rs` — including a real
  NitrOS-9 grayscale regression).
- Mid-frame register changes ([`tests/scanline_split.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/scanline_split.rs) — the best-named
  test file in the repo): which registers are **live** (border, palette,
  mode, X-offset — take effect next scanline) vs **field-latched** (video
  base, COCO bit, scroll seed). `FieldScan` ([`gime_video.rs:124`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L124)) as the
  latching mechanism, mirroring MAME's `new_frame`. A timer-FIRQ border
  split generated by actual 6809 code running in ROM — raster tricks, the
  emulator-author's view.
- Legacy VDG *graphics* (PMODEs): GM bits from PIA1 `$FF22` × SAM V-bits
  cadence ([`video/graphics.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/graphics.rs), test `sam_video.rs`) — the CoCo 1/2 modes
  a generation of BASIC programmers POKEd at, finally explained.

**Reading:** [`gime/palette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs), [`video/graphics.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/graphics.rs); tests [`composite.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/composite.rs),
[`scanline_split.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/scanline_split.rs), [`sam_video.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/sam_video.rs).
**Exercises:** (1) predict the on-screen result of writing the border color
at line 100 vs writing the video base at line 100, then find the two tests
proving it; (2) identify which palette values NitrOS-9 uses for gray on
composite and why they're achromatic; (3) sketch how you'd add PAL 50 Hz
artifact handling — what breaks?

---

## Part IV — Input and sound (weeks 10–12)

### Week 10 — The PIAs, the keyboard matrix, and the joystick ADC-by-comparator

*Goal: understand the chip that mediates almost all CoCo I/O.*

- The 6821 (`pia.rs`) as a reusable component: data register vs DDR behind
  one address (the CR DDR-access bit), CA1/CB1 edge detection with latched
  flags, CA2/CB2 as outputs. **Reading the data register clears the
  interrupt flag** — this is the fact that forced `Bus::read(&mut self)` in
  week 1. Full circle.
- Keyboard: 7×8 matrix, ROM strobes columns on PIA0 PB (active-low), reads
  rows on PA ([`keyboard.rs:78`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/keyboard.rs#L78)). Host key → matrix position
  ([`coco-egui/src/keymap.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/keymap.rs)) and the positional-vs-symbolic mapping choice.
- Joystick: no ADC chip — the ROM ramps the 6-bit DAC and compares against
  the pot voltage, one bit on PIA0 PA7 (`joystick.rs`). Successive
  approximation in software, 1980-style. Fire buttons sit on keyboard rows
  and bypass the strobe (`button_rows()`).

**Reading:** [`pia.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/pia.rs), [`keyboard.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/keyboard.rs), [`joystick.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/joystick.rs); tests [`keyboard.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/keyboard.rs),
[`pia_sync.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/pia_sync.rs), [`joystick_bus.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/joystick_bus.rs).
**Exercises:** (1) trace the ROM's keyboard scan for the 'A' key press,
strobe pattern by strobe pattern; (2) write a test that a fire button reads
pressed even with no column strobed; (3) why does the DDR exist at all —
what would break if PIA registers were plain read/write bytes?

### Week 11 — Sound: from a 6-bit DAC to your speakers

*Goal: the whole audio path — the most "systems" chapter in the course.*

- The core side (`audio.rs`, [`machine/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/audio.rs)): CPU writes are recorded as
  **cycle-timestamped events**; once per scanline `flush_line_audio()`
  renders a 4× oversampled grid (~63 kHz). The analog mux (SEL bits from PIA
  CA2/CB2): DAC vs cassette vs cartridge sound. Why event-timestamping beats
  sampling the DAC register per-sample.
- The host side ([`coco-egui/src/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs)): cross-thread ring buffer
  (`Arc<Mutex<VecDeque>>` — the first concurrency in the course), DC
  blocker (`y[n]=x[n]-x[n-1]+0.995·y[n-1]`), Butterworth low-pass before
  decimation, linear-interpolation resampler, underrun fade instead of
  clicks. Each of these exists because of an audible artifact — the chapter
  walks through each one.
- The optional chips as a taxonomy of PSGs: Orchestra-90 (two dumb DACs),
  SN76489 (tone + LFSR noise, in the GMC cart), AY-3-8913 (adds envelopes,
  in the SSC) — `orch90.rs`, `sn76489.rs`, `ay8913.rs`.

**Reading:** [`coco-core/src/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/audio.rs), [`machine/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/audio.rs),
[`coco-egui/src/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs); tests [`sound.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/sound.rs), [`audio_grid.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/audio_grid.rs), [`orch90.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/orch90.rs).
**Exercises:** (1) compute the grid sample rate for NTSC from first
principles; (2) remove the DC blocker and describe what you'd hear and why;
(3) implement one AY envelope shape from the datasheet and test it against
[`ay8913/envelope.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/ay8913/envelope.rs).

### Week 12 — The cassette: FSK modems, 1980 edition

*Goal: a complete, self-contained signal-processing story — many students'
favorite chapter.*

- CSAVE as modulation: 0-bit = 1100 Hz, 1-bit = 2060 Hz square waves, LSB
  first; leader `$55` bytes, sync `$3C` (`cassette.rs`). CLOAD as
  demodulation: measure rising-edge periods against a threshold, hunt for
  leader/sync exactly the way the ROM does — the emulator re-implements the
  ROM's own decoder in reverse.
- The motor spin-up delay the ROM blindly assumes, and why the emulator must
  model tape *mechanics* (512K cycles of latency) for CLOAD to work.
- `.cas` (decoded bytes) vs `.wav` (audio) round-tripping
  (`cassette_wav.rs`); [`examples/cassette_calibrate.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/cassette_calibrate.rs).

**Reading:** [`cassette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cassette.rs), [`cassette_wav.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cassette_wav.rs); tests [`cassette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/cassette.rs),
[`coco2_boot/cassette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/coco2_boot/cassette.rs).
**Exercises:** (1) compute both FSK half-periods in CPU cycles and find the
constants in the source; (2) CSAVE a program in the emulator, export WAV,
load it in Audacity, and read the bits by eye; (3) break the demodulator
threshold by ±20% — which bit pattern fails first, and why?

---

## Part V — Storage and serial (weeks 13–14)

### Week 13 — Disks: the WD1773 state machine, and three ways to store bytes

*Goal: device protocol emulation in the large.*

- The WD1773 FDC (`wd1773.rs`) as a command state machine: Type I
  (seek/step), II (read/write sector), III (format), dispatched from
  `$FF48`. Byte-paced transfers (~32 µs DRQ intervals), and the **HALT/NMI
  handshake**: the FD-502 halts the CPU until each byte is ready, and NMI
  fires at sector end — this is why `step_cpu_unit()` checks HALT before
  polling interrupts (week 6's mystery resolved). Functional-not-cycle-exact
  as an explicit modeling decision.
- JVC image geometry, including sniffing OS-9's LSN0 to guess sidedness
  ([`fdc/jvc.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/jvc.rs)).
- Contrast with two simpler designs: VHD (`vhd.rs`) — pure register
  interface, no state machine, 24-bit LRN × 256-byte sectors; and DriveWire
  (`drivewire/`) — a checksummed multi-message serial RPC protocol with
  timeout recovery. Same job, three protocol philosophies.

**Reading:** [`wd1773.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773.rs), [`fdc/`](https://github.com/sperano/cocovm/tree/main/crates/coco-core/src/fdc), [`vhd.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/vhd.rs), [`drivewire/protocol.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire/protocol.rs); tests
[`fdc/`](https://github.com/sperano/cocovm/tree/main/crates/coco-core/tests/fdc), [`halt.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/halt.rs), [`vhd.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/vhd.rs), [`drivewire_bus.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/drivewire_bus.rs).
**Exercises:** (1) diagram the full read-sector sequence: command write →
DRQ/HALT per byte → NMI, with who-does-what between CPU, FDC, and bus;
(2) compute the byte offset of track 17/sector 3/side 0 in a 35-track JVC;
(3) implement a "disk activity" callback and count sectors read during a
DOS boot.

### Week 14 — Serial: bit-banging, a real UART, and a printer

*Goal: three rungs of the serial ladder, from GPIO to protocol stack.*

- The bitbanger (`bitbanger.rs`): the ROM bit-bangs PA1 with software
  timing; the emulator decodes it with an edge-triggered receiver sampling
  at mid-cell. Baud from `POKE 150,n` → cycle math.
- The ACIA 6551 (`acia6551.rs`): a real UART — status/command/control
  registers, crystal-divider baud math, DCD/DSR edge IRQs. Byte-level
  (not bit-level) modeling as another explicit fidelity decision.
- The DMP-105 printer (`dmp105.rs`, plus `coco-egui`'s paper view): a
  control-code interpreter over the bitbanger stream, rendering to fanfold
  paper with fixed-point units chosen so every documented pitch is an exact
  integer. Protocol → renderer layering.

**Reading:** [`bitbanger.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs), [`acia6551.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551.rs), [`dmp105.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/dmp105.rs), [`rs232.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/rs232.rs); tests
[`bitbanger.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bitbanger.rs), [`serial_test.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/serial_test.rs), [`dmp105_boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/dmp105_boot.rs).
**Exercises:** (1) derive the bit period for `POKE 150,18` and verify
against `DEFAULT_BIT_PERIOD`; (2) draw the 6551 IRQ sources and when each
fires; (3) add one DMP-105 control code (e.g. elongated mode) with a test.
*Elective add-on:* the cartridge system — MultiPak slot mux, banked ROM
paks, the CART FIRQ auto-start Q-burst ([`cart/`](https://github.com/sperano/cocovm/tree/main/crates/coco-core/src/cart), tests `mpi.rs`, `cart.rs`).

---

## Part VI — The frontend, the debugger, and time travel (weeks 15–16)

### Week 15 — The egui frontend: pixels, keys, and real time

*Goal: everything host-side, for the graphics-shy — there is less GPU here
than you might fear.*

- The whole per-frame story in one file ([`coco-egui/src/app/frame.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/frame.rs)):
  wall-clock delta → `field_debt` fractional accumulator → run N whole
  fields → upload the RGBA framebuffer as an egui texture → draw it
  letterboxed at 4:3. Guardrails: `MAX_FIELDS_PER_UPDATE`, dropping >250 ms
  gaps. Why you never trust the UI's repaint cadence for emulation timing
  ([DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §4).
- "Graphics programming" here is exactly one texture upload per frame —
  demystified. Integer scaling and aspect correction as the only math.
- Input routing ([`app/input.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/input.rs), `keymap.rs`): positional vs symbolic
  keymaps, the `TypeAhead` queue that types for you in tests.
- The VM manager (`manager/`): machine definitions as TOML, thumbnails,
  multi-window viewports — a study in frontend state *around* an emulator.
- UI testing with kittest ([`ui_tests/harness.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/ui_tests/harness.rs)): headless egui, queries by
  accessibility label. Tests that read like scripts.

**Reading:** [`app/frame.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/frame.rs), [`app/input.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/input.rs), [`keymap.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/keymap.rs), [`main.rs:91-168`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/main.rs#L91-L168),
[`manager/lifecycle.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/lifecycle.rs), [`ui_tests/harness.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/ui_tests/harness.rs).
**Exercises:** (1) explain `field_debt` and what goes wrong with naive
"one field per repaint" on a 120 Hz monitor; (2) add a 2× turbo menu item;
(3) write a kittest that boots a VM and asserts the window title.

### Week 16 — The debugger and save states: the payoff of every earlier decision

*Goal: see how the architecture choices from week 1 cash out.*

- The debugger core ([`coco-core/src/debug.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/debug.rs)): breakpoints, watchpoints
  compiled into a lean table installed only while running, the 1024-entry
  trace ring (week 4's trace-diff tool). **`peek()`**
  ([`bus/peek.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/peek.rs)) — the side-effect-free twin of `read()`; why the debugger
  would corrupt the machine without it (PIA flags, GIME IRQ acks).
- The debugger UI (`coco-egui/src/debugger/`): registers, disassembly with
  run-to-cursor, logical *and physical* memory views (the MMU made that
  distinction real in week 5), stack unwinding, hardware panels.
- Save states ([`coco-core/src/snapshot/`](https://github.com/sperano/cocovm/tree/main/crates/coco-core/src/snapshot)): the entire machine is a plain
  owned serde tree — because week 1 banned `Rc<RefCell>`. Container format
  (magic + version + gzip CBOR), media stored as **path + SHA-256, never
  bytes** (copyright + size), `#[serde(skip)]` host resources re-injected on
  restore, schema-evolution rules. Hostile-payload tests
  ([`snapshot_engine/hostile_payload.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/snapshot_engine/hostile_payload.rs)) — parsing untrusted input safely.
- Course retrospective: which decisions were load-bearing (Bus trait, no
  shared ownership, headless core, scanline loop) and what each cost.

**Reading:** [`debug.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/debug.rs), [`bus/peek.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/peek.rs), [`snapshot/`](https://github.com/sperano/cocovm/tree/main/crates/coco-core/src/snapshot),
[`coco-egui/src/save_state/`](https://github.com/sperano/cocovm/tree/main/crates/coco-egui/src/save_state); tests [`debug.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/debug.rs), [`snapshot_roundtrip.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/snapshot_roundtrip.rs),
[`snapshot_engine/`](https://github.com/sperano/cocovm/tree/main/crates/coco-core/tests/snapshot_engine).
**Exercises:** (1) explain what specifically breaks if the memory panel used
`read()` instead of `peek()` while BASIC waits for a keypress; (2) add a
field to a device struct following the evolution rules and prove old
snapshots still load; (3) capstone: pick any real CoCo program (game, demo,
OS-9), get it running, and use the debugger + trace ring to explain one
thing it does with the hardware.

---

## Appendices (self-study)

- **A. Emulating a machine you can't fully document:** how this codebase
  handles conflicting sources — the GIME timer-clock disagreement between
  references ([DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §4), hand-measured composite palettes, MAME/XRoar
  cross-checks, `hw-verify`-style claim verification against `docs/`.
- **B. ROM licensing and provenance:** `rom_db.rs` CRC validation,
  [`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md), why fonts and ROMs have different legal weight than code.
- **C. The road not taken:** cycle-exact mid-instruction timing, the 6309,
  bit-level UARTs — deferred-scope decisions and what would force them.
- **D. Tooling:** `examples/` as a lab bench (PPM out, no GPU); trace-diff
  workflow against MAME; `quick-check`-style CI habits.
