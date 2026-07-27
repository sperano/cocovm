# Chapter 12 — The Cassette: FSK Modems, 1980 Edition

*Week 12. Goal: a complete, self-contained signal-processing story, start to
finish, in one subsystem small enough to hold in your head. Week 10 gave you
the PIA — the chip that mediates almost all CoCo I/O — and week 11 walked
the audio path the PIA's DAC feeds. This week reuses both: the cassette
"modem" is nothing but a PIA pin and a DAC register, driven by software.
By the end of this chapter you will know exactly how a byte becomes a tone,
how a tone becomes a byte again, why the emulator has to model a mechanical
delay that has nothing to do with data at all, and — the chapter's best
lesson — what to do when the documentation you're relying on simply stops
covering the code you need. This closes Part IV.*

---

## 12.1 The ritual, and the machine underneath

If you owned a CoCo before you owned a disk drive, you know this sequence
in your hands before you know it in words. You set the cassette deck's
volume somewhere in the middle of its dial — too quiet and the load fails
with a garbled program, too loud and it also fails, for a reason you
couldn't articulate at the time but will understand precisely by the end of
§12.2. You wrote down the tape counter's three-digit number after a `CSAVE`
so that next time you could wind straight to it instead of listening
through ten minutes of previous programs. You typed `CLOAD"NAME"`, pressed
`PLAY` on the deck, and waited — the motor engaging with an audible clunk,
a faint high-pitched warble leaking out of the deck's speaker if you had
the monitor turned up, and the cursor sitting there giving you nothing to
watch. If you'd wound to the wrong counter position, nothing visibly
happened: BASIC was reading blocks, checking the eight-character name
against the one you asked for, and — silently, patiently — skipping every
block that didn't match, waiting for the next one. That's not a UI
omission. It's the literal shape of the tape format: every program starts
with a small **namefile** block carrying its name, followed by a run of
**data** blocks, followed by an **end-of-file** block — and `CLOAD"NAME"`
is nothing more than "read namefile blocks until one matches, then read
data blocks until EOF." You can see that exact three-part shape asserted
against a real, booted ROM in this codebase's own test, which we'll walk
in §12.10:

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

Now the reveal, and it's the reason this chapter exists at all: **the CoCo
has no tape controller chip.** There is no MC6850-style UART wired to the
cassette jack, no dedicated modem IC doing the frequency-shift keying for
you. There's a $5 op-amp comparator (the "SALT" chip on the CoCo 3's board
— Sound And Light Transducer-ish, functionally a zero-crossing detector)
between the tape jack and one bit of one PIA, and everything else — timing
the tones, generating them, decoding them, hunting for byte alignment,
checksumming — is done by 6809 machine code in ROM, bit-banging a single
pin. `CSAVE` and `CLOAD` are software. The "modem" you're used to thinking
of as a piece of hardware is, on this machine, a delay loop.

That fact doubles the emulator's job, and it's worth stating precisely
because it's the thesis of this whole chapter:

1. **Model the tape** — a piece of analog mechanism with a motor, a
   physical position, a recorded signal, and startup latency.
2. **Satisfy the ROM's own software demodulator** — because there is no
   hardware standard to target, only whatever the specific delay-loop
   constants in Color BASIC happen to produce. Get the tone frequencies
   even slightly wrong and it isn't "close enough" — the ROM's own
   polling loop, expecting real physical time, either never triggers or
   triggers on the wrong cycle.

Put those together and you get this chapter's real subject: the emulator
contains a *second* software modem, of its own, pointed at the ROM's
software modem. Two decoders, written thirty-some years apart, that have
to agree byte for byte on a made-up FSK dialect neither one invented.

The whole thing lives in one file, and its own header states the fidelity
contract up front — read it closely, because every section from here on is
an expansion of one sentence in it:

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

(`cassette-verified-facts` in that comment is an internal project note, not
a file you can open — treat it the way you'd treat "per an earlier
conversation with the hardware." What matters for you is that every claim
in this chapter traces back either to code you can read yourself or to the
empirical measurement story in §12.8.)

---

## 12.2 FSK from zero

**Frequency-shift keying (FSK)** is the oldest trick in the modem book:
instead of sending a 0 or a 1 as a voltage level (which a cassette tape,
an AC-coupled medium, can't hold — DC drifts away, that's what "AC-coupled"
means), you send it as one of two *tones*. A 0-bit is a burst of one
frequency; a 1-bit is a burst of another. Whatever plays the tape back only
has to answer one question over and over — "was that tone the slow one or
the fast one?" — and a tone survives tape hiss, wow, and flutter far better
than an absolute voltage level would.

The CoCo's convention, as measured against the real ROM (the whole story
of *how* is §12.8):

- **0-bit** ≈ 1100 Hz
- **1-bit** ≈ 2060 Hz
- **one full sine cycle per bit** — not several cycles of a fixed carrier
  the way a real telephone modem would do it, just exactly one period of
  the appropriate tone, then straight on to the next bit's tone. A byte is
  eight back-to-back single-cycle tone bursts.
- **bits are sent LSB first** — the same order the 6809's `ROR`/shift
  instructions would naturally produce if you were bit-banging a byte out
  of a register one carry-flag at a time, which is exactly what the ROM is
  doing.

Here are the constants exactly as measured, from the top of `cassette.rs`:

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

### From cycles to Hertz, and back

Emulator code, as you know from week 1, thinks in CPU cycles, not seconds.
The CoCo's clock is `CPU_HZ = 894_886.0` (`crates/coco-core/src/machine.rs:26`
— the NTSC color subcarrier 3.579545 MHz divided by 4, the same constant
ch. 1 introduced). Converting a period in cycles to a frequency is just
`CPU_HZ / period_cycles`:

| Bit | Period (cycles) | Frequency | Nominal FSK name |
|-----|-----------------:|----------:|-------------------|
| 0   | 396 + 418 = 814   | 894886/814 ≈ **1099.4 Hz** | "1200 Hz" |
| 1   | 207 + 227 = 434   | 894886/434 ≈ **2061.9 Hz** | "2400 Hz" |

