# Chapter 11 — Sound: from a 6-bit DAC to your speakers

*Week 11. Goal: the whole audio path, core to speaker cone — the most
"systems" chapter in the course. Weeks 1–10 gave you a CPU, a bus, a
scanline clock, a raster, and the PIA chip that mediates most CoCo I/O.
This week you'll watch all of it converge on a single number twice: once
where the 6809 writes a byte to `$FF20`, and once, several layers and one
thread-hop later, where a `f32` lands in a buffer `cpal` is about to hand
to your sound card. In between are two small problems — "the CPU pokes at
arbitrary cycles, the host wants a steady stream" and "62.8 kHz isn't
48 kHz" — and this codebase's answer to each, built up from nothing. If
you have never done digital signal processing, that is fine; every filter
in this chapter is derived from the audible defect it exists to prevent,
not from a textbook.*

---

## 11.1 The hardware you POKEd

There is no sound chip in a stock CoCo. Say that once, plainly, because
it explains everything that follows: the "SOUND" and "PLAY" commands in
Extended Color BASIC, and every machine-language music routine ever typed
in from a magazine listing, worked by wiggling a handful of bits on a
**general-purpose parallel port** fast enough, and in the right pattern,
to approximate a waveform. The chip doing the wiggling is PIA1 — the same
MC6821 you spent week 10 learning as "keyboard, joystick, cassette,
printer, DAC." This week is the "DAC" part.

Three signals matter, all on PIA1 (base `$FF20`, `$FF20–$FF23` — the
table you memorized in week 1):

- **PA2–PA7: a 6-bit DAC.** Six of PIA1's port-A output pins feed a
  resistor ladder (an R-2R network) that converts a 6-bit binary value
  into an analog voltage — the same trick as the Orchestra-90 cartridge
  in §11.10, just built onto the motherboard instead of a cartridge.
  `POKE 65312,n` (`$FF20 = 65312` decimal) with the high six bits of `n`
  set is, quite literally, setting an analog voltage by hand. BASIC's
  `SOUND` and `PLAY` statements call ROM routines that ramp this value up
  and down at audio rates; digitized-speech programs (compressed sample
  data unpacked and written to `$FF20` in a tight loop) push it as fast
  as the CPU can manage. Either way, from the hardware's point of view
  it's the same thing: a stream of bytes landing on six output pins.
- **PB1: a single-bit "beeper."** One pin, on or off, with no ladder —
  just a switch between two voltage levels. Toggle it at an audio
  frequency (a square wave) and you get the harsh, buzzy tone every CoCo
  game's "you lost a life" sound used, because it's the cheapest possible
  way to make a noise: one bit, no D/A conversion at all. It is wired
  straight to the speaker path with no gate — unlike the DAC, described
  next, the single-bit output is *always* connected, which is worth
  remembering as a small "wait, why is this always on?" fact you'll meet
  again in §11.3.
