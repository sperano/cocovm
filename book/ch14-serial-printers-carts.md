# Chapter 14 — Serial: Bit-Banging, a Real UART, and a Printer

*Week 14. Goal: climb three rungs of the same ladder — a software-timed GPIO
pin, a real hardware UART, and a protocol interpreter built on top of
either — and, along the way, finish two stories earlier chapters left
open. Chapter 6's `poll_cart_interrupt` and Chapter 10's PIA1 CB1 path both
terminate here, in the code that turns a cartridge's interrupt line into a
running 6809 program. This chapter closes Part V; an elective second half
covers the cartridge system that makes that termination possible in the
first place.*

---

Every subsystem in this course so far has had a chip to point at. The
video came from the GIME, the keyboard from a PIA, the sound from a DAC
and a PSG, the floppy from a WD1773. This week breaks that pattern twice
over. The first serial device in the chapter has no chip at all — the
"printer port" every CoCo shipped with is two pins on a PIA you already
know, and the entire serial protocol lives in a busy-wait loop inside the
BASIC ROM. The last device in the chapter has no CoCo hardware in it
whatsoever: a DMP-105 is a printer sitting at the far end of a cable,
interpreting bytes, and the only reason it belongs in an emulator is that
somebody has to decide what those bytes mean.

Between those two sits the one genuine UART of the course, a MOS 6551
riding a cartridge, doing in silicon exactly what the ROM's delay loop
does by counting cycles. Three devices, three fidelity budgets, one
theme: the further you get from a chip, the more the interesting work
moves into deciding *what to model at all*.

There is a second thread running underneath, and it is the one that
closes out Part V. Chapter 6 introduced a function called
`poll_cart_interrupt` and immediately deferred it — nothing in the
machine asserted the cartridge interrupt line yet, so there was nothing
to poll. Chapter 10 built PIA1's `set_c1` for horizontal and field sync and
noted, without elaborating, that a third source feeds the same input pin.
This chapter supplies both missing halves. By §14.4 you will have watched
a 6551's interrupt output travel through the cartridge connector, into a
PIA input, out of the PIA's FIRQ pin, and into the CPU; by §14.6 you will
have watched a game cartridge do the same thing with no chip at all, just
a wire tied to the CPU's clock.

The chapter is long, and the second half is explicitly elective. Read
§14.2 through §14.5 as one continuous argument about serial I/O; treat
§14.6 as the reference chapter on cartridges that the RS-232 pak makes
you want.

---

## 14.1 The serial ladder

"Serial port" is a phrase that hides three unrelated pieces of hardware
under one name, and this codebase implements all three at different
fidelity — not out of inconsistency, but because 1980s CoCo software used
them at different fidelity. It is worth having all three in view before
reading any of them, because each one is a rung above the last on the
same ladder: pin, then chip, then protocol.

1. **Bit-banging** (`bitbanger.rs`). The CoCo's "printer port" is not a
   UART chip at all — it's one output pin (PIA1 PA1) and one input pin
   (PIA1 PB0), and Color BASIC's ROM does the framing, timing, and
   handshaking entirely in software: a busy-wait loop toggling a GPIO pin
   at a rate it counts out in cycles. There is no hardware here to model
   except a PIA you already met in Chapter 10 — the "protocol" lives in ROM,
   and the emulator's job is to *decode what the software transmits*, the
   same relationship the cassette deck (Chapter 12) has to its FSK tones.
2. **A real UART** (`acia6551.rs`). The Tandy Deluxe RS-232 Program Pak
   plugs a genuine MOS 6551 ACIA into the cartridge port: hardware
   framing, a programmable baud-rate generator, modem control lines, and
   its own interrupt source. This is the chapter's first appearance of a
   chip that does in silicon what the bit-banger's ROM code does by
   counting cycles.
3. **A protocol stack on top of either transport** (`dmp105.rs`). A DMP-105
   printer doesn't care whether the bytes reached it over the bit-banger
   or (on other CoCo software) a real serial cable — it interprets a
   stream of control codes and ASCII into ink on paper. This is the
   chapter's example of a device that is *itself* built from a
   byte-stream seam plus a state machine, the same "state, loop, seam"
   shape Chapter 1 promised applies all the way down.

Run Chapter 1's three questions against each rung as you read, and the
chapter's structure falls out immediately. The bit-banger's state is a
half-assembled byte and a cycle count since the last edge; its loop is a
per-instruction tick; its seam is a trait with one required method that
takes a decoded byte. The 6551's state is four registers plus two frame
timers; its loop is the same per-instruction tick; its seam is a second
trait, this one facing the host operating system. The DMP-105's state is
a print head position and a dozen style flags; its loop is one byte at a
time through a dispatcher; its seam is a shared handle the frontend can
read paper out of. Same three answers, three times, at three completely
different distances from the silicon.

Two design threads also run underneath all three rungs, and both are
worth watching for deliberately rather than noticing in passing.

The first is that **timing is measured in CPU cycles, never in wall
time**. Rung 1's bit period and rung 2's baud generator are both
expressed as a count of cycles, and that single decision is why two
famous CoCo pokes need no special-case code anywhere in this chapter. The
CoCo 3's `$FFD9` high-speed poke doubles the CPU clock; BASIC's own `POKE
150,n` changes how many cycles the ROM's delay loop counts. Both change
the effective baud rate, and the decoder finds out about it the only way
it can — by counting the cycles that actually elapse. Chapter 12 built the
cassette deck on exactly this idea, and it recurs here unchanged.

The second is that **fidelity is a budget, spent unevenly**. Chapter 1's
table already flagged the punchline in one row: "Serial UART (6551):
byte-granular frames, not bit-serial." §14.3 is where you find out
precisely what that row costs, what it buys, and — the question Chapter 1
insisted matters more than the abstraction — who would notice. The answer
turns out to be nobody who ever bought software for this pak, which is
what makes the choice defensible rather than merely convenient.

The elective second half, §14.6, is a genuine tangent from serial I/O.
The cartridge port is its own subsystem with its own trait, its own enum,
and its own four-slot expansion box. It is grouped here for two reasons.
The CART* auto-start interrupt is the other half of the story
`poll_cart_interrupt` (Chapter 6) and PIA1's `set_c1` (Chapter 10) began, so
the two halves belong in one chapter. And the Deluxe RS-232 pak of rung 2
*is* a cartridge, so §14.4 has to borrow from §14.6 anyway. Reading
§14.3–14.4 before §14.6 will make the auto-start story land better; the
dependency runs in that direction, not the reverse.

---

## 14.2 Rung 1: the bit-banger — a GPIO pin and a stopwatch

### What the printer port actually is

There is no printer UART chip in a stock CoCo. What the machine has
instead is two pins of a PIA and a subroutine. Color BASIC's
`LLIST`/`LPRINT` driver bit-bangs RS-232-style serial timing on PIA1's
PA1 and PB0. PA1 (`$FF20` bit 1, DIN pin 4) is the output: a plain async
serial line, 1 start bit + 8 data bits LSB-first + 1 stop bit, no parity.
PB0 (`$FF22` bit 0, DIN pin 2) carries BUSY feedback from the printer,
with polarity 0=ready/1=busy.

If the vocabulary of async serial is new, the three terms worth pinning
down are *mark*, *space*, and *frame*. An idle line sits at mark, the
high level. A frame begins when the transmitter pulls the line to space
for exactly one bit time — the *start bit* — and the receiver, which
shares no clock with the transmitter, uses that falling edge as its only
timing reference for everything that follows. Eight data bits go out
least-significant first, and the frame closes with a *stop bit* back at
mark, which both signals the end and guarantees there is a fresh mark
level for the next start bit to fall from. Ten bit times, one byte, no
shared clock, no handshake. That is the entire protocol, and Color BASIC
implements it in a loop.

The module doc comment for `bitbanger.rs` states the consequence for the
emulator directly, and its framing of the problem is the one to hold onto
for the rest of this section:

```rust
/// Unlike the cassette deck (FSK tones demodulated by zero-crossing
/// threshold), the printer port is a plain async serial line: 1 start bit
/// (space) + 8 data bits (LSB-first) + 1 stop bit (mark), no parity
/// (`bitbanger-spec.md` "Framing"). [`BitBanger`] models the *receive* side
/// only — decoding what the ROM's bit-bang driver transmits on PA1 — since
/// that's the only direction a virtual printer needs.
```

