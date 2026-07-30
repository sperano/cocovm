# Chapter 9 — Splits, Composite, and PMODEs

*Week 9. Goal: the effects that made demos possible, and the emulation-policy
questions they force. Chapters 7–8 gave you a raster, VDG text, and the GIME's
native text/graphics modes rendered from a register snapshot taken once per
field. This week removes that simplification in two directions at once: the
register snapshot turns out to be a lie (real software rewrites those
registers **while the beam is still scanning**, and the picture must show
both halves), and the "one renderer" story is a lie too — the same 6-bit
palette value paints a different color depending on which cable is plugged
into the back of the machine. Both lies have famous symptoms on
real hardware: the two-tone game screen where the top third and the bottom
two-thirds clearly came from different POKEs, and the muddy-brown mess a
composite TV made of colors that looked crisp on an RGB monitor. By the end
of this chapter you can point at the exact struct and the exact test that
explain each.*

---

Chapters 7 and 8 were about geometry. Given a register file frozen at one
instant, where does each byte of video RAM land on the screen, and what
color does it come out? That question has a clean answer, and the last two
chapters gave it: a decode function per mode, a fetch address per row, a
palette lookup per pixel. Every one of those answers is still correct. What
was quietly missing is that neither the register file nor the palette lookup
is as fixed as the previous chapters made them look.

Both simplifications were deliberate, and both are the kind that make a
first pass tractable and a second pass necessary. The first is temporal: the
GIME's registers are not a snapshot but a signal that changes while the
picture is being drawn, and a 6809 program with an interrupt handler can
change them at a *chosen* scanline. The second is physical: the six bits in
a palette register are not a color but a number, and two entirely
different pieces of analog hardware turn that number into two entirely
different colors, with nothing in the machine recording which one is
attached.

This chapter takes them in that order — physical first, temporal second —
because the color half is self-contained and short, and it sets up a habit
of mind the split half needs: separating *what the register says* from *what
the hardware downstream of the register does with it*. Sections 9.1 through
9.7 are the monitor story, and end with a claim worth stating up front so
you can watch it be defended: composite output is not a second renderer.
Sections 9.8 through 9.11 are the raster-split story, ending in a test where
hand-assembled 6809 code arms a hardware timer, takes a FIRQ, and paints two
different border colors into one field with no help at all from the test
harness. Sections 9.12 through 9.15 then step back to the CoCo 1/2's own
graphics modes, the `PMODE`s a generation of BASIC programmers typed
without ever being told what the numbers meant — plus one honest accounting
of a famous effect this codebase does *not* reproduce.

There is one new device to understand this week (the GIME's interval timer,
which arrives in §9.10) and no new renderer at all. Everything else is
reading code that already exists, and finding out that the simplifications
were load-bearing.

---

## 9.1 Two pictures from one register file

Start with the question that decides how much work the composite path is
going to take, because the answer shapes every section after it: is
composite output a different *renderer*, or a different *interpretation of
the same render*? If it were a different renderer, this chapter would be
twice as long and the codebase would have two of everything — two text
painters, two graphics painters, two sets of tests.

Here is the claim this chapter defends: **composite vs. RGB is not a second
renderer.** There is exactly one code path that walks video RAM and produces
pixels — the `paint_scanline`/`paint_text_row`/`paint_graphics_row` functions
you read in Chapter 8, untouched since. What changes between an RGB monitor and
a composite one is a single function call at the very last step, after every
geometric decision (which byte, which bit, which palette register) has
already been made: turning a 6-bit register *value* into an RGBA pixel. That
function is `GIME::color`, and this chapter is largely about the two
different things it can do with the same six bits.

