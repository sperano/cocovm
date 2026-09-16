# Appendices

*Self-study material for after Chapter 16. These are reference documents, not
lectures: denser than the chapters, fewer worked examples, no required
exercises (a couple of "try this" prompts are scattered throughout for the
curious). Every file path and code excerpt below was checked against this
worktree while writing; where an example needs a resource this worktree
doesn't have (`roms/`, `docs/*.pdf` — both git-ignored, machine-local), that
is stated rather than glossed over.*

---

## Appendix A — Emulating a machine you can't fully document

Every other appendix in this book assumes you can look something up. This
one is about the weeks when you can't: the GIME's timer clock has two
mutually exclusive published values, the composite palette isn't a formula
at all, and the cassette ROM's bit-bang algorithm lives in 8K of
undocumented 6809 code with no listing anywhere. The codebase's answer to
"what do you do when the sources disagree, or there are no sources" is not
a single trick — it's a small discipline, used consistently enough that you
can read it straight out of the comments. This appendix collects the cases
and distills the method at the end.

### Case 1: the GIME timer clock — two sources, one number, and a comment that never got updated

[`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §4 states the problem as sharply as a design document can:

> **Caution — sources disagree on the fast clock.** Sock's reference gives
> the two periods as **279.365 ns (≈3.58 MHz, the NTSC colour clock) fast**
> and **63.695 µs (≈15.7 kHz, the horizontal line rate) slow**. The
> cococommunity register reference instead lists **70 ns (≈14.3 MHz dot
> clock)** for the fast source. This must be pinned empirically against a
> reference emulator (XRoar / MAME `gime.cpp`) during implementation — do
> **not** hard-code a number on authority alone. The slow =
> horizontal-line-rate value is consistent across sources and is the safer
> one to rely on first.

Two secondary references, both purporting to describe the same register,
off by a factor of four (279.365 ns vs. 70 ns — not a rounding difference,
a wrong unit or a wrong clock entirely). The design document's response is
not to pick one on a coin flip and move on — it names both, names the
disagreement's shape, and defers the decision to a step the codebase can
actually verify: cross-checking against a reference emulator's source.

Here is what got implemented, [`crates/coco-core/src/machine.rs:28-34`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine.rs#L28-L34):

```rust
/// GIME timer input clocks per normal-speed CPU cycle with INIT1 TINS=1. The
/// fast timer clock is 3.579545 MHz (279.365 ns — hardware-measured; MAME
/// `gime.cpp`. SEB's "70 ns" is wrong), exactly 4× the 0.89 MHz CPU clock —
/// and 2× the double-speed CPU clock, since the timer runs off the fixed
/// video crystal and ignores the CPU rate. With TINS=0 the input is the
/// ~63.5 µs horizontal sync: one tick per scanline.
const FAST_TIMER_TICKS_PER_CPU_CYCLE: u32 = 4;
```

Sock's number won, cross-checked against MAME's `gime.cpp` source directly
rather than a secondary write-up of it — and along the way the comment
picks up a *third* data point: Super Extended BASIC Unravelled II also gives
70 ns, and the comment states flatly that it's wrong too. That's worth
sitting with for a second, because it inverts the naive heuristic "prefer
the primary hardware documentation over a community wiki" — here, two
independent documentary sources agree with each other and disagree with the
number a reference emulator's own device-model source code implements, and
the codebase sided with the emulator source. The reason is implicit but
inferable: MAME's `gime.cpp` is the artifact whose author had to make real
software boot correctly against it for years; a mistaken constant there
gets caught by a regression the first time a game misbehaves. A one-line
register-reference table has no such feedback loop.

Now the honest part: [`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §4 itself was never updated. Its "Caution"
block still reads as an open question — "this must be pinned empirically…
do not hard-code a number on authority alone" — months after the code
actually pinned it and did hard-code the resulting number, with a citation.
This is not a contradiction to paper over; it's exactly the shape of drift
you should expect in any project where the design doc is written *before*
the code ([`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md)'s own header says so) and reality is left to correct
it in place. `ch01`'s reading assignment points you at the "Correction
(2026-07…)" annotations elsewhere in the same file as the model for how
this is *supposed* to work — §4's timer note is a case where the
correction never got written back. When you read a design document
alongside its implementation, budget for this: the comment beside the
constant is more likely to be current than the paragraph in the design doc
that originally posed the question.

*Try this:* grep [`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) for "Caution" and "TODO", then check whether
each one still describes an open question by reading the file(s) it points
at. You will find at least one more where the code has since resolved the
question and the design doc hasn't caught up — that's not a bug in the
project; it's what "design doc written before the code" costs, permanently.

The MMU sizing footnotes in the same document (§3) are a second instance of
the same pattern, worth knowing about even though the numbers matter
less than the shape: the write-8-bits/read-6-bits banking asymmetry is
"verified against the Sock GIME reference / cococommunity reference and
corroborated by an owner of a 2 MB machine" — three independent
confirmations for one claim — while the 128K machine's exact valid bank
range is flagged "**not yet pinned** here — verify against the Super
Extended BASIC Unravelled docs and a reference emulator before coding it.
Do not assume a low-bit mask." Same vocabulary, same discipline: state what
is confirmed, name what isn't, and never let an unconfirmed number pass for
a confirmed one just because it's the only number you have.

### Case 2: the composite palette — when there is no formula to check against

[`crates/coco-core/src/gime/palette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs) opens with a sentence that rules out
an entire category of "just derive it" reasoning:

```rust
//! Monitor-signal-path colour resolution: RGB output is a straight bit
//! unpack, composite output goes through hand-measured lookup tables.
```

The table itself repeats the point:

```rust
/// Composite-monitor palette (BPI=0), 64 entries indexed by the 6-bit GIME
/// palette value, `0xRRGGBB`. Hand-measured on real hardware — there is no
/// formula. Verbatim from MAME `src/mame/trs/gime.cpp` `get_composite_color`
/// (BSD-3-Clause, Nathan Woods; see NOTICE.md).
const COMPOSITE_PALETTE: [u32; 64] = [ /* 64 entries */ ];
```

RGB monitor output is a straight 2-bit-per-channel unpack (`GIME::rgb_color`,
same file) — that one *is* a formula, three lines of bit shuffling, because
the real hardware genuinely just wires 6 palette bits to 6 DAC pins.
Composite output is a different physical process: the GIME's composite
encoder mixes luminance and a phase-shifted chroma subcarrier, and a real
NTSC monitor's decoder recovers hue and saturation from that mixed signal
in a way that does not reduce to a clean per-channel formula for a
*discrete* set of 64 register values. The relationship between "6-bit
palette register" and "perceived RGB" was measured on real silicon by the
MAME authors, not derived, and this codebase inherits that measurement
verbatim rather than attempting its own derivation (`ch09` §9.2 has you
hand-verify several entries against the table by eye, which is the honest
version of "checking the math": there is no simpler ground truth to check
it against).

The **methodology**, generalized past this one table: when a physical
process genuinely has no closed form worth deriving — and composite video
decoding is a canonical example, because it depends on subcarrier phase,
not just the register value — the correct move is not to approximate with
a formula that will be subtly wrong everywhere, but to obtain a
measurement that's known correct (here, borrowed from a reference
emulator's own hand-measured table, with the borrowing declared in the
source comment and in [`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md)) and encode it verbatim, with its
provenance attached. A formula you derive yourself, unchecked against
hardware, is a *guess dressed as rigor* — it looks more principled than a
borrowed table, but it isn't, unless you can verify it against something
real.

### Case 3: the cassette FSK timing — measuring what no document describes

The GIME's timer clock has two competing documents to arbitrate between.
The CSAVE/CLOAD bit-bang routine has *zero* — it lives inside the
undocumented $A000–$BFFF region of Color BASIC ROM, hand-written 6809
assembly with no published disassembly this project has access to. The
module comment on [`crates/coco-core/examples/cassette_calibrate.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/cassette_calibrate.rs) says so
directly:

```rust
//! Calibration probe: boot the real ROM, type a one-liner program, run
//! CSAVE"X", then dump the raw DAC transition capture
//! ([`coco_core::cassette::Cassette::capture`]) so the FSK timing (cycles/bit,
//! bit order, block framing) can be measured empirically — the CSAVE/CLOAD
//! bit-bang code lives in the undocumented $A000-$BFFF Color BASIC ROM, so it
//! can't be derived from local docs (`cassette-verified-facts` memory).
```

When documentation runs out entirely, the method becomes: **build an
instrument, run the real thing through it, and read the result off the
instrument.** `cassette_calibrate.rs` boots the actual ROM (needs
`roms/coco3.rom`, absent in this worktree — the tool itself is the
evidence, not a claim about a number you have to take on faith), types a
one-line BASIC program, issues `CSAVE"X"`, and captures every DAC level
transition the ROM's bit-bang routine produces on the cassette output line.
It then histograms the deltas between transitions to find the two tone
periods empirically, and separately does a zero-crossing analysis matching
what the codebase's own demodulator ([`cassette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cassette.rs)) does, so the calibration
method mirrors the consuming code's detection strategy rather than an
idealized one.

