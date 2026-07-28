# Chapter 12 — The Cassette: FSK Modems, 1980 Edition

*Week 12. Goal: a complete, self-contained signal-processing story, start to
finish, in one subsystem small enough to hold in your head. Chapter 10 gave you
the PIA — the chip that mediates almost all CoCo I/O — and Chapter 11 walked
the audio path the PIA's DAC feeds. This week reuses both: the cassette
"modem" is nothing but a PIA pin and a DAC register, driven by software.
By the end of this chapter you will know exactly how a byte becomes a tone,
how a tone becomes a byte again, why the emulator has to model a mechanical
delay that has nothing to do with data at all, and — the chapter's best
lesson — what to do when the documentation you're relying on simply stops
covering the code you need. This closes Part IV.*

---

Every subsystem in this course so far has had a chip behind it. The 6809
has a data sheet. The GIME has a service manual and a register map. Both
PIAs have a published pinout that says, in so many words, what happens
when the CPU writes a byte to `$FF20`. Emulating those parts is careful,
literal work, but the shape of the work is never in doubt: find the
document, believe it, encode it.

The cassette interface is the week where that comfort runs out. There is
no cassette chip on a CoCo, no data sheet describing the tape format, and
— for the specific stretch of ROM that does the work — no disassembly
with commentary either. What exists is a jack, a comparator, one bit of
one PIA, and a few hundred bytes of hand-tuned 6809 machine code that
nobody has annotated. The behavior the emulator has to reproduce is not
documented anywhere; it is *latent in the ROM*, and the only way to find
it is to run the ROM and watch.

That makes this chapter unusual in two ways worth flagging before you
start. First, it is a complete signal-processing story — modulation,
demodulation, thresholds, clock recovery, byte framing — told end to end
in about 450 lines of Rust, which is small enough to read in one sitting
and general enough that the vocabulary transfers to any serial protocol
you meet later. Second, it is the chapter where the *method* matters more
than the result. The constants at the top of `cassette.rs` are worth
maybe ten lines of explanation. How anyone came to know them is worth the
whole of §12.8, and it is the single most portable skill in this book.

---

## 12.1 The ritual, and the machine underneath

Before disk drives were common, loading a program on a CoCo followed a
ritual that owners learned in their hands before they could put it into
words. The cassette deck's volume sat somewhere in the middle of its
dial — too quiet and the load failed with a garbled program, too loud and
it failed just as surely, for a reason nobody could articulate at the
time and §12.2 will explain precisely. The tape counter's three-digit
number went into a notebook after every `CSAVE`, so that the next
session could wind straight to it instead of listening through ten
minutes of previous programs. Then came `CLOAD"NAME"`, `PLAY` on the
deck, and a wait: the motor engaging with an audible clunk, a faint
high-pitched warble coming out of the television — the machine routes the
tape signal through its own sound mux, which §12.5 traces — and a cursor
sitting there giving nothing to watch.

That last detail is the interesting one, because it looks like a
missing feature and is in fact a direct consequence of the file format.
If the tape sat at the wrong counter position, nothing visibly happened
at all. BASIC was not idle; it was reading blocks, checking the
eight-character name in each one against the name it had been asked for,
and — silently, patiently — skipping every block that didn't match while
it waited for the next. There is no progress indicator because there is
nothing meaningful to indicate. The loader genuinely does not know
whether the program it wants is ten seconds ahead on the tape or not on
this side at all.

The shape of the tape format falls straight out of that behavior. Every
program on tape begins with a small **namefile** block carrying its name,
followed by a run of **data** blocks, followed by an **end-of-file**
block. `CLOAD"NAME"` is nothing more elaborate than "read namefile blocks
until one of them matches, then read data blocks until EOF." That
three-part structure is not folklore; it is asserted against a real,
booted ROM in this codebase's own end-to-end test, which §12.10 walks in
full:

```rust
// crates/coco-core/tests/cassette.rs:341-349
let blocks = parse_blocks(&tape);
assert_eq!(blocks[0].0, BLOCK_NAMEFILE);
assert_eq!(&blocks[0].1[..8], b"X       ", "namefile name");
assert_eq!(blocks[0].1.len(), 15, "namefile payload is 15 bytes");
assert!(
    blocks[1..blocks.len() - 1].iter().all(|(t, _)| *t == BLOCK_DATA),
    "middle blocks are data blocks"
);
assert_eq!(blocks.last().unwrap().0, BLOCK_EOF);
```

Read that assertion block as a specification rather than as a test. The
first block is a namefile. Its payload is fifteen bytes, the first eight
of which are the program's name padded with spaces — which is why
`CSAVE"X"` produces `X` followed by seven blanks rather than a
single-character string. Every block between the first and the last is a
data block. The final block is the end-of-file marker. Five assertions,
and between them they pin down the entire logical structure of a CoCo
tape without a word of prose specification anywhere.

### The chip that isn't there

Now the reveal, and it is the reason this chapter exists at all: **the
CoCo has no tape controller chip.** There is no MC6850-style UART wired
to the cassette jack, no dedicated modem part doing the frequency-shift
keying on the machine's behalf, nothing analogous to the WD1773 that
Chapter 13 will meet on the disk side. Between the tape jack and one bit of
one PIA sits an inexpensive op-amp comparator — the part labeled SALT on
the CoCo 3's board, functionally a zero-crossing detector — and that is
the entire hardware contribution. Everything else is 6809 machine code in
ROM, bit-banging a single pin: timing the tones, generating them,
decoding them, hunting for byte alignment, checksumming.

`CSAVE` and `CLOAD` are software. The "modem" that sounds like a piece of
hardware is, on this machine, a delay loop.

That fact doubles the emulator's job, and it is worth stating precisely,
because it is the thesis of the whole chapter. Two things have to be
right, and they are right for completely different reasons.

1. **Model the tape.** A cassette deck is a piece of analog mechanism
   with a motor, a physical position along a length of tape, a recorded
   signal, and — importantly — startup latency before any of that is
   trustworthy.
2. **Satisfy the ROM's own software demodulator.** There is no hardware
   standard to target here, no published tolerance band, only whatever
   the specific delay-loop constants in Color BASIC happen to produce.
   Get the tone frequencies even slightly wrong and the result is not
   "close enough": the ROM's polling loop, which is counting real
   physical time, either never triggers or triggers on the wrong cycle.

Put those two obligations together and the chapter's real subject comes
into focus. The emulator contains a *second* software modem, of its own
design, pointed at the ROM's software modem. Two decoders, written some
thirty-odd years apart by people who never spoke, that have to agree byte
for byte on a made-up FSK dialect neither one invented and neither one
can renegotiate.

### The fidelity contract, stated up front

The whole thing lives in one file, and that file's header states its
fidelity contract in the first fourteen lines. Read it closely, because
every section from here on is an expansion of one sentence in it:

```rust
// crates/coco-core/src/cassette.rs:1-14
//! Cassette tape deck: PIA1 $FF20/$FF21 tape I/O (CSAVE/CLOAD).
//!
//! The tape is stored as the *decoded* byte stream — the .cas convention:
//! leaders, sync bytes and blocks as plain bytes, "what the BIOS reads and
//! writes", not audio samples. Recording captures the DAC output with
//! cycle-accurate timestamps and demodulates it to bytes; playback
//! synthesizes the squared FSK signal the SALT chip's zero-crossing detector
//! would produce and feeds it to PIA1 PA0 (`cassette-verified-facts`).
//!
//! FSK timing was measured empirically against the stock ROM's CSAVE — the
//! bit-bang code lives in the $A000–$BFFF Color BASIC region that SEB
//! Unravelled II does not cover, so it cannot be derived from the local docs.
//! Measured: one full sine cycle per bit, serialized LSB first; see
//! [`ZERO_BIT_PERIOD`]/[`ONE_BIT_PERIOD`].
```

Four claims, each of which gets a section. The storage format is decoded
bytes rather than audio, which §12.3 argues for. Recording captures the
DAC with cycle-accurate timestamps and demodulates, which is §12.6.
Playback synthesizes a *squared* signal — not a sine — and feeds it to
PIA1 PA0, which is §12.5. And the timing was measured rather than looked
up, because the relevant ROM code is undocumented, which is §12.8.

One piece of notation in that header needs explaining before it appears
again. The token `cassette-verified-facts` is an internal project note
rather than a file that can be opened from this repository; it carries
roughly the weight of "per an earlier investigation against the
hardware." Wherever it appears, treat it as a citation you cannot follow
here. Everything else in this chapter traces either to code you can read
directly or to the empirical measurement story in §12.8, and where a
claim rests on neither, the text says so.

---

## 12.2 FSK from zero

The tape is an audio medium. It stores sound, and the read head produces
a voltage that swings up and down. Given that, the obvious way to store a
bit would be a voltage level: high for 1, low for 0, hold it for a fixed
time, read it back. This does not work, and understanding exactly why it
doesn't is the fastest route into the scheme that does.

A cassette signal path is *AC-coupled*, meaning the constant component of
the signal — its long-run average — is filtered out and discarded on its
way through. That is not a defect; it is how magnetic tape and the
amplifiers around it work. The practical consequence is that a signal
held at a constant level does not stay there. It decays back toward the
midpoint, and it does so within a small fraction of a second. There is no
way to hold a DC level on tape long enough to represent a bit at any
useful data rate.

**Frequency-shift keying**, or FSK, is the oldest answer in the modem
book, and it sidesteps the problem entirely. Rather than sending a 0 or a
1 as a voltage level, you send it as one of two *tones*. A 0-bit is a
burst of one frequency; a 1-bit is a burst of another. Whatever plays the
tape back only ever has to answer one question, over and over: was that
tone the slow one or the fast one? A tone is a pattern of changes rather
than a level, so it survives AC coupling by construction, and it survives
tape hiss, wow, and flutter far better than an absolute voltage ever
could. Play the tape back a few percent fast and both tones shift by the
same few percent; the *ratio* between them, which is all the decision
depends on, does not move at all.

The CoCo's particular flavour of FSK, as measured against the real ROM
(the whole story of *how* is §12.8), rests on four facts:

- **0-bit** ≈ 1100 Hz.
- **1-bit** ≈ 2060 Hz.
- **One full sine cycle per bit** — not several cycles of a fixed carrier
  the way a telephone modem of the era would do it, but exactly one
  period of the appropriate tone, then straight on to the next bit's
  tone. A byte is eight back-to-back single-cycle tone bursts.
- **Bits are sent LSB first**, which is the order a 6809 naturally
  produces if you bit-bang a byte out of a register one carry flag at a
  time with a right shift, and that is precisely what the ROM is doing.

The third fact is the one that most distinguishes this scheme from the
textbook picture of a modem, and it is worth dwelling on. There is no
carrier here in the usual sense. The signal is not "a 1100 Hz tone
modulated by data"; it is a sequence of single cycles, each of which is
whichever length its bit calls for. The waveform's instantaneous period
*is* the data. That makes demodulation conceptually trivial — measure one
cycle, compare against a threshold, emit a bit — and it makes the data
rate variable, since a byte of 1-bits takes a little over half as long to
transmit as a byte of 0-bits.

Here are the constants exactly as measured, from the top of
`cassette.rs`. Note that they are stored as *half* periods, four numbers
rather than two, for a reason the doc comment states and §12.8 returns to:

```rust
// crates/coco-core/src/cassette.rs:18-31
/// Half-cycle durations, in CPU cycles, of the tape sine the stock ROM
/// writes — measured empirically against `roms/coco3.rom` with
/// `examples/cassette_calibrate.rs` (modal midpoint-crossing spacings). The
/// ROM's waveform is slightly asymmetric (the high half runs shorter than
/// the low half) and playback mirrors it exactly: the ROM's demodulator
/// classifies bits by polling-loop counts of these very widths, and a
/// symmetric wave puts the 0-bit halves on its decision boundary.
///
/// 0 bit: 396 + 418 = 814-cycle full period, ~1100 Hz (nominal "1200 Hz").
const ZERO_BIT_HIGH: u32 = 396;
const ZERO_BIT_LOW: u32 = 418;
/// 1 bit: 207 + 227 = 434-cycle full period, ~2060 Hz (nominal "2400 Hz").
const ONE_BIT_HIGH: u32 = 207;
const ONE_BIT_LOW: u32 = 227;
```

The doc comment's middle sentence is doing more work than it appears to.
It says the emulator does not merely reproduce the right *period* — it
reproduces the right *lopsidedness within* the period, and it gives the
reason: the ROM's demodulator does not measure whole cycles. It counts
trips around a polling loop while waiting for the input line to change
state, so what it actually measures is a *half* cycle at a time. Feed it
a symmetric wave whose full period is exactly right and its half-period
counts land in the wrong place. The comment is blunt about the
consequence: a symmetric wave "puts the 0-bit halves on its decision
boundary," which is another way of saying the loader becomes a coin flip.

### From cycles to Hertz, and back

Emulator code, as Chapter 1 established, thinks in CPU cycles rather than
seconds, and this chapter is the strongest illustration of why that habit
pays. Nothing in `cassette.rs` ever mentions Hertz. Every timing quantity
in the file is an integer count of CPU cycles, compared against other
integer counts of CPU cycles, with no floating point and no unit
conversion anywhere on the hot path.