- **The analog mux.** The 6-bit DAC, the cassette input, and cartridge
  audio all want to reach one physical speaker/line-out, and only one of
  them should be audible at a time (plus the cassette's own record path,
  not this chapter's concern — that's week 12's SAVE-side story). A
  4-to-1 analog multiplexer picks which source reaches the amp, gated by
  a master enable and steered by two select bits — and, cleverly, all
  three control signals live on pins the two PIAs already had spare: PIA1
  CB2 is **SNDEN**, the mux's master gate; PIA0 CA2 and CB2 are
  **SEL1/SEL2**, the two address bits that choose the mux's input; PIA1
  CA2 is the cassette motor relay, which additionally gates the
  cassette's own input to the mux. No new chip, no new address range —
  Tandy just found four spare output pins on hardware that already
  existed for other jobs and wired them to a $2 multiplexer IC. This is
  the same "reuse what's already on the bus" instinct you saw with the
  GIME answering to the dead SAM's addresses in week 1 — cheap hardware
  reusing cheap hardware.

Here is the exact table this codebase encodes (`crates/coco-core/src/bus/audio_bridge.rs:15-28`,
`crates/coco-core/src/audio.rs:82-114` — both quoted in full in §11.3):

| SNDEN (PIA1 CB2) | SEL2:SEL1 (PIA0 CB2:CA2) | Mux routes... |
|---|---|---|
| 0 | (any) | nothing — mux output silent |
| 1 | `00` | the 6-bit DAC (PIA1 PA2–7) |
| 1 | `01` | the cassette input, gated by the motor relay |
| 1 | `10` | cartridge audio (the SSC's AY-3-8913, §11.10) |
| 1 | `11` | grounded — silent |

And underneath all of that, unconditionally: the single-bit output on PB1
sums in regardless of what the mux is doing. That's why the beeper could
interrupt a `PLAY` statement's music with a sound effect on some titles —
it was never behind the mux gate at all.

One more thing worth internalizing before you look at a single line of
Rust: **every one of those signals is a PIA output pin**, exactly the
kind of "byte to an address" write you learned in week 5's bus chapter
and week 10's PIA chapter. There is no dedicated audio hardware to model
here beyond "some PIA writes are audio-affecting, and the emulator needs
to notice." The whole interesting engineering problem in this chapter is
what happens to those writes *after* PIA1 records them — which is exactly
where the chapter goes next.

---

## 11.2 The problem: the CPU writes at arbitrary cycles, the speaker wants a steady stream

A modern audio device — the one `cpal` opens on your machine right now —
doesn't accept "here's a value, hold it until further notice." It wants a
**fixed-rate stream** of samples: 44,100 or 48,000 numbers per second,
delivered on a schedule, forever, whether or not anything interesting
happened in between. Real analog electronics don't have this constraint
— a voltage just *is* whatever the DAC ladder is outputting at this
instant, continuously, no sampling involved. The gap between "continuous
voltage" and "discrete stream at a fixed rate" is the entire subject of
digital audio, and it is where every decision in this chapter comes from.

The CPU, meanwhile, writes to `$FF20` whenever the *program* wants to,
not on any fixed schedule the audio device would recognize. A `PLAY`
statement's ROM routine might update the DAC once every few hundred
cycles to shape a tone; a hand-written digitized-speech player might slam
a new byte into `$FF20` every dozen or so cycles, faster than almost
anything else in the machine. The emulator has to bridge "CPU writes
whenever" to "device wants exactly N samples per second," and it has to
do it without losing what the software actually did.

Two naive approaches, and why each fails audibly:

**Sample once at the end of each field/frame.** Read whatever's in
`$FF20` when you're about to hand a frame to the video renderer (60 times
a second) and call that "the audio for this frame." This is *catastrophically*
coarse — 60 Hz sampling can represent almost nothing musical (the
Nyquist limit for 60 Hz sampling is 30 Hz, below the lowest note on a
piano), and a program that writes the DAC a hundred times between frames
— exactly what digitized speech does — has its entire waveform collapsed
to one number per 1/60th of a second. You'd hear silence, or a dull thud,
never speech.

**Sample once per instruction, or once per bus cycle.** Read `$FF20`
after every single CPU cycle and you'd capture everything, in principle
— but at 0.895 MHz that's 895,000 samples per second of bookkeeping for
every field, most of which never changes (the DAC often holds a value for
hundreds of cycles between writes), and you've bought fidelity by paying
for a sample rate no downstream code needs and every consumer would then
have to decimate back down anyway. It's also *still* the wrong shape:
what you actually want isn't "a sample of what the DAC held at cycle N,"
it's "know exactly when it changed and to what" — the write *is* the
event, not a level that happens to get read.

This codebase's actual answer, in one sentence, is: **record every
audio-affecting write as a cycle-timestamped event when it happens, and
reconstruct a fixed-rate sample grid from those events once per
scanline.** That's not "sample more often" — it's a change of
representation, from "levels sampled at some rate" to "an event log,
rendered to a grid on demand." The next section walks the code that does
exactly this.

---

## 11.3 The core's answer: events in, an oversampled grid out

Three pieces cooperate: a small struct that snapshots "everything about
the current sound-affecting state" (`AudioInputs`), a bus-side hook that
notices when that snapshot changes and timestamps it
(`note_audio_write`), and a per-scanline renderer that replays those
timestamped events into a fixed-size grid of samples
(`flush_line_audio`). Read them in that order.

### `AudioInputs`: everything the mux could be looking at, right now

From `crates/coco-core/src/audio.rs:50-68`:

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

This is §11.1's table, turned into a struct: every bit or byte that
governs what the mux does, in one `Copy` value cheap enough to snapshot
on every write. Note what's *not* here: the cassette's own audio level
(sampled once per line — its 1200/2400 Hz tone is far too slow to need
event timestamping, week 12's territory) and anything generator-driven
(the SSC's AY-3-8913, the GMC's SN76489 — those are sampled at flush
time, not latched, because they're continuously running oscillators, not
values a write sets and holds; §11.10 returns to this distinction).

### The snapshot function and the write hook

`crates/coco-core/src/bus/audio_bridge.rs:15-28`, building the snapshot
straight from PIA state:

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

Two details worth pausing on. `self.pia1.a.output & self.pia1.a.ddr &
0xFC` — the DAC value is masked by the **data direction register**, not
just the output register. This is week 10's DDR lesson cashing in
directly: a pin that BASIC's `SOUND` routine never configured as an
output reads as whatever floats on that bit, not as program-intended
data, and the DDR mask is what keeps un-configured pins from leaking
garbage into the mix. `0xFC` then keeps only bits 2–7 (the DAC's six
wires), and `>> 2` slides them down to a 0–63 value — `DAC_MAX` in
`audio.rs` is exactly `63.0`, the top of that range.

And `note_audio_write`, the hook that actually turns "the mux state
changed" into a **timestamped event** (`bus/audio_bridge.rs:48-57`):

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

This runs on every PIA and cartridge-window write (the doc comment says
so explicitly), and it's cheap on the writes that *don't* matter: one
snapshot, one equality check, no push if nothing changed. Only a write
that actually alters the mux's inputs — a new DAC value, a mux-select
flip, SNDEN toggling — gets an `AudioEvent` recorded, with
`self.cycle_clock` (the same cycle counter week 6 built the whole
scanline loop around) as its timestamp. This is the representation
change from §11.2 made concrete: instead of a level sampled at some
fixed rate, you have a sparse log of *exactly when the hardware state
changed and to what*, at full cycle resolution, for the cost of a few
bytes per meaningful write — and most CoCo audio, even digitized speech,
changes the DAC far less often than once per cycle, so the log stays
small.

### `mix`: one grid slot's worth of physics, as a function

Given an `AudioInputs` snapshot (plus the cassette bit and any generator
samples — the sources this section doesn't event-timestamp), `mix`
computes one stereo sample. This is the mux table from §11.1, as code
(`crates/coco-core/src/audio.rs:82-114`):

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

Notice the shape: `if inputs.snden { match inputs.sel { ... } }` is
§11.1's table verbatim — SNDEN gates the whole mux, SEL picks the branch.
And notice what's *outside* that gate: `single_bit` (always summed, per
§11.1's "wait, why is this always on?"), and the cartridge's latched
stereo pair and generator pair — both of which reach the mix
unconditionally too, because the Orchestra-90 and GMC/SSC carts drive
their own RCA jacks, not the CoCo's internal SND pin (you'll see a test
proving exactly this in §11.10). The gain constants
(`DAC_GAIN = 0.75`, `SINGLE_BIT_GAIN = 0.25`, `CASSETTE_GAIN = 0.35`, and
so on, all named at the top of `audio.rs`) are relative loudness
calibration, not hardware facts — someone had to decide the DAC and the
beeper shouldn't compete at equal volume, and the file's comments record
the reasoning per constant rather than leaving magic numbers to guess at.
`CARTRIDGE_GAIN` is also worth a note: it applies to mux-routed cartridge
audio (SEL=10), separately from `CART_GAIN`, which applies to the
always-summed latched/generator paths — two different mux positions for
"cartridge sound," gain-tuned independently.

### `flush_line_audio`: replaying events into a grid, once per scanline

This is where the timestamped log becomes a fixed-rate stream. Called
once per scanline, from the per-line trailer you read in week 6
(`crates/coco-core/src/machine/run.rs:147`, inside `end_of_line`), the
function renders **`OVERSAMPLE` grid slots** — four, in this codebase —
per line, from `crates/coco-core/src/machine/audio.rs:41-68`:

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

Walk it slot by slot. `line_start`/`line_end` bracket the scanline that
just finished, in cycle-clock terms — exactly the same `cycle_clock`
counter `note_audio_write` timestamped events against. `span` is that
line's real cycle width; note the comment about HALT — a line stretched
by the FD-502's halt/DRQ handshake (week 13's subject) still divides
evenly into four slots of its *actual* width, not the nominal one, so
event timestamps always land in the slot they belong to even on an odd
line.

The loop then walks the `OVERSAMPLE` slots in order, and for each one
computes `slot_start` — the slot's left edge, in cycles — and pulls in
every event whose timestamp is `<= slot_start`, updating `inputs` each
time. This is the key move: **events don't get distributed across
slots by count, they get placed by their actual cycle timestamp**, and a
slot picks up whatever the latest event at-or-before its start left the
hardware in. Between events, the hardware just *holds its level* — that's
what a latch is — so this isn't interpolation or guessing, it's exact
reconstruction of a piecewise-constant signal, accurate to one grid
slot's worth of cycles (roughly 14 cycles per slot at 56 cycles/line ÷ 4
— good enough that a change lands in the right ~1/4-scanline window, not
exact to the cycle, which is the resolution `OVERSAMPLE` buys you and the
doc comment at the top of `audio.rs` calls "grid resolution... not
interpolation").

Generators (the AY's accumulator, crystal-clocked PSGs) are sampled
fresh every slot via `audio_sample()`/`generator_sample(slot_dt)` — they
aren't event-driven because they're not latches, they're free-running
oscillators that need to be asked "what have you produced since I last
asked," every slot, regardless of whether any PIA write happened at all.
`mix` then folds latched inputs and generator samples into one stereo
sample, pushed onto `self.audio_buffer`.

The final line matters and is easy to miss: `self.audio_line_inputs =
self.bus.audio_inputs` carries the *current* bus state forward as next
line's starting point — not `inputs` (the loop's local cursor variable,
which might lag behind if an event landed after the last slot's
`slot_start`). Any event in the tail of this line that didn't quite make
the last slot's cutoff still updates `self.bus.audio_inputs` (via
`note_audio_write`), so the next line correctly starts from the
up-to-the-moment truth, and nothing is lost across the line boundary —
just deferred to the first slot of the next line, exactly the "quantized
to grid resolution" behavior the module doc promises.

### `take_audio` and the self-capping buffer