Those "nominal" names in the right column are doing real work: they're the
classic **Kansas City standard** tone pair that a lot of 1970s/80s home
computers used for cassette I/O (300-baud CUTS-style). The CoCo's ROM
tones are *close* to that convention — close enough that "1200/2400" is
how CoCo cassette I/O gets described casually — but not exact, because
they were never crystal-derived. They're the byproduct of a hand-tuned
6809 delay loop counting cycles, and hand-tuned delay loops land wherever
the programmer's arithmetic put them, not on a round number. This
discrepancy between "the number everyone calls it" and "the number the
silicon (or in this case, the software) actually produces" is a pattern
you'll meet constantly writing emulators: never trust the folklore
figure when you can measure the real one.

Both bit periods also encode an asymmetry: the 0-bit's high half (396)
is shorter than its low half (418) — a 94.7% ratio — and the 1-bit shows
the same skew (207 vs. 227, a 91.2% ratio). That's not measurement noise
smoothed differently in two places; it's the same physical lopsidedness
in both tones, which is exactly the kind of detail that only survives if
you measured the real waveform instead of assuming a textbook symmetric
square wave. §12.8 explains why matching it, not just the gross period,
matters.

### Why square waves are enough

A real cassette tape stores an analog sine wave — that's genuinely what
the CoCo's 6-bit DAC synthesizes on the way out (`cassette_wav.rs`
reproduces that sine exactly for WAV export, §12.9). But between the tape
head and the CPU sits the SALT chip, a **zero-crossing detector**: a
comparator that outputs a clean digital high or low depending on which
side of the signal's midpoint the (AC-coupled, so midpoint ≈ 0V) waveform
currently sits. It throws away *everything* about the signal except "is it
above or below the middle right now" — amplitude, harmonic content, exact
sine shape, all discarded. The ROM never sees a sine wave. It sees a
square wave, one edge per zero crossing.

That means the emulator doesn't need to synthesize sine samples to be
correct for the CPU-facing side of this interface at all — it only needs
to flip a single bit (PA0) at the right cycle counts, which is exactly
what `Cassette::input_bit()` does (§12.5). The sine only has to exist where
something *outside* the emulated CPU could plausibly care about it: real
audio export to a WAV file that might feed a physical tape deck's AGC and
line input (§12.9). Two representations of the same signal, for two
different consumers, and the codebase keeps them in two different
functions rather than pretending one waveform serves both jobs.

---

## 12.3 The `.cas` decision: bytes, not audio

Before any code, one architectural choice governs everything else in this
chapter, and it's a textbook example of the fidelity-budget thinking
ch. 1 asked you to practice on every subsystem: **what does the tape image
on disk actually store?**

Two options exist, and both are real formats other emulators use:

- **`.wav`** — literal audio samples: 44,100 (or whatever) amplitude
  values per second, exactly what a sound card would play through a real
  deck's line input.
- **`.cas`** — the *decoded byte stream*: the leader bytes, the sync byte,
  the block type/length/payload/checksum, as plain `u8`s. Not audio at
  all. "What the BIOS reads and writes," as the module header puts it.

This codebase chose `.cas`, and it's worth spelling out why, because the
consequences ripple through the rest of the chapter.

**Storage cost.** A `.wav` recording of a CSAVE'd one-line BASIC program
easily runs to hundreds of kilobytes of 8-bit PCM at a real deck's
sample rate; the same program as decoded bytes is maybe 60 of them. Four
orders of magnitude, for information that's 100% redundant — the audio
*is* the bytes, just re-expanded into a wasteful representation.

**Work at load time.** If tapes were stored as audio, then even a
same-emulator round trip (CSAVE now, CLOAD later) would force every
`CLOAD` through the full crossing-detection/demodulation pipeline just
to recover data the emulator itself wrote moments earlier and already
knew perfectly. Storing decoded bytes means playback is just "walk this
byte array" — no DSP unless you're crossing a format boundary on purpose
(§12.9).

**The consequence you must accept in exchange:** because the file format
is bytes, not waveform, the emulator has to be able to go **both**
directions between bytes and edges, honestly, every time:

- **Playback** = modulate: bytes → FSK edges on PA0, in real time, cycle
  by cycle, exactly as a real tape deck's read head would present them to
  the SALT chip.
- **Recording** = demodulate: DAC writes → edges → measured tone periods
  → bits → byte-aligned blocks, exactly as the SALT chip + ROM software
  would have to do it from a real tape.

That's the "software modem pointed at a software modem" framing from
§12.1, now concrete: this file *is* a modem, in both directions, because
the storage format demands it be one. A `.wav`-backed design could get
away with only ever demodulating (decode once at load, if it even bothers
storing anything other than raw samples); `.cas` cannot cut that corner.

---

## 12.4 The deck's state

Before walking playback and recording individually, look at what the
`Cassette` struct actually holds — it's small, and every field earns its
place:

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