That's also why the real GIME needed no mode switch for this at all. The
chip drives an RGB output pin *and* a composite output pin simultaneously,
all the time — it's the monitor cable, not a GIME register, that decides
which signal the phosphors respond to. `MonitorType` in this codebase is
purely an emulator/UI choice ([`crates/coco-core/src/gime/palette.rs:8-17`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs#L8-L17))
threaded in through `MachineConfig`, not something CoCo software can read or
set:

```rust
/// Which monitor signal path resolves 6-bit palette values to RGB: the real
/// GIME drives both an RGB and a composite output simultaneously, and it's
/// the monitor cable — not a GIME register — that decides which one matters.
/// Emulator config/UI choice; default matches prior (RGB-only) behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MonitorType {
    #[default]
    RGB,
    Composite,
}
```

Take the doc comment as the design decision it is. Almost everything else in
`gime.rs` models a real register with a real address, and the module is
scrupulous about saying so — `$FF98`, `$FF9A`, `$FFB0`–`$FFBF` all appear in
their fields' doc comments. This type has no address, because on the real
machine there is nothing to address. The choice it represents was made once,
physically, by whoever ran a cable from the back of the computer, and the
chip on the other end of that cable never learned the outcome.

That has a consequence worth holding on to for the rest of the chapter: a
CoCo 3 program cannot branch on it. There is no "am I on composite?" call to
make, no status bit to poll, no `PEEK` that answers the question. A program
that wanted to look right on both had to either pick colors that survived
both decodes or ship two color schemes and ask the user which one to load —
and §9.7 comes back to what that meant in practice.

Because the choice belongs to the machine's configuration rather than to its
registers, it lives in `MachineConfig` alongside the other things a person
decides before the machine boots ([`crates/coco-core/src/config.rs:165-171`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/config.rs#L165-L171)):

```rust
    /// Which monitor cable is plugged in (RGB vs composite decode of the
    /// GIME's 6-bit palette values). Not a hardware register — see
    /// [`MonitorType`]. `None` on machines with no monitor port at all:
    /// a stock CoCo 1/2's only video output is the RF modulator into a TV
    /// (RGB and composite ports are CoCo 3 additions), enforced by
    /// [`Self::validate`].
    pub monitor: Option<MonitorType>,
```

The `Option` is not defensive programming, and it is not "we might not have
decided yet." It encodes a fact about three different machines: a CoCo 3 has
both an RGB port and a composite port, so exactly one of two answers applies
to it; a stock CoCo 1 or CoCo 2 has neither, only an RF modulator feeding a
television, so *no* answer applies. `None` means "this machine has no
monitor port to have an opinion about," which is a genuinely different
statement from "RGB," and `MachineConfig::validate` rejects configurations
that mix them up. The CoCo 1/2 chapters lean on this; the GIME itself never
finds out either way.

> **Rust corner: `Option<T>` as "this thing does not exist here."** It is
> tempting to read every `Option` as nullability — a value that might be
> missing because someone forgot to set it. That reading makes code worse.
> The useful reading is that `Option<T>` models a *domain* in which the
> question sometimes has no answer at all, and `None` is the answer.
>
> `MachineConfig` uses this twice, side by side, for exactly the same reason.
> `monitor: Option<MonitorType>` is `None` on a CoCo 1/2 because those
> machines have no monitor port. The very next field, `vdg:
> Option<VDGVariant>`, is `None` on a CoCo 3 because that machine has no
> MC6847 chip — the GIME does its own character generation, as Chapter 7
> established. Two variants, two absences, each one making the *other*
> machine's mandatory field meaningless.
>
> The payoff is that the type carries the constraint into every function
> signature that touches it. Code reading `config.monitor` cannot silently
> assume a CoCo 1 has an RGB monitor, because the compiler will not let it
> pretend a `None` is an `RGB`. Compare the alternative — a plain
> `MonitorType` field with a comment saying "ignored on CoCo 1/2" — where
> the constraint exists only in prose and the first person to forget it
> ships a bug that renders fine and means nothing.

With the switch itself pinned down as configuration rather than hardware,
the interesting question becomes what the two branches actually *do*. The
next two sections take them one at a time, and the contrast between them is
the whole point: one is four lines of arithmetic, the other is sixty-four
hand-measured constants, and the reason for the difference is physics.

## 9.2 The RGB decode: a formula

Take the easy half first, because it sets up the contrast. Each GIME palette
register is 6 bits, laid out `RGBrgb` — a high bit and a low bit per
channel, giving four intensity levels (`0, 1, 2, 3`) per channel. RGB output
is a direct, arithmetic unpack — no lookup table, no hardware quirks, only
bit-picking and a fixed scale ([`palette.rs:56-62`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs#L56-L62)):

```rust
/// Convert a 6-bit GIME palette value to RGBA. The register format is
/// `RGBrgb` (two bits per channel); each channel scales `0..3` to `0..0xFF`
/// via `×0x55` — matching the GIME's RGB output (MAME `gime.cpp`).
pub fn rgb_color(value: u8) -> [u8; 4] {
    let chan = |hi_bit: u8, lo_bit: u8| {
        let v = ((value >> hi_bit) & 1) << 1 | ((value >> lo_bit) & 1);
        v * 0x55
    };
    [chan(5, 2), chan(4, 1), chan(3, 0), 0xFF]
}
```

Read the bit assignment carefully — it's not "high nibble, low nibble." Red
takes bits 5 and 2, green takes bits 4 and 1, blue takes bits 3 and 0: the
register is `R1 G1 B1 R0 G0 B0`, interleaved. `chan` reassembles the two bits
for one channel into a 2-bit value (`0..3`) and multiplies by `0x55`
(`0x55 × 3 = 0xFF`, so the four levels land exactly on `0x00/0x55/0xAA/0xFF`
— evenly spaced, no rounding error). This is Chapter 8's `RGBrgb` walkthrough
in executable form; the worked example there built SEB Unravelled II's
"purple" as decimal 43 and got `(0xAA, 0x55, 0xFF)` back out of exactly this
function.

The `×0x55` deserves one moment of attention, because it is the kind of
constant that looks arbitrary and is not. Four evenly spaced levels have to
span `0x00` to `0xFF` inclusive: the bottom level must be fully off and the
top level must be fully on, or the emulator's white is not white. The only
multiplier that does that with integer arithmetic is `0xFF / 3 = 0x55`.
Consider the two obvious alternatives and what they cost. A shift, `v << 6`,
produces `0x00/0x40/0x80/0xC0` — cheap, but the machine can then never
display white, only a slightly grubby light gray. Scaling by `0x50` produces
`0x00/0x50/0xA0/0xF0`, which is worse in a subtler way: it is *almost* right
everywhere, so nothing looks obviously broken and every screenshot compared
against a reference is off by a few counts in every channel. The exact
constant is what lets a bit-exact comparison against another emulator mean
something, which is Chapter 4's testing philosophy showing up in a color
routine.

That's pure digital-to-analog arithmetic, and it matches the physical
situation exactly. An RGB monitor has three separate electron guns, one per
channel, and three separate wires to drive them. The GIME's only job is to
put a voltage on each wire proportional to two bits, and the monitor's only
job is to believe it. There is nothing to measure, no interaction between
channels, and no way for the value of one pixel to affect the appearance of
the next — which is exactly why this is a four-line function instead of a
64-entry table, and why the whole RGB path fits in one paragraph of
explanation.

> **Rust corner: a closure as a local, named subroutine.** `chan` is a
> closure bound to a `let`, called three times, and never escaping the
> function. It exists for the same reason a private helper method would,
> minus the ceremony: the two-bits-to-one-channel computation is written
> once and applied to three different bit pairs.
>
> The detail that makes it worth writing as a closure rather than a nested
> `fn` is the capture. `value` never appears in `chan`'s parameter list —
> the closure reaches out and borrows it from the enclosing scope, so the
> call sites read `chan(5, 2)` instead of `chan(value, 5, 2)`. A nested
> `fn` cannot do that; Rust's plain functions capture nothing, so `value`
> would have to be threaded through by hand at all three call sites. When
> the captured thing is the *subject* of the computation and the parameters
> are the *variation*, a closure keeps the call sites saying only what
> varies.
>
> Notice one more thing about the signature: `rgb_color` is an associated
> function, not a method. There is no `&self`, because nothing about
> converting `RGBrgb` to RGB depends on any GIME state. That is why the
> tests in this chapter call it as `GIME::rgb_color(0x09)` with no GIME
> anywhere in sight, and it is the cleanest possible statement of "this
> half of the color path is a pure function." The composite half, as
> you're about to see, cannot make that claim: it has to consult two mode
> bits, so its entry point takes `&self`.

So much for the half that reduces to arithmetic. The other half of
`GIME::color` starts from an entirely different physical arrangement — one
wire instead of three — and it does not reduce to arithmetic at all.

## 9.3 The composite decode: a table, because there is no formula

Composite video has no separate channels. Luminance and color are
multiplexed onto a single carrier wave — color rides as the phase and
amplitude of a 3.58 MHz subcarrier added on top of the brightness signal —
and a real TV's tuner has to demodulate that carrier back into something a
phosphor can use. That subcarrier frequency is not a stray number: Chapter 1
pulled the machine's whole clock tree out of it, since the 28.636363 MHz
crystal that the CPU clock and every video timing constant descend from was
chosen as a multiple of the NTSC color subcarrier in the first place. The
same frequency that dictates how fast the 6809 runs is the one the color
information rides on.

Demodulating it is an analog, lossy process that depends on exact component
tolerances in both the GIME's video DAC and the TV's decoder circuit. You
cannot derive it from the `RGBrgb` bit layout with arithmetic; there is no
clean function from "6-bit register value" to "NTSC composite color" the
way `rgb_color` is a clean function from "6-bit register value" to RGB
voltage. So the codebase doesn't try. It ships the actual measured result,
ripped from MAME's own hand-calibrated table ([`palette.rs:19-44`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs#L19-L44),
comment preserved verbatim because the provenance matters):

```rust
/// Composite-monitor palette (BPI=0), 64 entries indexed by the 6-bit GIME
/// palette value, `0xRRGGBB`. Hand-measured on real hardware — there is no
/// formula. Verbatim from MAME `src/mame/trs/gime.cpp` `get_composite_color`
/// (BSD-3-Clause, Nathan Woods; see NOTICE.md).
const COMPOSITE_PALETTE: [u32; 64] = [
    0x000000, 0x004c00, 0x004300, 0x0a3100, 0x2f1b00, 0x550100, 0x6c0000, 0x770006, 0x71004b,
    0x5c008b, 0x3b00b8, 0x1100ca, 0x001499, 0x002c62, 0x004011, 0x004b00, 0x2d2d2d, 0x069800,
    0x288f00, 0x537d00, 0x786700, 0xa04c00, 0xb63402, 0xc3224c, 0xbd1693, 0xa814d5, 0x881cfe,
    0x5e2cff, 0x105ee9, 0x0076b2, 0x008b60, 0x009618, 0x747474, 0x41d714, 0x62cf00, 0x8ebd00,
    0xb4a700, 0xdd8c01, 0xf5733a, 0xfe6085, 0xfd53ce, 0xe950ff, 0xc958ff, 0x9e67ff, 0x4e9aff,
    0x36b3f7, 0x26c9a3, 0x2bd558, 0xfdfdfe, 0x88e85a, 0xa1e03f, 0xbed238, 0xd8c342, 0xf1b161,
    0xfea08d, 0xfe95bf, 0xfd8ef1, 0xef8eff, 0xd895ff, 0xb9a1ff, 0x86c4ff, 0x78d4f2, 0x71e2b6,
    0xffffff,
];
```

Four sentences of doc comment, and every one of them is doing work. "Hand-
measured on real hardware" is the justification for the table existing at
all. "There is no formula" is a claim strong enough that it needs the
sentence before it as evidence. "Verbatim from MAME" is a provenance
statement that makes the numbers auditable — someone can diff them against
the upstream source and find out whether they drifted. And the parenthetical
names a licence and points at a file, which matters practically: the project
[`NOTICE.md`](https://github.com/sperano/cocovm/blob/main/NOTICE.md) records
that both composite tables were copied from MAME's `gime.cpp`, that the
upstream file is BSD-3-Clause rather than GPL, and that BSD-3-Clause is
compatible with this crate's GPL licensing so long as the attribution is
carried. Copying sixty-four constants out of another project is a licensing
event, not just an engineering one, and the codebase treats it as one.

There is a broader methodological point here that Appendix B states in
general terms and this table demonstrates concretely. When a physical
process genuinely has no closed form worth deriving, the correct move is not
to approximate it with a formula that will be subtly wrong everywhere. It is
to obtain a measurement known to be correct and inherit it verbatim,
including its imperfections — which is why §9.6's regression test has to
tolerate a one-count blue tint rather than pretending the data is cleaner
than it is.

The entries themselves are `0xRRGGBB` packed 32-bit words, unpacked by
shifting ([`palette.rs:48-50`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs#L48-L50)):

```rust
/// Unpack an `0xRRGGBB` composite-table entry into RGBA, matching
/// [`GIME::rgb_color`]'s return convention (opaque alpha).
fn unpack_rgb(v: u32) -> [u8; 4] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8, 0xFF]
}
```

Three shifts and a hardcoded alpha. The doc comment's "matching
[`GIME::rgb_color`]'s return convention" is the load-bearing part: both
halves of the color path must hand back the same shape, `[u8; 4]` with an
opaque alpha, or the caller would have to know which branch it took — and
the entire claim of §9.1 is that no caller knows.

> **Rust corner: packed hex literals as a compact data table.** `0x004c00`
> is a `u32` in the source, but nobody ever does arithmetic on it as a
> number — it's three bytes wearing a trenchcoat. `unpack_rgb` peels them
> back apart with shifts and an `as u8` truncation (`v >> 16` keeps only the
> byte that matters once cast down). This is a common trick for hand-written
> constant tables: one line per entry, red/green/blue visually grouped in
> pairs of hex digits, at the cost of needing a tiny unpack function instead
> of just indexing a `[[u8; 3]; 64]` array directly. You'll see the same
> `0xRRGGBB` packing anywhere a codebase ports a color table from a C
> source (MAME's own `rgb_t` does exactly this) rather than reshaping it.
>
> The `as u8` casts deserve a note of their own, because `as` is the one
> conversion in Rust that will silently discard information. `v as u8` on a
> `u32` keeps the low eight bits and throws the rest away, which is a bug in
> most code and precisely the intent here: after `v >> 16`, the low eight
> bits are the red channel and the upper bits are noise from the green and
> blue fields. This is the narrow case where truncation *is* the operation,
> and it is worth being able to tell that case apart on sight from the far
> more common one where an `as` cast is quietly losing data nobody meant to
> lose.

`GIME::color` is the dispatcher — the *only* place `MonitorType` is ever
consulted ([`palette.rs:69-87`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs#L69-L87)):

```rust
/// Resolve a 6-bit GIME palette value to RGBA through the currently
/// selected monitor path (`self.monitor`). RGB mode is [`Self::rgb_color`]
/// unconditionally; composite mode picks [`COMPOSITE_PALETTE`] or
/// [`COMPOSITE_PALETTE_180`] per $FF98 BPI, then averages channels to
/// grey when $FF98 MOCH is set (MAME `gime.cpp` `update_composite`).
pub fn color(&self, value: u8) -> [u8; 4] {
    match self.monitor {
        MonitorType::RGB => Self::rgb_color(value),
        MonitorType::Composite => {
            let table = if self.vmode & vmode::BPI != 0 {
                &COMPOSITE_PALETTE_180
            } else {
                &COMPOSITE_PALETTE
            };
            let [r, g, b, a] = unpack_rgb(table[value as usize & 0x3F]);
            if self.vmode & vmode::MOCH != 0 {
                let avg = ((r as u16 + g as u16 + b as u16) / 3) as u8;
                [avg, avg, avg, a]
            } else {
                [r, g, b, a]
            }
        }
    }
}
```

Three details in nineteen lines are worth naming before moving on. First,
the signature takes `&self` and not `&mut self` — resolving a color reads
GIME state but changes nothing, so unlike Chapter 1's `Bus::read`, this really
is a pure function of the register file. That matters more than it sounds:
it means `resolve_colors` can call it sixteen times per scanline (Chapter 8's
per-line palette resolve) while the renderer holds only a shared borrow of
the GIME, which is exactly the borrow arrangement Chapter 1's §1.4 arrived at
for `paint_scanline`.

Second, `value as usize & 0x3F` masks to six bits before indexing. The
callers all pass genuine 6-bit palette values, so the mask never changes an
answer; what it changes is the failure mode. Without it, a future caller
passing a full byte would index past the end of a 64-entry array and panic
at run time. With it, the array access is provably in bounds for every
possible `u8`, which is a small, permanent guarantee bought for one `AND`.

Third — and this is the structural point the rest of the chapter rests on —
the RGB arm returns immediately, before `vmode` is read at all. Neither
`BPI` nor `MOCH` can affect an RGB monitor, not because someone wrote a rule
saying so, but because control flow never reaches the code that consults
them. §9.5's `rgb_monitor_ignores_bpi_and_moch` is a test of exactly this
`match` arm.

Every other renderer in this codebase — `paint_text_row`, `paint_graphics_row`,
the legacy VDG text and graphics painters you'll meet later in this chapter —
calls `g.color(register_value)` and never asks which branch it took. That's
the "not a different renderer" claim made concrete: swap `MonitorType` and
every pixel on screen can change without one line of the scanout code
running differently.

### Reading the table: a hue wheel, not a color wheel

Sixty-four hand-measured constants look, at first glance, like sixty-four
independent facts — the sort of data you can verify but not understand. They
aren't. The table has a structure, and finding it is what turns "a magic
array copied from MAME" into something you can reason about, predict, and
write meaningful tests against.

Look at the four indices `0x00`, `0x10`, `0x20`, `0x30` in `COMPOSITE_PALETTE`
— they land at array offsets 0, 16, 32, 48, i.e. every 16th entry starting
from zero:

```
0x00 -> 0x000000   (black)
0x10 -> 0x2d2d2d   (dark grey)
0x20 -> 0x747474   (mid grey)
0x30 -> 0xfdfdfe   (near-white)
```

All four are achromatic — equal (or near-equal) R, G, and B — and they form
a rising brightness ramp. That's not a coincidence baked into four
hand-picked entries; it's the structure of the whole table. Split the 6-bit
value into a low nibble (bits 0–3, 16 values) and a high 2 bits (bits 4–5, 4
values): the low nibble selects a **hue**, and the high bits select a
**luminance level**. Hue `0` is defined as "no color," so all four
luminance steps at hue `0` are gray — which is exactly the `0x00/0x10/0x20/0x30`
family above. This matches the real GIME/CoCo palette-register convention
documented in SEB Unravelled II, and it's directly testable:
`composite_decode_grey_anchors` ([`tests/composite.rs:16-26`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/composite.rs#L16-L26))
is precisely these four values:

```rust
#[test]
fn composite_decode_grey_anchors() {
    let g = GIME {
        monitor: MonitorType::Composite,
        ..GIME::new()
    };
    assert_eq!(g.color(0x00), [0x00, 0x00, 0x00, 0xFF]);
    assert_eq!(g.color(0x10), [0x2d, 0x2d, 0x2d, 0xFF]);
    assert_eq!(g.color(0x20), [0x74, 0x74, 0x74, 0xFF]);
    assert_eq!(g.color(0x30), [0xfd, 0xfd, 0xfe, 0xFF]);
}
```

Two things about the construction of that test generalize beyond this file.
The GIME is built with struct-update syntax — `monitor: MonitorType::Composite,
..GIME::new()` — which sets exactly the one field under test and leaves
every other register at its reset value, so the test's name is an honest
description of its only variable. And because `color` takes `&self`, the
binding doesn't even need to be `mut`; the tests that do need `mut` in this
file (§9.4's and §9.5's) need it because they poke `vmode`, not because
resolving a color changes anything.

Walk one non-zero hue across its four luminance steps and the same pattern
holds — hue `1` (green-family) at increasing brightness:

```
0x01 -> 0x004c00   (0,   76,  0)    dark green
0x11 -> 0x069800   (6,   152, 0)    brighter green
0x21 -> 0x41d714   (65,  215, 20)   bright green
0x31 -> 0x88e85a   (136, 232, 90)   pale green
```

Brightness rises cleanly with each step, and green stays dominant
throughout — but the ramp isn't a pure "same hue, more luminance" scale
either: the top step picks up a real amount of red (`0x88` out of `0xE8` of
green), pulling it slightly warmer and paler than a mechanical multiply-by-
luminance would produce. That's exactly what you'd expect from a *measured*
NTSC recording rather than a computed HSV ramp. A formula-generated table
wouldn't have that kind of drift; a hand-calibrated one does, because the
real hardware has it.

That drift is not noise to be cleaned up, and the temptation to clean it up
is the trap this section exists to warn against. A tidier table would be a
*less accurate* table, and it would break for exactly the software that
depended on the untidiness — which is precisely the war story §9.6 tells.
First, though, there is a second complete table to account for, selected by
a bit that has appeared twice already without explanation.

## 9.4 BPI: burst-phase invert, and a correction

`COMPOSITE_PALETTE_180` is a second, complete 64-entry table, selected when
`$FF98` bit 5 (`vmode::BPI`) is set:

```rust
/// Burst phase invert (alternate composite colour set).
pub const BPI: u8 = 0x20;
```

Seven words of doc comment for a bit that doubles the size of this chapter's
data. Expand them. "Burst phase" is the reference signal a composite decoder
locks onto to know what phase angle corresponds to what hue — the color
information rides as a phase offset, so a decoder needs an agreed zero point
to measure offsets against, and the burst is that agreement transmitted once
per scanline. Inverting it moves the agreed zero point by half a turn, which
should, in principle, rotate every hue by half the color wheel: 180°, not
some smaller angle.

Because the rotation is not derivable from the un-inverted table by
arithmetic — same reason as §9.3, the decode is analog — the codebase ships
the inverted case as its own complete measurement
([`palette.rs:33-44`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs#L33-L44)):

```rust
/// Composite-monitor palette with $FF98 BPI (burst phase invert) set.
/// Verbatim from MAME `gime.cpp` `get_composite_color`.
const COMPOSITE_PALETTE_180: [u32; 64] = [
    0x000000, 0x5a0e5a, 0x4f0c4f, 0x360f40, 0x0d213c, 0x003334, 0x004141, 0x004943, 0x005409,
    0x005600, 0x114c00, 0x263700, 0x392500, 0x491d00, 0x4f0f3e, 0x590e59, 0x2d2d2d, 0xb11fb7,
    0x9932c1, 0x7248c5, 0x4a5bc2, 0x1a6eba, 0x0077a9, 0x008c62, 0x009619, 0x039700, 0x238f00,
    0x467800, 0x9c4e00, 0xb23c00, 0xb92e59, 0xb6209e, 0x747474, 0xe852ff, 0xcd60ff, 0xa677ff,
    0x7d8aff, 0x4d9eff, 0x32b4ed, 0x29c7a2, 0x2ad459, 0x39d223, 0x50c11a, 0x72a911, 0xcf831e,
    0xf47733, 0xff5f85, 0xfe54d1, 0xfdfdfc, 0xef8fff, 0xd697ff, 0xb8a4ff, 0x9eb3ff, 0x86c6ff,
    0x76d4e7, 0x74ddb3, 0x77e683, 0x80e170, 0x92d56b, 0xacc466, 0xeaac71, 0xffa385, 0xff95c1,
    0xffffff,
];
```

Compare the two tables at the four gray anchors before going near the
colors, because it's the fastest sanity check available. Three of the four
are byte-identical between the tables — entries `0`, `16`, and `32`:
`0x000000`, `0x2d2d2d`, `0x747474` — as is entry `63`, `0xffffff`, the last
value in both. That is exactly what "rotate the hue" predicts — rotating a
color with no saturation does nothing, so the achromatic column must
survive the inversion untouched. The fourth anchor, entry 48, differs by a
single count (`0xfdfdfe` versus `0xfdfdfc`), which is measurement noise on
a near-white, not a hue rotation. The structure holds where structure is
predictable and wobbles where measurement wobbles.

Now the colors, and a number this chapter needs to get right. Precision
matters here, because an earlier pass at this material claimed "roughly a
120° hue shift" for palette value `0x01` under BPI. The actual test data
doesn't support that number. `composite_decode_hue_and_bpi`
([`tests/composite.rs:28-38`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/composite.rs#L28-L38)):

```rust
#[test]
fn composite_decode_hue_and_bpi() {
    let mut g = GIME {
        monitor: MonitorType::Composite,
        ..GIME::new()
    };
    assert_eq!(g.color(0x01), [0x00, 0x4c, 0x00, 0xFF]);

    g.vmode = vmode::BPI;
    assert_eq!(g.color(0x01), [0x5a, 0x0e, 0x5a, 0xFF]);
}
```

Checking a hue rotation by eye is unreliable, so it's worth writing down the
arithmetic once — exercise 9.2 asks for it, and the rest of this section
uses it. Take a color's channels as fractions, let `max` and `min` be the
largest and smallest of the three, and let `delta = max − min`. The hue in
degrees is `60 × ((G − B) / delta)` when red is the largest channel,
`60 × (2 + (B − R) / delta)` when green is, and `60 × (4 + (R − G) / delta)`
when blue is, taken modulo 360. Pure red is 0°, pure green 120°, pure blue
240°, and magenta — red and blue equal, green absent — sits at 300°.

Apply it. `0x01` un-inverted is `(0, 76, 0)` — pure green, hue exactly 120°.
Inverted it's `(90, 14, 90)`: R and B tied and both far above G, which is
the textbook signature of magenta, hue exactly 300°. `300 − 120 = 180`.
Check a second, darker entry the same way: `0x03` un-inverted is
`(10, 49, 0)`, green-dominant, so `60 × (2 + (0 − 10)/49) ≈ 108°`, a
yellow-green. Inverted (`COMPOSITE_PALETTE_180[3]`) it's `(54, 15, 64)`,
blue-dominant, so `60 × (4 + (54 − 15)/49) ≈ 288°`, a violet — again a 180°
delta, from a completely different starting hue. **The claim should read
"~180°," not "~120°"** — and that number isn't a coincidence, it's the
register's own name: BPI *inverts* the phase, which is a half-turn by
construction, not a third-turn.

Then check a more saturated entry, and the clean story stops being clean.
`0x0A` un-inverted is `(59, 0, 184)`, blue-dominant, hue `60 × (4 + 59/184)
≈ 259°` — a blue-violet. Inverted, `COMPOSITE_PALETTE_180[10]` is
`(17, 76, 0)`, green-dominant, hue `60 × (2 − 17/76) ≈ 107°` — a green.
That's a delta of about 207°, not 180°, and no amount of squinting turns it
into a half-turn.

That is the hand-measured table telling the truth about hand-measured
tables. A real chip's phase response isn't perfectly linear across its whole
brightness and saturation range; the amplitude that carries saturation and
the phase that carries hue are not fully independent in an analog encoder,
and the measurement captures whatever the silicon actually did rather than
whatever the block diagram promised. So "invert the burst" is the right
mental model for what the bit means, and "exactly 180° for every entry" is
not something you can rely on pixel-by-pixel. Trust the table, not a
formula, exactly as the source comment insists — and notice that this is the
second time in two sections that the correct engineering response to
measured data has been to describe its structure honestly rather than to
tidy it into a rule.

One bit of `$FF98` down. The other composite-only bit does something
structurally different: it doesn't choose a table at all.

## 9.5 MOCH: averaging to gray

The last bit `GIME::color` consults is `vmode::MOCH` (`$FF98` bit 4) —
monochrome-on-composite, for driving a green-screen or B&W composite
monitor. Monochrome monitors were common and cheap in the period, and a
color signal shown on one is not automatically an improvement: hues that
are clearly distinct in color can carry nearly identical brightness, so a
carefully color-coded screen can collapse into an unreadable smear of one
gray. A bit that tells the video hardware "this display has no chroma, throw
the color away deliberately" is a text-legibility feature, not an
aesthetic one.

Whatever it does on real silicon, in this codebase it doesn't touch the
lookup at all; it post-processes whatever color the table produced,
averaging the three channels:

```rust
if self.vmode & vmode::MOCH != 0 {
    let avg = ((r as u16 + g as u16 + b as u16) / 3) as u8;
    [avg, avg, avg, a]
} else {
    [r, g, b, a]
}
```

The order matters and is easy to miss: `BPI` is consulted first and picks a
table, then the entry is unpacked, and only then is `MOCH` applied to the
result. So the two bits compose rather than conflict — with both set, the
value is looked up in the inverted table and *then* flattened to gray. The
widths matter too. Each channel is promoted to `u16` before the addition,
because three `u8` channels can sum past 255 and `u8 + u8` would overflow;
this is the same instinct as Chapter 1's `wrapping_add` house rule applied in
the opposite direction, where the fix is a wider type rather than a defined
wrap.

The averaging is an unweighted mean, one third each, not a perceptual
luminance weighting — the source comment traces it to MAME's
`update_composite`, and matching the reference implementation is the whole
requirement. `composite_moch_averages_channels`
([`tests/composite.rs:40-50`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/composite.rs#L40-L50)) nails down
the exact arithmetic, including the truncation, because "average" is
ambiguous until you specify the rounding:

```rust
#[test]
fn composite_moch_averages_channels() {
    // Normal-table index 0x01 = 0x004c00 -> r=0x00, g=0x4c (76), b=0x00.
    // (0 + 76 + 0) / 3 = 25 (integer division) = 0x19.
    let mut g = GIME {
        monitor: MonitorType::Composite,
        ..GIME::new()
    };
    g.vmode = vmode::MOCH;
    assert_eq!(g.color(0x01), [0x19, 0x19, 0x19, 0xFF]);
}
```

`(0 + 76 + 0) / 3` is `25.33...`, and `u16` integer division truncates
rather than rounds. At this input the two agree on `25` (`0x19`); at a
channel sum of `77` they wouldn't — truncation gives `25` where rounding
gives `26`. The difference is never more than one count, but it's exactly
the kind of off-by-a-hair detail that a bit-exact trace-diff against MAME
(Chapter 4's testing philosophy) would catch and a "looks about right"
implementation would let through silently. Note also what the test chose as
its input: `0x01`, the same entry §9.4 used, whose
un-inverted value has two channels at zero. That makes the expected
arithmetic checkable by hand in the comment above the assertion, which is
worth more than picking a "realistic" color whose expected value nobody can
verify without running the code.

`MonitorType::RGB` never looks at `MOCH` or `BPI` at all, and there is a
test that says so out loud rather than leaving it as an inference from the
`match` — `rgb_monitor_ignores_bpi_and_moch`
([`tests/composite.rs:52-60`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/composite.rs#L52-L60)):

```rust
#[test]
fn rgb_monitor_ignores_bpi_and_moch() {
    let g = GIME {
        monitor: MonitorType::RGB,
        vmode: vmode::BPI | vmode::MOCH,
        ..GIME::new()
    };
    assert_eq!(g.color(0x10), GIME::rgb_color(0x10));
}
```

Both bits set at once, and the assertion is not against a hardcoded triple
but against `GIME::rgb_color(0x10)` — the pure function from §9.2, called
directly. That's the sharpest possible statement of the property: whatever
the RGB formula produces, `color()` on an RGB monitor produces the same
thing, and no combination of composite mode bits can put a gap between them.
If someone later restructured `color()` and accidentally applied the
grayscale step to both arms, this test fails immediately and names the
reason.

Three sections have now taken `GIME::color` apart bit by bit. What has not
yet been shown is why any of it matters to software that actually existed —
and there is one very concrete case where getting this exact structure wrong
would have silently ruined a real, shipping desktop environment.

## 9.6 War story: the NitrOS-9 EOU grayscale regression

Here's where the "no formula, hand-measured, hue-then-luminance" structure
stopped being a curiosity and started mattering. NitrOS-9's `gshell`
desktop, part of the **EOU** ("Ease of Use") package, draws a grayscale
interface — window chrome, shading, the works — using exactly the four
palette values you just met: `0x00`, `0x10`, `0x20`, `0x30`. It's real
software, written by people who owned real composite CoCo 3s, who chose
those four values specifically *because* they're the hardware's own gray
ramp — not because they picked four arbitrary-looking numbers and hoped.

Put yourself in the position of that program. It wants four shades of gray.
It cannot ask what monitor is attached (§9.1), so whatever it picks has to
be gray on both. On an RGB monitor the answer is easy and derivable: any
value with all three channels equal, and `0x00`/`0x10`/`0x20`/`0x30` are not
those values — run them through `rgb_color` and hue-0 luminance steps come
out as black, green, red, and yellow, because the `RGBrgb` bit layout has
no notion of "hue" at all. The four values are gray on *composite*
specifically, because composite is the decode where the low nibble means
hue and hue 0 means no color. Choosing them is a statement about which
monitor the authors expected, and it is the kind of statement
that only makes sense once you know the table's structure.

Anyone running that software through a composite decode that got the
hue-vs-luminance split wrong would see EOU's "neutral gray desktop" rendered
in whatever stray color the wrong decode produced instead — black, green,
red, yellow, something plausible-looking but *wrong*, and wrong in a way
that's easy to miss if you never happen to boot NitrOS-9 with a composite
monitor selected. That is the shape of the nastiest class of emulator bug:
not a crash, not a garbled screen, but a screen that looks entirely
reasonable to anyone who hasn't seen the real thing.

The regression test that guards this is `eou_greyscale_regression`
([`tests/composite.rs:62-85`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/composite.rs#L62-L85)), and it's worth reading end to end because it
encodes exactly the property real software depended on — achromatic *and*
monotonically brighter — not just "matches these four hex triples":

```rust
#[test]
fn eou_greyscale_regression() {
    // NitrOS-9 EOU's gshell greyscale desktop programs palette regs 0/0x10/
    // 0x20/0x30. On a composite monitor these must decode to achromatic,
    // strictly increasing brightness (not black/green/red/yellow as an
    // RGB-only decode would render them).
    let g = GIME {
        monitor: MonitorType::Composite,
        ..GIME::new()
    };
    let regs = [0x00u8, 0x10, 0x20, 0x30];
    let colors: Vec<[u8; 4]> = regs.iter().map(|&r| g.color(r)).collect();
    // The hand-measured table isn't perfectly achromatic at every entry
    // (0x30 = 0xfdfdfe, one LSB off on blue), so allow a 1-count tolerance
    // rather than exact channel equality.
    const GREY_TOLERANCE: u8 = 1;
    for c in &colors {
        assert!(c[0].abs_diff(c[1]) <= GREY_TOLERANCE, "achromatic: r ~= g ({c:?})");
        assert!(c[1].abs_diff(c[2]) <= GREY_TOLERANCE, "achromatic: g ~= b ({c:?})");
    }
    assert!(colors[0][0] < colors[1][0]);
    assert!(colors[1][0] < colors[2][0]);
    assert!(colors[2][0] < colors[3][0]);
}
```

Two things about this test are worth slowing down on, because both are
lessons about testing hardware you didn't build.

First, the comment's parenthetical — `0x30 = 0xfdfdfe`, blue is one count
short of the other two channels — is the test author refusing to pretend the
measured data is cleaner than it is. A stricter test (`c[0] == c[1] &&
c[1] == c[2]`) would be *more* elegant and *less* true. It would either need
to be relaxed the first time someone re-measured the table from a slightly
different real GIME, or it would quietly encourage "fixing" the table to be
exactly gray — which would be fixing it to be wrong, since the real chip's
`0x30` really does carry a one-LSB blue tint. The tolerance is the correct
response to "hardware doesn't round the way your intuition does," and
`GREY_TOLERANCE` being a named constant rather than a bare `1` in two
assertions is the difference between a documented allowance and a
mysterious fudge factor.

Second, notice what the test does *not* assert: it never checks the four
colors are identical to `COMPOSITE_PALETTE[0]`/`[0x10]`/`[0x20]`/`[0x30]`
by value — that would just be re-testing `composite_decode_grey_anchors`
under a different name. Instead it asserts the *property* the real desktop
software actually needed (achromatic, strictly brighter) so that if the
table's exact hex values ever get re-measured against a different real GIME
unit, this test keeps passing as long as the property real software depends
on still holds. That's a regression test written from the *consumer's*
requirement, not from the implementation's current output — the right level
to pin a hand-measured constant at.

The distinction generalizes, and it's worth carrying into every device in
the rest of this course. A test written against an implementation's current
output tells you the code changed. A test written against a consumer's
requirement tells you the code *broke*. The first kind is noise the day
someone legitimately improves a table; the second kind is the thing that
wakes you up. Two tests in this one file cover both jobs deliberately:
`composite_decode_grey_anchors` pins the exact values so an accidental edit
to the table is caught immediately, and `eou_greyscale_regression` pins the
property so a *deliberate* re-measurement is still allowed to happen without
anyone having to decide whether the desktop still works.

> **Rust corner: `abs_diff`, and the unsigned-subtraction trap.** The
> achromatic check is written `c[0].abs_diff(c[1]) <= GREY_TOLERANCE`, and
> the obvious-looking alternative would have been `(c[0] - c[1]).abs()`.
> That alternative does not compile, and the reason is worth internalizing
> because it recurs everywhere emulator code compares two unsigned values.
>
> These are `u8`s. There is no such thing as a negative `u8`, so `.abs()`
> doesn't exist on one, and the subtraction itself is the hazard: if `c[0]`
> is `0x2c` and `c[1]` is `0x2d`, then `c[0] - c[1]` underflows. In a debug
> build that's a panic; in a release build it wraps to `255`, and the test
> silently passes or fails for reasons unrelated to color. Writing
> `(c[0] as i16 - c[1] as i16).abs()` works but adds two casts and a wider
> type to say something simple.
>
> `u8::abs_diff` returns the absolute difference directly, as a `u8`, with
> no casts and no underflow to reason about. Reach for it any time the
> question is "how far apart are these two unsigned numbers" — which, in an
> emulator that compares pixels, registers, and cycle counts constantly, is
> more often than you'd guess.

## 9.7 What monitor choice meant for software

Put the two halves of `GIME::color` side by side and a piece of CoCo 3
software history falls out for free: **the exact same POKE produced a
different-looking screen depending on what was plugged into the back of the
machine**, and nothing the program did could tell which monitor it was
talking to. There is no monitor-sense register in the GIME's map; `MonitorType`
exists only in this emulator's configuration, and the codebase's own comment
is explicit that it is not a hardware register at all.

It is easy to conflate that with a different choice programs *could* make,
so it's worth separating them carefully. Extended Color BASIC's `SCREEN`
statement and its graphics-mode `PMODE`/`PCLS` calls choose a **color
set** — `CSS`, bit 3 of PIA1 `$FF22` — which selects *which* GIME palette
registers a legacy mode reads from (regs 8/9 vs. 10/11 for two-color modes,
0–3 vs. 4–7 for four-color; you'll meet the exact tables in §9.13). That's
a choice about *which register*, made by the program, at run time, and
observable by the program. The monitor choice is a choice about *how any
register's value is decoded*, made by a human with a cable, invisible to
every instruction the 6809 can execute. The two are orthogonal and they
happen at completely different layers: `CSS` selects an index, the monitor
decides what the value at that index looks like.

Follow that through to what a 1986 programmer actually experienced. A
program tuned its four `PMODE` colors by picking a `CSS` set that looked
good on the monitor its author owned. On a different machine, a composite TV
would run those exact same register values through `COMPOSITE_PALETTE`
instead of `rgb_color` and show something else entirely — not a slightly
different shade, but potentially a different hue, because the two decodes
share no structure whatsoever. Recall §9.6's arithmetic in reverse: the four
values that are a clean gray ramp on composite are black, green, red, and
yellow on RGB. A program written for either monitor is, from the other
monitor's point of view, using its palette registers to say something it
never meant.

This is the actual, mundane reason 1980s CoCo software sometimes shipped a
"for composite" and "for RGB" color scheme, and why a magazine type-in's
screenshot never quite matched the picture on the machine it was typed into.
Two totally different physical processes — a three-gun CRT reading three
separate voltages, versus an NTSC decoder demodulating a shared subcarrier —
were being asked to render six identical bits, and physics, not software,
decided they wouldn't agree.

### The test that proves the renderer never asks

So far, §9.1's "one renderer" claim has rested on an argument from reading
the code: `GIME::color` is the only place `MonitorType` is consulted, and
every painter calls it without inspecting the answer. Arguments from reading
code are exactly the arguments that quietly stop being true. The last test
in [`tests/composite.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/composite.rs)
turns it into something executable by rendering an actual field — a real
character, through the real text painter, into a real framebuffer — and
checking the pixel that comes out the far end
([`tests/composite.rs:98-125`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/composite.rs#L98-L125)):

```rust
#[test]
fn render_text_routes_through_composite_decode() {
    let mut g = GIME::new();
    g.monitor = MonitorType::Composite;
    g.vmode = TEXT_LPR8;
    g.vres = VRES_TEXT40;
    g.vertical_offset = VOFF;
    // Palette reg 1 (foreground for attribute-less text) holds a distinctive
    // 6-bit value; palette reg 0 (background) stays 0 (black either way).
    g.palette[1] = 0x01;

    let mut ram = vec![0u8; RAM_LEN];
    ram[BASE] = b'A';

    let mut fb = Vec::new();
    let (fb_w, _) = render_field(&g, &ram, false, &mut fb);

    // 'A' row 0 is 0x10: native pixel 3 lit -> foreground (palette reg 1).
    // 40 columns is a wide canonical mode: xscale 2, no side border, body
    // starts at canvas row 25 (LPF=%00).
    let expected_fg = g.color(0x01);
    assert_ne!(
        expected_fg,
        GIME::rgb_color(0x01),
        "test is only meaningful if composite and RGB decode differ here"
    );
    assert_eq!(px(&fb, fb_w, 3 * 2, 25), expected_fg);
}
```

Everything before `render_field` is Chapter 8's material: a 40-column
attribute-less text mode, one letter `A` at the video base, foreground from
palette register 1. What makes this a composite test rather than a text test
is the two assertions at the end, and the first one is the more interesting.

`assert_ne!(expected_fg, GIME::rgb_color(0x01), ...)` asserts nothing about
the renderer at all. It is a *guard on the test's own premise*: palette value
`0x01` decodes to `(0, 76, 0)` on composite and to something else entirely
on RGB, so a pixel matching the composite answer is proof the composite path
ran. If someone ever changed the test's palette value to one where the two
tables happen to agree — `0x00` and `0x3F` are exactly such values, black
and white in both decodes — the final assertion would still pass while
proving nothing, and this guard is what stops that from happening silently.
The failure message says so in plain language: "test is only meaningful if
composite and RGB decode differ here."

The second assertion then checks one specific framebuffer pixel — column
`3 × 2`, row `25` — against `g.color(0x01)`, not against a hardcoded triple.
Its coordinates come straight from Chapter 8's geometry: the glyph's top row
lights native pixel 3, a 40-column mode is wide so each native pixel is two
canvas pixels with no side border, and `LPF=%00` starts the body at canvas
row 25. Chase the value backwards through the code and the chain is the
whole claim of this half of the chapter: `render_field` → `paint_scanline` →
`resolve_colors` → `GIME::color` → `COMPOSITE_PALETTE`. Not one of those
functions was written for composite output, not one of them branches on
`MonitorType`, and swapping the monitor changes the pixel anyway.

That closes the color half of the chapter. Everything from here on holds
the monitor fixed and varies something else: *time*.

---

## 9.8 The lie Chapter 8 told you (on purpose)

Chapter 8's `render_field` reads every GIME register exactly once and paints an
entire field from that one snapshot. As a way of learning geometry it is
ideal — one set of registers, one picture, nothing moving. As a model of the
machine it describes a computer where BASIC's `PALETTE` and `PMODE`
statements only ever run between fields, never while one is being drawn.

Real 6809 code makes no such promise, and the interesting software made a
point of breaking it. A game can poke the border color from inside an
interrupt handler that fires at a *specific scanline*, sixty-odd times a
second, forever. A demo can rewrite a video register the instant the beam
crosses a row it knows by heart. The picture that results — different
colors in different bands of the *same* field — is a **raster split**, and
it was one of the CoCo demo scene's bread-and-butter tricks. Section 9.11
comes back to why anyone bothered, once the mechanism is on the table.

Here is the pleasant surprise: the machine loop already had everything this
needs, because Chapter 6 built it that way. `end_of_line()`
([`crates/coco-core/src/machine/run.rs:130-172`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs#L130-L172))
calls `self.render_scanline()` — one canvas row — **every single scanline**,
not once per field. Nothing had to be added to make splits work. What had to
be added was a small amount of machinery to stop *some* registers from
splitting, and that inversion is the whole subject of this section.

The per-scanline dispatcher ([`machine/render.rs:29-64`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L29-L64))
is what you actually extend this week:

```rust
pub(super) fn render_scanline(&mut self) {
    if self.config.variant != MachineVariant::Coco3 {
        return;
    }
    if self.line == 0 {
        let legacy = self.bus.gime.init0 & gime::init0::COCO != 0;
        self.field_scan = Some(gime_video::FieldScan::latch(&self.bus.gime, legacy));
        self.framebuffer
            .resize(raster::CANVAS_W * raster::CANVAS_H * BYTES_PER_PIXEL, 0);
        self.fb_width = raster::CANVAS_W as u32;
        self.fb_height = raster::CANVAS_H as u32;
    }
    let row = self.line as usize;
    let Some(scan) = self.field_scan.as_ref() else {
        return;
    };
    if row >= raster::CANVAS_H {
        return; // blanking lines 240..262
    }
    if scan.legacy {
        self.paint_legacy_scanline(row);
        return;
    }
    let blink_on = self.bus.gime.blink_state;
    let scan = self.field_scan.as_mut().expect("checked Some above");
    gime_video::paint_scanline(
        &self.bus.gime,
        &self.bus.ram,
        scan,
        blink_on,
        row,
        &mut self.framebuffer,
    );
}
```

Two things happen only at `self.line == 0`: the framebuffer is resized, and
`FieldScan::latch` runs. Every other line, the function paints straight from
whatever `self.bus.gime`'s registers currently say — live. That's the whole
split mechanism, stated in one sentence: **most registers are read fresh
every line; a small, named group is read once, at line 0, and frozen for the
rest of the field.**

The early `return` for lines 240 and up is worth a glance in passing, since
it explains a number this chapter uses repeatedly. The canonical raster is
262 lines tall but only 240 of them are visible; the rest is vertical
blanking, when a real CRT is dragging its beam back to the top. Those lines
still tick the machine loop — the timer still counts, interrupts still fire
— but they have nowhere to paint. A program can perfectly well write a
video register during blanking, and the effect shows up on line 0 of the
next field, which is exactly how a program that wants a *whole-field* change
rather than a split arranges one.

### `FieldScan`: the latch, made a value

`FieldScan` ([`gime_video.rs:117-159`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L117-L159)) is that freeze, reified as a struct
instead of scattered `if self.line == 0` checks:

```rust
/// Per-field video scanout state, latched at field start — the register group
/// MAME `gime.cpp` `new_frame` samples once per field and never re-reads
/// mid-frame: the video base address ($FF9D/$FF9E), the INIT0 COCO
/// legacy-vs-GIME switch, and the VSC smooth-scroll seed ($FF9C). Everything
/// else ($FF98/$FF99 mode bits, $FF9F offset/HVEN, $FF9A border) is read live
/// per line by [`paint_scanline`].
#[derive(Serialize, Deserialize)]
pub struct FieldScan {
    pub legacy: bool,
    pub(crate) row_base: usize,
    pub(crate) line_in_row: usize,
}

impl FieldScan {
    /// Latch the per-field register group (MAME `new_frame`). Legacy fields
    /// seed from the SAM-compat page base with `line_in_row` 0 (MAME:
    /// `m_line_in_row = COCO ? 0 : vsc`).
    pub fn latch(g: &GIME, legacy: bool) -> Self {
        let vsc = (g.vertical_scroll & 0x0F) as usize;
        let lpr = g.lines_per_row();
        Self {
            legacy,
            row_base: if legacy {
                g.sam_display_base() as usize
            } else {
                g.video_base()
            },
            line_in_row: if legacy || vsc >= lpr { 0 } else { vsc },
        }
    }
}
```

Three fields, three pieces of state that must survive across scanlines
without being re-derived from a register that might have changed underneath
them: which display path this field is on (`legacy`), where the current data
row starts (`row_base`, the field-latched video base, seeded from
`$FF9D/$FF9E` or the SAM page), and how far into the current character or
pixel row the scan is (`line_in_row`, the smooth-scroll seed, taken from the
`$FF9C` scroll nibble).

Chapter 8 introduced this struct as a fact about the code. It is worth asking
now *why* it is a struct at all, because the alternative is genuinely
tempting. Each of these three values could have been a field on `Machine`
guarded by an `if self.line == 0`, and the code would work. Making them a
named type with one constructor buys three things. The latch happens in
exactly one place, so "what gets frozen at field start" is answerable by
reading one function instead of grepping for line-zero checks. The
`Option<FieldScan>` on `Machine` distinguishes "no field has started yet"
from "a field is in progress" without a separate boolean. And, quietly, the
`#[derive(Serialize, Deserialize)]` means a save state taken mid-field
restores mid-field correctly, which is Chapter 16 collecting on Chapter 1's
plain-owned-tree discipline yet again.

`row_base` and `line_in_row` aren't just latched once and left alone,
though. `advance_scan` ([`gime_video.rs:247-258`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L247-L258))
mutates them at the bottom of every body row:

```rust
fn advance_scan(scan: &mut FieldScan, g: &GIME, row_bytes: usize) {
    let pitch = if g.horizontal_offset & hoff::HVEN != 0 {
        gime::HVEN_ROW_BYTES
    } else {
        row_bytes
    };
    scan.line_in_row += 1;
    if scan.line_in_row >= g.lines_per_row() {
        scan.line_in_row = 0;
        scan.row_base += pitch;
    }
}
```

Trace which values in there come from where, because the mix is the point.
`scan.line_in_row` and `scan.row_base` are the latched, carried-across-lines
state. But `g.horizontal_offset`, `g.lines_per_row()`, and the `row_bytes`
the caller computed from this line's decode are all read *live*, off the
current register file. So the cursor steps forward by the *current* line's
pitch, once the counter wraps past the *current* line's LPR. Origin frozen
at field start, cursor advancing live thereafter.

That split is exactly why a mid-field mode switch can walk off the end of a
row it started in text mode and finish it in graphics mode: nothing in
`advance_scan` knows or cares that the byte layout underneath the pointer
just changed shape. You'll see that precise scenario as §9.9's fourth test.

Liveness at the other end of the function is equally uncontrived. Deciding
whether a canvas row is even inside the active picture is itself a live
read ([`gime_video.rs:191-195`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L191-L195)):

```rust
fn in_active_rows(g: &GIME, row: usize) -> bool {
    let lpf = ((g.vres & vres::LPF_MASK) >> vres::LPF_SHIFT) as usize;
    let (top, body) = vertical_window(lpf);
    row >= top && row < top + body
}
```

`$FF99`'s LPF field — how many active lines this field has — is re-read on
every single line, which means a program that changes it mid-field changes
where the bottom border starts *for the remainder of that same field*. The
function's own doc comment says as much ("from the LIVE LPF bits, applies
even mid-frame"). Nobody designed that as a feature. It is what happens when
a function takes `&GIME` and asks it a question instead of consulting a
cached answer.

Put together, that's the register taxonomy for the whole chapter:

| Group | Registers | When it takes effect |
|---|---|---|
| **Live** | `$FF9A` border, `$FF98` mode bits (BP/BPI/MOCH/LPR), `$FF99` VRES (LPF/HRES/CRES), `$FF9F` X-offset/HVEN, all 16 palette regs | The very next scanline painted |
| **Field-latched** | `$FF9D/$FF9E` video base, INIT0 `COCO`, `$FF9C` scroll seed | The *next field* — a write mid-frame is invisible until the wrap back to line 0 |

Notice what's *not* on the field-latched list: the palette registers
themselves. A `PALETTE` statement (or a raw `STA $FFBx`) is live, exactly
like the border — which is precisely why EOU's grayscale desktop and any
palette-cycling demo effect work at all on real hardware without waiting a
whole frame.

Notice also that the field-latched group is small, specific, and not
obviously principled. Why is the video base frozen when the border isn't?
The honest answer this codebase gives is that the reference implementation
does it that way — MAME's `new_frame` samples exactly this group — and the
tests in the next section exist to make sure the emulator keeps doing it
that way. Section 9.11 offers a plausible hardware reason after the fact,
but the reason the code is shaped like this is that the behavior was
verified first and rationalized second, which is the correct order.

## 9.9 Four registers, four tests

A taxonomy in a table is a claim. [`tests/scanline_split.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/scanline_split.rs)
is the same taxonomy as executable assertions, and it earns its "best-named
test file in the repo" reputation (course README, Chapter 9) by testing one
register class at a time, in isolation, with the rest of the machine held
deliberately still.

Holding the machine still takes some doing. The tests run against a
**zero-filled ROM**: the reset vector points at `$0000`, which decodes to a
harmless `NEG` loop, so the CPU has something legal to execute forever while
scanlines advance at a known, deterministic pace, and nothing in ROM touches
a single video register. That last part is the real requirement. Booting the
actual Color BASIC ROM would give a machine that constantly reprograms video
registers on its own, and a test asserting "this pixel is palette 5" would
be arguing with the ROM about who gets to decide. Every test in this file
pokes `m.bus.gime` fields directly from the harness instead.

The shared setup ([`tests/scanline_split.rs:16-55`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/scanline_split.rs#L16-L55)):

```rust
/// $FF9D/$FF9E value → physical $8000.
const VOFF: u16 = 0x1000;
const BASE: usize = (VOFF as usize) << 3;

/// The scanline the tests poke registers at: inside the 192-line body
/// (rows 25..217 for LPF=%00).
const SPLIT_LINE: u32 = 100;

/// A CoCo 3 machine in GIME-native 320×192×16 graphics with an identity
/// palette, running zero-ROM filler code.
fn gime_graphics_machine() -> Machine {
    let rom = vec![0u8; 32 * 1024].into_boxed_slice();
    let mut m = Machine::new(MachineConfig::default(), rom);
    let g = &mut m.bus.gime;
    g.init0 = 0; // COCO=0: GIME-native video
    g.vmode = vmode::BP;
    g.vres = 0x1E; // HRES=%111 (160 bytes), CRES=%10 → 320px, 16 colours
    g.vertical_offset = VOFF;
    for (i, reg) in g.palette.iter_mut().enumerate() {
        *reg = i as u8;
    }
    m
}

/// Run to the end of the current field (the wrap back to line 0).
fn finish_field(m: &mut Machine) {
    while !m.step_instruction().field_complete {}
}

/// Run until the machine is at the start of `line` within the current field.
fn run_to_line(m: &mut Machine, line: u32) {
    while m.scanline() != line {
        m.step_instruction();
    }
}
```

`palette[i] = i` is a deliberate identity mapping, and it is the trick that
makes every later assertion legible. Reading pixel value `v` back from a
rendered row and comparing it to `GIME::rgb_color(v)` proves the render
picked up palette register `v` specifically — there is no ambiguity about
which of sixteen registers a given on-screen color came from. (An RGB
monitor is in force here, since `MachineConfig::default()` doesn't select
composite, so there's no table indirection to reason about on top of the
palette indirection.)

`SPLIT_LINE = 100` is chosen to sit comfortably inside the active picture,
and the comment's "rows 25..217 for LPF=%00" is not a magic number either.
It comes from the canonical raster's vertical placement table
([`raster.rs:33-40`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/raster.rs#L33-L40)):

```rust
pub const fn vertical_window(lpf: usize) -> (usize, usize) {
    match lpf {
        0 => (25, 192),
        1 => (23, 200),
        2 => ((CANVAS_H - 210) / 2, 210),
        _ => (8, 225),
    }
}
```

`LPF=%00` is 25 rows of top border followed by 192 rows of body, so the body
occupies canvas rows 25 through 216 inclusive and line 100 lands about
two-fifths of the way down it. The same function is what `in_active_rows`
consulted a page ago, and what this section's third test uses to convert
"body row 105" into "canvas row 130."

The two helpers at the bottom are how a test addresses a moment in time.
`finish_field` runs instructions until the machine reports the field wrapped;
`run_to_line` free-runs the CPU until the scanline counter reads a chosen
value. Neither can rewind, which shapes every test in the file: to reach a
known line, you finish the current field first and then run forward into a
fresh one.

### Live: the border splits mid-field

```rust
#[test]
fn border_write_mid_field_splits_the_border_at_the_line() {
    let mut m = gime_graphics_machine();
    m.bus.gime.border = 0x09;
    finish_field(&mut m); // first full field with the old border

    run_to_line(&mut m, SPLIT_LINE);
    m.bus.gime.border = 0x2A;
    finish_field(&mut m);

    let fb = &m.framebuffer;
    assert_eq!(
        px(fb, 0, 0),
        GIME::rgb_color(0x09),
        "top border painted before the write keeps the old colour"
    );
    assert_eq!(
        px(fb, 0, CANVAS_H - 1),
        GIME::rgb_color(0x2A),
        "bottom border painted after the write has the new colour"
    );
}
```

The first `finish_field` is bookkeeping, not physics: it exists purely to
get the machine's internal line counter back to a known `0` so the following
`run_to_line` lands where the test thinks it does. The interesting part is
the second field. Border is `0x09` when the scan reaches row `SPLIT_LINE`,
gets rewritten to `0x2A` mid-field, and the top row (painted while `border`
was still `0x09`) and the bottom row (painted after the write) genuinely
differ within the *same* framebuffer.

The assertion sites are chosen to be unarguable. Canvas row 0 and canvas row
`CANVAS_H - 1` are both border rows in every mode this test could be in, so
the test never has to reason about where the body starts; it just asks
whether the frame around the picture is one color or two.

What's worth appreciating is how little code exists to make this work.
`paint_side_borders` and the "not in the active window" fill both call
`resolve_colors(g)` fresh, every line ([`gime_video.rs:161-168`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L161-L168),
Chapter 8's code, unmodified), and `resolve_colors` reads `g.border` straight
off the live `GIME`. Liveness here isn't a feature that was added; it's what
happens when nothing was added to *prevent* it. The correct emulator
behavior fell out of writing the renderer as a function of the current
register file rather than of a snapshot — which is the same design instinct
that made `GIME::color` a pure function in §9.3.

### Live: a palette write recolours only what's below it

```rust
#[test]
fn palette_write_mid_field_recolors_only_lines_below_it() {
    let mut m = gime_graphics_machine();
    // Zeroed video RAM → every body pixel reads palette register 0.
    m.bus.gime.palette[0] = 0x01;
    finish_field(&mut m);

    run_to_line(&mut m, SPLIT_LINE);
    m.bus.gime.palette[0] = 0x02;
    finish_field(&mut m);

    let fb = &m.framebuffer;
    let split = SPLIT_LINE as usize;
    assert_eq!(px(fb, 0, split - 1), GIME::rgb_color(0x01), "body above the split keeps the old palette");
    assert_eq!(px(fb, 0, split), GIME::rgb_color(0x02), "the split line onward has the new palette");
    // Both orders: the same field shows both colours at once.
    assert_eq!(px(fb, 0, 30), GIME::rgb_color(0x01));
    assert_eq!(px(fb, 0, 200), GIME::rgb_color(0x02));
}
```

Same shape as the border test, different register — and it matters that it's
a different *kind* of register. The border is a color directly; a palette
register is one level of indirection, since video RAM holds an *index* and
the palette register holds the *color* that index currently means. Leaving
video RAM zero-filled means every body pixel reads index 0, so the test is
asking one clean question: when register 0's meaning changes mid-field, do
the pixels already painted keep the old meaning?

They do, because `resolve_colors` rebuilds the whole 16-entry resolved-color
array from `g.palette` on every single call to `paint_scanline`
([`gime_video.rs:264-273`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L264-L273)).
That per-line rebuild looked, in Chapter 8, like a small performance
compromise — sixteen `GIME::color` calls per scanline instead of sixteen per
field. This test is the bill coming due in the other direction: the rebuild
is not a compromise, it is the feature. A demo that cycles palette registers
every scanline to get more than sixteen colors on screen (§9.11) needs
exactly this property, and a renderer that cached the resolved palette per
field would render such a demo as a flat, wrong, single-palette picture with
no error message anywhere.

The last two assertions are the ones that make the test's name literally
true. Row 30 is near the top of the body and row 200 near the bottom, and
they are checked *in the same framebuffer* — this is not two fields compared
against each other, it is one field showing two palettes at once.

### Field-latched: the video base waits

```rust
#[test]
fn video_base_write_mid_field_waits_for_the_next_field() {
    let mut m = gime_graphics_machine();
    // Marker bytes for a body row BELOW the split (row 130 = body row 105),
    // distinct at the two candidate bases: only a row painted after the poke
    // can tell whether the base was re-latched mid-field.
    const MARKER_ROW: usize = 105; // body row → canvas row 25 + 105 = 130
    let other_voff = 0x1100u16;
    let other_base = (other_voff as usize) << 3;
    m.bus.ram[BASE + MARKER_ROW * 160] = 0x50; // palette 5 at the latched base
    m.bus.ram[other_base + MARKER_ROW * 160] = 0x70; // palette 7 at the new base
    finish_field(&mut m);

    run_to_line(&mut m, SPLIT_LINE);
    m.bus.gime.vertical_offset = other_voff;
    finish_field(&mut m);
    let marker_canvas_row = 25 + MARKER_ROW;
    assert_eq!(
        px(&m.framebuffer, 0, marker_canvas_row),
        GIME::rgb_color(5),
        "mid-field base write must NOT retarget this field (MAME new_frame)"
    );

    finish_field(&mut m);
    assert_eq!(
        px(&m.framebuffer, 0, marker_canvas_row),
        GIME::rgb_color(7),
        "the next field latches the new base"
    );
}
```

This is the field-latched half of the taxonomy, and the test is built to be
unforgiving about it. Start with the setup arithmetic, since three constants
have to line up for the assertions to mean anything. `MARKER_ROW = 105` is a
*body* row, so it paints at canvas row `25 + 105 = 130`, comfortably below
`SPLIT_LINE = 100`. The stride `160` is this mode's bytes per row, from
`HRES=%111` in the shared setup. And the two bases are eight bytes apart per
`vertical_offset` count, since `video_base()` shifts the register left by
three. The marker byte `0x50` is the pixel pair `(5, 0)` at 4 bits per pixel,
so its leftmost pixel is palette index 5; `0x70` gives index 7.

Now notice the deliberate redundancy: the setup plants **two different
marker bytes at two different physical addresses** — `0x50` at the field's
actual video base, `0x70` at the *other* candidate base the mid-field write
points at. If the test only checked "does row 130 show palette 5," then a
broken implementation that re-read `g.video_base()` live every row would
have to coincidentally paint something equal to `GIME::rgb_color(5)` to
pass — unlikely, but that isn't the point. The point is that the marker at
the *new* base exists so a wrong answer is forced to be *visibly, specifically*
wrong: a bug that re-latches mid-field produces palette 7, a recognizable
symptom rather than an ambiguous one. A test that can only fail vaguely is a
test that gets debugged slowly.

The sequence then does the actual work. The write happens mid-field, at line
`SPLIT_LINE`, but the marker row — canvas row 130, well below the split — is
asserted to *still* show the old base's byte after that same field finishes.
That proves `row_base`, latched once at line 0 by `FieldScan::latch`, was
never touched by the mid-field write to `$FF9D/$FF9E`. Only after a *second*
`finish_field`, which runs a fresh `FieldScan::latch` at the new field's line
0, does the marker flip to palette 7.

Hold both halves of `FieldScan` in view at once here, because this test is
where they visibly do different jobs. `advance_scan` marched `scan.row_base`
forward on every body row of that field, using a pitch computed from live
registers — so the pointer was moving the entire time. What never moved was
its **origin**, fixed the instant the field began. A mid-field write to the
vertical offset registers changes a value that nothing will read again until
the next field starts.

That is a design constraint real software had to respect, and it comes
straight from the hardware rather than from any simplification here. Switching
which of two double-buffered screens is being scanned out is a *whole-field*
operation on a CoCo 3, never a mid-field one. Section 9.11 returns to what
that ruled out.

### Mode switches split too

```rust
#[test]
fn mode_switch_mid_field_splits_text_and_graphics() {
    let mut m = gime_graphics_machine();
    finish_field(&mut m);

    run_to_line(&mut m, SPLIT_LINE);
    // Switch to 80-column text with attributes mid-field. The row pointer has
    // already advanced through the graphics rows, so fill a wide swath of
    // char/attr pairs (' ' on attr bg palette 2) wherever the fetch lands.
    m.bus.gime.vmode = 0x03; // BP=0, LPR=%011 (8-line rows)
    m.bus.gime.vres = 0x15;
    for i in (BASE..BASE + 0x8000).step_by(2) {
        m.bus.ram[i] = b' ';
        m.bus.ram[i + 1] = 0x02;
    }
    finish_field(&mut m);

    let fb = &m.framebuffer;
    assert_eq!(px(fb, 0, 30), GIME::rgb_color(0), "graphics decode above the split (zeroed RAM → palette 0)");
    assert_eq!(px(fb, 0, SPLIT_LINE as usize + 1), GIME::rgb_color(2), "text decode below the split");
}
```

The border and the palette are colors. This test changes something more
drastic: the *meaning of every byte* the renderer fetches. Above the split
the machine is in 16-color graphics, where one byte is two pixels; below
it, 80-column attribute text, where one byte is a character code and the
next is its attributes. Both halves paint into the same field.

`$FF98`'s BP bit (graphics/text select) is in the *live* group, same as the
border — `paint_body_row` re-checks `g.vmode & vmode::BP` on every call
([`gime_video.rs:220-241`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L220-L241)), dispatching to `paint_graphics_row` or
`paint_text_row` fresh each line. So a program can flip from a graphics
canvas to a text status bar partway down the screen, and the emulator does
exactly what the register file says to do, one line at a time, with no
special-casing anywhere for "mode changed since last line."

The test's own comment about "the row pointer has already advanced through
the graphics rows" is worth sitting with, because it is the practical
consequence of §9.8's origin-versus-cursor split. `scan.row_base` and
`scan.line_in_row` don't know or care that the byte layout underneath them
just changed shape — they're purely a byte-address cursor, advanced by
whatever pitch the *current* line's decode reports. When the mode flips, the
cursor keeps going from wherever the graphics rows left it, at a new pitch,
into RAM that was laid out for a different purpose.

Which is why the RAM fill is so broad. `step_by(2)` across the whole 32K
region writes character/attribute pairs *everywhere* the text decode could
possibly land, because the test author declined to hand-compute the exact
byte offset the cursor would be sitting at after a hundred graphics rows at
160 bytes each. That is a legitimate and underrated testing move: when the
precise value of an intermediate is irrelevant to the property under test,
arrange for every possible value to give the same answer rather than
predicting the one that occurs. The assertion — palette 2 below the split,
because the attribute byte `0x02` names background palette 2 and the
character is a space — holds no matter where the fetch lands.

Four tests, four register classes, one mechanism. What none of them do is
prove the mechanism works when driven by the thing that actually drives it
on real hardware.

## 9.10 The centerpiece: a split written in 6809, not the harness

Every test above works by reaching into `m.bus.gime` from the test harness
and writing a register directly. That is the right way to isolate one
register class — it removes the CPU, the interrupt controller, and the timer
from the list of things that could be wrong. It also sidesteps the actual
mechanism a real raster-split demo used: **an interrupt handler**, running as
ordinary 6809 code, timed by the GIME's own interval timer.

`timer_firq_from_rom_code_splits_the_border`
([`tests/scanline_split.rs:174-239`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/scanline_split.rs#L174-L239)) is the test that closes that gap. It
proves the whole path — timer hardware, interrupt controller, FIRQ delivery,
and the live-register raster split — works end to end, with **zero harness
register pokes**. It hand-assembles a tiny ROM and lets the emulated CPU do
everything, which makes it the closest thing in this codebase to running a
real demo effect. It is also the first time this course drives the GIME's
timer directly: Chapter 6 asserted only that `tick_timer` gets *called*, and
here you see what arms it.

### The setup program

```rust
const OLD_BORDER: u8 = 0x09;
const NEW_BORDER: u8 = 0x2A;
/// FIRQ handler location in the ROM image ($8000 + offset).
const ISR: u16 = 0x8040;

let program: &[u8] = &[
    0x10, 0xCE, 0x1F, 0xF0, // LDS  #$1FF0      stack in low RAM
    0x86, OLD_BORDER,       // LDA  #OLD_BORDER
    0xB7, 0xFF, 0x9A,       // STA  $FF9A       border = old colour
    0x86, 0x20,             // LDA  #intr::TMR
    0xB7, 0xFF, 0x93,       // STA  $FF93       FIRQENR: timer source
    0x7F, 0xFF, 0x91,       // CLR  $FF91       INIT1: TINS=0 (hsync rate)
    0x7F, 0xFF, 0x94,       // CLR  $FF94       timer MSB = 0
    0x86, SPLIT_LINE as u8, // LDA  #SPLIT_LINE
    0xB7, 0xFF, 0x95,       // STA  $FF95       timer LSB (restarts count)
    0x86, 0x10,             // LDA  #init0::FEN
    0xB7, 0xFF, 0x90,       // STA  $FF90       INIT0: FIRQ out, COCO=0
    0x1C, 0xAF,             // ANDCC #$AF       unmask FIRQ/IRQ
    0x20, 0xFE,             // BRA  *           wait for the timer
];
```

Read it as a straight line — this is exactly the kind of ROM setup code real
period software is built from, just shorter. `LDS #$1FF0` gives interrupts
somewhere to stack a return frame; nothing here touches the CPU's normal
work because there isn't any, since this ROM's only job is to arm one
interrupt and then spin. `STA $FF9A` sets the border to `OLD_BORDER`
(`0x09`) up front, so the field's top half has a known starting color.

Then five writes configure the GIME's timer and interrupt block one register
at a time, and each one is worth naming:

- `STA $FF93` with `intr::TMR` (`0x20`) in A writes **FIRQENR**, the
  per-source FIRQ enable register — "when the timer underflows, route it to
  FIRQ, not just IRQ." (`GIME::write_firq_enable`, [`gime.rs:311-314`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L311-L314).)
- `CLR $FF91` writes **INIT1** to zero — among other things, clearing `TINS`
  (`init1::TINS`, bit 5), which selects the *slow* timer clock: one tick per
  horizontal sync rather than the fixed high-frequency clock. One tick per
  scanline is exactly the granularity a raster split needs, and it means the
  programmed value can be read as "how many scanlines from now."
- `CLR $FF94` / `LDA #SPLIT_LINE` + `STA $FF95` load the 12-bit timer value,
  MSB first (zero, since `SPLIT_LINE` fits in one byte) and LSB second. Both
  `write_timer_msb` and `write_timer_lsb` call `restart_timer()`
  ([`gime.rs:346-370`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L346-L370)), so the **second** write is what actually
  (re)starts the countdown, seeded from whatever the MSB write already
  latched into the top nibble. Writing the bytes in the other order would
  start a countdown from a half-programmed value.
- `STA $FF90` writes **INIT0** with `init0::FEN` (`0x10`) set — the master
  FIRQ-output-enable gate. `GIME::firq_asserted()` ([`gime.rs:341-344`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L341-L344))
  requires *both* this bit and a nonzero `firq_pending`; without it, the
  timer could underflow all day and never reach the CPU. Note that the same
  write also puts `COCO` at 0, keeping the machine on the GIME-native video
  path.
- `ANDCC #$AF` clears bits `0x50` of CC — `cc::IRQ_MASK` (`0x10`) and
  `cc::FIRQ_MASK` (`0x40`), the two interrupt masks reset sets on every CoCo
  (Chapter 4). This is the instruction that actually lets the CPU *notice* the
  interrupt once it arrives; everything before it was arming hardware that
  stays silent while masked.

`BRA *` is the whole rest of the "program": branch to self, forever. There
is deliberately nothing else for the CPU to do — the only way this loop ever
does anything again is an interrupt breaking it open. It is also, quietly, a
demonstration of why interrupts exist at all. Counting scanlines by
executing a precisely tuned delay loop is possible and miserable; arming a
timer and going to sleep is neither.

One period detail is worth flagging because two source comments in this
codebase disagree with each other about it, and reading both is instructive.
`init1::TINS`'s doc comment in [`gime.rs:62-68`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L62-L68)
describes the fast clock as "~70 ns (14.318 MHz)," repeating SEB Unravelled
II. The constant that the machine loop actually uses says otherwise
([`machine.rs:28-34`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine.rs#L28-L34)):

```rust
/// GIME timer input clocks per normal-speed CPU cycle with INIT1 TINS=1. The
/// fast timer clock is 3.579545 MHz (279.365 ns — hardware-measured; MAME
/// `gime.cpp`. SEB's "70 ns" is wrong), exactly 4× the 0.89 MHz CPU clock —
/// and 2× the double-speed CPU clock, since the timer runs off the fixed
/// video crystal and ignores the CPU rate. With TINS=0 the input is the
/// ~63.5 µs horizontal sync: one tick per scanline.
const FAST_TIMER_TICKS_PER_CPU_CYCLE: u32 = 4;
```

Two comments in one crate, one of them repeating the standard reference and
one of them correcting it with a measured figure and a citation. The
implementation follows the corrected one: four ticks per CPU cycle, which is
the 3.579545 MHz subcarrier frequency §9.3 has already met. This test uses
`TINS=0` and never touches the fast path, but the lesson applies everywhere
in this book — when the code and a comment disagree, the code is what runs,
and when two comments disagree, the one carrying a measurement and a source
is the one to trust.

### The handler

```rust
let isr: &[u8] = &[
    0xB6, 0xFF, 0x93, // LDA  $FF93   read status (clears the latch)
    0x86, NEW_BORDER, // LDA  #NEW_BORDER
    0xB7, 0xFF, 0x9A, // STA  $FF9A   border = new colour
    0x7F, 0xFF, 0x93, // CLR  $FF93   no further timer FIRQs
    0x3B,             // RTI
];
```

Five instructions, and the first one is the most interesting thing in the
test. `LDA $FF93` reads **FIRQENR** as a *status* register, not the enable
register it was a moment ago in the setup code — same address, two different
jobs depending on whether the access is a read or a write. This is the exact
"reads have side effects" story Chapter 1 promised would come back;
`Bus::read` takes `&mut self` specifically because of registers like this
one.

`GIME::read_firq_status` ([`gime.rs:322-325`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L322-L325))
is the whole implementation:

```rust
    /// Read FIRQENR ($FF93): returns the latched FIRQ status and clears it.
    pub fn read_firq_status(&mut self) -> u8 {
        std::mem::take(&mut self.firq_pending)
    }
```

Reading returns the latched bits *and* zeroes them in the same motion. This
ISR doesn't even look at the value it read — the very next instruction
overwrites A — so the read exists purely for its side effect, acknowledging
the interrupt so the CPU's FIRQ line drops and `RTI` doesn't just re-enter
the handler instantly. An emulator whose `read` had been declared `&self`
could not model this without interior mutability in the GIME, and a
handler-loop hang is exactly the symptom that would result from getting it
wrong.

> **Rust corner: `std::mem::take` as the read-and-clear idiom.** The
> hardware behavior here is "give me the value and reset the register to
> zero, atomically." The naive Rust spelling is three lines: copy the field
> into a local, assign `0` to the field, return the local. `std::mem::take`
> is that operation as one call — it swaps the field with
> `Default::default()` for its type and hands back what was there.
>
> The reason it exists as a standard function rather than a hand-rolled
> pattern is ownership. For a `u8` you could get away with the three-line
> version, since `u8` is `Copy`. For a field you cannot copy out of — a
> `Vec` of queued events, say, or a `String` — the three-line version does
> not compile at all: moving out of `&mut self` leaves the struct in a
> state the borrow checker refuses to accept. `mem::take` never creates
> that hole, because it puts the default in place at the same instant it
> removes the old value.
>
> Emulator device code is full of registers with exactly this contract, so
> the idiom recurs: any time hardware documentation says "cleared on read,"
> `std::mem::take(&mut self.field)` is very likely the whole function body.
> `GIME::read_irq_status` immediately above it is the same line for the IRQ
> twin.

`STA $FF9A` is the actual raster-split write: border becomes `NEW_BORDER`
(`0x2A`), live, effective starting the *next* scanline exactly like every
test in §9.9. `CLR $FF93` then writes zero through `write_firq_enable`
([`gime.rs:311-314`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L311-L314)), which does two things at once —
`self.firq_enable = 0`, so no source can raise FIRQ anymore, and, per that
function's own doc comment, `self.firq_pending &= val` re-clears pending as
well. That second effect is a documented hardware anomaly rather than an
implementation convenience: writing `0` to an enable bit clears that
source's latched status too, which "SEB Unravelled II documents and MAME
models," per the source comment. Belt and suspenders against a second FIRQ
firing before `RTI` retires.

`RTI` (`0x3B`) restores the **partial frame** — just CC and PC, three bytes,
because FIRQ only ever stacks the partial frame (Chapter 4) — and the CPU drops
back into `BRA *`, forever, border now `NEW_BORDER`. The choice of FIRQ over
IRQ is not incidental for this kind of effect: a handler that must land
within a scanline wants the cheapest possible entry and exit, and three
bytes of stack traffic instead of twelve is most of that difference.

### Wiring the vectors

An interrupt handler that nothing points at never runs. The FIRQ hardware
vector, `$FFF6/$FFF7` (`VECTOR_FIRQ`, Chapter 4), has to point at `ISR`, and
the reset vector at `$FFFE/$FFFF` has to point at the setup program:

```rust
rom[isr_off..isr_off + isr.len()].copy_from_slice(isr);
// Vectors (hardwired-internal $FFE0+ region): FIRQ → ISR, RESET → $8000.
rom[0x7FF6..0x7FF8].copy_from_slice(&ISR.to_be_bytes());
rom[0x7FFE..0x8000].copy_from_slice(&0x8000u16.to_be_bytes());
```

`0x7FF6` is `$FFF6 − $8000`. The ROM image is a flat 32K buffer representing
`$8000–$FFFF`, so every absolute 6809 address in this test is offset by
`$8000` to find its byte in `rom[]` — which is also why `ISR` is declared as
`0x8040` (a CPU address) and then converted with `(ISR - 0x8000) as usize`
to index the array. Mixing those two address spaces up is a classic way to
spend an afternoon.

`to_be_bytes()` matters exactly as much here as it did in Chapter 1's
`Bus::write_u16` default method. The 6809 fetches vectors big-endian, and a
little-endian byte order here would send the CPU to a FIRQ handler at
completely the wrong address on the very first interrupt — in this ROM, to
`$4080`, which is RAM full of zeros, where it would execute whatever `$00`
decodes to until something gave. Getting a wrong-endian vector is rarely
subtle, but it is always confusing the first time.

### Reading the assertion

```rust
let mut m = Machine::new(MachineConfig::default(), rom.into_boxed_slice());
finish_field(&mut m);

let old = GIME::rgb_color(OLD_BORDER);
let new = GIME::rgb_color(NEW_BORDER);
let column: Vec<[u8; 4]> = (0..CANVAS_H).map(|y| px(&m.framebuffer, 0, y)).collect();
assert_eq!(column[0], old, "field starts on the old border");
assert_eq!(column[CANVAS_H - 1], new, "field ends on the new border");
let transitions: Vec<usize> = (1..CANVAS_H)
    .filter(|&y| column[y] != column[y - 1])
    .collect();
assert_eq!(transitions.len(), 1, "exactly one border split, got {transitions:?}");
let split_row = transitions[0];
let expected = SPLIT_LINE as usize;
assert!(
    (expected..=expected + 4).contains(&split_row),
    "split at row {split_row}, expected within {expected}..={}",
    expected + 4
);
```

This samples the entire left border column, row by row, and scans it for
color changes. Three assertions follow, and each is answering a different
question. Does the field start on the old color and end on the new one — so
the split happened at all? Is there exactly *one* transition — so the timer
didn't re-arm and paint stripes? And is that transition at approximately the
right line?

The transition scan is worth stealing as a technique. Rather than asserting
against specific rows the test predicted in advance, it derives the set of
rows where the color changes and then makes claims about that set. The
failure message prints the whole set (`got {transitions:?}`), so a broken
run tells you immediately whether the problem is "no split," "split in the
wrong place," or "splitting repeatedly" — three quite different bugs that a
row-by-row assertion would have reported identically as "row 100 was the
wrong color."

The tolerance window (`expected..=expected + 4`) is the honest
acknowledgment of a chain of real delays, and it is worth walking end to
end because every link is a piece of hardware this course has built. The
timer's countdown doesn't start when the CPU boots; it starts when the
`$FF95` write executes, a handful of instructions into the setup program and
already partway through line 0. From there, `tick_timer`
([`gime.rs:382-394`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L382-L394))
counts down from `SPLIT_LINE + TIMER_RELOAD_OFFSET`, one tick per
`end_of_line` call, since `TINS` selects the horizontal-sync rate. The
reload always adds the hardware's documented `+2` offset ([`gime.rs:162-165`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L162-L165))
on top of the programmed value, a measured quirk of the 1986 GIME that this
codebase models deliberately.

So there are `SPLIT_LINE + 2` scanlines of countdown before the FIRQ source
even latches. Then the CPU has to notice it, and interrupts are recognized
only at instruction boundaries (Chapter 6), so the `BRA *` in progress finishes
first. Then the FIRQ sequence stacks its partial frame and vectors. Then
three ISR instructions execute before the border write lands. And the border
write only affects lines painted *after* it, so the visible transition is
one more line down. All of that stacks up to "a few lines later than
`SPLIT_LINE` exactly," which is what the four-line tolerance absorbs.

Resist the temptation to think a tighter bound would be a better test. An
exact expected row would encode the cycle costs of five instructions, the
FIRQ entry sequence, and the emulator's line-granular interrupt polling into
a single magic number that changes meaning the first time any of those is
refined — and it would be asserting something the test does not care about.
What this test proves is the *mechanism*: timer hardware, FIRQ delivery, a
live-register raster write, from genuine 6809 code, landing where a
programmer aiming at line 100 would expect to see it. That is the claim, and
the tolerance is the claim stated honestly.

## 9.11 Why demos did this on real hardware

Once you've read that test, the motive for raster splits stops being
abstract. The constraint every CoCo 3 programmer worked under is that the
palette is 16 *simultaneous* registers, and that number never changes no
matter what resolution or bit depth is selected. A 16-color graphics mode
uses all sixteen; a 4-color mode uses four of them; there is no mode
anywhere in the GIME's register map that gives you a seventeenth color.

But "simultaneous" is doing a lot of work in that sentence, and the raster
is a *sequence*, not a snapshot. Nothing stops a program from reprogramming
those sixteen registers partway down the screen. A status bar at the bottom
of a game screen, drawn in colors that would clash with the play field
above it, can have its own sixteen-color palette for free. Set the game's
colors, run the beam down to the status bar's first line, then rewrite all
sixteen palette registers — a `PALETTE` statement's worth of writes, or the
raw `STA $FFBx` sixteen times a machine-language routine would do — and the
bottom band renders in an entirely different color scheme. Still only
sixteen colors *at any given instant*, but thirty-two colors *on screen*,
because "on screen" spans more than one instant.

Push it further and the arithmetic gets silly in a good way. Nothing limits
a program to one split. A handler that fires every eighth scanline and
rewrites the palette each time gives a picture with dozens of distinct
colors in it, on hardware whose data sheet says sixteen — at the cost of
the CPU spending a meaningful fraction of every field inside an interrupt
handler doing nothing but writing color registers. That trade, screen
richness paid for in CPU time, is the characteristic shape of demo-scene
programming on every machine of the era.

A border split, exactly what the FIRQ test demonstrates, is the same trick
applied to the cheapest possible canvas: no video RAM at all, one register,
one interrupt, turning a plain rectangular border into a two- or three-band
frame around the action. Purely decorative, essentially free, and purely a
product of understanding that the picture is drawn over time.

Split-screen games — a status HUD with its own scroll position, distinct
from a playfield scrolling underneath it — want to go one step further and
rewrite the *video base* partway down. As §9.9's third test proved, that
register is field-latched, not live. So this particular effect was not
available in this form on a CoCo 3: real split-screen work had to be
built from two independent *fields* interleaved by persistence of vision, or
from arranging the two regions inside one fixed video-RAM window and
scrolling within it, never from a video-base write mid-field. Knowing which
registers are live and which are latched isn't an emulator-accuracy detail
— it's the same knowledge a 1988 demo-scene programmer needed to work out
which effects were even possible before writing a line of code.

From the emulator author's chair, this section is also the entire
justification for `render_scanline` painting one canvas row per call instead
of snapshotting the whole field at once — the way Chapter 8 first showed it,
and the way the CoCo 1/2 legacy path in §9.14 still does it. A whole-field
snapshot renderer is simpler to write and faster to run. It is also
*provably wrong* the moment any real program does what this section
describes, because it can only ever show the state of the registers at the
one instant it happened to sample them. Every effect above would render as a
single flat band.

That is what `FieldScan` buys, and why it exists as a named struct with a
documented latch-versus-live split rather than as a snapshot taken at field
end: so the emulator's picture matches what a CRT actually painted —
continuously, one line at a time, at the mercy of whatever the CPU had
written by the time the beam got there.

---

## 9.12 Legacy VDG graphics: the two-chip tango

Step back from the GIME-native modes entirely, and back in time. Long before
`HSCREEN` and `$FF98`, the CoCo 1 and 2's actual video chip — the Motorola
MC6847 Video Display Generator — drove a family of resolution-graphics modes
that Extended Color BASIC exposed as `PMODE 0` through `PMODE 4`. For most
of the machine's commercial life, `PMODE` *was* CoCo graphics. It is the
statement in every magazine type-in listing, the thing `PCLS` clears and
`LINE` draws into, and the vocabulary in which an entire generation of
users learned that a computer could draw.

The CoCo 3 has no MC6847 at all. What it has is the GIME's CoCo-compatible
path (INIT0 `COCO`=1), which *imitates* one closely enough that the same
BASIC programs, and the same POKEs, still work — the same backward-
compatibility story Chapter 1 traced through the SAM-compat register range and
Chapter 7 found sitting under the boot prompt. This section is legacy content
in the strict sense: a compatibility surface, not something Chapter 8's
GIME-native pipeline touches. But these are the modes most CoCo software of
the era actually drew in, and they hide a genuinely strange piece of
hardware history: **two separate chips decided two separate axes of the same
picture, and they never had to agree.**

The horizontal geometry — how many bytes get fetched per row, how many bits
each pixel takes, which color set applies — comes entirely from PIA1
`$FF22`, the same register that also carries the VDG's
alphanumeric/semigraphics switch ([`video/graphics.rs:15-18`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/graphics.rs#L15-L18)):

```rust
/// PIA1 $FF22 bit 7: 1 = VDG graphics, 0 = alphanumeric/semigraphics.
pub const VDG_AG: u8 = 0x80;
/// PIA1 $FF22 bit 3: colour-set select (picks which GIME palette registers apply).
pub const VDG_CSS: u8 = 0x08;
```

plus three bits, `GM2:GM1:GM0`, in `$FF22` bits 6–4. Note where those bits
live: a PIA output port, not a video register. On a CoCo 1 or 2 the VDG had
no registers of its own at all — it was a chip with mode *pins*, and the way
software set a video mode was to drive those pins from a general-purpose
parallel output port. Chapter 10 builds the PIA that owns them.

But the *vertical* geometry — how many actual RAM rows get fetched before
the picture repeats, and how many times each fetched row gets redrawn to
fill the fixed 192-line active area — comes from an entirely different chip:
the SAM's `V0–V2` strobes at `$FFC0–$FFC5`, or, on a CoCo 3, the GIME's
SAM-compatibility overlay at the same addresses. Two chips, two halves of
one picture, no communication between them.

BASIC always programs matching `GM`/`V` pairs when a program executes
`PMODE n`, so on stock software the two axes always agree and this split is
invisible. But the split is real, and the module doc comment says so plainly
([`video/graphics.rs:1-11`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/graphics.rs#L1-L11)):

```rust
//! All VDG graphics modes scan out into the same 256×192 active area as text, so
//! lower-resolution modes are pixel-doubled to fill it. The horizontal decode (bytes
//! per row, bits per pixel, colour set) comes from PIA1 $FF22 (A/G, GM2–0, CSS); the
//! display base from the SAM page register; and the actual colours from the GIME
//! palette (SEB Fig 13). The *vertical* cadence (how many RAM rows are fetched, and
//! how many times each is repeated to fill the 192-line active area) instead comes
//! from the SAM V0–V2 bits — see [`LEGACY_GFX_LINES_PER_ROW`]. Real hardware doesn't
//! reconcile the two: if a program sets V and GM to a non-standard pairing, the
//! vertical cadence follows V and the horizontal decode follows GM independently.
```

"Real hardware doesn't reconcile the two" is not a defensive disclaimer —
it's a fact this codebase deliberately preserves rather than papers over.
There is a real temptation to paper over it. A decode function that took
only `$FF22` and derived a whole mode from it would be simpler, would give a
"sensible" answer for every input, and would be wrong. A program that pokes
`$FF22` and the SAM V strobes out of their documented pairing — whether by a
bug, or on purpose while hunting for an undocumented mode — gets whatever
the independent combination of the two axes actually produces on real
silicon. `decode_vdg_graphics` reproduces that rather than "helpfully"
snapping to the nearest legal `PMODE`, and this section's last test proves
it does.

### The SAM V bits: vertical cadence

Take the vertical axis first, because it is the smaller of the two and the
one with no BASIC-visible name at all ([`video/graphics.rs:45-50`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/graphics.rs#L45-L50)):

```rust
/// Lines-per-row for CoCo-compatible legacy graphics, indexed by the SAM V bits
/// packed `V2:V1:V0` (0–7). Hardware-verified (MAME
/// `gime_legacy_lines_per_row_graphic`): RAM rows fetched = `ACTIVE_H /
/// LEGACY_GFX_LINES_PER_ROW[v]` (64, 96, or 192), each repeated this many times
/// vertically to fill the 192-line active area.
pub const LEGACY_GFX_LINES_PER_ROW: [usize; 8] = [3, 3, 3, 2, 2, 1, 1, 1];
```

Read this table as "how many times each fetched RAM row gets redrawn before
the scan moves to the next one." `V=%000..010` (values 0–2) repeat every row
three times, so `192 / 3 = 64` RAM rows are fetched for the whole screen.
`V=%011..100` repeat twice, so 96 rows are fetched. `V=%101..111` repeat
once — the full 192 rows, one RAM row per scanline, no vertical doubling at
all.

The reason this axis exists is memory. Video RAM was the scarcest resource
on a 16K machine, and the vertical cadence is the knob that decides how much
of it a screen costs: a 64-row screen occupies a third of the RAM of a
192-row one and looks chunkier by exactly that factor. Every step down the
table is a memory-versus-detail trade, made at a time when the difference
mattered enormously.

What makes it strange is that it is *entirely independent* of how wide a row
is or how many colors it holds. A mode can be simultaneously "128 pixels
wide, 2 colors" — a horizontal decision, from `$FF22` — and "every RAM row
shown three times" — a vertical decision, from the SAM. That particular
pairing is exactly `PMODE 0` (`RG2` with `V=%000`), and the fact that it
takes two chips to say so is the whole point of this section.

### The GM bits: horizontal decode, and the PMODE table

The horizontal axis is where the familiar names live. Its decode produces a
small struct describing the mode's geometry
([`video/graphics.rs:52-63`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/graphics.rs#L52-L63)):

```rust
/// A decoded VDG resolution-graphics mode.
pub struct VdgGraphicsMode {
    /// Bytes fetched per displayed row.
    pub bytes_per_row: usize,
    /// RAM rows fetched (before vertical repetition into [`ACTIVE_H`]); driven by
    /// the SAM V bits, not the GM bits (see [`LEGACY_GFX_LINES_PER_ROW`]).
    pub rows: usize,
    /// Bits per pixel: 1 = 2 colours, 2 = 4 colours.
    pub bpp: usize,
    /// Logical pixels across (before horizontal doubling into [`ACTIVE_W`]).
    pub logical_w: usize,
}
```

Four fields, and the doc comments carefully label which register each one
descends from — `rows` is called out explicitly as coming from the SAM
rather than the GM bits, in a struct otherwise full of `$FF22`-derived
values. That labelling is the module's earlier warning made local: the type
mixes both axes, so each field says which chip it belongs to.

The function that fills it in is a single `match`
([`video/graphics.rs:75-96`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/graphics.rs#L75-L96)):

```rust
pub fn decode_vdg_graphics(ff22: u8, sam_video: u8) -> VdgGraphicsMode {
    let gm = (ff22 & VDG_GM_MASK) >> VDG_GM_SHIFT;
    // (logical width, 4-colour?) for GM2..GM0 = 0..7.
    let (logical_w, four_colour) = match gm {
        0 => (64, true),   // CG1
        1 => (128, false), // RG1
        2 => (128, true),  // CG2
        3 => (128, false), // RG2  (PMODE 0)
        4 => (128, true),  // CG3  (PMODE 1)
        5 => (128, false), // RG3  (PMODE 2)
        6 => (128, true),  // CG6  (PMODE 3)
        _ => (256, false), // RG6  (PMODE 4)
    };
    let bpp = if four_colour { 2 } else { 1 };
    let lines_per_row = LEGACY_GFX_LINES_PER_ROW[(sam_video & SAM_VIDEO_MASK) as usize];
    VdgGraphicsMode {
        bytes_per_row: logical_w * bpp / 8,
        rows: ACTIVE_H / lines_per_row,
        bpp,
        logical_w,
    }
}
```

Two parameters, consulted in complete isolation from one another: `ff22`
decides `logical_w` and `bpp`, `sam_video` decides `lines_per_row`, and no
line of the function lets either influence the other. The independence is
not enforced by a rule; it is the natural shape of a function that reads two
inputs and never compares them.

Lay the whole `GM` table out with the BASIC name — the number every CoCo
owner actually typed — and the horizontal geometry `decode_vdg_graphics`
derives from it:

| `GM` | MC6847 name | BASIC | `logical_w` | colors | `bpp` |
|---|---|---|---|---|---|
| `000` | CG1 | — | 64 | 4 | 2 |
| `001` | RG1 | — | 128 | 2 | 1 |
| `010` | CG2 | — | 128 | 4 | 2 |
| `011` | RG2 | `PMODE 0` | 128 | 2 | 1 |
| `100` | CG3 | `PMODE 1` | 128 | 4 | 2 |
| `101` | RG3 | `PMODE 2` | 128 | 2 | 1 |
| `110` | CG6 | `PMODE 3` | 128 | 4 | 2 |
| `111` | RG6 | `PMODE 4` | 256 | 2 | 1 |

Several things jump out of that table once it is laid flat.

The naming convention decodes itself: `RG` modes are "resolution graphics,"
two colors, one bit per pixel; `CG` modes are "color graphics," four
colors, two bits per pixel. The pattern alternates all the way down, which
is why `PMODE`'s odd numbers and even numbers behave so differently — the
`PMODE` numbering walks up the `GM` values one at a time, so it alternates
between two-color and four-color on every step.

`PMODE 0`–`4` only ever reaches `GM` values 3–7. `CG1`, `RG1`, and `CG2`
(`GM` 0–2) exist on the chip and in this decode table, but stock Extended
Color BASIC's `PMODE` statement never programs them. They were reachable
only by POKEing `$FF22` directly, which is exactly the kind of undocumented-
mode corner the module doc comment was talking about — and exactly the kind
of thing magazine listings did on purpose.

And `bytes_per_row = logical_w * bpp / 8` is the whole horizontal
byte-count story. `PMODE 4` (`RG6`, 256 pixels × 1 bit) is `256/8 = 32`
bytes per row; `PMODE 3` (`CG6`, 128 pixels × 2 bits) is `128×2/8 = 32`
bytes too. Same memory footprint per row, resolution traded against color
depth — the classic trade-off table, and one a period programmer could feel
directly, because both modes cost the same 6K of a machine that had 16K.
(That 6K is `32 bytes × 192 rows`, and it is why `PMODE 4` screens and
`PMODE 3` screens could be swapped for each other without rearranging
memory.)

The geometry test states three of those rows as executable fact
([`tests/render_graphics.rs:36-46`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_graphics.rs#L36-L46)):

```rust
#[test]
fn decodes_pmode_geometry() {
    let m = decode_vdg_graphics(RG6, V_RG6);
    assert_eq!((m.bytes_per_row, m.rows, m.bpp, m.logical_w), (32, 192, 1, 256));

    let m = decode_vdg_graphics(CG6, V_CG6);
    assert_eq!((m.bytes_per_row, m.rows, m.bpp, m.logical_w), (32, 192, 2, 128));

    let m = decode_vdg_graphics(RG3, V_RG3);
    assert_eq!((m.bytes_per_row, m.rows, m.bpp, m.logical_w), (16, 192, 1, 128));
}
```

Asserting the four fields as one tuple rather than four separate
`assert_eq!` calls is a small choice with a real payoff: a failure prints
the entire actual geometry next to the entire expected geometry, so a wrong
`bpp` is diagnosed in one line instead of after three passing assertions and
a fourth that only says `2 != 1`.

Now the vertical half of those pairings, which is where the codebase is
careful about what it knows. The test file's own comment documents the exact
SAM `V` value BASIC's `PMODE` setup pairs with three of these modes
([`tests/render_graphics.rs:30-34`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_graphics.rs#L30-L34)):

```rust
// SEB Unravelled II's GM/V pairing table: the SAM V value BASIC's PMODE setup
// programs alongside each PIA1 $FF22 GM value.
const V_RG6: u8 = 0b111; // paired with GM=111 (RG6 / PMODE 4)
const V_CG6: u8 = 0b110; // paired with GM=110 (CG6 / PMODE 3)
const V_RG3: u8 = 0b101; // paired with GM=101 (RG3 / PMODE 2)
```

Run those three through `LEGACY_GFX_LINES_PER_ROW` and each `PMODE`'s real
vertical cadence falls out. `LEGACY_GFX_LINES_PER_ROW[0b101]`,
`[0b110]`, and `[0b111]` are all `1`, so `PMODE 2`, `PMODE 3`, and `PMODE 4`
all fetch the full `192/1 = 192` RAM rows with no vertical repetition at
all — which is exactly the `rows: 192` the geometry test asserts three times
over.

`PMODE 0` and `PMODE 1`'s exact `V` pairing isn't named anywhere in this
codebase's source or test comments, so this chapter won't invent one. Their
vertical cadence is still bounded by which *bucket* of
`LEGACY_GFX_LINES_PER_ROW` values (`3` or `2`) the real hardware's
documented `PMODE`-to-`V` mapping falls into, and reasoning about a mode the
chapter didn't hand you the answer for is precisely what §9.17's exercise
9.5 asks you to do. Leaving a gap visible is better than filling it with a
plausible guess that a reader would then have no way to tell apart from the
verified rows.

### The V strobes: three more write-only address pairs

The V bits reach the GIME the same way every other SAM-compatibility bit
does — not as a value written to a register, but as an *address* touched.
[`tests/sam_video.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/sam_video.rs)
tests that half independently of any graphics decode:

```rust
#[test]
fn vdg_strobes_latch_sam_video_bits_independently() {
    let mut b = bus();
    assert_eq!(b.gime.sam_video, 0, "V bits reset to 0");
    b.write(V0_SET, 0); // written data is ignored — only the address matters
    assert_eq!(b.gime.sam_video, 0b001);
    b.write(V2_SET, 0xFF);
    assert_eq!(b.gime.sam_video, 0b101);
    b.write(V1_SET, 0);
    assert_eq!(b.gime.sam_video, 0b111);
    b.write(V0_CLEAR, 0);
    assert_eq!(b.gime.sam_video, 0b110);
    b.write(V2_CLEAR, 0);
    b.write(V1_CLEAR, 0);
    assert_eq!(b.gime.sam_video, 0);
}
```

The comment on the first write is the entire design of the SAM's register
interface in eight words. `write(V0_SET, 0)` sets bit 0; `write(V2_SET, 0xFF)`
sets bit 2; the value written is discarded in both cases, and only the
*address* — `$FFC1` and `$FFC5`, the odd halves of two strobe pairs —
carries information. Three bits, six addresses, each pair being one "set
this bit" address and one "clear this bit" address. Chapter 5 introduced this
pattern for the page-select bits; the V bits are three more of exactly the
same shape.

Writing the sequence out as set-set-set then clear-clear-clear, checking the
packed value after each step, is what makes the test say "independently" and
mean it: each strobe changes exactly one bit and leaves the other two alone.
A decode bug that wired two strobes to the same bit would pass a test that
only ever set one bit at a time.

A companion test extends the same suspicion outward, to the boundary between
this strobe range and the next one
([`tests/sam_video.rs:45-59`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/sam_video.rs#L45-L59)):

```rust
#[test]
fn vdg_strobes_do_not_disturb_the_adjacent_page_bits() {
    // F0 ($FFC6) is the very next strobe pair after V2 ($FFC4/$FFC5) — confirm
    // the ranges don't bleed into each other.
    let mut b = bus();
    b.write(V0_SET, 0);
    b.write(V1_SET, 0);
    b.write(V2_SET, 0);
    assert_eq!(b.gime.sam_video, 0b111);
    assert_eq!(b.gime.sam_page, 0, "page bits untouched by V strobes");

    b.write(0xFFC7, 0); // F0 set
    assert_eq!(b.gime.sam_page, 0b01);
    assert_eq!(b.gime.sam_video, 0b111, "V bits untouched by a page strobe");
}
```

This is cheap insurance against an off-by-one in the address decode that
would otherwise be invisible for a long time. Both `sam_video` and
`sam_page` are just integer fields on `GIME`; a decode that routed `V2_SET`
into `sam_page` instead would compile without complaint, pass every test
that checks one field in isolation, and show up eventually as a screen
fetched from the wrong page with the wrong number of rows. Testing the
*seam* between two adjacent address ranges — rather than testing each range
comfortably in its middle — is where address-decode bugs actually live.

### Worked example: unpacking pixels

Geometry decided, the remaining question is what a byte of video RAM
actually becomes. [`tests/render_graphics.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_graphics.rs)
exercises the bit-unpacking at both depths, using single-byte inputs chosen
so the expected output can be read straight off the binary literal:

```rust
#[test]
fn two_color_unpacks_msb_first_with_border() {
    let mode = decode_vdg_graphics(RG6, V_RG6); // 1:1, no scaling
    let mut data = vec![0u8; mode.bytes_per_row * mode.rows];
    data[0] = 0b1000_0000; // only the leftmost pixel is colour 1
    let mut fb = fb();
    render_graphics(&data, &mode, &[C0, C1], BD, &mut fb);

    assert_eq!(px(&fb, 0, 0), BD, "corner is border");
    assert_eq!(px(&fb, BORDER, BORDER), C1, "MSB pixel = colour 1");
    assert_eq!(px(&fb, BORDER + 1, BORDER), C0, "next pixel = colour 0");
}

#[test]
fn four_color_maps_two_bit_values_and_doubles_width() {
    let mode = decode_vdg_graphics(CG6, V_CG6); // 128 wide → hscale 2
    let mut data = vec![0u8; mode.bytes_per_row * mode.rows];
    data[0] = 0b00_01_10_11; // pixel values 0,1,2,3 left→right
    let mut fb = fb();
    render_graphics(&data, &mode, &[C0, C1, C2, C3], BD, &mut fb);

    // Each logical pixel is 2 host pixels wide.
    assert_eq!(px(&fb, BORDER, BORDER), C0);
    assert_eq!(px(&fb, BORDER + 1, BORDER), C0, "pixel 0 doubled");
    assert_eq!(px(&fb, BORDER + 2, BORDER), C1);
    assert_eq!(px(&fb, BORDER + 4, BORDER), C2);
    assert_eq!(px(&fb, BORDER + 6, BORDER), C3);
}
```

Both tests use Rust's binary literals with underscores placed at the
*semantic* boundaries, a choice that turns each input into its own
documentation. `0b1000_0000` for `RG6` at one bit per pixel lights exactly
the leftmost of eight pixels. `0b00_01_10_11` for `CG6` at two bits per
pixel packs four pixel values — `00`, `01`, `10`, `11` — left to right in
one byte, grouped so a reader can see the four pixels without counting
bits. Choosing test data that reads correctly is worth more than choosing
test data that looks realistic.

Both follow the same "MSB first" rule, and the unpacking loop encodes it
directly ([`video/graphics.rs:112-132`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/graphics.rs#L112-L132)):

```rust
pub fn paint_legacy_graphics_line(
    row_data: &[u8],
    mode: &VdgGraphicsMode,
    colors: &[[u8; 4]],
    xscale: usize,
    out: &mut [u8],
) {
    let pixels_per_byte = 8 / mode.bpp;
    let mask = (1u8 << mode.bpp) - 1;
    let mut x = 0;
    for bx in 0..mode.bytes_per_row {
        let byte = row_data.get(bx).copied().unwrap_or(0);
        for j in 0..pixels_per_byte {
            // Pixels are packed MSB-first within the byte.
            let shift = 8 - mode.bpp * (j + 1);
            let value = ((byte >> shift) & mask) as usize;
            let color = colors[value.min(colors.len() - 1)];
            paint_px(out, &mut x, xscale, color);
        }
    }
}
```

`shift = 8 - bpp*(j+1)`: for `j = 0`, the first pixel in the byte, that's
`8 - bpp` — the *high* bits. "MSB first" isn't a comment, it's what the
shift arithmetic does, and the comment is there to save the next reader from
re-deriving it. The rest of the loop is defensive in two small ways worth
noticing: `row_data.get(bx).copied().unwrap_or(0)` treats a short row as
zeros rather than panicking, and `value.min(colors.len() - 1)` clamps a
pixel value to the palette actually supplied, so passing a two-entry color
slice to a four-color mode degrades instead of indexing out of bounds.
Neither should ever trigger with correct callers. Both turn a class of
future bug from a crash into a visible wrong color, which in a renderer is
the better failure.

The horizontal scaling is the last piece, and it explains something that
looks paradoxical in the geometry table. The second test's `PMODE 3` (`CG6`,
`logical_w = 128`) doubles horizontally to fill the fixed 256-pixel active
area, `hscale = ACTIVE_W / mode.logical_w = 256/128 = 2`, so every logical
pixel becomes two adjacent framebuffer pixels. That is why `PMODE 3`'s
128×192 four-color picture and `PMODE 4`'s 256×192 two-color picture both
fill the identical physical screen area despite having wildly different
logical resolutions. The doubling — or quadrupling, for the 64-wide `CG1`
mode — is baked into every mode's presentation and is completely invisible
to the program that set it up. A `PMODE 3` pixel is simply twice as wide as a
`PMODE 4` pixel, on the same screen, in the same inches of glass.

Which brings the section back to where it started. A fourth test in the same
file, `mismatched_v_and_gm_pairing_follows_v_for_vertical_cadence`, is the
module doc comment's disclaimer made concrete and checkable. It pairs
`GM=%111` — `RG6`'s 256-wide, 1-bpp horizontal decode — with `V=%011`, a
combination stock BASIC's `PMODE` setup never programs together, since
`RG6`'s documented partner is `V=%111`. It then confirms the vertical
cadence follows `V` (`LEGACY_GFX_LINES_PER_ROW[3] == 2`, so 96 rows fetched,
each shown twice) while the horizontal decode keeps following `GM` (still
256 pixels wide, one bit per pixel).

The test doesn't stop at the decoded geometry, either. It renders a field in
which each fetched row lights a different pixel of its first byte, then
asserts that each row's pixels appear identically on *both* of its two
active lines — the doubling actually happening, not merely being reported.
It finishes with an `assert_ne!` confirming that different fetched rows
really did produce different pixels, ruling out a stub that leaves
everything one color. The two axes really are that independent, proven by a
test that deliberately programs them out of their usual sync and then checks
the pixels rather than the struct.

## 9.13 Where the colors come from

Pixel *values* out of `paint_legacy_graphics_line` are small integers — `0`
or `1` for two-color modes, `0..3` for four-color — and those integers are
indices, exactly like GIME-native graphics. Indices into *what* is the last
open question in the legacy path, and its answer is where the CoCo 3's
imitation of a chip it doesn't contain becomes visible.

What a pixel value indexes into depends on bit depth and `CSS`, per SEB
Unravelled II's Figure 13, reproduced directly as two small compile-time
tables ([`video/graphics.rs:34-39`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/graphics.rs#L34-L39)):

```rust
/// Palette-register indices for 2-colour modes, indexed by CSS (SEB Fig 13):
/// CSS=0 → regs 8,9; CSS=1 → regs 10,11.
const G2_PALETTE_INDICES: [[usize; 2]; 2] = [[8, 9], [10, 11]];
/// Palette-register indices for 4-colour modes, indexed by CSS (SEB Fig 13):
/// CSS=0 → regs 0–3; CSS=1 → regs 4–7.
const G4_PALETTE_INDICES: [[usize; 4]; 2] = [[0, 1, 2, 3], [4, 5, 6, 7]];
```

The shapes repay a careful look, because they are the reason `CSS` felt like
a "color set" to anyone using it from BASIC. Each constant is an array of
two arrays: the outer index is `CSS`, the inner one is the pixel value. A
two-color mode has two color sets of two registers each; a four-color
mode has two sets of four. `CSS` never changes how many colors a mode has,
only *which* group of palette registers those colors are drawn from — one
bit selecting between two banks, which is precisely what "color set" meant
on the box.

Selecting between them is a function that hands back a borrowed slice
([`video/graphics.rs:98-106`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/graphics.rs#L98-L106)):

```rust
/// GIME palette-register indices for a VDG graphics mode, in pixel-value order
/// (SEB Fig 13). `css` is 0 or 1. Borrows a compile-time table — no allocation.
pub fn vdg_palette_indices(bpp: usize, css: usize) -> &'static [usize] {
    if bpp == 1 {
        &G2_PALETTE_INDICES[css]
    } else {
        &G4_PALETTE_INDICES[css]
    }
}
```

> **Rust corner: `&'static [T]` as a view into constant data.** The return
> type is a slice reference with a `'static` lifetime, which says something
> quite specific: the data lives for the entire duration of the program, and
> the caller is borrowing it rather than owning it.
>
> That lets one function return slices of two *different lengths* — two
> entries for a two-color mode, four for a four-color one — without
> allocating anything or committing to a fixed-size array type. A `Vec<usize>`
> would have worked and would have heap-allocated on a path that runs once
> per scanline. A `[usize; 4]` would have forced the two-color case to pad
> with meaningless values and the caller to know how many to trust.
>
> `'static` is the right lifetime here for a reason worth internalizing: the
> referent is a `const` table baked into the binary, so there is no owner
> that could drop it and no borrow that could outlive it. The doc comment's
> closing phrase — "Borrows a compile-time table — no allocation" — is the
> whole justification, and the signature enforces it. Whenever a function
> needs to return "one of several fixed tables, chosen at run time," this is
> the shape to reach for.

The mapping is confirmed against `render_graphics.rs`'s own lookup test,
which is short enough to check by eye against the two constants above:

```rust
#[test]
fn palette_indices_follow_css_and_depth() {
    assert_eq!(vdg_palette_indices(1, 0), [8, 9].as_slice());
    assert_eq!(vdg_palette_indices(1, 1), [10, 11].as_slice());
    assert_eq!(vdg_palette_indices(2, 0), [0, 1, 2, 3].as_slice());
    assert_eq!(vdg_palette_indices(2, 1), [4, 5, 6, 7].as_slice());
}
```

Four assertions, four rows of SEB's figure. This is the kind of test that
looks trivial and earns its keep the first time someone "simplifies" the
two tables into one and gets the `CSS` indexing backwards.

Now the part that makes the CoCo 3 different from the machine it is
imitating. On a real MC6847 — the actual CoCo 1/2 chip — those eight colors
per depth are wired to a fixed internal ROM. The chip itself decides what
"color 2 of `RG3`" looks like, and no program can change it, which is why
`video::VDG_FIXED_PALETTE` exists as a separate, hardcoded RGB table for
that variant. On a CoCo 3 there is no MC6847 at all: the GIME's
compatibility path routes those *same* eight palette-register indices —
`0–7` for four-color modes, `8–11` for two-color — through its own sixteen
programmable registers, the very registers you already know from GIME-native
modes and from `PALETTE`.

`Machine::legacy_palette` ([`machine/video_mode.rs:62-75`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/video_mode.rs#L62-L75))
is the one place that branches on variant:

```rust
pub(super) fn legacy_palette(&self, css: bool) -> [[u8; 4]; video::PALETTE_LEN] {
    match self.config.variant {
        MachineVariant::Coco3 => {
            let mut resolved = [[0u8; 4]; video::PALETTE_LEN];
            for (i, entry) in resolved.iter_mut().enumerate() {
                *entry = self.bus.gime.color(self.bus.gime.palette[i]);
            }
            video::ColorSource::GIMEPalette(&resolved).resolve(css)
        }
        MachineVariant::Coco1 | MachineVariant::Coco2 => {
            video::ColorSource::VDGFixed.resolve(css)
        }
    }
}
```

Two consequences are worth naming, and both matter for the rest of the
course.

First, look at what resolves each register on the CoCo 3 arm:
`self.bus.gime.color(...)`, not `rgb_color` directly. Legacy `PMODE`
graphics therefore run through the exact same RGB-versus-composite fork that
the first half of this chapter was about. A `PMODE 4` screen looks different
on an RGB CoCo 3 than on a composite one, for precisely the same reason an
`HSCREEN` picture does — and the code path proving it is one method call, in
a function whose subject is a chip from 1980. The single-`color`-function
claim from §9.1 covers even the compatibility modes.

Second, `PMODE` colors are *programmable* on a CoCo 3 in a way they
categorically aren't on a real CoCo 1 or 2. A BASIC program can `PALETTE`
its way to `RG6` colors no MC6847-equipped machine could ever produce,
because the chip that decides what color index 1 means changed from a fixed
ROM into a register file — while the pixel-index arithmetic upstream of it
stayed bit-for-bit identical on both machines. That is a precise statement
of what "compatible" meant for this generation of hardware: the same program
produces the same *pixels* and possibly different *colors*, and the machine
is considered compatible anyway.

## 9.14 The live per-line path vs. the whole-field snapshot

One more asymmetry is worth pointing out before moving on, because it
connects the legacy modes just covered straight back to §9.8–9.11's split
mechanism, and because it is the clearest example in the codebase of a
fidelity choice being made per machine variant rather than globally.

`Machine::paint_legacy_scanline` ([`machine/render.rs:75-171`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L75-L171))
is the CoCo 3's **per-line** legacy renderer — called from the very same
`render_scanline` dispatcher as the GIME-native path, once per canvas row,
reading `$FF22` and `sam_video` fresh every single line:

```rust
let ff22 = self.bus.pia1.b.output;
// ...
let ag = ff22 & video::VDG_AG != 0;
let css = ff22 & video::VDG_CSS != 0;
let sam_video = self.bus.gime.sam_video;
let (row_bytes, lines_per_row) = if ag {
    let mode = video::decode_vdg_graphics(ff22, sam_video);
    let lpr = video::LEGACY_GFX_LINES_PER_ROW[(sam_video & 0x07) as usize];
    (mode.bytes_per_row, lpr)
} else {
    (video::COLS, video::CELL_H)
};
```

Every register this function touches — `$FF22`, the SAM V bits, and the
border via `video::legacy_border_value(ff22)` — is read live, exactly like
the GIME-native "live" group from §9.8's table. And the function ends by
advancing the same shared cursor the native painter does
([`machine/render.rs:164-170`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L164-L170)):

```rust
        // Advance the shared vertical counter (MAME `record_full_body_scanline`).
        let scan = self.field_scan.as_mut().expect("legacy field latched");
        scan.line_in_row += 1;
        if scan.line_in_row >= lines_per_row {
            scan.line_in_row = 0;
            scan.row_base += row_bytes;
        }
```

That is `advance_scan`'s logic again, in the legacy path's own terms:
`lines_per_row` from the SAM V bits instead of `$FF98`'s LPR, `row_bytes`
from the `GM` decode instead of the HRES decode, and the same `FieldScan`
fields being stepped. The consequence is that a raster split on a
CoCo-3-in-legacy-mode screen — `PMODE`'s color set flipped, or the
graphics/alpha bit toggled, partway down the picture — works exactly as well
as it does in native mode, for the identical structural reason: nothing here
is cached across lines except the shared cursor, carried in the same
`FieldScan` this chapter has been reading about. Setting `field_scan.legacy
= true` at the `FieldScan::latch` call in `render_scanline` merely picks this
function instead of `gime_video::paint_scanline`.

Contrast that with a real **CoCo 1/2** — no GIME, no `render_scanline`
dispatch, no per-line anything. `Machine::render_field` returns immediately
for a CoCo 3 (`if self.config.variant == MachineVariant::Coco3 { return; }`)
and only does real work for the older machines, and what it does is render
the *entire* field in one shot at field end, from a single snapshot read
through the bus ([`machine/render.rs:229-279`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L229-L279)):

```rust
fn render_coco_graphics(&mut self) {
    self.reset_legacy_fb();
    let ff22 = self.bus.pia1.b.output;
    let sam_video = match self.config.variant {
        MachineVariant::Coco3 => self.bus.gime.sam_video,
        MachineVariant::Coco1 | MachineVariant::Coco2 => self.bus.sam.v_bits(),
    };
    let mode = video::decode_vdg_graphics(ff22, sam_video);
    // ... one shot: fetch mode.bytes_per_row * mode.rows bytes, render once
```

This isn't an oversight, and it isn't laziness either — it's a scope
decision with a reason attached. A real CoCo 1 or 2 has no GIME sitting
between the CPU and the VDG, so there is no canonical 640×240 raster to
paint into one line at a time in the first place. The VDG has its own,
entirely separate raster geometry (`docs/coco12-plan.md` Phase 3, cited
directly in the comments here), and this codebase renders it as a
whole-field snapshot rather than building a second per-scanline scanout
model for a display chip this course never sets out to model.

Note also what the excerpt shows about code reuse across that boundary:
`decode_vdg_graphics` is called by both paths, and only the *source* of
`sam_video` differs — the GIME's SAM-compat overlay on a CoCo 3, the real
`SAM` type on a CoCo 1/2, exactly the two implementations of one legacy
interface that Chapter 1 pointed at. The decode itself is shared, so the two
machines cannot disagree about what `GM=%110` means.

Practically, then: **mid-field raster splits are a CoCo 3 phenomenon in this
emulator**, on both the GIME-native and the CoCo-3-legacy paths, because both
go through `render_scanline`. A CoCo 1/2 target — real VDG hardware,
whole-field renderer here — cannot show one in this codebase today, though
nothing about real VDG hardware rules it out; the actual chip scans a real
raster continuously, exactly as the GIME does. It's a scope line this
codebase draws deliberately, not a hardware fact, and stating which is which
is the difference between a documented limitation and a bug nobody has
noticed yet.

## 9.15 Artifact colors: what the code actually does

One phrase belongs in this chapter precisely because CoCo veterans expect it
and the code needs to be honest about it either way: **NTSC composite
artifact colors** — the famous trick where `PMODE 4`'s black-and-white
checkerboard of alternating 1-bit pixels resolves, on a real composite
display, into *colored* fringes along vertical edges that were never
programmed as any palette value at all. This happens because a real NTSC
composite decoder derives chroma from how *rapidly* the luminance signal
changes between adjacent pixels: a fine alternating black/white pattern
looks, to the subcarrier-phase math, exactly like a saturated color, even
though the video hardware only ever "meant" to send two shades of gray.
Software exploited it constantly on real CoCo hardware to fake extra colors
out of a nominally two-color mode — getting four colors out of `PMODE 4`
for the price of one bit per pixel was too good a bargain to leave alone.

Searching this codebase for that mechanism comes up empty. The string
`"artifact"` doesn't appear anywhere in `coco-core`'s video code, and having
now read both color paths end to end, it's clear why: **there is no
pixel-pattern-dependent color synthesis anywhere in this renderer.**

The evidence is all in place already. `GIME::color` (§9.2–9.5) is a pure
function of a single 6-bit register value, with no visibility into what
color the *neighboring* pixel resolved to. The two composite tables model
what a monitor does with a **fully-formed 6-bit color value** the GIME's
video DAC already decided to output — the analog decode of an *intentional*
color choice, not the decode of a *pattern* of luminance-only pixels that
was never meant to carry color at all. And every legacy graphics pixel
§9.12 walked through, `RG6`'s black-and-white bits included, resolves
through the same `vdg_palette_indices` → `legacy_palette` → `GIME::color`
chain as everything else on screen.

So if a program sets a two-color mode's palette registers to plain black
and white, this emulator renders plain black and white, full stop — no
matter what bit pattern is in video RAM, and regardless of `MonitorType`.
A real composite television showing that exact same signal would not.

That's a real, honest gap, and it's worth being precise about *why* it's a
gap rather than a bug. Implementing genuine artifact color would mean
rewriting the innermost loop of `paint_legacy_graphics_line` — and its
GIME-native `paint_graphics_row` cousin, for `HSCREEN`'s composite path — to
stop being a per-pixel color lookup and become a small sliding-window NTSC
decoder. It would have to track several consecutive pixels' luminance
values, their position relative to the color subcarrier's phase (which
advances a fixed, non-integer amount per pixel clock, so the *same* bit
pattern artifacts differently depending on which screen column it starts
at), and synthesize chroma from the transition pattern rather than looking
anything up in a 64-entry table at all.

That is a materially different algorithm from everything else in this
chapter. Every other color decision in this codebase is `O(1)` per pixel
with no neighbor dependence, which is what makes the whole renderer a
sequence of independent lookups; artifact color is inherently
neighbor-dependent, and adding it changes the shape of the loop rather than
the contents of a table. It also only matters when `MonitorType::Composite`
is selected, and only for the subset of legacy two-color graphics modes
where programs relied on the trick.

It is exactly the kind of deferred-scope decision Appendix C exists for:
real, named, sized, and left for later — because the 64-entry table
correctly covers every *intentional* color choice a program makes, and only
misses the *unintentional* color a real analog television invents from a
bit pattern nobody asked it to color at all. Naming a gap that precisely is
worth more than a vague "composite is approximate," because it tells the
next person exactly which programs would notice and exactly what work
closing it would take.

One more loose thread is worth pulling while the subject is bits that the
code sees but doesn't act on. `$FF98` has five named fields, and it is worth
seeing them together ([`gime.rs:70-83`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L70-L83)):

```rust
/// Video Mode Register ($FF98) bit assignments (SEB Unravelled II). Only meaningful
/// when INIT0 COCO=0 (GIME native modes); ignored in CoCo-compatible mode.
pub mod vmode {
    /// Bit-plane / graphics select: 1 = graphics (HSCREEN), 0 = hi-res text.
    pub const BP: u8 = 0x80;
    /// Burst phase invert (alternate composite colour set).
    pub const BPI: u8 = 0x20;
    /// Monochrome on composite output.
    pub const MOCH: u8 = 0x10;
    /// 50 Hz field rate (else 60 Hz).
    pub const H50: u8 = 0x08;
    /// Lines per character row (text modes).
    pub const LPR_MASK: u8 = 0x07;
}
```

Three of those five have appeared already: `BP` split a field between text
and graphics in §9.9, and `BPI` and `MOCH` are the two bits `GIME::color`
consults on the composite path. `LPR_MASK` is Chapter 8's character-row height.
`H50` is the odd one out — it is parsed, named, documented, and never
*read* anywhere else in the crate.

Field rate in this codebase comes from `MachineConfig`'s `VideoStandard`
(`NTSC` or `PAL`), chosen once at machine-configuration time, the same
moment as `MonitorType` — not from this live register that a real GIME lets
software toggle mid-operation. That makes `H50` a small, sharp instance of
the same category as artifact color: not a missing bit definition, but a
bit whose live behavior would break a structural assumption several
subsystems deep. Section 9.17's essay exercise asks you to work out exactly
which assumption, and Appendix C records the answer as a scoped, named gap
rather than an oversight.

---

## 9.16 Reading assignment

This week's reading is unusually satisfying to do in order, because each
file answers a question the previous one raises. Roughly two hours, and the
two test files in the middle are the ones to read most slowly.

1. **[`crates/coco-core/src/gime/palette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs), all of it** (88 lines) — both
   composite tables, `unpack_rgb`, `rgb_color`, `GIME::color`. Small enough
   to read in one sitting; everything in §9.1–9.7 traces back to this file.
   Read the two tables side by side and confirm for yourself that entries 0,
   16, 32, and 63 are identical in both, which is §9.4's structural check.
2. **[`crates/coco-core/tests/composite.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/composite.rs), all of it** — five focused unit
   tests plus the `eou_greyscale_regression` war story. Pay attention to how
   differently `composite_decode_grey_anchors` and
   `eou_greyscale_regression` are written: exact values versus asserted
   properties, and §9.6's argument for why both belong in one file. Run it
   and watch every assertion you just read pass:
   ```
   cargo test -p coco-core --test composite
   ```
3. **[`crates/coco-core/src/gime_video.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs), `FieldScan` and `paint_scanline`**
   ([`gime_video.rs:117–299`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L117-L299)) — re-read `advance_scan` in particular; it's
   the one function that's *not* purely "live" or purely "latched," and
   understanding why (cursor advances live, origin frozen) is the key to
   the fourth [`scanline_split.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/scanline_split.rs) test. A useful exercise while reading:
   for each register the file touches, decide which row of §9.8's taxonomy
   table it belongs in *before* checking.
4. **[`crates/coco-core/tests/scanline_split.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/scanline_split.rs), all of it** — five tests,
   the whole chapter's central claim made executable. The first four are
   harness-driven and take a minute each; the fifth is the hand-assembled
   ROM from §9.10 and rewards being read with a 6809 instruction card next
   to it.
   ```
   cargo test -p coco-core --test scanline_split
   ```
5. **[`crates/coco-core/src/video/graphics.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/graphics.rs), all of it** (178 lines) —
   `decode_vdg_graphics`, the palette-index tables, the MSB-first unpacker.
   Start with the module doc comment; it states the two-chip independence
   this chapter's §9.12 is built around, in nine lines.
6. **[`crates/coco-core/tests/sam_video.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/sam_video.rs)** and **[`tests/render_graphics.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_graphics.rs)**,
   all of both — the SAM V-strobe tests and the PMODE-decode worked
   examples. The last test in `render_graphics.rs` is the mismatched-pairing
   one; read its comment block before its code.
   ```
   cargo test -p coco-core --test sam_video --test render_graphics
   ```
7. **[`crates/coco-core/src/machine/render.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs)**, all of it — `render_scanline`,
   `paint_legacy_scanline`, and the contrast with the CoCo 1/2 whole-field
   `render_coco_graphics`/`render_coco_text`. This is the file where the
   chapter's two halves meet: one dispatcher, three renderers, one
   `FieldScan`.

---

## 9.17 Exercises

**9.1 — Predict the split (recall, then verify).** Without re-reading
§9.9, answer from memory: a program writes the border color ($FF9A) at
scanline 100. A different program writes the video base ($FF9D/$FF9E) at
scanline 100. Sketch what each field looks like — where (if anywhere) does
each field show a visible seam? Now name the exact two tests in
`scanline_split.rs` that prove your answer for each case, and run them:
```
cargo test -p coco-core --test scanline_split border_write_mid_field_splits_the_border_at_the_line
cargo test -p coco-core --test scanline_split video_base_write_mid_field_waits_for_the_next_field
```
If your prediction for the video-base case was "it also splits mid-field,"
find the exact line in `FieldScan::latch` that makes that impossible, and
explain in one sentence why the test plants *two* marker bytes instead of
one.

**9.2 — Composite table lookup drill (read, by hand).** Using only the
`COMPOSITE_PALETTE` array printed in §9.3 — no running code yet — decode
palette values `0x05`, `0x15`, `0x25`, and `0x35` (hue 5, all four
luminance steps) into their `(R, G, B)` triples. State whether the four
results are achromatic or chromatic, and whether brightness increases
monotonically the way the `0x00/0x10/0x20/0x30` gray ramp does. Then write a
two-line test (or a `cargo test -p coco-core --test composite -- --nocapture`
scratch assertion) confirming your hand-decoded values against
`GIME::color`. Finally: compute the *hue* (standard HSV formula) of `0x05`
and of `COMPOSITE_PALETTE_180[0x05]`, and check whether the delta is closer
to 120° or 180° — does this entry support or complicate the "BPI is a clean
180° invert" claim from §9.4?

**9.3 — Sabotage the field latch, then verify (sabotage — do this for
real).** In `gime_video.rs::paint_scanline`, the body-row fetch passes
`scan.row_base` (the field-latched origin) into `paint_body_row`. Change
that one call site to pass `g.video_base()` instead — bypassing the latch
entirely, re-reading the live vertical-offset registers on every line. Run:
```
cargo test -p coco-core --test scanline_split
```
Exactly one test should fail:
`video_base_write_mid_field_waits_for_the_next_field`. Before reading
further, predict what the failure message will show. The actual result is
more surprising than "shows palette 7 (the new base) instead of palette 5":
the sabotaged code reports `left: [0, 0, 0, 255]` — plain black, neither
candidate color. Explain why, in terms of what else `scan.row_base`'s
per-row *advancement* (`advance_scan`) was doing that a bare
`g.video_base()` call does not. (Hint: `advance_scan` still runs and still
updates `scan.row_base` — but nothing reads that field anymore once the
fetch bypasses it. What address does every single row fetch from, now, no
matter how far down the field it is?) Revert your change and confirm
`git status` is clean and the full suite passes again before moving on.

**9.4 — Build a three-band border (build).** Extend
`border_write_mid_field_splits_the_border_at_the_line` (copy it to a new
test, don't just edit the original) to produce **three** visible border
bands in one field, not two: pick two split lines (e.g. 60 and 160, both
inside the 25..217 active body from `vertical_window`), write a different
border color at each via `run_to_line`, and assert all three colors
appear at the right canvas rows with exactly two transitions in the left
border column (reuse the `transitions` scan from §9.10's FIRQ test as a
model, or just sample three specific rows directly). No ROM code required —
this is a pure register-poke test in the same style as the other four in
the file. Get it passing, then delete it (or keep it if you'd rather send a
PR) — the exercise is in building it and predicting *before running* which
of the two splits you expect to fall between rows.

**9.5 — PMODE geometry drill (recall, then verify).** From the table in
§9.12 and `LEGACY_GFX_LINES_PER_ROW`, work out — without looking anything up
— `bytes_per_row`, `rows`, `bpp`, and `logical_w` for: (a) `PMODE 2` (`RG3`)
with its documented `V=%101` pairing; (b) `GM=%110` (`CG6`) paired with a
*mismatched* `V=%000` instead of its documented `%110`. Then check both
against `decode_vdg_graphics` — for (a), against `render_graphics.rs`'s
`decodes_pmode_geometry` test directly; for (b), by calling
`decode_vdg_graphics(CG6, 0b000)` yourself (`CG6` is `AG | (6 << 4)` from
that test file) and inspecting the result. State in one sentence which axis
of your (b) answer changed from the "textbook" `CG6` geometry (still `V=%110`)
and which didn't — and why a program that meant to select `PMODE 3` but
fat-fingered only the SAM strobes, leaving `$FF22` untouched, would see a
picture that's still 128 pixels wide and 4 colors, just fetched from a
third as much RAM (64 rows instead of 192) with each fetched row repeated
three times vertically instead of drawn once.

**9.6 — Essay, five sentences max (no running code).** `vmode::H50` ($FF98
bit 3) is a real, named bit — a real GIME lets software toggle 50 Hz/60 Hz
field rate live, mid-operation — but nothing in this crate ever reads it;
field rate comes entirely from a static `VideoStandard` chosen once at
machine-configuration time. Sketch what would have to change to honor a
live `H50` write instead: which fixed assumption in `end_of_line`/`run.rs`
(Chapter 6) would break first, and what would `render_scanline`'s canvas-row
math (§9.8, `CANVAS_H = 240` fixed) have to do differently mid-field if the
line count *itself* could change under it? You do not need to implement
this — the point is naming the load-bearing assumption that a "just read the
register live, like the border" fix would violate, and that a border write
never touches.

---

## What's next

Video is done. You now own the whole visible half of the machine: raster
timing (Chapter 6), VDG text and the surprise CoCo-3-boots-in-VDG-mode fact
(Chapter 7), GIME native text and graphics and the palette register file
(Chapter 8), and — this week — the two ways that register file lies to a naive
whole-field renderer. It means something different depending on a cable
nobody can query, and it can change its mind mid-field in ways only some of
its registers are allowed to respect.

Two habits from this week are worth carrying forward, because both recur in
every device still to come. The first is separating *what a register says*
from *what the hardware downstream of it does*, which is what made composite
output a table rather than a renderer. The second is asking, of every piece
of device state, whether it is read live or sampled once — `FieldScan` is
the video answer to that question, and the cassette, the floppy controller,
and the serial port each have their own version of it.

Every remaining chapter through Chapter 14 is a different device hanging off
the same bus you mastered in Chapter 5, each with its own version of "state, a
loop, and a seam" from Chapter 1. Chapter 10 starts the input side: the PIAs,
whose output ports this chapter has already read `$FF22` from without ever
building one, the keyboard matrix, and a joystick that has no ADC chip at
all — just a comparator, a DAC, and 1980-style software doing
successive approximation one bit at a time.