The frontend drains this buffer once per UI update
(`crates/coco-core/src/machine/audio.rs:12-14`):

```rust
pub fn take_audio(&mut self) -> std::vec::Drain<'_, [f32; 2]> {
    self.audio_buffer.drain(..)
}
```

`Drain` hands ownership of the buffered samples to the caller and empties
`audio_buffer` in the same call — no copy, no leftover state. But what if
nothing ever calls `take_audio`? Headless tests, trace tooling, anything
that runs fields without a sound sink attached would otherwise grow this
`Vec` forever. `machine.rs` guards against exactly that
(`crates/coco-core/src/machine.rs:38`, checked in `end_of_line` just
before `flush_line_audio` runs, `run.rs:144-146`):

```rust
const AUDIO_BUFFER_CAP: usize = 8 * 262 * crate::audio::OVERSAMPLE as usize;
```

Eight fields' worth of grid samples (8 × 262 lines × 4 slots = 8,384
frames) is the ceiling; cross it and the buffer is cleared rather than
grown. This is the same "derived scratch, self-bounding" philosophy
you've seen elsewhere in the core — audio is produced whether or not
anyone's listening (a real CoCo's speaker doesn't care if a human is in
the room), so the buffer has to survive being ignored indefinitely
without becoming a leak.

> **Rust corner: `std::mem::take`.** `let events =
> std::mem::take(&mut self.bus.audio_events);` swaps `audio_events` for
> its `Default` (an empty `Vec`) and hands you the old value, in one
> move, with no cloning and no `unsafe`. It's the idiomatic way to say "I
> want to consume this field's current contents and leave something valid
> behind" when you don't have a natural "drain and refill" API (like
> `Vec::drain` above) to reach for — a pattern you'll see again in week
> 15's frame loop. The alternative, `std::mem::replace(&mut x,
> Vec::new())`, does the same thing with one more character to type;
> `take` exists purely because "replace with the default" is common
> enough to deserve its own name.

---

## 11.4 Deriving the grid rate honestly

Chapter 6 taught you a habit: when a comment gives you a round number,
recompute it from the actual constants and see if it agrees. Time to
apply that habit here, because `audio.rs`'s own doc comment invites it:

```rust
/// Grid samples per scanline. 4 → ~62.9 kHz internal rate on NTSC; a named
/// constant per the plan — bump to 8 only if a digitized-speech title
/// measurably needs it.
pub const OVERSAMPLE: u32 = 4;
```

The actual rate, per `crates/coco-core/src/machine/audio.rs:16-24`:

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

`line_rate` is lines-per-field times fields-per-second — the scanline
frequency, the same quantity every NTSC-era engineer called "the
horizontal rate." Plug in the constants week 6 already gave you
(`VideoStandard::NTSC`: `lines_per_field() = 262`, `field_rate_hz() =
59.94`):

```
line_rate  =  262 × 59.94  =  15,704.28 Hz
grid_rate  =  15,704.28 × 4  =  62,817.12 Hz
```

**62,817 Hz, not 62,900.** This is a gentler version of chapter 6's
"56, not 57" lesson — not a truncation bug this time (both factors here
are `f64`, and `audio_sample_rate` never rounds), just a comment and a
handful of test constants (`sound.rs`'s `PROBE_DT = 1.0 / 62_866.0`,
`audio_test.rs`'s `62_866.0`) that rounded to a convenient nearby number
instead of the exact product. Compare the two "the code disagrees with a
round number in a comment" moments and notice the difference in kind:
week 6's 56-vs-57 came from *floor division compounding*, an artifact of
how the arithmetic is written; this one is just imprecise rounding in
prose and test fixtures, with the real computed value sitting one call
away the whole time. Both are worth catching, but only one of them would
have surprised you if you'd trusted the comment and gone looking for a
bug — the lesson is the same either way: **run the numbers yourself
before you trust a comment's "~".**

For a sanity check, compare against the *horizontal rate* every NTSC
reference quotes: 15,734 Hz (from the broadcast-standard 63.5 µs line
period). This codebase's `line_rate()` — 15,704.28 Hz — is close but not
identical, because it's built from `field_rate_hz() = 59.94` rather than
NTSC's exact `30000/1001 ≈ 59.940060` fps, itself a rounding one level up
(the same "6 significant figures is usually enough" convention you'll
find all through `config.rs`). None of this affects correctness inside
the emulator — `audio_sample_rate()` and `take_audio()` always agree with
each other because both derive from the same `line_rate()` call, so the
frontend's resampler (§11.8) is always fed the true rate its input
actually arrived at, whatever that rate's relationship to the "canonical"
15,734 Hz textbook number happens to be.

---

## 11.5 Crossing the thread boundary

Everything so far has lived entirely inside `coco-core`, single-threaded,
no synchronization needed — a `Machine` is just a struct one thread calls
methods on. That changes the moment audio has to reach a speaker. Video
in this codebase gets uploaded to a GPU texture once per UI repaint
(week 15's subject) — the UI thread both produces and consumes it, no
handoff required. Audio can't work that way: the operating system's audio
API calls your code back **on its own thread, on its own schedule**,
expecting samples to already be waiting. `cpal` (the cross-platform audio
library this frontend uses) opens a device and hands you a closure that
runs whenever the OS wants more frames — could be a professional-grade
low-latency driver calling back every few milliseconds, could be
whatever your OS decided today. You do not control when that callback
fires, and it must never be kept waiting.

So `coco-egui/src/audio.rs` has two producers-and-one-consumer running on
two different threads, connected by exactly one piece of shared state
(`crates/coco-egui/src/audio.rs:158-181`):

```rust
pub struct AudioOutput {
    stream: Option<cpal::Stream>,
    ring: Arc<Mutex<VecDeque<[f32; 2]>>>,
    ring_cap: usize,
    device_rate: f64,
    muted: bool,
    volume: f32,
    dc: [DcBlocker; 2],
    lowpass: Option<[LowPass; 2]>,
    lowpass_rate: f64,
    resampler: Resampler,
}
```

`ring: Arc<Mutex<VecDeque<[f32; 2]>>>` is the seam. The **UI thread**
calls `push_samples` once per `update()` (once per rendered frame,
roughly 60 times a second): it drains `Machine::take_audio()`, runs the
whole DSP chain the rest of this chapter covers, and pushes the result
into `ring`. The **cpal callback thread**, running independently and
potentially far more often, locks `ring` and pops frames off the front,
one per output sample the device asked for. Nothing else coordinates the
two threads — no channel, no condvar, no "wait until ready." The queue
either has data or it doesn't, and §11.9/§11.10 cover what happens in
each case.

> **Rust corner: `Arc`, not `Rc`.** Week 1 told you the core crate uses
> *no* `Rc<RefCell<…>>` anywhere, ever — the whole machine is a plain
> owned tree, borrow-checked at compile time. `coco-egui`'s audio module
> is the first place in this codebase that needs *shared ownership*
> across two threads at once, and the type that buys that is `Arc`
> (atomic reference count), never `Rc` (plain, non-atomic reference
> count). The difference is one word — atomic — and it's load-bearing:
> `Rc`'s internal counter increments/decrements with ordinary, non-atomic
> reads and writes, which are only safe if a single thread ever touches
> them. Rust's type system enforces this at compile time: `Rc<T>` doesn't
> implement `Send`, so the compiler simply refuses to let you move one
> across a thread boundary — try to hand a `Rc<Mutex<VecDeque<...>>>` to
> `cpal`'s callback and you get a compile error, not a runtime data race.
> `Arc<T>` costs a little more per clone (an atomic increment instead of
> a plain one) in exchange for that `Send`/`Sync` guarantee. The rule of
> thumb this codebase follows: `Rc` when everything stays on one thread
> (never — the core avoids shared ownership entirely), `Arc` the instant
> two threads need the same allocation, which is exactly and only this
> one ring buffer.

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
> `lock()` in `crates/coco-egui/src/audio.rs:344-346`:
> `ring.lock().unwrap_or_else(PoisonError::into_inner)` — recovering from
> mutex poisoning instead of propagating a panic across the thread
> boundary. Why bother? Because a `Mutex::lock()` that panics on a
> poisoned lock would tear down the audio thread on any bug in the UI
> thread's audio code, and — worse — that poison could persist,
> permanently silencing audio for the rest of the process. But the
> deeper rule this section exists to teach is about *time*, not panics:
> a real-time audio callback has a hard deadline (roughly "device buffer
> size ÷ sample rate," often single-digit milliseconds) to hand back a
> full buffer or the device audibly glitches — a stutter, a click, a
> dropout the user hears immediately. Any operation on that thread that
> could block for an unbounded time — a page fault, a slow allocation, a
> lock held by a thread that's itself blocked on something slow — risks
> that deadline. This is why `push_samples`'s entire DSP chain (DC
> block, low-pass, resample) runs on the **producer** side, off the
> audio thread entirely, and the callback's own critical section is
> nothing but `pop_front` calls and, on underrun, a multiply — the
> absolute minimum of work under the lock.

---

## 11.6 Artifact one: DC offset, and the one-pole blocker

Time to leave the core crate and walk `coco-egui/src/audio.rs`'s DSP
chain artifact by artifact, in the order `push_samples` applies them.

Think about what the CoCo's 6-bit DAC actually outputs when a program
sets it to, say, `32` (roughly mid-scale) and *holds it there* — not a
tone, just a steady level, the kind of thing a lazy `SOUND` implementation
or an idling music routine does between notes. That's a constant, nonzero
voltage. Constant voltage carries no audio information at all (your ear
can't hear "the air pressure sitting slightly high"), but it does two
concrete, bad things once you start processing it digitally:

1. **It eats headroom.** If your sample format's usable range is roughly
   -1.0 to +1.0 and the "silent" resting level sits at, say, +0.4 instead
   of 0.0, you've lost 40% of your dynamic range before a single note
   plays — anything that would have swung the signal down toward -0.6
   now clips at -1.0 first.
2. **It clicks on every *change*.** A DC level itself is silent, but a
   *jump* from one DC level to another — exactly what happens the instant
   a program starts or stops driving the DAC, or when SNDEN toggles the
   mux on — is a sudden voltage step, and a sudden step is, acoustically,
   an impulse: a pop or thump, heard once, at the transition.

The CoCo's DAC parks at a nonzero resting level (it's a 6-bit unsigned
value; "off" isn't a special voltage, it's whatever byte happens to be
latched), so both problems are real and constant across a typical play
session, not edge cases.

The fix is a **DC blocker** — a filter that lets everything through
except "the part of the signal that isn't changing." Here's the whole
thing (`crates/coco-egui/src/audio.rs:56-70`):

```rust
struct DcBlocker {
    prev_in: f32,
    prev_out: f32,
}