The findings recorded in the same file's header are the payoff, and they
directly contradict the "obvious" assumption:

```
//! - Each bit is one full DAC sine cycle: a 0-bit measures ~793 CPU cycles
//!   (~1128 Hz), a 1-bit ~455 cycles (~1967 Hz) — close to, but not exactly,
//!   the canonical 1200/2400 Hz (a hand-tuned ROM delay loop, not a crystal-
//!   locked tone; the ROM's own hysteresis demodulator tolerates the drift).
//! - Bit order is LSB-first.
//! - Block framing and checksum exactly match `cassette-verified-facts`:
//!   `$55* $3C type len data… checksum`, checksum = `sum(type,len,data) & 0xFF`.
```

"1200/2400 Hz Kansas City standard" is the textbook answer every FSK
write-up gives for CoCo cassette encoding, and it is *approximately* right
— close enough that trusting it outright would have produced a
working-looking emulator that occasionally desyncs on real captured tapes,
because the real ROM's delay loop isn't crystal-locked to those round
numbers at all. The only way to find that out was to measure the actual
behavior of real ROM code and cross-check the decode against a known-correct
answer (the tokenized program bytes and block checksums for the specific
one-liner that was CSAVEd) — not to trust the canonical spec, and not to trust a
hand-derived cycle count either. `ch12` tells this as a narrative; the
generalizable lesson is the instrument-first method: when there is no
document, there is still the running system, and a purpose-built probe that
captures what it actually does — cross-checked against an independent ground
truth you can verify by hand (here: does the decoded output match the
program you know you typed?) — is strictly more trustworthy than any
number you could look up.

### Case 4: reference emulators as a source class of their own

MAME and XRoar show up throughout this codebase in two distinct roles, and
it's worth keeping them separate:

1. **As a validation oracle** — [`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5's CPU testing strategy
   (there is no TomHarte-style per-instruction suite for the 6809): boot
   the real ROM in both this emulator and a reference one from the same
   reset vector, dump a per-instruction trace from each (PC, opcode,
   registers, cycles), and diff. The first divergent line is the bug. This is
   how the codebase substitutes for a test suite that doesn't exist for
   this CPU family — see Appendix D for the concrete workflow.
2. **As a source of verbatim, licensed data** — the MC6847/GIME font
   bitmaps and the composite palette tables aren't *inspired by* MAME's
   implementation; they are copied from it byte-for-byte, because the
   alternative (re-measuring a font ROM or a composite decoder from
   scratch) is enormous, redundant effort for data MAME's authors already
   extracted correctly. [`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md) documents exactly what was borrowed and
   under what license (see Appendix B).

Both roles rest on the same premise: MAME and XRoar are, in aggregate,
better-verified than any single secondary document, because they've been
cross-checked against thousands of pieces of real software for decades.
That doesn't make them infallible — Case 1 above shows this project siding
with MAME's *source code* over two other documents, not over MAME's word
alone — but it does make "does a reference emulator agree?" a
cheap, high-value check to run before trusting your own reading of a
register description.

### Case 5: what "verify against local docs" actually means, and what this worktree can't do