The conversion still matters for talking about the signal, and it is one
division. The CoCo's clock is `CPU_HZ = 894_886.0`
([`crates/coco-core/src/machine.rs:26`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine.rs#L26)),
which is the NTSC color subcarrier of 3.579545 MHz divided by 4 — the
same constant Chapter 1 introduced by a different route, as the 28.636363
MHz crystal divided by 32. A period expressed in cycles becomes a
frequency by dividing it into the clock:

| Bit | Period (cycles) | Frequency | Nominal FSK name |
|-----|-----------------:|----------:|-------------------|
| 0   | 396 + 418 = 814   | 894886/814 ≈ **1099.4 Hz** | "1200 Hz" |
| 1   | 207 + 227 = 434   | 894886/434 ≈ **2061.9 Hz** | "2400 Hz" |

Those "nominal" names in the right-hand column are doing real work rather
than decorating the table. 1200 Hz and 2400 Hz are the classic **Kansas
City standard** tone pair that a great many 1970s and 1980s home
computers used for cassette I/O. The CoCo's ROM tones are *close* to that
convention — close enough that "1200/2400" is how CoCo cassette I/O gets
described casually, and close enough that the two schemes are describing
the same idea — but they are not exact, and the reason they are not is
instructive.

Nothing in the CoCo derives these tones from a crystal. They are the
byproduct of a hand-tuned 6809 delay loop counting cycles, and a
hand-tuned delay loop lands wherever the programmer's arithmetic put it,
not on a round number. Somebody wrote a loop, counted the cycles in it,
decided the result was near enough to the convention that real tapes
would interchange, and shipped. The gap between "the number everyone
calls it" and "the number the software actually produces" is roughly 8%
on the 0-bit and 14% on the 1-bit, which is enormous by the standards of
a crystal-derived clock and completely unremarkable by the standards of a
delay loop.

This is a pattern you will meet constantly writing emulators, and it is
worth adopting as a reflex: **never trust the folklore figure when you
can measure the real one.** Folklore figures are the numbers that
propagate through magazine articles and forum posts because they are
memorable. They are usually the *design intent*, and the design intent
and the shipped silicon — or, here, the shipped software — agree only as
often as anyone bothered to check.

### The asymmetry is a measurement, not noise

Look once more at the four constants and notice that both tones lean the
same way. The 0-bit's high half is 396 cycles against a low half of 418,
a ratio of 94.7%. The 1-bit's is 207 against 227, a ratio of 91.2%. In
both cases the high half runs shorter.

That consistency is the tell. If these numbers were rounding artifacts or
measurement noise, the two tones would not lean in the same direction by
similar proportions; noise does not conspire. What the numbers record is
a single physical lopsidedness in the ROM's output waveform, sampled
twice at two different frequencies, and it is exactly the kind of detail
that only survives if somebody measured the real waveform instead of
assuming a textbook symmetric square wave. A model built from the
assumption would have stored two numbers, 814 and 434, and would have
been wrong in a way that no amount of staring at the code would reveal.
Section 12.8 explains why matching the asymmetry, and not merely the
gross period, is what makes the difference between a `CLOAD` that works
and one that hangs.

### Why square waves are enough

There is a sine wave in this system, and knowing exactly where it does
and does not exist saves a great deal of confusion later.

A real cassette tape stores an analog signal, and what the CoCo's 6-bit
DAC synthesizes on the way *out* genuinely is a stepped sine: the ROM
walks a sine table, writing successive amplitude values to the DAC, and
the analog output traces a recognizable sine cycle per bit. (The
`cassette_wav` module reproduces that sine exactly for WAV export, which
is §12.9.) So on the record side, a sine is really there.

On the playback side it is destroyed before the CPU can see it. Between
the tape head and the PIA sits the SALT comparator, a **zero-crossing
detector**: a part whose entire job is to output a clean digital high or
low depending on which side of the signal's midpoint the waveform
currently sits. Because the path is AC-coupled, that midpoint is
effectively zero volts, which is where the name comes from. The
comparator throws away *everything* about the signal except "is it above
or below the middle right now." Amplitude, harmonic content, the exact
curvature of the sine, tape hiss riding on top of it: all discarded. What
comes out is a square wave with one edge per zero crossing.

This is also, incidentally, the answer to the volume-knob mystery from
§12.1. A comparator needs the signal to actually cross its threshold with
authority. Too quiet and the swings never clear the noise floor cleanly,
so crossings get missed or doubled. Too loud and the amplifier stage
ahead of it clips and distorts, smearing where the crossings fall in
time. Both failures corrupt the *timing* of the edges, which is the only
thing the ROM measures — and that is why both ends of the dial fail
while the middle works.

For the emulator, the consequence is a substantial simplification. The
ROM never sees a sine wave, so to be correct on the CPU-facing side of
this interface the emulator does not need to synthesize sine samples at
all. It only needs to flip a single bit — PA0 — at the right cycle
counts, which is exactly what `Cassette::input_bit()` does in §12.5. The
sine only has to exist where something *outside* the emulated CPU could
plausibly care about it: real audio export to a WAV file that might feed
a physical tape deck's automatic gain control and line input, which is
§12.9.

Two representations of the same signal, for two different consumers. The
codebase keeps them in two different functions rather than pretending one
waveform can serve both jobs, and that separation is why neither function
has to carry an "is this for the CPU or for a file?" flag.

---

## 12.3 The `.cas` decision: bytes, not audio

Before any code, one architectural choice governs everything else in this
chapter, and it is a textbook example of the fidelity-budget thinking
Chapter 1 asked you to practise on every subsystem. The question is simple
to state: **what does the tape image on disk actually store?**

Two answers exist, and both are real formats that real emulators use.

The first is **`.wav`**: literal audio samples, some tens of thousands of
amplitude values per second, exactly what a sound card would play into a
real deck's line input. This is the maximally literal choice. A `.wav`
tape image is, in a defensible sense, *the tape* — the same physical
quantity, sampled.

The second is **`.cas`**: the *decoded byte stream*. Leader bytes, the
sync byte, and each block's type, length, payload and checksum, stored as
plain `u8` values. Not audio at all. "What the BIOS reads and writes," as
the module header puts it.

This codebase chose `.cas`, and the reasoning is worth spelling out,
because the consequences ripple through every section that follows.

**Storage cost.** A `.wav` recording of a `CSAVE`d one-line BASIC program
runs to hundreds of kilobytes of 8-bit PCM at any sensible sample rate.
The same program as decoded bytes runs to perhaps sixty of them. That is
roughly four orders of magnitude, spent on information that is one
hundred percent redundant: the audio *is* the bytes, re-expanded into a
representation that carries no additional meaning.

**Work at load time.** If tapes were stored as audio, then even a
same-emulator round trip — `CSAVE` now, `CLOAD` an hour later — would
force every single `CLOAD` through the full crossing-detection and
demodulation pipeline, purely to recover data that the emulator itself
wrote moments earlier and already knew perfectly. Storing decoded bytes
means playback is "walk this byte array," with no signal processing
involved unless a format boundary is being crossed deliberately (§12.9).

**Fidelity, honestly accounted.** The tempting objection is that `.wav`
must be *more accurate* because it is closer to the physical artifact.
For this system, that intuition is backwards. The only consumer of a
tape image inside the emulator is a demodulator that recovers the exact
same byte stream either way; the audio carries no information the byte
stream lacks, and the decode is lossless in both directions. Storing the
audio would preserve extra *detail* while adding no extra *fidelity*,
which is the definition of spending fidelity budget in the wrong place.

There is a price for this choice, and it is the one that shapes the rest
of the chapter. Because the file format is bytes rather than waveform,
the emulator has to be able to travel in **both** directions between
bytes and edges, honestly, every time:

- **Playback is modulation.** Bytes become FSK edges on PA0, in real
  time, cycle by cycle, exactly as a real tape deck's read head would
  present them to the SALT comparator.
- **Recording is demodulation.** DAC writes become edges, edges become
  measured tone periods, periods become bits, and bits become
  byte-aligned blocks — exactly as the SALT comparator and the ROM's
  software would have to do it from a real tape.

That is the "software modem pointed at a software modem" framing from
§12.1, now made concrete: this file *is* a modem, in both directions,
because the storage format demands that it be one. A `.wav`-backed design
could have gotten away with only ever demodulating, decoding once at load
time if it bothered to store anything but raw samples. The `.cas`
convention cannot cut that corner, and the rest of this chapter is what
paying that bill looks like.

---

## 12.4 The deck's state

Chapter 1 proposed three questions to ask of every new device before writing
a line of code: what is the state, what is the loop, where is the seam?
The cassette is small enough to answer all three at once, so start with
the state. The `Cassette` struct is compact, and every field in it earns
its place:

```rust
// crates/coco-core/src/cassette.rs:80-116
#[derive(Default, Serialize, Deserialize)]
pub struct Cassette {
    clock: u64,
    last_level: Option<u8>,
    capture: Vec<Transition>,
    mounted: bool,
    #[serde(skip)]
    tape: Vec<u8>,
    pos: usize,
    bit: u8,
    bit_elapsed: u32,
    spinup_left: u32,
    motor_was_on: bool,
    dirty: bool,
}
```

(The doc comments are trimmed here to make the shape visible; the
annotated original sits above the fold in
[`crates/coco-core/src/cassette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cassette.rs#L80-L116),
and it repays a second pass once this chapter is finished, because
several of those comments presuppose facts §12.6 and §12.10 have not
covered yet.)

The eleven fields sort into four groups. `clock`, `pos`, `bit`,
`bit_elapsed`, `spinup_left` and `motor_was_on` are **playback** state:
where the deck's read head currently is, expressed both as a
tape-position and as a sub-bit timing offset. `last_level` and `capture`
are **recording** state: what the deck is hearing from the DAC right now,
and everything it has heard so far this session. `tape` is the mounted
medium itself. `mounted` and `dirty` are bookkeeping.

One struct, both directions, because it is one physical deck. At any
given moment it is either playing or being recorded onto, never both —
but it does not know in advance which the machine will ask it to do next,
so it carries the state for both and lets the motor line and the ROM
decide.

### The clock that stops

Of those fields, `clock` is the one whose doc comment contains a fact you
would not guess and would miss if you skimmed:

```rust
// crates/coco-core/src/cassette.rs:82-86
    /// Motor-on cycle clock: advanced by [`Cassette::tick`] only while the
    /// motor relay (PIA1 CA2) is energized — tape position doesn't move
    /// otherwise. Freezing it across motor-off gaps also collapses the
    /// inter-block silences out of the capture.
    clock: u64,
```

The first half is unsurprising: tape does not move when the motor is off,
so a clock measuring tape motion should not advance either. The second
half is the consequential part. Because the recorder timestamps every DAC
transition against *this* clock rather than against the machine's
wall-clock cycle counter, and because this clock freezes whenever the
relay opens, the silent gaps between blocks — during which the ROM has
switched the motor off to do other work — vanish from the recorded
timeline entirely. Two transitions separated by half a second of real
time, with the motor off in between, come back from the capture looking
adjacent.

That is a deliberate simplification with a visible payoff in §12.6. The
demodulator has a notion of "a gap too long to be a tone," and if
motor-off silences survived into the capture, every inter-block pause
would trip it. Freezing the clock means the demodulator sees a continuous
signal punctuated only by genuine glitches, which is a much easier thing
to reason about — and, since the tape itself does not record silence
either, it is arguably the more faithful model as well.

> **Rust corner: resetting a struct by rebuilding it.** Mounting and
> ejecting a tape both have to put the deck into a known state, and both
> do it in one line:
>
> ```rust
> // crates/coco-core/src/cassette.rs:125-134
> pub fn insert_tape(&mut self, bytes: Vec<u8>) {
>     *self = Self { mounted: true, tape: bytes, ..Self::default() };
> }
>
> pub fn eject_tape(&mut self) {
>     *self = Self::default();
> }
> ```
>
> Neither function hand-resets eleven fields one at a time. The expression
> `Self { mounted: true, tape: bytes, ..Self::default() }` is Rust's
> *struct update syntax*: build a fresh value from `Default`, override
> the fields you name, and take the rest from the default. Assigning that
> through `*self` overwrites the whole struct behind the `&mut self`
> reference in one go.
>
> This is not merely shorter than the alternative — it is *safer* against
> exactly the bug class this file's own doc comments worry about
> elsewhere, which is a stale `pos`, `bit` or `spinup_left` surviving a
> tape swap and corrupting the next load. There is no way to add a
> twelfth field to `Cassette` later and forget to reset it here, because
> nothing here names fields to reset. `Default` does that job once, in
> one place, and every reset site inherits the fix for free.
>
> The habit generalizes beyond this file. When you see
> `*self = Self { field: value, ..Self::default() }` anywhere in this
> codebase, read it as "everything not named here goes back to
> power-on," which is a stronger and more maintainable guarantee than a
> hand-written list of assignments can offer, and one that survives
> refactoring by people who have never read this chapter.

### The one place the state is not trusted

There is a third entry point into this reset machinery, and it is the
interesting one, because it deliberately does *not* reset anything. When
Chapter 16's save-state machinery restores a snapshot, the tape's bytes are
not in it — a mounted tape is media, and commercial tapes are
copyrighted, so the `tape` field is `#[serde(skip)]` and the file is
referenced by path and hash instead. Everything *else* about the deck,
including `pos` and `bit`, does come back from the snapshot, and the
restore path has to reattach the bytes without disturbing it:

```rust
// crates/coco-core/src/cassette.rs:156-162
    pub fn reattach_tape(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        if self.bit >= 8 {
            return Err(format!(
                "cassette reattach: restored bit index {} is out of range (must be < 8)",
                self.bit
            ));
        }
```

That guard exists because a snapshot is untrusted input. `bit` is an
ordinary deserialized field, and a hand-crafted or corrupted payload can
set it to anything a `u8` can hold. Look at what happens downstream if it
does: `current_bit_is_one` shifts a byte right by `bit` with no bounds
check of its own, and a shift wider than the type panics in debug builds
and becomes a masked shift — the shift amount wrapped to the type's
width — in release ones. The function's own doc comment
([`cassette.rs:140-155`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cassette.rs#L140-L155))
walks through the other two failure modes it screens for: a restored
`pos` past the end of a tape file that has changed shape since the
snapshot was taken, and the subtler case of `pos == tape.len()` with a
nonzero `bit`, which `tick` can never produce and which therefore
indicates a corrupted payload rather than a legitimate end-of-tape.

The general lesson is worth carrying into Chapter 16 and into any project
with a load path. Deserialization is a trust boundary. Fields that were
invariants while the program was running become *assertions to check* the
moment they arrive from a file, and the cheapest place to check them is
the function that puts the pieces back together.

---

## 12.5 Playback: bytes become edges

With the state understood, the loop is next. The cassette's loop runs
from the same place as every other per-instruction device in this
machine, and its cadence was chosen for a reason the doc comment states
plainly:

```rust
// crates/coco-core/src/cassette.rs:222-226
    /// Advance the motor-on cycle clock and the playback position. Called
    /// once per instruction from `Machine::run_cycles`, alongside
    /// `bus.cart.tick` (same per-instruction cadence as the FD-502
    /// precedent) — per-scanline would be far too coarse against the
    /// ~217-cycle half-periods of the 1-bit tone.
```

That last clause is the fidelity budget from Chapter 1, spent explicitly. A
scanline on this machine is on the order of fifty-seven CPU cycles at
normal speed, and a video field is 262 of them; most devices in this
emulator are perfectly happy being ticked once per scanline or once per
field. The cassette cannot be, because the *shortest thing it has to
represent* is a 207-cycle half-period, and a device that only gets to
change its output at scanline boundaries cannot place an edge at cycle
207 of a bit cell. Per-instruction ticking is the coarsest cadence that
still lets edges land where the ROM expects them.

Here is the call site, sitting between the cartridge and the bit-banger
in `step_cpu_unit`:

```rust
// crates/coco-core/src/machine/run.rs:118-119
self.bus.cart.tick(cycles);
self.bus.cassette.tick(cycles, self.bus.pia1.a.c2_output());
```

The second argument is the motor relay line — PIA1's CA2 output, which
Chapter 10 introduced as one of the two handshake lines every PIA port
carries and which Tandy wired to the deck's remote-control jack. When
BASIC energizes that relay, a real deck's motor starts turning. More on
what that costs in §12.7.

### The tick loop

`tick` is the whole playback engine, and at thirty-three lines it is
short enough to read in one pass before analyzing it:

```rust
// crates/coco-core/src/cassette.rs:227-259
pub fn tick(&mut self, cycles: u32, motor_on: bool) {
    if motor_on && !self.motor_was_on {
        self.spinup_left = MOTOR_SPINUP_CYCLES;
    }
    self.motor_was_on = motor_on;
    if !motor_on {
        return;
    }
    if self.spinup_left > 0 {
        self.spinup_left = self.spinup_left.saturating_sub(cycles);
        return;
    }
    self.clock += u64::from(cycles);
    if !self.playing() {
        return;
    }
    self.bit_elapsed += cycles;
    while self.playing() {
        let period = self.current_bit_period();
        if self.bit_elapsed < period {
            break;
        }
        self.bit_elapsed -= period;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.pos += 1;
        }
    }
    if !self.playing() {
        self.bit_elapsed = 0;
    }
}
```

Walk it in order, because the order is the argument.

**Motor edge detection comes first.** The test `motor_on &&
!self.motor_was_on` catches the off-to-on transition specifically, not
the steady on state, and arms the spin-up countdown. The tape does not
move an inch until that countdown drains, for reasons §12.7 devotes
itself to. Note that this is an *edge* detector built from one bool of
history, which is the same shape you saw in the PIA's own Cx1 handling in
Chapter 10 and will see again in the disk controller's HALT logic in week
13.

**Motor off means nothing moves, full stop.** No clock advance, no bit
progress, no partial credit. This early return is also why the recorder's
`record_dac` resets `last_level` to `None` on motor-off: the relay
opening is a hard boundary between sessions on both sides of the deck,
and both sides honor it identically.

**The cycle budget accumulates into `bit_elapsed`,** and it is drained by
a `while` loop rather than an `if`. That distinction is load-bearing and
easy to get wrong. `tick` is called once per *instruction*, and a 6809
instruction can cost quite a few cycles — on a slow addressing mode,
more than a substantial fraction of a bit period. Push the machine into
the double-speed mode Chapter 6 introduced and each emulated instruction
covers twice as much tape. Under enough cycle pressure, more than one bit
can legitimately complete inside a single `tick` call, and an `if` here
would silently discard the surplus, dropping bits under exactly the
conditions that make them hardest to notice. The `while` loop drains the
budget until less than one bit's worth remains.

**Bit and byte advance together.** `bit` counts from 0 to 7 and wraps,
incrementing `pos` as it does — LSB first, matching how
`current_bit_is_one` indexes the byte. That single-site coupling of the
two counters is also what makes the save-state invariant in §12.4 true:
`pos` only ever advances in the same step that resets `bit` to zero, so
`pos == tape.len()` with a nonzero `bit` cannot arise from normal
execution.

The two helpers that `tick` leans on are three lines each, and between
them they answer what the tape's next bit wants and how long its tone
burst lasts:

```rust
// crates/coco-core/src/cassette.rs:277-283
fn current_bit_is_one(&self) -> bool {
    self.tape[self.pos] >> self.bit & 1 == 1
}

fn current_bit_period(&self) -> u32 {
    if self.current_bit_is_one() { ONE_BIT_PERIOD } else { ZERO_BIT_PERIOD }
}
```

`self.tape[self.pos] >> self.bit & 1` is the LSB-first serialization in
one expression: shift the byte right by the bit index and mask the bottom
bit, so `bit == 0` yields the least significant bit. If the ROM had
serialized MSB first, this line would read `>> (7 - self.bit)`, and
exercise 12.3 explores what happens when the convention is broken on the
receiving side.

### The line PA0 actually sees

Everything so far tracks *where* on the tape the deck is. The output — the
single bit that eventually reaches the CPU — is computed on demand, from
that position, by a function with no side effects at all:

```rust
// crates/coco-core/src/cassette.rs:261-275
pub fn input_bit(&self) -> bool {
    if !self.motor_was_on || self.spinup_left > 0 || !self.playing() {
        return true;
    }
    let high = if self.current_bit_is_one() { ONE_BIT_HIGH } else { ZERO_BIT_HIGH };
    self.bit_elapsed >= high
}
```

Two behaviors, in five lines. The guard clause establishes that the line
**idles high** whenever nothing is playing: motor off, still spinning up,
or past the end of the tape. That default matches the CoCo's general rule
that unused and idle input pins float high, which is exactly what
`PiaPort::default()` encodes
([`crates/coco-core/src/pia.rs:52-56`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/pia.rs#L52-L56),
Chapter 10) by initializing every port's `input` to `0xFF`.

Otherwise the function renders a **square wave with one flip per half
period**. For the first `high` cycles of the current bit's tone —
`ZERO_BIT_HIGH` at 396 or `ONE_BIT_HIGH` at 207 — the line reads low; for
the remaining cycles of the period, it reads high. Notice that
`bit_elapsed` is doing double duty: `tick` uses it to decide when the
current bit ends, and `input_bit` uses the same value to decide which
half of that bit is currently on the wire. One counter, two consumers, no
possibility of them disagreeing.

The doc comment above that function is where this chapter's methodology
first shows through, and it names a genuine piece of reverse engineering
rather than a design choice:

```rust
// crates/coco-core/src/cassette.rs:261-268
/// The squared tape signal as PA0 sees it: idle high with no tape moving
/// (or still spinning up), otherwise a square wave — the SALT
/// zero-crossing detector's rendering of the tape sine — with each bit
/// cell opening on its LOW half. The ROM's DAC sine table starts rising,
/// but the line reaching PA0 is inverted somewhere in the record→play
/// analog path (AC coupling/comparator polarity in the SALT): verified
/// empirically — the ROM's `CASON` ($A77C) lock never succeeds with
/// high-first cells and locks reliably with low-first.
```

That is a fact nobody could derive from a data sheet, and it is worth
following the reasoning. The ROM's own sine table starts *rising* from
the midpoint, so the natural expectation is that each bit cell opens on
its high half. It doesn't. Somewhere in the real analog chain between the
DAC writing a rising sine and the comparator's output arriving at PA0,
the polarity flips — plausibly in the AC coupling, plausibly in the
comparator's own wiring, and the comment does not pretend to know which.

What it does know is which choice makes the ROM work, and how that was
established: try both, and watch which one lets the ROM's lock-on routine
at `$A77C` actually succeed. One choice never locks. The other locks
reliably. The comment does not hedge about it either — "verified
empirically" is a specific claim about having watched real ROM code
behave differently under two alternatives, which is a much stronger
statement than a guess dressed up as a fact. When you write comments like
this in your own emulator, the distinction is worth preserving: say
whether you *know* or whether you *inferred*, because the person reading
it in two years will need to know which claims are safe to build on.

### Where the bit crosses the seam

`input_bit()` returns a `bool`, and something has to turn that into a bit
in a register the CPU can read. That happens at the bus level, in the
function that samples PIA1's port-A input pins:

```rust
// crates/coco-core/src/bus/pins.rs:29-40
    /// PIA1 port-A input pins: only bit 0 (cassette data in, `$FF20` —
    /// Service Manual / `cassette-verified-facts`) is driven by anything
    /// emulated; the rest float high like every other unused CoCo input pin
    /// ([`crate::pia::PiaPort`]'s default).
    pub(super) fn pia1_pa_pins(&self) -> u8 {
        const CASSETTE_IN: u8 = 0x01;
        if self.cassette.input_bit() {
            0xFF
        } else {
            !CASSETTE_IN
        }
    }
```

Eight lines, and they are the entire electrical interface between a tape
deck and a CPU on this machine. Bit 0 carries tape data; the other seven
bits float high, as unconnected CoCo input pins do. The whole cassette
subsystem — modulation, timing, motor physics, the lot — is visible to
the 6809 as one bit of one byte at one address.

That byte is refreshed on every read of the PIA1 register range, which is
the other half of the seam:

```rust
// crates/coco-core/src/bus/io.rs:67-71
            PIA1_BASE..=PIA1_LAST => {
                self.pia1.a.input = self.pia1_pa_pins();
                self.pia1.b.input = self.pia1_pb_pins();
                self.pia1.read((addr & 0x03) as u8)
            }
```

Put that in the context of what the ROM is doing on the other side.
Color BASIC's `BITIN` routine sits in a tight loop reading `$FF20` and
testing bit 0, counting iterations until the bit changes state. Every one
of those reads lands here, re-samples `input_bit()` against the deck's
current position, and hands back a freshly computed pin state. The
emulator is not pushing edges at the CPU; the CPU is pulling the line's
current value, hundreds of times per byte, and the deck is answering
from a position that `tick` has been advancing between each of those
reads. That pull-based arrangement is why `input_bit` is a pure function
of state rather than something that has to be scheduled: it can be asked
at any cycle and give the right answer for that cycle.

### The warble in the speaker

One last connection, because it ties this chapter back to Chapter 11 and
explains a detail from §12.1's opening ritual. The tape signal does not
only reach the CPU. It also reaches the *speaker*, through the same
four-way analog mux Chapter 11 dissected:

```rust
// crates/coco-core/src/audio.rs:92-97
            SEL_CASSETTE => {
                if inputs.cassette_relay && cassette_bit {
                    l += CASSETTE_GAIN;
                    r += CASSETTE_GAIN;
                }
            }
```

With the mux select set to `01` and the motor relay energized, the
cassette input is what the speaker is amplifying, and `cassette_bit` is
that same `input_bit()` value sampled once per scanline
([`crates/coco-core/src/machine/audio.rs:49`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/audio.rs#L49)).
The tape's FSK tones come out of the television, which is the audible
warble a `CLOAD` produces — and, incidentally, the reason a listener
could tell a leader run from data by ear, since the leader's strict
alternation of two frequencies sounds quite different from the
irregular chatter of real program bytes.

It is a small thing, but it is a good example of a subsystem's output
being consumed by two entirely unrelated parts of the machine, and of why
`input_bit` was worth writing as a cheap, side-effect-free query rather
than as something that mutates state on each call.

---

## 12.6 Recording: edges become bytes

Recording is playback's mirror image, and it is the harder half by a
comfortable margin. Playback *asserts* structure: the emulator knows
exactly which byte, which bit, and which half-cycle it is producing,
because it has the tape in hand. Recording has to *discover* structure in
a signal — where the cycles are, where the bits are, and, hardest of all,
where the bytes are — with nothing to go on but a list of timestamped
level changes.

That progression is worth holding in mind as the section proceeds,
because the code follows it exactly: transitions, then bits, then bytes.
Each stage knows strictly less than the one that produced it and has to
reconstruct what was lost.

### Capturing the DAC

The raw material is DAC writes. Every write anywhere in PIA1's address
range is checked for a change to the cassette-out level:

```rust
// crates/coco-core/src/bus/io.rs:104-113
PIA1_BASE..=PIA1_LAST => {
    self.pia1.write((addr & 0x03) as u8, val);
    // Cassette record-out is a direct, unconditional tap of the DAC
    // (not gated by SNDEN/the mux — `cassette-verified-facts`), fed
    // on every PIA1 write since any of them (port A output/DDR or
    // CRA, which carries the motor relay) can change it.
    let dac = (self.pia1.a.output & self.pia1.a.ddr & 0xFC) >> 2;
    self.cassette.record_dac(dac, self.pia1.a.c2_output());
    self.note_audio_write(); // DAC / PB1 / SNDEN / relay
}
```

The expression `(output & ddr & 0xFC) >> 2` is the same 6-bit DAC
extraction Chapter 11's mixer uses: PIA1 port A bits 2 through 7, masked
against the data-direction register so that only pins actually configured
as outputs contribute, then shifted down to a plain `0..=63` value. The
`& ddr` term is easy to skim past — a pin configured as an input
contributes nothing to the analog output no matter what the output
register holds, and modeling that keeps the emulator honest during the
brief windows when the ROM is reconfiguring the port.

The important word in that comment, though, is **unconditional**. Unlike
the speaker path, the cassette tap does not consult the sound-enable bit
or the mux selection. The DAC always feeds the tape record line, whether
or not anything would be audible, which matches the hardware: the DAC's
analog output is wired straight to the record circuit, and the
sound-enable and mux logic only ever affected what reached the speaker.
The comment also explains why the tap is fed on *every* PIA1 write rather
than only on writes to the data register — a write to the direction
register changes which pins drive the DAC, and a write to the control
register can change the motor relay, so any of the four addresses can
change what the tape is receiving.

`record_dac` itself is deliberately unclever:

```rust
// crates/coco-core/src/cassette.rs:285-298
pub fn record_dac(&mut self, level: u8, motor_on: bool) {
    if !motor_on {
        self.last_level = None; // next motor-on write starts a fresh run
        return;
    }
    if self.last_level != Some(level) {
        self.capture.push(Transition { level, cycle: self.clock });
        self.last_level = Some(level);
    }
}
```

This is **sparse, event-timestamped storage**: a `Transition { level,
cycle }` is appended only on an actual change, never one entry per cycle
and never one entry per write. A program spending ten seconds in `CSAVE`
writes the DAC an enormous number of times, but if you keep only the
changes you retain exactly the information a demodulator needs — when the
signal moved, and to what — without ever materializing a dense sample
array. Chapter 11 built its entire audio-event grid on the same principle,
and recognizing it here as the same idea solving the same class of
problem is worth more than either instance in isolation.

Notice also what the motor-off branch does: it clears `last_level` rather
than merely returning. That matters because of the frozen clock from
§12.4. When the relay closes again, the first DAC write must be recorded
even if it happens to repeat the level that was in force when the relay
opened, because in the capture's timeline those two writes are adjacent
and a suppressed transition would silently merge two tone cycles into
one.

### From transitions to bit periods

`finalize_recording` hands the whole capture to `demodulate`, which is a
two-stage pipeline: transitions become bits in `capture_to_bits`, and
bits become bytes in `bits_to_bytes`. The first stage is where the signal
processing happens:

```rust
// crates/coco-core/src/cassette.rs:333-377
fn capture_to_bits(capture: &[Transition]) -> Vec<Option<bool>> {
    let Some(max) = capture.iter().map(|t| t.level).max() else {
        return Vec::new();
    };
    if max == 0 {
        return Vec::new();
    }
    let mid = max / 2;

    let mut bits: Vec<Option<bool>> = Vec::new();
    let mut side = capture[0].level > mid;
    let mut last_rise: Option<u64> = side.then_some(capture[0].cycle);
    let mut last_fall: Option<u64> = None;
    for t in &capture[1..] {
        let new_side = t.level > mid;
        if new_side && !side {
            if let Some(prev) = last_rise {
                let period = t.cycle - prev;
                bits.push(if period > PERIOD_BREAK {
                    None
                } else {
                    Some(period <= BIT_PERIOD_THRESHOLD)
                });
            }
            last_rise = Some(t.cycle);
        } else if !new_side && side {
            last_fall = Some(t.cycle);
        }
        side = new_side;
    }
    if let (Some(rise), Some(fall)) = (last_rise, last_fall)
        && fall > rise
        && fall - rise <= PERIOD_BREAK / 2
    {
        bits.push(Some(2 * (fall - rise) <= BIT_PERIOD_THRESHOLD));
    }
    bits
}
```

The core idea is in one line. `mid = max / 2` computes a zero-crossing
threshold from the capture's *own observed levels*, not from a hardcoded
constant, which is exactly what a real comparator does with an
AC-coupled signal: it has no notion of what "high" means in absolute
terms, only of where the signal's own midpoint sits. The two
early returns handle the degenerate cases honestly — an empty capture and
a capture that never left zero both yield no bits rather than a stream of
garbage. Note what the second one actually avoids: not a division by zero
(`max / 2` is a perfectly well-defined 0) but a *degenerate threshold*, a
midpoint of zero derived from a signal that never moved, against which
"above the middle" would have stopped meaning anything at all.

From there the loop is a state machine over one boolean, `side`. Every
time the recorded level crosses from below `mid` to above it — a **rising
crossing** — one full tone cycle has completed since the *previous*
rising crossing. That is one bit's worth of time, measured the same way
§12.2 said a CoCo tape bit is defined: one full sine period, crossing to
matching crossing. Falling crossings are tracked separately in
`last_fall` but do not produce bits; they exist only for the salvage case
at the end.

Two lines in the middle deserve their own comment, and they have one in
the source:

```rust
// crates/coco-core/src/cassette.rs:344-347
    // A capture that starts on the high side starts mid-cycle: count the
    // first period from its first sample, or the opening bit is lost.
    let mut last_rise: Option<u64> = side.then_some(capture[0].cycle);
    let mut last_fall: Option<u64> = None;
```

`side.then_some(x)` yields `Some(x)` when `side` is true and `None`
otherwise, which reads as: if the capture opens above the midpoint, treat
its very first sample as though it were a rising crossing. The reasoning
is that a capture beginning on the high side has necessarily started
partway through a cycle whose true rising edge happened before recording
began. Without this seed, the first genuine rising crossing would have no
predecessor to measure against, and the opening bit — which, in a leader
run, is one of the bits the receiver needs most — would vanish.

### The 624-cycle threshold

A measured period is only half the job. The other half is the
comparison that turns a duration into a value, and the constants that
define it:

```rust
// crates/coco-core/src/cassette.rs:48-58
pub(crate) const ZERO_BIT_PERIOD: u32 = ZERO_BIT_HIGH + ZERO_BIT_LOW;
pub(crate) const ONE_BIT_PERIOD: u32 = ONE_BIT_HIGH + ONE_BIT_LOW;

/// Demodulation decision boundary between the two measured periods
/// (midpoint of 455 and 793): a full period at or below this is a 1 bit.
const BIT_PERIOD_THRESHOLD: u64 = (ZERO_BIT_PERIOD as u64 + ONE_BIT_PERIOD as u64) / 2;

/// A period twice the 0-bit's is no tone at all: a discontinuity (motor
/// spin-up glitch, inter-block artifact). The demodulator drops sync and
/// re-hunts for a leader when it sees one.
const PERIOD_BREAK: u64 = 2 * ZERO_BIT_PERIOD as u64;
```

Do the arithmetic and `BIT_PERIOD_THRESHOLD` lands on exactly **624**:
`(814 + 434) / 2 = 624`. A measured period at or below 624 cycles is
"fast," hence a 1-bit; anything above is "slow," hence a 0-bit. The
threshold sits 190 cycles from either real period, which is a comfortable
margin — roughly 30% in both directions — and exercise 12.2 measures just
how comfortable by pushing it until something breaks.

`PERIOD_BREAK` is a second, much looser threshold at 1628 cycles, or
twice the 0-bit period. A gap that long is not a slow 0-bit; it is not a
tone at all. Something has gone wrong — a motor spin-up glitch, a
discontinuity between blocks, or a genuine dropout — and the right
response is not to guess a bit value but to admit ignorance and let the
caller re-synchronize. That is what the `None` in `Option<bool>` carries.

There is a discrepancy in that excerpt worth catching, because it is
exactly the sort of thing that only surfaces when you check the
arithmetic instead of trusting the prose. The doc comment above
`BIT_PERIOD_THRESHOLD` says "midpoint of 455 and 793" — not 434 and 814,
which are the values the constants two lines below it actually produce.
Both pairs average to the same 624, since 455 + 793 = 1248 and 434 + 814
= 1248, so both midpoints land in the same place; but they are visibly
not the same measurement.

The most likely explanation is that this is a fossil: an earlier, coarser
pass at calibration, superseded by a more careful one without every
comment being updated to match. Its numbers turn up again in §12.8, in
the calibration probe's own header, which strengthens the theory. And the
reason the fossil survived undetected is precisely the arithmetic
coincidence above — because the two pairs sum identically, refining the
constants left `BIT_PERIOD_THRESHOLD`'s value unchanged, so nothing broke
and no test went red.

The constant is correct today regardless, since it is computed from the
current constants rather than written as a literal. But the discrepancy
is real and checkable in the file as it stands, and the habit that caught
it is the point: **when a comment states a number, do the arithmetic
yourself rather than taking the prose on faith.** Exercise 12.2 asks you
to use that habit again.

> **Rust corner: `Option<bool>` as an ad hoc three-state value.** Notice
> that `capture_to_bits` returns `Vec<Option<bool>>` rather than
> `Vec<bool>`. A bit demodulated from a real capture is not always
> cleanly 0 or 1. Sometimes the gap between crossings is too long to be
> *any* tone, and that third outcome is one the caller genuinely needs to
> distinguish from a confident 0 or a confident 1, because it means
> something entirely different: not "this bit is unclear" but "there was
> no bit here at all."
>
> Rather than invent a three-variant enum for this one call site, the
> code reaches for `Option<bool>`. `None` piggybacks on a type every Rust
> programmer already has intuition for, and the eventual consumer,
> `bits_to_bytes`, handles it with the same `let Some(bit) = bit else
> { … }` pattern it would use for any other optional value.
>
> Compare that against `BlockState` a few dozen lines away, which *is* a
> purpose-built enum. The difference is instructive. `BlockState` has
> more than two outcomes and one of its variants carries data of its own
> (`Locked { seen, total }`), and neither of those fits inside `Option`
> without contortion. The rule of thumb this file demonstrates: reach for
> `Option` when "absent or invalid" is genuinely the third state of an
> otherwise binary question; reach for a dedicated `enum` once a variant
> needs to carry its own fields or once you have more than a couple of
> cases. Using `Option<bool>` here instead of an
> `enum Bit { Zero, One, Break }` is not laziness — a bespoke enum would
> need its own `match` arms everywhere that `Option`'s already-idiomatic
> combinators (`then_some`, `let`-`else`) apply for free.

> **Rust corner: let-chains for the dangling last bit.** The salvage code
> at the end of `capture_to_bits` uses a syntax that is worth naming,
> because it is recent enough that plenty of Rust code in the wild still
> works around its absence:
>
> ```rust
> if let (Some(rise), Some(fall)) = (last_rise, last_fall)
>     && fall > rise
>     && fall - rise <= PERIOD_BREAK / 2
> {
>     bits.push(Some(2 * (fall - rise) <= BIT_PERIOD_THRESHOLD));
> }
> ```
>
> This combines a *pattern match* — `if let (Some(rise), Some(fall)) =
> …` — with two ordinary boolean conditions, joined by `&&`, all inside
> one `if`. Older Rust required nesting: an outer `if let` whose body
> contained a second plain `if` for the extra conditions, adding a level
> of indentation for no semantic reason and pushing the actual work
> further from the condition that guards it.
>
> **Let-chains**, stabilized as part of the Rust 2024 edition this
> workspace targets (check `crates/coco-core/Cargo.toml`'s
> `edition = "2024"`), allow the whole condition — pattern matches and
> boolean tests together — to be written as one flat `&&` chain. Read
> `if let PAT = EXPR && cond1 && cond2 { … }` as "all of these must
> hold," exactly like a normal boolean chain, except that some of the
> terms happen to be pattern matches that also bind names the later terms
> and the body can use. Here `rise` and `fall` are bound by the first
> term and used by the two that follow it, which is the case that makes
> the feature worth having. When you meet this shape in 2024-edition
> code, you are looking at what would have been two or three nested
> `if`s a few years ago.

Why does a recording end with a bit that needs salvaging at all? Because
the ROM turns the motor off immediately after writing the final byte.
There is no trailing rising edge to close out the very last tone cycle,
since nothing gets written after it, and `capture_to_bits` only emits a
bit when it sees the crossing that *ends* a cycle. Without a fallback,
the tape's last bit — in practice, part of the EOF block's trailer —
would go missing, and playing that recording back would leave the
ROM's own `BITIN` routine at `$A755` polling forever for an edge that is
never coming.

The fix is a reasonable inference rather than a fabrication. If the
capture ends mid-cycle, with a rise followed by a fall and no closing
rise, the *half* period that did complete is known exactly. Doubling it
estimates the full period, and the guard `fall - rise <= PERIOD_BREAK / 2`
refuses to do so when even the half period is implausibly long. The
estimate can only be wrong if the ROM's waveform is far more asymmetric
than §12.2's measured 91–95% ratios, which it is not.

### Byte alignment: hunting the way the ROM hunts

A bit stream on its own has no byte boundaries. That is not a CoCo quirk;
it is a fact about serial data in general, and every serial protocol ever
designed has had to answer it somehow. Asynchronous UARTs answer it with
start bits. Ethernet answers it with a preamble. The CoCo's ROM answers
it by scanning the incoming bits for a known pattern before trusting any
of them, and `bits_to_bytes` re-implements that same strategy:

```rust
// crates/coco-core/src/cassette.rs:379-450
enum BlockState {
    Hunt,
    Locked { seen: usize, total: usize },
}

const BLOCK_OVERHEAD: usize = 4; // type + length + checksum + trailer $55

fn bits_to_bytes(bits: Vec<Option<bool>>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut state = BlockState::Hunt;
    let mut window: u8 = 0;
    let mut window_bits = 0u32;
    let mut leader_count = 0usize;
    for bit in bits {
        let Some(bit) = bit else {
            state = BlockState::Hunt;
            window = 0;
            window_bits = 0;
            leader_count = 0;
            continue;
        };
        window = window >> 1 | u8::from(bit) << 7; // LSB arrives first
        window_bits += 1;
        match state {
            BlockState::Hunt => {
                if window_bits < 8 {
                    continue;
                }
                if window == LEADER {
                    leader_count += 1;
                    window_bits = 0;
                } else if window == SYNC {
                    out.extend(std::iter::repeat_n(LEADER, leader_count));
                    out.push(SYNC);
                    leader_count = 0;
                    window_bits = 0;
                    state = BlockState::Locked { seen: 0, total: usize::MAX };
                }
            }
            BlockState::Locked { ref mut seen, ref mut total } => {
                if window_bits < 8 {
                    continue;
                }
                out.push(window);
                window_bits = 0;
                *seen += 1;
                if *seen == 2 {
                    *total = usize::from(window) + BLOCK_OVERHEAD;
                }
                if *seen >= *total {
                    state = BlockState::Hunt;
                }
            }
        }
    }
    out.extend(std::iter::repeat_n(LEADER, leader_count));
    out
}
```

Two states, and the trick that makes byte-alignment recovery possible at
all is *how each state consumes bits*.

**`Hunt` uses a sliding window.** This is easy to miss on a first read.
`window_bits` gates only the very first eight bits — the check `if
window_bits < 8 { continue; }` suppresses comparison until a full window
exists — but once it reaches 8, nothing in the `Hunt` arm ever resets it
except an actual `LEADER` or `SYNC` match. Every subsequent bit therefore
shifts into `window` and gets compared, one bit later, against the same
two targets. The window advances one bit at a time rather than jumping
eight bits at a stretch.

That is precisely what a receiver with no independent clock has to do. It
does not know where byte boundaries "should" fall, because there is no
external timing reference telling it. It cannot assume the first bit it
happened to hear was the first bit of a byte. So it checks *every*
possible alignment, one bit at a time, until one of them produces a value
distinctive enough to bet on.

**`LEADER` (`$55`, Service Manual §5.10) is that distinctive value** — or
rather, it is distinctive in an unusual way. As a bit pattern, `0x55` is
`01010101`, a strict alternation. A run of consecutive `$55` bytes on the
wire is therefore indistinguishable, bit for bit, from one continuous
alternating stream with no byte structure visible in it anywhere. It is
not a flaw in the choice; it is the entire point. A leader is not data;
it is *clock recovery*. Its job is to give the receiver dozens of bytes'
worth of opportunity to settle in — to establish that a signal is
present, that its two tone widths are what they should be, and that the
alternation is stable — before anything arrives whose value actually
matters. The receiver cannot lock byte alignment on the leader, and is
not meant to.

Alignment comes from the **`SYNC` byte, `$3C`**, which is declared
`pub(crate)` in this module because `cassette_wav` needs it too (§12.9).
When the sliding window matches `$3C`, the hunt has found an alignment
that produces a meaningful byte, and it commits: it flushes the leader
run it has been counting (as plain `$55` bytes, so the caller can see how
long the leader actually was), pushes the sync byte itself, and switches
to `Locked`.

**`Locked { seen, total }`** is a small state machine of its own,
carrying exactly the two numbers it needs. `seen` counts block bytes read
so far. `total` is how many the block will contain — which is not known
until the *second* byte arrives, since that byte is the payload length.
Hence the `usize::MAX` placeholder: an initial value that no real block
can reach, so the "have we finished?" test cannot fire prematurely.
`BLOCK_OVERHEAD = 4` accounts for the type, length, checksum and trailer
bytes surrounding the `length`-byte payload.

Read `bits_to_bytes` carefully enough and you can reconstruct the entire
block layout from it, without an external specification:

```
$55* $3C  type  len  payload[len]  checksum  $55
 |    |    |     |        |           |       |
leader sync |   length  data       sum(type,   trailer
 run       block-type  byte                    (looks just
                                    len, data)   like a leader
                                    & 0xFF        byte — and
                                                   is treated
                                                   as one)
```

The last column is the elegant part. The trailer byte is a plain `$55`,
structurally identical to a leader byte, and `bits_to_bytes` does not
special-case it at all. Once `Locked` reads it as byte number `total` and
returns to `Hunt`, that same `$55` — already consumed as block data,
already appended to `out` — looks to everything downstream like the first
byte of the next block's leader run. The format needs no explicit
end-of-block marker because the trailer *is* leader material by
construction, recycled. A format designer working under a byte budget
gets to feel quietly pleased about that one.

Finally, consider what a `None` bit does. It resets everything back to
`Hunt` from wherever the reader was, mid-leader or mid-block, discarding
the partial window and the leader count. That is a deliberate design
choice worth naming: **a glitch costs the current block, not the
recording.** Real tape behaves the same way. A dropout corrupts the block
it lands in and nothing after it, because the next block's own leader run
gives the reader a fresh chance to re-synchronize. An emulator that
treated a discontinuity as fatal would be modeling a stricter medium
than the one it is pretending to be.

### Deciding whether the recording counts

The last piece is the one that decides whether any of this work becomes
the mounted tape:

```rust
// crates/coco-core/src/cassette.rs:305-313
    pub fn finalize_recording(&mut self) {
        let decoded = demodulate(&self.capture);
        self.capture.clear();
        self.last_level = None;
        if self.mounted && decoded.contains(&SYNC) {
            self.tape = decoded;
            self.dirty = true;
        }
    }
```

The guard `decoded.contains(&SYNC)` is doing something genuinely
important. Remember from this section's first excerpt that the DAC tap
is unconditional: *every* DAC write reaches the recorder while the motor
relay is closed, including writes that were meant for the speaker. A
program that plays sound effects with the relay energized will leave
noise in the capture, and demodulating noise produces a byte stream with
no sync byte in it, because sync bytes only arise from real block
structure.

So a capture without a sync is discarded and the existing tape is kept.
The practical effect is the one that matters at the user interface: a
`CLOAD` followed by a rewind does not wipe the tape. The rewind path
calls `finalize_recording` first — that is how `CSAVE` → rewind →
`CLOAD` works without an explicit eject
([`cassette.rs:213-220`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cassette.rs#L213-L220))
— and without this guard, every rewind after a load would replace a
perfectly good tape with the demodulated garbage of whatever the DAC
happened to be doing. One `contains` call, one whole class of data-loss
bug prevented.

---

## 12.7 Motor mechanics: modeling an assumption, not a chip

Everything in §12.5 and §12.6 assumed the tape was already rolling. It is
not, at the instant the motor relay closes, and the gap between those two
statements is this chapter's cleanest example of something Chapter 1
promised would recur: sometimes what an emulator has to reproduce is not
a chip at all, but an *assumption* baked into the ROM's timing.

```rust
// crates/coco-core/src/cassette.rs:33-41
/// Motor spin-up: cycles after the relay closes before the tape reaches
/// speed and bits start flowing. The ROM pairs every motor-on with a blind
/// ~0.5 s countdown (`LA7D1`: 65536 iterations x 8 cycles, Color BASIC
/// Unravelled) precisely because real mechanisms need this long — without
/// modeling it, the tape rolls during the ROM's blind window and CLOAD's
/// second `CASON` ($A77C) lock-on eats the 128-byte leader before it ever
/// listens (CLOAD cycles the motor off/on between the namefile and data
/// blocks: `LA701`/`LA4D0`).
const MOTOR_SPINUP_CYCLES: u32 = 65536 * 8;
```

Start with the number. `65536 * 8` is 524,288 cycles, and dividing by
`CPU_HZ` gives approximately **0.586 seconds** — matching both the
comment's "~0.5 s" and the codebase's own test comment, which spells out
the same figure at
[`tests/cassette.rs:58`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/cassette.rs#L58).
That it lands on a tidy `512 * 1024` is a coincidence rather than the
point, and reading it as a round binary figure gets the derivation
backwards. The ROM's own delay loop at `LA7D1` runs 65,536 iterations of
an 8-cycle body, and this constant is that loop transcribed literally,
cycle for cycle, rather than a rounded-off "about half a second."
Writing it as `65536 * 8` instead of `524288` keeps the derivation
visible in the source, which is a small kindness to the next reader.

### The ROM never checks

Here is what makes this a *necessary* piece of emulation rather than an
optional flourish: **the ROM does not check whether the tape is up to
speed.** It closes the relay, then blindly burns `LA7D1`'s countdown —
0.586 seconds of doing nothing else whatsoever — on the theory that a
real electromechanical deck's motor and capstan need real physical time
to reach a stable read speed, and that after this much time has passed it
is safe to assume they have.

That is an assumption about *physics*, encoded as a fixed delay, with no
feedback loop verifying it. There is no "motor ready" line on the
cassette connector, no status bit to poll, nothing to ask. It is the
software equivalent of counting to ten before you start listening,
because you cannot ask the motor whether it is ready yet.

Now run the thought experiment the comment is implicitly describing.
Suppose the emulator's tape starts producing bits the instant the motor
bit goes high, with no spin-up delay modeled at all — which is the
obvious first implementation, and the one most people would write.

The ROM is still blindly burning its 0.586-second countdown. It has no
way of knowing, and no reason to care, that the emulated tape is already
moving. But the tape *is* moving in the emulator's model, feeding real
FSK edges to PA0 the entire time the ROM is not listening. Half a second
of tape at roughly 1500 bits per second is on the order of a hundred
bytes. By the time the countdown expires and `CASON`'s lock-on routine
finally starts sampling PA0, the tape has already played through the
entire leader run — the run that exists for the sole purpose of giving
the lock-on routine something to synchronize against. The comment's own
phrase for this, "eats the 128-byte leader before it ever listens," is
exactly the failure.

`CLOAD` does not crash. It does something worse from a debugging
standpoint: it times out, or reads garbage, or reports an I/O error, with
no indication anywhere that the problem is a missing delay in a
subsystem that contains no timing bug of its own. The emulator handed the
ROM a perfectly correct signal at the wrong moment relative to when the
ROM's unverified assumption said it was safe to start reading.

### Emulating the assumption

Model the delay — hold the tape completely still, bits and all, for those
524,288 cycles after every motor-on edge, exactly as `tick` does in its
`spinup_left` branch — and the leader run is still sitting there,
untouched, right where the ROM expects it, at the moment the ROM actually
starts listening.

This is "emulate the assumption, not the chip" in its purest form. There
is no motor object anywhere in this codebase. No torque, no capstan, no
inertia, no mass, no ramp-up curve. There is exactly one `u32` counter
that holds still for the same number of cycles a real motor's physics
would have consumed, because that is the *only observable consequence*
the ROM's blind trust in elapsed physical time produces, and it is
therefore the only consequence worth reproducing.

Compare that to the alternative an eager engineer might build: a motor
model with an angular-velocity variable, an acceleration constant, and a
tape speed that ramps smoothly from zero. It would be more physically
faithful, it would take an afternoon, and no software on the machine
could tell the difference — because nothing in the system ever samples
tape speed. That is the fidelity budget from Chapter 1 applied to a device
rather than to a bus: spend where software can notice.

### Why it has to be an edge detector

The comment's final parenthetical is practical rather than trivial, and
it explains a detail of `tick` that would otherwise look like defensive
programming. `CLOAD` does not spin the motor up once and leave it
running. It **cycles the motor off and then on again** between reading
the namefile block and reading the data blocks, at `LA701` and `LA4D0` —
which is why the end-to-end test in §12.10 has to wait for a *sustained*
period of motor-off before concluding that a tape operation has finished,
rather than treating the first motor-off as the end.

Every one of those re-engagements re-arms the same 0.586-second spin-up
latency, exactly as a real deck would. That is precisely why `tick`'s
detection is `motor_on && !self.motor_was_on` rather than a one-shot flag
set at mount time: the spin-up has to fire on *every* off-to-on
transition, not merely the first one after a tape is inserted. An
implementation that armed the delay only once would work perfectly for
the namefile block and then fail on the data blocks, which is a
frustrating and very believable bug to end up with.

---

## 12.8 The measurement story

This section is the chapter's best lesson, and it deserves telling in
full, because it is the clearest example in this entire codebase of a
methodology you will need again: **when the documentation runs out,
instrument your own emulator and measure the software's actual
behavior.**

### The gap in the documentation

Start with the gap, because the gap is specific and it is the reason
everything else in this section had to happen.

Color BASIC's `CSAVE` and `CLOAD` bit-banging code lives in the
**$A000–$BFFF** region of the 32K ROM — the second, un-extended half of
Color BASIC. This project's local reference material lives in a
git-ignored `docs/` directory of copyrighted PDFs, and among them is
*Super Extended Color BASIC Unravelled II*, a disassembly with
commentary that covers a great deal of the ROM in exactly the
line-by-line detail an emulator author wants. It does not cover the
$A000–$BFFF bit-bang routines. That is stated in the module header
quoted in §12.1, and it is the whole problem.

So there is no available prose describing what tones this ROM emits, at
what timing, for what reason. There is no data sheet, because there is no
chip. There is no standard to consult, because the tones are not quite
the standard ones. The only remaining source of truth is the ROM's own
bytes, executing.

Take a moment to appreciate how thoroughly this closes off the normal
routes. You cannot look it up. You cannot ask the hardware, absent a real
CoCo, an oscilloscope, and a Saturday. You could read another emulator's
source, but then you would be copying a number whose provenance you
cannot check, which is how wrong constants propagate between projects for
decades. What remains is to run the ROM and watch what it does.

### Turning the emulator into an instrument

That is what
[`crates/coco-core/examples/cassette_calibrate.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/cassette_calibrate.rs)
exists for:

```rust
// crates/coco-core/examples/cassette_calibrate.rs:51-65 (excerpt)
fn main() {
    let rom = std::fs::read("roms/coco3.rom").unwrap().into_boxed_slice();
    let mut m = Machine::new(MachineConfig::default(), rom);
    m.reset();
    for _ in 0..BOOT_FIELDS {
        m.run_field();
    }
    type_line(&mut m, "10 PRINT \"HI\"");
    type_line(&mut m, "CSAVE\"X\"");
    for _ in 0..SAVE_FIELDS {
        m.run_field();
        if m.bus.cassette.capture().len() > 200_000 {
            break; // safety valve
        }
    }
    let cap = m.bus.cassette.capture();
    // ... dump / histogram it
}
```

Read that for what it is, because it is not what it superficially
resembles. This is *not* a unit test with an expected answer baked in.
There is no assertion anywhere in the file. It boots the actual,
unmodified `roms/coco3.rom`, types a real one-line BASIC program through
the emulated keyboard exactly the way §12.1's opening ritual describes
doing it by hand, types `CSAVE"X"`, and then does nothing but *record
what the emulator's own DAC-tap mechanism already captures*.

That last point is the elegant bit. No instrumentation was added to the
emulator to make this measurement possible. The `record_dac` and
`capture()` machinery walked in §12.6 already existed, for the entirely
different purpose of implementing `CSAVE`, and this probe repurposes it
as a measuring instrument. The ROM has no idea it is being measured; it
is executing its normal cassette-save routine against a `Cassette` that
happens to double as a stopwatch.

Notice the `200_000` safety valve as well. A probe that runs against
real ROM code should assume the ROM might do something unexpected —
hang, loop, never turn the motor off — and bound its own resource use
rather than filling memory. Measurement code deserves the same defensive
instincts as production code, and gets them less often.

This worktree does not have `roms/coco3.rom`, since `roms/` is
git-ignored and present only on the machine this course was authored on,
so the probe was not re-run while this chapter was written and no claim
is made otherwise. What the source *does* make fully readable, without
the ROM in hand, is the methodology — and the constants the probe
produced are the ones baked into `cassette.rs` today, checkable by anyone
with a copy of the ROM.

### Two analyses, and why they disagree

The probe performs two distinct kinds of analysis on the same capture,
and the difference between them is the whole lesson.

**The first pass histograms raw transition deltas.** It walks the
capture and counts the time between *every consecutive DAC write*, no
matter what that gap represents:

```rust
// crates/coco-core/examples/cassette_calibrate.rs:86-97 (excerpt)
let mut hist: BTreeMap<u64, u32> = BTreeMap::new();
for i in 1..cap.len() {
    let delta = cap[i].cycle - cap[i - 1].cycle;
    *hist.entry(delta).or_insert(0) += 1;
}
```

**The second pass histograms zero-crossing deltas.** It computes the
midpoint of the observed DAC levels, finds every place the signal crosses
it, and measures the time *between crossings* — the same computation
`capture_to_bits` performs in production:

```rust
// crates/coco-core/examples/cassette_calibrate.rs:110-118 (excerpt)
let max_level = *levels.iter().max().unwrap();
let mid = max_level / 2;
let mut crossings: Vec<(u64, bool)> = Vec::new();
let mut side = cap[0].level > mid;
for t in &cap[1..] {
    let new_side = t.level > mid;
    if new_side != side {
        crossings.push((t.cycle, new_side));
        side = new_side;
    }
}
```

These two analyses do not measure the same thing, and understanding
exactly how they differ is the payoff of the whole exercise.

The first pass is dominated by the DAC's *step* rate. The ROM writes a
stepped approximation of a sine — the `input_bit` doc comment in §12.5
calls it "the ROM's DAC sine table" — so a single tone cycle involves
many DAC writes as the table walks the 6-bit output up to its peak and
back down. A raw-transition histogram counts every one of those steps as
a delta, which means its dominant values are the spacing between
individual sine-table entries, several times finer than a bit period. As
a measurement of the thing you actually want, it is a rough proxy at
best.

The second pass throws away every intermediate step and keeps only the
midpoint crossings, which is precisely what the SALT comparator does to
the signal on its way to PA0. Its histogram is therefore measuring
half-periods: the time from a rising crossing to the next falling one is
the high half of a bit cell, and from that falling crossing to the next
rising one is the low half. That is why the constants in `cassette.rs`
are stored as **four half-period numbers rather than two full periods**,
and why their doc comment describes them as "modal midpoint-crossing
spacings." The four modes of the second pass's histogram *are*
`ZERO_BIT_HIGH`, `ZERO_BIT_LOW`, `ONE_BIT_HIGH` and `ONE_BIT_LOW`. The
asymmetry §12.2 made so much of is not an extra refinement applied on
top of the measurement; it falls out of the measurement automatically,
because a half-period histogram cannot help but report both halves
separately.

### The numbers the comments remember

The probe's own module header records what an earlier pass concluded, and
it has been left in place rather than quietly corrected:

```rust
// crates/coco-core/examples/cassette_calibrate.rs:14-17
//! - Each bit is one full DAC sine cycle: a 0-bit measures ~793 CPU cycles
//!   (~1128 Hz), a 1-bit ~455 cycles (~1967 Hz) — close to, but not exactly,
//!   the canonical 1200/2400 Hz (a hand-tuned ROM delay loop, not a crystal-
//!   locked tone; the ROM's own hysteresis demodulator tolerates the drift).
```

**793 and 455 are not the constants in `cassette.rs` today**, which are
814 and 434. Those are not an error in this chapter; they are the
header's own honestly reported earlier figures, preserved rather than
silently rewritten, and the same pair that turns up in
`BIT_PERIOD_THRESHOLD`'s doc comment in §12.6. Whatever produced them —
a coarser analysis, an earlier state of the emulator's own instruction
timing, or both — they were superseded by the refined half-period
measurement that produced the four constants in force today.

There is a piece of arithmetic here worth checking rather than taking on
faith, because it explains why the stale figures survived so long
unnoticed: `793 + 455 = 1248`, and `814 + 434 = 1248`. The *same sum*.
The two pairs disagree about each individual period by roughly 21 cycles,
in opposite directions, and agree exactly about their midpoint —
`1248 / 2 = 624`, which is `BIT_PERIOD_THRESHOLD` to the cycle.

That is why refining the constants changed no behavior that any test
could see. The decision boundary, which is the only number demodulation
actually consults, did not move. The refinement mattered for a different
reason entirely: the *asymmetry*, which the old two-number framing could
not express at all, and which §12.2 explained is what keeps the ROM's own
half-period-counting demodulator off its decision boundary.

### The lesson, stated generally

The methodological lesson generalizes far past cassette tape, and it is
worth stating on its own terms: **when a data sheet or a disassembly does
not cover the behavior you need, make your own emulator into an
instrument.**

By the time you reach this chapter you have a CPU that executes the real
ROM correctly (Chapters 2 through 4), a bus that routes every access
correctly (Chapter 5), and I/O devices honest enough that the ROM cannot
tell it is not talking to real hardware (Chapters 10 and 11). That
combination is a measuring apparatus. It is not a disassembler, not a
data sheet, and not another emulator's source code; it is a working
replica of the machine, into which you can insert a probe at any point
and read out a timestamped record of what the original software actually
does.

The catch, and it is a real one, is that this only works for behavior
your emulator already gets right for other reasons. Measuring tape timing
with an emulator whose instruction cycle counts were wrong would produce
confident, precise, wrong numbers. The chain of dependencies runs
backwards through the whole course: the tape constants are trustworthy
because the CPU's cycle counts are trustworthy, which is why Chapters 2
through 4 spent so much effort on a table nobody enjoys copying. Every
measurement you take with an instrument you built yourself inherits every
error in the instrument.

---

## 12.9 WAV round-tripping

`cassette.rs` owns the CPU-facing FSK domain, in which the only thing
that matters is when a single bit flips. `cassette_wav.rs` owns
translating that domain to and from real audio. Its header states the two
reasons this is worth doing:

```rust
// crates/coco-core/src/cassette_wav.rs:1-10
//! WAV audio export/import for the cassette subsystem ([`crate::cassette`]):
//! turns the deck's decoded .cas byte stream into real tape audio, and real
//! tape audio back into decoded bytes — so a tape can be played into or
//! recorded from actual cassette hardware, or exchanged with tools that only
//! speak audio, not the .cas convention.
//!
//! This module owns the *audio* domain only. The bit-timing facts it needs
//! (tone periods, LSB-first bit order, the sync byte) are measured and
//! documented in `cassette.rs` and reused here via `pub(crate)` items rather
//! than duplicated: [`ZERO_BIT_PERIOD`], [`ONE_BIT_PERIOD`], [`SYNC`].
```

The first paragraph is the feature: a `.cas` tape can be turned into
audio and played into a real deck, or a WAV recorded from a physical
tape can be brought into the emulator. This is where the §12.3 argument
gets its escape hatch — the one scenario in which audio genuinely is the
more accurate representation is when the audio is the *only* thing you
have, because it came off a physical tape whose exact bytes nobody knows.

The second paragraph is a code-organization point with more consequence
than its brevity suggests. `ZERO_BIT_PERIOD`, `ONE_BIT_PERIOD` and
`SYNC` are declared `pub(crate)` in `cassette.rs` specifically so that
this module can `use` the same measured values rather than re-typing
`814`, `434` and `0x3C` as a second set of magic numbers. Two copies of a
measured constant will eventually disagree, because someone will refine
one and miss the other, and the resulting bug — WAV export producing
tones the emulator's own demodulator rejects — would be baffling to
track down. One source of truth, two consumers, and the visibility
modifier documents the relationship.

### Export: synthesizing real audio

```rust
// crates/coco-core/src/cassette_wav.rs:57-71
pub fn synthesize_wav(tape: &[u8], cpu_hz: f64) -> Vec<u8> {
    let mut samples = Vec::new();
    push_silence(&mut samples, WAV_LEAD_IN_SECS);

    for &byte in tape {
        for bit_index in 0..8u8 {
            let one = byte >> bit_index & 1 == 1; // LSB first
            let period_cycles = if one { ONE_BIT_PERIOD } else { ZERO_BIT_PERIOD };
            push_sine_cycle(&mut samples, period_cycles, cpu_hz);
        }
    }

    push_silence(&mut samples, WAV_LEAD_OUT_SECS);
    build_wav_bytes(&samples)
}
```

The structure mirrors `Cassette::tick` almost line for line — bytes,
then bits LSB first, then one tone per bit — which is the point. Both
functions are serializing the same tape under the same conventions; they
differ only in what they emit at the bottom of the loop. Where `tick`
advances a position and lets `input_bit` render a square edge,
`synthesize_wav` calls a function that writes actual samples:

```rust
// crates/coco-core/src/cassette_wav.rs:79-90
/// Append one full sine cycle representing a single tape bit's tone.
fn push_sine_cycle(samples: &mut Vec<u8>, period_cycles: u32, cpu_hz: f64) {
    let period_secs = f64::from(period_cycles) / cpu_hz;
    let cycle_samples = (period_secs * f64::from(WAV_SAMPLE_RATE_HZ)).round().max(1.0) as usize;
    for i in 0..cycle_samples {
        let phase = i as f64 / cycle_samples as f64;
        let value = f64::from(WAV_MIDPOINT) + WAV_AMPLITUDE * (std::f64::consts::TAU * phase).sin();
        // Float-to-int casts saturate (Rust since 1.45), so this can't
        // overflow even with float rounding at the extremes.
        samples.push(value.round() as u8);
    }
}
```

This is the actual sine from §12.2's "why square waves are enough"
discussion, finally synthesized, because here the consumer is not the
emulated CPU. It might be a real deck's line input, or a waveform viewer
(exercise 12.7), and both want the analog shape the ROM's DAC would have
produced rather than the comparator's squared rendering of it.

Three details in twelve lines are worth pulling out. The conversion from
cycles to samples goes through seconds: `period_cycles / cpu_hz` gives a
duration, and multiplying that by the WAV sample rate gives a sample
count. That is the only place in this entire chapter where a time is
expressed in anything but CPU cycles, and it is there because a WAV
file's timebase is defined in seconds. The `.max(1.0)` guards against a
pathological `cpu_hz` producing a zero-length cycle. And the comment on
the cast records a genuine Rust semantics fact: float-to-integer casts
saturate rather than wrapping or being undefined, so a rounding excursion
at the peak cannot produce a wrapped-around sample.

The doc comment above `synthesize_wav` is explicit that this output is
*not* inverted the way `Cassette::input_bit`'s PA0 rendering is, and the
reason bears repeating. `synthesize_wav` is modeling the record-side
output jack, where the DAC's own un-inverted sine appears.
`input_bit` is modeling the far end of the record-to-play analog path,
after whatever flips the polarity (§12.5). Same underlying tone, two
different points in the signal chain, two different polarities — and the
code keeps them straight by never sharing a function between them, which
is a more reliable defense than a comment saying "remember to invert."

`push_silence` frames the audio with a full second of dead air before the
tone and a quarter second after. Neither number came from measuring a
CoCo, and the constants' own doc comments say so directly: the lead-in
exists so that a real deck's motor and automatic gain control have time
to spin up and settle before data arrives, and the convention mirrors
what other tools, including MAME's `.cas`-to-audio loader, do. This is a
different and looser kind of fidelity than everything else in this
chapter — "what makes a real deck happy" rather than "what the ROM
measures" — and flagging it as such in the source is exactly right.

### Import: decoding uncertain audio

Decoding a WAV back into tape bytes has to cope with uncertainty that a
synthetic capture never has. Two uncertainties, specifically. The
**polarity** is unknown, because which side of the signal counts as
"high" depends on how the recording was made — which deck, which cable,
which input stage — and nothing in the file format records it. And there
is **noise near the zero crossing**, because a real recording is a real
recording.

Both are handled honestly rather than assumed away:

```rust
// crates/coco-core/src/cassette_wav.rs:390-406 (excerpt)
pub fn decode_wav(bytes: &[u8], cpu_hz: f64) -> Result<Vec<u8>, WavError> {
    let (fmt, data) = parse_wav_chunks(bytes)?;
    let samples = extract_mono_samples(data, &fmt);
    // ... compute mid/hysteresis from the capture's own min/max
    let normal = capture_transitions(&samples, mid, hysteresis, false, cpu_hz, fmt.sample_rate_hz);
    let inverted = capture_transitions(&samples, mid, hysteresis, true, cpu_hz, fmt.sample_rate_hz);
    Ok(choose_best_decode(demodulate(&normal), demodulate(&inverted)))
}
```

Rather than guess the polarity, `decode_wav` tries **both**. It runs the
entire crossing-detection and demodulation pipeline twice, once on the
signal as read and once phase-inverted, and then picks:

```rust
// crates/coco-core/src/cassette_wav.rs:369-377
fn choose_best_decode(a: Vec<u8>, b: Vec<u8>) -> Vec<u8> {
    match (a.contains(&SYNC), b.contains(&SYNC)) {
        (true, false) => a,
        (false, true) => b,
        _ => {
            if a.len() >= b.len() { a } else { b }
        }
    }
}
```

The selection criterion is the presence of a `SYNC` byte, which is the
same structural signal `finalize_recording` uses in §12.6 and for the
same reason: `bits_to_bytes` only emits a sync byte when it has locked
onto real block structure, so a decode containing one is overwhelmingly
likely to be the correct interpretation. When both decodes contain a sync
byte, or neither does, the tie-break falls back to length, which is a
weaker signal but better than a coin flip.

This mirrors, at the audio boundary, exactly the "hunt, don't assume"
philosophy §12.6 walked inside `bits_to_bytes`. When you cannot be
certain of something — byte alignment there, signal polarity here — the
answer is not to pick one interpretation and hope. It is to try every
plausible interpretation and let a strong structural signal tell you
which one was right. The cost is running the pipeline twice, on a code
path that executes once per file import, which is exactly the kind of
place where doubling the work is free.

The noise problem gets its own treatment, and its own honest label:

```rust
// crates/coco-core/src/cassette_wav.rs:291-296
/// Hysteresis band half-width as a fraction of the signal's peak-to-peak
/// range, centered on the midpoint: real recordings carry noise near the
/// zero crossing, so without a deadband a slow drift around the midpoint
/// alone would produce spurious extra transitions. A tunable heuristic, not
/// a hardware fact.
const WAV_HYSTERESIS_FRACTION: f64 = 0.1;
```

*Hysteresis* here means the crossing detector uses two thresholds rather
than one: the signal must rise above `mid + hysteresis` to count as
having gone high, and fall below `mid - hysteresis` to count as having
gone low. A signal loitering within the band changes nothing. Without
that deadband, a recording whose baseline drifts slowly across the
midpoint would generate a burst of spurious crossings, each of which
would be measured as an absurdly short period and demodulated as a
1-bit. The fully synthetic playback path in `cassette.rs` never needs
this, because its crossings are computed rather than measured and cannot
jitter.

Notice, too, that the constant's doc comment ends by classifying itself:
"a tunable heuristic, not a hardware fact." That is a small discipline
with a large payoff in a codebase like this one, where most constants
*are* hardware facts and a reader is entitled to assume so. Labelling the
exceptions means the unlabeled ones can be trusted.

One last detail bridges this module back to the rest of the chapter:

```rust
// crates/coco-core/src/cassette_wav.rs:361-363
fn sample_to_cycle(sample_index: usize, cpu_hz: f64, wav_sample_rate_hz: u32) -> u64 {
    (sample_index as f64 * cpu_hz / f64::from(wav_sample_rate_hz)).round() as u64
}
```

`demodulate` consumes `Transition`s timestamped in CPU cycles, and a WAV
file is indexed in samples, so the audio importer converts. Three lines,
and they are why the entire demodulation pipeline could be reused
verbatim across a format boundary: the pipeline's interface was
specified in the machine's own currency rather than in whatever unit
happened to be convenient at the call site. Chapter 1 claimed cycles are the
unit of account for the whole machine; this function is the invoice.

The tests in §12.10 confirm that this round-trips through both 8-bit and
16-bit PCM and survives a full polarity inversion, which is worth reading
now that you know what machinery is being exercised.

---

## 12.10 Reading the tests

[`crates/coco-core/tests/cassette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/cassette.rs)
splits cleanly into two halves: tests that need nothing but the
`Cassette` and `cassette_wav` APIs, which always run, and one end-to-end
test against the real `roms/coco3.rom`. Running the file on a checkout
without `roms/` — a fresh clone or a git worktree, since Chapter 1
established that the directory is git-ignored and lives only in the main
checkout — confirms exactly that split:

```
$ cargo test -p coco-core --test cassette
running 8 tests
skipping csave_rewind_cload_round_trips_a_basic_program: roms/ not present
test csave_rewind_cload_round_trips_a_basic_program ... ok
test motor_off_freezes_the_tape_and_records_nothing ... ok
test wav_decode_rejects_truncated_header ... ok
test playback_waveform_demodulates_back_to_the_same_bytes ... ok
test wav_decode_rejects_non_pcm_format_tag ... ok
test wav_round_trip_preserves_the_tape_bytes ... ok
test wav_round_trip_survives_inverted_polarity ... ok
test wav_round_trip_via_16_bit_pcm ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

That is the actual output of running it, not a hypothetical, and there is
one detail in it worth noticing: `csave_rewind_cload_round_trips_a_basic_program`
still reports `... ok`. It is not marked `#[ignore]`. It is an ordinary
test that runs, looks for the ROM at `../../roms/coco3.rom`, prints a
message to `stderr` when it is not there, and returns early. That pattern
has appeared before in this course — [`tests/coco1_boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/coco1_boot.rs)
in Chapter 6 does the same thing — and it will appear again in disks and
serial in Chapters 13 and 14. Every other test in the file needs nothing but
the code you have already read.

### `playback_waveform_demodulates_back_to_the_same_bytes`

This is the tightest and most important test in the file, and its claim
is a strong one: the whole modulation pipeline must be its own inverse,
with no ROM involved anywhere.

```rust
// crates/coco-core/tests/cassette.rs:36-73 (excerpt)
#[test]
fn playback_waveform_demodulates_back_to_the_same_bytes() {
    const TICK_CYCLES: u32 = 7;

    let mut tape = vec![LEADER; 16];
    tape.extend(tape_block(0x00, b"X       \x00\x00\x01\x3F\x00\x3F\x00"));
    tape.extend(vec![LEADER; 16]);
    tape.extend(tape_block(0x01, &[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x55]));
    tape.extend(tape_block(0xFF, &[]));

    let mut deck = Cassette::new();
    deck.insert_tape(tape.clone());

    let mut capture: Vec<Transition> = Vec::new();
    let mut clock = 0u64;
    let mut last = None;
    deck.tick(600_000, true);
    assert_eq!(deck.position().0, 0, "tape must hold still through spin-up");
    while deck.playing() {
        deck.tick(TICK_CYCLES, true);
        clock += u64::from(TICK_CYCLES);
        let level = if deck.input_bit() { 0 } else { 63 };
        if last != Some(level) {
            capture.push(Transition { level, cycle: clock });
            last = Some(level);
        }
    }

    assert_eq!(demodulate(&capture), tape);
}
```

Before the details, notice the shape of the fixture. The synthetic tape
is a plausible miniature of a real one: a leader run, a namefile block
whose payload begins with the same space-padded `X` the ROM produces,
another leader run, a data block carrying deliberately awkward bytes, and
an empty EOF block. The payload `0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x55` is
chosen with some care — it includes a byte of all zeros, a byte that is
itself a leader value, and four bytes with plenty of adjacent-bit
variety, which between them exercise the demodulator's bit-boundary
handling far better than a run of identical bytes would.

Three details in the body deserve a second look.

**`TICK_CYCLES = 7`** is described by its own comment as "deliberately
not a divisor of either bit period," and the reasoning is worth adopting
as a habit — though the comment overstates its case by one period, which
is §12.6's arithmetic-checking habit paying off a second time. Do the
division: 814 is 7 × 116 + 2, so 7 genuinely is not a divisor there, but
434 is 7 × 62 exactly, so the 1-bit's period is a whole number of ticks.
What actually holds is the property the test needs. The 0-bit's period is
not a multiple of the stride, so the moment one 0-bit goes by the phase
shifts and every bit boundary after it falls at a different offset within
some 7-cycle step. A bug in how `tick` handles a partial bit at the end
of its cycle budget — which is the `while` loop's whole job, §12.5 —
therefore shows up as compounding phase error rather than being
accidentally masked. A test that ticked in units of 814 would pass
against a badly broken implementation. Choose the granularity that
stresses the general case, not the convenient one.

**`deck.tick(600_000, true)` in a single call** burns through the entire
524,288-cycle spin-up latency in one jump, which is legitimate precisely
because `tick` accumulates a cycle budget rather than assuming small
increments. The assertion immediately after it, `deck.position().0 == 0`,
directly checks §12.7's "the tape must hold still" claim rather than
assuming it — and it would catch an implementation that armed the
spin-up but forgot to gate playback on it.

**The level mapping is inverted**, and this is the detail most likely to
trip up someone writing a second test in this file. `if deck.input_bit()
{ 0 } else { 63 }` maps a *high* PA0 to a DAC level of *zero*. That is
not a typo. `input_bit()` returns PA0's value, which is the SALT-inverted
rendering of the tape signal (§12.5), while `demodulate` consumes
DAC-domain transitions (§12.6), which are not inverted. The test is
translating between the two domains itself, because nothing else in the
system ever has to: in the real machine, the record path and the playback
path are connected through a tape, not through each other. If you add a
test that feeds `input_bit()` output into `demodulate`, this inversion is
the first thing to get right and the easiest to get wrong.

### `motor_off_freezes_the_tape_and_records_nothing`

A short, sharp confirmation of the motor-gating claims from §12.5 and
§12.6, worth reading for how little code it takes once the API is shaped
correctly:

```rust
// crates/coco-core/tests/cassette.rs:75-86
#[test]
fn motor_off_freezes_the_tape_and_records_nothing() {
    let mut deck = Cassette::new();
    deck.insert_tape(vec![LEADER; 8]);
    deck.tick(10_000, false);
    assert_eq!(deck.position().0, 0, "tape must not move with the motor off");
    assert!(deck.input_bit(), "input idles high with the motor off");

    deck.record_dac(63, false);
    deck.record_dac(0, false);
    assert!(deck.capture().is_empty(), "nothing records with the motor off");
}
```

Both halves of the deck are checked against the same condition,
independently, in one test. Ticking for ten thousand cycles with the
motor off moves the tape exactly nowhere and leaves the line idling
high. Two DAC writes with the motor off produce exactly zero captured
transitions. It looks almost too simple to be worth writing, but it is
checking a real invariant that spans two otherwise unrelated code paths:
the motor line gates *everything*, not just the side of the deck you
happened to be thinking about when you wrote the gate.

### `csave_rewind_cload_round_trips_a_basic_program`

The full end-to-end test walks §12.1's entire ritual in code. It mounts a
blank tape, types a program, issues `CSAVE"X"`, waits for the motor to go
idle, and rewinds — which finalizes the recording, since
`Cassette::rewind` calls `finalize_recording` first, so `CSAVE` → rewind
→ `CLOAD` works with no eject cycle. It then checks the block structure,
types `NEW` to wipe BASIC's program, loads it back with `CLOAD`, runs it,
and checks the screen for the program's actual output.

This is the test whose block-structure assertions opened the chapter in
§12.1, and you now have every piece of machinery needed to read the rest
of it top to bottom without help. Two of its supporting functions repay
attention on their own. `run_until_motor_idle` waits for a *sustained*
stretch of motor-off rather than the first one, for exactly the reason
§12.7 gave: `CLOAD` cycles the relay mid-operation, and treating the
first motor-off as completion would cut the test short in the middle of a
tape operation. And `parse_blocks` re-derives the block structure from
the decoded bytes and asserts every checksum, which means the test is
verifying the *format*, not merely that some bytes survived a round trip.

The test needs `roms/coco3.rom`, so on a checkout without it — as
confirmed by actually running the suite above — it exercises only its own
early-return path.
[`crates/coco-core/tests/coco2_boot/cassette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/coco2_boot/cassette.rs)
runs a near-identical scenario against a CoCo 2 boot, using
`extbas11.rom` and `bas12.rom`, and its header explains why that
duplication is worth its weight:

```rust
// crates/coco-core/tests/coco2_boot/cassette.rs:1-10
//! Phase 6 acceptance test 4: cassette CSAVE/CLOAD (`docs/coco12-plan.md`)
//!
//! The cassette deck (`Cassette`, `bus.rs`'s PIA1 record/playback wiring) is
//! entirely machine-neutral — confirmed by inspection: `bus.rs::sam_io_write`'s
//! PIA1 branch feeds `Cassette::record_dac` exactly like the GIME path's
//! `io_write` does, and `Machine::pia1_pa_pins`'s cassette-input bit doesn't
//! consult `self.config.variant` at all. This is a regression guard for that
//! wiring on the plain-SAM bus path...
```

The claim being guarded is "machine-neutral," and the header is careful
to say it was confirmed *by inspection* rather than by construction.
Nothing in the type system prevents someone from adding a
variant-dependent branch to the cassette wiring later; the test exists so
that if they do, something goes red. This is a regression guard in the
precise sense — it protects a property that is currently true and could
quietly stop being true.

That test's behavior on a ROM-less checkout is the same, for the same
reason:

```
$ cargo test -p coco-core --test coco2_boot -- cassette
running 1 test
skipping coco2_boot: extbas11.rom/bas12.rom not present in roms/ (see docs/coco12-plan.md "ROM files")
test cassette::coco2_csave_rewind_cload_round_trips_a_basic_program ... ok
```

Both end-to-end tests are honest about their dependency and skip cleanly
rather than failing or, worse, passing silently without exercising
anything real. That is a pattern worth stealing for your own device
tests: a test that cannot run should say so on `stderr`, in a sentence
that names both the missing file and where to read about it. Chapter 13's
disk tests do exactly the same thing, for the same reason.

---

## 12.11 Reading assignment

In this order:

1. **[`crates/coco-core/src/cassette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cassette.rs),
   the whole file (~451 lines).** Read the module header first, then the
   constants block (lines 1–66), then `Cassette::tick` and `input_bit`
   for playback (§12.5), then `record_dac`, `demodulate`,
   `capture_to_bits` and `bits_to_bytes` for recording (§12.6). By now
   every doc comment in this file should read as a claim you can verify
   rather than a fact to take on faith — and where a comment says
   "verified empirically," you should be able to describe what the
   verification would have looked like.
2. **[`crates/coco-core/src/cassette_wav.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cassette_wav.rs).**
   Read the constants and `synthesize_wav` closely; skim `decode_wav`'s
   chunk-parsing machinery, since it is ordinary defensive file-format
   parsing. The interesting part is the polarity guessing in §12.9.
   While you are in the file, note how many constants explicitly
   label themselves as conventions rather than hardware facts.
3. **[`crates/coco-core/examples/cassette_calibrate.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/cassette_calibrate.rs).**
   Read this as a measurement instrument rather than as application code.
   Notice what it captures (raw DAC transitions) as distinct from what
   it computes from that capture (two different histograms), and connect
   both back to §12.8.
4. **[`crates/coco-core/tests/cassette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/cassette.rs).**
   You have now read every test in this file in §12.10. Re-read
   `playback_waveform_demodulates_back_to_the_same_bytes` once more, end
   to end without stopping, and confirm that you can predict what
   `demodulate(&capture)` returns before you reach the assertion.

Then run the ROM-free suite and confirm you see the same eight tests this
chapter did:

```
cargo test -p coco-core --test cassette
```

---

## 12.12 Exercises

**12.1 — FSK arithmetic, both directions (build).** Starting from
`CPU_HZ = 894_886.0` and the nominal Kansas-City figures (1200 Hz / 2400
Hz), compute what bit period in CPU cycles each *would* be if the CoCo's
tones matched them exactly (`period = CPU_HZ / freq_hz`, rounded). Then
go the other direction: starting from the actual constants in
`cassette.rs` (`ZERO_BIT_PERIOD = 814`, `ONE_BIT_PERIOD = 434`), compute
the frequencies they really produce. Confirm your four numbers land where
§12.2's table says they do, and write one sentence on why "1200/2400" is
a reasonable *name* for this FSK scheme despite not being its exact
frequencies.

**12.2 — Threshold sensitivity (verify, then reason).** This one was
actually run rather than merely reasoned about, and it should be
reproduced: temporarily change `BIT_PERIOD_THRESHOLD`'s definition in
`cassette.rs` to a literal `499` (≈ `624 * 0.8`, a −20% shift) and run
`cargo test -p coco-core --test cassette`. All 8 tests still pass — the
synthetic playback test's periods (814 and 434) sit far enough from 624
(190 cycles either way, ≈30.4% of 624) that a 20% threshold shift doesn't
cross either one. Now push it further: try `430` (just below the
1-bit's own period of 434) and re-run — `playback_waveform_demodulates_back_to_the_same_bytes`
fails, because *every* 1-bit in the test's synthetic tape now measures a
period (434) exceeding the threshold (430) and gets classified as a 0.
Two things to explain in your own words: (a) why the ±20% case in the
syllabus's own framing doesn't break *this* codebase's tests at all — what
would need to be true of the test fixture for it to be threshold-sensitive
in that range, and why isn't it? (b) Once you push far enough to break
something, why does it break as an *all-or-nothing* flip (every 1-bit at
once) rather than "which bit pattern fails first" — and what would have
to change about how the test's tape is synthesized for the question "which
pattern fails first" to even have an answer? (Hint: compare how
`ZERO_BIT_HIGH`/`ZERO_BIT_LOW` encode *measured* asymmetry within one bit
against how the test's synthetic capture generates bits with zero
per-instance jitter.) Revert your edit to the real formula
(`(ZERO_BIT_PERIOD as u64 + ONE_BIT_PERIOD as u64) / 2`) and confirm
`cargo test -p coco-core --test cassette` is clean and `git status` is
clean before moving on.

**12.3 — Sabotage the bit order, and watch leader survive (sabotage,
verified).** In `bits_to_bytes`, change
`window = window >> 1 | u8::from(bit) << 7;` to
`window = window << 1 | u8::from(bit);` — assembling each byte MSB-first
instead of LSB-first. Predict the outcome before you run
`cargo test -p coco-core --test cassette`. The verified outcome:
**3 of 8 tests fail**
(`playback_waveform_demodulates_back_to_the_same_bytes`,
`wav_round_trip_preserves_the_tape_bytes`,
`wav_round_trip_survives_inverted_polarity`) — and the failure output is
the interesting part: the demodulated output's *leader and sync bytes
still come out correct* (`85, 85, ..., 60, ...` at the start of both
`left` and `right` in the assertion diff), while every byte after the
sync diverges. Explain why, using two facts about the specific bytes
`LEADER = 0x55` and `SYNC = 0x3C`: `0x3C` (`00111100`) is a literal
bit-palindrome, so byte-order doesn't affect whether it's recognized at
all; `0x55` (`01010101`) is a period-2 alternating pattern, and an 8-bit
window sliced from an infinite alternating bitstream is `0x55` or `0xAA`
depending only on *phase*, regardless of which shift convention
assembled it — so the sliding hunt in `Hunt` state still locks on
correctly even under the sabotaged shift. Only once real (non-repeating,
non-palindromic) payload data starts flowing does the bug become visible.
Revert your edit exactly (back to
`window = window >> 1 | u8::from(bit) << 7; // LSB arrives first`),
re-run the test suite to confirm all 8 pass again, and confirm
`git status` shows a clean tree.

**12.4 — Build: demodulate a byte you chose yourself (build).** Using
only the public API (`Cassette::new`, `insert_tape`, `tick`, `playing`,
`input_bit`, and `cassette::{demodulate, Transition}`), write a new test
that mounts a tiny hand-picked tape — say `vec![0x55, 0x55, 0x3C, 0xA5,
0x00, checksum, 0x55]` framed as one block via a helper like this
chapter's `tape_block` — plays it all the way through the way
`playback_waveform_demodulates_back_to_the_same_bytes` does (burn the
spin-up in one `tick`, then loop `tick`/`input_bit` capturing transitions
on every level change, translating PA0 polarity to DAC polarity as
§12.10 explained), and asserts `demodulate(&capture) == tape`. Confirm it
passes. This is deliberately close to the existing test — the exercise is
building it yourself from the public surface, not inventing new
machinery.

**12.5 — Read and predict: spin-up removed (read + predict).** Without
running anything yet, read `Cassette::tick`'s `spinup_left` handling
(§12.5) and its own doc comment (§12.7) closely, then predict, in your
own words, what would happen to the
`csave_rewind_cload_round_trips_a_basic_program` test (§12.10) if
`MOTOR_SPINUP_CYCLES` were changed to `0` — specifically, would `CSAVE`
still work, would `CLOAD` still work, and if one breaks and not the
other, which one and why (tie your answer to the comment's specific claim
about `CASON`'s lock-on eating the leader). Then — if you have
`roms/coco3.rom` available — actually make the change and run the test to
check your prediction; if you don't have the ROM, say so honestly and
leave your prediction as reasoned-through rather than verified, the same
distinction this chapter draws throughout §12.8. Either way, revert the
constant back to `65536 * 8` before moving on.

**12.6 — Essay, three sentences max (essay).** A colleague suggests: "storing
`.wav` audio instead of `.cas` bytes would actually be *more* accurate,
since it's what a real tape really contains — why throw that away for a
lossy decoded format?" Give the two strongest reasons this codebase
rejects that framing for its primary storage format (one is about what
"accurate" should mean for a format whose only consumer is the emulator's
own demodulator, which decodes losslessly either way; one is about the
cost `.wav`-as-primary would impose on every `CLOAD`, not just the ones
that need it). If you can name a scenario where `.wav` genuinely *is* the
more accurate choice — this codebase supports one — you've found the
exception that proves the rule.

**12.7 — Read the bits by eye (explore).** If you have access to a real
`roms/coco3.rom` and a working `cargo run -p coco-egui` build (or any
build of this emulator with cassette support wired to the UI): `CSAVE` a
short program, export the tape as a `.wav` via `cassette_wav`, and open
it in Audacity or any waveform viewer. Zoom in far enough to see
individual cycles and find the leader run (long, uniform, one frequency),
the sync transition (a visible frequency change), and try reading the
first data byte's bits by eye, LSB first, using §12.2's two frequencies
as your guide. This exercise depends on resources this worktree doesn't
have (`roms/`), so treat it as a lab you run on a machine that does, not
one you can complete here — but it's the single most direct way to
confirm everything this chapter told you about the signal actually looks
like that on a real waveform.

---

## What's next

Part IV is done, and the throughline of the whole part is worth naming
before it disappears. Chapter 10 built the PIAs. Chapter 11 built the audio
path they feed. Chapter 12 turned one PIA pin, plus a DAC register, into a
complete two-way modem under nothing but ROM software. Every device in
this part turned out to be "a PIA pin plus interpretation," and that is
not a coincidence of how the chapters were ordered — it is the real
reason this machine could be built as cheaply as it was. Tandy bought
general-purpose parts and spent software on making them behave like
special-purpose ones.

Part V trades that pattern for its opposite. Chapter 13 introduces the
WD1773 floppy controller: a real, dedicated chip with its own command
state machine, which the CPU talks to rather than bit-banging. You will
meet the **HALT/NMI handshake** that Chapter 6 quietly set up for — the HALT
check in `step_cpu_unit`, whose exact call site you have now read, two
lines above the cassette and cartridge `tick` calls in
[`machine/run.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs).

You will also see the same "functional, not cycle-exact" fidelity
decision this chapter's tape faced, resolved in the opposite direction —
and for an instructive reason. It will not be because the exact behavior
cannot be derived; the WD1773 has a data sheet, which is more than the
cassette ever had. It will be because disk software, unlike the cassette
ROM you have just spent a whole chapter matching cycle for cycle, does
not count cycles at all. It waits on a status bit, and a status bit does
not care whether the answer arrives in eighty microseconds or a hundred.
Same question, opposite answer, because a different piece of 1980s
software is asking it.