impl DcBlocker {
    fn process(&mut self, x: f32) -> f32 {
        let y = x - self.prev_in + DC_BLOCKER_POLE * self.prev_out;
        self.prev_in = x;
        self.prev_out = y;
        y
    }
}
```

with `DC_BLOCKER_POLE = 0.995`. Build the intuition for this formula from
scratch, because you'll reuse the same reasoning for the low-pass in
§11.7.

Start with `y = x - prev_in` alone (drop the `+ 0.995·prev_out` term for
a moment). That's a *difference* filter: it outputs how much the signal
changed since the last sample. If `x` is constant, `x - prev_in` is
always zero — a perfectly flat DC level vanishes entirely, which is
exactly what you want. But a pure difference filter has a problem of its
own: it also crushes *slowly-changing* signal, not just truly-flat
signal, because "changed a little" and "changed not at all" both produce
small outputs. Feed it a real audio waveform and the low end (bass,
mostly) gets thinned out along with the DC you were trying to remove.

That's what the `+ DC_BLOCKER_POLE * prev_out` term fixes: instead of a
one-shot difference, it's a difference filter with **memory** — a small
fraction (99.5%) of the *previous output* feeds back in. That feedback
term is what turns "kill DC entirely, and also thin out the bass" into
"kill DC, and roughly leave everything above some low cutoff frequency
alone." The comment on the constant gives you the cutoff formula without
requiring you to derive the z-transform yourself
(`crates/coco-egui/src/audio.rs:32-37`):

```rust
/// One-pole DC-blocker feedback coefficient (`y[n] = x[n] - x[n-1] + R*y[n-1]`).
/// Close to 1.0 keeps the cutoff well below audible range (roughly
/// `(1-R) * sample_rate / (2*pi)` Hz) while still pulling the DAC's resting
/// offset (the CoCo's DAC parks at a nonzero level, not 0V) down to ~0 within
/// a few thousand samples.
const DC_BLOCKER_POLE: f32 = 0.995;
```

`(1 - 0.995) × 48,000 / (2π) ≈ 38 Hz` — below the lowest note most CoCo
software would ever play, so real bass content survives untouched while
the DC level bleeds away over "a few thousand samples" (at 48 kHz, a few
thousand samples is on the order of a tenth of a second — fast enough
that you'd never consciously notice the fade-in, slow enough that it
isn't itself an audible click). Push the pole *closer* to 1.0 and the
cutoff drops further (gentler, slower to settle); push it *away* from 1.0
and the cutoff rises (more aggressive, but starts eating real bass) — the
"how close to 1.0" choice is a direct trade between "kill DC fast" and
"don't touch music," and 0.995 is this codebase's calibrated answer for
that trade.

`audio_test.rs` proves both halves of the promise directly
(`crates/coco-egui/src/audio_test.rs:54-75`): a constant input converges
to near-zero within 2,000 samples (the DC-killing behavior), and an
already-centered alternating signal (`+1, -1, +1, -1, ...` — no DC
component at all) stays bounded near its own amplitude rather than
blowing up or getting crushed (the "don't touch real signal" behavior).
Read both tests; they're the cleanest possible demonstration of what "a
filter" even means before you've built any intuition for the word.

---

## 11.7 Artifact two: aliasing, and why the low-pass only runs when decimating

The core hands the frontend samples at ~62.8 kHz (§11.4). Almost no
consumer sound device runs at 62.8 kHz — 44,100 Hz and 48,000 Hz are the
overwhelming defaults. So `push_samples` has to throw away roughly a
quarter of its input samples to match the device's rate. This is called
**decimation**, and doing it naively — just dropping every Nth sample —
creates a defect with a specific, memorable name: **aliasing**.

Here's the intuition, no math required first. Imagine a wagon wheel
filmed at 24 frames a second. If the wheel spins fast enough, it can look
like it's spinning *backward*, or standing still, purely because the
camera isn't sampling fast enough to track the true motion — each frame
catches the wheel at a slightly-wrong point in its rotation, and your eye
stitches those wrong points into a fake, slower apparent motion. That's
aliasing: a signal component too fast for the sampling rate to represent
doesn't just disappear — it **reappears disguised as a slower, wrong
frequency**, folded back down into the range the sampling rate *can*
represent. The technical threshold is the **Nyquist rate**: a sampling
rate of `R` Hz can only faithfully represent frequencies up to `R/2` Hz;
anything above that folds back down, mirrored around `R/2`, into
frequencies you'll actually hear as noise, buzz, or garbled artifacts
that were never in the original signal.

The core's 62.8 kHz grid can carry real content up to ~31.4 kHz (its own
Nyquist limit) — inaudible to begin with, since human hearing tops out
around 20 kHz, but still *representable* in the sample stream, especially
from a hard-edged waveform like a square wave (the beeper, digitized
speech's staircase steps) whose harmonics extend far above its
fundamental. Drop straight to 48 kHz (Nyquist 24 kHz) by picking every
~1.31st sample, and any content that lived between 24 kHz and 31.4 kHz —
still inaudible on its own — folds down into the audible band as
new, spurious tones that were never part of the original sound. The
higher, honest frequencies you can't hear anyway; the fold-back product
you very much can.

The fix, and the reason it's called **anti-aliasing**, is to remove the
problem frequencies *before* decimating, with a low-pass filter — a
filter that passes frequencies below some cutoff and attenuates
everything above it, so there's nothing left above the new Nyquist limit
to fold back down. This codebase uses a **2-pole Butterworth low-pass**,
"2-pole" meaning it has two feedback terms (compare the DC blocker's
one), which buys a steeper roll-off — content above the cutoff is
attenuated more aggressively per octave than a 1-pole filter could
manage, which matters because you want to actually suppress the
fold-back range, not just gently discourage it.

```rust
/// 2-pole (biquad) low-pass, RBJ-cookbook coefficients, run at the SOURCE
/// rate before decimation. One instance per channel.
#[derive(Default, Clone, Copy)]
struct LowPass {
    b0: f32, b1: f32, b2: f32,
    a1: f32, a2: f32,
    x1: f32, x2: f32,
    y1: f32, y2: f32,
}
```

(`crates/coco-egui/src/audio.rs:72-114`; `design()` builds the five
coefficients from a cutoff frequency and the Butterworth `Q` — the
"RBJ cookbook" the comment cites is a well-known standard reference for
exactly these formulas, and this codebase doesn't re-derive them, just
applies them, which is the right call: nobody hand-derives biquad
coefficients from scratch when a citable standard formula exists).
`process` is a **direct-form-II biquad**: each output depends on the
current and two previous inputs (`x`, `x1`, `x2`) and the two previous
outputs (`y1`, `y2`) — more history than the DC blocker's one-sample
memory, which is exactly what "2-pole" means concretely.

Two design choices worth flagging by name. First, the cutoff:

```rust
/// Anti-alias low-pass cutoff, as a fraction of the DEVICE rate — just under
/// Nyquist, per the plan ("a 2-pole IIR at ~0.45·device-rate is enough").
const LOWPASS_CUTOFF_OF_DEVICE_RATE: f64 = 0.45;
```

At a 48 kHz device rate, Nyquist is 24 kHz; the cutoff sits at `0.45 ×
48,000 = 21,600` Hz — a little *below* Nyquist, not right at it. Why the
margin? No real filter has an instant, brick-wall cutoff; a Butterworth
rolls off gradually starting near its design frequency. Set the cutoff
exactly at Nyquist and the filter's own gradual roll-off would still let
some content just above Nyquist through mostly unattenuated, defeating
the point. Sitting at 0.45× instead of 0.50× leaves margin for the
filter's real-world roll-off curve to have done meaningful work by the
time you reach the fold-back boundary. `LOWPASS_Q =
std::f64::consts::FRAC_1_SQRT_2` (≈0.707) is the textbook "maximally
flat passband" Q for a Butterworth design — the value that avoids
introducing its own ripple or peaking in the frequencies you're trying to
preserve, which is the entire point of choosing "Butterworth" as the
filter family in the first place (other 2-pole designs trade flatness for
a sharper cutoff, at the cost of passband ripple this codebase doesn't
want).

Second, and this is the detail worth remembering above all others in
this section — the filter **only runs when decimating**:

```rust
if source_rate != self.lowpass_rate {
    self.lowpass_rate = source_rate;
    self.lowpass = (source_rate > self.device_rate).then(|| {
        let fc = LOWPASS_CUTOFF_OF_DEVICE_RATE * self.device_rate;
        [LowPass::design(fc, source_rate); 2]
    });
}
```

`(source_rate > self.device_rate).then(|| ...)` — the filter is `None`
unless the source rate (62.8 kHz on NTSC) genuinely exceeds the device
rate. Aliasing is *only ever* a downsampling problem: if you're
*upsampling* (feeding a 62.8 kHz stream to a hypothetical 96 kHz device),
you're not throwing samples away, so there's nothing to fold back and
running a low-pass would just needlessly dull the signal for no
protective benefit. This is also, incidentally, why the filter is
redesigned "lazily... on the first `push_samples` call" and again "on
change ([NTSC↔PAL machine swap])" — the source rate isn't a fixed
constant of the *program*, it's a property of *which machine config is
currently running*, and the filter has to track it.

`audio_test.rs` proves the filter does its actual job with a test built
directly on the Nyquist-folding scenario: alternating `+1/-1` at the
*source* rate is, by construction, exactly the fold-back material a
naive decimation to 48 kHz would turn into garbage — and the test asserts
the filtered output is crushed to near-zero, while a constant DC input
passes through at unit gain (`crates/coco-egui/src/audio_test.rs:77-98`).
Read both assertions together: a low-pass has to do two things
simultaneously — kill the high stuff, leave DC/low stuff alone — and the
test checks both, in one function.

---

## 11.8 Resampling: linear interpolation and the carried remainder

Filtering handles *what* survives; resampling handles *how many samples
land where*. The source stream runs at ~62.8 kHz (or whatever `NTSC`/`PAL`
computes); the device wants exactly 44,100 or 48,000 samples every
second, and — this is the part that makes resampling nontrivial rather
than "every Nth sample" — the ratio between those two rates is essentially
never a clean integer. `62,817.12 / 48,000 = 1.30869...` — you cannot
just "take every 1.3rd sample," because there's no such thing as sample
1.3.

**Linear interpolation** is the answer: for each output sample, compute
where it falls *between* two input samples (a fractional position), and
blend those two neighbors proportionally. If the output position falls a
third of the way from input sample 5 to input sample 6, the output is
`⅔ × sample[5] + ⅓ × sample[6]`. It's the same idea as reading a
value off a graph between two plotted points by eye — draw a straight
line between the two known points and read the height at the position
you want.

```rust
#[derive(Default, Clone, Copy)]
struct Resampler {
    pos: f64,
    prev: [f32; 2],
}