(Doc comments trimmed here — you've already read the annotated original
above the fold in `cassette.rs`; they're worth a second pass once you've
finished this chapter, because several of them presuppose facts §12.6 and
§12.10 haven't given you yet.)

Notice the shape: `clock`, `pos`, `bit`, `bit_elapsed`, `spinup_left`,
`motor_was_on` are **playback** state — where the deck's read head
currently is, in both tape-position and sub-bit-timing terms. `last_level`
and `capture` are **recording** state — what the deck is hearing from the
DAC right now, and everything it's heard so far this session. `tape` is
the mounted medium itself, and `mounted`/`dirty` are bookkeeping. One
struct, both directions, because it's one physical deck: at any moment
it's either playing or being recorded onto, never both, but it doesn't
know in advance which you'll ask it to do next.

> **Rust corner: resetting a struct by rebuilding it.** Look at
> `insert_tape` and `eject_tape`:
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
> Neither function hand-resets ten fields one at a time. `*self = Self {
> ..Self::default() }` (Rust's *struct update syntax*) builds a brand-new
> value from `Default`, overrides the two fields that matter, and
> overwrites the whole struct through the `&mut self` reference in one
> assignment. This isn't just shorter — it's *safer* against the exact bug
> class this file's own doc comments worry about elsewhere (stale
> `pos`/`bit`/`spinup_left` surviving a swap): there is no way to add an
> eleventh field to `Cassette` later and forget to reset it here, because
> nothing here names fields to reset. `Default` does that job once, in one
> place, and every reset site inherits the fix for free. When you see
> `*self = Self { field: value, ..Self::default() }` in this codebase,
> read it as "everything not named here goes back to power-on," which is
> a stronger and more maintainable guarantee than a hand-written list of
> assignments could offer.

---

## 12.5 Playback: bytes become edges

Every CPU unit (once per instruction, from `Machine::step_cpu_unit`), the
machine ticks the cassette forward:

```rust
// crates/coco-core/src/machine/run.rs:118-119
self.bus.cart.tick(cycles);
self.bus.cassette.tick(cycles, self.bus.pia1.a.c2_output());
```

`pia1.a.c2_output()` is the motor relay line — more on that in §12.6.
`tick` is the whole playback engine:

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

Walk it in order:

1. **Motor edge detection.** `motor_on && !self.motor_was_on` catches the
   off→on transition and arms the spin-up countdown — the tape doesn't
   move an inch until that latency drains (§12.6).
2. **Motor off → nothing moves, full stop.** No clock advance, no bit
   progress. This is also why the recorder's `record_dac` (below) resets
   `last_level` to `None` on motor-off: motor-off is a hard boundary
   between "sessions" on both sides of the deck.
3. **Cycle budget accumulates into `bit_elapsed`,** and a `while` loop —
   not an `if` — drains it against `current_bit_period()`. It has to be a
   `while`: `tick` is called once per *instruction*, and some instructions
   cost more cycles than a single bit period (434 cycles is well within a
   handful of the CPU's slower instructions, especially in the
   double-speed poke case week 6 introduced), so more than one bit could
   legitimately complete within a single `tick` call. An `if` here would
   silently drop bits under enough cycle pressure.
4. **Bit and byte advance.** `bit` counts 0–7 and wraps into `pos`
   incrementing — LSB first, matching `current_bit_is_one`'s `tape[pos] >>
   bit & 1`.

`current_bit_is_one`/`current_bit_period` are the tiny functions that ask
"what does the tape's *next* bit want, and how long does its tone burst
last":

```rust
// crates/coco-core/src/cassette.rs:277-283
fn current_bit_is_one(&self) -> bool {
    self.tape[self.pos] >> self.bit & 1 == 1
}

fn current_bit_period(&self) -> u32 {
    if self.current_bit_is_one() { ONE_BIT_PERIOD } else { ZERO_BIT_PERIOD }
}
```

And the line PA0 actually sees — the squared, SALT-filtered rendering of
whichever tone is currently playing:

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

Idle-high (motor off, spinning up, or past end of tape) is the default —
matching the CoCo's general rule that unused/idle input pins float high
(`PiaPort::default()`, week 10). Otherwise, this is a **square wave with
one flip per half-period**: for the first `high` cycles of the current
bit's tone (`ZERO_BIT_HIGH`=396 or `ONE_BIT_HIGH`=207), the line reads
low; for the remaining `LOW` cycles, it reads high.

Read the doc comment above `input_bit` closely, because it names a real
piece of reverse-engineering, not a design choice:

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

That's a fact nobody could derive from a datasheet: somewhere in the real
analog chain between the DAC writing a rising sine and the SALT chip's
output reaching PA0, the polarity flips. The only way to *know* that is to
try both and watch which one makes the real ROM's lock routine
(`CASON`, `$A77C`) actually succeed — which is exactly what happened, and
it's the same empirical methodology §12.8 tells the full story of. The
comment doesn't hedge, either: "verified empirically" is a specific claim
about having watched real ROM code behave differently under the two
choices, not a guess dressed up as a fact.

---

## 12.6 Recording: edges become bytes

Recording is playback's mirror image, and it's the harder half, because
now the emulator is the one that has to *discover* structure in a signal
instead of asserting it.

### Capturing the DAC

Every PIA1 write is checked for a change to the cassette-out DAC level:

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

`(output & ddr & 0xFC) >> 2` is the same 6-bit-DAC extraction the audio
chapter's mixer uses (PIA1 port A bits 2–7, masked to output-configured
pins only, shifted down to a `0..=63` value). The important word in that
comment is **unconditional**: unlike the speaker output, the cassette
tap doesn't check whether sound is enabled or which mux position is
selected. The DAC always feeds the tape line, whether or not you'd ever
hear it — which matches real hardware (the DAC's analog output is wired
straight to the record circuit; the sound-enable/mux logic only affects
the speaker path). `record_dac` itself just appends a change:

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

This is deliberately sparse storage — a `Transition { level, cycle }`
only on an actual *change*, not one entry per cycle. A program CSAVEing
for ten seconds might write the DAC thousands of times, but if you plot
only the changes, you get exactly the information a demodulator needs
(when did the signal cross which levels) without ever materializing a
dense sample array. This is the same "event-timestamped, not
per-sample" philosophy week 11 built the whole audio-event grid around —
recognize it here as the same idea solving the same kind of problem.

### From transitions to bit periods: the 624-cycle threshold

`finalize_recording` hands the whole capture to `demodulate`, which is a
two-stage pipeline: transitions → bits (`capture_to_bits`), bits → bytes
(`bits_to_bytes`).

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

The core idea: `mid = max / 2` is a zero-crossing threshold computed from
the capture's own observed levels — exactly what a real SALT comparator
does with an AC-coupled signal, no hardcoded assumption about what "high"
means. Every time the recorded level crosses from below `mid` to above it
(a **rising** crossing), that's one full tone cycle completed since the
*previous* rising crossing — one bit's worth of time, measured the same
way §12.2 told you a real cassette bit is defined: one full sine period.

The bit's *value* comes from comparing that measured period against
`BIT_PERIOD_THRESHOLD`:

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

Do the arithmetic yourself and you land on exactly **624**:
`(814 + 434) / 2 = 624`. A measured period at or below 624 cycles is
"fast," hence a 1-bit; above 624 is "slow," hence a 0-bit. `PERIOD_BREAK`
(1628 = 2 × 814) is a second, much looser threshold: a gap *that* long
isn't a slow 0-bit, it's not a tone cycle at all — the motor spinning up,
silence between blocks, or some other discontinuity — and the caller
should throw away whatever it was tracking and start hunting for a fresh
leader (§12.7 has a concrete example of exactly this kind of gap).

One more thing worth catching, because it's the sort of thing that only
shows up when you check the arithmetic instead of trusting the prose: the
doc comment above `BIT_PERIOD_THRESHOLD` says "midpoint of 455 and 793" —
not 434 and 814, the constants actually in force two lines below it. Both
pairs *do* average to the same 624 (455 + 793 = 1248; 434 + 814 = 1248,
so both midpoints land in the same place), but they're visibly not the
same measurement. This is very likely a fossil: an earlier, coarser pass
at calibration (you'll meet its numbers again in §12.8 — they match the
rough figures in `cassette_calibrate.rs`'s own doc comment almost
exactly) that got superseded by a more careful one without every comment
being updated to match. It's a small thing, and it doesn't change the
constant's actual value — but it's a genuine, checkable discrepancy in
this file today, and it's worth internalizing the habit that caught it:
when a comment states a number, do the arithmetic yourself rather than
taking the prose on faith. You'll use that habit again in exercise 12.2.

> **Rust corner: `Option<bool>` as an ad hoc three-state value.** Notice
> `capture_to_bits` returns `Vec<Option<bool>>`, not `Vec<bool>`. A bit
> demodulated from a real capture isn't always cleanly 0 or 1 — sometimes
> the gap between crossings is too long to be *any* tone
> (`PERIOD_BREAK`), and that's a third outcome the caller genuinely needs
> to distinguish from "definitely a 0" or "definitely a 1." Rather than
> invent a three-variant enum for this one call site, the code reaches
> for `Option<bool>`: `None` piggybacks on a type every Rust programmer
> already has intuition for, and the eventual consumer
> (`bits_to_bytes`, next) handles it with the same `let Some(bit) = bit
> else { ... }` pattern you'd use for any other optional value. Compare
> this against `BlockState` a few dozen lines away (§12.7): that *is* a
> purpose-built enum, because it has more than one piece of state to
> carry per variant (`Locked { seen, total }`) and more than two
> outcomes. The rule of thumb this file demonstrates: reach for `Option`
> when "absent/invalid" is genuinely the third state of an otherwise
> binary question; reach for a dedicated `enum` once a variant needs to
> carry its own data or you have more than a couple of cases. Using
> `Option<bool>` here instead of a 3-variant `enum Bit { Zero, One,
> Break }` isn't laziness — a bespoke enum would need its own `match`
> arms everywhere `Option`'s already-idiomatic combinators
> (`.then_some`, the `let-else` below) apply for free.

> **Rust corner: let-chains for the dangling last bit.** The salvage code
> at the end of `capture_to_bits`:
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
> combines a *pattern match* (`if let (Some(rise), Some(fall)) = ...`)
> with two ordinary boolean conditions using `&&`, all in one `if`. Older
> Rust required nesting: an outer `if let` whose body contained a second
> plain `if` for the extra conditions, adding a level of indentation for
> no semantic reason. **Let-chains** (stabilized as part of the Rust 2024
> edition this workspace targets — check `crates/coco-core/Cargo.toml`'s
> `edition = "2024"`) let you write the whole condition, pattern-matches
> and boolean tests together, as one flat `&&` chain. Read `if let PAT =
> EXPR && cond1 && cond2 { ... }` as "all of these must hold," exactly
> like a normal boolean `&&` chain, except some of the terms happen to be
> pattern matches that also bind names (`rise`, `fall`) the later terms
> and the body can use. When you see this shape in 2024-edition code,
> you're looking at what would have been two or three nested `if`s a few
> years ago.

Why does the recording end with a bit that needs "salvaging"? Because the
ROM turns the motor off immediately after writing the final byte — there's
no trailing rising edge to close out the very last tone cycle, since
nothing gets written after it. Without this fallback, the tape's last bit
(part of the EOF block's trailer, typically) would simply be missing, and
playback of that recording would leave the ROM's own `BITIN` routine
(`$A755`) polling forever for an edge that's never coming. The fix: if the
capture ends mid-cycle (a rise with no matching fall-then-rise after it,
but *with* a matching fall), estimate the missing half-period from the
half that *did* complete, by doubling it.

### Byte alignment: hunting the way the ROM hunts

A bit stream on its own has no byte boundaries — that's a fact about
serial data in general, and it's exactly the problem the CoCo's own
`CASON`/`GETBYT` ROM routines solve by scanning for a known pattern
first. `bits_to_bytes` re-implements that same strategy:

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
all is *how* each one consumes bits.

**`Hunt`** is a **sliding** window. Look carefully: `window_bits` only
gates the very first eight bits (`if window_bits < 8 { continue; }` skips
comparison until a full window exists) — but once it reaches 8, nothing
in the `Hunt` arm ever resets it back down except an actual `LEADER` or
`SYNC` match. That means every *subsequent* bit still shifts into
`window` and still gets compared, one bit later, against the same two
targets — the window slides one bit at a time rather than jumping eight
bits at a stretch. This is exactly what a receiver with no independent
clock has to do: it doesn't know where byte boundaries "should" fall
until it finds a byte value distinctive enough to bet on, so it checks
*every* possible alignment, one bit at a time, until one of them matches.

**`LEADER` ($55 — Service Manual §5.10) is that distinctive value.** As a
literal bit pattern, `0x55` is `01010101` — a strict alternation, and a
run of consecutive `$55` bytes on the wire is indistinguishable, bit for
bit, from one continuous alternating `...01010101...` stream with no byte
structure visible at all. That's the whole point: a leader isn't data,
it's clock recovery, giving the receiver dozens of bytes' worth of
"lock the phase in" opportunity before anything that actually needs to be
read correctly arrives. Once the hunt sees the block's **`SYNC` byte
($3C — `pub(crate)` in this module because `cassette_wav` needs it too,
§12.9)**, it knows *this* alignment is the real one, flushes the leader
run it counted (as plain `$55` bytes — the caller gets to see how long the
leader actually was) plus the sync byte itself, and switches to
`Locked`.

**`Locked { seen, total }`** is a small state machine of its own,
carrying exactly the two numbers it needs: how many block bytes have been
read so far, and how many it expects in total — which it doesn't know
until the *second* byte (the length field) arrives, hence `total:
usize::MAX` as a "not yet known" placeholder. `BLOCK_OVERHEAD = 4`
accounts for type + length + checksum + trailer around the
`length`-byte-sized payload — read `bits_to_bytes` and you can reconstruct
the whole block layout without needing an external spec at all:

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

Notice the last column: the trailer byte is a plain `$55`, structurally
identical to a leader byte, and `bits_to_bytes` doesn't need to special-
case it at all — once `Locked` reads it as byte number `total` and
returns to `Hunt`, that same `$55` (already consumed as data, already in
`out`) simply looks like the first byte of the *next* block's leader run
to whatever comes after it. The format doesn't need an explicit
"end of block" marker because the trailer *is* leader material by
construction, recycled.

A `None` bit — a `PERIOD_BREAK`-sized discontinuity, from `capture_to_bits`
— resets everything back to `Hunt` from wherever it was, mid-leader or
mid-block. That's a deliberate design choice worth naming: **a glitch only
costs the current block**, not the whole recording. Real tape behaves this
way too — a dropout corrupts the block it happens in, not everything after
it, because the next block's own leader run gives the reader a fresh
chance to re-synchronize.

---

## 12.7 Motor mechanics: modeling an assumption, not a chip

Everything in §12.5–12.6 assumed the tape was already rolling. It isn't,
the instant the motor relay closes — and this is the chapter's cleanest
example of a fact ch. 1 promised you'd meet all through this course:
sometimes what you have to emulate isn't a chip at all, it's an
*assumption* baked into the ROM's timing.

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

`65536 * 8 = 524288` cycles — do the conversion to seconds yourself
(524288 / 894886) and you get **≈ 0.586 s**, matching the comment's "~0.5
s" and the codebase's own test comment ("spin-up (~0.5 s = 524288
cycles)", `tests/cassette.rs:58`). It's not a coincidence that this is
also exactly `512 * 1024` — the ROM's own delay loop is `65536` iterations
of an 8-cycle inner body, and this constant is that loop transcribed
literally, cycle-accurate, not a rounded-off "about half a second."

Here's the part that makes it a *necessary* piece of emulation, not an
optional nicety: **the ROM doesn't check whether the tape is actually up
to speed.** It closes the relay, then blindly burns `LA7D1`'s countdown —
0.586 seconds of doing nothing else, on the theory that a real
electromechanical deck's motor and capstan need real physical time to
reach a stable read speed, and after that much time has passed, it's safe
to assume they have. That's an assumption about *physics*, encoded as a
fixed delay, with no feedback loop verifying it — the software equivalent
of "count to ten before you start listening" because you can't ask the
motor "are you up to speed yet."

Now consider what happens if the emulator's tape starts producing bits
the instant the motor bit goes high, with no spin-up delay at all. The
ROM is still blindly burning its 0.586-second countdown — it doesn't know
or care that the emulated tape is already "moving." But the tape *is*
moving in the emulator's model, feeding real FSK edges to PA0 the whole
time the ROM isn't listening yet. By the time the ROM's countdown expires
and `CASON`'s lock-on routine actually starts sampling PA0, the tape has
already played through the *entire* leader run that was there specifically
to give the lock-on routine something to synchronize against — the
comment's own phrase, "eats the 128-byte leader before it ever listens,"
is exactly this failure. `CLOAD` doesn't crash; it just times out or reads
garbage, because the emulator handed it a signal at the wrong moment
relative to when the ROM's own (unverified, blind) assumption said it was
safe to start reading.

Model the delay — hold the tape completely still, bits and all, for
those 524288 cycles after every motor-on edge, exactly as `tick` does
(§12.5, the `spinup_left` branch) — and the leader run is still sitting
there, untouched, right where the ROM expects it, the moment the ROM
actually starts listening. This is "emulate the assumption, not the chip"
in its purest form: there is no real motor object anywhere in this
codebase, no torque or capstan model, nothing with mass or inertia. There
is exactly one `u32` counter that holds still for the same number of
cycles a real motor's physics would have taken — because that's the only
observable consequence the ROM's blind trust in "physical time has
passed" produces, and it's the only consequence worth reproducing.

And the comment's last parenthetical matters practically, not just as
trivia: `CLOAD` doesn't spin the motor up once and leave it running.
It **cycles the motor off, then on again**, between reading the namefile
block and reading the data blocks (`LA701`/`LA4D0`). Every one of those
re-engagements re-arms the *same* 0.586-second spin-up latency — which is
exactly why `Cassette::tick`'s edge-detection (`motor_on &&
!self.motor_was_on`) has to fire on every off→on transition, not just the
very first one after mounting a tape.

---

## 12.8 The measurement story

This section is the chapter's best lesson, and it deserves to be told in
full, because it's the clearest example in this entire codebase of a
methodology you will need again: **when the documentation runs out,
instrument your own emulator and measure the software's actual
behavior.**

Here's the gap. Color BASIC's `CSAVE`/`CLOAD` bit-banging code lives in
the **$A000–$BFFF** region of the 32K ROM — the second, un-extended half
of Color BASIC. This project's local reference material (`./docs/`, a
git-ignored directory of copyrighted PDFs present only on the original
author's machine — this worktree doesn't have it, and I'm not claiming to
have read it) includes *Super Extended Color BASIC Unravelled II*, a
disassembly-with-commentary of the ROM. But per this module's own header
(quoted in full in §12.1), that reference **does not cover** the
$A000–$BFFF bit-bang routines. There is, in other words, no available
prose describing exactly what tones this ROM emits, at exactly what
timing, for exactly what reason. The only remaining source of truth is
the ROM's own bytes, executing.

So the answer was: run the real ROM, make it actually `CSAVE` something,
and watch. That's what `examples/cassette_calibrate.rs` is for:

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

Read that as what it is: this is *not* a unit test with an expected
answer baked in. It boots the actual, unmodified `roms/coco3.rom`, types
a real one-line BASIC program through the emulated keyboard exactly the
way §12.1's opening ritual describes doing it by hand, types `CSAVE"X"`,
and then does nothing but *record what the emulator's own DAC-tap
mechanism already captures* — the same `record_dac`/`capture()` machinery
walked in §12.6, repurposed here as a measuring instrument instead of a
CLOAD demodulator. The ROM has no idea it's being measured; it's just
executing its normal cassette-save routine against a `Cassette` that
happens to also be a stopwatch.

This worktree doesn't have `roms/coco3.rom` (`./roms/` is git-ignored,
present only on the machine this course was authored on), so I have not
run this example myself in this session and won't claim otherwise — but
its own source and the fact section below let you read the measurement
methodology in full even without the ROM in hand, and the constants it
produced are the ones baked into `cassette.rs` today, checkable by anyone
with a copy of the ROM.

The probe does two distinct kinds of analysis, and the difference between
them is the whole lesson:

**First pass: raw transition deltas.** It builds a histogram of the time
between *every consecutive DAC write*, no matter what it represents:

```rust
// crates/coco-core/examples/cassette_calibrate.rs:86-97 (excerpt)
let mut hist: BTreeMap<u64, u32> = BTreeMap::new();
for i in 1..cap.len() {
    let delta = cap[i].cycle - cap[i - 1].cycle;
    *hist.entry(delta).or_insert(0) += 1;
}
```

**Second pass: zero-crossing deltas.** It separately computes the
midpoint of the observed DAC levels, finds every place the signal crosses
it, and histograms the time *between crossings* — the same computation
`capture_to_bits` performs for real, in production:

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

These two analyses do **not** agree, and understanding why is the actual
payoff of the whole exercise. The ROM's DAC writes a stepped approximation
of a sine wave — not one write per tone, several writes as the software
sine table steps the 6-bit DAC through intermediate levels on the way up
and down. A raw-transition histogram counts *every one of those steps* as
a "delta," so its dominant deltas are the spacing between individual
DAC-level steps, not full tone periods — noisier, and only a *rough*
proxy for the number you actually want. The file's own top-of-module
comment, written from that first pass, says exactly this:

```rust
// crates/coco-core/examples/cassette_calibrate.rs:14-17
//! - Each bit is one full DAC sine cycle: a 0-bit measures ~793 CPU cycles
//!   (~1128 Hz), a 1-bit ~455 cycles (~1967 Hz) — close to, but not exactly,
//!   the canonical 1200/2400 Hz (a hand-tuned ROM delay loop, not a crystal-
//!   locked tone; the ROM's own hysteresis demodulator tolerates the drift).
```

**793 and 455 are not the constants in `cassette.rs` today (814 and 434).**
That's not an error in this chapter — it's the module comment's own
honestly-reported first-pass numbers, preserved rather than silently
corrected, and it's genuinely useful to leave visible: it shows the
*naive* measurement (raw DAC-write deltas) landing close to, but visibly
off from, the *refined* measurement (crossing-to-crossing deltas, "modal
midpoint-crossing spacings," which is the phrase the constants' own doc
comment uses). The refined number is the one that survived into the
actual constants because it's the one that matches what a zero-crossing
comparator — what the real SALT chip does, and what `capture_to_bits`
does — would measure, not what a naive "count every DAC write" pass
would.

There's a small, satisfying piece of corroboration buried in the
arithmetic, and it's worth checking yourself rather than taking on faith:
`793 + 455 = 1248`, and `814 + 434 = 1248` — the *same* sum. Two different
measurement techniques, on two different (though related) definitions of
"how long is this bit," independently landed on the same decision
boundary (`1248 / 2 = 624`, exactly `BIT_PERIOD_THRESHOLD`) even while
disagreeing on the two periods individually by about 21 cycles apiece, in
opposite directions. That's the kind of agreement that makes you trust a
measurement — not because the two passes were identical, but because two
different lenses on the same underlying signal converged on the same
answer for the number that actually matters for demodulation.

The methodological lesson generalizes far past cassette tape, and it's
worth stating on its own: **when a datasheet or disassembly doesn't cover
the behavior you need, make your own emulator into an instrument.** You
already have, by the time you reach this chapter, a CPU that executes the
real ROM correctly (weeks 2–4), a bus that routes every access correctly
(week 5), and I/O devices honest enough that the ROM can't tell it isn't
talking to real hardware (weeks 10–11). That combination — not a
disassembler, not a datasheet, not another emulator's source code — is
what let this project measure a fact that genuinely exists nowhere else
in writing.

---

## 12.9 WAV round-tripping

`cassette.rs` owns the CPU-facing FSK domain; `cassette_wav.rs` owns
translating that domain to and from real audio, for two reasons stated in
its own header: feeding a `.cas` recording to an actual cassette deck (or
any tool that only understands audio), and accepting a WAV recorded from
an actual physical tape back into the `.cas` convention.

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

That last line matters as a code-organization point, not just trivia:
`ZERO_BIT_PERIOD`, `ONE_BIT_PERIOD`, and `SYNC` are `pub(crate)` in
`cassette.rs` specifically so this module can `use` the exact same
measured values instead of re-typing `814`/`434`/`0x3C` as a second set of
magic numbers that could drift out of sync with the originals over time.
One source of truth, two consumers.

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

This is where the *actual sine* from §12.2's "why square waves are
enough" digression gets synthesized — because now the consumer isn't the
emulated CPU (which only ever needs a squared edge), it's potentially a
real deck's line input, or a human looking at a waveform in Audacity
(exercise 12.7), both of which want the real analog shape the ROM's DAC
would have produced. Note the doc comment's explicit statement that this
*isn't* inverted the way `Cassette::input_bit`'s PA0 rendering is —
`synthesize_wav` reproduces the DAC's own un-inverted sine, because it's
modeling the record-side output jack, not the SALT-filtered playback
input. Same underlying tone, two different points in the analog signal
chain, two different polarities — and the code keeps that straight by
never sharing one function between the two.

`push_silence` adds a full second of dead air before the tone (a real
deck's motor and AGC need time to physically spin up and settle — an
engineering convention borrowed from how other tools, including MAME's
own `.cas`-to-audio loader, do it, and stated explicitly as *not* a
hardware timing fact in the constant's own doc comment) and a quarter
second after. Neither number came from measuring the CoCo; they're
"what makes a real deck happy," a different and looser kind of fidelity
than everything else in this chapter.

### Import: decoding uncertain audio

Decoding a WAV back into tape bytes has to handle uncertainty that a
synthetic capture never has: unknown polarity (which side of the signal
is "high" depends on how the recording was made, not anything the file
format tells you) and unknown noise near the zero crossing. Both are
handled honestly rather than assumed away:

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

Rather than guess which polarity is correct, `decode_wav` just tries
**both** — runs the *entire* crossing-detection-plus-demodulate pipeline
twice, once as read and once phase-inverted — and keeps whichever result
actually contains a `SYNC` byte (§12.6's `bits_to_bytes` would only
produce one from real block structure, not noise):

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

This mirrors, at the audio boundary, the exact same "hunt, don't assume"
philosophy §12.6 walked inside `bits_to_bytes` — when you can't be
certain of alignment (there, byte alignment; here, signal polarity), the
answer isn't to pick one and hope, it's to try every plausible
interpretation and let a strong structural signal (a `SYNC` byte
appearing where it should) tell you which one was right. There's also a
hysteresis band (`WAV_HYSTERESIS_FRACTION`, a tunable heuristic explicitly
flagged in its own doc comment as *not* a hardware fact) around the
midpoint, because real recordings genuinely do have noise that would
otherwise register as spurious crossings right at the zero line — a
problem the fully synthetic playback path in `cassette.rs` never has to
solve, because its "crossings" are computed, not measured.

The tests in §12.10 confirm this round-trips through both 8-bit and
16-bit PCM, and survives a full polarity inversion — worth reading now
that you understand what machinery is being exercised.

---

## 12.10 Reading the tests

`crates/coco-core/tests/cassette.rs` splits cleanly into two halves: tests
that need nothing but the `Cassette`/`cassette_wav` API (no ROM, always
run), and one end-to-end test against the real `roms/coco3.rom`. Running
the file confirms exactly that split:

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

That's the actual output of running it, in this worktree, right now — not
a hypothetical. Notice: `csave_rewind_cload_round_trips_a_basic_program`
still shows `... ok`. It isn't skipped by `#[ignore]`; it's a normal test
that runs, checks for the ROM at `../../roms/coco3.rom`, prints a message
to `stderr` when it's absent, and returns early — a pattern you've seen
before in this course (`tests/coco1_boot.rs`, week 6) and will see again
in disks and serial (weeks 13–14). This worktree has no `roms/`
directory, so that's the only test in the file that can't fully exercise
itself here; every other test needs nothing more than the code you've
already read.

### `playback_waveform_demodulates_back_to_the_same_bytes`

This is the tightest, most important test in the file: the whole
modulation pipeline must be its own inverse, with no ROM involved at all.

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

Three details worth pulling out:

1. **`TICK_CYCLES = 7`, "deliberately not a divisor of either bit
   period"** — its own comment says so. Both periods (814, 434) are even;
   7 is odd and shares no useful factor with either. Ticking in a
   non-aligned stride means every bit boundary falls at a slightly
   different offset within some 7-cycle step, so a bug in how `tick`
   handles a partial bit at the end of its cycle budget (the `while`
   loop's job, §12.5) would show up as compounding phase error rather
   than being accidentally masked by always landing exactly on a
   boundary. This is a genuinely good testing habit: pick a granularity
   that stresses the *general* case, not the convenient one.
2. **`deck.tick(600_000, true)` in one call, before the loop**, burns
   through the entire ~524288-cycle spin-up latency (§12.6) in a single
   jump — and the assertion right after, `deck.position().0 == 0`,
   directly checks the "tape must hold still" claim from §12.7, not just
   assuming it.
3. **The level mapping, `if deck.input_bit() { 0 } else { 63 }`, is
   inverted relative to what you might expect**, and the comment explains
   exactly why: `input_bit()` returns PA0's value, which is the
   SALT-inverted rendering (§12.5); `demodulate` consumes DAC-domain
   transitions (§12.6), which are *not* inverted. So the test has to
   translate between the two domains itself — PA0 low maps to DAC-domain
   "high" (`63`) — which is a small, easy-to-miss detail that only makes
   sense once you've read both `input_bit`'s doc comment and
   `record_dac`'s calling convention. If you ever add a new test that
   feeds `input_bit()`'s output into `demodulate`, this inversion is the
   first thing to get right.

### `motor_off_freezes_the_tape_and_records_nothing`

A short, sharp confirmation of §12.5/§12.6's motor-gating claims, worth
reading for how little code it takes once the API is right:

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

Both halves of the deck — playback and recording — are checked against
the same motor-off condition, independently, in the same test. Ten
thousand cycles of ticking with the motor off moves the tape exactly
nowhere; two DAC writes with the motor off produce exactly zero captured
transitions. Simple, but it's checking a real invariant: the motor line
gates *everything*, not just one side of the deck.

### `csave_rewind_cload_round_trips_a_basic_program`

The full end-to-end test, against a real booted ROM, walking the entire
ritual from §12.1 in code: mount a blank tape, type a program, `CSAVE"X"`,
wait for the motor to go idle, rewind (which finalizes the recording —
`Cassette::rewind` calls `finalize_recording` first, so `CSAVE` →
`Rewind` → `CLOAD` works without an explicit eject cycle), check the
block structure, `NEW` to wipe BASIC's program, `CLOAD` it back, `RUN`
it, and check the screen for the program's actual output. This is the
test whose block-structure assertions opened this chapter in §12.1 — you
now have every piece of machinery needed to read the rest of it, top to
bottom, without help.

It needs `roms/coco3.rom`, which this worktree doesn't have, so — as
confirmed by actually running it above — it currently exercises only its
own early-return path here. `crates/coco-core/tests/coco2_boot/cassette.rs`
runs the near-identical scenario against a CoCo 2 boot (`extbas11.rom` +
`bas12.rom`) instead, specifically as a regression guard that the same
`Cassette`/PIA1 wiring works correctly on the plain-SAM bus path (§5, week
5) and not just the GIME path — its own header says so directly:

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

I confirmed this test's actual behavior in this worktree too — same
outcome, for the same reason:

```
$ cargo test -p coco-core --test coco2_boot -- cassette
running 1 test
skipping coco2_boot: extbas11.rom/bas12.rom not present in roms/ (see docs/coco12-plan.md "ROM files")
test cassette::coco2_csave_rewind_cload_round_trips_a_basic_program ... ok
```

Both end-to-end tests are honest about their dependency and skip cleanly
rather than failing or silently passing without exercising anything real
— worth noting as a pattern for your own device tests once you get to
building your own (week 13's disk tests do the same thing, for the same
reason).

---

## 12.11 Reading assignment

In this order:

1. **`crates/coco-core/src/cassette.rs`, the whole file (~451 lines).**
   Read the module header first, then the constants block (lines 1–66),
   then `Cassette::tick`/`input_bit` (playback, §12.5), then
   `record_dac`/`demodulate`/`capture_to_bits`/`bits_to_bytes`
   (recording, §12.6). By now every doc comment in this file should read
   as a claim you can verify, not a fact to take on faith.
2. **`crates/coco-core/src/cassette_wav.rs`** — skim the constants and
   `synthesize_wav` closely, `decode_wav`'s chunk-parsing machinery more
   lightly (it's ordinary defensive file-format parsing; the interesting
   part is the polarity-guessing in §12.9).
3. **`crates/coco-core/examples/cassette_calibrate.rs`** — read it as a
   measurement instrument, not application code. Notice what it captures
   (raw DAC transitions) versus what it computes from that capture (two
   different histograms), and connect that back to §12.8.
4. **`crates/coco-core/tests/cassette.rs`** — you've now read every test
   in it in §12.10; re-read `playback_waveform_demodulates_back_to_the_same_bytes`
   once more end to end without stopping, and confirm you can predict
   what `demodulate(&capture)` returns before you reach the assertion.

Run the ROM-free suite and confirm you see the same eight tests this
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

**12.2 — Threshold sensitivity (verify, then reason).** I actually ran
this one rather than just reasoning about it, and you should reproduce
it: temporarily change `BIT_PERIOD_THRESHOLD`'s definition in
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
in that range, and why isn't it?; (b) once you push far enough to break
something, why does it break as an *all-or-nothing* flip (every 1-bit at
once) rather than "which bit pattern fails first" — and what would have
to change about how the test's tape is synthesized for the question "which
pattern fails first" to even have an answer? (Hint: compare how
`ZERO_BIT_HIGH`/`ZERO_BIT_LOW` encode *measured* asymmetry within one bit
against how the test's synthetic capture generates bits with zero
per-instance jitter.) Revert your edit back to the real formula
(`(ZERO_BIT_PERIOD as u64 + ONE_BIT_PERIOD as u64) / 2`) and confirm
`cargo test -p coco-core --test cassette` is clean and `git status` is
clean before moving on.

**12.3 — Sabotage the bit order, and watch leader survive (sabotage,
verified).** In `bits_to_bytes`, change
`window = window >> 1 | u8::from(bit) << 7;` to
`window = window << 1 | u8::from(bit);` — assembling each byte MSB-first
instead of LSB-first. Run `cargo test -p coco-core --test cassette`
before you predict the outcome. When I ran this: **3 of 8 tests fail**
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
as your guide. This exercise depends on hardware this worktree doesn't
have (`roms/`), so treat it as a lab you run on a machine that does, not
one you can complete here — but it's the single most direct way to
confirm everything this chapter told you about the signal actually looks
like that on a real waveform.

---

## What's next

Part IV is done: PIAs (week 10), the audio path they feed (week 11), and
now the one PIA pin, plus a DAC, that turns into a complete two-way modem
under nothing but ROM software (week 12). Notice the throughline — every
device in this part turned out to be "a PIA pin plus interpretation,"
which is the real reason this machine could be built as cheaply as it
was.

Part V trades that pattern for its opposite. Week 13 introduces the
WD1773 floppy controller — a real, dedicated chip, with its own command
state machine, that the CPU talks to instead of bit-banging. You'll meet
a **HALT/NMI handshake** that week 6 quietly set up for
(`step_cpu_unit`'s HALT check, which you've now read the exact call site
of, right next to the two cassette/cartridge `tick` calls in
`machine/run.rs`), and you'll see the same "functional, not cycle-exact"
fidelity choice this chapter's tape made — but for a completely different
reason: not because you can't derive the exact behavior, but because
disk software, unlike the cassette ROM you just spent a whole chapter
matching cycle-for-cycle, doesn't count cycles at all.
