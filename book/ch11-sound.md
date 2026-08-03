# Chapter 11 — Sound: from a 6-bit DAC to your speakers

*Week 11. Goal: follow sound from a write to `$FF20` to samples consumed by
the host audio device. The path must translate irregular CPU-timed events
into a steady stream, filter it, and resample it to the device's chosen
rate. No digital signal processing background is assumed.*

---

Every previous week in this course has had a natural stopping point at
the crate boundary. The CPU chapters lived inside `mc6809`. The bus, the
GIME, the raster, and the PIAs all lived inside `coco-core`, and Chapter 15
will finally open `coco-egui` properly. Audio refuses to respect that
boundary. A byte written to `$FF20` has to survive a journey that starts
in a PIA's output latch and passes through a cycle timestamp, a scanline
flush, a mixing function, a per-frame drain, two filters, a gain stage,
a resampler, a mutex, and a second operating-system thread before
anything moves a speaker cone. Every layer in that list exists because
of a specific defect, and none of them can be skipped without producing
a specific, audible symptom.

That is what makes this the most "systems" chapter in the course, and it
is also what makes the finished pipeline a good case study in something
this book keeps returning to: each stage turns one honest representation
into another, and each is named after the problem it solves rather than
after the technology it uses.

The chapter is organized around that chain. Sections 11.1 through 11.4
stay inside `coco-core` and answer a single question: how do you turn
"a program wrote a byte at some arbitrary CPU cycle" into "a stream of
numbers at a fixed rate"? Sections 11.5 through 11.10 cross into
`coco-egui` and answer a different one: how do you get that stream out
of the emulator, at a rate the host's sound card actually wants, without
clicks, buzzes, aliasing artifacts, unbounded memory growth, or a
missed real-time deadline? Section 11.11 steps sideways to look at the
three optional sound chips this codebase models, because seeing them
next to each other is the fastest way to understand what a "sound chip"
even meant across the early 1980s. Sections 11.12 onward are the usual
lab work.

A word about the vocabulary, since this book assumes no graphics or DSP
background. Terms like *aliasing*, *Nyquist rate*, *decimation*,
*low-pass*, and *DC offset* turn up in this chapter, and every one of
them is introduced here from the defect it names rather than from a
definition. If a section starts by describing something that sounds
wrong — a click, a buzz, a fake tone that was never in the original
signal — that description *is* the motivation for whatever filter comes
next. Read the defect first; the formula afterwards will look like the
obvious response to it.

One last framing note before the hardware. Nothing in this chapter
requires a sound card. Every claim it makes about the core's half of the
pipeline is checked by tests that run headless, and every claim about
the frontend's DSP chain is checked by unit tests that process arrays of
`f32` with no audio device open anywhere. The entire chapter can be
verified on a machine with the speakers unplugged, which is a good
property for a subsystem whose bugs are otherwise diagnosed by ear.

---

## 11.1 The hardware behind the POKEs

There is no sound chip in a stock CoCo. Say that once, plainly, because
it explains everything that follows: the "SOUND" and "PLAY" commands in
Extended Color BASIC, and every machine-language music routine ever typed
in from a magazine listing, worked by wiggling a handful of bits on a
**general-purpose parallel port** fast enough, and in the right pattern,
to approximate a waveform. The chip doing the wiggling is PIA1 — the same
MC6821 Chapter 10 introduced as "keyboard, joystick, cassette, printer,
DAC." This week is the "DAC" part.

That absence is worth dwelling on for a moment, because it sets the
whole chapter's difficulty level. Machines that shipped with a dedicated
sound chip give an emulator author a clean target: model the chip's
registers, run its oscillators, ask it for a sample. The chip's own
design decides what "a sample" means and how often one exists. A machine
with no sound chip gives you none of that. What it gives you instead is
a pin, a voltage, and a program that changes the voltage whenever it
feels like it — and the entire engineering problem of this chapter is
that "whenever it feels like it" has to be reconciled with a sound card
that wants a number every twenty microseconds, forever, on a schedule
neither the CoCo nor the emulator controls.

Three signals matter. The first two are PIA1's (base `$FF20`, spanning
`$FF20–$FF23` — the table from Chapter 1); the third borrows two more pins
from PIA0 next door. Take them one at a time.

### Six pins and a resistor ladder

The first and most important of the three is the **6-bit DAC** on port A
pins PA2 through PA7. Those six output pins feed a resistor network whose
job is to convert a six-bit binary
number into a single analog voltage. The trick is arithmetic done in
copper: each of the six wires contributes current through resistors
sized so that its contribution is exactly half the contribution of the
wire above it. The resulting voltage on the output node represents
the binary number the six pins are carrying, scaled to some voltage
range. There is no clock in a resistor ladder, no register, and nothing
to configure. It is a purely combinational lump of passive components
whose output tracks its inputs continuously.

That last property matters more than it might sound. Set the six pins to
a value and the ladder's output *holds* that voltage, unchanging, until
the pins change. Nothing decays, nothing refreshes, nothing needs
servicing. The emulator will need a word for that behavior, and the word
is *latch*: a value that persists exactly as written until overwritten.
Almost every source in this chapter's mixer is a latch, and §11.2's
central design decision falls straight out of that fact.

`POKE 65312,n` (`$FF20 = 65312` decimal) with the high six bits of `n`
set is, quite literally, setting an analog voltage by hand. BASIC's
`SOUND` and `PLAY` statements call ROM routines that ramp this value up
and down at audio rates; digitized-speech programs, which unpack
compressed sample data and write it to `$FF20` in a tight loop, push it
as fast as the CPU can manage. Either way, from the hardware's point of
view it's the same thing: a stream of bytes landing on six output pins,
each byte held until the next one arrives.

### One bit, no ladder

The second signal is **PB1, a single-bit "beeper."** One pin, on or off,
with no ladder behind it — just a switch between two voltage levels.
Toggle it at an audio frequency and the result is a square wave, and a
square wave from a single pin is the harsh, buzzy tone every CoCo game's
"you lost a life" sound used. It caught on because it is the cheapest
possible way to make a noise: one bit, one store instruction, no D/A
conversion at all, and no need to keep six pins coordinated.

There is one structural fact about PB1 that will come back twice in this
chapter, so it's worth planting now. The single-bit output is wired
straight to the speaker path with no gate. Unlike the DAC, which sits
behind a multiplexer that can cut it off entirely, PB1 is *always*
connected. Keep that filed under "wait, why is this always on?" —
§11.3's mixing function makes it visible as a literal difference in
indentation.

### Four inputs, one speaker: the analog mux