impl Resampler {
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

(`crates/coco-egui/src/audio.rs:120-153`.) `pos` is the fractional
read-position into `input`, in *input-frame units* — not an index, a real
number. `step = source_rate / device_rate` (≈1.309 for 62.8 kHz →
48 kHz) is how far `pos` advances per output frame: since `step > 1`
here, each output frame consumes slightly more than one input frame's
worth of position, which is exactly "downsampling" expressed as
arithmetic — you produce fewer output frames than input frames because
you're stepping through the input faster than one-for-one. The loop
peels off output frames until `pos` runs past the end of the current
batch (`i >= n`), then stops.

Inside the loop: `i = pos.floor()` is the input sample *just before* the
desired position, `frac = pos - i` is how far past that sample you are
(0.0 = exactly on `input[i-1]`/`prev`, approaching 1.0 = almost at
`input[i]`), and the output is `a + (b - a) * frac` — the straight-line
blend between neighbor `a` (`input[i-1]`, or `prev` when `i == 0`) and
neighbor `b` (`input[i]`).

Now the two lines that make this correct across repeated calls, not just
within one batch:

```rust
self.prev = input[n - 1];
self.pos -= n as f64;
```

`push_samples` is called once per `update()` — a batch at a time, not the
whole stream at once — so the resampler has to produce output that's
*seamless* across batch boundaries, as if it were one continuous call.
Two problems would break that seamlessness if left unhandled, and the
struct carries exactly the state needed to fix both:

1. **The fractional position itself.** If `pos` reset to 0.0 at the start
   of every batch, the output rate would silently be wrong — you'd
   always restart interpolation from the same phase instead of
   continuing where the last batch left off, producing an audible warble
   as the true rate ratio drifts against the reset-every-batch
   approximation. `self.pos -= n as f64` instead carries the *leftover*
   fractional position forward: after consuming `n` input frames this
   call, whatever `pos` overshot past `n` becomes next call's starting
   position, exactly the same "carry the remainder" idea `main.rs`'s
   `field_debt` uses for video timing (mentioned in the struct's own doc
   comment) — you'll meet that one properly in week 15.
2. **The first output frame of a new batch might need to interpolate
   *before* the new batch's first sample** — i.e., between the *previous*
   batch's last sample and this batch's first. Without `prev`, there's no
   "sample -1" to interpolate from, and the resampler would either panic
   indexing `input[-1]` or have to special-case the batch boundary into
   producing a wrong (stale or zero) value. `self.prev = input[n - 1]`
   stashes the last frame of *this* batch so the *next* call's `i == 0`
   branch has something real to blend from — the `if i == 0 { self.prev
   } else { input[i - 1] }` line you already read.

`audio_test.rs`'s
`resampler_carries_fractional_position_and_prev_frame_across_calls` test
proves exactly this: it feeds the same two frames through `process`
split across two separate calls, and asserts the output is *identical*
to feeding them in one call — the whole point of carrying `pos`/`prev` is
that the caller's batch boundaries should be invisible in the output.

---

## 11.9 The ring buffer: bounded, drop-oldest, 250 ms deep

`push_samples` ends by appending the resampled frames to `ring`
(§11.5), but not unconditionally:

```rust
let mut buf = lock(&self.ring);
buf.extend(resampled);
while buf.len() > self.ring_cap {
    buf.pop_front();
}
```

`ring_cap` is computed once, at stream-open time
(`crates/coco-egui/src/audio.rs:231`): `(device_rate * RING_BUFFER_SECS)
as usize`, with `RING_BUFFER_SECS = 0.25` — a quarter-second of buffered
audio, at the device's own rate. Two design choices here, and both are
about **what happens when producer and consumer drift out of sync**,
which they inevitably will (§11.5 already told you nothing coordinates
their timing beyond the mutex).

**Why bounded at all?** The UI thread pushes roughly once per frame,
whatever the audio callback's actual drain rate happens to be. If the UI
stalls — a slow repaint, the window backgrounded, a debugger breakpoint
mid-frame — samples keep queuing while nothing drains them, or if the UI
thread races ahead of a slow device, the queue would otherwise grow
without limit. An unbounded ring buffer under sustained producer/consumer
mismatch is a slow memory leak with an audible side effect: growing
latency, since a fuller buffer means the *oldest* queued sample is
further in the past by the time the callback finally gets to it.

**Why drop-oldest, not drop-newest or block?** Once the cap is exceeded,
the *front* of the queue — the oldest, stalest frames — gets discarded
(`pop_front`), not the newest. This is a deliberate choice: audio, unlike
a network protocol, has no way to signal "please resend" or benefit from
buffering the past — the user cares about hearing *now*, not about
eventually catching up on everything that happened while the UI was
stalled. Dropping the oldest frames caps worst-case latency at
`RING_BUFFER_SECS` and lets the stream "catch up" to real time as fast as
possible; dropping the newest, by contrast, would mean the audio the user
eventually hears is *always* from a quarter-second ago even once the
stall clears, forever behind. Blocking — making `push_samples` wait for
the audio thread to drain space — is even worse: that's the UI thread
now stalling on the audio thread, exactly the kind of cross-thread
dependency §11.5 warned the audio callback must never create in the
*other* direction, and here it would additionally make video stutter to
protect audio, backward priorities for an emulator whose whole
architecture (week 6) is built around video's scanline clock as the
timing backbone.

---

## 11.10 Underrun: a fade, not a click or a stuck note

The mirror-image failure to "producer races ahead" is "consumer starves"
— the ring buffer runs dry and the audio callback has nothing to pop.
This will happen routinely, not just as an edge case: any time the
device's natural draw rate transiently exceeds what's been pushed (buffer
under-fill on startup, a UI frame that took a touch too long), `pop_front`
returns `None`. What should the callback output for that sample?

Two bad answers, both audible: **output silence (0.0) immediately** —
that's an instant jump from whatever the last real sample was down to
zero, which (per §11.6's DC-blocker discussion) is exactly what a sudden
level *step* sounds like: a click or pop, once per underrun, however
brief. Or **repeat the last sample forever** — that avoids the click but
replaces it with something arguably worse for a sustained underrun: a
stuck, buzzing tone at whatever level happened to be playing when the
buffer ran dry, which keeps sounding *present* long after the real signal
should have stopped.

This codebase's answer is a **fade with exponential decay** — hold the
last real sample, but multiply it toward zero a little more on every
underrun sample, so a brief starvation is inaudible and a sustained one
decays smoothly to silence instead of parking on a wrong, audible level:

```rust
let fade_frames = (device_rate * UNDERRUN_FADE_SECS).max(1.0);
let decay = UNDERRUN_FADE_FLOOR.powf(1.0 / fade_frames as f32);
let mut held = [0.0f32; 2];

// ...inside the callback, per output frame:
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

(`crates/coco-egui/src/audio.rs:237-257`.) `held` tracks the last real
frame that was actually popped; on an underrun, instead of outputting
`held` unchanged (the "stuck tone" failure) or zero (the "click" failure),
it's multiplied by `decay` — a number just under 1.0 — and *that*
becomes both this frame's output and the new `held`, so a run of
consecutive underruns keeps shrinking the output toward zero, sample by
sample, instead of jumping there.

The constants that shape the curve:

```rust
const UNDERRUN_FADE_SECS: f64 = 0.05;
const UNDERRUN_FADE_FLOOR: f32 = 0.001;
```

`UNDERRUN_FADE_SECS = 0.05` (50 ms) is *how long* the fade should take
to reach "effectively silent"; `UNDERRUN_FADE_FLOOR = 0.001` (-60 dB, a
thousandth of the held amplitude — a standard "call it silent" threshold
in audio engineering, where -60 dB is well below what's perceptible
against typical background noise) is *how close to zero* counts as
"there." `decay` is derived from both, by asking "what per-sample
multiplier, applied `fade_frames` times in a row, lands exactly on the
floor?" — which is precisely `FLOOR^(1/fade_frames)`, since multiplying
by the same factor `fade_frames` times is the same as raising it to that
power, and you want that repeated product to equal `FLOOR` starting from
1.0. This is a clean, general pattern worth keeping: **"decay to X by N
steps" is `X^(1/N)` per step**, and it's how you'd compute a fade,
reverb tail, or envelope release constant in any audio code, not just
this one.

`audio_test.rs`'s `underrun_decay_reaches_floor_within_fade_window` test
verifies the arithmetic directly: apply `decay` to a starting value of
1.0, `fade_frames` times, and assert the result has actually reached
`UNDERRUN_FADE_FLOOR` (with a 1% tolerance for floating-point rounding).

---

## 11.11 Three rungs of PSG complexity: the optional sound chips

Everything so far has been the CoCo's *built-in* sound path — the DAC and
beeper every stock machine has. Cartridges could add real sound chips,
and this codebase models three, each one strictly richer than the last.
None of them is this week's deep-dive (the GMC and SSC carts that host
two of them are weeks 13–14 territory), but seeing all three side by
side, as a taxonomy, tells you something about how PSG (programmable
sound generator) hardware evolved through the early '80s — and each one
plugs into the audio pipeline you just spent ten sections learning, via
exactly the seams (`sound_levels`, `generator_sample`, `audio_sample`)
already visible in `mix` and `flush_line_audio`.

**Rung 1 — Orchestra-90/CC: two dumb latched DACs.** The simplest
possible "more than one channel" upgrade: two independent 8-bit
resistor-ladder DACs (the same R-2R idea as the CoCo's own 6-bit DAC,
just wider and doubled), one per stereo channel, each a write-only
latch with no logic behind it at all
(`crates/coco-core/src/orch90.rs:75-105`):

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

`$FF7A` latches left, `$FF7B` latches right, both write-only (no read
path back from a 74LS374 octal latch feeding an R-2R ladder — hence
`IO_OPEN_BUS` on reads, the same "the hardware genuinely can't answer
this" honesty week 5 taught you to expect from I/O space). There is no
timer, no counter, no waveform generator on the cartridge at all — "sound
generation" *is* the CPU's delay loop between writes, exactly the CoCo's
own internal DAC, just doubled and stereo. `sound_levels()` is the seam:
you saw it consumed directly inside `AudioInputs.cart_left/cart_right`
back in §11.3, and — per the mix function you already read — it sums
into the mix **unconditionally**, ignoring SNDEN/SEL entirely, because
the Orch-90 drives its own RCA jacks, not the CoCo's internal SND pin.
`crates/coco-core/tests/orch90.rs`'s
`cart_audio_reaches_the_speaker_regardless_of_mux_state` test proves
exactly that bypass, and the `mpi_dac_writes_ignore_the_slot_select_and_audio_sums`
test shows the flip side: through a MultiPak, deselecting the Orch-90's
slot still leaves its *held* latch values summed into the output — SND
is an analog line common to every slot, only the digital SCS*/CTS*/CART*
select lines are switched, so muting a slot doesn't silence a cart
that's already latched a level onto the shared wire.

**Rung 2 — SN76489A: tone counters plus LFSR noise.** A genuine PSG chip
(the same family Sega and TI used across a generation of consoles and
computers), on the Games Master Cartridge. Where the Orch-90 has zero
internal logic, the SN76489A has **three independent square-wave tone
generators plus one noise channel**, each with its own 4-bit attenuator,
all clocked by the cart's own crystal rather than the CPU's timing loops
— real oscillators, not "whatever the software pokes." Each tone channel
is a down-counter that flips a flip-flop on expiry
(`crates/coco-core/src/sn76489.rs:242-255`):

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

— a down-counter reaching zero flips the output and reloads from
`period`, which is exactly how you'd build a square-wave oscillator out
of a counter and a register: period sets the pitch, the flip-flop *is*
the waveform. The noise channel is worth a closer look, because it's the
one genuinely new idea here: an **LFSR** (linear feedback shift register)
— a shift register where some of the bits shifting out get XORed back in
at the top, which produces a sequence that *looks* random (a good
approximation of white noise when sampled as a bitstream) but is
completely deterministic and, eventually, periodic:

```rust
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

Two "taps" (fixed bit positions read before the shift) are XORed
together (`tap1 != tap2` is exactly XOR for booleans) and the result
becomes the new top bit after the register shifts right by one; the
output — the noise waveform itself — is just the LFSR's bottom bit,
read every tick (`level()`, `crates/coco-core/src/sn76489.rs:281-284`:
`if self.lfsr & 1 != 0 { sum += self.volume[NOISE_CHANNEL] }`). The
comment on `shift_lfsr` explains the two noise modes this produces:
"white" noise XORs both taps (genuinely broadband-sounding hiss), while
"periodic" noise disables the second tap so only one feedback path
remains — with a single bit circulating through a fixed-length register,
the output repeats every 15 shifts, producing the distinctive low buzzy
tone classic games used for engine/explosion effects rather than true
hiss. This is a good chip to remember the shape of: **an LFSR is how a
huge fraction of 1980s hardware generated "random-sounding" noise**
without any actual randomness, and you'll recognize the pattern (a shift
register, a couple of XORed taps) anywhere pseudo-noise shows up in
retro hardware.

**Rung 3 — AY-3-8913: adds an envelope generator.** The SSC's PSG, and
the most capable of the three — three tone channels and a noise
generator like the SN76489A, but each channel can *either* hold a fixed
volume *or* follow a single, shared **envelope generator**: a
hardware-automated volume-over-time shape (attack, decay, sustain-style
ramps, long before "ADSR" was a synth-plugin household term), so a
composer could get swelling or decaying notes without the CPU
re-writing a volume register every few milliseconds. The envelope is one
shared unit (the module doc: "all three channels that select envelope
mode... read the same `Envelope::volume`") driven by a 4-bit shape
register that a real chip decodes into one of ten distinct ramp shapes
(`crates/coco-core/src/ay8913/envelope.rs:54-65`):

```rust
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

`attack` decides whether the ramp counts up or down; `hold` decides
whether it sticks at the final level or repeats; `alternate` decides
whether it flips direction at each end (turning a sawtooth into a
triangle) — four shape bits combining into ten *effective* shapes because,
per the comment, "CONT=0 shapes... duplicate 4 of the CONT=1 ones," a
quirk of the real AY-3-8910's design that this emulator reproduces
exactly rather than "cleaning up." `volume()` — `(self.step as u8) ^
self.attack` — is the neat trick that makes both ramp directions share
one down-counter: XORing the (always-decreasing) `step` counter against
`attack` flips a falling ramp into a rising one when `attack` is set,
without needing a second counting direction in the hardware at all.

Position these three exactly as the syllabus does: **rungs of a ladder**,
not competing designs — the Orch-90 is "sound is entirely software's
job," the SN76489A is "give the CPU real oscillators and a noise source
to program instead," and the AY-3-8913 is "and automate volume-over-time
too, so the CPU can set a note going and walk away." Weeks 13–14 will
put real cartridges (the GMC, the SSC) around the SN76489A and AY-3-8913
respectively and drive them from actual 6809 code; this week's job was
only to show you where each chip's samples *enter* the pipeline you
already understand — `sound_levels()` for latched cartridge DACs (the
Orch-90, summed unconditionally, exactly as walked above), and
`audio_sample()`/`generator_sample()` for the mux-gated and
crystal-clocked generator paths `flush_line_audio` calls every grid slot
(§11.3).

---

## 11.12 Reading the tests

You've already read `sound.rs`'s three tests and `audio_grid.rs`'s three
tests inline, as evidence for specific claims (§11.1's mux table, §11.3's
event-timestamping). Step back and look at what each *file* is testing
as a whole, because the two files test different layers of the same
pipeline on purpose.

**`crates/coco-core/tests/sound.rs`** tests the **mux and the mix
function**, at the bus level, with no scanline loop involved —
`SystemBus::sound_probe` (§11.3's `mix` wrapper, meant for exactly this:
"an instantaneous speaker level, for tests and level meters") lets a test
poke PIA registers and immediately ask "what would the speaker hear right
now?" without running a single CPU cycle. This is the right layer to
prove §11.1's table: SNDEN gates everything, SEL picks the DAC/cassette/
cartridge branch, the beeper bypasses the gate entirely. Notice
`machine_collects_oversample_grid_frames_per_scanline`, the one test in
this file that *does* go through `Machine`/`run_field`, is doing
something different from the other two — it's not testing the mux table
at all, it's testing the **accounting**: exactly `lines_per_field ×
OVERSAMPLE` frames land in the buffer per field, and a second field
produces exactly the same count after a drain, proving `take_audio` truly
empties the buffer rather than leaving stragglers.

**`crates/coco-core/tests/audio_grid.rs`** tests the thing `sound_probe`
*can't*: **sub-scanline timing**. Its own module doc says so directly —
"sub-scanline DAC timing must land in the right grid slot, and a level
pulse entirely inside one scanline... must reach the grid" — and its
three tests are built directly around §11.2's aliasing scenario, made
concrete. `dac_write_mid_line_splits_the_grid_slots` writes the DAC ~30
cycles into a ~57-cycle line and asserts *exactly one* transition appears
across the four grid slots, at the right place — proof that events land
by cycle timestamp, not by arbitrary slot count.
`dac_pulse_within_one_line_reaches_the_grid` is the sharpest test in the
file and worth re-reading now that you understand the whole pipeline: it
raises the DAC, then drops it again, entirely within one scanline, and
asserts the *pulse itself* is audible on the grid even though the line's
**final** state (what a once-per-line point sampler would have seen) is
silent — this is §11.2's "sample once per frame loses everything"
failure mode, demonstrated as a passing test against the fix rather than
argued in prose. You confirmed this test's teeth yourself in this
chapter's own exercise (§11.14, exercise 3) by breaking the exact
mechanism that makes it pass and watching both `audio_grid` tests fail
with the precise diagnostic the sabotage predicts.

**`crates/coco-core/tests/orch90.rs`** is worth one honest note before
you run it yourself: six of its seven tests need nothing but a
zero-filled ROM image and pass in any checkout, including a bare clone
with no `roms/` directory. The seventh,
`orch90_autostarts_and_its_cart_code_drives_the_dacs`, boots the **real**
CoCo 3 ROM (`roms/coco3.rom`) to prove the Orch-90's CART*→FIRQ autostart
path actually runs cartridge code from a cold machine — and it fails
loudly, with a clear "cannot read .../roms/coco3.rom: No such file or
directory" message, in a worktree (like this course's) that doesn't have
`roms/` checked out. That's the intended behavior, not a bug in the test
— week 1's reading-assignment habit ("ROMs are local-only... tests that
need a ROM either skip or fail loudly") applies here exactly as
advertised, and this is your first chapter where you can see it happen
in practice rather than take it on faith.

---

## 11.13 Reading assignment

In this order:

1. **`crates/coco-core/src/audio.rs`, all of it** (115 lines) — the
   module doc's two-paragraph summary, `OVERSAMPLE` and the gain
   constants, `AudioInputs`, `AudioEvent`, and `mix`. Small enough to
   read start to finish in one sitting; everything else in this chapter
   builds on it.
2. **`crates/coco-core/src/bus/audio_bridge.rs`** — `snapshot_audio_inputs`,
   `note_audio_write`, `sound_probe`.
3. **`crates/coco-core/src/machine/audio.rs`** — `flush_line_audio`,
   `take_audio`, `audio_sample_rate`. Read it next to `machine/run.rs`'s
   `end_of_line` (`run.rs:130-172`) so you see exactly where in the
   per-line trailer it's called.
4. **`crates/coco-egui/src/audio.rs`, all of it** — the whole host chain
   in one file: `DcBlocker`, `LowPass`, `Resampler`, `AudioOutput`, and
   `push_samples`. Read the module doc first; it previews every artifact
   this chapter walked in two short paragraphs.
5. Run both test suites and watch them pass with nothing but a
   zero-filled ROM and no audio hardware required:

   ```
   cargo test -p coco-core --test sound --test audio_grid
   cargo test -p coco-egui audio::
   ```

   (The second command runs `coco-egui`'s unit tests filtered to the
   `audio` module — `crates/coco-egui/src/audio_test.rs`, gated in via
   `#[cfg(test)] #[path = "audio_test.rs"] mod tests;` at the bottom of
   `audio.rs`. It builds `cpal` and its platform audio backends, so
   expect a slower first compile than `coco-core`'s suites.)

---

## 11.14 Exercises

**11.1 — Derive the grid rate (recall + math).** Without looking back at
§11.4, recompute the audio grid's sample rate from first principles: you
need `VideoStandard::NTSC`'s `lines_per_field()` and `field_rate_hz()`
(week 6 gave you both; they're also in `crates/coco-core/src/config.rs`)
and `audio::OVERSAMPLE`. Show the two multiplications. Then do the same
for PAL (`lines_per_field() = 312`, `field_rate_hz() = 50.0`) — is PAL's
grid rate higher or lower than NTSC's, and does that match your intuition
about why (fewer fields per second, but how many more lines per field)?

**11.2 — Sabotage the event grid, verified (sabotage — run the actual
suite).** In `crates/coco-core/src/machine/audio.rs`, inside
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
`crates/coco-core/src/audio.rs`; (b) what would you *hear*, specifically,
at the moment a program first enables SNDEN or writes a new steady DAC
level after a period of silence — connect this to what §11.6 called "a
sudden voltage step." Don't run the code for this one; the point is
building the intuition without a scope or an ear on hand.

**11.4 — Change `OVERSAMPLE`, predicted then checked (build).** In
`crates/coco-core/src/audio.rs`, change `OVERSAMPLE` from `4` to `2`.
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
`crates/coco-egui/src/audio.rs` and answer, citing line numbers: (a)
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
`crates/coco-core/tests/sound.rs` (or a new test file) that: configures
the DAC path exactly like `dac_reaches_speaker_only_with_snden_and_mux_zero`'s
`bus()` helper, then alternates `PIA1_DA` between `0xFC` (full scale) and
`0x00` every 16 CPU cycles for several full field's worth of scanlines,
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

Week 12 stays inside the audio system but flips the direction: instead
of the CPU driving a speaker, the speaker (or rather, a cassette deck's
read head) drives the CPU — CSAVE and CLOAD encode and decode data as
audio tones entirely in software, and the emulator has to re-implement
the ROM's own FSK demodulator well enough to fool it. You already met
the cassette's *output* path in passing this week (`SEL_CASSETTE`
routing the tape's square wave through the same mux you now understand
completely); next week is where that square wave's timing — leader
bytes, sync bytes, the motor's spin-up delay — becomes the whole subject,
and where cycle-accurate timing (the fidelity table from week 1) turns
out to matter far more for a tape deck than it ever did for the DAC you
just spent a week mastering.