Receive-only is a real simplification, and it is the right one: no CoCo
program reads bytes back through the printer port, so a transmit path
would be code with no caller. What remains is two constants naming the
two pins ([`crates/coco-core/src/bitbanger.rs:44-49`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs#L44-L49)):

```rust
pub const TX_PIN: u8 = 0x02;
pub const BUSY_PIN: u8 = 0x01;
```

Both are PIA1 bits Chapter 10 already gave you the vocabulary for — this
chapter adds no new hardware primitive, only a new *use* of one. The
entire physical layer of the CoCo's printer port, as far as this emulator
is concerned, is those two masks plus the question of what a receiver
watching PA1 would see.

### How PA1 reaches the decoder

That last question has a subtlety in it. The bus doesn't hand
`BitBanger::tick` a raw PIA register, because a PIA output register's
contents are only meaningful if the corresponding pin is configured as an
output in the first place. A receiver watching a pin the CPU has not yet
claimed sees whatever the pin floats to, not whatever bit pattern happens
to be sitting in a register the pin isn't connected to. So the bus
computes the line level explicitly
([`crates/coco-core/src/bus/pins.rs:85-99`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/pins.rs#L85-L99)):

```rust
/// PA1 ($FF20) as the bit-banger's TX line sees it: mark (idle-high)
/// unless PIA1 DDRA bit 1 is set to make PA1 an output and its output
/// register bit is clear (space). The ROM only ever drives PA1 once it
/// has configured it as an output (DDRA = $FE at `$A048` —
/// `bitbanger-spec.md` "Register map"); an unconfigured PA1 is treated
/// as idle mark, matching how a floating output pin would look to a
/// receiver expecting idle-high (not itself asserted in the spec, since
/// the ROM always configures DDRA before touching the printer port).
pub(crate) fn pia1_tx_mark(&self) -> bool {
    if self.pia1.a.ddr & bitbanger::TX_PIN == 0 {
        true
    } else {
        self.pia1.a.output & bitbanger::TX_PIN != 0
    }
}
```

This is exactly the kind of "read the DDR before trusting the output
register" logic Chapter 10 built into the PIA itself, reused at the call
site rather than duplicated, because a pin's DDR bit answers "is this
even driven" independently of whatever the output register holds. Note
the doc comment's honesty about the one part it had to decide rather than
look up: the spec never says what an unconfigured PA1 looks like, because
the ROM always configures DDRA first, so the "floating reads as mark"
choice is flagged as this codebase's reasoning rather than a hardware
fact. That flag matters more than it might seem — the alternative choice
(floating reads as space) would have the decoder see a permanent start
bit before the ROM ever touches the port.

### The RX state machine

`BitBanger::tick` is driven once per instruction from the machine loop.
You have already seen the call site, in Chapter 6's `step_cpu_unit`:
`self.bus.bitbanger.tick(cycles, self.bus.pia1_tx_mark());`. Each call
carries a cycle delta and PA1's level, and the contract is that the level
was held constant across that whole delta — which is true because the
CPU cannot change a PIA register in the middle of an instruction.

What the decoder does with that stream of (cycles, level) pairs is what a
real UART's receiver does: hunt for a falling edge, then take timed
samples relative to it. That is two states, and the enum says so
([`crates/coco-core/src/bitbanger.rs:226-237`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs#L226-L237)):

```rust
enum RxState {
    /// Idle at mark, hunting for the next mark→space edge (a start-bit
    /// candidate).
    Idle,
    /// Mid-frame: `elapsed` CPU cycles since the falling edge that started
    /// this frame, `sample` samples taken so far ([`START_SAMPLE`] = start
    /// validation, then 8 data bits LSB-first, then the stop-bit check),
    /// and the data bits assembled so far.
    Receiving { elapsed: u32, sample: u8, bits: u8 },
}
```

Notice that the mid-frame variant carries all three pieces of in-flight
state as payload rather than as fields on `BitBanger`. There is no way to
be halfway through a frame without also having an elapsed count and a
sample index, and no way to be idle and still have them; encoding that in
the enum makes the invalid combinations unrepresentable instead of merely
unlikely.

Here is the tick function itself
([`crates/coco-core/src/bitbanger.rs:391-449`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs#L391-L449)):

```rust
    pub fn tick(&mut self, cycles: u32, pa1_mark: bool) {
        self.state = match self.state {
            RxState::Idle => {
                if self.last_mark && !pa1_mark {
                    // Falling edge: mark -> space, a start-bit candidate.
                    RxState::Receiving {
                        elapsed: cycles,
                        sample: 0,
                        bits: 0,
                    }
                } else {
                    RxState::Idle
                }
            }
            RxState::Receiving {
                mut elapsed,
                mut sample,
                mut bits,
            } => {
                elapsed += cycles;
                let mut false_start = false;
                while sample < TOTAL_SAMPLES && elapsed >= self.sample_threshold(sample) {
                    if sample == START_SAMPLE {
                        if pa1_mark {
                            // Line back at mark mid-start-cell: the falling
                            // edge was a glitch, not a start bit. Abandon
                            // the frame silently (not a framing error — no
                            // frame ever began).
                            false_start = true;
                            break;
                        }
                    } else if sample <= DATA_BITS {
                        bits |= u8::from(pa1_mark) << (sample - 1);
                    } else if pa1_mark {
                        self.sink.write_byte(bits);
                        self.bytes_out += 1;
                    } else {
                        // Stop bit read space: framing error. Discard the
                        // byte and resync — go back to Idle and hunt for
                        // the next mark->space edge, rather than assuming
                        // the following bits are frame-aligned
                        // (`bitbanger-spec.md` "Decoder spec").
                        self.framing_errors += 1;
                    }
                    sample += 1;
                }
                if false_start || sample >= TOTAL_SAMPLES {
                    RxState::Idle
                } else {
                    RxState::Receiving {
                        elapsed,
                        sample,
                        bits,
                    }
                }
            }
        };
        self.last_mark = pa1_mark;
    }
```

Read the sample-index arithmetic against the frame layout. `START_SAMPLE
= 0` validates the start bit, samples 1 through 8 (`sample <= DATA_BITS`)
pull each data bit in with `bits |= u8::from(pa1_mark) << (sample - 1)`,
and sample 9 checks the stop bit. A stop bit that reads mark delivers the
assembled byte to the sink and bumps `bytes_out`, a monotonic counter the
frontend's status bar reads to light its printer activity icon (Chapter
15). Notice the `sample - 1`: data-bit
sample 1 lands in bit position 0, **LSB first**, matching the framing
spec. `TOTAL_SAMPLES = DATA_BITS + 2 = 10`, which is the same ten bit
times the frame layout describes, one sample each.

The `while` loop is worth a second look, because at first glance a
per-instruction tick could never need to take more than one sample. Most
of the time it doesn't: a bit cell at the default rate is nearly fifteen
hundred cycles wide and a 6809 instruction is a handful of cycles, so the
common case is that `elapsed` creeps past one threshold every few hundred
ticks. But the loop is not an optimization; it's a correctness
requirement. The tick's `cycles` argument is whatever the last
instruction cost, and nothing in the contract bounds it below a bit
period — a `CWAI`, a long indexed instruction, or any future caller that
batches cycles can hand over a delta that crosses two thresholds at
once. Writing the catch-up as a loop means the decoder's behavior depends
only on how much time has passed, never on how that time happened to be
chopped up by the caller. That property is exactly what lets the same
decoder handle both a per-instruction drip and a test harness feeding it
in deliberate chunks.

### Why mid-cell, not edge-aligned

Every sample happens at the *middle* of its bit cell, not at its edge.
`sample_threshold` computes `(0.5 + k)` bit-times for sample `k`
([`crates/coco-core/src/bitbanger.rs:451-458`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs#L451-L458)):

```rust
    /// CPU-cycle offset of sample `sample` (0-indexed) after the start-bit
    /// edge: sample times are 0.5 (start validation), 1.5, …, 8.5 (data),
    /// 9.5 (stop) bit-times (`bitbanger-spec.md` "Decoder spec"), so sample
    /// `k` sits at `(0.5 + k)` bit-times = `bit_period * (2k + 1) / 2`.
    fn sample_threshold(&self, sample: u8) -> u32 {
        let scaled = u64::from(self.bit_period) * (2 * u64::from(sample) + 1);
        (scaled / 2) as u32
    }
```

The reason is the same one that makes a real UART oversample its input,
typically sixteen times per bit cell, and sample near the theoretical
center. Transmitter and receiver run off independent clocks with no
shared reference. Their idea of where a bit cell starts drifts apart the
longer a frame runs, and nothing resynchronizes them until the next start
edge. Sampling at the *edge* of a cell means the smallest clock
disagreement flips you onto the wrong side of a transition, so you read
the neighboring bit. Sampling at the *center* buys a full half bit-time
of slack in either direction before that can happen — and the slack is
needed most exactly where the design puts it, at the *last* bit of a
ten-bit frame, where drift has had the longest time to accumulate.

Two tests measure that margin rather than asserting it
([`crates/coco-core/src/bitbanger_test.rs:101-121`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger_test.rs#L101-L121)).
Each builds a waveform at a bit period 2% away from the decoder's
configured rate — one fast, one slow — and confirms the byte still
decodes with zero framing errors. The doc comment does the arithmetic
that explains why 2% is a comfortable rather than a lucky number: "by bit
9 (9.5 bit-times in) accumulated drift is under 0.19 bit-times, well
inside the 0.5 bit-time margin each mid-cell sample has." That is the
whole design justified in one sentence, with the number that makes it
true.

The same mechanism doubles as glitch rejection, which is the other half
of what `START_SAMPLE` is for. Its job is specifically to catch a falling
edge that *isn't* a real start bit: if the line is already back at mark
by the half-bit-time sample point, the edge that triggered `Receiving`
was too short to be a genuine transmission, so the frame is abandoned
silently — "not a framing error, since no frame ever began." The doc
comment names a concrete real-world trigger rather than a hypothetical
one: without the check, "the ~30-cycle low pulse PA1 emits while the
ROM's boot code reconfigures DDRA (`$A02F`) free-runs into a phantom
0xFF." Real boot code, real glitch, real phantom byte on the paper before
the machine has even reached the `OK` prompt. The test
`sub_bit_glitch_is_rejected_as_false_start` reproduces it: a 30-cycle
glitch against a ~1486-cycle bit period — about 2% of one cell — followed
by two full bit periods of mark. The assertions are that nothing decoded
and no framing error was counted, and then that a real byte sent
afterward still decodes cleanly.

### When the stop bit reads space

The stop-bit check is the decoder's one genuine error path, and what it
does *after* detecting an error matters as much as the detection. A
framing error means the receiver's idea of where the frame boundaries
are has come apart from the transmitter's — the bits it just assembled
are not a byte anyone sent. The code's comment spells out the policy:

```rust
                    } else {
                        // Stop bit read space: framing error. Discard the
                        // byte and resync — go back to Idle and hunt for
                        // the next mark->space edge, rather than assuming
                        // the following bits are frame-aligned
                        // (`bitbanger-spec.md` "Decoder spec").
                        self.framing_errors += 1;
                    }
```

Discard, don't deliver; resync, don't guess. The tempting alternative —
hand the bits to the sink anyway and let the printer sort it out — would
turn one lost edge into a stream of plausible-looking garbage, because
each subsequent frame would be sampled at whatever offset the bad frame
left behind. Returning to `Idle` throws away the corrupt byte and waits
for the next honest falling edge, which is the only event in the protocol
that carries reliable timing information. The test states both halves as
one fact ([`crates/coco-core/src/bitbanger_test.rs:123-141`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger_test.rs#L123-L141)):

```rust
/// A stop bit that reads space is a framing error: counted, the byte
/// discarded (never reaches the sink), and the decoder resyncs by
/// going back to hunting for the next mark->space edge rather than
/// assuming the following bits are frame-aligned. A valid byte sent
/// afterward, starting from a fresh edge, must still decode correctly.
#[test]
fn framing_error_counts_and_resyncs() {
    let capture = CaptureSink::new();
    let mut bb = BitBanger::new();
    bb.set_sink(Box::new(capture.clone()));

    feed_bad_frame(&mut bb, 0x2A, DEFAULT_BIT_PERIOD, TICK_SIZE);
    assert_eq!(bb.framing_errors(), 1);
    assert!(capture.bytes().is_empty());

    feed_byte(&mut bb, 0x2A, DEFAULT_BIT_PERIOD, TICK_SIZE);
    assert_eq!(capture.bytes(), vec![0x2A]);
    assert_eq!(bb.framing_errors(), 1);
}
```

The three assertions after the bad frame are the interesting ones: the
error is counted, the byte is *not* in the capture buffer, and — after a
good frame — the error count has not moved again. A decoder that
half-recovered would fail the third assertion, not the first.

### The bit period: 78 + 16×N, and why N = 88 means 600 baud

`BitBanger` doesn't hardcode "600 baud" anywhere. It stores a bit period
in CPU cycles, settable at runtime, because that is what the real machine
stores too — `POKE 150,n` writes a ROM variable, not a baud rate:

```rust
pub const DEFAULT_BIT_PERIOD: u32 = 78 + 16 * 88;
```

The formula and the constant `88` both come from an actual ROM
disassembly rather than a datasheet. This project's `docs/` directory
holds copyrighted reference material and isn't part of the public
repository, but the finding it records is this. The bit-bang delay loop
is a self-referential `BSR` that runs the countdown *twice* per bit, so
`cycles_per_bit = 78 + 16×N`, where `N` is the live 16-bit value of the
ROM variable `LPTBTD` at `$0095`/`$0096` — decimal 149/150, which is
where `POKE 150,n` gets its address. The default N the ROM actually
initializes at boot is `88` (`$0058`).

That number is worth a paragraph, because it is a small lesson in
sourcing. The CoCo 3 Service Manual's Table 2 prints `87`, not `88`, and
the spec's own cross-check identifies the printed value as a stale
pre-1.2 constant; the reference to Color BASIC Unravelled confirms the
87→88 change happened at Color BASIC 1.2. When a manual and the ROM
bytes disagree, the ROM bytes are what the machine executes. This is the
same instinct Chapter 1 asked for when it said the tests are the
specification — prefer the artifact that runs over the document that
describes it.

The full measured table, straight from the ROM trace:

| Baud label | N | cycles/bit (78 + 16N) | Effective @ 0.894886 MHz |
|---|---|---|---|
| 120 | 458 | 7406 | 120.9 |
| 300 | 180 | 2958 | 302.5 |
| **600 (default)** | **88** | **1486** | **602.2** |
| 1200 | 41 | 734 | 1219.2 |
| 2400 | 18 | 366 | 2445.6 |

Verify the default row by hand: `78 + 16×88 = 78 + 1408 = 1486`, exactly
the source constant. Baud is `CPU_HZ / cycles_per_bit`: `894886 / 1486 =
602.15…`, close enough to nominal 600 baud that "600 baud" is the label
everyone uses, even though the true rate running on real silicon is
602.2. None of the five rows is exact, and none of them needs to be — an
async receiver only has to agree with the transmitter to within a
fraction of a bit cell, which is the margin the previous section
measured. The chapter's baud-arithmetic exercise, §14.8.1, asks you to
redo this for a different `N`.

Because the emulator counts *CPU cycles* rather than wall-clock
milliseconds, two behaviors fall out with no special-case code at all.
`POKE 150,n` becomes a call to `BitBanger::set_bit_period` with a
recomputed `78 + 16*n`, and the decoder neither knows nor cares that
BASIC changed its mind about the rate mid-session. And the `$FFD9`
high-speed poke doubles the CPU clock without touching the ROM's
cycle-counted delay loop, so it *exactly* doubles the effective baud —
the loop still counts the same number of now-faster cycles. Test
`double_rate_bit_period_decodes`
([`crates/coco-core/src/bitbanger_test.rs:172-180`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger_test.rs#L172-L180))
configures the decoder at half the default period and confirms a byte
sent at that rate still decodes, proving the relationship is pure
arithmetic with "no special-cased fast mode."

### A driver that doesn't play along: NitrOS-9

Not every piece of CoCo software shares Color BASIC's obliviousness to
clock speed, and the codebase found this out the hard way. The module doc
comment of [`tests/bitbanger_os9.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bitbanger_os9.rs)
records an empirical finding arrived at by instrumenting the decoder's
raw edge timings during a real NitrOS-9 boot, rather than by
disassembling the OS-9 driver:

```rust
//! ## Bit rate: NitrOS-9 is not Color BASIC's 600-baud constant
//!
//! EOU boots the CoCo 3 GIME straight into high-speed mode (`$FFD9`,
//! `GIME::cpu_fast == true` — confirmed live, not assumed) and its `/p`
//! driver does **not** behave like Color BASIC's, which busy-waits a fixed
//! cycle count that the speed poke exactly doubles the effective baud of...
//! Direct instrumentation of `BitBanger::tick`'s raw PA1 edge intervals
//! during a live boot ... found the bit-cell quantum is **twice**
//! [`bitbanger::DEFAULT_BIT_PERIOD`] (1486 cycles): NitrOS-9's driver holds
//! true wall-clock baud at 600 regardless of `cpu_fast` by doubling its own
//! delay-loop cycle count to compensate for the doubled clock, the opposite
//! of BASIC's speed-oblivious driver.
```

Two independently written 6809 device drivers adopted opposite policies
on the same hardware fact. Color BASIC's printer driver counts a fixed
number of *cycles* per bit and lets the machine's clock speed decide what
that means in wall-clock time. NitrOS-9's `/p` driver detects the doubled
clock and doubles its own delay-loop count to compensate, holding *true*
baud constant at 600 regardless of speed. Neither is wrong; they simply
disagree about whether "600 baud" is a promise to the printer or a
property of the loop.

The emulator's decoder encodes neither policy, and that is the point
worth taking away. It counts whatever cycles actually elapse between
edges, which is all a physical receiver could do, so it decodes both
drivers without being told which one is talking. The test
([`crates/coco-core/tests/bitbanger_os9.rs:204-237`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bitbanger_os9.rs#L204-L237))
boots the real EOU disk images to a shell, runs `echo hello >/p`,
retunes the decoder to `2 * DEFAULT_BIT_PERIOD`, and asserts the captured
bytes are exactly `b"hello \r"` with zero framing errors — a second,
independently written driver validating the same decoder that Color
BASIC's `LLIST` exercises.

### BUSY feedback: what's real, what's a stub

PB0 carries BUSY back from the (virtual) printer, and `BitBanger` exposes
it as a plain settable bit:

```rust
pub fn busy(&self) -> bool { self.busy }
pub fn set_busy(&mut self, busy: bool) { self.busy = busy; }
```

What this buys today is worth being explicit about, because it is the
chapter's first example of a wire that is fully implemented and fully
unused. Nothing in this codebase's `DMP105` sink calls `set_busy` — the
module doc comment says so plainly, calling out "a (currently
unimplemented) DMP-105 buffer model," and the reason is a gap in the
source material rather than a gap in the code. BUSY's *assertion
granularity* — does the real printer's 134-character receive buffer
assert BUSY after every byte? only when nearly full? — is marked INFERRED
rather than VERIFIED in `dmp105-protocol.md`, because the manual
documents the polarity and the existence of a 134-character buffer but
never states the byte-count trigger.

So the wire is real and tested — three bus-level tests in
[`tests/bitbanger.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bitbanger.rs)
confirm PB0 reads 0 by default and reflects `set_busy` — but no code path
drives it from print volume. A virtual printer in this emulator is always
"ready." Ask Chapter 1's question about it: who would notice? Only a program
that depended on flow control the emulator never needs to apply, and an
infinite-speed printer never needs to say "slow down." Guessing at the
trigger threshold would produce a number that looks authoritative in the
source and isn't; leaving the setter unused leaves the honest shape of
the hardware in place for the day the fact turns up.

### Where the decoded bytes go: the sink family

The decoder's whole output is a single call, `self.sink.write_byte(bits)`,
and everything interesting about what happens to a printed byte lives on
the other side of it. That seam is a trait with one required method and
three defaulted ones
([`crates/coco-core/src/bitbanger.rs:81-90`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs#L81-L90)):

```rust
pub trait PrinterSink {
    fn write_byte(&mut self, b: u8);

    /// Snapshot this sink's state for serialization (see the `sink_serde`
    /// module below) — the default, kept by every sink with no state worth
    /// carrying across a save-state (`NoopSink`, [`CaptureSink`]), is
    /// [`sink_serde::SinkState::Noop`].
    fn snapshot(&self) -> sink_serde::SinkState {
        sink_serde::SinkState::Noop
    }
```

The two further defaults, `as_dmp105` and
`was_file_capture_stopped_by_restore`, both return "no" and both exist
for the save-state machinery discussed below. The shape is the one Chapter 1
praised in the `Bus` trait: one method every implementor must write, and
a set of defaults that let a trivial sink stay trivial. `NoopSink`, the
sink installed until something more interesting is plugged in, is
therefore three lines, and its `write_byte` body is empty.

Four sinks ship in the crate. `NoopSink` discards. `CaptureSink` appends
to a shared buffer and is what the tests use. `DMP105Handle` — §14.5's
whole subject — feeds a printer interpreter. And `FileSink` is the "print
to a text file" implementation behind the CLI's `--print-capture` flag
and the GUI's Machine menu, which is worth reading because its
`write_byte` makes two decisions a naive version wouldn't
([`crates/coco-core/src/bitbanger.rs:198-214`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs#L198-L214)):

```rust
    fn write_byte(&mut self, b: u8) {
        let b = if self.translate_cr_to_lf && b == b'\r' {
            b'\n'
        } else {
            b
        };
        // Best-effort: a full disk or a revoked permission has no useful
        // recovery path from inside the decoder's per-instruction tick, and
        // the alternative (propagating an error out of
        // `PrinterSink::write_byte`) would infect the hot CPU loop with I/O
        // error handling for a side-channel that was never guaranteed to
        // succeed on real hardware either (a jammed printer just eats bytes).
        let _ = self.file.write_all(&[b]);
        if b == b'\r' || b == b'\n' {
            let _ = self.file.flush();
        }
    }
```

The first decision is the optional CR-to-LF rewrite. BASIC's line ending
is a bare CR (`$0D`), so a faithful capture of an `LLIST` reads back
exactly as the ROM sent it and looks like one enormous line in most host
text editors. The translation is offered as a convenience mode, fixed for
the lifetime of the capture session rather than toggleable mid-file, and
the sink's doc comment notes why the naive byte swap is sufficient: a
CoCo never sends CRLF pairs, so there is no pair to collapse.

The second decision is the pair of discarded `Result`s, and the comment
explains rather than apologizes. `write_byte` has no error channel, and
giving it one would push I/O error handling into a function called once
per printed character from inside the CPU loop. The justification that
makes it more than laziness is the last clause: a jammed printer on real
hardware also just eats bytes. The emulator's failure mode matches the
hardware's failure mode, which is a better argument than "errors here are
unlikely." Note also the flush on every line ending rather than on every
byte — one `write` syscall per character would be absurd, but a capture
file nobody can `tail` until the emulator exits would be useless, and
line granularity is exactly the shape a printer's output naturally has.

> **Rust corner: the `Rc<RefCell<_>>` handle pattern for pluggable
> sinks.** `BitBanger` owns its destination as `Box<dyn PrinterSink>`, so
> once `set_sink` moves a sink in, the caller no longer has a path to it.
> That is fine for `FileSink`, whose output goes somewhere the caller can
> reach anyway, and fatal for a sink whose whole purpose is to accumulate
> something the caller wants to read back. `CaptureSink`'s one-line
> definition is how that is solved:
>
> ```rust
> #[derive(Clone, Default)]
> pub struct CaptureSink(Rc<RefCell<Vec<u8>>>);
> ```
>
> Why not a plain `Vec<u8>`? Because the `Vec` would be locked inside a
> trait object with no getter. `Rc<RefCell<Vec<u8>>>` fixes this by
> splitting *ownership* (shared, via `Rc`) from *access* (checked at run
> time, via `RefCell`): clone the handle before handing one half to
> `set_sink`, keep the other half, and both names refer to the same
> buffer. Every test in `bitbanger_test.rs` opens with exactly that
> two-step, and `DMP105Handle` (§14.5) is the same pattern one level
> richer — a shared handle to a whole interpreter rather than to a `Vec`.
>
> Contrast this with Chapter 11's cross-thread audio ring buffer,
> `Arc<Mutex<VecDeque<_>>>`. Both are "shared ownership plus a way to
> mutate through a shared reference," but `BitBanger`, its sink, and the
> frontend code that later reads a `DMP105Handle`'s paper all run on the
> *same* thread — the emulator core has no background thread of its own —
> so the atomic reference counting and OS-level locking that `Arc` and
> `Mutex` pay for buy nothing here. `Rc` and `RefCell` do the identical
> job at a fraction of the cost, because `Rc`'s count is a plain
> non-atomic integer and `RefCell`'s borrow check is a runtime comparison
> rather than a kernel-mediated lock. The rule of thumb: reach for
> `Arc`/`Mutex` only once you actually cross a thread boundary, as week
> 11's audio callback genuinely does; everywhere else in this
> single-threaded core, `Rc`/`RefCell` is the right and cheaper tool.
>
> One clarification, because Chapter 1 spent an entire section warning
> against exactly these types. This is *not* the architecture it warned
> about. `Machine` and `SystemBus` remain a plain owned tree with zero
> `Rc`/`RefCell` anywhere in them, which is what makes save states
> trivial. The pattern appears at this one narrow seam, where a value must
> legitimately be reachable from two independent owners — the bus's sink
> slot and the frontend's paper-window state — for reasons that have
> nothing to do with the core machine's own structure. Chapter 1's "one
> honest caveat" section named this exact spot in advance.

> **Rust corner: serializing a trait object that cannot be
> serialized.** `BitBanger` derives `Serialize`/`Deserialize`, and one of
> its fields is a `Box<dyn PrinterSink>`. Those two facts should not be
> compatible: `serde` needs to know at compile time every concrete type
> that could be behind a pointer, and a public trait implementable by
> `coco-egui` is by construction an open set. The field's attribute is
> where the contradiction gets resolved — `#[serde(with = "sink_serde")]`
> — and the module it names explains itself
> ([`crates/coco-core/src/bitbanger.rs:461-469`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs#L461-L469)):
>
> ```rust
> /// `#[serde(with = "sink_serde")]` for [`BitBanger::sink`]: the trait object
> /// itself isn't `Serialize`/`Deserialize` (and shouldn't be — a serialized
> /// `Box<dyn PrinterSink>` would either need typetag machinery for a
> /// two-implementation seam or leak host file handles into the snapshot), so
> /// this maps it to and from the small [`SinkState`] enum instead
> /// (`docs/plan-save-states.md`).
> ```
>
> The trick is to serialize not the sink but a *description* of it, and
> the description is a three-variant enum:
>
> ```rust
> pub enum SinkState {
>     Noop,
>     FileCapture,
>     DMP105(DMP105),
> }
> ```
>
> That is the `snapshot` default method earning its place in the trait:
> every sink declares what kind of sink it is and what part of it is
> worth keeping. A `NoopSink` and a `CaptureSink` answer `Noop`, a
> `FileSink` answers `FileCapture`, and a `DMP105Handle` answers with a
> clone of the entire interpreter and its paper. Restoring inverts it
> ([`crates/coco-core/src/bitbanger.rs:502-512`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs#L502-L512)):
>
> ```rust
>     pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
>         deserializer: D,
>     ) -> Result<Box<dyn PrinterSink>, D::Error> {
>         Ok(match SinkState::deserialize(deserializer)? {
>             SinkState::Noop => Box::new(NoopSink),
>             // Distinct from `SinkState::Noop`, even though both currently
>             // behave identically: see `StoppedFileCaptureSink`'s doc comment.
>             SinkState::FileCapture => Box::new(super::StoppedFileCaptureSink),
>             SinkState::DMP105(state) => Box::new(DMP105Handle::from_state(state)),
>         })
>     }
> ```
>
> `FileCapture` restores to `StoppedFileCaptureSink` rather than to
> `NoopSink` even though the two behave identically, and the distinction
> is the interesting part. An open host file handle cannot survive a
> snapshot, so a restored capture is simply stopped — but the frontend
> needs to *say so*, and "capture was running and is now stopped" is a
> different message from "capture was never running." A distinct type,
> answering `true` to one defaulted trait method, carries that one bit
> across the restore boundary without inventing a state field for it.
>
> There is one more Rust detail hiding in the same module, in the
> serializer's signature. It takes `&Box<dyn PrinterSink>` rather than the
> `&dyn PrinterSink` a reviewer would ask for, and clippy has a lint
> saying so, silenced with an explanation:
>
> ```rust
>     // `&Box<dyn PrinterSink>`, not `&dyn PrinterSink`: this is what the
>     // `#[serde(with = "sink_serde")]` codegen actually calls with (the
>     // field's declared type is `Box<dyn PrinterSink>`) — `&Box<T> -> &dyn
>     // Trait` isn't a coercion rustc applies at a plain call site, only at
>     // method-call receiver position, so narrowing the parameter here would
>     // fail to compile.
>     #[allow(clippy::borrowed_box)]
> ```
>
> This is the shape of a good `#[allow]`: not "the lint is annoying" but
> "here is the specific language rule that makes the lint's advice
> inapplicable here." Rust will coerce a `&Box<T>` to a `&dyn Trait` when
> it is the receiver of a method call, which is why `sink.snapshot()`
> works on the very next line, but not when it is an ordinary argument to
> an ordinary function — and the caller in this case is generated code
> that cannot be edited to insert the coercion itself.

---

## 14.3 Rung 2: the 6551 ACIA — a real UART

### Why a second serial device at all

The bit-banger models what stock BASIC already does with zero extra
hardware, and its limits are the limits of a CPU doing signal work in a
delay loop: one direction, no error detection worth the name, and a
transmitter that owns the processor for the whole duration of every byte.
The Tandy Deluxe RS-232 Program Pak (26-2226) is the answer people bought
when they wanted the machine to talk to a modem instead of a printer. It
plugs an actual MOS 6551 ACIA — Asynchronous Communications Interface
Adapter — into the cartridge port. The module doc is careful to note that
this is "not a Motorola MC-prefixed part," since so much of this
codebase's other silicon is Motorola.

A 6551 does in silicon what the printer driver does in a busy-wait loop,
and then does three things the busy-wait loop cannot do at all. It frames
bytes with a real shift register, so the CPU writes a byte and walks
away. It derives its baud rate from its own crystal rather than from the
CPU clock, so the speed poke cannot retune it. And it has genuine
modem-control inputs, DCD and DSR, with their own interrupt sources — the
first device in this chapter that can interrupt the CPU on its own
initiative rather than waiting to be polled. That last capability is what
makes §14.4 possible, and it is the reason this section comes before the
cartridge material rather than after it.

### Register bitmaps

Four registers live at offsets 0 through 3, which land at `$FF68`–`$FF6B`
once you know where the pak decodes them (§14.4). Their bit assignments
are transcribed as three modules of named constants — the house style
Chapter 2 established for the condition-code register and Chapter 8 reused for
the GIME:

```rust
pub mod status {
    pub const PARITY_ERROR: u8 = 0x01;
    pub const FRAMING_ERROR: u8 = 0x02;
    pub const OVERRUN: u8 = 0x04;
    pub const RDRF: u8 = 0x08;   // Receive Data Register Full
    pub const TDRE: u8 = 0x10;   // Transmit Data Register Empty
    pub const DCD: u8 = 0x20;
    pub const DSR: u8 = 0x40;
    pub const IRQ: u8 = 0x80;
}

pub mod command {
    pub const DTR: u8 = 0x01;
    pub const RX_IRQ_DISABLE: u8 = 0x02;   // inverted sense!
    pub const TX_CONTROL_MASK: u8 = 0x0C;
    pub const TX_CONTROL_SHIFT: u8 = 2;
    pub const ECHO: u8 = 0x10;
    pub const PARITY_MASK: u8 = 0xE0;
    pub const PARITY_SHIFT: u8 = 5;
}

pub mod control {
    pub const BAUD_MASK: u8 = 0x0F;
    pub const RX_CLOCK_SOURCE: u8 = 0x10;
    pub const WORD_LENGTH_MASK: u8 = 0x60;
    pub const WORD_LENGTH_SHIFT: u8 = 5;
    pub const STOP_BITS_2: u8 = 0x80;
}
```

Most of that is self-explanatory once you know a UART, but two bits are
easy to misread and both have bitten implementers before. The first is
`command::RX_IRQ_DISABLE`, whose sense is *inverted*: clear means the
receive interrupt is enabled, set means disabled. The doc comment calls
this out explicitly — "0 = rx-IRQ enabled, 1 = disabled" — because the
power-on state of the whole command register is zero, which therefore
means "receive interrupts armed" rather than the "everything off" a
zeroed register usually implies.

The second is `tx_control`, the two bits under `TX_CONTROL_MASK`, which
look like a pair of independent flags and are in fact a four-value enum:

```rust
pub mod tx_control {
    pub const RTS_OFF: u8 = 0;       // RTS off, no TDRE IRQ (tx still runs)
    pub const IRQ_ENABLED: u8 = 1;   // tx on, RTS on, TDRE IRQ enabled
    pub const RTS_ON: u8 = 2;        // tx on, RTS on, TDRE IRQ disabled
    pub const BREAK: u8 = 3;         // forced BREAK; TDR never consumed
}
```

That table is where the register bitmap stops being a transcription and
starts encoding real 6551 behavior. Only value `1` ever arms the TDRE
interrupt, and only value `3` — BREAK — actually stops the transmitter.
Value `0`, the one whose name reads like "off," still transmits; it only
drops the RTS output and the interrupt. A reader who assumed the field
was "transmitter enable plus interrupt enable" would get two of the four
cases wrong. The unit test that pins this down is
`consume_at_start_fires_irq_only_when_tx_irq_enabled`: it writes `DTR`
alone, which leaves the transmitter-control field at `RTS_OFF`, confirms
that writing a byte fires no interrupt, then writes
`DTR | (IRQ_ENABLED << TX_CONTROL_SHIFT)` and confirms that one does.

### Four addresses, and what they mean in each direction

The four registers are not four storage locations. Two of the offsets
mean entirely different things depending on whether the CPU is reading or
writing them, which is a hardware convention 6809 programmers know well
but which is easy to lose when transcribing a datasheet into a struct.
`ACIA6551::read` and `ACIA6551::write` are therefore two separate `match`
statements over the same 0–3 range rather than one accessor pair, and the
interesting arms are the side-effecting ones
([`crates/coco-core/src/acia6551/registers.rs:8-28`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551/registers.rs#L8-L28)):

```rust
    /// RDR read: returns the byte, then clears RDRF and all three error
    /// bits together (MAME `mos6551.cpp` `read_receive_data_register` — does
    /// *not* touch the IRQ output; that needs a status read or disabling
    /// the RDRF IRQ source via a command write).
    pub(super) fn read_rdr(&mut self) -> u8 {
        let val = self.rdr;
        self.status &=
            !(status::RDRF | status::PARITY_ERROR | status::FRAMING_ERROR | status::OVERRUN);
        val
    }

    /// Status read: returns the pre-clear snapshot, then (side effect)
    /// clears every armed IRQ source at once and drops the IRQ output bit.
    /// Does not touch parity/framing/overrun/RDRF/DCD/DSR (MAME
    /// `mos6551.cpp` `read_status_register`).
    pub(super) fn read_status(&mut self) -> u8 {
        let val = self.status;
        self.irq_sources = 0;
        self.update_irq_output();
        val
    }
```

Two reads, two different clear sets, and neither is a superset of the
other. Reading the data register clears "there is a byte waiting" and the
three error flags that describe *that* byte, but leaves the chip's
interrupt output asserted. Reading the status register clears the
interrupt output and every armed source, but leaves RDRF and the error
bits exactly where they were. A driver that reads only one of the two
gets a chip that is half-acknowledged, and §14.4's integration test walks
that exact two-step in a running machine.

Offset 1 is the more surprising one, because writing it is not a write at
all ([`crates/coco-core/src/acia6551/registers.rs:61-82`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551/registers.rs#L61-L82)):

```rust
    /// Programmed reset (any write to reg 1, value ignored): clears *only*
    /// the overrun status bit and *only* the DCD/DSR IRQ-source bits — the
    /// RDRF/TDRE IRQ sources deliberately survive, unlike the general
    /// command-write rule in [`ACIA6551::write_command`] (verified MAME
    /// `mos6551.cpp` `write_status_command_register` behavior: this is a
    /// narrower reset than a full command-register disable would produce).
    /// Command bits 0-4 are cleared (DTR off, rx-IRQ enabled, transmitter
    /// control = RTS_OFF, echo off); parity bits 7:5 survive. Control is
    /// untouched.
    pub(super) fn programmed_reset(&mut self) {
        self.status &= !status::OVERRUN;
        self.irq_sources &= !(irq_source::DCD | irq_source::DSR);
        self.update_irq_output();

        const RESET_MASK: u8 = command::DTR
            | command::RX_IRQ_DISABLE
            | command::TX_CONTROL_MASK
            | command::ECHO;
        self.command &= !RESET_MASK;
        self.rts = false;
    }
```

The status register is read-only, so the address does double duty: a
write to it, of any value, is the chip's software reset command. What
makes this worth quoting rather than mentioning is how *narrow* the reset
is, and how carefully the doc comment fences off what it does not do. It
clears overrun but not the other error bits. It clears the DCD and DSR
interrupt sources but deliberately leaves the receive and transmit ones
armed. It wipes the low five command bits but preserves the parity field
in bits 7:5, and never touches the control register at all. Every one of
those asymmetries is a fact checked against MAME rather than inferred
from what a reset "should" do, and the comment's parenthetical — "this is
a narrower reset than a full command-register disable would produce" —
exists precisely because the plausible guess is wrong.

### Baud math from a crystal, not a table lookup

The 6551 does not remember "1200 baud" as a concept. It divides its
reference crystal by a fixed factor and then by a per-index divider, and
*that* result happens to equal 1200. The crystal is standard across the
whole 6551 family and is not affected by anything the CoCo does to its
own clock:

```rust
const ACIA_CRYSTAL_HZ: u64 = 1_843_200;      // 1.8432 MHz
const BAUD_CLOCK_DIVISOR: u64 = 16;
const BAUD_DIVIDER: [u32; 16] = [
    1, 2304, 1536, 1048, 856, 768, 384, 192, 96, 64, 48, 32, 24, 16, 12, 6,
];
```

The rule is `baud = ACIA_CRYSTAL_HZ / 16 / BAUD_DIVIDER[index]`. Work
four of the sixteen indices by hand, because, unlike the bit-banger's
table, the numbers land exactly:

| Baud | Index | Divider | `1843200 / 16 / divider` |
|---|---|---|---|
| 300 | 6 | 384 | `1843200 / 16 / 384 = 300` |
| 1200 | 8 | 96 | `1843200 / 16 / 96 = 1200` |
| 9600 | 14 | 12 | `1843200 / 16 / 12 = 9600` |
| 19200 | 15 | 6 | `1843200 / 16 / 6 = 19200` |

That exactness is the entire reason 1.8432 MHz is such a strange-looking
number to find on a circuit board. It was chosen specifically because
dividing it by 16 and then by small integers lands on every standard
RS-232 baud rate with zero rounding error, which is why 6551s, 8250s, and
16550s across the whole microcomputer era all used the same crystal
value. Compare that against §14.2's table, where every row is off by a
fraction of a percent because the bit period is derived from a clock
chosen for NTSC television rather than for serial communication. Two
devices, two clock lineages, and the difference shows up in the fourth
significant figure.

One index is not a real divider at all. Index 0 nominally selects an
external clock source, which this model has no way to supply; MAME's own
table puts a divider of `1` there, and this codebase does the same rather
than inventing external-clock behavior it cannot verify. It is a small
example of the sourcing discipline the whole module follows — where the
reference implementation punts, this one punts identically and says so.

### The byte-level frame engine

Here is the fidelity choice from Chapter 1's table, in code. The model
does *not* shift bits one at a time. `cycles_per_frame` computes how many
CPU cycles one whole frame takes — start bit, data bits, optional parity
bit, stop bits, all at the configured baud — and a single countdown timer
turns into the event "the whole byte just arrived" or "the whole byte
just finished transmitting"
([`crates/coco-core/src/acia6551/frame.rs:127-141`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551/frame.rs#L127-L141)):

```rust
    fn frame_bits(&self) -> u32 {
        START_BITS + self.word_length() + u32::from(self.parity_enabled()) + self.stop_bits()
    }

    /// CPU cycles (crate clock, [`CPU_HZ`]) for one complete RX or TX frame
    /// at the currently configured baud/word-length/parity/stop-bits:
    /// `cycles_per_frame = frame_bits * divider * `[`BAUD_CLOCK_DIVISOR`]`
    /// `* `[`CPU_HZ`]` / `[`ACIA_CRYSTAL_HZ`]`` (u64 math to avoid overflow
    /// and rounding surprises before the final division).
    pub(super) fn cycles_per_frame(&self) -> u32 {
        let frame_bits = u64::from(self.frame_bits());
        let divider = u64::from(self.baud_divider());
        let cycles = frame_bits * divider * BAUD_CLOCK_DIVISOR * CPU_HZ / ACIA_CRYSTAL_HZ;
        cycles as u32
    }
```

Note what `frame_bits` counts: everything on the wire, including the bits
that carry no data. A parity bit costs an eleventh of the frame's time
whether or not this model ever checks its value, because the wire time is
real even when the checking isn't. That is the sort of detail a
byte-level model can still get right, and getting it right is what keeps
the timing honest at the granularity it does claim.

Work the arithmetic by hand for one of the configurations
`acia6551_test.rs` asserts against. At 19200 baud — index 15, divider 6 —
with 8 data bits, no parity, and 1 stop bit, `frame_bits = 1 + 8 + 0 + 1
= 10`:

```
cycles = 10 * 6 * 16 * 894886 / 1843200
       = 960 * 894886 / 1843200
       = 894886 / 1920           (since 1843200 / 960 = 1920)
       = 466.08...  ->  466
```

and the test confirms it to the cycle ([`crates/coco-core/src/acia6551_test.rs:41-52`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551_test.rs#L41-L52)):

```rust
fn take_tx_byte_after_exact_frame_cycles_baud_19200() {
    let mut acia = ACIA6551::new();
    acia.write(2, command::DTR);
    acia.write(3, 15); // baud index 15
    acia.write(0, 0xA5);

    const EXPECTED_CYCLES: u32 = 466; // 10 * 6 * 16 * 894_886 / 1_843_200
    acia.tick(EXPECTED_CYCLES - 1);
    assert_eq!(acia.take_tx_byte(), None);
    acia.tick(1);
    assert_eq!(acia.take_tx_byte(), Some(0xA5));
}
```

The two-step tick is the assertion that matters: one cycle early the byte
is not on the wire, one cycle later it is. At 1200 baud, divider 96, same
frame shape, the same arithmetic gives `10 * 96 * 16 * 894886 / 1843200 =
894886 / 120 = 7457.38… → 7457`, again matching that test's
`EXPECTED_CYCLES` exactly. Both computations simplify the same way:
`frame_bits * divider * 16` always divides `ACIA_CRYSTAL_HZ` evenly —
that is the crystal's whole purpose — leaving `CPU_HZ / <some clean
integer>`, truncated toward zero by Rust's integer division. Deriving the
9600-baud case is the second half of exercise §14.8.1.

### Consume at start, not at finish

The transmit and receive sides run near-identical timers, and the
transmit side is worth reading closely because of *when* it fires TDRE,
which is the single easiest thing to get backwards in a UART model:

```rust
/// If the transmitter is idle and a byte is pending (TDRE clear) and DTR
/// is enabled and BREAK is not active: consume TDR into the shifter, set
/// TDRE (freeing TDR for a new write) and arm the TDRE IRQ source if
/// enabled, and start the frame timer. This is the "consume-at-start"
/// moment MAME fires TDRE/IRQ at — deliberately *not* at frame end.
pub(super) fn start_tx_frame_if_ready(&mut self) {
    if self.tx_timer.is_some() { return; }
    if !self.dtr_enabled() || self.break_active() { return; }
    if self.status & status::TDRE != 0 { return; } // no pending byte
    self.tx_shift_byte = self.tdr;
    self.status |= status::TDRE;
    if self.tx_irq_enabled() {
        self.irq_sources |= irq_source::TDRE;
        self.update_irq_output();
    }
    self.tx_timer = Some(self.cycles_per_frame());
}

fn complete_tx_frame(&mut self) {
    self.tx_output.push_back(self.tx_shift_byte);
}
```

TDRE stands for "Transmit Data Register Empty," and it is the CPU's cue
that writing the *next* byte is safe. It sets the instant the shifter
starts consuming the current byte, not when that byte finishes going out
the wire. This matches the physical chip exactly: the holding register
really is free again the moment its contents move into the shift
register, because the shift register is where the byte lives for the rest
of its journey. The consequence is throughput. A CPU watching TDRE can
keep a 6551 fed back-to-back at close to the wire rate, writing byte N+1
while byte N is still shifting, instead of writing one byte and then
idling for a full frame time. Fire TDRE at frame end instead and every
transfer runs at half speed — a bug that produces correct output and
wrong timing, which is the hardest kind to notice.

Two smaller decisions in that function are worth naming. The three early
returns are a guard sequence, not a condition chain: already
transmitting, not permitted to transmit, nothing to transmit. And
`write_tdr` calls `start_tx_frame_if_ready` immediately on every write,
so a CPU write that lands while the transmitter happens to be idle is
picked up synchronously rather than waiting for the next `tick` — the
same "MAME picks a freshly loaded TDR up as soon as it's written"
behavior the register module documents.

The transmit timer's own advance is the other half of the same
throughput story ([`crates/coco-core/src/acia6551/frame.rs:54-74`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551/frame.rs#L54-L74)):

```rust
    /// Runs the TX frame timer to completion, possibly chaining straight
    /// into the next frame within the same `tick` call if a byte was
    /// already pending and `cycles` outlasts the current frame.
    pub(super) fn tick_tx(&mut self, mut cycles: u32) {
        loop {
            if self.tx_timer.is_none() {
                self.start_tx_frame_if_ready();
            }
            let Some(remaining) = self.tx_timer else {
                break;
            };
            if cycles >= remaining {
                cycles -= remaining;
                self.tx_timer = None;
                self.complete_tx_frame();
            } else {
                self.tx_timer = Some(remaining - cycles);
                break;
            }
        }
    }
```

This is the same catch-up loop the bit-banger's sampler uses, for the
same reason and with one extra move: after finishing a frame it does not
just subtract and stop; it *starts the next one* with the cycles left
over. Without the loop, a caller that handed over a large `cycles` delta
would complete one frame and silently discard the remainder, and the
transmitter's effective rate would depend on how finely the caller
happened to tick it. With it, the only thing that determines output
timing is how much emulated time passed.

### The receive side: overrun, and echo at the wrong granularity

Receiving is the mirror image with two wrinkles
([`crates/coco-core/src/acia6551/frame.rs:76-96`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551/frame.rs#L76-L96)):

```rust
    /// RX frame completion: the pending byte always replaces RDR. Overrun
    /// is set if RDRF was already set (the previous byte was never read).
    /// RDRF is set unconditionally; its IRQ source is armed only if rx-IRQ
    /// is enabled. In echo mode the byte is also queued onto the TX wire
    /// (module doc's byte-level echo approximation — this skips MAME's
    /// force-to-mark-during-overrun nuance).
    fn complete_rx_frame(&mut self) {
        let byte = self.rx_pending_byte;
        if self.status & status::RDRF != 0 {
            self.status |= status::OVERRUN;
        }
        self.rdr = byte;
        self.status |= status::RDRF;
        if self.rx_irq_enabled() {
            self.irq_sources |= irq_source::RDRF;
            self.update_irq_output();
        }
        if self.command & command::ECHO != 0 {
            self.tx_output.push_back(byte);
        }
    }
```

*Overrun* is the receiver's way of reporting that software was too slow:
a new byte completed while the previous one was still sitting unread in
RDR. Notice that the new byte lands in RDR regardless — the chip does not
protect the old byte; it simply records that one was lost. That is the
right behavior to copy, and it is also the reason overrun is one of the
bits `read_rdr` clears: the flag describes the delivery of the byte you
are reading, so reading it retires the complaint.

*Echo mode* is where the byte-level model shows its seam. A real 6551 in
echo mode retransmits each bit as it is shifted in, live, so the echoed
copy of a byte is already halfway back down the wire before the byte has
finished arriving. This model queues the whole received byte onto the
transmit wire the instant the receive frame completes — one frame time
later than the hardware would have started, and all at once rather than
progressively. Nothing that shipped for this pak can tell, because echo
mode's consumers are terminals that care about characters rather than
edges. The divergence is nonetheless documented at the point where it
happens rather than only in the module header, which is the pattern to
copy when you make a fidelity trade of your own.

### The deliberate fidelity gap, and who would notice

The module doc comment states the whole trade-off up front, and it is the
chapter's clearest example of accuracy as a budget rather than a goal:

```rust
//! MAME's `mos6551_device` is a bit-serial engine: it shifts one bit at a
//! time off a per-bit timer and can therefore generate real parity/framing
//! errors and expose bit-accurate RS-232 waveforms. This model is
//! deliberately **byte-level**: [`ACIA6551::tick`] runs a whole-frame timer
//! for the receiver and transmitter, sized from the same baud-rate math MAME
//! uses..., and delivers/consumes a complete byte when that timer expires.
```

Concretely, four things follow from that one sentence.

The model **never generates a parity or framing error internally**. There
is no bit shifter to mis-sample a noisy line, so the status bits exist
and clear at exactly the moments MAME clears them, but nothing in this
implementation ever *sets* them from its own receive process. Such an
error could only arrive already baked into a host-injected byte, and
nothing in the `SerialEndpoint` layer injects corrupted frames.

It **approximates echo mode at the byte boundary**, as the previous
section walked through, and skips a further MAME nuance that forces the
echoed output to mark while overrun is set.

It **collapses the 5-bit-word with 2-stop-bits corner case**. A real 6551
gives that specific combination 1.5 stop bits — a half-width stop
interval, which is meaningful to a bit-serial engine and meaningless to a
frame timer. `ACIA6551::stop_bits` rounds it to a plain 2 and says so in
its doc comment.

And it **resolves DCD/DSR interrupt arming once per `tick` rather than on
a live edge**. The module doc is candid about why that is defensible:
MAME itself "carries `TODO` comments admitting the exact timing is
unresolved" here, so tying the check to this model's own tick boundary
"is no worse and is simpler to reason about." Matching a reference
implementation's uncertainty is a legitimate move; pretending to a
precision the reference does not have would not be.

So who notices? Practically nobody running ordinary terminal software or
BASIC's `OPEN "S"` I/O over the pak. A byte either arrives correctly or
it doesn't, and correct bytes at the right cadence are the entire
contract a terminal emulator or a file-transfer protocol checks. The gap
would matter only to software that deliberately *depends on* bit-level
RS-232 misbehavior — a modem-diagnostic program that injects a framing
error on purpose to exercise its own recovery path, or an
oscilloscope-style RS-232 line monitor. Nothing that shipped for the
CoCo did either of those things through this pak. That is DESIGN.md §5's
philosophy from Chapter 1 — "tighten later only if a game needs it" —
applied for the first time in this course to a non-video subsystem. Note
that it remains falsifiable: the day a program turns up that can tell,
the frame timer becomes a bit timer and the tests that already exist keep
passing.

### DCD/DSR and the IRQ source bitmask

Every interrupt source the 6551 has is tracked as one bit in a private
bitmask rather than as a scatter of booleans, and the *status register's*
IRQ bit is derived from it rather than stored:

```rust
mod irq_source {
    pub const DCD: u8 = 0x01;
    pub const DSR: u8 = 0x02;
    pub const RDRF: u8 = 0x04;
    pub const TDRE: u8 = 0x08;
}

pub(super) fn update_irq_output(&mut self) {
    if self.irq_sources != 0 {
        self.status |= status::IRQ;
    } else {
        self.status &= !status::IRQ;
    }
}
```

One function recomputes the output bit, and every site that arms or
clears a source calls it. The pattern is worth copying whenever a device
has an output that is a pure function of several internal conditions:
storing the derived value but recomputing it in exactly one place gives
you the cheap read (`irq_asserted` is a single mask test) without the
usual risk of the derived value drifting out of step with its inputs.

A status-register read clears every armed source at once, which the
previous section quoted: `self.irq_sources = 0; self.update_irq_output();`.
That is the same "reading has side effects" fact Chapter 1 built the whole
`Bus::read(&mut self)` contract around, showing up in a chip that isn't a
PIA — a useful reminder that the contract was not designed for the PIA
specifically. DCD and DSR arming is gated by DTR and checked once per
tick ([`crates/coco-core/src/acia6551/irq.rs:41-60`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551/irq.rs#L41-L60)):

```rust
    /// Check the DCD/DSR inputs for a change since the last tick and arm
    /// their IRQ source if DTR is enabled (module doc: MAME ties this to
    /// the RX clock with admittedly-unresolved exact timing; this model
    /// resolves it once per `tick` call instead).
    pub(super) fn tick_modem_lines(&mut self) {
        if self.dcd_level != self.dcd_checked {
            self.dcd_checked = self.dcd_level;
            if self.dtr_enabled() {
                self.irq_sources |= irq_source::DCD;
                self.update_irq_output();
            }
        }
        if self.dsr_level != self.dsr_checked {
            self.dsr_checked = self.dsr_level;
            if self.dtr_enabled() {
                self.irq_sources |= irq_source::DSR;
                self.update_irq_output();
            }
        }
    }
```

The pair of fields per line — `dcd_level` and `dcd_checked` — is the
minimal edge detector: one holds the live input, the other holds what was
seen last time, and a difference is an edge. It is the same shape as
`BitBanger`'s `last_mark`, and the same shape as the PIA's `c1_level` in
Chapter 10. Three devices, three edge detectors, one idea.

The distinction to internalize from this function is what DTR gates and
what it doesn't. The *status bits* `status::DCD` and `status::DSR` track
the live input level unconditionally: `set_dcd` and `set_dsr` update them
on every call, whatever DTR is doing. Only the *arming of an interrupt on
a change* is DTR-gated. Phrase it as a principle and it generalizes well
beyond this chip: the status bit is honest about the wire regardless of
software state, while the interrupt is a software-configurable filter
layered on top of it. You will meet that separation everywhere a device
exposes both a live level and a latched, maskable event derived from it —
and the very next example is PIA1's CB1 input in §14.4.

### The wire interface: `SerialEndpoint`

`ACIA6551` never touches a socket or a file. Its host-facing seam is five
methods — `take_tx_byte`, `receive_byte`, `rx_ready`, `set_dcd`,
`set_dsr` — and the module doc describes it as "a pure chip model with a
byte-level wire interface: no knowledge of hosts, sockets, or files."
That is exactly the relationship `PrinterSink` gave the bit-banger in
§14.2, one abstraction level up, and `serial.rs` says so itself: "Same
shape as `crate::bitbanger::PrinterSink`."

The trait is three methods
([`crates/coco-core/src/serial.rs:26-38`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/serial.rs#L26-L38)):

```rust
pub trait SerialEndpoint {
    /// Next byte from the host side, if one is available. Non-blocking:
    /// `None` means "nothing waiting right now", never an error.
    fn poll_rx(&mut self) -> Option<u8>;
    /// Transmit one byte to the host side. Non-blocking / best-effort — a
    /// serial line with nothing attached on the other end just eats the
    /// byte, exactly like real hardware with an unplugged RS-232 cable.
    fn tx(&mut self, b: u8);
    /// Data Carrier Detect: is something connected on the host side? Feeds
    /// the ACIA's status register DCD bit (`docs/plan-deluxe-rs232.md`
    /// "Status bits: ... 5 DCD").
    fn dcd(&self) -> bool;
}
```

Read the two doc comments as a contract rather than as description,
because both of them rule something out. `poll_rx` returning `Option`
rather than `Result` means "nothing waiting" is not an error condition,
so no implementation may report transient emptiness as failure. And `tx`
returning `()` means the caller has no way to learn that a byte was
dropped — which is stated not as a limitation but as fidelity: an
unplugged RS-232 cable also has no way to tell the transmitter that
nobody is listening. Every implementation must be non-blocking end to
end, because the ACIA is polled from the CPU loop and a backend that
could block would stall emulation.

Three implementations ship. `Loopback` is the test and CI endpoint: every
byte handed to `tx` comes straight back out of `poll_rx`, in order, and
`dcd()` is always true because there is nothing to disconnect. It is also
the acceptance seam that made the pak's plan testable at all — "byte
written to `$FF68` reappears at `$FF68`" is a one-line guarantee once
this type exists, and it needs no host I/O whatsoever, so it runs in CI
on a machine with no network and no terminal.

`TCPEndpoint` binds a listener and serves one non-blocking client at a
time, so a host terminal program can connect at any point without the
emulator having a separate "wait for a client" step — every `poll_rx` and
`tx` first tries to accept a pending connection. Its `tx` is the clearest
statement of the trait's best-effort contract
([`crates/coco-core/src/serial.rs:158-175`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/serial.rs#L158-L175)):

```rust
    fn tx(&mut self, b: u8) {
        self.try_accept();
        let Some(stream) = self.client.as_mut() else {
            // Nothing attached: a serial line with an unplugged cable
            // silently drops what's sent to it (module doc comment).
            return;
        };
        match stream.write_all(&[b]) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                // Send buffer momentarily full: drop this byte rather
                // than block the CPU loop.
            }
            Err(_) => {
                self.client = None;
            }
        }
    }
```

Three outcomes, three physical analogies. No client is an unplugged
cable, and the byte evaporates. A full send buffer is a receiver that
cannot keep up, and rather than stall the 6809 the byte evaporates too —
the one place where the analogy is imperfect, since real hardware would
have asserted a flow-control line. Any other error means the peer is
gone, so the endpoint drops back to listening and the next connection
finds a working port. Note that "peer disconnected" is not an error path
at all: `dcd()` simply starts returning false, the ACIA's DCD status bit
follows, and a program watching carrier detect sees exactly what it would
see if a modem had hung up.

`PTYEndpoint` is the third, and it is the one that forced this crate to
allow `unsafe` at all.

> **Rust corner: `unsafe` with `SAFETY` comments, and the crate that
> doesn't forbid it.** Chapter 1 flagged `mc6809`'s `#![forbid(unsafe_code)]`
> as a promise the CPU crate makes and has never needed to break.
> `coco-core` — the crate this whole chapter lives in — makes no such
> promise, and `serial.rs` is why. Allocating a Unix pseudo-terminal pair
> genuinely requires raw `libc` calls (`posix_openpt`, `grantpt`,
> `unlockpt`, `ptsname_r`, `fcntl`) that have no safe wrapper in the
> standard library. The crate handles that honestly, rather than pulling
> in an external PTY crate to hide it:
>
> ```rust
> pub fn new() -> io::Result<Self> {
>     // SAFETY: each libc call's return value is checked before the next
>     // is made; the fd is closed on every early-return error path so no
>     // fd is leaked.
>     unsafe {
>         let master_fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
>         if master_fd < 0 {
>             return Err(io::Error::last_os_error());
>         }
>         if libc::grantpt(master_fd) != 0 {
>             let err = io::Error::last_os_error();
>             libc::close(master_fd);
>             return Err(err);
>         }
>         // ...
>     }
> }
> ```
>
> Every `unsafe` block in the file carries a `// SAFETY:` comment stating
> *why* the invariants the compiler cannot check are actually upheld —
> here, that every fallible call's return value is checked before the next
> one runs, and that no path leaks the file descriptor. Read the elided
> remainder in the source and you will find the same shape repeated for
> `unlockpt` and for the slave-path lookup: allocate, check, and on
> failure close before returning. That discipline is what makes `unsafe`
> in an otherwise-safe codebase legible. A reviewer does not have to
> re-derive the safety argument from scratch, because the comment states
> it and the code is short enough to check the statement against.
>
> The ownership story is closed by a `Drop` implementation that closes the
> master descriptor, with its own one-line SAFETY note: "`master_fd` is
> owned exclusively by this struct and only ever closed here." That
> sentence is the whole invariant. Rust cannot express "this integer is a
> file descriptor with a unique owner" in the type system without a
> wrapper type, so the invariant is stated in prose and enforced by the
> module's small size.
>
> There is a portability wrinkle in the same file worth noticing, because
> it is the kind of thing that only bites on someone else's machine.
> `ptsname_r`, the thread-safe way to resolve a PTY's slave path, is
> bound by `libc` on Linux but not on macOS, so there are two
> `slave_name` implementations behind `#[cfg(target_os = "linux")]` and
> its negation, and the fallback's comment explains that the non-reentrant
> `ptsname` is acceptable "since PTY allocation isn't done concurrently."
> The Linux version carries a second comment explaining why its buffer is
> declared as `[libc::c_char; 128]` rather than `[i8; 128]`: "`c_char`
> signedness is ABI-specific (i8 on x86-64/Apple, u8 on aarch64 Linux), so
> the buffer must use the alias." Both notes are the same lesson —
> `unsafe` FFI is where platform assumptions become compile errors on
> hardware you don't own.
>
> Compare the *shape* of this justification against `mc6809`'s blanket
> `forbid`. The CPU crate can promise "no unsafe, ever" because nothing it
> does needs a raw pointer or a syscall. `coco-core` cannot make that
> promise once it needs to talk to the host operating system's PTY
> subsystem, so instead it localizes every unsafe operation to the few
> functions that truly need it and documents each one individually.
> Neither approach is more correct in the abstract: `forbid` is right
> where it is achievable, scoped-`unsafe`-with-`SAFETY`-comments is right
> where it isn't, and recognizing which situation you are in is the actual
> skill.

---

## 14.4 The RS-232 Pak: completing the CART* interrupt story

### How a UART "rides" a cartridge

Electrically, the Deluxe RS-232 Program Pak is nothing but a 6551 and an
optional 4K EPROM sitting behind the cartridge port's address and data
bus — the same port Chapter 5's I/O map first showed you, and the same port
the whole second half of this chapter is about. Chapter 1 made the claim
that the cartridge port "is barely a device at all," just a raw extension
of the bus with a chip-select line and two interrupt lines brought out to
a connector. This pak is the cleanest demonstration of that claim in the
codebase: a chip, an address range, and an interrupt wire.

Two decode facts do the work:

```rust
pub const ACIA_BASE: u16 = 0xFF68;
pub const ACIA_LAST: u16 = 0xFF6B;
pub const EPROM_LEN: usize = 0x1000; // 4K, mirrored across the CTS window
```

`$FF68`–`$FF6B` sits *outside* the `$FF40`–`$FF5F` SCS* window that most
cartridge I/O decodes, which §14.6 covers properly. Real hardware doesn't
strobe SCS* at these addresses at all; the pak decodes the raw address
bus directly, which it can do because the connector carries all sixteen
address lines. The Sound/Speech Cartridge and the Orchestra-90 use the
same trick at their own addresses. The bus forwards the whole
`$FF60`–`$FF7E` "spare window" to whatever cartridge is installed,
whether or not the address is one that cartridge recognizes, and the pak
answers for its four bytes and floats for everything else:

```rust
fn read(&mut self, addr: u16) -> u8 {
    match addr {
        ACIA_BASE..=ACIA_LAST => self.acia.read((addr & 0x03) as u8),
        _ => IO_OPEN_BUS,
    }
}
```

`addr & 0x03` is the register select, exactly as the connector's low two
address lines would be wired to the chip's own RS0/RS1 pins. The ROM-image
half — the pak's own BASIC/terminal EPROM, reached through `rom_read` —
decodes only twelve address bits, so a real 4K dump mirrors across the
whole CTS window: `image[(addr & 0x0FFF) as usize]`. That is the same
"device doesn't bother decoding every address line" story you will meet
again with ROM paks proper in §14.6, and it is a cost decision rather
than an oversight: address decoding is chips, and chips were money.

The pak also works with no EPROM at all, which is worth knowing before
you go looking for a dump. Using the serial port from OS-9, or from
hand-written BASIC `PEEK`/`POKE` code, needs only the ACIA; a missing
image simply makes CTS reads answer open bus.

### Where the chip meets the host: a poll budget

`ACIA6551` is a pure chip model and `SerialEndpoint` is a pure host
backend, so something has to introduce them. That something is the pak's
own `tick`, and it is the only place in the design where the two halves
touch. It is also where a performance consideration enters the chapter
for the first time, because unlike everything else ticked from the CPU
loop, this one can make a *syscall*
([`crates/coco-core/src/rs232.rs:34-39`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/rs232.rs#L34-L39)):

```rust
/// CPU cycles between host-endpoint polls in [`DeluxeRS232::tick`] (~143 µs
/// at the 0.894886 MHz clock). Polling the endpoint can cost a syscall
/// (nonblocking socket read / `accept`), so it must not run per instruction;
/// this interval stays well under one serial frame even at the ACIA's top
/// rate (19200 baud ≈ 466 cycles/frame), so throughput is never poll-bound.
const HOST_POLL_INTERVAL: u32 = 128;
```

That comment is a complete engineering argument in five lines, and it is
worth taking apart because the *form* of the argument recurs whenever an
emulator has to touch the outside world. There is a lower bound: polling
per instruction would mean a `read` syscall several hundred thousand
times a second, for a device that produces a byte at most every 466
cycles. There is an upper bound: poll less often than one frame time and
the receiver goes idle waiting for input that is already sitting in a
kernel buffer, so throughput would be limited by the poll rate rather
than by the baud rate. And 128 cycles sits comfortably between them, with
the check that proves it stated numerically rather than asserted.

The tick itself splits its work along that same line
([`crates/coco-core/src/rs232.rs:160-188`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/rs232.rs#L160-L188)):

```rust
    fn tick(&mut self, cycles: u32) {
        self.acia.tick(cycles);
        // TX side: frames complete rarely (at most once per serial frame),
        // and checking is a cheap in-memory read, so no throttle here.
        while let Some(b) = self.acia.take_tx_byte() {
            self.endpoint.tx(b);
            self.tx_bytes += 1;
        }
        // RX side + modem lines: endpoint polls can cost syscalls, so run
        // them on the HOST_POLL_INTERVAL cadence.
        self.since_host_poll += cycles;
        if self.since_host_poll < HOST_POLL_INTERVAL {
            return;
        }
        self.since_host_poll = 0;
        // The endpoint trait models one "is anything there" line; feed it
        // to both DCD and DSR — the pak has no independent DSR source.
        let carrier = self.endpoint.dcd();
        self.acia.set_dcd(carrier);
        self.acia.set_dsr(carrier);
        // Pull a host byte only when the receiver is between frames; the
        // rest stays queued host-side (module doc comment).
        if self.acia.rx_ready()
            && let Some(b) = self.endpoint.poll_rx()
        {
            self.acia.receive_byte(b);
            self.rx_bytes += 1;
        }
    }
```

Transmit is unthrottled because draining `take_tx_byte` is an in-memory
queue pop that almost always returns `None`; the syscall only happens on
the rare tick where a frame actually completed. Receive is throttled
because *asking* costs a syscall whether or not a byte is waiting. The
asymmetry is the whole design.

The receive guard is the subtler line. `self.acia.rx_ready()` means "the
receiver has no frame in progress," and pulling a byte only when that is
true is what gives this model flow control for free. A host that types
faster than 19200 baud does not overrun the emulated 6551; the surplus
stays queued in the kernel's socket or PTY buffer until the ACIA is
between frames. The module doc names that substitution explicitly — the
host-side queue "is this model's stand-in for the sender's own pacing."
It is a good trade: no buffer of the emulator's own to size, and the
backpressure lands in the place that already knows how to hold bytes.

Two smaller notes. `set_dcd` and `set_dsr` both receive the same
`carrier` value because the `SerialEndpoint` trait models one "is
anything attached" line and the pak has no independent data-set-ready
source to model separately — an honest simplification, stated where it
happens. And the two `u64` counters exist purely so the frontend can show
serial activity; they are the sort of field that is easy to dismiss as
debug decoration until you are trying to work out whether a silent
terminal session is failing to send or failing to receive.

The pak's save-state behavior follows the same rule Chapter 1 set out for
`SystemBus`: host resources and copyrighted bytes do not travel. The
`endpoint` field is `#[serde(skip, default = "default_endpoint")]`, so a
restored machine comes back with an inert `Loopback` until the frontend
re-plugs a real backend, and the `eprom` field is skipped outright and
re-injected on restore. `DeluxeRS232::set_endpoint`'s doc comment states
the resulting semantics in one sentence — "The ACIA's in-flight frame
state is untouched — this is re-plugging the cable, not resetting the
chip" — which is exactly the distinction a user swapping backends in a
running session would expect.

### `cart_interrupt` as a level, and `poll_cart_interrupt`'s job

The `Cartridge` trait, introduced fully in §14.6, offers two different
interrupt mechanisms, and the RS-232 pak is the one cartridge in this
codebase that uses the *level* form rather than the Q-burst form:

```rust
/// ACIA `_IRQ` → CART* as a level (plan "Prerequisite B"); the bus
/// converts transitions into the PIA1 CB1 edge / GIME EI0 raise.
fn cart_interrupt(&mut self) -> bool {
    self.acia.irq_asserted()
}
```

One line of body, and it is the entire cartridge-side interrupt
implementation: the 6551 already computes "is any source armed" for its
own status register, so the pak just republishes it on the connector pin.

This is the function Chapter 6 named but deferred. `poll_cart_interrupt` is
called once per instruction from `step_cpu_unit` — the very function §6.3
walked through — and its job is to sample that level and convert a
*change* into whatever the physical CART* pin actually feeds
([`crates/coco-core/src/bus/sync.rs:52-62`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sync.rs#L52-L62)):

```rust
/// Sample the level-driven CART* interrupt (e.g. the Deluxe RS-232's 6551
/// ACIA IRQ) and convert transitions into what the shared physical pin
/// feeds: PIA1 CB1 sees the line level itself — CART* is active-low, so
/// asserted = CB1 low, and the PIA latches whichever edge its control
/// register selects — while the GIME EI0 source is raised on the falling
/// (assert) edge only... Polled per-instruction from `Machine::run_cycles`
/// so serial-interrupt latency isn't quantized to scanlines.
pub fn poll_cart_interrupt(&mut self) {
    let level = self.cart.cart_interrupt();
    if level == self.prev_cart_int {
        return;
    }
    self.prev_cart_int = level;
    self.pia1.b.set_c1(!level);
    if level && self.variant == MachineVariant::Coco3 {
        self.gime.raise(gime::intr::EI0);
    }
}
```

Here is the third edge detector of the chapter, in the same shape as the
other two: a stored previous level, an early return when nothing changed,
and real work only on a transition.

`self.pia1.b.set_c1(!level)` is the one line that closes the loop, and
the `!` is load-bearing. CART* is active-low on the real connector, so an
*asserted* interrupt — `level == true` — must drive the PIA's CB1 input
*low*. Get the inversion wrong and the interrupt fires on the wrong edge,
which on a PIA configured for falling edges means it never fires at all.
`set_c1` is the same primitive Chapter 10 introduced for horizontal and
field sync ([`crates/coco-core/src/pia.rs:66-85`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/pia.rs#L66-L85));
it latches the CB1 flag only on a genuine transition whose direction
matches PIA1's control-register edge selection, exactly as it did for
HS/FS pulses. Nothing about it needed to change to accept a third source.

Notice also the CoCo 3 branch. On a CoCo 3 the same physical pin also
feeds the GIME's EI0 interrupt input, raised on the assert edge only, so
the machine has two independent paths from one wire and a program may use
either. On a CoCo 1 or 2 there is no GIME, and the `variant` check skips
that half.

What finally reaches the CPU is the PIA's own output, and Chapter 10 already
built the OR that carries it:

```rust
pub fn firq_asserted(&self) -> bool {
    self.pia1.irq() || self.gime.firq_asserted()
}
```

PIA1's output feeds **FIRQ**, never IRQ. CART* has always been a FIRQ
source on this machine, and this is the same wire the game-pak auto-start
mechanism in §14.6 uses, driven a different way — a Q-clock burst rather
than a level change. One instruction-boundary poll, one shared PIA input
pin, two entirely different cartridges asserting it for two entirely
different reasons.

### Watching the whole chain fire

Descriptions of interrupt chains are easy to write and hard to trust.
[`tests/rs232.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/rs232.rs)'s
`rx_irq_fires_firq_via_pia1_cb1` drives every layer of this one in a
single test, and its doc comment reads as a script for exactly the chain
just walked:

```rust
/// The full interrupt chain of the plan's acceptance test: rx-IRQ enabled,
/// loopback byte completes → ACIA IRQ asserts CART* → `poll_cart_interrupt`
/// drives PIA1 CB1 low (falling edge) → PIA1 FIRQ. Then unwinding it:
/// reading RDR + status drops the ACIA IRQ (CART* deasserts, CB1 returns
/// high — a non-selected edge), and reading PIA1's port B data register
/// clears the latched CB1 flag, releasing FIRQ.
#[test]
fn rx_irq_fires_firq_via_pia1_cb1() {
    let mut bus = bus_with_pak();
    bus.write(PIA1_CRB, cr::C1_IRQ_ENABLE | cr::DDR_ACCESS);
    bus.write(ACIA_CONTROL, CTL_19200_8N1);
    bus.write(ACIA_COMMAND, command::DTR);
    assert!(!bus.firq_asserted());

    bus.write(ACIA_DATA, 0x99);
    run(&mut bus, ROUND_TRIP_BUDGET);
    assert!(bus.firq_asserted(), "RDRF with rx-IRQ enabled must reach FIRQ");

    // Unwind: RDR read clears RDRF, status read clears the ACIA IRQ output.
    assert_eq!(bus.read(ACIA_DATA), 0x99);
    bus.read(ACIA_STATUS);
    run(&mut bus, 16); // let the seam observe CART* deasserting
    assert!(
        bus.firq_asserted(),
        "PIA1 CB1 flag is latched until the data register is read"
    );
    bus.read(PIA1_PORTB_DATA);
    assert!(!bus.firq_asserted());
}
```

Three details in that test repay attention.

The first is `run(&mut bus, ROUND_TRIP_BUDGET)` after writing a byte. The
test writes to `ACIA_DATA`, the pak's loopback endpoint hands the byte
straight back to the receiver, and *then the bus has to run for real
cycles* before anything can be checked. §14.3's byte-level frame timer is
not a formality — a loopback test that never touches a socket still costs
one transmit frame plus one receive frame of emulated time, and a test
that forgot to spend it would see no interrupt and blame the wiring.

The second is the two-step unwind at the end, which is §14.3's
read-side-effect asymmetry showing up one device downstream of where it
lives. Reading the ACIA's status register clears *its* interrupt output,
so CART* deasserts and CB1 returns high — but that rising edge is not the
edge PIA1 was configured to latch, and the flag PIA1 latched earlier is
still set. FIRQ therefore stays asserted, which the test asserts
explicitly with a message explaining why. Only reading PIA1's port B
*data* register clears the latched flag and releases the line. That is
the "reading a data register clears the interrupt flag" fact from Chapter 1
and Chapter 10, now mattering across a device boundary: the chip that
raised the interrupt and the chip that holds it are not the same chip,
and acknowledging one does not acknowledge the other.

The third is the sixteen-cycle `run` between those two steps, with its
comment "let the seam observe CART* deasserting." `poll_cart_interrupt`
is edge-triggered on a polled level; a level that changes and is never
polled has not changed as far as the bus is concerned. Sixteen cycles is
enough time for the poll to happen.

A companion test, `machine_loop_polls_cart_interrupt`, proves the same
chain fires through the real `Machine::run_field` loop rather than the
bus-level `run` helper this test builds. That distinction matters more
than it sounds: it confirms the per-instruction `poll_cart_interrupt`
wiring inside `step_cpu_unit` is what actually does the work in a running
machine, and not an artifact of the test harness calling the seam by
hand.

---

## 14.5 Rung 3: the DMP-105 — a protocol on top

### Framing the layering

Nothing about the bit-banger or the 6551 knows a printer is on the other
end of the wire. Both are pure byte-in, byte-out transports, and that is
the property that makes a third rung possible at all. `DMP105` is the
layer that turns a byte stream into ink. It implements `PrinterSink`, so
it plugs into `BitBanger` exactly where `CaptureSink` and `FileSink` did,
and it interprets every byte arriving there as either printable text or
one of the DMP-105's documented control codes, maintaining a print-head
position and drawing into a shared paper model.

The split between what is general and what is model-specific was drawn
deliberately, and `docs/printer-plan.md`'s framing — quoted in the module
doc comment — is worth keeping in mind while reading. `crate::printer`
holds the paper model shared across the whole DMP family, so a future
DMP-130 or Epson dialect would reuse it unchanged; `dmp105.rs` and its
`protocol` submodule hold everything specific to this one model's
control-code dialect. Paper is paper regardless of which printer marked
it; only the marking rules differ.

There is one more reason this section belongs in a chapter about serial
ports. Everything in §14.2 and §14.3 was verified against a ROM
disassembly or a reference implementation. This section's source material
is a *manual* — a document written for the person who bought the printer,
not for someone implementing it — and the difference in what such a
document does and doesn't tell you shapes the code in ways worth
noticing. Three subsections below exist entirely because the manual is
silent where an implementer needs it to speak.

### The control-code interpreter

Every decoded byte flows through one entry point, `DMP105::feed`, which
dispatches on whether a multi-byte escape or repeat sequence is already
in progress:

```rust
enum Pending {
    None,
    Esc,
    EscOperands { selector: u8, operands: Vec<u8>, need: usize },
    Repeat1,
    Repeat2 { n: u8 },
}

pub(super) fn feed(&mut self, b: u8) {
    match std::mem::replace(&mut self.pending, Pending::None) {
        Pending::None => self.dispatch_fresh(b),
        Pending::Esc => self.begin_esc_operands(b),
        Pending::EscOperands { selector, mut operands, need } => {
            operands.push(b);
            if operands.len() == need {
                self.execute_esc(selector, &operands);
            } else {
                self.pending = Pending::EscOperands { selector, operands, need };
            }
        }
        Pending::Repeat1 => self.pending = Pending::Repeat2 { n: b },
        Pending::Repeat2 { n } => self.execute_repeat(n, b),
    }
}
```

The `std::mem::replace` at the top is doing real work, not ceremony. It
takes the current pending state *out* by value and leaves `Pending::None`
behind, so the match arms own their payloads — `operands` can be pushed
to and moved back in without a clone — and, more importantly, so the
default outcome of every arm is "no sequence in progress." An arm that
forgets to store a new pending state cannot accidentally leave the old
one in place. Chapter 11's Rust corner made the same observation about
`std::mem::take` in the audio event queue; this is the same technique
being used for state-machine hygiene rather than for buffer draining.

Two small modules of named constants stand in for a dispatch table:
`control::` for bare byte codes (`LF`, `CR`, `SELECT_GRAPHICS`, the
repeat introducer `$1C`, and so on) and `esc::` for the byte immediately
following an `ESC` (`$1B`) introducer (`ELONGATE_START`, `POSITION`,
`PITCH_CONDENSED`, …). One value appears in both modules with different
meanings — `$1C` is the repeat introducer as a bare code and "LF pitch =
1/12 inch" as an escape selector — and the constants' doc comments call
that collision out explicitly on both sides, because it is exactly the
sort of coincidence that looks like a copy-paste error to a later reader.

How many operand bytes a selector takes is a `match`, not a runtime
table:

```rust
fn begin_esc_operands(&mut self, selector: u8) {
    let need = match selector {
        esc::POSITION => 2,
        esc::DIRECTION | esc::FEED_IMMEDIATE | esc::FEED_LATCH => 1,
        _ => 0,
    };
    // ...
}
```

Everything else takes none and executes immediately. The manual's own
statement that "all sequences are 2-4 bytes, fixed lengths" is what makes
this safe: there is no self-describing length byte to parse and no
sequence that runs until a terminator, so the parser can never be left
waiting for a byte that isn't coming.

Character-Print-mode dispatch is a straight `match` over the byte's value
range — control codes at the bottom, ASCII in the middle, and undefined
codes falling through to a literal `X` glyph, which is what the manual
specifies rather than what a programmer would invent:

```rust
fn dispatch_cp(&mut self, b: u8) {
    match b {
        control::NUL_IGNORED_0 | control::NUL_IGNORED_1 => {}
        control::LF => self.y = self.y.saturating_add(self.lf_pitch_units),
        control::CR => self.control_cr(),
        control::END_UNDERLINE => self.underline = false,
        control::START_UNDERLINE => self.underline = true,
        control::SELECT_GRAPHICS => { self.graphics_pitch = self.pitch; self.mode = Mode::Graphics; }
        control::END_GRAPHICS => {} // already CP mode: ignored
        0x20..=0x7E => self.print_glyph(dmp105_font::ascii_glyph(b).expect("in range")),
        0xA0..=0xBF => self.print_glyph(dmp105_font::european_glyph(b).expect("in range, TODO placeholder")),
        0xE0..=0xFE => self.print_glyph(dmp105_font::block_glyph(b).unwrap_or_else(dmp105_font::undefined_glyph)),
        _ => self.print_glyph(dmp105_font::undefined_glyph()),
    }
}
```

The `SELECT_GRAPHICS` arm hides a rule worth naming, because it is the
one piece of state in this interpreter that is deliberately *not* live.
Entering graphics mode latches the current pitch into `graphics_pitch`,
and the graphics plotter uses that latched copy rather than the live
`pitch` field. The manual is the source: pitch "must be selected *before*
entry (ignored inside)." So a pitch change arriving mid-graphics still
updates `pitch`, for whenever character mode resumes, but an in-progress
graphics run's dot spacing never shifts underneath it. Modeling that as
two fields rather than one is what makes the rule impossible to violate
by accident.

Graphics mode is the interesting contrast in dispatch policy. Bit 7 set
is *always* data — the manual calls it the data marker, and it is never a
control code — while bit 7 clear falls through to a small set of
recognized codes with **no** undefined-code fallback glyph at all. The
manual states outright that undefined bytes are simply ignored inside
graphics mode, never printed. Two modes, two opposite answers to the
question of what to do with an unrecognized byte, and neither answer is
guessable from the other.

### A real bug the tests remember: repeating the repeat introducer

The DMP-105's repeat sequence — `$1C n c`, "repeat byte `c` `n` times" —
looks at first like it should recurse straight through `feed`. After all,
`c` is just another byte, and `feed` is where bytes go. It deliberately
doesn't:

```rust
/// The repeated byte is expanded through the per-mode dispatchers, NOT
/// through [`DMP105::feed`]: inside a repeat, `c` is the datum being
/// repeated, never a new ESC/repeat sequence introducer... Recursing
/// through `feed` here would let the stream `1C 1C 1C` rebuild its own
/// spawning state unboundedly — a stack-overflow crash on three bytes of
/// arbitrary printer traffic.
fn execute_repeat(&mut self, n: u8, c: u8) {
    if self.mode == Mode::Graphics && c & 0x80 == 0 {
        return;
    }
    for _ in 0..n {
        match self.mode {
            Mode::CharacterPrint => self.dispatch_cp(c),
            Mode::Graphics => self.dispatch_graphics(c),
        }
    }
}
```

Calling `dispatch_cp` rather than `feed` is what breaks the cycle: the
per-mode dispatchers can print and can move the head, but they cannot
start a new pending sequence, so a repeat can never spawn a repeat. The
justification is not merely defensive, either — the spec gives no
semantics for repeating an introducer, and `dispatch_cp` already has a
rule for an out-of-band `$1C`: it is an undefined code, and undefined
codes print `X`.

The regression test names the exact failure mode this prevents:

```rust
/// Regression: `1C 1C 1C` (repeat of the repeat introducer) once
/// re-entered `feed` and rebuilt its own spawning state without bound —
/// a stack overflow on three bytes of arbitrary traffic. Inside a
/// repeat, `c` is data: an undefined CP code that prints the `X`
/// placeholder glyph, `n` times, and the state machine ends clean.
#[test]
fn repeating_the_repeat_introducer_terminates_and_prints_placeholders() {
    let mut dmp = DMP105::default();
    feed_str(&mut dmp, &[control::REPEAT, 3, control::REPEAT]);
    assert_eq!(dmp.x, 3 * normal_cell_width());
    assert!(dmp.paper.extent().dot_count > 0); // three X glyphs
    feed_str(&mut dmp, b"A");
    assert_eq!(dmp.x, 4 * normal_cell_width());
}
```

The final two lines are the ones that make it a good regression test
rather than a crash test. It is not enough that three bytes of hostile
input fail to blow the stack; the interpreter has to come out the other
side in a *clean* state, which the test proves by feeding one more
character and checking the head is where a fourth cell would put it.

This is the class of hardware-fed vulnerability Chapter 1's "fidelity is a
budget" table doesn't capture, because it isn't about accuracy at all. It
is about robustness against untrusted, hostile-shaped input. A printer
byte stream becomes attacker-controlled the moment it arrives from a
socket — §14.3's `SerialEndpoint`, or a `.wav`/`.cas` decode in Chapter 12's
world — so the interpreter has to survive arbitrary bytes, not merely
documented ones, without taking the emulator process down with it. The
same instinct explains every `saturating_add` in the module, and the
print-zone clamp two subsections below.

### Putting ink on paper: glyphs, styles, and one physical limit

Rendering a character is where the interpreter stops being a parser and
starts being a printer, and it is short enough to read whole
([`crates/coco-core/src/dmp105.rs:285-309`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/dmp105.rs#L285-L309)):

```rust
    /// Render one glyph at the current head position, advance `x` by one
    /// character cell, and apply the active style bits
    /// (`docs/printer-plan.md` T4): bold is a second pass one dot column to
    /// the right; elongation doubles every glyph dot column (and thus the
    /// cell advance) horizontally; underline draws a full-cell-width rule on
    /// the descender row.
    fn print_glyph(&mut self, glyph: Glyph) {
        let dot = self.pitch.dot_spacing();
        let col_step = if self.elongation { dot * 2 } else { dot };
        for (col, &bits) in glyph.iter().enumerate() {
            let cx = self.x.saturating_add(col as u32 * col_step);
            self.plot_column(cx, bits);
            if self.bold {
                self.plot_column(cx.saturating_add(dot), bits);
            }
        }
        if self.underline {
            self.draw_underline_rule(dot, CELL_DOTS * col_step);
        }
        // Saturating, like every head-position advance in this module: a
        // long-enough stream without CR would otherwise overflow (a panic in
        // debug builds), and the interpreter must never panic on arbitrary
        // input. Marks past PRINT_WIDTH_X_UNITS are dropped in mark_dot.
        self.x = self.x.saturating_add(CELL_DOTS * col_step);
    }
```

Each of the three style bits is implemented as the physical thing the
printer does, which is why none of them needs a special case anywhere
else. Bold is a second strike of the same column one dot to the right — a
dot-matrix printer has no heavier ink, only more of it. Elongation
doubles the column step, so the same nine columns of glyph data land
twice as far apart and the cell advance doubles with them; there is no
"wide font." Underline is a rule drawn along the descender row, and its
own helper steps at the *base* dot spacing rather than the elongated one,
so the line stays solid rather than dotted under stretched text.

Every dot lands through one function, and that funnel is what enforces
the printer's one physical limit: `mark_dot` drops anything past
`PRINT_WIDTH_X_UNITS`, which is 8 inches in fixed-point units. A real
print head cannot move past the platen's edge, and neither can this one.
The clamp is doing double duty, as the constant's own doc comment says:
without it, "a stream that never sends CR (or an out-of-range `1B 10`
position) grows the paper model without bound." Physical fidelity and
memory safety turn out to want the same check.

One more decision in the same spirit is the printer's reset
([`crates/coco-core/src/dmp105.rs:265-275`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/dmp105.rs#L265-L275)):

```rust
    /// Power-cycle reset (`dmp105-protocol.md` §7: "No software reset code
    /// exists" — the only reset entry point is a power cycle). Restores
    /// every register to its power-on default. Does **not** clear the
    /// paper: already-printed pages are physical and a power cycle doesn't
    /// erase them on real hardware either — see [`Paper::clear`] for the
    /// (separate, explicit) tear-off action that does.
    pub fn reset(&mut self) {
        let paper = std::mem::take(&mut self.paper);
        *self = Self::default();
        self.paper = paper;
    }
```

Reset the machine, keep the paper. It would have been easier to write
`*self = Self::default()` and stop, and the result would have been a
printer that eats its own output when the user clicks a button — which is
not a thing printers do. The three-line dance (take the paper out,
rebuild everything else from defaults, put the paper back) is Chapter 12's
"reset a struct by rebuilding it" pattern with one field held aside, and
the doc comment points at the *separate* explicit action that does erase
paper: tearing it off.

### Fixed-point units: why 1/3600" and 1/72"

Every position `DMP105` tracks — head `x`, head `y` — is a plain integer,
never a float, and the two axes use different denominators chosen for
different reasons.

Vertically, `Y_UNITS_PER_INCH = 72`. Every documented vertical fact in
the DMP-105 manual is already a whole number of 1/72 inch: the three text
line-feed pitches are 1/6" (= 12 units), 1/8" (= 9 units), and 1/12" (= 6
units), and the fixed graphics-mode line feed is 7/72" (= 7 units,
exactly). 1/72" is the *finest* unit that keeps every one of those an
exact integer, and nothing in the source material needs finer.

Horizontally, `X_UNITS_PER_INCH = 3600`, and this one is not a hardware
register at all — it is derived, which the constant's doc comment is
careful to say. The manual's Appendix G gives dots per 8-inch print line
directly: Normal 960, Compressed (12 CPI) 1152, Condensed (16.7 CPI)
1600. Those work out to 120, 144, and 200 dots per inch respectively, so
each pitch's per-dot spacing in inches is 1/120, 1/144, and 1/200. And
3600 is the least common multiple of 120, 144, and 200, chosen
specifically so all three spacings become exact integers of one shared
unit: `3600/120 = 30`, `3600/144 = 25`, `3600/200 = 18`. Three clean
integers, one shared grid, and a pitch change in the middle of a line
(`1B 13`/`1B 14`/`1B 17`) that never needs to round.

```rust
const fn dot_spacing(self) -> u32 {
    X_UNITS_PER_INCH / self.dots_per_inch()
}
```

Ask what the alternative would have cost, because the alternative is what
most people would write first: represent `x` and `y` as `f64` inches.
Every glyph column advance, every `LF`, every `1B 5A n` immediate feed
becomes a floating-point addition — and floating-point addition is not
associative, so `(a + b) + c` and `a + (b + c)` can produce different bit
patterns. A print job is thousands of sequential position updates. Over
that many additions, small representation errors in fractions like 1/144
— which has no exact binary floating-point representation, the same
problem `0.1 + 0.2 != 0.3` demonstrates in every language with IEEE
floats — would accumulate into visible column drift by the end of a long
line. Dots would land a fraction of a position off from where the same
byte stream landed the first hundred times, purely because of summation
order or how many pitch changes happened along the way.

Integer arithmetic in a shared fixed-point unit has none of that.
`30 + 30 + 30` is exactly `90`, always, regardless of grouping, and the
LCM choice guarantees no pitch's per-dot step is a repeating fraction of
the unit in the first place. This is the same reasoning CPU
cycle-counting is built on — §14.2, §14.3, and every timing-sensitive
chapter before this one — applied to spatial position instead of time:
pick an integer unit fine enough that every value you need is exact, and
every operation on it stays exact by construction.

### One documented fact this model refuses to fake

The manual contains a statement that the implementation cannot reproduce,
and what the codebase does about it is the most instructive thing in this
section. The module doc comment lays out the whole problem
([`crates/coco-core/src/dmp105.rs:21-36`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/dmp105.rs#L21-L36)):

```rust
//! One documented fact this module does **not** attempt to reproduce as a
//! literal equality: `dmp105-protocol.md` §5 states "11 full-pitch LFs = 18
//! graphics LFs exactly; 11 half LFs = 9 graphics LFs" as a manual-verified
//! "rounding trap". Using the spec's own independently-verified numbers (a
//! text LF pitch of 1/6" = 12 y-units, and the fixed graphics LF of 7/72" = 7
//! y-units), `11 * 12 = 132` while `18 * 7 = 126` — not equal, and no integer
//! y-unit choice makes `11 * k = 126` work either (126 isn't divisible by
//! 11). Reconciling the manual's stated identity would require a physical
//! stepper-motor step-resolution fact that isn't in the spec document (the
//! most likely explanation: the identity is an artifact of discrete motor
//! step rounding across repeated feeds, not a statement about nominal inch
//! math) — that fact was not provided, so it is not fabricated here. The y-
//! unit arithmetic itself (7/72" graphics LF, 1/6"/1/8"/1/12" text LF
//! pitches) is implemented exactly per the individually-verified numbers;
//! see the `graphics_lf_vs_text_lf_rounding_trap_is_not_reproducible_from_
//! given_facts` test for the documented discrepancy.
```

Follow the reasoning, because it is a template. Two facts from the manual
are individually verified and mutually consistent with the rest of the
model: the text line feed is 1/6 inch, the graphics line feed is 7/72
inch. A third statement in the same manual asserts an identity between
counts of those two feeds. The arithmetic does not close — 132 against
126 — and, crucially, the doc comment goes further and shows that *no*
choice of vertical unit can close it, since 126 is not divisible by
11. That elevates the finding from "our units are wrong" to "these three
statements cannot all be about nominal inch math."

At that point there are three available moves. Fudge the constants until
the identity holds, which breaks the two facts that *are* verified.
Invent the missing physical fact — a stepper-motor step resolution that
would explain the rounding — and quietly encode a guess as hardware.
Or implement the verified numbers exactly, document the discrepancy, and
write a test that *asserts the disagreement* so that nobody later
"fixes" it by accident. The codebase takes the third, and the test's name
is a full sentence about why: the trap "is not reproducible from given
facts."

That last move is the one to steal. A test that encodes a known
unexplained divergence is worth as much as a test that encodes correct
behavior, because without it the next reader has only two hypotheses for
the mismatch — the manual is wrong, or the code is — and no way to tell
which one the previous reader already investigated.

### The paper model: a continuous roll

`Paper` stores marked dots as bands: one `BTreeMap` entry per row `y`,
holding the list of `x` columns marked on that row.

```rust
pub struct Paper {
    rows: BTreeMap<u32, Vec<u32>>,
    #[serde(skip)]
    dirty_min: Option<u32>,
    #[serde(skip)]
    dirty_max: Option<u32>,
}

pub fn dots_in_range(&self, y0: u32, y1: u32) -> Vec<(u32, u32)> {
    self.rows.range(y0..=y1).flat_map(|(&y, xs)| xs.iter().map(move |&x| (x, y))).collect()
}
```

`BTreeMap` rather than a flat `Vec<(u32, u32)>` or a dense 2D array is
what makes `dots_in_range` — "give me everything a scrolled viewport
needs to redraw" — an efficient range scan rather than a linear filter
over the whole roll. That matters once a session has accumulated pages of
print history, which is the normal state of a printer left attached
during a long BASIC session. A dense raster was never an option for a
different reason: at 3600 units per inch horizontally, an 8-inch line is
28,800 addressable columns, almost all of them empty.

There is deliberately no page or form-feed concept anywhere in `Paper`.
The module doc comment cites the protocol spec's own finding that form
feed is "VERIFIED ABSENT" from the DMP-105's firmware entirely — no
page-length register, no top-of-form, nothing. An 11-inch page boundary,
if a frontend wants to draw one, is purely a rendering choice layered on
top of an infinite roll rather than a fact this model tracks. Note the
phrasing in the spec: not "undocumented," which would leave the question
open, but *verified absent*, which closes it.

Two smaller design notes round the type out. `PaperExtent::dot_count` is
deliberately not deduplicated — a dot re-struck at the same position, by
the repeat code or by bold's second pass, counts twice, "matching real
ink laid down twice." And `take_dirty` returns and resets the row range
touched since the last call, which is the cheapest possible "what
changed" signal for a live-updating paper window. It works because of a
physical property the interpreter guarantees: a print head's `y` only
ever advances forward within a job, so nothing below the dirty floor
could possibly have changed. The two dirty fields are `#[serde(skip)]`
for a reason spelled out in the field's own comment: after a snapshot
restore the paper window has no prior frame to diff against anyway, so it
repaints in full regardless of what `take_dirty` would have said.

### The font: data, and honestly labeled as guesswork

Chapter 7 built the VDG's MC6847 text font as *verified* hardware data —
real glyph bitmaps traceable to the chip. The DMP-105's font cannot be
that, and the module doc comment says so as its very first line, in bold,
before anything else:

```rust
//! **ARTISTIC APPROXIMATION — not hardware-verified.** The manual gives cell
//! *geometry* (9x7 + descender row) but, obviously, not the ROM's actual
//! per-dot bitmaps; those are unobtainable from the source material this
//! project has. Every glyph bit pattern below is hand-authored to be a
//! plausible, legible dot-matrix rendering of the character at this cell
//! size — it is not a transcription of real DMP-105 ROM data and must never
//! be cited as a hardware fact.
```

A `Glyph` is `[u8; 9]` — nine columns, matching how a dot-matrix printer
actually prints, column by column rather than row by row, with bits 0–6
as the seven body rows and bit 7 as the descender-row dot. Glyphs are
authored as ASCII art and transposed at compile time by a `const fn`:

```rust
const fn glyph(rows: [&str; 8]) -> Glyph {
    let mut cols: Glyph = [0u8; 9];
    let mut r = 0;
    while r < 8 {
        let bytes = rows[r].as_bytes();
        let bit = if r < 7 { 1u8 << r } else { DESCENDER_BIT };
        let mut c = 0;
        while c < 9 {
            if bytes[c] == b'#' { cols[c] |= bit; }
            c += 1;
        }
        r += 1;
    }
    cols
}
```

The `while` loops rather than iterators are the price of `const fn` —
iterator adapters are not usable in a const context — and the payoff is
that a source line reading `"..#...#.."` looks like the glyph it
produces. That is far more reviewable than raw hex, and it is the same
"author data the way a human actually checks it" instinct Chapter 7's font
tables used, applied to data that has no authoritative source at all.

The honest labeling matters precisely *because* everything else in this
chapter has been verified against ROM disassembly or a manufacturer's
manual. This is the one place the codebase had to guess, and it flags
exactly which byte ranges are guesses: `$A0`–`$BF` European symbols and
`$E0`–`$FE` block graphics are placeholders rather than even
hand-authored guesses, because the manual never transcribes their bitmaps
at all. Only the ASCII range `$20`–`$7E` is a deliberate rendering. A
reader who takes one habit from this chapter could do worse than this
one: when you cannot verify, say so at the top of the file, in the
loudest formatting the language allows.

### End to end: `LLIST` through the whole stack

[`tests/dmp105_boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/dmp105_boot.rs)
is the integration test that proves this chapter's first two rungs and
this section actually compose. It boots the real `coco3.rom`, attaches a
`DMP105Handle` as the bit-banger's sink, types a one-line program,
`LLIST`s it, and checks the paper picked up plausible content:

```rust
let dmp = m.bus.bitbanger.start_dmp105();
// ...
type_str(&mut m, "10 PRINT \"HELLO\"");
tap_char(&mut m, '\r');
type_str(&mut m, "LLIST");
tap_char(&mut m, '\r');

let screen = wait_for_new_ok_prompt(&mut m, baseline_ok, MAX_LLIST_FIELDS);

let extent = dmp.paper_extent();
assert!(extent.dot_count > 0, "LLIST produced no dots ...");
assert!(extent.dot_count > 50, "expected a plausible amount of ink ...");
assert!(
    extent.max_y < Y_UNITS_PER_INCH / 6,
    "a single printed line's dots should stay within one glyph cell's body rows..."
);
```

This is deliberately a "shape" test rather than a glyph-exact one, and
the module doc comment is explicit that "glyph-exact assertions are the
unit golden tests' job." Those live back in `dmp105_test.rs`, where
`hello_cr_at_normal_pitch_produces_expected_glyph_columns_and_row` checks
that `H`'s left stroke lands at exactly the seven expected rows. Splitting
the two kinds of assertion is what keeps the integration test stable: the
font is an admitted approximation and may be redrawn, and a test that
asserted on its exact dots would break every time somebody improved a
letter.

What the integration test buys instead is proof that the *wiring* works
against unmodified ROM code, end to end. Real PIA1 DDR and CRA setup, the
real ROM's bit-bang transmit loop, the BUSY handshake never hanging,
`BitBanger`'s decoder, and `DMP105`'s interpreter, all chained, with the
only stimulus being simulated keystrokes. The third assertion is the
sharpest: every dot from a single printed line has to fall inside one
glyph cell's body rows, which fails loudly if the interpreter mistakes a
line ending for a form feed or if the head's `y` advances when it
shouldn't. That is the same "does the whole path actually compose"
question `bitbanger_boot.rs`'s `LLIST` test asked one layer down, asked
again with a printer on the end of it.

---

## 14.6 Elective: the cartridge system

### The trait as a plugin architecture

Every device in §14.4 — the RS-232 pak — and every device in the rest of
this section implements one trait, `Cartridge`. DESIGN.md named that seam
before any of it existed: "make `Cartridge` a trait so a WD1773 floppy
controller, plain ROM packs, and the Multi-Pak slot all plug in." What is
worth studying is how far that one sentence stretched. Read the trait's
full surface ([`crates/coco-core/src/cart.rs:35-186`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cart.rs#L35-L186),
doc comments trimmed for space) and count the kinds of "cartridge" it has
since learned to express:

```rust
pub trait Cartridge {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
    fn rom_read(&mut self, _addr: u16) -> u8 { ROM_OPEN_BUS }
    fn rom_peek(&self, _addr: u16) -> u8 { ROM_OPEN_BUS }
    fn peek(&self, _addr: u16) -> u8 { IO_OPEN_BUS }
    fn peek_control(&self) -> u8 { IO_OPEN_BUS }
    fn cart_line_ties_q(&self) -> bool { false }
    fn cart_interrupt(&mut self) -> bool { false }
    fn tick(&mut self, _cycles: u32) {}
    fn generator_sample(&mut self, _dt: f64) -> (f32, f32) { (0.0, 0.0) }
    fn halt_asserted(&self) -> bool { false }
    fn take_nmi(&mut self) -> bool { false }
    fn sound_levels(&self) -> (f32, f32) { (0.0, 0.0) }
    fn nmi_pending(&self) -> bool { false }
    fn control_read(&mut self) -> u8 { IO_OPEN_BUS }
    fn control_write(&mut self, _val: u8) {}
    fn reset(&mut self) {}
    fn audio_sample(&mut self) -> f32 { 0.0 }
    fn after_restore(&mut self) {}
    fn validate_restored(&self) -> Result<(), String> { Ok(()) }
}
```

Twenty methods, and only the first two have no default body. That ratio
is what makes this a *plugin* seam rather than a heavyweight interface
every implementor must satisfy in full. `ROMPak` needs `rom_read` and
`cart_line_ties_q` above the required pair, and nothing else. `GamesMasterCartridge` adds
`generator_sample` for its sound chip. Only `DiskCart` implements
`halt_asserted` and `take_nmi`; only the Sound/Speech Cartridge
implements `audio_sample`. Every method a given cartridge doesn't
override answers with the value an absent device would produce, which is
why the empty slot is a struct with no fields and no method bodies at
all.

Notice that the defaults are not `0` or `false` chosen for convenience —
they are the *bus's* answers. `ROM_OPEN_BUS` is `0x00` and `IO_OPEN_BUS`
is `0xFF`, and the constants' doc comments explain the asymmetry: the
external ROM window reads as `$00` when nothing drives it, "verified by
MAME trace-diff, 2026-07-02 — an empty slot's `LDD $C000` yields `$0000`,
not `$FFFF`," while the cartridge I/O window floats high like an
unstrobed PIA input pin. Two windows on the same connector with opposite
idle values, and both are facts somebody had to check rather than assume.

Two categories of method in that list are already familiar from earlier
chapters, wearing new names. `cart_line_ties_q` and `cart_interrupt` are
this section's whole subject, the two halves of the FIRQ story. And
`peek`, `rom_peek`, and `peek_control` are Chapter 16's side-effect-free
debugger twins of `read`, `rom_read`, and `control_read`, arriving here
because a cartridge is exactly the kind of device — like a PIA, like the
GIME — whose plain reads can have side effects a memory viewer must not
trigger. The defaults chosen for the peeks are conservative in the right
direction: a device that cannot read its own state without side effects
reports open bus rather than perturbing anything.

One method is worth a paragraph on its own, because it contradicts the
rest of the chapter and is right to
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

Everything else in this chapter is measured in CPU cycles, and §14.1 made
a point of it. This one method takes seconds, as an `f64`, and the reason
is exactly the reason the rule holds elsewhere: the timebase should be
whatever physically clocks the device. A cartridge's sound generator runs
off a crystal soldered to the cartridge, so it does not care what the
CoCo's CPU is doing — and a cycle-based timebase would let the `$FFD9`
speed poke transpose the music, which is precisely the failure the 6551's
own crystal-based baud generator avoids in §14.3. The rule was never
"count cycles"; it was "count whatever the hardware counts."

> **Rust corner: a closed enum, not `Box<dyn Cartridge>`.** There is a
> genuine surprise waiting for anyone who reads `Cartridge` and assumes
> the cartridge in the port must therefore be stored as a
> `Box<dyn Cartridge>`. It isn't. It is a closed enum
> ([`crates/coco-core/src/cart/cart_enum.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cart/cart_enum.rs)):
>
> ```rust
> #[non_exhaustive]
> #[derive(Serialize, Deserialize)]
> pub enum Cart {
>     Empty(EmptySlot),
>     ROMPak(ROMPak),
>     BankedROMPak(BankedROMPak),
>     GamesMasterCartridge(GamesMasterCartridge),
>     DiskCart(Box<crate::fdc::DiskCart>),
>     MultiPak(Box<MultiPak>),
>     Orch90(crate::orch90::Orch90),
>     DistoRTC(crate::rtc::DistoRTC),
>     DeluxeRS232(crate::rs232::DeluxeRS232),
>     SoundSpeechCartridge(Box<crate::ssc::SoundSpeechCartridge>),
>     #[serde(skip)]
>     Custom(Box<dyn Cartridge>),
> }
> ```
>
> The comment at the top of `cart.rs` gives the reason in one clause: "an
> enum is what lets `SystemBus`/`Machine` derive serde for save-states."
> This is Chapter 1's `#[derive(Serialize, Deserialize)]` story (§1.4)
> resurfacing at a new layer. A trait object has no fixed,
> known-in-advance shape that `serde` can generate a serializer for,
> because `serde` must know at compile time every concrete type that could
> be behind the pointer. An enum *is* that closed list, written out. The
> same trade-off the printer sink faced in §14.2 appears here with the
> opposite resolution: the sink kept its trait object and paid for it with
> a hand-written `SinkState` mapping, while the cartridge kept its
> derivability and paid for it by enumerating.
>
> Two further details in that declaration are worth reading. Three
> variants are boxed — `DiskCart`, `MultiPak`, and `SoundSpeechCartridge` — for two
> different reasons. An enum is as large as its largest variant, so the
> Sound/Speech Cartridge's AY-plus-speech-engine state would otherwise
> inflate every `Cart` value in the program, which clippy flags as
> `large_enum_variant`. `MultiPak` is boxed for a stronger reason: it
> holds four `Cart` slots of its own, so without the indirection the type
> would be infinitely sized. And `#[non_exhaustive]` means out-of-crate
> code cannot write an exhaustive `match` over the variants, which keeps
> adding a cartridge type from being a breaking change.
>
> `Cart::Custom` is the deliberate escape hatch, for out-of-crate test
> doubles like the `TestCart` in
> [`tests/mpi.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/mpi.rs).
> It is excluded from serialization entirely, so a save-state attempted
> with a test double plugged in fails cleanly rather than silently losing
> the cartridge. `Cart::contains_custom` is what checks before
> `crate::snapshot::save` tries, and it recurses into a `MultiPak`'s four
> slots — a test double three levels down inside an expansion box is still
> a test double.
>
> Dispatch to the concrete type is a macro-generated `match`, not a vtable
> call:
>
> ```rust
> macro_rules! with_each_cart {
>     ($self:expr, $cart:ident => $body:expr) => {
>         match $self {
>             Cart::Empty($cart) => $body,
>             Cart::ROMPak($cart) => $body,
>             // ... one arm per variant ...
>             Cart::Custom($cart) => $body,
>         }
>     };
> }
>
> impl Cart {
>     pub fn read(&mut self, addr: u16) -> u8 {
>         with_each_cart!(self, cart => cart.read(addr))
>     }
>     // ... eighteen more delegation methods, identical shape
> }
> ```
>
> The macro exists because the alternative is nineteen hand-written
> eleven-arm `match` statements differing only in the method being called
> — precisely the kind of repetition where a typo in one arm of one method
> produces a bug nobody finds for months. Every arm except `Custom`
> resolves to a statically known concrete type at the call site, which is
> the same monomorphization story Chapter 1's Rust corner told about
> `impl Bus`, except that here a `match` does the dispatch instead of
> generics, because the *set* of cartridge types is closed and known
> rather than "any type the caller supplies." Only `Cart::Custom` pays for
> a genuine virtual call through its box.
>
> Contrast this against where `dyn Trait` genuinely *is* the right tool in
> this same codebase. `BitBanger`'s `Box<dyn PrinterSink>` (§14.2) and
> `DeluxeRS232`'s `Box<dyn SerialEndpoint>` (§14.3) are both open trait
> objects, because those sinks and endpoints are meant to be extended by
> consumers of the crate — a frontend's own printer-capture UI, a future
> host backend — without `coco-core` knowing about them in advance.
> `Cart`'s cartridge set is finite and lives entirely inside this one
> crate: every cartridge Tandy or a third party ever sold for a CoCo is a
> fixed, enumerable list. A closed enum is not a limitation there, it is a
> more precise description of the problem, and it is what buys the whole
> tree its free serialization.

### Auto-start: the Q-burst that boots a game pak instantly

You already have both halves of this mechanism from earlier chapters;
this section is where they are named as one story. `cart_line_ties_q` is
the *other* interrupt mechanism the trait offers, sitting immediately
above `cart_interrupt`:

```rust
/// True while this cartridge ties the expansion-port CART* line to the Q
/// clock (~895 kHz): auto-start game paks do this so edges arrive
/// continuously, which drives both the legacy PIA1 CB1 FIRQ path and the
/// GIME EI0 input the same physical pin feeds. `SystemBus::hsync` polls
/// this once per scanline — plenty to model a ~895 kHz signal, since the
/// PIA/GIME only care that *an* edge keeps arriving. Disk-BASIC-style
/// paks (no autostart) leave this false; they rely on BASIC's cold-start
/// probe of `$C000`/`$C001` for `'D'`,`'K'` instead.
fn cart_line_ties_q(&self) -> bool { false }
```

Real hardware for an auto-starting game pak wires CART* (connector pin 8)
directly to the Q clock pin (pin 7). That is a jumper, not a chip: the
moment the cartridge is plugged into a powered-on CoCo, CART* starts
toggling at the CPU's own quadrature clock rate, continuously, with no
software involved on the cartridge side at all. It is the cheapest
possible way for a cartridge to say "something is here, please look at
me," and it costs the manufacturer a trace.

Modeling a ~895 kHz square wave inside a scanline-granular machine loop
sounds like a problem, and the trait doc comment's answer is the right
one: "the PIA/GIME only care that *an* edge keeps arriving." So
`SystemBus::hsync` samples the flag once per scanline and emits a
complete falling-then-rising pair:

```rust
pub fn hsync(&mut self) {
    // ...HS pulse, HBORD, EI1 keyboard sampling elided...
    if self.cart.cart_line_ties_q() {
        self.pia1.b.set_c1(false);
        if is_gime {
            self.gime.raise(gime::intr::EI0);
        }
        self.pia1.b.set_c1(true);
    }
}
```

Both edges back to back, every scanline — the same "emit both edges
together since there is no sub-line timing resolution" trick Chapter 10's
`hsync` used for the HS pulse itself, reused verbatim for the cartridge
Q-burst. Contrast the two polling rates deliberately. This one is per
scanline, which is wildly more than enough for a signal whose only
information content is "still here." §14.4's `poll_cart_interrupt` is per
instruction, because a 6551's interrupt latency at 19200 baud is short
enough that scanline granularity would be visible to software. Two
sampling rates for two lines on the same connector, each chosen against
what a program could actually detect.

Now the ROM side of the story. It is verified against a real
disassembly — `docs/cartridges.md`, cross-checked against Super Extended
BASIC Unravelled II and Bob Russell's memory map — rather than being this
codebase's own invention. BASIC's reset code enables PIA1's CB1 FIRQ
(`$FF23` bit 0). The continuously toggling Q clock then fires the ROM's
default FIRQ handler at `$A0F6`. On the CoCo 3, that handler's cartridge
path (`$A0FC`) calls a routine at `$8C28` which clears the interrupt,
writes GIME `INIT0 = COCO|MMUEN|MC3|MC2` — that is, `MC1 = 0`, selecting
the 16K-internal plus 16K-external ROM map, with `MC2` enabling standard
SCS — forces ROM mode, and jumps to `$C000`, the external ROM window a
cartridge's `rom_read` answers.

Two consequences of that sequence are easy to miss. The cartridge's code
begins executing inside an interrupt handler, from a FIRQ the cartridge
itself is still asserting; and the memory map it lands in was chosen by
the ROM, not by the cartridge. A non-autostarting cartridge reaches the
same `$C000` by two entirely different routes: BASIC's cold-start code
recognizes a disk controller ROM specifically by its `'D'`,`'K'`
signature bytes at `$C000`/`$C001`, and a human typing `EXEC &HE010`
reaches an arbitrary cartridge's entry point by hand.

[`tests/cart.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/cart.rs)
proves this whole chain fires against the real ROM rather than a
synthetic stub, with one autostart test and one negative control:

```rust
/// A pak whose code at $C000 writes [`MARKER_BYTE`] to [`MARKER_ADDR`] and
/// loops forever: `LDA #$A5 ; STA $0400 ; BRA *` (bytes verified by hand:
/// `86 A5 B7 04 00 20 FE`).
fn marker_pak(autostart: bool) -> ROMPak {
    let mut image = vec![0u8; ROM_PAK_MAX_LEN];
    let program = [0x86, 0xA5, 0xB7, 0x04, 0x00, 0x20, 0xFE];
    image[CART_ENTRY_OFFSET..CART_ENTRY_OFFSET + program.len()].copy_from_slice(&program);
    ROMPak::from_bytes(&image, autostart).unwrap()
}

#[test]
fn autostart_pak_runs_its_cart_code_via_the_firq_boot_path() {
    const MAX_FIELDS: usize = 400;
    let mut m = boot_machine();
    m.insert_cartridge(marker_pak(true));
    m.reset();

    let mut fired = false;
    for _ in 0..MAX_FIELDS {
        m.run_field();
        if m.bus.read(MARKER_ADDR) == MARKER_BYTE {
            fired = true;
            break;
        }
    }
    assert!(
        fired,
        "cart code at $C000 never ran: CART*->PIA1 CB1->FIRQ->$8C28->$C000 path didn't fire"
    );
}

#[test]
fn non_autostart_pak_boots_to_normal_basic_and_never_runs_cart_code() {
    // ...same pak, autostart=false: never writes the marker byte, boots
    // to a normal OK prompt instead.
}
```

The seven bytes of `marker_pak` are the smallest program that can prove
it ran: load a byte, store it somewhere observable, and spin. `$0400` is
the top-left character cell of the text screen, which Chapter 1 promised
would become as familiar as `$FF90`, so the marker is visible on the
emulated screen as well as through `bus.read`.

The assertion message spells out the whole chain by real ROM address:
`CART* -> PIA1 CB1 -> FIRQ -> $8C28 -> $C000`. That is a deliberate
choice about failure ergonomics. If this broke — say `cart_line_ties_q`
stopped being polled, or `set_c1` stopped latching — the test would not
fail at some informative intermediate step. The loop would simply run all
400 fields with `fired` never flipping, and the only diagnostic available
would be whatever the message says. It is also why the loop is bounded at
all: the alternative is waiting forever for a marker byte that will never
arrive, and a hung test is worse than a failed one.

The negative control matters just as much. A test that only checked
autostart firing would pass equally well against an emulator that ran
cartridge code *unconditionally* — which would break every
non-autostarting disk system ever sold. Pairing the two tests pins the
behavior from both sides.

### The Multi-Pak Interface: one port, four slots

The MPI (26-3024) is a passive four-slot expansion adapter with no chip
of its own beyond a 74-series decoder and an 8-bit select register at
`$FF7F`. The key hardware fact, stated once in the module doc comment and
worth internalizing before reading any code: **only three of the
connector's signals are switched per slot** — SCS*, CTS*, and CART*.
Every other line — the address bus, the data bus, HALT*, NMI*, the
clocks — is common to all four slots simultaneously.

```rust
pub mod mpi {
    pub const SLOT_COUNT: usize = 4;
    pub const SCS_MASK: u8 = 0x03;           // bits 1-0
    pub const CTS_SHIFT: u8 = 4;
    pub const CTS_MASK: u8 = 0x03 << CTS_SHIFT; // bits 5-4
    pub const READBACK_OR_MASK: u8 = 0xCC;   // bits 7,6,3,2 forced high on read
    pub const SWITCH_VALUES: [u8; SLOT_COUNT] = [0xCC, 0xDD, 0xEE, 0xFF];
}
```

`$FF7F`'s bits 1–0 select which slot answers the SCS I/O window
(`$FF40`–`$FF5F`); bits 5–4 select which slot's CTS ROM window *and*
CART* line are live. Those last two follow each other always, because a
real MPI switches them together — CART* is not independently selectable
from CTS. So `read` and `write` route the SCS window to
`self.slots[self.scs_slot()]`, while `rom_read`, `cart_line_ties_q`, and
`cart_interrupt` all route to `self.slots[self.cts_slot()]`:

```rust
fn cart_line_ties_q(&self) -> bool {
    self.slots[self.cts_slot()].cart_line_ties_q()
}
fn cart_interrupt(&mut self) -> bool {
    self.slots[self.cts_slot()].cart_interrupt()
}
```

Two independent selectors are exactly why Disk-BASIC-era software could
point SCS at the disk controller's slot while CTS and CART pointed at a
game slot. That is not a clever emulator accommodation; it is what the
hardware does, and software depended on it.

Meanwhile HALT*, NMI*, and `tick` reach *every* slot regardless of
selection. The doc on `MultiPak::tick` says why in one sentence: "a
device doesn't stop just because it isn't currently addressed." An FD-502
sitting in slot 4 can still hold HALT* while the select register points
SCS and CTS at slot 1, and
[`tests/mpi.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/mpi.rs)'s
`halt_and_nmi_are_wire_ored_across_all_slots_regardless_of_selection`
proves exactly that with a synthetic `TestCart`.

Software-write blocking is the switch-and-register interaction most worth
getting straight, because it is the kind of stateful hardware quirk that
is easy to get backwards:

```rust
pub fn set_switch(&mut self, slot: usize) {
    self.switch_slot = slot;
    if !self.switch_blocked {
        self.select = mpi::SWITCH_VALUES[slot];
    }
}

fn control_write(&mut self, val: u8) {
    self.select = val;         // full-byte replace, never a nibble merge
    self.switch_blocked = true;
}

fn reset(&mut self) {
    self.select = mpi::SWITCH_VALUES[self.switch_slot];
    self.switch_blocked = false;
    for slot in &mut self.slots { slot.reset(); }
}
```

The front-panel switch controls `$FF7F` — until software writes to
`$FF7F` itself, at which point the switch is locked out. Turning the
physical dial afterward still records the new position, so the next reset
picks it up, but it has no live effect until RESET* fires. Note that
`set_switch` keeps recording while blocked rather than refusing: the
switch is a physical object and turning it does something whether or not
the machine is listening.
[`tests/mpi.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/mpi.rs)'s
`software_write_blocks_the_switch_until_the_next_reset` walks exactly
that sequence — a software write selecting slot 0, then `set_switch(1)`
recorded but inert, confirmed still reading slot 0's value, then
`reset()`, which both restores switch control *and* loads the now-moved
switch's value in one step.

Two MAME facts are deliberately *not* modeled here, and the module doc
lists them rather than leaving a reader to wonder:

```rust
/// Two MAME facts are deliberately NOT modeled here, per spec: the CoCo 3
/// never delivers external-ROM-window *writes* to cartridges at all (already
/// true of `SystemBus`, independent of the MPI), and the field-mod some real
/// MPIs have that ties all 4 slots' CART* lines together (a hardware hack,
/// not stock behaviour) is not reproduced — CART* here strictly follows the
/// CTS select, as spec'd.
```

The first is redundant rather than wrong — the bus already refuses those
writes — and saying so prevents a future reader from adding a second
guard for a case that cannot occur. The second is a scope decision worth
copying: a modification some units received in the field is not the
behavior of the product, and reproducing it would make correct software
misbehave on a stock machine. Both notes exist so that "the MPI doesn't
do X" reads as a decision rather than as an omission.

### ROMPak: mirror-fill, and the half-swap quirk

A ROM pak dump is a headerless raw file. The emulator community's
`.ccc`/`.rom`/`.bin` convention carries no size or banking metadata at
all — size *is* the metadata, and everything else has to be inferred from
how the hardware behaves. Two facts about how such a dump maps onto the
CPU's 32K external ROM window are easy to get backwards on a first try.

**Mirror-fill.** Real cartridge ROMs frequently don't decode every
address line. An 8K EPROM in a socket wired for up to 32K repeats every
8K across the window on real silicon, because the chip simply never looks
at the address bits above its own capacity — the same economy that makes
the RS-232 pak's EPROM mirror every 4K in §14.4. `mirror_fill` matches
MAME's own load-time doubling loop exactly, comment included:

```rust
/// Copy `bytes` into a `total_len` buffer and mirror-fill the rest with MAME
/// `cococart_slot_device::call_load`'s doubling loop... Each copy lands at a
/// multiple of the image length and copies a prefix of an already-periodic
/// buffer, so the result is byte-identical to plain repetition
/// (`image[i % len]`) for every image size — the doubling is only an
/// efficiency trick, kept in MAME's shape so the provenance is obvious.
fn mirror_fill(bytes: &[u8], total_len: usize) -> Box<[u8]> {
    let mut image = vec![0u8; total_len].into_boxed_slice();
    image[..bytes.len()].copy_from_slice(bytes);
    let mut read_length = bytes.len();
    while read_length < total_len {
        let len = read_length.min(total_len - read_length);
        let (src, dst) = image.split_at_mut(read_length);
        dst[..len].copy_from_slice(&src[..len]);
        read_length += len;
    }
    image
}
```

That comment is doing something subtle and worth imitating. Written from
scratch, this function would be one line: fill `image[i]` with
`bytes[i % bytes.len()]`. The doubling loop is kept anyway, in MAME's
shape, "so the provenance is obvious" — and the comment then *proves*
the imported shape equivalent to the obvious one, so a reader has to
trust neither. `mirror_fill_equals_plain_repetition_for_any_size` turns
that proof into a checked fact for a deliberately non-power-of-two size,
5000 bytes, asserting that every one of the 32K window's bytes equals
`bytes[i % LEN]`. Front-loading the image this way also means `rom_read`
needs no bounds logic at all, whatever the original image size was.

**The half-swap.** This is the genuinely surprising one. A 32K pak dump
is conventionally laid out CTS-window-first: file offset 0 is the byte
that should appear at `$C000`, and for full 32K carts file offset `$4000`
is the byte that appears at `$8000` once the GIME's INIT0 map switches to
32K-external. But the GIME does not route cartridge banks in that
straightforward an order. Its actual bank-routing formula, from MAME's
`gime.cpp`, is `((bank & 3) ^ 2) * 0x2000`, which swaps the two 16K
halves relative to a naive `addr - $8000` index. `ROMPak::rom_read`
reproduces the swap directly:

```rust
const ROM_PAK_HALF_SWAP: u16 = 0x4000;

fn rom_read(&mut self, addr: u16) -> u8 {
    self.image[((addr - ROM_PAK_BASE) ^ ROM_PAK_HALF_SWAP) as usize]
}
```

`full_32k_image_maps_cts_half_first` checks the consequence directly:
`rom_read(0xC000)` — the byte a `JMP $C000` autostart landing executes
first — returns `bytes[0x0000]`, file offset zero, while
`rom_read(0x8000)` returns `bytes[0x4000]`, the file's second half.

Now consider the failure mode if that XOR were omitted or inverted,
because it is a small masterclass in why some bugs survive testing. Any
pak of 16K or less still works perfectly, since mirror-fill has already
made both halves byte-identical and the swap is therefore invisible. Only
a real 32K cartridge — Arkanoid, whose entry code genuinely lives at file
offset 0 expecting to be fetched at `$C000` — boots into garbage. That is
exactly the shape of bug that passes every synthetic test and fails on
one specific commercial cartridge, which is why the test constructs a
full 32K image with `bytes[i] = i as u8`, making every offset
individually distinguishable, rather than anything smaller and more
convenient.

Banked paks — `BankedROMPak`, the RoboCop/Predator circuit the Games
Master Cartridge reuses — trade the fixed 32K window for a **16K** window
that a whole-byte write to `$FF40` slides across up to 128K of image. The
effective bank is `bank * 16K mod 128K`, wrapping so an undersized image
mirrors across the unused upper banks exactly like a plain pak does, and
the latch resets to bank 0 on RESET*. Interestingly, the 16K window makes
the half-swap bit *disappear* rather than needing separate handling:
`BANKED_PAK_WINDOW_LEN - 1` as the address mask discards precisely the
bit `ROM_PAK_HALF_SWAP` would have flipped. The code comment says so, and
`window_mirrors_across_both_16k_halves_of_the_external_map` confirms the
same bank shows identically at `$8000`, `$A000`, and `$C000` with no
special-casing in `BankedROMPak::rom_read` at all.

### The Games Master Cartridge: banked ROM plus a chip you already know

John Linville's GMC is, structurally, nothing but a `BankedROMPak` with
one extra write-only register:

```rust
const GMC_PSG_REG: u16 = 0xFF41;

pub struct GamesMasterCartridge {
    rom: BankedROMPak,
    psg: crate::sn76489::SN76489A,
}

fn write(&mut self, addr: u16, val: u8) {
    match addr {
        GMC_PSG_REG => self.psg.write(val),
        _ => self.rom.write(addr, val),  // $FF40: the bank latch
    }
}
fn generator_sample(&mut self, dt: f64) -> (f32, f32) {
    let level = self.psg.sample(dt);
    (level, level)
}
```

The SN76489A itself is Chapter 11's chip, unchanged and unaware it is in a
cartridge. The GMC's only contribution is *plumbing* — that chip, running
at its own 4 MHz crystal, alongside a bank-switched ROM window — which is
why `generator_sample` is four lines, and why it takes `dt` in seconds
rather than in cycles, for the reason the trait's doc comment gave
earlier in this section.

One detail is worth flagging as a documented MAME-fidelity choice rather
than an oversight: GMC audio mixes into the speaker *unconditionally*,
bypassing the analog sound mux's SEL-bit-gated cartridge-sound input
entirely. The doc comment notes that MAME routes the GMC's PSG to a
dedicated speaker device, ignoring the SNDEN and mux path "entirely,"
with no independent schematic settling what the real cartridge's SND-pin
wiring should do instead. Faced with a choice between copying a reference
implementation's behavior and inventing a mux-gated path with no evidence
behind it, this model copies — and records that it copied, so the day a
schematic turns up there is a note saying what to revisit.

[`tests/gmc.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/gmc.rs)'s
`bank_latch_pages_the_16k_window` confirms the inherited banking behaves
exactly like a standalone `BankedROMPak`, writing each of 8 bank values
to `$FF40` and checking `rom_read($C000)` returns that bank's marker
byte. A separate MPI test confirms the PSG's audio genuinely plays "from
any slot" — the analog bus is common across an MPI's four slots, just
like HALT* and NMI* — even while a *different* slot answers the register
writes. That pair demonstrates the section's whole architecture at once:
one cartridge composed from two existing pieces, dropped into a four-slot
adapter, with the per-slot lines and the common lines behaving
differently and correctly.

---

## 14.7 Reading assignment

Read these in this order. The first four are the chapter's three rungs
plus the cartridge that carries the middle one; the fifth is elective and
is best saved until the rest has settled.

1. **[`crates/coco-core/src/bitbanger.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs), whole file** — the module doc
   comment first, then `tick`/`sample_threshold`, then the sink family and
   `sink_serde` at the bottom. You have every fact needed to hand-derive
   the baud table in §14.2 from this file alone.
2. **[`crates/coco-core/src/acia6551.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551.rs) and its [`frame.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551/frame.rs)/[`registers.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551/registers.rs)/
   [`irq.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551/irq.rs) submodules** — read the module doc comment's "byte-level
   timing divergence" section slowly; it's the chapter's clearest single
   statement of the fidelity-is-a-budget philosophy applied to a chip
   this course hasn't met before. Then read `registers.rs` end to end and
   notice how much of a UART's behavior is *which bits a read clears*.
3. **[`crates/coco-core/src/rs232.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/rs232.rs) and [`serial.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/serial.rs)** — both small
   enough to read whole in one sitting. `rs232.rs` is the chapter's best
   example of "a cartridge is just a chip wired to a bus," reusing every
   mechanism §14.6 names; `serial.rs` is where the emulator stops and the
   host operating system begins.
4. **[`crates/coco-core/src/dmp105.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/dmp105.rs), [`dmp105/protocol.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/dmp105/protocol.rs), and
   [`printer.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/printer.rs)** — read `feed`/`dispatch_cp`/`execute_repeat`
   together; the recursion-avoidance comment on `execute_repeat` is worth
   re-reading after you've seen the regression test that motivated it.
   Read `printer.rs`'s two unit constants for the fixed-point argument in
   its original form.
5. **[`crates/coco-core/src/cart.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cart.rs) and [`cart/cart_enum.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cart/cart_enum.rs)** (elective) —
   the trait first, then `with_each_cart!`; compare against
   `BitBanger`'s `Box<dyn PrinterSink>` to feel the difference between
   "closed set, needs serde" and "open set, extended by consumers."

While reading, run the whole chapter's test surface:

```
cargo test -p coco-core --lib bitbanger:: acia6551:: dmp105:: serial::
cargo test -p coco-core --test bitbanger rs232 dmp105_boot cart mpi gmc
```

Four of the six test targets in that second line need `roms/coco3.rom`:
`dmp105_boot`, plus the real-ROM tests inside [`cart.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/cart.rs)/
[`mpi.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/mpi.rs)/[`gmc.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/gmc.rs) (the `cart.rs` and `mpi.rs` ones additionally need
`roms/disk11.rom`). So do the two boot tests §14.2 quoted,
[`bitbanger_boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bitbanger_boot.rs) and [`bitbanger_os9.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bitbanger_os9.rs), which that line doesn't name —
and `bitbanger_os9.rs` additionally needs disk images under `disks/`.
None of those assets ship in this repository or in a fresh worktree;
they're git-ignored and present only on the machine this course was
authored on. Most of those tests check for the asset; if it's missing,
the test prints `eprintln!("skipping ...")` and returns cleanly rather
than failing — the `try_load_rom()`-returning-`Option` pattern at the
top of each file. Two are stricter: `cart.rs` and `gmc.rs` panic with a
clear message when the ROM is absent, on the theory that their whole
file is about real-ROM behavior and a silent skip would be misleading. The pure unit and bus-level tests ([`bitbanger_test.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger_test.rs),
[`acia6551_test.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551_test.rs), [`serial_test.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/serial_test.rs), [`dmp105_test.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/dmp105_test.rs), the synthetic
parts of [`tests/cart.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/cart.rs)/[`tests/mpi.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/mpi.rs)/[`tests/gmc.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/gmc.rs)) need nothing and
will run regardless.

---

## 14.8 Exercises

**14.1 — Baud arithmetic, both rungs (build/compute).** Two parts:

  (a) The bit-banger. Using `cycles_per_bit = 78 + 16N`, compute the bit
  period for `POKE 150,41` (the table in §14.2 lists this as the
  "1200 baud" row — confirm it: `78 + 16*41 = ?`, then `894886 / that
  cycles/bit = ?` baud). Then compute `POKE 150,180` (labeled "300
  baud" in the same table) the same way, showing both steps.

  (b) The 6551. Using `cycles_per_frame = frame_bits * divider * 16 *
  CPU_HZ / ACIA_CRYSTAL_HZ`, derive the cycle count for **9600 baud**
  (baud index 14, divider 12), 8 data bits, no parity, 1 stop bit
  (`frame_bits = 10`). Show the simplified fraction (`10*12*16 = 1920`;
  `1843200/1920 = ?`; `894886 / that = ?`, truncated). Then write a test
  in the shape of `acia6551_test.rs`'s
  `take_tx_byte_after_exact_frame_cycles_baud_19200` asserting your
  computed value, and run it to confirm.

**14.2 — Control-code trace (read/predict).** Without running any code,
trace this DMP-105 byte stream by hand and predict (i) the head's final
`(x, y)` in fixed-point units, and (ii) how many glyph cells get printed:

```
ESC $17          ; 1B 17 — select Compressed (12 CPI)
"HI"              ; two ASCII glyphs
ESC $0E           ; 1B 0E — start elongation
"!"               ; one glyph, elongated
CR                ; return to column 0, feed one line (CR+LF is the default NL mode)
```

Work `Pitch::Compressed.dot_spacing()` from the table in §14.5
(`X_UNITS_PER_INCH / dots_per_inch`, where Compressed is 144 dots/inch)
before computing cell advances, and remember elongation doubles the
*column step* (hence the cell width) only for the glyph it's active
during. Check your trace against a test you write using `DMP105::feed`
directly (see `dmp105_test.rs`'s `feed_str` helper for the pattern) —
assert on `dmp.x` and `dmp.y` after the sequence.

**14.3 — Sabotage: break false-start rejection (sabotage, verified).**
In [`crates/coco-core/src/bitbanger.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs), find the `if pa1_mark {
false_start = true; break; }` guard inside `tick`'s `START_SAMPLE` arm
(§14.2 quotes it in full) and change the condition so the check never
fires (for instance, replace `pa1_mark` with the literal `false`). Run
`cargo test -p coco-core --lib bitbanger::`. Confirm exactly one test
fails — `sub_bit_glitch_is_rejected_as_false_start` — and read its
failure output closely: the captured byte is not the `0xFF` you might
predict from "the glitch free-runs into a phantom byte" (that was the
*pre-fix* bug the check exists to prevent, described in the module doc
comment); with the guard merely disabled rather than removed, the
subsequent frame realigns differently and a different phantom byte
comes out. Note the actual value your run produces, then revert your
edit with a second `Edit` call restoring `pa1_mark` exactly (do not
`git checkout` the file), rerun the test module, and confirm all 17
tests pass again and `git status` is clean.

**14.4 — Build: an elongated-mode DMP-105 test (build).** `1B 0E`/`1B
0F` (start/end elongation) are already implemented (`esc::ELONGATE_START`/
`esc::ELONGATE_END` in [`dmp105/protocol.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/dmp105/protocol.rs)) but exercise them yourself:
write a new test in the shape of `dmp105_test.rs`'s existing tests that
feeds `ESC $0E`, one glyph, `ESC $0F`, another glyph, and asserts the
*second* glyph's cell starts exactly `2 * normal_cell_width()` after the
first (elongation doubles `col_step`, hence doubles the cell advance,
only while active). Then go one step further: add a *new* control code
this codebase doesn't have — `1B 09` for "elongate exactly the next
character only" (a documented DMP-130 feature, not part of the DMP-105
manual verified in `docs/dmp105-protocol.md` — treat this as your own
extension, not a hardware claim) — implemented as a one-shot flag that
clears itself after the next `print_glyph` call. Test it the same way.

**14.5 — Read/predict: an MPI test (read/predict).** Read
[`tests/mpi.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/mpi.rs)'s `write_replaces_the_whole_byte_not_a_nibble_merge` test
(quoted context: it writes `0x10` to `$FF7F`, then `0x01`) without
running it, and predict, before checking: what does `mp.scs_slot()`
return after each write, and what does `mp.cts_slot()` return after the
*second* write specifically? Explain in one sentence why the second
write's CTS value is *not* whatever bits `0x10`'s write left behind —
tie your answer to the `control_write` snippet in §14.6.

**14.6 — Design essay, three sentences max (essay).** The 6551 model in
this codebase is byte-level; MAME's is bit-serial. Name one concrete
category of CoCo software (real or hypothetical) that would behave
differently under the two models — not "less accurate" in the abstract,
but a specific observable difference a specific piece of software could
detect — and explain why nothing that shipped for the Deluxe RS-232 Pak
actually depended on that difference.

**14.7 — Recall: the two interrupt paths (recall).** From memory, name
which `Cartridge` trait method a game pak overrides to auto-boot, which
method the Deluxe RS-232 pak overrides instead, which PIA and which side
(A or B) both ultimately drive, and whether the result reaches the CPU's
IRQ or FIRQ pin. Then check your answer against §14.4/§14.6 — if you got
the PIA side wrong, re-read `poll_cart_interrupt`'s `self.pia1.b.set_c1`
line and `hsync`'s matching `self.pia1.b.set_c1` line side by side; both
land on the same field for a reason.

---

## What's next

Chapter 15 leaves the core entirely and spends a week on `coco-egui`, the
frontend: the per-frame loop, input routing, the VM manager, and headless
UI testing with kittest. One thread from this chapter continues there —
the DMP-105's *paper*, as an actual on-screen scrolling view a user can
watch fill up while `LLIST` runs, is Chapter 15's to build; everything about
what goes *onto* that paper (the protocol, the fixed-point coordinate
system, the font) was this chapter's, and so was `take_dirty`, the
one-call answer to the only question a live paper view actually asks.
Chapter 16 closes the course with the debugger and save states — and now
that you've read `Cart`'s enum-versus-trait-object story, and watched a
`Box<dyn PrinterSink>` get serialized through a state enum it doesn't
know about, you already understand *why* the whole cartridge tree rides
along for free in every `.ccstate` snapshot — ROMPak images excepted,
since those are copyrighted bytes, reattached separately.