The third signal is not one signal but three, and they exist to solve a
routing problem. The 6-bit DAC, the cassette input, and cartridge audio
all want to reach one physical speaker or line-out jack, and only one of
them should be audible at a time. (The cassette's own record path is a
separate concern and belongs to Chapter 12's SAVE-side story.) The
component that arbitrates is a **4-to-1 analog multiplexer**: four
inputs, one output, and two address bits that select which input gets
connected, plus a master enable that can disconnect everything.

The clever part is where the three control signals live. Tandy did not
add a control register for the mux, and did not decode a new address for
it. All three signals came from pins the two PIAs already had spare:

- PIA1 CB2 is **SNDEN**, the mux's master gate.
- PIA0 CA2 and CB2 are **SEL1** and **SEL2**, the two address bits that
  choose the mux's input.
- PIA1 CA2 is the cassette motor relay, which additionally gates the
  cassette's own input to the mux.

No new chip, no new address range. Four spare output pins on hardware
that already existed for other jobs, wired to an inexpensive
multiplexer IC. This is the same "reuse what's already on the bus"
instinct Chapter 1 described when the GIME turned out to answer to the dead
SAM's addresses: cheap hardware reusing cheap hardware, with the cost
paid in documentation confusion rather than in parts.

### The table this codebase encodes

Here is the exact table the emulator implements
([`crates/coco-core/src/bus/audio_bridge.rs:15-28`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/audio_bridge.rs#L15-L28),
[`crates/coco-core/src/audio.rs:82-114`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/audio.rs#L82-L114) — both quoted in full in §11.3):

| SNDEN (PIA1 CB2) | SEL2:SEL1 (PIA0 CB2:CA2) | Mux routes... |
|---|---|---|
| 0 | (any) | nothing — mux output silent |
| 1 | `00` | the 6-bit DAC (PIA1 PA2–7) |
| 1 | `01` | the cassette input, gated by the motor relay |
| 1 | `10` | cartridge audio (the SSC's AY-3-8913, §11.11) |
| 1 | `11` | grounded — silent |

Four rows of behavior from three control bits, and one row that does
nothing at all. The `11` position being grounded rather than made into a
fourth useful source is the kind of detail that looks like waste until
you remember that a 4-to-1 mux has four positions whether or not anyone
has a use for the fourth, and grounding an unused input is cheaper than
leaving it floating.

And underneath all of that, unconditionally: the single-bit output on
PB1 sums in regardless of what the mux is doing. That's why the beeper
could interrupt a `PLAY` statement's music with a sound effect on some
titles. It was never behind the mux gate at all, so nothing a program
did to SNDEN or the select bits could silence it.

The provenance of this table is worth noting, because it is the kind of
claim an emulator gets wrong quietly. The doc comment on
`snapshot_audio_inputs` cites its sources by name: the Tandy Service
Manual's mux table by way of MAME's `coco.cpp` `update_sound`, plus
Super Extended BASIC Unravelled II's treatment of `$FF22`/`$FF23`. Two
independent sources for a table nobody can check by inspection, because
a wrong row here produces "sound sometimes doesn't play" rather than a
crash.

### Two pins, two jobs

One more piece of wiring deserves attention, because it connects this
week directly to last week and it will change how you read the mux
select. PIA0's CA2 and CB2 are the sound mux's SEL1 and SEL2. They are
*also* the select lines for the joystick's potentiometer multiplexer,
which Chapter 10 walked through in detail
([`crates/coco-core/src/joystick.rs:1-10`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/joystick.rs#L1-L10)):

```rust
//! CoCo analog joysticks (`DESIGN.md` §7).
//!
//! There is no joystick register: software ramps the 6-bit DAC (PIA1 PA2–PA7,
//! $FF20) and reads the comparator on PIA0 PA7 ($FF00 bit 7), which is high
//! while the DAC level is at or below the selected pot. The pot is chosen by
//! an analog mux driven by PIA0's CA2 (SEL1: 0 = X, 1 = Y) and CB2 (SEL2:
//! 0 = right stick, 1 = left stick). Fire buttons sit on the keyboard row
//! lines PA0–PA3 and pull them low regardless of the column strobe.
//! (Verified: SEB Unravelled II Appendix A $FF00/$FF01/$FF03; MAME `coco.cpp`
//! `poll_keyboard`/`joyin` — PA7 = `dac_output() <= joyval`.)
```

Read that next to the mux table above and the overlap is total: the same
two pins, read through the same `c2_output()` accessor, choose which
potentiometer reaches the joystick comparator *and* which source reaches
the speaker. In this codebase both consumers call that one accessor, so
they cannot disagree.

The consequence is a genuinely entertaining thought experiment, and one
you can trace all the way through the code in §11.3. A joystick-reading
routine works by *sweeping the 6-bit DAC* while watching the comparator
bit, which means that for the whole duration of the sweep the DAC is
producing a rising staircase of voltages. If SNDEN happens to be on and
the axis being polled happens to be the one that leaves SEL at `00`,
that staircase goes straight to the speaker: the mixer has no way to
distinguish "the DAC is being driven to make a sound" from "the DAC is
being driven to measure a potentiometer," because on this hardware they
are the same act. Reading a joystick and playing a note are the same
six pins doing the same thing for different reasons.

### What this means for the emulator

One point is worth internalizing before you look at a single line of
Rust: **every one of those signals is a PIA output pin**, exactly the
kind of "byte to an address" write Chapter 5's bus chapter and Chapter 10's
PIA chapter covered. There is no dedicated audio hardware to model here
beyond "some PIA writes are audio-affecting, and the emulator needs to
notice."

That is worth contrasting with what the original design document
expected. `DESIGN.md` §7's audio bullet, written before any of this
existed, reads in full: "**Audio** — accumulate the 6-bit DAC by cycle,
downsample to 48 kHz, feed `cpal`. Defer until video+CPU work; just
leave the sink interface." Every noun in that sentence survived. The
verb did not: nothing in the shipped pipeline "accumulates by cycle,"
and the downsampling target turned out to be "whatever the device asks
for" rather than a hardcoded 48 kHz. Chapter 1's reading assignment made a
point of praising `DESIGN.md` for keeping its corrections attached
rather than tidying them away; this is one of the places where the first
guess and the built thing differ, and the next two sections are the
argument for why.

The whole interesting engineering problem in this chapter is what
happens to those writes *after* PIA1 records them, which is exactly
where the chapter goes next.

---

## 11.2 The problem: the CPU writes at arbitrary cycles, the speaker wants a steady stream

A modern audio device — the one `cpal` opens on the host machine right
now — doesn't accept "here's a value, hold it until further notice." It
wants a **fixed-rate stream** of samples: 44,100 or 48,000 numbers per
second, delivered on a schedule, forever, whether or not anything
interesting happened in between. Real analog electronics don't have this
constraint. A voltage just *is* whatever the DAC ladder is outputting at
this instant, continuously, no sampling involved. The gap between
"continuous voltage" and "discrete stream at a fixed rate" is the entire
subject of digital audio, and it is where every decision in this chapter
comes from.

Since the audience for this book is assumed to have no DSP background,
it's worth being precise about what a *sample* is, because the word gets
used loosely enough that it can hide the problem. A sample is the value
of a signal at one instant, and a sampled stream is a list of such
values taken at evenly spaced instants. The stream carries no
information about what happened *between* two samples. If a voltage rose
and fell entirely inside one sampling interval, the stream simply does
not contain that event, and no amount of later processing can recover
it. A sampled representation is lossy by construction, and the only
question is whether what it loses is anything a listener could have
heard.

The CPU, meanwhile, writes to `$FF20` whenever the *program* wants to,
not on any fixed schedule the audio device would recognize. A `PLAY`
statement's ROM routine might update the DAC once every few hundred
cycles to shape a tone. A hand-written digitized-speech player might
slam a new byte into `$FF20` every dozen or so cycles, faster than
almost anything else the machine does. Neither one is synchronized to a
sound card that didn't exist yet. The emulator has to bridge "CPU writes
whenever" to "device wants exactly N samples per second," and it has to
do it without losing what the software actually did.

The module documentation at the top of
[`crates/coco-core/src/audio.rs:1-10`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/audio.rs#L1-L10)
opens by naming the version of this problem the codebase already had and
fixed, which is the most useful possible framing for what follows:

```rust
//! Event-timestamped stereo audio pipeline (`docs/plan-audio-pipeline.md`).
//!
//! The old path point-sampled one mono level per scanline (~15.7 kHz), which
//! aliased software-timed DAC playback (digitized speech writes `$FF20` far
//! faster than the line rate) and quantized every level change to a line
//! boundary. This pipeline instead records a [`AudioEvent`] snapshot at the
//! CPU-cycle timestamp of every audio-affecting write, then renders each
//! scanline to a fixed [`OVERSAMPLE`]-slot stereo grid at the line's end:
//! between events the hardware holds its level (latches), so grid rendering
//! is exact reconstruction up to grid resolution, not interpolation.
```

There is a whole engineering story compressed into that paragraph, and
the rest of this section unpacks it by walking the two obvious answers
first and letting each one fail audibly.

### Naive answer one: sample once per field

The first idea most people have is to reuse the video clock. The
emulator already runs a field sixty times a second and has a natural
place to do per-field work, so read whatever's in `$FF20` when you're
about to hand a frame to the video renderer and call that "the audio for
this frame."

This is *catastrophically* coarse, and it fails on two separate counts.
The first is a matter of representable frequency: a stream sampled 60
times a second can only carry content up to 30 Hz, which is below the
lowest string on a bass guitar and down at the bottom edge of where
human hearing registers pitch at all. Nothing musical survives. The
second is worse and more specific. A program that writes the DAC a
hundred times between frames, which is exactly what digitized speech
does, has its entire waveform collapsed to one number per sixtieth of a
second. Ninety-nine of those hundred writes are discarded, and the one
that survives is whichever happened to be latched at the instant the
sampler looked. The result is silence, or a dull thud, never speech.

### Naive answer two: sample once per instruction, or once per bus cycle

The opposite extreme has an obvious appeal: sample as fast as the
machine can possibly change. Read `$FF20` after every single CPU cycle
and you'd capture everything, in principle.

In practice this fails in three ways at once. At 0.895 MHz it means
895,000 samples per second of bookkeeping, which is nearly twenty times
the rate any downstream consumer wants, so every consumer would then
have to decimate back down. Most of that work is wasted on values that
never changed, since the DAC commonly holds one level for hundreds of
cycles between writes; a latch that isn't being written is not producing
information. And the sample rate would be tied to the *CPU* clock rather
than to anything the audio device knows about, so the speed poke
(`POKE 65495`, Chapter 6's material) would silently double the audio rate
mid-program.

The deepest objection is a matter of shape rather than of cost. What
you actually want isn't "a sample of what the DAC held at cycle N." It's
"know exactly when it changed, and to what." The write *is* the event.
A level that happens to get read is a derived, lossy view of that event,
and building the whole pipeline on the derived view means paying for
resolution you don't need while still quantizing away the timing you do.

### The third representation: an event log

This codebase's actual answer, in one sentence, is: **record every
audio-affecting write as a cycle-timestamped event when it happens, and
reconstruct a fixed-rate sample grid from those events once per
scanline.**

That's not "sample more often." It is a change of representation, from
"levels sampled at some rate" to "an event log, rendered to a grid on
demand," and the change is what makes the whole thing cheap *and*
accurate at the same time. The event log costs nothing when nothing is
happening, because silence produces no events. It costs a few bytes per
meaningful write when something is happening. And because the hardware
between events is a latch that holds its value exactly, replaying the
log onto any grid is not interpolation or estimation. It is exact
reconstruction of a piecewise-constant signal, limited only by how
finely the grid divides time.

That "limited only by the grid" caveat is the one honest loss in the
scheme, and the module doc names it in the same breath: grid rendering
is exact "up to grid resolution." A level change lands in the slot whose
start it precedes, not at the exact cycle it happened. Section 11.3
walks the code that makes this concrete, and shows exactly how coarse
that quantization is in practice.

---

## 11.3 The core's answer: events in, an oversampled grid out

Three pieces cooperate, and reading them in the right order matters
because each one only makes sense given the previous. First, a small
struct that snapshots "everything about the current sound-affecting
state" (`AudioInputs`). Second, a bus-side hook that notices when that
snapshot changes and timestamps it (`note_audio_write`). Third, a
per-scanline renderer that replays those timestamped events into a
fixed-size grid of samples (`flush_line_audio`). Between the second and
third sits `mix`, the function that turns one snapshot into one stereo
sample: §11.1's mux table written as code.

### `AudioInputs`: everything the mux could be looking at, right now

The struct is small enough to read in one breath, and every field in it
is a signal §11.1 already named. From
[`crates/coco-core/src/audio.rs:50-68`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/audio.rs#L50-L68):

```rust
/// The latched audio-affecting inputs, snapshotted on every write that
/// changes one of them (see `SystemBus::note_audio_write`).
#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub(crate) struct AudioInputs {
    /// PIA1 port A bits 2–7: the 6-bit DAC (already masked by DDR, shifted).
    pub dac: u8,
    /// PIA1 PB1 (masked by DDR): the single-bit beeper.
    pub single_bit: bool,
    /// PIA1 CB2: SNDEN, the analog mux's master gate.
    pub snden: bool,
    /// PIA0 CB2:CA2 — the mux select (SEL2:SEL1).
    pub sel: u8,
    /// PIA1 CA2: the cassette motor relay (gates the mux's cassette input).
    pub cassette_relay: bool,
    /// Latched cartridge left/right outputs (`Cartridge::sound_levels`) —
    /// the Orchestra-90's DAC pair, wire-summed by the MPI.
    pub cart_left: f32,
    pub cart_right: f32,
}
```

This is §11.1's table turned into a struct: every bit or byte that
governs what the mux does, in one value cheap enough to snapshot on
every write. Seven small fields, all `Copy`, no allocation and no
indirection anywhere in it.

What's *not* in it is as informative as what is. The cassette's own
audio level isn't here, because it's sampled once per line rather than
event-timestamped. Its 1200/2400 Hz tone is far slower than the line
rate, so per-cycle timing buys nothing; Chapter 12 takes that up properly.
Nothing generator-driven is here either. The SSC's AY-3-8913 and the
GMC's SN76489A are sampled at flush time rather than latched,
because they are continuously running oscillators rather than values a
write sets and holds. Section 11.11 returns to that distinction; for now
the rule is simply that `AudioInputs` holds *latches*, and latches are
the things that change only when written.

The derive list on line 52 is doing quiet work that the next function
depends on completely. `Copy` is what makes "snapshot the whole state"
a register-width move rather than a clone. `PartialEq` is what makes
"did anything change?" a single expression. And `Serialize`/
`Deserialize` put the latched audio state into save-state snapshots
alongside everything else, which Chapter 16 will care about and which is
only possible because Chapter 1 refused shared ownership.

> **Rust corner: `#[derive(PartialEq)]` as a change detector.** Deriving
> `PartialEq` on a plain data struct generates a field-by-field
> comparison, and the derived implementation is exactly as good as a
> hand-written one for a struct of `u8`s, `bool`s, and `f32`s. What's
> worth noticing is the *design* move: because the audio state is one
> `Copy` struct rather than seven fields scattered across two PIAs, the
> question "did this write affect audio?" collapses into `inputs !=
> self.audio_inputs`. There is no list of "which addresses are
> audio-relevant" to maintain, no per-register hook to remember to add,
> and no way for a newly added audio-affecting field to be forgotten by
> the change detector: add a field to `AudioInputs` and the derived
> comparison covers it automatically. Deriving equality on a snapshot
> type is a cheap and durable substitute for hand-maintained dirty
> flags, and the pattern generalizes well past audio.

### The snapshot function and the write hook

The snapshot is built straight from live PIA state, with no cached
intermediate anywhere
([`crates/coco-core/src/bus/audio_bridge.rs:15-28`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/audio_bridge.rs#L15-L28)):

```rust
fn snapshot_audio_inputs(&self) -> crate::audio::AudioInputs {
    /// PIA1 PB1: the single-bit sound output.
    const SINGLE_BIT: u8 = 0x02;
    let (cart_left, cart_right) = self.cart.sound_levels();
    crate::audio::AudioInputs {
        dac: (self.pia1.a.output & self.pia1.a.ddr & 0xFC) >> 2,
        single_bit: self.pia1.b.output & self.pia1.b.ddr & SINGLE_BIT != 0,
        snden: self.pia1.b.c2_output(),
        sel: u8::from(self.pia0.b.c2_output()) << 1 | u8::from(self.pia0.a.c2_output()),
        cassette_relay: self.pia1.a.c2_output(),
        cart_left,
        cart_right,
    }
}
```

Two details are worth pausing on. The first is the DAC expression,
`self.pia1.a.output & self.pia1.a.ddr & 0xFC`. Note that the value is
masked by the **data direction register**, not just by the output
register. This is Chapter 10's DDR lesson cashing in directly. A PIA pin
that the running program never configured as an output isn't driving
anything, so whatever bit sits in the output register for that pin is
not what the outside world sees. Letting it through would leak
program-irrelevant garbage into the mix. The DDR mask is what keeps
un-configured pins out. `0xFC` then keeps only bits 2 through 7, the
DAC's six wires, and `>> 2` slides them down into a 0–63 value.
`DAC_MAX` in [`audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/audio.rs) is exactly `63.0`, which is the top of that range and
not a coincidence.

The second is the `sel` expression, which reconstructs a two-bit number
from two unrelated PIA pins: `u8::from(self.pia0.b.c2_output()) << 1 |
u8::from(self.pia0.a.c2_output())`. CB2 becomes bit 1, CA2 becomes bit
0, and the result indexes §11.1's table directly. This is the same
`c2_output()` accessor the joystick code reads, which is what makes the
"two pins, two jobs" overlap from §11.1 structurally true rather than
merely coincidental.

Now the hook that turns "the mux state changed" into a **timestamped
event** ([`bus/audio_bridge.rs:48-57`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/audio_bridge.rs#L48-L57)):

```rust
pub(super) fn note_audio_write(&mut self) {
    let inputs = self.snapshot_audio_inputs();
    if inputs != self.audio_inputs {
        self.audio_inputs = inputs;
        self.audio_events.push(crate::audio::AudioEvent {
            cycle: self.cycle_clock,
            inputs,
        });
    }
}
```

This runs on every PIA and cartridge-window write, and the function's
doc comment says so explicitly, along with the reason it's affordable
there: "cheap even there: one snapshot + compare per write." Follow the
cost. A write that doesn't affect audio, which is nearly all of them,
costs one struct construction and one derived comparison, both of which
the optimizer sees straight through. No allocation, no push, no growth.
Only a write that genuinely alters the mux's inputs — a new DAC value, a
mux-select flip, SNDEN toggling — records an `AudioEvent`.

The event itself is two fields, and its doc comment fixes the semantics
precisely ([`crates/coco-core/src/audio.rs:70-76`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/audio.rs#L70-L76)):

```rust
/// One latched-input change at a CPU-cycle timestamp (`SystemBus::cycle_clock`).
/// `inputs` is the state FROM this cycle onward.
#[derive(Serialize, Deserialize)]
pub(crate) struct AudioEvent {
    pub cycle: u64,
    pub inputs: AudioInputs,
}
```

"The state FROM this cycle onward" is the sentence that makes the replay
in `flush_line_audio` correct. An event isn't a sample of a moment; it's
the beginning of an interval that runs until the next event. The
timestamp is `self.cycle_clock`, the same free-running cycle counter
Chapter 6 built the entire scanline loop around, incremented in
`step_cpu_unit` after every CPU unit
([`crates/coco-core/src/machine/run.rs:121`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs#L121)).
Audio does not get its own clock. It reads the machine's.

This is §11.2's representation change made concrete. Instead of a level
sampled at some fixed rate, there is now a sparse log of exactly when
the hardware state changed and to what, at full cycle resolution, for
the cost of a few bytes per meaningful write. Most CoCo audio, digitized
speech very much included, changes the DAC far less often than once per
cycle, so the log stays small even under the heaviest load the machine
can produce.

### `mix`: one grid slot's worth of physics, as a function

Given an `AudioInputs` snapshot plus the cassette bit and any generator
samples — the sources this section doesn't event-timestamp — `mix`
computes one stereo sample. This is the mux table from §11.1 as code,
and the resemblance is close enough to read the table off it
([`crates/coco-core/src/audio.rs:82-114`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/audio.rs#L82-L114)):

```rust
pub(crate) fn mix(inputs: &AudioInputs, cassette_bit: bool, ay: f32, generators: (f32, f32)) -> [f32; 2] {
    let mut l = 0.0f32;
    let mut r = 0.0f32;
    if inputs.snden {
        match inputs.sel {
            0 => {
                let v = DAC_GAIN * f32::from(inputs.dac) / DAC_MAX;
                l += v;
                r += v;
            }
            SEL_CASSETTE => {
                if inputs.cassette_relay && cassette_bit {
                    l += CASSETTE_GAIN;
                    r += CASSETTE_GAIN;
                }
            }
            SEL_CARTRIDGE => {
                l += CARTRIDGE_GAIN * ay;
                r += CARTRIDGE_GAIN * ay;
            }
            _ => {} // 11: grounded
        }
    }
    if inputs.single_bit {
        l += SINGLE_BIT_GAIN;
        r += SINGLE_BIT_GAIN;
    }
    l += CART_GAIN * inputs.cart_left;
    r += CART_GAIN * inputs.cart_right;
    l += CART_GAIN * generators.0;
    r += CART_GAIN * generators.1;
    [l, r]
}
```

Read the shape before the arithmetic. `if inputs.snden { match
inputs.sel { ... } }` is §11.1's table verbatim: SNDEN gates the whole
mux, and SEL picks the branch inside it. The `_ => {}` arm with its
one-word comment is the `11` row, present because the hardware has four
mux positions whether or not the fourth is useful.

Now notice what sits *outside* that gate, at the same indentation as the
`if` rather than inside it. `single_bit` is summed unconditionally,
which is §11.1's "wait, why is this always on?" turned into a literal
indentation difference you can see at a glance. The cartridge's latched
stereo pair and its generator pair are unconditional too, and for a
different reason: the Orchestra-90 and the GMC/SSC cartridges drive
their own RCA jacks rather than the CoCo's internal SND pin, so the
machine's mux has no authority over them at all. Section 11.11 shows a
test that proves exactly this bypass.

The gain constants are the one part of this function that is *not* a
hardware fact, and the file is honest about that. `DAC_GAIN = 0.75`,
`SINGLE_BIT_GAIN = 0.25`, `CASSETTE_GAIN = 0.35`, and the rest are all
named at the top of [`audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/audio.rs) with a comment apiece explaining the
reasoning. They encode relative loudness calibration: somebody had to
decide that the DAC and the beeper shouldn't compete at equal volume,
and the file records that decision per constant rather than leaving
magic numbers for a future reader to reverse-engineer. `CASSETTE_GAIN`'s
comment is a good example of the tone these comments are written in,
explaining that tape playback is "kept below the DAC's full scale like
the real attenuated level."

`CARTRIDGE_GAIN` deserves a note of its own, because two similarly named
constants sit next to each other. `CARTRIDGE_GAIN` applies to
*mux-routed* cartridge audio, the `SEL = 10` branch. `CART_GAIN` applies
to the always-summed latched and generator paths further down. Two
different routes for "cartridge sound," tuned independently, and reading
them as one constant would make the mixer look redundant when it isn't.

There's one more entry point worth knowing about before moving on,
because most of §11.12's tests use it rather than the real pipeline
([`crates/coco-core/src/bus/audio_bridge.rs:30-42`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/audio_bridge.rs#L30-L42)):

```rust
    /// Mix one stereo sample from the CURRENT latched inputs plus one
    /// `dt`-second generator step — the instantaneous speaker level, for
    /// tests and level meters. The machine's real audio path renders the
    /// event-timestamped grid instead (`Machine::flush_line_audio`); this
    /// probe advances the generator clocks (AY drain, PSG crystals) as a
    /// side effect exactly like one grid slot does.
    pub fn sound_probe(&mut self, dt: f64) -> [f32; 2] {
        let inputs = self.snapshot_audio_inputs();
        let cassette_bit = self.cassette.playing() && self.cassette.input_bit();
        let ay = self.cart.audio_sample();
        let generators = self.cart.generator_sample(dt);
        crate::audio::mix(&inputs, cassette_bit, ay, generators)
    }
```

`sound_probe` answers the question "what would the speaker be doing
right now?" without running a scanline, which makes it exactly the right
instrument for testing the mux table and exactly the wrong one for
testing timing. That division of labor is what §11.12 is about.

### `flush_line_audio`: replaying events into a grid, once per scanline

This is where the timestamped log becomes a fixed-rate stream, and it is
the single most important function in the chapter. It is called once per
scanline from the per-line trailer Chapter 6 walked through
([`crates/coco-core/src/machine/run.rs:147`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs#L147), inside `end_of_line`),
and it renders **`OVERSAMPLE` grid slots** — four, in this codebase —
per line. From
[`crates/coco-core/src/machine/audio.rs:41-68`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/audio.rs#L41-L68):

```rust
    pub(super) fn flush_line_audio(&mut self) {
        let line_start = self.audio_line_start;
        let line_end = self.bus.cycle_clock;
        self.audio_line_start = line_end;
        // A HALT-free line spans `line_budget` cycles; keep the real span so
        // event timestamps land in the right slot even on odd lines.
        let span = line_end.saturating_sub(line_start).max(1);
        let slot_dt = 1.0 / self.audio_sample_rate();
        let cassette_bit = self.bus.cassette.playing() && self.bus.cassette.input_bit();

        let events = std::mem::take(&mut self.bus.audio_events);
        let mut inputs = self.audio_line_inputs;
        let mut cursor = 0;
        for k in 0..u64::from(audio::OVERSAMPLE) {
            let slot_start = line_start + span * k / u64::from(audio::OVERSAMPLE);
            while cursor < events.len() && events[cursor].cycle <= slot_start {
                inputs = events[cursor].inputs;
                cursor += 1;
            }
            let ay = self.bus.cart.audio_sample();
            let generators = self.bus.cart.generator_sample(slot_dt);
            self.audio_buffer
                .push(audio::mix(&inputs, cassette_bit, ay, generators));
        }
        // Events in the final slot's tail take effect from the next line's
        // first slot: the bus's current state is the next line's start state.
        self.audio_line_inputs = self.bus.audio_inputs;
    }
```

Walk it in three passes: the setup, the loop, and the handoff.

The setup establishes the window. `line_start` and `line_end` bracket
the scanline that just finished, in cycle-clock terms, using exactly the
same `cycle_clock` counter that `note_audio_write` stamped events
against. Storing `line_end` back into `self.audio_line_start`
immediately means the next call picks up precisely where this one
stopped, with no gap and no overlap. `span` is the line's *real* cycle
width, and the comment above it explains why that matters: a line
stretched by the FD-502's halt and DRQ handshake, which is Chapter 13's
subject, still divides evenly into four slots of its actual width rather
than of some nominal width. Event timestamps therefore land in the slot
they belong to even on an odd line. The `.max(1)` guards against a
zero-length span, which would otherwise make the slot arithmetic
degenerate.

The loop is where the reconstruction happens. For each of the
`OVERSAMPLE` slots in order, `slot_start` computes the slot's left edge
in cycles, and the inner `while` pulls in every event whose timestamp is
at or before that edge, updating `inputs` each time. This is the key
move, and it is worth stating flatly: **events are not distributed
across slots by count, they are placed by their actual cycle
timestamp**. A slot picks up whatever the latest event at or before its
start left the hardware in, and if three events happened inside one
slot, all three are consumed and the last one wins. Between events the
hardware holds its level, because that's what a latch does, so this
isn't interpolation or guessing. It is exact reconstruction of a
piecewise-constant signal, accurate to one grid slot.

Generators are handled differently, and the difference is the
latch-versus-oscillator distinction from earlier. `audio_sample()` and
`generator_sample(slot_dt)` are called fresh on every slot, not
event-driven, because a free-running oscillator has to be asked "what
have you produced since the last request?" regardless of whether any PIA
write happened at all. Note that `generator_sample` takes `slot_dt`, a
*duration in seconds*, computed from `audio_sample_rate()` at the top of
the function. Section 11.11 comes back to why that argument is in
seconds rather than in cycles; the short version is in the trait's own
doc comment and involves the speed poke.

`mix` then folds the latched inputs and the generator samples into one
stereo sample, which is pushed onto `self.audio_buffer`. Four slots,
four pushes, once per scanline, forever.

The handoff is the last line, and it is easy to skim past.
`self.audio_line_inputs = self.bus.audio_inputs` carries the *current
bus state* forward as the next line's starting point. Note that it does
not carry `inputs`, the loop's local cursor variable, which may lag
behind if an event landed after the last slot's `slot_start`. Any event
in the tail of this line that didn't quite make the final slot's cutoff
still updated `self.bus.audio_inputs` when `note_audio_write` recorded
it, so the next line starts from the up-to-the-moment truth. Nothing is
lost across the line boundary; a late event is merely deferred to the
first slot of the next line, which is exactly the "quantized to grid
resolution" behavior the module doc promises.

### A worked line

Abstract descriptions of slot arithmetic are hard to hold onto, so put
real numbers through the machinery. Chapter 6 established that an NTSC line
at normal speed has a budget of 56 cycles. With `OVERSAMPLE = 4`, the
slot edges computed by `line_start + span * k / 4` fall at offsets 0,
14, 28, and 42 cycles into the line. (The integer division is doing real
work here: with a 57-cycle span the edges land at 0, 14, 28, and 42 as
well, since `57 * 1 / 4 = 14` and `57 * 3 / 4 = 42` in integer
arithmetic. The grid is robust to a cycle of slop in the line length.)

Now suppose a program writes the DAC to full scale about 30 cycles into
that line, which is precisely what
`dac_write_mid_line_splits_the_grid_slots` in
[`crates/coco-core/tests/audio_grid.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/audio_grid.rs) arranges. Follow the loop slot by
slot. Slot 0's start is offset 0, and the event at offset 30 is not at
or before it, so slot 0 renders the line's inherited starting state:
silence. Slot 1's start is 14, still before the event: silence. Slot 2's
start is 28, and 30 is *after* 28, so the event still hasn't been
consumed: silence again. Slot 3's start is 42, the event's timestamp of
30 is at or before it, so the event is consumed and `inputs` becomes the
full-scale DAC state. Slot 3 renders loud.

Three silent slots, one loud slot, exactly one transition, and the
transition placed at the first slot boundary at or after the write. That
is precisely what the test asserts: `grid[0]` equals silence, the last
slot's left channel exceeds 0.5, and the count of adjacent unequal pairs
is exactly one. It also shows the quantization honestly. The write
happened at cycle 30; the grid says it happened at cycle 42. The error
is at most one slot, about 14 cycles, which is roughly sixteen
microseconds. That is the resolution `OVERSAMPLE` buys, and it is what
the doc comment at the top of [`audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/audio.rs) means by "grid resolution...
not interpolation."

Exercise 11.4 asks what happens to this arithmetic when `OVERSAMPLE`
becomes 2. Work the slot edges out on paper before running it.

### `take_audio` and the self-capping buffer

Samples accumulate in `audio_buffer` line after line, and something has
to drain them. The frontend does, once per UI update
([`crates/coco-core/src/machine/audio.rs:12-14`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/audio.rs#L12-L14)):

```rust
    pub fn take_audio(&mut self) -> std::vec::Drain<'_, [f32; 2]> {
        self.audio_buffer.drain(..)
    }
```

`Drain` hands ownership of the buffered samples to the caller and
empties `audio_buffer` in the same call, with no copy and no leftover
state to reconcile. The caller gets an iterator; the machine gets an
empty `Vec` with its capacity intact, ready to refill.

But what if nothing ever calls `take_audio`? Headless tests, trace
tooling, and the PPM lab bench from Chapter 1 all run fields with no sound
sink attached, and every one of those would otherwise grow this `Vec`
forever. The core guards against exactly that
([`crates/coco-core/src/machine.rs:38`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine.rs#L38)):

```rust
const AUDIO_BUFFER_CAP: usize = 8 * 262 * crate::audio::OVERSAMPLE as usize;
```

The check runs in `end_of_line`, immediately before `flush_line_audio`
([`run.rs:144-147`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs#L144-L147)):

```rust
        if self.audio_buffer.len() >= super::AUDIO_BUFFER_CAP {
            self.audio_buffer.clear();
        }
        self.flush_line_audio();
```

Eight fields' worth of grid samples, which works out to 8 × 262 × 4 =
8,384 frames, is the ceiling. Cross it and the buffer is cleared rather
than grown. The choice of `clear` over "stop producing" is deliberate
and matches the physical intuition: a real CoCo's speaker doesn't care
whether a human is in the room, so audio is produced whether or not
anyone's listening, and the buffer only has to survive being ignored
indefinitely without becoming a leak. This is the same "derived scratch,
self-bounding" philosophy Chapter 1 saw applied to the framebuffer, where
the answer to "what if nobody looks at this?" was likewise "then it
doesn't need to be kept."

> **Rust corner: `std::mem::take`.** `let events =
> std::mem::take(&mut self.bus.audio_events);` swaps `audio_events` for
> its `Default` (an empty `Vec`) and hands you the old value, in one
> move, with no cloning and no `unsafe`. It's the idiomatic way to say
> "consume this field's current contents and leave something valid
> behind" when there's no natural "drain and refill" API (like
> `Vec::drain` above) to reach for — a pattern you'll see again in week
> 15's frame loop. The alternative, `std::mem::replace(&mut x,
> Vec::new())`, does the same thing with more noise to type; `take`
> exists purely because "replace with the default" is common
> enough to deserve its own name.
>
> Notice that it is also doing two jobs in one line here, and the second
> one is a correctness requirement rather than a convenience. The events
> this line consumed must not be replayed on the next line, so
> `audio_events` has to end up empty before the next scanline starts
> accumulating into it. `take` hands over the list and empties the field
> in the same operation, which means there is no way to write the "read
> the events" half without also writing the "reset for next line" half.
> A separate `clear()` call afterwards would do the same thing and would
> be one refactor away from being forgotten.

---

## 11.4 Deriving the grid rate honestly

Chapter 6 established a habit: when a comment gives a round number,
recompute it from the actual constants and see if it agrees. Time to
apply that habit here, because [`audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/audio.rs)'s own doc comment invites it:

```rust
/// Grid samples per scanline. 4 → ~62.9 kHz internal rate on NTSC; a named
/// constant per the plan — bump to 8 only if a digitized-speech title
/// measurably needs it.
pub const OVERSAMPLE: u32 = 4;
```

Two things in that comment are worth reading closely before doing the
arithmetic. The first is "a named constant per the plan," which is the
codebase's standing objection to magic numbers: the number 4 appears in
the slot loop, in the buffer cap, and in two test files, and it appears
as `OVERSAMPLE` in every one of them. The second is the upgrade path.
"Bump to 8 only if a digitized-speech title measurably needs it" is a
fidelity-budget decision in exactly Chapter 1's sense: the rung above is
identified, the cost of climbing it is one constant, and the trigger for
climbing it is evidence rather than taste.

Now the arithmetic. The rate is computed rather than declared, per
[`crates/coco-core/src/machine/audio.rs:16-24`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/audio.rs#L16-L24):

```rust
    /// The audio sample rate matching [`Machine::take_audio`]'s stream: the
    /// oversampled grid rate, [`audio::OVERSAMPLE`] × the scanline rate.
    pub fn audio_sample_rate(&self) -> f64 {
        self.line_rate() * f64::from(audio::OVERSAMPLE)
    }

    /// Scanlines per second (~15.7 kHz NTSC) — the audio grid's line clock.
    fn line_rate(&self) -> f64 {
        self.config.video.lines_per_field() as f64 * self.config.video.field_rate_hz()
    }
```

`line_rate` is lines-per-field times fields-per-second, which is the
scanline frequency, the same quantity every NTSC-era engineer called
"the horizontal rate." Plug in the constants Chapter 6 already established
for `VideoStandard::NTSC`, namely `lines_per_field() = 262` and
`field_rate_hz() = 59.94`:

```
line_rate  =  262 × 59.94  =  15,704.28 Hz
grid_rate  =  15,704.28 × 4  =  62,817.12 Hz
```

**62,817.12 Hz in this timing model, not 62,900.** This is a gentler version of Chapter 6's "56,
not 57" lesson. It is not a truncation bug this time, since both factors
here are `f64` and `audio_sample_rate` never rounds. It is just a
comment, plus a handful of test constants — `sound.rs`'s `PROBE_DT = 1.0
/ 62_866.0` and `audio_test.rs`'s `62_866.0` — that rounded to a
convenient nearby number instead of carrying the exact product.

Compare the two "the code disagrees with a round number in a comment"
moments and notice the difference in kind. Chapter 6's 56-versus-57 came
from floor division compounding, an artifact of how the arithmetic was
written, and it changed what the emulator actually did. This one is
imprecise rounding in prose and test fixtures, with the real computed
value sitting one function call away the whole time, and it changes
nothing the emulator does. Both are worth catching, but only one of them
would have sent you hunting for a bug that wasn't there. The lesson
survives either way: **run the numbers yourself before you trust a
comment's "~".**

For a sanity check, compare against the horizontal rate every NTSC
reference quotes: 15,734 Hz, from the broadcast standard's 63.5 µs line
period. This codebase's `line_rate()` of 15,704.28 Hz is close but not
identical, because it is built from `field_rate_hz() = 59.94` rather
than NTSC's exact 30000/1001, which is approximately 59.940060 frames
per second. That is a rounding one level further up, and it follows the
same "six significant figures is usually enough" convention you'll find
all through [`config.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/config.rs).

None of this affects correctness inside the emulator, and the reason is
worth stating because it is a design property rather than luck.
`audio_sample_rate()` and `take_audio()` always agree with each other,
because both derive from the same `line_rate()` call on the same config.
The frontend never assumes a rate; it asks for one on every frame and
uses whatever it gets, as §11.8's resampler shows. So the resampler is
always fed the true rate its input actually arrived at, whatever that
rate's relationship to the canonical 15,734 Hz textbook number happens
to be. A rate that is *self-consistent* is worth more here than a rate
that matches a reference book, because self-consistency is what keeps
the resampler's step ratio honest.

One more property of this rate is worth appreciating before moving on:
62,817 Hz is a deeply strange number to hand a sound card. Nothing in
the consumer audio world runs at 62.8 kHz. It is not a multiple of
44,100, it is not a multiple of 48,000, and it is not close to either.
That awkwardness is not an accident or an oversight. It is what happens
when a sample rate is derived from *video* timing, which is where every
timing constant in this machine ultimately comes from (Chapter 1's
crystal). The frontend's entire DSP chain exists to reconcile that
video-derived rate with an audio-derived one, and §11.7 and §11.8 are
the two halves of that reconciliation.

---

## 11.5 Crossing the thread boundary

Everything so far has lived entirely inside `coco-core`, single-threaded,
with no synchronization needed anywhere. A `Machine` is just a struct
that one thread calls methods on, which is the direct payoff of Chapter 1's
"plain owned tree" decision. That changes the moment audio has to reach
a speaker.

Compare the two output paths and the asymmetry is stark. Video in this
codebase gets uploaded to a GPU texture once per UI repaint, which is
Chapter 15's subject: the UI thread both produces the framebuffer and
consumes it, so no handoff is required and no lock exists. Audio cannot
work that way, because the operating system's audio API calls your code
back **on its own thread, on its own schedule**, expecting samples to
already be waiting. `cpal`, the cross-platform audio library this
frontend uses, opens a device and runs a closure you supply whenever the
OS wants more frames. That might be a professional-grade low-latency
driver calling back every few milliseconds, or it might be whatever the
host's default device happens to do. You do not control when that
callback fires, and it must never be kept waiting.

So [`coco-egui/src/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs) has a producer and a consumer running on two
different threads, connected by exactly one piece of shared state. Here
is the struct that owns both ends
([`crates/coco-egui/src/audio.rs:158-181`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs#L158-L181)):

```rust
pub struct AudioOutput {
    /// `None` when no output device/config/stream could be opened — every
    /// other method then degrades to a no-op instead of touching cpal.
    stream: Option<cpal::Stream>,
    /// Stereo frames at the device's rate, shared with the stream's callback
    /// thread: `push_samples` (producer) pushes resampled frames; the cpal
    /// callback (consumer) pops one per output frame and maps L/R onto the
    /// device's channels.
    ring: Arc<Mutex<VecDeque<[f32; 2]>>>,
    /// Bound on `ring`'s length, in frames (`RING_BUFFER_SECS` of device rate).
    ring_cap: usize,
    /// The open device's output rate, or 0.0 when disabled.
    device_rate: f64,
    muted: bool,
    volume: f32,
    dc: [DCBlocker; 2],
    /// Anti-alias low-pass per channel, designed lazily for the source rate
    /// seen on the first `push_samples` call (`None` until then, or when
    /// upsampling makes it unnecessary).
    lowpass: Option<[LowPass; 2]>,
    /// The source rate `lowpass` was designed for — redesign on change.
    lowpass_rate: f64,
    resampler: Resampler,
}
```

Most of those fields are the DSP chain, and §11.6 through §11.8 take
them one at a time. The field that matters *right now* is `ring:
Arc<Mutex<VecDeque<[f32; 2]>>>`, which is the seam.

The **UI thread** calls `push_samples` once per `update()`, roughly
sixty times a second. It drains `Machine::take_audio()`, runs the whole
DSP chain the rest of this chapter covers, and pushes the result into
`ring`. The **cpal callback thread**, running independently and
potentially far more often, locks `ring` and pops frames off the front,
one per output frame the device asked for. Nothing else coordinates the
two threads: no channel, no condition variable, no "wait until ready."
The queue either has data or it doesn't, and §11.9 and §11.10 cover what
happens in each case.

The module's own header states this arrangement and then says something
worth pausing on
([`crates/coco-egui/src/audio.rs:6-10`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs#L6-L10)):

```rust
//! The device runs on its own high-priority thread and pulls frames out of a
//! `Mutex<VecDeque<[f32; 2]>>` that `push_samples` (called once per `update()`
//! on the UI thread) fills. There is no synchronisation beyond that mutex —
//! audio and video are independently paced, exactly like a real CoCo's TV and
//! speaker.
```

"Exactly like a real CoCo's TV and speaker" is not a throwaway line. On
the real machine, the television's scan and the speaker's cone are
driven by the same crystal but by entirely separate circuits, and
neither waits for the other. The emulator reproduces that relationship
structurally rather than by accident: video is paced by the field loop,
audio is paced by the sound card, and the only thing they share is a
queue that one fills and the other empties.

Here is the producer's call site, which is a single line in the
frontend's per-frame loop
([`crates/coco-egui/src/app/frame.rs:50-51`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/frame.rs#L50-L51)):

```rust
            let sample_rate = self.machine.audio_sample_rate();
            self.audio.push_samples(self.machine.take_audio(), sample_rate);
```

Two things about those two lines repay attention. First, the rate is
fetched fresh every frame rather than cached at startup, which is what
lets a machine swap between NTSC and PAL configurations without anything
downstream going stale. Second, and less obviously, both lines sit
*inside* the `if self.running` branch. When the emulator is paused, by
the user or by a debugger breakpoint, `push_samples` is not called at
all. Nothing new enters the ring, the callback thread keeps draining it
at the device's rate, and within a fraction of a second the queue is
empty. What the user hears at that moment is the subject of §11.10, and
it is not silence-by-accident — it is a designed fade.

> **Rust corner: `Arc`, not `Rc`.** Chapter 1 established that the core
> crate uses *no* `Rc<RefCell<…>>` anywhere, ever — the whole machine is
> a plain owned tree, borrow-checked at compile time. `coco-egui`'s audio
> module is the first place in this codebase that needs *shared ownership*
> across two threads at once, and the type that buys that is `Arc`
> (atomic reference count), never `Rc` (plain, non-atomic reference
> count). The difference is one word — atomic — and it's load-bearing:
> `Rc`'s internal counter increments and decrements with ordinary,
> non-atomic reads and writes, which are only safe if a single thread
> ever touches them. Rust's type system enforces this at compile time:
> `Rc<T>` doesn't implement `Send`, so the compiler refuses to let you
> move one across a thread boundary — try to hand an
> `Rc<Mutex<VecDeque<...>>>` to `cpal`'s callback and you get a compile
> error, not a runtime data race.
> `Arc<T>` costs a little more per clone (an atomic increment instead of
> a plain one) in exchange for that `Send`/`Sync` guarantee. The rule of
> thumb this codebase follows: `Rc` when everything stays on one thread
> (never — the core avoids shared ownership entirely), `Arc` the instant
> two threads need the same allocation, which is exactly and only this
> one ring buffer.
>
> Note where the second handle is made:
> `Self::try_build_stream(Arc::clone(&ring))`. The clone happens once, at
> stream-open time, and the cloned handle is moved into the callback
> closure and lives as long as the stream does. There is no per-frame
> reference counting on the hot path at all.

> **Rust corner: what the `Mutex` actually protects.** It's tempting to
> read `Mutex<VecDeque<[f32; 2]>>` as "the audio data is protected" and
> stop there, but be precise: the mutex protects the `VecDeque`'s
> *internal invariants* — its length, its buffer pointer, its head/tail
> indices — during the brief window either thread is pushing or popping.
> It says nothing about *timing*: the UI thread can be preempted between
> "check `buf.len()`" and "push a frame" for an arbitrarily long time (a
> repaint stall, a debugger breakpoint, an OS scheduling hiccup), and the
> audio callback will just see whatever the queue's state happens to be
> when it gets the lock — possibly empty, possibly full, never *corrupt*.
> That's the entire promise a mutex makes: no torn reads, no
> use-after-free, no data race. It makes no promise about freshness or
> latency; those are the ring buffer's and the underrun logic's job
> (§11.9, §11.10), layered on top.

> **Rust corner: the audio callback must never block long.** Look at
> `lock()` in [`crates/coco-egui/src/audio.rs:344-346`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs#L344-L346):
> `ring.lock().unwrap_or_else(PoisonError::into_inner)` — recovering from
> mutex poisoning instead of propagating a panic across the thread
> boundary. Why bother? Because a `Mutex::lock()` that panics on a
> poisoned lock would tear down the audio thread on any bug in the UI
> thread's audio code, and — worse — that poison could persist,
> permanently silencing audio for the rest of the process. The function's
> own doc comment states the tradeoff in one line: "a lost frame or two
> of audio is far preferable to tearing down the whole app."
>
> But the deeper rule this section exists to teach is about *time*, not
> panics: a real-time audio callback has a hard deadline (roughly "device
> buffer size ÷ sample rate," often single-digit milliseconds) to hand
> back a full buffer or the device audibly glitches — a stutter, a click,
> a dropout the user hears immediately. Any operation on that thread that
> could block for an unbounded time — a page fault, a slow allocation, a
> lock held by a thread that's itself blocked on something slow — risks
> that deadline. This is why `push_samples`'s entire DSP chain (DC
> block, low-pass, resample) runs on the **producer** side, off the
> audio thread entirely, and the callback's own critical section is
> nothing but `pop_front` calls and, on underrun, a multiply — the
> absolute minimum of work under the lock.
>
> The same reasoning explains a design choice you might otherwise read as
> timidity. `AudioOutput::new()` folds every failure mode — no device, an
> unsupported config, a stream that won't build or won't play — into one
> `Err`, logs a warning, and leaves `stream: None`. It does not panic and
> it does not retry. Optional host hardware that fails should degrade to
> a no-op, not take the emulator with it, and exercise 11.5 asks you to
> trace what every later method does in that state.

---

## 11.6 Artifact one: DC offset, and the one-pole blocker

Time to leave the core crate and walk [`coco-egui/src/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs)'s DSP
chain artifact by artifact, in the order `push_samples` applies them.
Three stages, three defects, and this section is the first: a defect
that is completely inaudible on its own and yet causes two audible
problems.

Think about what the CoCo's 6-bit DAC actually outputs when a program
sets it to, say, 32 (roughly mid-scale) and *holds it there*. Not a
tone, just a steady level, which is the kind of thing an idling music
routine does between notes. The ladder produces a constant, nonzero
voltage, and it holds that voltage for as long as the program leaves the
pins alone.

Constant voltage carries no audio information at all. Hearing is a
response to *changes* in air pressure; a microphone diaphragm sitting
slightly displaced but not moving produces no sound, and neither does a
speaker cone held slightly off-center. A listener cannot hear "the air
pressure sitting a bit high." So the constant part of a signal, its
**DC offset** (the term comes from "direct current," the electrical
engineering name for a component that doesn't alternate), is inaudible
by definition.

It is also, digitally, a problem in two concrete ways.

The first is headroom. Sample values in this pipeline live in a range of
roughly -1.0 to +1.0, which is what the device expects and what
everything downstream assumes. If the "silent" resting level sits at,
say, +0.4 instead of 0.0, then 40% of the available swing in one
direction is gone before a single note plays. A waveform that would have
swung down to -0.6 now runs into -1.0 first and clips, and clipping is
audible in a way DC never is: a squared-off waveform peak sounds like
distortion.

The second is that DC *changes* click. A steady offset is silent, but a
*jump* from one offset to another is a sudden voltage step, and a sudden
step is acoustically an impulse: a pop or thump, heard once, at the
transition. These jumps happen constantly in normal CoCo operation.
Every time a program starts driving the DAC, stops driving it, or
toggles SNDEN to gate the mux on or off, it produces exactly such a
step.

The CoCo's DAC parks at a nonzero resting level, because it is a 6-bit
unsigned value where "off" isn't a special voltage but merely whatever
byte happens to be latched. So both problems are real and constant
across a typical play session rather than being edge cases you could
choose to ignore.

### The filter

The fix is a **DC blocker**, a filter that passes everything except the
part of the signal that isn't changing. It is a species of *high-pass*
filter: it lets high frequencies through and attenuates low ones, with
"zero frequency," which is what DC is, attenuated most of all. Here's
the whole thing
([`crates/coco-egui/src/audio.rs:58-70`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs#L58-L70)):

```rust
struct DCBlocker {
    prev_in: f32,
    prev_out: f32,
}

impl DCBlocker {
    fn process(&mut self, x: f32) -> f32 {
        let y = x - self.prev_in + DC_BLOCKER_POLE * self.prev_out;
        self.prev_in = x;
        self.prev_out = y;
        y
    }
}
```

Two `f32`s of state and one line of arithmetic. Build the intuition for
that line from scratch, because the same reasoning gets reused for the
low-pass in §11.7 and this is the smallest possible place to learn it.

Start by deleting the last term and considering `y = x - prev_in` alone.
That's a *difference* filter: its output is how much the signal changed
since the previous sample. If `x` is constant then `x - prev_in` is
always zero, so a perfectly flat DC level vanishes entirely, which is
exactly the goal. Feed it a rising ramp and it outputs the ramp's slope.
Feed it a fast alternation and it outputs something roughly twice the
amplitude, because consecutive samples differ by twice the amplitude.

But a pure difference filter has a problem of its own: it doesn't just
crush truly-flat signal; it crushes *slowly-changing* signal too, since
"changed a little between samples" and "changed not at all" both produce
small outputs. Real audio's low frequencies change slowly between
samples by definition — that is what low frequency *means* at a 62.8 kHz
sample rate — so a pure difference filter thins out the bass along with
the DC it was aimed at.

That's what the `+ DC_BLOCKER_POLE * prev_out` term fixes. Instead of a
one-shot difference, the filter now has **memory**: a fraction of the
*previous output* feeds back into the current one. This kind of feedback
term is called a *pole*, and one feedback term makes this a "one-pole"
filter. What the feedback does, intuitively, is let the output keep
coasting in the direction it was already going, so a slow change
accumulates across many samples instead of being flattened one sample at
a time. The result is a filter that still kills DC completely but leaves
everything above some low cutoff frequency roughly alone.

The comment on the constant gives that cutoff without requiring anyone
to derive a z-transform
([`crates/coco-egui/src/audio.rs:32-37`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs#L32-L37)):

```rust
/// One-pole DC-blocker feedback coefficient (`y[n] = x[n] - x[n-1] + R*y[n-1]`).
/// Close to 1.0 keeps the cutoff well below audible range (roughly
/// `(1-R) * sample_rate / (2*pi)` Hz) while still pulling the DAC's resting
/// offset (the CoCo's DAC parks at a nonzero level, not 0V) down to ~0 within
/// a few thousand samples.
const DC_BLOCKER_POLE: f32 = 0.995;
```

Run the formula: `(1 - 0.995) × 48,000 / (2π) ≈ 38 Hz`. That is below
the lowest note most CoCo software would ever play, so real bass content
survives untouched while the DC level bleeds away over "a few thousand
samples." At 48 kHz, a few thousand samples is on the order of a tenth
of a second, which is fast enough that no listener would consciously
notice the settling and slow enough that the settling is not itself an
audible click.

The value 0.995 is a *tuning*, and it's worth understanding which way
the knob turns. Push the pole closer to 1.0 and the cutoff drops
further: gentler on the bass, slower to settle. Push it away from 1.0
and the cutoff rises: more aggressive at killing DC, but it starts
eating real low-frequency content. The choice is a direct trade between
"kill DC fast" and "don't touch music," and 0.995 is this codebase's
calibrated answer to that trade at typical device rates.

### The tests, which are the clearest explanation available

`audio_test.rs` proves both halves of the promise directly, and the two
tests together are arguably a better definition of "what a filter does"
than any paragraph above
([`crates/coco-egui/src/audio_test.rs:54-75`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio_test.rs#L54-L75)):

```rust
#[test]
fn dc_blocker_converges_toward_zero_on_constant_input() {
    let mut dc = DCBlocker::default();
    let mut last = 1.0;
    for _ in 0..2000 {
        last = dc.process(1.0);
    }
    assert!(last.abs() < 1e-3, "expected near-zero, got {last}");
}

#[test]
fn dc_blocker_passes_already_centered_signal_without_blowing_up() {
    let mut dc = DCBlocker::default();
    let mut max_abs = 0.0f32;
    for i in 0..1000 {
        let x = if i % 2 == 0 { 1.0 } else { -1.0 };
        max_abs = f32::max(max_abs, dc.process(x).abs());
    }
    // A signal already centered at 0 should stay bounded near its own
    // amplitude, not grow — a highpass shouldn't amplify AC content.
    assert!(max_abs < 2.5, "expected bounded output, got {max_abs}");
}
```

The first test feeds pure DC and asserts it's gone. The second feeds a
signal that is already centered on zero, with no DC component at all,
and asserts it comes through bounded near its own amplitude rather than
growing without limit or being crushed. Both properties are necessary; a
filter that only did the first would be free to destroy everything else
in the signal, and one that only did the second would be an
identity function. Note what the second test does *not* assert: that
the output equals the input. A DC blocker is allowed to alter an
alternating signal somewhat, and this one does, which is why the bound
is 2.5 rather than 1.0.

Neither test opens an audio device, allocates a ring buffer, or knows
that `cpal` exists. That is the payoff of factoring `DCBlocker` out of
`AudioOutput` as its own struct, which is exactly what its doc comment
says it's for: "factored out of `AudioOutput` so the math can be
unit-tested without a live stream."

---

## 11.7 Artifact two: aliasing, and why the low-pass only runs when decimating

The core hands the frontend samples at roughly 62.8 kHz (§11.4). Almost
no consumer sound device runs at 62.8 kHz; 44,100 Hz and 48,000 Hz are
the overwhelming defaults. So `push_samples` has to throw away roughly a
quarter of its input samples to match the device's rate. Discarding
samples to lower a sample rate is called **decimation**, and doing it
naively, by just dropping every Nth sample, creates a defect with a
specific and memorable name: **aliasing**.

### The wagon wheel

Here's the intuition, with no math required first. Picture a wagon wheel
filmed at 24 frames a second. If the wheel spins fast enough, it can
appear to spin *backward*, or to stand still, purely because the camera
isn't sampling fast enough to track the true motion. Each frame catches
the wheel at a slightly wrong point in its rotation, and the eye
stitches those wrong points into a fake, slower apparent motion.

That's aliasing: a signal component too fast for the sampling rate to
represent doesn't simply disappear. It **reappears disguised as a
slower, wrong frequency**, folded back down into the range the sampling
rate *can* represent. The technical threshold has a name, the **Nyquist
rate**: a sampling rate of `R` Hz can faithfully represent frequencies
only up to `R/2` Hz. Anything above that folds back down, mirrored
around `R/2`, into frequencies a listener will actually hear as noise,
buzz, or garbled artifacts that were never in the original signal.

The mirroring is worth making concrete with a number, because "folds
back" is vague until you've computed one. Sample a 28 kHz tone at 48 kHz
and Nyquist is 24 kHz, so the tone is 4 kHz above the limit. It comes
back 4 kHz *below* it: `48,000 - 28,000 = 20,000` Hz. A component nobody
could hear at 28 kHz reappears at 20 kHz, which is right at the top of
human hearing. Push the original a little higher, to 34 kHz, and it
folds to 14 kHz, squarely audible. The higher the offending content, the
lower and more obvious its alias, which is a genuinely counterintuitive
property and the reason aliasing artifacts sound so wrong: they move the
wrong way when the source moves.

### Why the CoCo is a hard case

The core's 62.8 kHz grid can carry real content up to about 31.4 kHz,
its own Nyquist limit. That's inaudible to begin with, since human
hearing tops out around 20 kHz, but it is still *representable* in the
sample stream, and CoCo audio is unusually good at producing it. A
square wave, which is what the beeper produces and what digitized
speech's staircase steps approximate, has harmonics extending far above
its fundamental. Sharp edges are, in frequency terms, exactly what
"lots of high-frequency content" means.

Drop straight to 48 kHz, whose Nyquist is 24 kHz, by picking roughly
every 1.31st sample, and any content living between 24 kHz and 31.4 kHz
folds down into the audible band as new, spurious tones that were never
part of the original sound. The honest high frequencies are inaudible
anyway. Their fold-back products are very much not.

### The fix and its two design decisions

The fix, and the reason it's called **anti-aliasing**, is to remove the
problem frequencies *before* decimating, using a low-pass filter. A
low-pass passes frequencies below some cutoff and attenuates everything
above it, so that by the time samples are discarded there is nothing
left above the new Nyquist limit to fold back down. Order matters
absolutely: filtering after decimation is useless, because by then the
aliases are already sitting in the audible band, indistinguishable from
real signal.

This codebase uses a **2-pole Butterworth low-pass**. "Two-pole" means
it has two feedback terms, compared to the DC blocker's one, and the
extra pole buys a steeper roll-off: content above the cutoff is
attenuated more aggressively per octave than a one-pole filter could
manage. That steepness matters here, because the goal is to actually
suppress the fold-back range rather than to gently discourage it.

Here is the whole filter, coefficients and all
([`crates/coco-egui/src/audio.rs:72-114`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs#L72-L114)):

```rust
/// 2-pole (biquad) low-pass, RBJ-cookbook coefficients, run at the SOURCE
/// rate before decimation. One instance per channel.
#[derive(Default, Clone, Copy)]
struct LowPass {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl LowPass {
    /// Coefficients for cutoff `fc` Hz at sample rate `fs` Hz (fc < fs/2).
    fn design(fc: f64, fs: f64) -> Self {
        let w0 = std::f64::consts::TAU * fc / fs;
        let alpha = w0.sin() / (2.0 * LOWPASS_Q);
        let cos_w0 = w0.cos();
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 - cos_w0) / 2.0 / a0) as f32,
            b1: ((1.0 - cos_w0) / a0) as f32,
            b2: ((1.0 - cos_w0) / 2.0 / a0) as f32,
            a1: (-2.0 * cos_w0 / a0) as f32,
            a2: ((1.0 - alpha) / a0) as f32,
            ..Self::default()
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}
```

The struct's nine fields split cleanly into two groups. `b0`, `b1`,
`b2`, `a1`, and `a2` are the five *coefficients*, computed once by
`design()` and constant thereafter. `x1`, `x2`, `y1`, and `y2` are the
*state*: the two previous inputs and the two previous outputs. That
state is what "2-pole" means concretely, and comparing it to
`DCBlocker`'s single `prev_in`/`prev_out` pair makes the progression
obvious. More memory, steeper filter.

`process` is a **direct-form-I biquad**, which is a standard structure
whose shape you can read straight off the arithmetic: each output is a
weighted sum of the current input, the two previous inputs, and the two
previous outputs, followed by shifting the history along. The `b`
coefficients weight inputs, the `a` coefficients weight outputs, and the
minus signs in front of the `a` terms are a convention baked into how
the coefficients are defined.

`design()` deserves one observation about engineering judgment. The
comment cites the "RBJ cookbook," a well-known standard reference for
exactly these biquad coefficient formulas. This codebase does not
re-derive them, and that is the right call. Nobody hand-derives biquad
coefficients from scratch when a citable standard formula exists, and a
hand-derivation would be harder to review than a citation. Note also
that `design` computes in `f64` and stores in `f32`: the coefficient
math is done once and benefits from the extra precision, while the
per-sample math runs in the narrower type where it happens millions of
times.

The first design choice worth flagging by name is the cutoff:

```rust
/// Anti-alias low-pass cutoff, as a fraction of the DEVICE rate — just under
/// Nyquist, per the plan ("a 2-pole IIR at ~0.45·device-rate is enough").
const LOWPASS_CUTOFF_OF_DEVICE_RATE: f64 = 0.45;
```

At a 48 kHz device rate, Nyquist is 24 kHz, and the cutoff sits at
`0.45 × 48,000 = 21,600` Hz — a little *below* Nyquist rather than right
at it. The margin exists because no real filter has an instant,
brick-wall cutoff. A Butterworth rolls off gradually, starting near its
design frequency, so a cutoff placed exactly at Nyquist would still let
content just above Nyquist through nearly unattenuated, defeating the
point. Sitting at 0.45× instead of 0.50× gives the roll-off curve some
distance to do meaningful work before the fold-back boundary arrives.

Note also that the cutoff is expressed as a fraction of the **device**
rate while the filter is *designed* at the **source** rate:
`LowPass::design(fc, source_rate)`. That is correct and worth
double-checking your intuition on. The frequency that must be removed is
determined by where the *output* Nyquist limit will be, which is a
property of the device. The filter, however, runs on samples arriving at
the source rate, so its coefficients must be computed for that rate.
Getting this pair backwards is an easy mistake and would produce a
filter cutting at completely the wrong frequency.

The companion constant fixes the filter's character:

```rust
/// Butterworth Q for the 2-pole low-pass (maximally flat passband).
const LOWPASS_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;
```

`FRAC_1_SQRT_2` is approximately 0.707, and it is the textbook Q for a
Butterworth design. What "Butterworth" buys is a *maximally flat
passband*: the frequencies you're trying to preserve come through with
no ripple and no peaking near the cutoff. Other 2-pole designs exist and
trade that flatness for a sharper transition, at the cost of passband
ripple this codebase doesn't want. Choosing the family is choosing which
imperfection you'd rather have.

### The detail worth remembering above all others

The filter **only runs when decimating**
([`crates/coco-egui/src/audio.rs:288-294`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs#L288-L294)):

```rust
        if source_rate != self.lowpass_rate {
            self.lowpass_rate = source_rate;
            self.lowpass = (source_rate > self.device_rate).then(|| {
                let fc = LOWPASS_CUTOFF_OF_DEVICE_RATE * self.device_rate;
                [LowPass::design(fc, source_rate); 2]
            });
        }
```

Look at `(source_rate > self.device_rate).then(|| ...)`. The filter is
`None` unless the source rate genuinely exceeds the device rate. This is
not an optimization; it is a correctness statement about what aliasing
*is*. Aliasing is only ever a downsampling problem. When *upsampling* —
feeding a 62.8 kHz stream to a hypothetical 96 kHz device — no samples
are thrown away, so there is nothing to fold back, and running a
low-pass would merely dull the signal for no protective benefit. A
filter that runs unconditionally would be a filter that damages the one
case it can't help.

The surrounding `if source_rate != self.lowpass_rate` is the other half
of the same thought. The source rate is not a fixed constant of the
program; it is a property of *which machine configuration is currently
running*, and §11.5 already noted that the frontend re-asks for it on
every frame. So the filter is designed lazily, on the first
`push_samples` call, and redesigned whenever the rate changes, which is
what happens on an NTSC-to-PAL machine swap. `lowpass_rate` exists
purely to detect that change; without it the filter would either be
designed once against a rate that later became wrong, or redesigned
sixty times a second for no reason.

### The proof

`audio_test.rs` verifies the filter's actual job with a test built
directly on the fold-back scenario, using an input constructed to be the
worst possible case
([`crates/coco-egui/src/audio_test.rs:77-98`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio_test.rs#L77-L98)):

```rust
#[test]
fn lowpass_attenuates_nyquist_rate_alternation_but_passes_dc() {
    // 62.9 kHz source decimated to 48 kHz: a +1/-1 alternation at the
    // source rate (31.45 kHz — pure fold-back material) must be crushed,
    // while a constant passes nearly unchanged.
    let mut lp = LowPass::design(0.45 * 48_000.0, 62_866.0);
    let mut max_late = 0.0f32;
    for i in 0..4000 {
        let y = lp.process(if i % 2 == 0 { 1.0 } else { -1.0 });
        if i > 2000 {
            max_late = f32::max(max_late, y.abs());
        }
    }
    assert!(max_late < 0.2, "Nyquist alternation not attenuated: {max_late}");

    let mut lp = LowPass::design(0.45 * 48_000.0, 62_866.0);
    let mut last = 0.0;
    for _ in 0..4000 {
        last = lp.process(1.0);
    }
    assert!((last - 1.0).abs() < 0.01, "DC gain should be ~1, got {last}");
}
```

A `+1, -1, +1, -1` alternation at the source rate is a signal at exactly
half the source rate — 31.45 kHz — precisely the material that naive
decimation to 48 kHz would fold into the audible band. The test asserts
the filtered output is crushed below 0.2, and it only measures *after*
sample 2000 so that the filter's startup transient doesn't count against
it. The second half feeds constant 1.0 and asserts the output settles at
1.0 within 1%.

Read the two assertions together, because a low-pass has to do two
things simultaneously and each is trivial alone. Killing everything is
easy; passing everything is easier. Killing the high content while
leaving the low content at unit gain is the actual job, and one test
function checks both.

---

## 11.8 Resampling: linear interpolation and the carried remainder

Filtering decides *what* survives. Resampling decides *how many samples
land where*, and it is the stage that finally reconciles the two
incompatible rates this chapter has been circling since §11.4.

The source stream runs at about 62.8 kHz on NTSC, or whatever the
current configuration computes. The device wants exactly 44,100 or
48,000 samples every second. Here is the part that makes resampling
nontrivial rather than a matter of "take every Nth sample": the ratio
between those two rates is essentially never a clean integer.
`62,817.12 / 48,000 = 1.30869...`, and there is no such thing as sample
1.3.

### Linear interpolation

**Linear interpolation** is the answer, and it is the simplest thing
that could possibly work. For each output sample, compute where it falls
*between* two input samples, as a fractional position, then blend those
two neighbors proportionally. If the output position falls a third of
the way from input sample 5 to input sample 6, the output is
`⅔ × sample[5] + ⅓ × sample[6]`. It is the same operation as reading a
value off a graph between two plotted points: draw a straight line
between the two known points and read the height at the position you
want.

Straight lines are of course not what audio waveforms do between
samples, so linear interpolation is an approximation rather than a
reconstruction. It is a defensible one here precisely because of the
previous stage: the low-pass has already removed the content that
interpolation would handle worst, so what reaches the resampler is
comparatively smooth at the scale of a single sample. The module header
makes the dependency explicit, noting that the producer "low-passes
before decimating" because "plain linear decimation would fold the
>Nyquist half of the spectrum straight back into the audible band."
Filter first, interpolate second, and each stage covers the other's
weakness.

Here is the whole resampler, state and all
([`crates/coco-egui/src/audio.rs:116-153`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs#L116-L153)):

```rust
/// Streaming linear resampler state over stereo frames: a fractional position
/// into the input stream plus the last frame of the previous batch, carried
/// across calls so interpolation stays phase-accurate at a non-integer rate
/// ratio (the same "carry the remainder" pattern as `main.rs`'s `field_debt`).
#[derive(Default, Clone, Copy)]
struct Resampler {
    /// Fractional position, in input-frame units, of the next output frame
    /// relative to `prev`..`input[0]`.
    pos: f64,
    /// Last input frame consumed by the previous call (index -1 relative to
    /// the next call's `input`), so the first output frame of a new batch can
    /// still interpolate across the batch boundary.
    prev: [f32; 2],
}

impl Resampler {
    /// Resample `input` (at `step` input-frames-per-output-frame) into `out`,
    /// appending. `step = source_rate / device_rate`.
    fn process(&mut self, input: &[[f32; 2]], step: f64, out: &mut Vec<[f32; 2]>) {
        let n = input.len();
        if n == 0 {
            return;
        }
        loop {
            let i = self.pos.floor() as usize;
            if i >= n {
                break;
            }
            let frac = (self.pos - i as f64) as f32;
            let a = if i == 0 { self.prev } else { input[i - 1] };
            let b = input[i];
            out.push([a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]);
            self.pos += step;
        }
        self.prev = input[n - 1];
        self.pos -= n as f64;
    }
}
```

`pos` is the fractional read-position into `input`, measured in
*input-frame units*. It is not an index; it's a real number, and that is
the entire trick. `step = source_rate / device_rate`, roughly 1.309 for
62.8 kHz into 48 kHz, is how far `pos` advances per output frame. Since
`step > 1` here, each output frame consumes slightly more than one input
frame's worth of position, which is what "downsampling" looks like
expressed as arithmetic: you produce fewer output frames than input
frames because you step through the input faster than one-for-one. The
loop peels off output frames until `pos` runs past the end of the
current batch, detected as `i >= n`, and then stops.

Inside the loop, `i = pos.floor()` picks the pair of neighbors to blend:
`b = input[i]` is the later of the two and `a` is the frame before it,
with `frac = pos - i` saying how far between them the output falls. The
output is then `a + (b - a) * frac`, the straight-line blend. The `if i
== 0 { self.prev } else { input[i - 1] }` expression is what picks `a`,
reaching back into the previous batch when the position falls before
this batch's first sample.

### The two lines that make it seamless

Now the two lines that make this correct across repeated calls rather
than only within one batch:

```rust
        self.prev = input[n - 1];
        self.pos -= n as f64;
```

`push_samples` is called once per `update()`, a batch at a time rather
than on the whole stream at once, so the resampler has to produce output
that is *seamless* across batch boundaries, exactly as if it had been
one continuous call. Two separate problems would break that seamlessness
if left unhandled, and the struct's two fields exist to fix one each.

The first problem is the fractional position itself. If `pos` reset to
0.0 at the start of every batch, the output rate would silently be
wrong: interpolation would always restart from the same phase instead of
continuing where the previous batch left off, and the accumulated error
would show up as an audible warble as the true rate ratio drifted
against the reset-every-batch approximation. `self.pos -= n as f64`
instead carries the *leftover* fractional position forward. After
consuming `n` input frames this call, whatever `pos` overshot past `n`
becomes the next call's starting position.

That is exactly the "carry the remainder" pattern the struct's own doc
comment points at, and the frontend's field pacing uses the identical
idea for video
([`crates/coco-egui/src/app/frame.rs:8-19`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/frame.rs#L8-L19)):

```rust
    /// Emulated fields owed for this update, from wall-clock time at the
    /// machine's field rate (60 Hz NTSC / 50 Hz PAL).
    pub(crate) fn fields_due(&mut self) -> usize {
        let now = std::time::Instant::now();
        let dt = match self.last_update.replace(now) {
            Some(prev) => (now - prev).as_secs_f64().min(MAX_FRAME_DT),
            None => 0.0,
        };
        self.field_debt += dt * self.machine.config.video.field_rate_hz();
        let due = (self.field_debt as usize).min(MAX_FIELDS_PER_UPDATE);
        self.field_debt = (self.field_debt - due as f64).min(1.0);
        due
    }
```

Same shape, different units. A fractional quantity accumulates, the
integer part is consumed, and the remainder is kept for next time so
that rounding never compounds. Chapter 15 covers `fields_due` properly;
seeing the pattern twice in two subsystems is the point of mentioning it
here.

The second problem is that the first output frame of a new batch might
need to interpolate from *before* the new batch's first sample, that is,
between the previous batch's last sample and this batch's first. Without
`prev` there would be no "sample -1" to interpolate from, and the
resampler would either index out of bounds or have to special-case the
boundary into producing a wrong, stale, or zero value. `self.prev =
input[n - 1]` stashes the last frame of *this* batch so that the next
call's `i == 0` branch has something real to blend from.

The test that proves this is one of the cleanest in the codebase,
because it makes the claim structurally rather than by asserting on
specific numbers
([`crates/coco-egui/src/audio_test.rs:19-28`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio_test.rs#L19-L28)):

```rust
#[test]
fn resampler_carries_fractional_position_and_prev_frame_across_calls() {
    // Same math as above, but split across two process() calls to prove
    // the carried `pos`/`prev` state reproduces one continuous stream.
    let mut r = Resampler::default();
    let mut out = Vec::new();
    r.process(&[[0.0; 2]], 0.5, &mut out);
    r.process(&[[10.0, -10.0]], 0.5, &mut out);
    assert_eq!(out, vec![[0.0; 2], [0.0; 2], [0.0; 2], [5.0, -5.0]]);
}
```

The expected value is copied verbatim from the preceding test, which
feeds the same two frames in a *single* call. Splitting the input across
two calls must produce byte-identical output, because the caller's batch
boundaries are an artifact of how often the UI thread happens to run and
should be invisible in the result. That is the entire purpose of
carrying `pos` and `prev`, stated as an equality between two `Vec`s.

### Where this sits in `push_samples`

The chain becomes concrete when read in order in the producer
([`crates/coco-egui/src/audio.rs:296-315`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs#L296-L315)):

```rust
        // DC-block, low-pass, and apply gain/volume on every frame regardless
        // of mute, so filter state and volume don't pop when unmuting
        // mid-stream.
        let gain = MASTER_GAIN * self.volume;
        let processed: Vec<[f32; 2]> = samples
            .map(|[l, r]| {
                let mut l = self.dc[0].process(l) * gain;
                let mut r = self.dc[1].process(r) * gain;
                if let Some(lp) = self.lowpass.as_mut() {
                    l = lp[0].process(l);
                    r = lp[1].process(r);
                }
                [l, r]
            })
            .collect();
        let step = source_rate / self.device_rate;
        let mut resampled = Vec::with_capacity(
            (processed.len() as f64 * self.device_rate / source_rate).ceil() as usize + 1,
        );
        self.resampler.process(&processed, step, &mut resampled);
```

DC blocker, then gain, then low-pass, then resample. One filter instance
per channel throughout, indexed `[0]` and `[1]`, which is what keeps the
stereo channels independent; there is a test asserting exactly that, and
its name is `resampler_keeps_channels_independent`.

The comment above the closure is worth reading twice, because it
describes a bug that would only appear under a specific user action.
Filtering runs "on every frame regardless of mute," and mute is applied
*after* resampling by overwriting the output with zeros. If muting
instead skipped the DSP chain, the filters' internal state would freeze
at whatever it held when mute was pressed, and unmuting would resume
from stale state against a signal that had moved on. The result would be
a pop at the moment of unmuting: exactly the discontinuity §11.6 spent a
section teaching you to recognize. Keeping the filters running while
silencing their output costs a few microseconds per frame and removes an
entire class of glitch.

The `Vec::with_capacity` line is a small piece of care worth noticing on
the way past. The output length is `input_length × device_rate /
source_rate`, rounded up, plus one for the fractional carry, so the
resampled `Vec` is allocated exactly once at the right size. On a
producer that runs sixty times a second, avoiding a reallocation mid-loop
is free to write and costs nothing to maintain.

---

## 11.9 The ring buffer: bounded, drop-oldest, 250 ms deep

`push_samples` ends by appending the resampled frames to `ring`, the
shared queue §11.5 introduced, but it does not do so unconditionally
([`crates/coco-egui/src/audio.rs:317-325`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs#L317-L325)):

```rust
        if self.muted {
            resampled.iter_mut().for_each(|s| *s = [0.0; 2]);
        }

        let mut buf = lock(&self.ring);
        buf.extend(resampled);
        while buf.len() > self.ring_cap {
            buf.pop_front();
        }
```

Note first how short the locked region is. The lock is taken at the very
end of the function, after every expensive operation has already
happened, and it is released when `buf` goes out of scope at the closing
brace. Everything the producer does under the lock is one bulk
`extend` and some pops. That is the same discipline §11.5's third Rust
corner demanded of the *consumer*, applied to the producer for the same
reason: whichever thread holds this lock is a thread the other one is
waiting on, and one of those two threads has a hard real-time deadline.

`ring_cap` is computed once, at stream-open time
([`crates/coco-egui/src/audio.rs:231`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs#L231)):

```rust
        let ring_cap = ((device_rate * RING_BUFFER_SECS) as usize).max(1);
```

with `RING_BUFFER_SECS = 0.25`, a quarter-second of buffered audio
expressed at the device's own rate. At 48 kHz that is 12,000 stereo
frames, or about 96 KB. Two design decisions are packed into those
lines, and both are about **what happens when producer and consumer
drift out of sync**, which they inevitably will, since §11.5 already
established that nothing coordinates their timing beyond the mutex
itself.

### Why bounded at all

The UI thread pushes roughly once per rendered frame, at whatever rate
the UI happens to repaint, and the audio callback drains at whatever
rate the device runs. Those two rates are related only by the fact that
both are approximately real time, and "approximately" is doing a lot of
work. If the UI stalls, whether from a slow repaint, a backgrounded
window, or a debugger breakpoint landing mid-frame, samples keep queuing
while nothing drains them. If the UI thread instead races ahead of a
slow device, the same thing happens for the opposite reason.

An unbounded queue under sustained producer/consumer mismatch is a slow
memory leak with an audible side effect. The memory growth is the
obvious problem; the audible one is worse. A fuller buffer means the
*oldest* queued sample is further in the past by the time the callback
finally reaches it, so unbounded queueing means unbounded **latency**:
the delay between the emulated machine producing a sound and the user
hearing it. Latency is not a subtle defect. Past about a tenth of a
second it makes an interactive program feel broken, because keypresses
and their sound effects stop coinciding.

### Why drop-oldest, not drop-newest or block

Once the cap is exceeded, the *front* of the queue is discarded, which
means the oldest and stalest frames go first. This is a deliberate
choice, and the alternatives are instructive.

Audio, unlike a network protocol, has no way to signal "please resend"
and gains nothing from buffering the past. The user cares about hearing
*now*, not about eventually catching up on everything that happened
while the UI was stalled. Dropping the oldest frames caps worst-case
latency at `RING_BUFFER_SECS` and lets the stream catch up to real time
as fast as possible. Dropping the *newest* instead would mean that the
audio the user eventually hears is always a quarter-second old, forever
behind, even long after the stall that caused it has cleared.

Blocking is worse still. Making `push_samples` wait for the audio thread
to drain space would put the UI thread in a dependency on the audio
thread, which is precisely the kind of cross-thread coupling §11.5
warned against in the other direction. It would also make video stutter
in order to protect audio, which inverts the priorities of an emulator
whose entire architecture, since Chapter 6, is built on video's scanline
clock as the timing backbone. A dropped audio frame is inaudible. A
dropped video frame is visible, and a stalled UI thread is both.

The choice, stated as a principle, is this: when a real-time producer
and a real-time consumer disagree about rate, prefer losing data to
losing time. Section 11.10 applies the same principle from the other
side, where the shortage is data rather than space.

---

## 11.10 Underrun: a fade, not a click or a stuck note

The mirror-image failure to "producer races ahead" is "consumer
starves": the ring buffer runs dry and the audio callback has nothing to
pop. This will happen routinely rather than only as an edge case. Any
time the device's natural draw rate transiently exceeds what's been
pushed — buffer under-fill in the first milliseconds after startup, a UI
frame that took a touch too long, or the emulator being paused at all,
per §11.5 — `pop_front` returns `None`. What should the callback output
for that sample?

Two bad answers, both audible. **Output silence (0.0) immediately** is
an instant jump from whatever the last real sample was down to zero, and
§11.6 established exactly what a sudden level step sounds like: a click
or pop, once per underrun, however brief the underrun was. **Repeat the
last sample forever** avoids the click but replaces it with something
arguably worse for a sustained underrun: a stuck, buzzing tone at
whatever level happened to be playing when the buffer ran dry, still
sounding *present* long after the real signal should have stopped. The
pause case makes the second failure vivid. Pausing the emulator would
leave a tone hanging in the room until the user resumed.

This codebase's answer is a **fade with exponential decay**. Hold the
last real sample, but multiply it toward zero a little more on every
underrun sample, so that a brief starvation is inaudible and a sustained
one decays smoothly to silence instead of parking on a wrong, audible
level:

```rust
        let fade_frames = (device_rate * UNDERRUN_FADE_SECS).max(1.0);
        let decay = UNDERRUN_FADE_FLOOR.powf(1.0 / fade_frames as f32);
        let mut held = [0.0f32; 2];

        let stream = device
            .build_output_stream(
                stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let mut buf = lock(&ring);
                    for frame in data.chunks_mut(channels) {
                        let [l, r] = match buf.pop_front() {
                            Some(s) => {
                                held = s;
                                s
                            }
                            None => {
                                held[0] *= decay;
                                held[1] *= decay;
                                held
                            }
                        };
```

([`crates/coco-egui/src/audio.rs:237-257`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs#L237-L257).) `held` tracks the last
real frame that was actually popped. On an underrun, instead of
outputting `held` unchanged (the stuck-tone failure) or zero (the click
failure), it is multiplied by `decay`, a number just under 1.0, and
*that* becomes both this frame's output and the new `held`. A run of
consecutive underruns therefore keeps shrinking the output toward zero,
sample by sample, instead of jumping there.

Notice where `held`, `decay`, and `fade_frames` are declared: outside
the closure, in `try_build_stream`, and captured by the `move` closure.
`decay` is computed once from the device's own rate, so the fade always
takes `UNDERRUN_FADE_SECS` regardless of whether the device runs at
44.1 kHz or 48 kHz. `held` is captured mutably and persists across
callback invocations, which is what makes the decay continuous across
however many callbacks a long starvation spans.

The constants that shape the curve:

```rust
const UNDERRUN_FADE_SECS: f64 = 0.05;
const UNDERRUN_FADE_FLOOR: f32 = 0.001;
```

`UNDERRUN_FADE_SECS = 0.05` (50 ms) is *how long* the fade should take
to reach effective silence. `UNDERRUN_FADE_FLOOR = 0.001` is *how close
to zero* counts as "there": a thousandth of the held amplitude, which is
-60 dB, a standard "call it silent" threshold in audio engineering and
well below what's perceptible against typical background noise.

`decay` is derived from both by asking a clean question: what per-sample
multiplier, applied `fade_frames` times in a row, lands exactly on the
floor? Multiplying by the same factor `fade_frames` times is the same as
raising it to that power, and the repeated product should equal `FLOOR`
starting from 1.0, so the answer is `FLOOR^(1/fade_frames)`. This is a
general pattern worth keeping: **"decay to X in N steps" is `X^(1/N)`
per step**, and it is how you would compute a fade, a reverb tail, or an
envelope release constant in any audio code, not only this one.

`audio_test.rs`'s `underrun_decay_reaches_floor_within_fade_window` test
verifies the arithmetic directly, with no stream involved
([`crates/coco-egui/src/audio_test.rs:100-110`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio_test.rs#L100-L110)):

```rust
#[test]
fn underrun_decay_reaches_floor_within_fade_window() {
    let device_rate = 48_000.0;
    let fade_frames = (device_rate * UNDERRUN_FADE_SECS) as u32;
    let decay = UNDERRUN_FADE_FLOOR.powf(1.0 / fade_frames as f32);
    let mut held = 1.0f32;
    for _ in 0..fade_frames {
        held *= decay;
    }
    assert!(held <= UNDERRUN_FADE_FLOOR * 1.01);
}
```

Apply `decay` to a starting value of 1.0, `fade_frames` times, and
assert the result has actually reached the floor, with a 1% tolerance
for floating-point rounding. It is the smallest possible test of the
smallest possible piece of arithmetic, and it would catch an inverted
exponent or a misplaced reciprocal immediately.

### The last three lines of the whole journey

One piece of the callback remains, and it is the actual end of the path
this chapter's title promised
([`crates/coco-egui/src/audio.rs:258-265`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs#L258-L265)):

```rust
                        // L→even channels, R→odd; a mono device gets the mix.
                        if frame.len() == 1 {
                            frame[0] = (l + r) * 0.5;
                        } else {
                            for (i, out) in frame.iter_mut().enumerate() {
                                *out = if i % 2 == 0 { l } else { r };
                            }
                        }
```

`data.chunks_mut(channels)` has already sliced the device's interleaved
buffer into one chunk per output frame, so `frame` here is one frame's
worth of channels. A mono device gets the average of left and right,
because dropping one channel would silence anything hard-panned, and the
Orchestra-90 hard-pans by design (§11.11). Anything else gets left on
even channels and right on odd, which handles stereo correctly and gives
a surround device something sensible rather than an error.

And that is the end of the line. A byte the 6809 wrote to `$FF20` is now
an `f32` in a buffer the operating system owns, on its way to a
converter this program will never see. Counting from §11.1, it passed
through a PIA output latch, a DDR mask, an `AudioInputs` snapshot, a
cycle-timestamped event, a grid slot, a mixing function, a `Vec`, a
`Drain`, a DC blocker, a gain stage, a biquad, a linear interpolator, a
mutex, and a ring buffer. Every one of those stages is named after a
problem, and exercise 11.7 asks you to name all of them from memory.

---

## 11.11 Three rungs of PSG complexity: the optional sound chips

Everything so far has been the CoCo's *built-in* sound path: the DAC and
beeper every stock machine has. Cartridges could add real sound chips,
and this codebase models three, each one strictly richer than the last.
None of them is this week's deep-dive, since the GMC and SSC cartridges
that host two of them are Chapters 13 and 14 territory. But seeing all
three side by side, as a taxonomy, tells you something about how PSG
(programmable sound generator) hardware evolved through the early 1980s.
Each one plugs into the audio pipeline you just spent ten sections
learning, through exactly the seams already visible in `mix` and
`flush_line_audio`.

Before the rungs, one distinction from §11.3 needs to be made precise,
because it is what decides where each chip's output enters the pipeline.
A **latch** changes only when written and holds otherwise, so it can be
snapshotted into an `AudioEvent` and replayed exactly. A **generator**
runs on its own clock and produces output whether or not anyone writes
to it, so it must be *asked* for a sample every grid slot. The
`Cartridge` trait has one method for each, and the doc comment on the
generator side explains a subtlety that is easy to get wrong
([`crates/coco-core/src/cart.rs:95-105`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cart.rs#L95-L105)):

```rust
    /// Sample the cartridge's crystal-clocked sound generators (the GMC's
    /// SN76489A) over the next `dt` seconds of wall time, returning the
    /// (left, right) level pair, 0.0–1.0 per channel. Wall time — not CPU
    /// cycles — because these chips run off their own crystal: a CPU-cycle
    /// timebase would let the GIME double-speed poke retune them. Called
    /// once per audio grid slot ([`crate::audio::OVERSAMPLE`] per scanline)
    /// and mixed unconditionally: such carts drive their own outputs, not
    /// the mux-gated SND pin.
    fn generator_sample(&mut self, _dt: f64) -> (f32, f32) {
        (0.0, 0.0)
    }
```

That is why `flush_line_audio` computes `slot_dt` in seconds and passes
it down. A cartridge with its own crystal does not care what the CoCo's
CPU is doing, so `POKE 65495` must not transpose its music. Getting this
wrong would produce a bug of a particularly annoying kind: correct
sound that goes sharp whenever a program enables double speed.

### Rung 1 — Orchestra-90/CC: two dumb latched DACs

The simplest possible "more than one channel" upgrade: two independent
8-bit resistor-ladder DACs, the same R-2R idea as the CoCo's own 6-bit
DAC but wider and doubled, one per stereo channel, each a write-only
latch with no logic behind it at all
([`crates/coco-core/src/orch90.rs:75-105`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/orch90.rs#L75-L105)):

```rust
impl Cartridge for Orch90 {
    fn read(&mut self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }

    fn write(&mut self, addr: u16, val: u8) {
        match addr {
            LEFT_DAC_REG => self.left = val,
            RIGHT_DAC_REG => self.right = val,
            _ => {}
        }
    }
    // ...
    fn sound_levels(&self) -> (f32, f32) {
        const DAC_MAX: f32 = u8::MAX as f32;
        (f32::from(self.left) / DAC_MAX, f32::from(self.right) / DAC_MAX)
    }
}
```

`$FF7A` latches left, `$FF7B` latches right, and both are write-only.
There is no read path back from a 74LS374 octal latch feeding an R-2R
ladder, which is why reads return `IO_OPEN_BUS`: the same "the hardware
genuinely can't answer this" honesty Chapter 5 established for I/O space.
There is no timer, no counter, and no waveform generator on the
cartridge at all. "Sound generation" *is* the CPU's delay loop between
writes, exactly like the CoCo's own internal DAC, just doubled and
stereo. The module header says as much in a sentence worth keeping:
"There is no on-cart timer or interrupt: sample timing is entirely the
CPU's delay loops."

`sound_levels()` is the seam. Section 11.3 showed it consumed directly
into `AudioInputs.cart_left`/`cart_right`, and the walk through `mix`
showed those fields summed into the output **unconditionally**, ignoring
SNDEN and SEL entirely, because the Orch-90 drives its own RCA jacks
rather than the CoCo's internal SND pin. That is not an assumption; it
is asserted by a test
([`crates/coco-core/tests/orch90.rs:69-89`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/orch90.rs#L69-L89)):

```rust
fn cart_audio_reaches_the_speaker_regardless_of_mux_state() {
    // The Orch-90 drives its own RCA outputs, not the CoCo's SND pin, so the
    // SNDEN/SEL mux must never gate it (MAME coco_orch90.cpp routes the DACs
    // to their own speaker, ignoring SOUND_ENABLE). Fresh bus: SNDEN low,
    // SEL=00 — the internal DAC path is silent either way.
    let mut b = bus_with_orch90();
    assert_eq!(b.sound_probe(PROBE_DT), [0.0; 2], "latches power on at 0: silent");

    b.write(LEFT_DAC_REG, 0xFF);
    b.write(RIGHT_DAC_REG, 0xFF);
    let [full_l, full_r] = b.sound_probe(PROBE_DT);
    assert!(full_l > 0.5, "full-scale L with SNDEN low: {full_l}");
    assert_eq!(full_l, full_r, "equal latches are centred");

    // True stereo: zeroing one DAC silences ONLY that channel (hard pan) —
    // the plan's stereo acceptance test.
    b.write(RIGHT_DAC_REG, 0x00);
    let [l, r] = b.sound_probe(PROBE_DT);
    assert_eq!(l, full_l, "left channel unchanged");
    assert_eq!(r, 0.0, "right channel silent");
}
```

Full scale reaches the speaker *with SNDEN low*, which no internal
source could manage, and zeroing one channel silences only that channel.
The second half is why §11.10's mono fold-down averages rather than
picking a channel: a hard-panned Orch-90 track would otherwise vanish on
a mono device.

The companion test, `mpi_dac_writes_ignore_the_slot_select_and_audio_sums`,
shows a related fact about the MultiPak. Deselecting the Orch-90's slot
still leaves its *held* latch values summed into the output, because SND
is an analog line common to every slot and only the digital SCS*, CTS*,
and CART* select lines are switched. Muting a slot does not silence a
cartridge that has already latched a level onto the shared wire, which
is the sort of behavior that emerges from analog wiring and would never
occur to someone designing this in software.

### Rung 2 — SN76489A: tone counters plus LFSR noise

A genuine PSG chip, from the same family that turned up across a
generation of consoles and computers, riding on the Games Master
Cartridge. Where the Orch-90 has zero internal logic, the SN76489A has
**three independent square-wave tone generators plus one noise channel**,
each with its own 4-bit attenuator, all clocked by the cartridge's own
crystal rather than by the CPU's timing loops. These are real
oscillators, not "whatever the software pokes," which is exactly why
they enter the pipeline through `generator_sample` rather than through
`AudioInputs`.

Each tone channel is a down-counter that flips a flip-flop on expiry
([`crates/coco-core/src/sn76489.rs:242-255`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/sn76489.rs#L242-L255)):

```rust
    fn tick(&mut self) {
        for c in 0..3 {
            self.count[c] -= 1;
            if self.count[c] <= 0 {
                self.tone_out[c] = !self.tone_out[c];
                self.count[c] = self.period[c] as i32;
            }
        }
        self.count[NOISE_CHANNEL] -= 1;
        if self.count[NOISE_CHANNEL] <= 0 {
            self.shift_lfsr();
            self.count[NOISE_CHANNEL] = self.period[NOISE_CHANNEL] as i32;
        }
    }
```

A down-counter reaching zero flips the output and reloads from `period`.
That is exactly how you would build a square-wave oscillator out of a
counter and a register: the period register sets the pitch, and the
flip-flop *is* the waveform. Three of these plus a shared clock is most
of a music chip.

The noise channel is the one genuinely new idea here. It is built on an
**LFSR**, a linear feedback shift register: a shift register where some
of the bits shifting out get XORed back in at the top. The result is a
sequence that *looks* random, and sounds like a decent approximation of
white noise when read out as a bitstream, while being completely
deterministic and, eventually, periodic
([`crates/coco-core/src/sn76489.rs:257-268`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/sn76489.rs#L257-L268)):

```rust
    /// White mode XORs taps $04 and $08 into the feedback bit; periodic mode
    /// holds tap 2 at 0, so only tap 1 feeds back — a single set bit then
    /// circulates over 15 shifts (the classic 1/15-duty "periodic noise").
    fn shift_lfsr(&mut self) {
        let tap1 = self.lfsr & LFSR_TAP1 != 0;
        let tap2 =
            self.lfsr & LFSR_TAP2 != 0 && self.regs[REG_NOISE_CTRL] & NOISE_MODE_WHITE != 0;
        self.lfsr >>= 1;
        if tap1 != tap2 {
            self.lfsr |= LFSR_FEEDBACK;
        }
    }
```

Two *taps*, meaning fixed bit positions read before the shift, are XORed
together — `tap1 != tap2` is exactly XOR for booleans — and the result
becomes the new top bit after the register shifts right by one. The
output, which is the noise waveform itself, is just the LFSR's bottom
bit, read every tick
([`crates/coco-core/src/sn76489.rs:281-284`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/sn76489.rs#L281-L284)):

```rust
        if self.lfsr & 1 != 0 {
            sum += self.volume[NOISE_CHANNEL];
        }
        sum
```

The doc comment explains the two noise modes this arrangement produces.
"White" noise XORs both taps, giving genuinely broadband-sounding hiss.
"Periodic" noise disables the second tap so only one feedback path
remains, and with a single bit circulating through a fixed-length
register the output repeats every 15 shifts, producing the distinctive
low buzzy tone classic games used for engine and explosion effects
rather than true hiss.

This is a good chip to remember the shape of, because the shape
generalizes: **an LFSR is how a huge fraction of 1980s hardware
generated "random-sounding" noise** without any actual randomness. Once
you can recognize the pattern — a shift register, a couple of XORed
taps, the bottom bit read as output — you will find it everywhere
pseudo-noise shows up in retro hardware, and often in checksum and
scrambler circuits too.

### Rung 3 — AY-3-8913: adds an envelope generator

The SSC's PSG, and the most capable of the three. It has three tone
channels and a noise generator like the SN76489A, but each channel can
*either* hold a fixed volume *or* follow a single shared **envelope
generator**: a hardware-automated volume-over-time shape, complete with
ramps that rise, fall, repeat, or alternate, long before "ADSR" was a
synth-plugin household term. The payoff for a composer is that a note
can be started and then left alone, swelling or decaying on its own,
without the CPU rewriting a volume register every few milliseconds.

The envelope is one shared unit. The module doc states the consequence
directly: "all three channels that select envelope mode (R8/R9/R10 bit
4) read the same `Envelope::volume`." One envelope for three voices is a
real constraint, and it is the kind of thing that shapes how music for
the chip was written.

The shape itself comes from a 4-bit register that the real chip decodes
into a small set of distinct ramp behaviors
([`crates/coco-core/src/ay8913/envelope.rs:47-65`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/ay8913/envelope.rs#L47-L65)):

```rust
    /// R13 write: (re)starts the envelope at the top of a fresh ramp (MAME
    /// `envelope_t::set_shape`). CONT=0 shapes (bit 3 clear) are folded to
    /// their CONT=1 equivalent — hold forced on, alternate following
    /// whatever attack came out to — exactly like real AY-3-8910 silicon,
    /// which only implements 10 of the 16 possible shape codes distinctly
    /// (the CONT=0 codes duplicate 4 of the CONT=1 ones).
    pub(super) fn set_shape(&mut self, shape_byte: u8) {
        self.attack = if shape_byte & shape::ATTACK != 0 { ENV_STEP_MASK as u8 } else { 0 };
        if shape_byte & shape::CONTINUE == 0 {
            self.hold = true;
            self.alternate = self.attack != 0;
        } else {
            self.hold = shape_byte & shape::HOLD != 0;
            self.alternate = shape_byte & shape::ALTERNATE != 0;
        }
        self.step = ENV_STEP_MASK;
        self.holding = false;
    }
```

Three flags do all the work. `attack` decides whether the ramp counts up
or down. `hold` decides whether it sticks at the final level or repeats.
`alternate` decides whether it flips direction at each end, which turns
a sawtooth into a triangle. Four shape bits combine into ten *effective*
shapes rather than sixteen, because the CONT=0 codes duplicate four of
the CONT=1 ones. That is a quirk of the real AY-3-8910's design, and
this emulator reproduces it exactly rather than "cleaning it up," which
is the right instinct: a program that writes a duplicate shape code
expects the duplicate behavior.

One line in the same file is worth admiring on its own
([`crates/coco-core/src/ay8913/envelope.rs:43-46`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/ay8913/envelope.rs#L43-L46)):

```rust
    /// Current output level, 0-15 ([`ENV_STEP_MASK`]).
    pub(super) fn volume(&self) -> u8 {
        (self.step as u8) ^ self.attack
    }
```

`step` is a counter that only ever decreases, and `attack` is either 0
or `ENV_STEP_MASK` (15). XOR the always-falling counter against 15 and
it becomes an always-rising one; XOR against 0 and it passes through
unchanged. One XOR gives both ramp directions from a single
down-counter, with no second counting direction needed anywhere in the
hardware. This is the kind of trick that shows up constantly in silicon
of this era, where a gate saved is a gate that doesn't cost money on
every unit shipped.

### The ladder, and where each rung enters the pipeline

Position these three exactly as the syllabus does: as **rungs of a
ladder**, not competing designs. The Orch-90 says "sound is entirely
software's job." The SN76489A says "give the CPU real oscillators and a
noise source to program instead." The AY-3-8913 says "and automate
volume-over-time too, so the CPU can set a note going and walk away."
Each rung moves work from software into silicon, which is the whole
story of audio hardware in that decade compressed into three cartridges
for one machine.

Chapters 13 and 14 will put real cartridges around the SN76489A and the
AY-3-8913 respectively and drive them from actual 6809 code. This week's
job was only to show where each chip's samples *enter* the pipeline you
already understand: `sound_levels()` for latched cartridge DACs like the
Orch-90, summed unconditionally; and `audio_sample()` and
`generator_sample()` for the mux-gated and crystal-clocked generator
paths that `flush_line_audio` calls on every grid slot (§11.3). Three
chips, three eras of design philosophy, two trait methods.

---

## 11.12 Reading the tests

You've already read `sound.rs`'s three tests and `audio_grid.rs`'s three
tests inline, as evidence for §11.1's mux table and §11.3's event
timestamping, and `audio_test.rs`'s frontend tests as evidence for
§11.6 and §11.7's filter behavior. Step back now and look at what each
*file* is testing as a whole, because the two core-side files test
different layers of the same pipeline on purpose, and the division
between them is a good model for how to structure tests for any layered
subsystem.

### `sound.rs`: the mux and the mix, with no clock running

**[`crates/coco-core/tests/sound.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/sound.rs)** tests the **mux and the mix
function**, at the bus level, with no scanline loop involved.
`SystemBus::sound_probe`, §11.3's `mix` wrapper, exists for exactly this
purpose: it lets a test poke PIA registers and immediately ask "what
would the speaker hear right now?" without running a single CPU cycle.

That makes the tests read almost like a truth table. Here is the one
that proves §11.1's gating, in full
([`crates/coco-core/tests/sound.rs:52-65`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/sound.rs#L52-L65)):

```rust
#[test]
fn dac_reaches_speaker_only_with_snden_and_mux_zero() {
    let mut b = bus();
    b.write(PIA1_DA, 0xFC); // DAC full scale

    assert_eq!(mono(b.sound_probe(PROBE_DT)), 0.0, "SNDEN low: silent");

    b.write(PIA1_CRB, CR_C2_HIGH); // SNDEN high
    let loud = mono(b.sound_probe(PROBE_DT));
    assert!(loud > 0.5, "SNDEN + SEL=00 routes the DAC: {loud}");

    b.write(PIA0_CRA, CR_C2_HIGH); // SEL1 high -> mux 01 (cassette): silent
    assert_eq!(mono(b.sound_probe(PROBE_DT)), 0.0, "mux away from DAC: silent");
}
```

Three states, three assertions, each one a row of §11.1's table. The
`mono` helper is doing double duty: it asserts that both channels carry
the same value, which is the correct behavior for all internal sources,
and then returns one of them so the rest of the test can read as scalar
arithmetic. A stereo regression in the internal path would fail inside
the helper rather than needing its own test.

`single_bit_sound_is_always_connected` is the companion, and it is four
lines long because the claim is simple: with SNDEN low and the mux
irrelevant, PB1 alone must still reach the speaker.

The third test in that file is doing something different from the other
two, and noticing the difference is the point of this subsection.
`machine_collects_oversample_grid_frames_per_scanline` is the only one
that goes through `Machine` and `run_field`, and it isn't testing the
mux table at all. It tests the **accounting**
([`crates/coco-core/tests/sound.rs:77-92`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/sound.rs#L77-L92)):

```rust
#[test]
fn machine_collects_oversample_grid_frames_per_scanline() {
    let mut m = Machine::new(
        MachineConfig::default(),
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    m.bus.write(0x0000, 0x20); // BRA *
    m.bus.write(0x0001, 0xFE);
    m.run_field();
    let frames =
        m.config.video.lines_per_field() as usize * coco_core::audio::OVERSAMPLE as usize;
    assert_eq!(m.take_audio().count(), frames);
    // Drained: the next field starts fresh.
    m.run_field();
    assert_eq!(m.take_audio().count(), frames);
}
```

Exactly `lines_per_field × OVERSAMPLE` frames land in the buffer per
field, and a second field produces exactly the same count after a drain.
The second half is the interesting assertion: it proves `take_audio`
truly empties the buffer rather than leaving stragglers behind, which is
the sort of off-by-a-few bug that would never be audible but would
slowly desynchronize the resampler's notion of how much time each batch
represents. Note also the two-byte program, `BRA *`, which parks the CPU
in a three-cycle loop so that cycles advance deterministically and
nothing writes to the PIAs.

### `audio_grid.rs`: the thing `sound_probe` can't test

**[`crates/coco-core/tests/audio_grid.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/audio_grid.rs)** tests **sub-scanline timing**,
which is precisely what an instantaneous probe cannot reach. Its module
doc states the mandate in one sentence
([`crates/coco-core/tests/audio_grid.rs:1-7`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/audio_grid.rs#L1-L7)):

```rust
//! The event-timestamped audio grid (`docs/plan-audio-pipeline.md`):
//! sub-scanline DAC timing must land in the right grid slot, and a level
//! pulse entirely inside one scanline — invisible to the old once-per-line
//! point sampler, the digitized-PCM aliasing defect — must reach the grid.
//!
//! The machine runs a zero-filled ROM (reset vector → $0000; the harness
//! parks a `BRA *` there) so cycles advance deterministically, 3 per loop.
```

"Three per loop" is what makes `step_into_line`'s cycle counting exact:
a `BRA *` is three cycles, so stepping until 30 cycles are spent lands
at a predictable place in the line rather than somewhere approximate.

The setup helper is worth reading next to Chapter 10, because it is the PIA
configuration dance from that chapter performed for audio's benefit
([`crates/coco-core/tests/audio_grid.rs:26-40`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/audio_grid.rs#L26-L40)):

```rust
fn dac_machine() -> Machine {
    let mut m = Machine::new(
        MachineConfig::default(),
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    m.bus.write(0x0000, 0x20); // BRA
    m.bus.write(0x0001, 0xFE); // -2
    m.bus.write(PIA1_CRA, CR_DDR);
    m.bus.write(PIA1_DDRA, 0xFC);
    m.bus.write(PIA1_CRB, CR_DDR);
    m.bus.write(PIA1_DDRB, 0x02);
    m.bus.write(PIA1_CRA, CR_C2_LOW);
    m.bus.write(PIA1_CRB, CR_C2_HIGH); // SNDEN on (PIA0 CA2/CB2 reset low = SEL 00)
    m
}
```

Clear the control register to expose the DDR, program the direction
bits, then restore the control register with the C2 output set the way
you want it. That is Chapter 10's three-step shape exactly, performed twice
(once per port), and the DDR values are §11.1's pin assignments: `0xFC`
for PA2 through PA7, the DAC's six wires, and `0x02` for PB1, the
beeper. A test that got this dance wrong would silently test nothing,
because the DDR mask in `snapshot_audio_inputs` would zero out the DAC
value.

`dac_write_mid_line_splits_the_grid_slots` is the test §11.3's worked
example walked through: a DAC write roughly 30 cycles into a line, and
an assertion that *exactly one* transition appears across the four grid
slots, at the right place. It proves events land by cycle timestamp
rather than by arbitrary slot count. Its final two lines then confirm
the other half of the latch model, asserting that the next full line is
loud in every slot, because a latch that was written stays written.

`dac_pulse_within_one_line_reaches_the_grid` is the sharpest test in the
file and is worth re-reading now that the whole pipeline is familiar
([`crates/coco-core/tests/audio_grid.rs:90-120`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/audio_grid.rs#L90-L120)):

```rust
#[test]
fn dac_pulse_within_one_line_reaches_the_grid() {
    // The aliasing defect the pipeline exists to fix: raise the DAC ~10
    // cycles into a line and drop it ~30 cycles later. The line's FINAL
    // state is silent — the old once-per-line point sampler read exactly
    // that and heard nothing — but the grid must carry the pulse.
    let mut m = dac_machine();
    m.run_field();
    m.take_audio().count();

    step_into_line(&mut m, 10);
    m.bus.write(PIA1_DA, 0xFC);
    let mut spent = 0;
    while spent < 30 {
        if let StepKind::Instruction { cycles } = m.step_instruction().kind {
            spent += cycles;
        }
    }
    m.bus.write(PIA1_DA, 0x00);
    let grid = finish_line(&mut m);

    assert!(
        grid.iter().any(|s| s[0] > 0.5),
        "the intra-line pulse must be audible on the grid: {grid:?}"
    );
    assert_eq!(
        *grid.last().unwrap(),
        [0.0; 2],
        "the line ends silent — the state the old sampler was limited to"
    );
}
```

The two assertions are the whole argument of §11.2, expressed as
executable code. The pulse is audible on the grid, *and* the line's
final state is silent. A once-per-line point sampler would have read
exactly that final state and heard nothing at all, so the second
assertion is not merely a sanity check; it is a proof that the first
assertion could not have been satisfied by accident. This is §11.2's
"point-sampling loses whatever happened in between" failure mode, at the
old path's once-per-line resolution, demonstrated as a passing test
against the fix rather than argued in prose.

This chapter's sabotage exercise (§11.14) puts that test's teeth on
display: break the exact mechanism that makes it pass and both of the
`audio_grid` timing tests above fail with the precise diagnostic the
sabotage predicts.

### `orch90.rs`: and one honest note about ROMs

**[`crates/coco-core/tests/orch90.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/orch90.rs)** is worth one honest note before
you run it yourself: six of its seven tests need nothing but a
zero-filled ROM image and pass in any checkout, including a bare clone
with no `roms/` directory. The seventh,
`orch90_autostarts_and_its_cart_code_drives_the_dacs`, boots the **real**
CoCo 3 ROM (`roms/coco3.rom`) to prove the Orch-90's CART*→FIRQ autostart
path actually runs cartridge code from a cold machine — and it fails
loudly, with a clear "cannot read .../roms/coco3.rom: No such file or
directory" message, in a worktree (like this course's) that doesn't have
`roms/` checked out. That's the intended behavior, not a bug in the
test. Chapter 1's reading-assignment habit ("ROMs are local-only... tests
that need a ROM either skip or fail loudly") applies here exactly as
advertised, and this is the first chapter where that behavior shows up
in practice rather than being taken on faith.

It is also worth noticing what that test would prove if it could run.
Its final assertion, after 400 fields of real ROM execution, is simply
that `take_audio()` produced a nonzero sample. One boolean, standing in
for the cartridge autostart path, the FIRQ delivery, the `$FF7A`/`$FF7B`
decode, the latch snapshot, the event record, the grid flush, and the
mix. That is the same "one assertion, most of the machine" property
Chapter 1 admired in the boot tests, applied to audio.

---

## 11.13 Reading assignment

In this order:

1. **[`crates/coco-core/src/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/audio.rs), all of it** (115 lines) — the
   module doc's two-paragraph summary, `OVERSAMPLE` and the gain
   constants, `AudioInputs`, `AudioEvent`, and `mix`. Small enough to
   read start to finish in one sitting; everything else in this chapter
   builds on it.
2. **[`crates/coco-core/src/bus/audio_bridge.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/audio_bridge.rs)** — `snapshot_audio_inputs`,
   `note_audio_write`, `sound_probe`.
3. **[`crates/coco-core/src/machine/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/audio.rs)** — `flush_line_audio`,
   `take_audio`, `audio_sample_rate`. Read it next to [`machine/run.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs)'s
   `end_of_line` ([`run.rs:130-172`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs#L130-L172)) so you see exactly where in the
   per-line trailer it's called.
4. **[`crates/coco-egui/src/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs), all of it** — the whole host chain
   in one file: `DCBlocker`, `LowPass`, `Resampler`, `AudioOutput`, and
   `push_samples`. Read the module doc first; it previews every artifact
   this chapter walked in two short paragraphs.
5. **[`crates/coco-egui/src/app/frame.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/frame.rs), `fields_due` and
   `step_emulation`** — the producer's call site, and the "carry the
   remainder" pattern §11.8 compared the resampler to. Chapter 15 owns this
   file; reading two of its functions now is enough to see where audio
   enters the frame loop and why nothing is pushed while paused.
6. Run both test suites and watch them pass with nothing but a
   zero-filled ROM and no audio hardware:

   ```
   cargo test -p coco-core --test sound --test audio_grid
   cargo test -p coco-egui audio::
   ```

   (The second command runs `coco-egui`'s unit tests filtered to the
   `audio` module — [`crates/coco-egui/src/audio_test.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio_test.rs), gated in via
   `#[cfg(test)] #[path = "audio_test.rs"] mod tests;` at the bottom of
   [`audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs). It builds `cpal` and its platform audio backends, so
   expect a slower first compile than `coco-core`'s suites.)

---

## 11.14 Exercises

**11.1 — Derive the grid rate (recall + math).** Without looking back at
§11.4, recompute the audio grid's sample rate from first principles: you
need `VideoStandard::NTSC`'s `lines_per_field()` and `field_rate_hz()`
(Chapter 6 covered both; they're also in [`crates/coco-core/src/config.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/config.rs))
and `audio::OVERSAMPLE`. Show the two multiplications. Then do the same
for PAL (`lines_per_field() = 312`, `field_rate_hz() = 50.0`) — is PAL's
grid rate higher or lower than NTSC's, and does that match your intuition
about why (fewer fields per second, but how many more lines per field)?

**11.2 — Sabotage the event grid, verified (sabotage — run the actual
suite).** In [`crates/coco-core/src/machine/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/audio.rs), inside
`flush_line_audio`, find this line:

```rust
while cursor < events.len() && events[cursor].cycle <= slot_start {
```

Change `slot_start` to `line_end`, so every event in the line gets
consumed on the very first grid slot regardless of when it actually
happened — collapsing the whole line to its *final* state, exactly the
"sample once per line" failure §11.2 argued against. Predict which tests
in `audio_grid.rs` will fail and how, then run:

```
cargo test -p coco-core --test sound --test audio_grid
```

and confirm your prediction against the actual failure output (pay
attention to which assertions fire and what values they report — do they
match your mental model of "everything happens in slot 0 now"?). Then
revert your one-line change with `Edit` (not `git checkout` — this file
may have other uncommitted work nearby) and re-run the suite to confirm
it's green and `git status` shows a clean tree.

**11.3 — Remove the DC blocker, reasoned (DSP-intuition drill — reason,
don't just assert).** Suppose `push_samples` skipped the `self.dc[0]/[1]
.process(...)` calls entirely and passed samples straight to the
low-pass/resampler stage. Using §11.6's explanation of what a DC level
*is* (a nonzero resting voltage the DAC parks at) and what a difference
filter removes, answer in your own words: (a) what would change about
the *loudness headroom* available to real audio content, quantitatively
if you can estimate it from the DAC's mux-gain constants in
[`crates/coco-core/src/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/audio.rs); (b) what would you *hear*, specifically,
at the moment a program first enables SNDEN or writes a new steady DAC
level after a period of silence — connect this to what §11.6 called "a
sudden voltage step." Don't run the code for this one; the point is
building the intuition without a scope or an ear on hand.

**11.4 — Change `OVERSAMPLE`, predicted then checked (build).** In
[`crates/coco-core/src/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/audio.rs), change `OVERSAMPLE` from `4` to `2`.
Before running anything, predict: (a) the new grid rate (redo exercise
11.1's math); (b) which specific assertion in
`dac_write_mid_line_splits_the_grid_slots` you'd now expect to behave
differently, given that test writes the DAC "~30 cycles into a ~57-cycle
line" and checks for "exactly one level step" — does halving the slot
count change whether that write still lands cleanly on a slot boundary,
or does it now straddle differently? Run
`cargo test -p coco-core --test sound --test audio_grid` and compare
against your prediction. Revert the constant back to `4` when done and
confirm `git status` is clean.

**11.5 — Read the threading boundary (read).** Open
[`crates/coco-egui/src/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs) and answer, citing line numbers: (a)
which method runs on the UI thread and which runs on the audio callback
thread — how do you know, from the code, without external documentation?
(b) name every operation the callback thread performs while holding the
`ring` lock, and estimate (in your own words, not a number) whether any
of them could plausibly take more than a few microseconds. (c) `AudioOutput::new()`
has an `Err` branch that logs a warning and leaves `stream: None` rather
than panicking or retrying — trace what happens to every later method
call (`push_samples`, `menu_ui`) when `stream` is `None`. What principle
does that design choice demonstrate about how optional hardware should
fail?

**11.6 — Add a square-wave test tone (build).** Add a `#[test]` to
[`crates/coco-core/tests/sound.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/sound.rs) (or a new test file) that: configures
the DAC path exactly like `dac_reaches_speaker_only_with_snden_and_mux_zero`'s
`bus()` helper, then alternates `PIA1_DA` between `0xFC` (full scale) and
`0x00` every 16 CPU cycles for several full fields' worth of scanlines,
using `Machine`/`step_instruction` rather than the raw `sound_probe`
(you'll need `dac_machine()`-style setup from `audio_grid.rs` as your
model). Drain `take_audio()` and assert the resulting sample stream
actually alternates between two levels at roughly the frequency you
expect, given the grid rate from exercise 11.1 and 16-cycle half-periods
at the CPU clock (`894,886 Hz`). This is the closest this course gets to
literally hearing your own code — the assertion is numeric, but you're
computing a real audible pitch.

**11.7 — The whole path, in your own words (essay, one paragraph).**
Trace one byte from `LDA #$3F` / `STA $FF20` in a hypothetical 6809
program to a speaker cone, naming every representation the sound passes
through and why each transition exists: PIA output register → `AudioInputs`
snapshot → `AudioEvent` → grid slot (`mix`) → `audio_buffer` → `push_samples`'s
DC-block/low-pass/resample chain → ring buffer → cpal callback → device.
For each arrow, name the one specific problem that stage's transformation
solves (not "it processes the audio" — the actual defect prevented, in
the vocabulary this chapter used: DDR masking, sub-scanline timing,
DC offset, aliasing, non-integer rate ratio, cross-thread scheduling,
underrun). If you can do this from memory without re-opening the
chapter, you've learned the pipeline; if you get stuck on one arrow,
that's the section to reread.

---

## What's next

Chapter 12 stays inside the audio system but flips the direction: instead
of the CPU driving a speaker, the speaker (or rather, a cassette deck's
read head) drives the CPU — CSAVE and CLOAD encode and decode data as
audio tones entirely in software, and the emulator has to re-implement
the ROM's own FSK demodulator well enough to fool it. You already met
the cassette's *output* path in passing this week, with `SEL_CASSETTE`
routing the tape's square wave through the same mux you now understand
completely. Next week is where that square wave's timing — leader
bytes, sync bytes, the motor's spin-up delay — becomes the whole subject,
and where cycle-accurate timing (the fidelity table from Chapter 1) turns
out to matter far more for a tape deck than it ever did for the DAC path
this week covered.