[`CLAUDE.md`](https://github.com/sperano/cocovm/blob/main/CLAUDE.md)'s project instructions for this repository state the discipline
plainly: verify hardware claims against the PDFs in `./docs/` — 6809/6309
instruction sets, the MC6809 programming manual, the CoCo 3 Service Manual,
Super Extended BASIC Unravelled II, memory maps — using `pdftotext -layout`
instead of guessing or web search. Both `./docs/` and `./roms/` are
git-ignored specifically because they're copyrighted; they exist only on
machines whose owner has independently obtained them.

Worth being honest about, in an appendix that's precisely about not
overclaiming what you can verify: **this worktree has neither.**

```
$ ls docs/
bitbanger-spec.md  cartridges.md  dmp105-protocol.md
idiomatic-review-2026-07-01.md  ssc-spec.md  visual-machine-mode.md
$ ls roms/
ls: roms/: No such file or directory
```

`docs/` here holds only this project's *own* design notes (Markdown,
tracked or not — several of these are themselves the kind of working
document `.gitignore`'s `plan-*.md` pattern excludes from version control),
not the copyrighted reference PDFs. There is no `coco3.rom` to boot, which
is why every ROM-dependent claim in this appendix and the later ones is
either (a) read directly out of source comments that themselves cite MAME
or the datasheet, or (b) explicitly marked as unrun. This is not a gap
specific to this worktree — it's the normal condition for a fresh clone,
a CI runner, or a contributor who hasn't obtained the same reference
material, and the project's whole [`rom_db.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/rom_db.rs) + [`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md)
apparatus (Appendix B) exists so that the *code* still works and is
still auditable under that condition. Writing this appendix without local
PDFs or ROMs is, in a small way, a test of whether the codebase's
provenance discipline is sufficient on its own — and for everything cited
above, it was: every fact traces to a comment, a test, or [`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md), not
to a PDF that had to be taken on faith.

### The method, distilled

Read across all five cases and the same handful of moves recur. Use them
in this order when you hit a hardware fact this codebase (or your own)
doesn't already encode:

1. **Rank your sources before you need one.** Hardware-owner-corroborated
   measurements and a reference emulator's own device-model *source code*
   sit above secondary write-ups (community wikis, register-reference
   pages), which sit above your own derivation from first principles when a
   formula is plausible but unverified. Case 1's resolution — MAME source
   over two agreeing secondary references — only makes sense if you've
   already decided that ranking; done in the moment, under time pressure,
   you'd likely trust the two-against-one majority instead.
2. **When sources disagree, write the disagreement down, don't silently
   pick one.** [`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §4's "Caution — sources disagree" block names
   both numbers, both sources, and defers to a specific verification step
   — that block is *more* useful to a future reader than a single
   confidently-stated (and possibly wrong) number would have been, even
   though it looks less finished.
3. **When documentation runs out, measure.** `cassette_calibrate.rs`
   exists because no document could answer the question; an instrument
   built out of the emulator's own instrumentation (`Cassette::capture`)
   could. This generalizes: a scratch `examples/` binary that logs the
   thing you can't find documented, run against real ROM code, beats
   guessing every time such code is available to run.
4. **Cross-check measurements and formulas against a reference
   implementation before trusting them.** Not because MAME/XRoar are
   infallible, but because "two independently-arrived-at numbers agree" is
   real evidence and costs one `grep` through a checked-out MAME source
   tree or one trace-diff run.
5. **Record provenance where the next reader will actually see it** — in
   the source comment beside the constant or table, not only in a design
   document that may drift out of sync (Case 1) or a NOTICE file that
   covers licensing but not the underlying hardware-fact reasoning. `Sock's
   reference gives…`, `Hand-measured on real hardware — there is no
   formula`, `verified against a MAME screenshot` — every one of these
   phrases is doing real work: it tells the next person exactly how much to
   trust the number next to it, and exactly what to go re-check if it ever
   turns out to be wrong.

---

## Appendix B — ROM licensing and provenance

An emulator's own code can be entirely original and still ship next to
material — ROM images, character-generator fonts, palette tables — that
isn't. This appendix reports what this codebase actually does about that,
file by file, and quotes [`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md) rather than paraphrasing it: the goal
is an accurate account of *this project's* choices, not legal advice.

### The workspace license, and why one crate is licensed differently

[`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md) states the split up front:

> - **`crates/mc6809`** (reusable MC6809 CPU core) — dual-licensed **MIT OR
>   Apache-2.0** at your option, so other projects can adopt it without
>   copyleft obligations.
> - **`crates/coco-core`, `crates/coco-egui`** (the emulator itself) —
>   **GPL-3.0-or-later**: you can redistribute and/or modify them under the
>   GNU GPL as published by the Free Software Foundation, version 3 or (at
>   your option) any later version.
>
> Note the GPL crates depend on the permissive `mc6809` crate (fine:
> permissive code may be combined into a GPL work), never the reverse —
> keep `mc6809` free of GPL-licensed code.

This mirrors the workspace boundary established in `ch01`
(`mc6809` depends on nothing, knows only the `Bus` trait): the *licensing*
boundary and the *dependency* boundary are the same boundary.
`crates/mc6809/LICENSE-MIT` and `crates/mc6809/LICENSE-APACHE` sit inside that
crate; the root `LICENSE` file (GPL-3.0-or-later) covers the workspace as a
whole via [`Cargo.toml`](https://github.com/sperano/cocovm/blob/main/Cargo.toml)'s `[workspace.package] license = "GPL-3.0-or-later"`.
The one-way dependency arrow from `ch01` §1.5 (`coco-egui → coco-core →
mc6809`) is also, it turns out, the one-way rule for what license terms are
allowed to flow into what: GPL code may depend on permissive code, never
the reverse, and keeping `mc6809` GPL-free is cheap to hold to, because the
crate has almost nothing to hold the line against. Cargo does not check
license compatibility, but [`crates/mc6809/Cargo.toml`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/Cargo.toml)'s `[dependencies]`
section has exactly one entry — an optional `serde`, itself MIT OR
Apache-2.0 — so there is no transitive dependency graph for a copyleft
license to arrive through, and adding one would be a visible line in a
fifteen-line manifest rather than something that sneaks in.

### Bundled MAME material — what, and under what terms

[`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md)'s "Bundled third-party material" section is short enough to
summarize completely. Three bullets covering four tables — two
character-generator fonts, two composite palette variants — all originally
copied from MAME source, all under the same license:

> Both character-generator bitmap tables were copied from **MAME**. The
> source files are per-file licensed **BSD-3-Clause**, copyright **Nathan
> Woods** (verified against the file headers 2026-07-01 — an earlier
> version of this notice recorded them as GPL-2.0-or-later, which was
> wrong). BSD-3-Clause is GPL-compatible; the attribution below satisfies
> its notice requirement.
>
> - **MC6847 font ([`crates/coco-core/src/font6847.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font6847.rs))** —
>   `vdg_t1_fontdata8x12` from `src/devices/video/mc6847.cpp`.
> - **GIME hi-res font ([`crates/coco-core/src/font_gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font_gime.rs))** —
>   `gime_device::hires_font` from `src/mame/trs/gime.cpp`.
> - **Composite-monitor palette tables** — `gime_device::get_composite_color`
>   from `src/mame/trs/gime.cpp`.

followed by the full BSD-3-Clause license text (three conditions: retain
the copyright notice, reproduce it in binary redistributions, don't use the
contributors' names to endorse derived products) and the standard
disclaimer of warranty.

Two things are worth stating carefully, because they're the kind of detail
an appendix like this exists to get right rather than gloss over:

**First, the license classification changed, and [`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md) says so
candidly.** Git history confirms it: an earlier commit filed the two font
tables under GPL-2.0-or-later (the palette tables were added to the notice
later, already BSD-3-Clause); a later one corrected the record to
BSD-3-Clause after checking the actual MAME file headers, and the current
[`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md) text explicitly flags its own prior version as wrong rather
than silently fixing it. That candor is worth more than it costs — a
reader who only skimmed an older revision, or who half-remembers "wasn't
this GPL?", gets told directly that the record was corrected and why.

**Second, the in-source comments were never updated to match, and still
say the old thing.** [`crates/coco-core/src/font6847.rs:24-25`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font6847.rs#L24-L25):

```rust
//! SOURCE / LICENSING: both tables are taken from MAME's `mc6847.cpp`
//! (`vdg_fontdata8x12` and `vdg_t1_fontdata8x12`, GPL-2.0+). They are
```

and [`crates/coco-core/src/font_gime.rs:3-5`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font_gime.rs#L3-L5):

```rust
//! LICENSING: copied from MAME's `src/devices/video/gime.cpp`
//! (`gime_device::hires_font`), GPL-2.0+ — same pending licensing decision as
//! `font6847.rs`; see `NOTICE.md`.
```

Both still describe a "pending licensing decision" that [`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md) records
as already resolved (BSD-3-Clause, verified against file headers). This is
Appendix A's doc-drift pattern again, in a place where it matters more than
most: **[`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md) is the authoritative, current record** — it's the
document that was explicitly corrected and explains why — and these two
source comments are stale. If you're ever auditing this codebase's
licensing for real (packaging it, redistributing it, adopting a piece of
it elsewhere), read [`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md), not the module doc comments; if you're
maintaining it, this is an outstanding cleanup — two doc comments that
should be edited to match the notice they already point at.

There's a third small staleness in the same neighborhood: [`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md)
names the composite palette tables' location as
[`crates/coco-core/src/gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs). Since the notice was last updated, the
"break oversized files into focused modules" refactor (commit `762f096`)
moved `COMPOSITE_PALETTE`/`COMPOSITE_PALETTE_180` into
[`crates/coco-core/src/gime/palette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs). The file that actually holds the
tables today still opens with `// Verbatim from MAME src/mame/trs/gime.cpp
get_composite_color (BSD-3-Clause, Nathan Woods; see NOTICE.md)` — so the
license attribution traveled with the code through the refactor even
though [`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md)'s path reference didn't get updated to follow it. Same
lesson as above: trust the in-code attribution comment for *which file*,
and [`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md) for *what license*.

### The 3D model, and the asset that isn't in the repo yet

```
## 3D model attribution

- "TRS-80 Color Computer 2" (https://skfb.ly/6V6IL) by ericomont, licensed
  under Creative Commons Attribution 4.0. Not yet committed to the repo or
  the assets tarball; whenever it ships or is rendered in-app, this credit
  must also be shown to the user (e.g. the About window).
```

Worth noting as a project-management pattern more than a licensing one:
the attribution obligation is recorded *before* the asset exists in the
tree, with an explicit note about where the runtime credit needs to
surface once it does. Attribution debt is tracked the same way code debt
is — as a TODO with enough context that whoever ships the feature doesn't
have to go re-derive the obligation from scratch.

### Local, git-ignored assets — what's excluded and why

```
## Local, git-ignored assets (not distributed)

- `roms/` — copyrighted Tandy/Microsoft ROM images (`coco3.rom`, `disk11.rom`).
- `docs/*.pdf` — copyrighted reference PDFs.

Both are excluded via `.gitignore`.
```

The relevant `.gitignore` lines, confirmed against this worktree:

```
/docs/*.pdf
/docs/plan-*.md
/docs/*-plan.md
/roms/*
/*.ccc
/*.rom
```

— ROM images, reference PDFs, and even loose cartridge dumps (`.ccc`,
`.rom`) at the repo root are excluded categorically, not just the two named
files. This is the practical consequence of the copyright question
`NOTICE.md` doesn't try to resolve on this project's behalf: Tandy/Microsoft
ROM images and third-party reference PDFs are not this project's to
redistribute, so the repository never contains them, and every tool
that needs one (the boot tests, the calibration examples, Appendix D's
whole catalog) is written to read them from a local, unversioned path and
degrade honestly (skip, or fail with a clear message) when they're absent
— exactly the behavior Appendix D verifies example-by-example.

### Why snapshots store a path and a hash, never the bytes

Save states (`ch16`, `crates/coco-core/src/snapshot/`) raise the same
question in a different shape: a `.ccstate` file captures the *entire*
machine, and the machine's RAM at any moment contains copyrighted ROM
content the CPU has been executing out of, plus whatever disk/tape/cart
image is mounted. [`crates/coco-core/src/snapshot/payload.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/snapshot/payload.rs) answers it
structurally, not by policy alone — the type that represents "a piece of
mounted media" has no field a ROM's bytes could go in:

```rust
/// Where one media file lived and what it hashed to, at save time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaRef {
    /// As the frontend knew it at save time — absolute or relative, whatever
    /// the frontend itself used; this module never resolves or interprets
    /// it, only carries it.
    pub path: PathBuf,
    /// Lowercase hex SHA-256 of the file's contents at save time (see
    /// [`super::sha256_file`]).
    pub sha256: String,
}
```

`MediaRefs` (the same file) holds one `Option<MediaRef>` per media slot —
`system_rom`, `cart_roms`, `disks`, `vhds`, `drivewire`, `tape` — and
`RestoredMachine`/`MediaSources` make the resulting contract explicit:
`load()` (deserializing the snapshot) never needs media bytes at all; only
`restore()` does, and it receives them from the *caller*, not from the
snapshot file — "a real frontend re-reads each `path`; tests inject bytes
directly." A snapshot is a self-describing pointer plus a checksum: enough
for the frontend to say "this is the same `coco3.rom` you had when you
saved" (or to warn you it isn't) without the snapshot file itself ever
holding a byte of Tandy's ROM. This gets you two things at once, from one
design decision: the copyright question doesn't arise (nothing
redistributable is embedded), and a snapshot's size tracks the emulated
machine's RAM rather than its mounted media. The RAM contents *are*
serialized — that's `Machine`'s own state, owned by this project, and
`SystemBus::ram` is by its own comment "the single biggest snapshot
payload, up to 2 MB" — while the ROM/disk/VHD images it was loaded from,
however large, are not.

*Try this:* read [`crates/coco-core/src/snapshot/payload.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/snapshot/payload.rs)'s doc comment
on `RestoreNote` and on `MediaRefs`'s `disks`/`vhds`/`drivewire` fields (why
`Vec<Option<MediaRef>>` and not a fixed-size array) — it's a second,
smaller instance of designing a serialized format so that a *future* change
to the code can't silently corrupt an *old* snapshot, the same
forward-compatibility discipline `ch16` covers for the rest of the machine
tree.

### What this appendix is not

Everything above is a report of what [`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md), the `.gitignore`, and the
snapshot code actually do — not an opinion about whether GPL-3.0 and
BSD-3-Clause compose the way [`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md) says they do, whether the
Creative-Commons-licensed 3D model's attribution requirement is satisfied
by an About-window credit, or any other question a lawyer would need to
answer for a specific redistribution plan. If you fork this project or
lift a piece of it, read [`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md) and the license files it points to
yourself; this appendix's job is only to make sure you know they exist
and roughly what they say.

---

## Appendix C — The road not taken: deferred scope, and what would force it

Every emulator author cuts scope constantly — the skill is cutting it
*visibly*, so a future maintainer (or you, eighteen months later) can tell
"nobody's gotten to this yet" from "this was decided against for a reason."
This codebase's convention is to name the cut in a comment or a design-doc
line at the moment it's made, with enough context that reopening the
question later doesn't require re-deriving it from nothing. This appendix
collects the standing cuts, states plainly who would notice each one in
practice, and — the part worth the most attention if you're ever deciding
whether to *un-defer* one — what implementing it would force elsewhere in
the architecture. Every claim below was re-checked against the current
source while writing this, not assumed still true from an earlier design
note.

### The Hitachi 6309

[`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) disposes of this in one parenthetical, twice:

> (Design for the Hitachi 6309 later as a feature-flagged superset, but
> don't pay for it now — ask before adding that scope.)

and again in the key-decisions list: "6309 and Multi-Pak designed-for but
not built — don't add that scope without asking." The 6309 is a pin- and
software-compatible 6809 replacement that Hitachi shipped; most CoCo 3
owners never had one, but OS-9/NitrOS-9 has long had optional support for
it. In broad terms (general 6309 knowledge, not sourced from a local
datasheet — none is present in this worktree, see Appendix A Case 5): it
adds a second accumulator pair (`E`/`F`, combinable as 16-bit `W`,
and as 32-bit `Q` with `D`), extra instructions available in either mode
(16-bit multiply and divide — `MULD`, `DIVD`/`DIVQ`, where the 6809 had
only 8×8 `MUL` — block-transfer `TFM`, bit-manipulation opcodes), and a
"native mode" that alters interrupt stacking and shaves cycles off many
existing instructions. **Who'd notice:** 6309-native OS-9/NitrOS-9 builds
and the handful of utilities written to detect and exploit the extra
registers or native-mode instructions; ordinary 6809 CoCo 3 software is
unaffected either way. **What it would force:** not a flag on the existing
`MC6809` struct — the 6309's extra registers and altered cycle table mean a
second, parallel core implementation (still generic over the same `Bus`
trait, per `ch01`'s design, but its own dispatch `match`, its own register
file, its own cycle table) selected at machine-configuration time, plus a
second trace-diff/validation pass (Appendix A's method again) against
whatever reference emulator support exists for it.

### Pixel-level NTSC artifact colors

`ch09` establishes the current state precisely (§9.15, excerpted): the
64-entry composite palette table (Appendix A Case 2) is exactly correct for
every *intentional* color a program selects via the GIME's palette
registers, and only misses a specific unintended effect real composite
monitors produce. Certain black/white bit patterns in the CoCo 1/2's
legacy two-color graphics modes "beat" against the NTSC color subcarrier
and appear tinted on a real TV, a side effect some early-80s software
exploited on purpose for extra colors nobody's palette register ever
selected. **Who'd notice:** legacy CoCo 1/2 software relying on the
"artifact color" trick in two-color PMODE graphics — a narrow, dated
category, but a real one (the trick was popular enough in the era's
graphics demos and a few games to have a name). **What it would force:** an
entirely different algorithm class, not a bigger table — a real per-pixel
NTSC decoder tracking several consecutive pixels' luminance and their
position relative to the subcarrier's phase (which advances a fixed,
non-integer amount per pixel clock, so identical bit patterns decode
differently depending on which screen column they start at), replacing the
current `O(1)`-per-pixel table lookup with something genuinely
neighbor-dependent.

### Cycle-exact mid-instruction CPU timing

[`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5 states the policy the whole CPU core follows: "Don't try to
be cycle-*exact* mid-instruction at first; instruction-granular cycle
counts are enough to get the ROM booting and sync interrupts roughly right.
Tighten later only if a game needs it." `ch02` §2.7 confirms this is
exactly what shipped: `step()` never tracks *when* during an instruction a
bus access happens, only that the whole instruction costs a fixed number of
cycles matching the datasheet's per-opcode table — `LDA` extended is 5
cycles regardless of which of those 5 the address-bus-valid moment "really"
falls on. **Who'd notice:** software timing a raster effect or a
copy-protection check to a specific bus cycle *within* an instruction, not
just to a specific instruction or scanline — demos doing sub-scanline
raster tricks, and copy-protection schemes designed specifically to detect
timing anomalies an interpreter would introduce. **What it would force:**
the `Bus` trait (`ch01` §1.3) would need to grow from "whole read/write per
call" to a tick model — something closer to `Bus::tick(&mut self)` called
once per bus cycle with reads/writes resolved at the specific cycle the
real 6809's bus-cycle table says they occur. That in turn means the
per-opcode cycle *table* this codebase currently has would need to become a
per-opcode cycle-by-cycle bus-access *sequence*, and the scanline-driven
main loop (`ch06`) would need sub-line granularity where it currently has
none.

### Interrupt-entry cycle cost — now accounted

A specific, already-documented instance of the previous item is now
implemented: accepted running `IRQ`/`NMI` entry costs 19 cycles, running
`FIRQ` costs 10, and an accepted interrupt waking `CWAI` costs 4 because its
full frame is already stacked. Masked or unarmed delivery costs zero. `SWI`,
`SWI2`, and `SWI3` remain step-costed and are not double charged. The
`coco-core` scheduler includes external entry by observing the CPU cycle delta
around line delivery. **Who'd notice:** software timing interrupt latency
itself (some copy protection did), and anyone trace-diffing this emulator
against real MAME (Appendix A Case 4, Appendix D): the cycle boundary now
remains aligned while register and frame behavior stay independently testable.

### Bit-level UART framing

`ch14` states the fidelity choice in the module doc comment of
[`crates/coco-core/src/acia6551.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551.rs) itself:

> MAME's `mos6551_device` is a bit-serial engine: it shifts one bit at a
> time off a per-bit timer and can therefore generate real parity/framing
> errors and expose bit-accurate RS-232 waveforms. This model is
> deliberately **byte-level**: `ACIA6551::tick` runs a whole-frame timer for
> the receiver and transmitter... and delivers/consumes a complete byte when
> that timer expires.

Concretely, per `ch14`: this model never generates a parity or framing
error from its own receive process (only from an already-corrupted byte a
caller injects), approximates echo mode by re-queuing a whole received byte
onto the transmit line rather than retransmitting bit-by-bit as it arrives,
and collapses the 5-bit-word/2-stop-bits corner case to a plain 2 stop bits
instead of the real chip's 1.5. **Who'd notice:** software that
*deliberately* depends on bit-level RS-232 misbehavior — a modem
diagnostic program injecting a framing error on purpose to exercise its own
recovery path, or an oscilloscope-style serial line monitor — behavior
that, per `ch14`, nothing shipped for the CoCo ever depended on. Ordinary
terminal software and BASIC's `OPEN "S"` I/O only ever check that correct
bytes arrive at the right cadence, which this model already delivers.
(Contrast with the printer bitbanger, [`crates/coco-core/src/bitbanger.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs) — that one *is*
already bit-level, an edge-triggered receive state machine sampling each
bit cell's midpoint, because there's no real UART silicon underneath a
software bit-bang driver to summarize into whole frames; "byte-level" was a
choice available for the 6551 specifically because a real chip does the
bit-shifting in hardware.) **What it would force:** replacing the
whole-frame timer with a per-bit shift register ticked at the configured
bit rate, plus a place to inject synthetic line noise for the diagnostic
software this would actually serve — meaningful added complexity for an
audience `ch14` couldn't name a single real example of.

### PAL timing for the CoCo 1/2's plain MC6847

The CoCo 3 already supports both `VideoStandard::NTSC` and `::PAL`
end-to-end ([`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md)'s "Settled decisions" list). CoCo 1/2 machines,
which used the plain MC6847 rather than the GIME, do not —
`crates/coco-core/src/config.rs` rejects the combination outright:

```rust
if self.video == VideoStandard::PAL {
    return Err(format!(
        "{:?} PAL is out of scope (plain MC6847 PAL timing not modeled)",
        self.variant
    ));
}
```

A comment a little further up the same file names exactly what's
unresolved and why the rejection exists rather than a best-effort guess:

```rust
// UNVERIFIED: MAME's PAL timing offsets this edge by
// `LINES_PADDING_TOP_PAL` (mc6847.cpp), which could not be pinned
// ...
// rather than guess a line number. CoCo 1/2 + PAL is rejected by
```

— the same Appendix A discipline (name what's unconfirmed, don't encode a
guess as a fact) applied as a hard `validate()` rejection rather than a
silent wrong answer. **Who'd notice:** European CoCo 1/2 owners and
software written for 50 Hz field-rate PAL hardware — a real but
geographically narrow audience, already served for the CoCo 3 case.
**What it would force:** pinning the real MC6847's PAL vertical-timing
offset (the specific value MAME encodes as `LINES_PADDING_TOP_PAL` but
which this project couldn't independently confirm — Appendix A's "measure
or cross-check before you code it" rule, currently blocked on the
cross-check step), then removing the [`config.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/config.rs) rejection and extending
the legacy VDG boot-test coverage (`ch07`) to a PAL configuration.

### The H50 bit: parsed, never wired

`ch09` §9.15 names this precisely. [`crates/coco-core/src/gime.rs:80`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L80)
defines the constant:

```rust
pub const H50: u8 = 0x08;
```

— "$FF98 bit 3, 50 Hz field rate (else 60 Hz)," a real, named, documented
GIME register bit that hardware lets software toggle live, mid-operation.
Nothing else in the crate ever *reads* it: field rate in this codebase is
decided once, at machine-construction time, by
`MachineConfig`'s `VideoStandard` enum, the same moment `MonitorType` is
chosen. **Who'd notice:** software that toggles `$FF98` bit 3 at runtime to
switch field rate mid-operation rather than accepting whatever rate the
machine booted with — rare in practice (most software picks a rate once,
if it cares at all), most plausible in European utilities or
diagnostic/demo code deliberately probing GIME register behavior. **What it
would force**, per `ch09`'s own framing of the question (§9.17 exercise 9.6):
`end_of_line`/`run_field`'s fixed `lines_per_field` assumption (`ch06`)
would have to become mutable mid-field rather than read once per field, and
`render_scanline`'s canvas math — currently a fixed `CANVAS_H = 240` — would
need to handle the active line count itself changing under it without
corrupting framebuffer geometry. A small register read turns into a
structural assumption breaking in two different files.

### Rotational latency and head settle time in the FDC

`ch13` §13.12 states the WD1773's fidelity choice directly: "Modelled
functionally rather than cycle-exact: command completion and byte transfers
are paced by `tick()` against fixed cycle counts... not the real chip's
per-command timing tables." What *is* paced precisely enough that real ROM
code depends on it: the ~32 µs byte interval during a transfer, the search
latency before the first byte, and the trailing gap before `INTRQ`. What is
**not** modeled at all: rotational latency (how long a real head waits for
the target sector to spin under it — anywhere from zero to a full
revolution, purely by luck) and head-settle time proportional to seek
distance (`COMMAND_SETTLE_CYCLES` is a fixed 64 cycles for both `Restore`
and `Seek`, regardless of how many tracks the head actually crosses).
**Who'd notice:** copy-protection schemes that timed seek duration
proportionally to distance, or that checked inter-sector gap timing or
rotational position deliberately — `ch13` names this as a real, historical
category of software, not a hypothetical. Ordinary DOS/OS-9 disk access,
which just wants the right bytes back in a bounded time, is unaffected.
**What it would force:** turning the FDC from a command-driven state
machine into something with an actual simulated spinning disk underneath
it — an angular-position clock derived from elapsed cycles and RPM, seek
time genuinely proportional to track distance, and a sector search that
can legitimately "miss" the target sector and have to wait out a full
revolution, rather than `sector_offset` answering instantly and correctly
regardless of where a real head would physically be.

### 8 MB CoCoZilla-style banking

[`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §3's MMU sizing footnote: "2 MB on a stock GIME is real
(owner-confirmed). 8 MB exists too via further banking (e.g. CoCoZilla) but
is out of scope." The current MMU model already addresses up to the real
GIME's own limit — 2048K, 256 blocks, the full 8-bit block register per the
table in the same section — so this isn't a case of the emulator falling
short of the stock chip; it's a case of a third-party hardware modification
(CoCoZilla) exceeding what the stock GIME's own address decoder can
express, by adding banking logic the GIME itself doesn't have. **Who'd
notice:** owners of CoCoZilla or similar third-party memory-expansion
modifications, and any homebrew software specifically written to exploit
one — a small, modern-hobbyist audience, not historical CoCo software.
**What it would force:** not a wider `MemorySize` enum — the GIME's own
8-bit block register genuinely tops out at 256 blocks / 2 MB — but an
entirely separate banking layer modeling the third-party board's own
address decoder sitting *in front of* the GIME, translating a further
selector register into which 2 MB window the GIME currently sees. It's a
new device, not a bigger number.

---

## Appendix D — Tooling: the lab bench

The core crate is headless by design (`ch01` §1.5) specifically so it can
be driven from small, disposable programs instead of the full GUI. This
appendix catalogs that toolbox as it exists today, run (or, where a ROM is
required, read) directly against this worktree.

### `crates/coco-core/examples/` — what's there and what each one needs

```
$ ls crates/coco-core/examples/
cart_boot_probe.rs   demo_frames.rs       gime_demo.rs      trace.rs
cassette_calibrate.rs disk_boot_probe.rs  palette_trace.rs  vdg_font_probe.rs
eou_gshell_probe.rs
```

Nine examples, run with `cargo run -p coco-core --example <name> --
<args>`. Eight of the nine call `std::fs::read("roms/coco3.rom")` (or an
equivalent path) somewhere in `main()` and will fail immediately in this
ROM-free worktree; **one does not**, and running it here confirms it works
exactly as documented:

```
$ cargo run -p coco-core --example gime_demo -- /tmp/ppm
wrote /tmp/ppm/text80.ppm (640x240)
wrote /tmp/ppm/hscreen2.ppm (640x240)
```

[`gime_demo.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/gime_demo.rs) synthesizes an 80-column attribute text screen and an
HSCREEN-2 color-bar frame entirely by poking `GIME` registers and RAM
directly — no ROM, no CPU execution at all — then calls `gime_video`'s
renderer and writes the resulting framebuffer as a PPM. The 640×240
dimensions match the canonical raster canvas `ch07` introduces
([`raster.rs:16`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/raster.rs#L16)), confirming the example renders through the same code path
the real machine's video pipeline does, just with hand-poked registers
standing in for ROM-driven ones.

The other eight, read from their own headers (not run here, since
`roms/coco3.rom` and friends are absent — see Appendix A Case 5):

| Example | Needs | Does |
|---|---|---|
| [`trace.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/trace.rs) | `roms/coco3.rom`, optionally a cart `.ccc` | Per-instruction CPU trace in a MAME-comparable format; two modes (see below). |
| [`palette_trace.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/palette_trace.rs) | `roms/coco3.rom` + a cart | Logs every GIME palette-register write with the PC that made it, plus periodic snapshots. |
| [`cassette_calibrate.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/cassette_calibrate.rs) | `roms/coco3.rom` | Boots BASIC, types a one-liner, `CSAVE`s it, dumps the captured FSK waveform (Appendix A Case 3). |
| [`demo_frames.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/demo_frames.rs) | `roms/coco3.rom` + a `LOADM` binary | Injects a demo binary the way `LOADM` would, dumps PPM frames periodically for per-scanline-effect comparison against MAME screenshots. |
| [`vdg_font_probe.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/vdg_font_probe.rs) | `roms/extbas11.rom`, `bas12.rom`, `coco3.rom` | Boots CoCo 1 (MC6847), CoCo 2 (MC6847T1), and CoCo 3 (GIME font) to the BASIC prompt, dumps each as a PPM for side-by-side glyph comparison. |
| [`cart_boot_probe.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/cart_boot_probe.rs) | `roms/coco3.rom` + a cart | Boots an arbitrary cartridge image, dumps periodic framebuffer + CPU-state snapshots. |
| [`disk_boot_probe.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/disk_boot_probe.rs) | `roms/coco3.rom`, `disk11.rom` + a `.dsk` | Boots Disk BASIC with a disk mounted, types commands, dumps the resulting text screen. |
| [`eou_gshell_probe.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/eou_gshell_probe.rs) | `roms/coco3.rom` + a VHD image | Boots NitrOS-9 EOU, starts `gshell`, dumps GIME video-register state to diagnose color rendering. |

One ROM-dependent example deserves a closer look at its doc comment — in
both of its modes — because it's the tool referenced elsewhere in this
book: `trace.rs` supports a no-cart mode (a deterministic cold-start trace with no
interrupts, "comparable 1:1 against MAME up to the point BASIC first
enables interrupts") and a cart mode that drives
`Machine::step_instruction()` — interrupt servicing, hsync, vsync, and GIME
timer ticks all included — "so the full boot-and-run stream can be diffed
against a MAME run with `-cart1 <pak.ccc>`." Those are the concrete
mechanics behind the trace-diff method Appendix A Case 4 and `ch04`
describe in the abstract.

### The PPM workflow

Every visual example in this list writes plain binary PPM (`P6`) files:
a short ASCII header (`P6\n{width} {height}\n255\n`) followed by raw RGB
triples, one per pixel, row-major. No library, no GPU, no window — `ch01`
§1.7 calls this "the PPM lab bench" for exactly that reason. To view one,
any general image viewer that reads PPM (macOS Preview, GIMP, most Linux
image viewers) opens it directly; to convert to something more universally
shareable, ImageMagick handles it in one line:

```
convert text80.ppm text80.png
```

`gime_demo.rs`'s output, confirmed above, is a normal 640×240 24-bit PPM —
nothing about the format needs special handling beyond what any of those
tools already do.

### Trace-diffing against MAME/XRoar, step by step

Putting Appendix A Case 4 and the `trace.rs` header together into a
concrete recipe:

1. **Get a reference trace.** Boot the identical ROM in MAME's or XRoar's
   built-in debugger from the same reset vector, and enable its
   instruction trace (MAME: `-debug`, then a `trace` debugger command;
   XRoar has an equivalent trace log option). Save it to a file.
2. **Get this emulator's trace.** `cargo run -p coco-core --example trace
   -- <max_instrs> > mine.trace` for the no-cart deterministic mode, or add
   a cart path for the full-fidelity mode. `TraceEntry::format()`
   ([`crates/coco-core/src/debug.rs:170-175`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/debug.rs#L170-L175)) emits one line per
   instruction: `PC: A=.. B=.. X=.... Y=.... U=.... S=.... DP=.. CC=..` —
   registers *before* the instruction at that `PC` executes, the
   convention most reference-emulator trace formats share, which is what
   makes a line-for-line diff meaningful in the first place.
3. **Diff.** Plain `diff -u reference.trace mine.trace` (`ch01` §1.7's
   "keep a trace notebook" advice — no special tooling required). The
   first line where the two disagree is the bug: either a wrong register
   value (a flag or ALU bug) or a wrong `PC` (a wrong branch, or a wrong
   cycle count throwing off when an interrupt lands). One failure mode
   doesn't announce itself in this diff at all — per Appendix C's
   "interrupt-entry cycle cost" entry, a cycle-count mismatch can sit
   latent while registers and `PC` still agree line for line, and only
   surfaces once it moves an interrupt boundary, thousands of instructions
   downstream of the actual bug.
4. **Narrow it.** Once you have a divergent `PC`, the disassembler
   (`ch03`) tells you which instruction it was; the CPU test suite
   (`cargo test -p mc6809`) is where you write the regression once you
   understand the bug, following `ch01` §1.7's "read the tests, run them,
   break something on purpose, then extend" loop.

For live, in-session debugging rather than an offline trace file, the
codebase also keeps a standing 1024-entry ring buffer
(`DEFAULT_TRACE_CAP`, [`crates/coco-core/src/debug.rs:19`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/debug.rs#L19)) inside the
`Debugger` type — the same `TraceEntry` format, always populated while a
run is in flight, so a breakpoint hit can show you the instructions
leading up to it even when no file trace was started in advance.

### The debugger as a tool

One paragraph, since the debugger's design is the subject of its own
material ([`book/README.md`](https://github.com/sperano/cocovm/blob/main/book/README.md)'s Part VI, "the debugger and save states"):
the piece worth knowing as you reach for it in day-to-day debugging is
`peek()` ([`crates/coco-core/src/bus/peek.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/peek.rs)) — a side-effect-free twin of
`Bus::read()` that the debugger's memory and register views use so that
*looking at* the machine's state can never itself change that state (no
accidentally acknowledging a pending PIA interrupt just by hovering the
memory viewer over `$FF02`). Everything else — breakpoints, watchpoints,
the trace ring above, register/disassembly/memory panels — is built on top
of that one guarantee.

### `cargo test` patterns, per crate

Three crates, two different test-organization styles:

- **`mc6809`** — `crates/mc6809/tests/`: [`alu.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/alu.rs), [`branches.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/branches.rs),
  [`disasm.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/disasm.rs), [`disasm_indexed.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/disasm_indexed.rs), [`indexed.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/indexed.rs), [`interrupts.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/interrupts.rs),
  [`loads.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/loads.rs), [`logic_rmw.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/logic_rmw.rs), [`stack.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/stack.rs), [`wide.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/wide.rs), plus `common/` (shared
  test harness) and a `disasm/` subdirectory. `cargo test -p mc6809` runs
  all of it, zero ROMs needed — every one of these is a synthetic-code
  test against a `FlatBus` (`ch01` §1.3).
- **`coco-core`** — `crates/coco-core/tests/`: roughly 45 files, including
  subdirectories `fdc/`, `snapshot_engine/`, `coco2_boot/`,
  `render_coco12/`, `ssc/`, and `fixtures/` for shared test assets.
  `cargo test -p coco-core` runs the whole suite; `cargo test -p coco-core
  --test <file-stem>` (e.g. `--test bus_map`, `--test scanline_split`)
  isolates one file — the fastest way to re-run just the subsystem you're
  actively changing. A handful of these ([`coco1_boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/coco1_boot.rs), [`boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/boot.rs), and
  anything importing a real ROM) need `roms/` and are written to fail
  loudly or skip when it's absent, per each chapter's stated policy
  (`ch01` §1.7).
- **`coco-egui`** — no top-level `tests/` directory at all; instead, tests
  live as sibling `*_test.rs` modules inside `src/` next to the code they
  cover ([`ui_tests.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/ui_tests.rs)/`ui_tests/`, [`save_state_test.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/save_state_test.rs),
  [`debugger_test.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/debugger_test.rs), [`manager_test.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager_test.rs), [`startup_test.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/startup_test.rs), and others —
  seventeen such files in this worktree). `cargo test -p coco-egui` runs
  all of them, including the kittest-based headless UI tests
  (`ch01`/README's Part VI). Worth noticing as a genuine style difference
  between crates, not an inconsistency to "fix": `coco-egui`'s tests are
  UI-adjacent enough that keeping them next to the widget/panel code they
  exercise reads better than a separate `tests/` tree would.

### `rom_db`'s CRC check as the first diagnostic

When emulated software misbehaves — garbled boot, wrong colors, a crash
partway through — the cheapest question to answer first, before suspecting
the emulator at all, is "is your ROM dump good?"
`crates/coco-core/src/rom_db.rs`'s module comment states the intent:
"Validation is advisory: an unrecognized or mismatching image still boots
(patched and homebrew ROMs are legitimate), but the loader can tell the
user exactly which known dump they have — or that they don't have one." `identify()`/`validate()` check a
loaded image's size and CRC32 (a from-scratch reflected-polynomial
implementation, [`rom_db.rs:53-67`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/rom_db.rs#L53-L67) — not a crate dependency) against
`SYSTEM_ROMS`, a ten-entry manifest copied directly from MAME's own ROM
definitions (`coco3.cpp`, `coco12.cpp`, `coco_fdc.cpp`), and report one of
three outcomes: `Verified` (byte-identical to a known-good dump),
`Mismatch` (the file name matches a known ROM but the bytes don't — a
corrupt dump, almost always), or `Unknown` (not in the manifest at all —
could be homebrew, could be a renamed/patched image, could be the wrong
file entirely).

The integration test that exercises this against whatever you actually
have locally, [`crates/coco-core/tests/rom_db_local.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/rom_db_local.rs), is itself a model
of graceful degradation — it skips any manifest entry whose file isn't
present rather than failing on it, and only fails hard on a file that
*is* present but doesn't match. Run in this worktree, which has no
`roms/` directory at all:

```
$ cargo test -p coco-core --test rom_db_local -- --nocapture
running 1 test
verified 0 of 10 known ROMs present locally
test local_roms_match_manifest ... ok
```

— a clean pass, correctly reporting zero ROMs found, not a failure. The
practical habit this is meant to support: the moment `coco3.rom` or
`disk11.rom` starts behaving oddly, run this test with `--nocapture` before
reaching for the debugger. A `Mismatch` result means you're chasing a
corrupt or mispatched dump, not an emulator bug — and it's a five-second
check against weeks of the wrong kind of debugging.
