# Chapter 14 — Serial: Bit-Banging, a Real UART, and a Printer

*Week 14. Goal: climb three rungs of the same ladder — a software-timed GPIO
pin, a real hardware UART, and a protocol interpreter built on top of
either — and along the way finish two stories earlier chapters left
open: week 6's `poll_cart_interrupt` and week 10's PIA1 CB1 path both
terminate here, in the code that turns a cartridge's interrupt line into a
running 6809 program. This chapter closes Part V; an elective second half
covers the cartridge system that makes that termination possible in the
first place.*

---

## 14.1 The serial ladder

"Serial port" undersells what's actually three unrelated pieces of
hardware wearing the same name, and this codebase implements all three at
different fidelity because 1980s CoCo software used them at different
fidelity:

1. **Bit-banging** (`bitbanger.rs`). The CoCo's "printer port" is not a
   UART chip at all — it's one output pin (PIA1 PA1) and one input pin
   (PIA1 PB0), and Color BASIC's ROM does the framing, timing, and
   handshaking entirely in software: a busy-wait loop toggling a GPIO pin
   at a rate it counts out in cycles. There is no hardware here to model
   except a PIA you already met in week 10 — the "protocol" lives in ROM,
   and the emulator's job is to *decode what the software transmits*, the
   same relationship the cassette deck (week 12) has to its FSK tones.
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
   shape week 1 promised applies all the way down.

Two design threads run underneath all three rungs, and you should watch
for them as you read:

- **Cycle-based timing, never wall time.** Rung 1's bit period and rung
  2's baud generator are both expressed in CPU cycles, which is why the
  CoCo 3's `$FFD9` high-speed poke and BASIC's own `POKE 150,n` fall out
  for free instead of needing special-cased "fast mode" code — you saw
  this exact idea drive the cassette deck in week 12, and it recurs here
  unchanged.
- **Fidelity is a budget, spent unevenly.** Chapter 1's table already
  flagged the punchline: "Serial UART (6551): byte-granular frames, not
  bit-serial." §14.3 is where you find out exactly what that costs and
  who would notice.

The elective second half, §14.6, is a genuine tangent from serial I/O —
the cartridge port is its own subsystem — but it's grouped here because
the CART* auto-start interrupt is the other half of the story
`poll_cart_interrupt` (week 6) and PIA1's `set_c1` (week 10) began, and
because the Deluxe RS-232 pak (rung 2) is itself a cartridge. Reading
§14.3–14.4 before §14.6 will make the auto-start story land better; the
dependency runs in that direction, not the reverse.

---

## 14.2 Rung 1: the bit-banger — a GPIO pin and a stopwatch

### The hardware, in one paragraph

There is no printer UART chip in a stock CoCo. Color BASIC's `LLIST`/`LPRINT`
driver bit-bangs RS-232-style serial timing on two PIA1 pins: PA1
(`$FF20` bit 1, DIN pin 4) is the output — a plain async serial line, 1
start bit + 8 data bits LSB-first + 1 stop bit, no parity — and PB0
(`$FF22` bit 0, DIN pin 2) is BUSY feedback from the printer, polarity
0=ready/1=busy. The module doc comment for `bitbanger.rs` states the
consequence directly:

```rust
/// Unlike the cassette deck (FSK tones demodulated by zero-crossing
/// threshold), the printer port is a plain async serial line: 1 start bit
/// (space) + 8 data bits (LSB-first) + 1 stop bit (mark), no parity
/// (`bitbanger-spec.md` "Framing"). [`BitBanger`] models the *receive* side
/// only — decoding what the ROM's bit-bang driver transmits on PA1 — since
/// that's the only direction a virtual printer needs.
```

Two constants pin down the pins ([`crates/coco-core/src/bitbanger.rs:44-49`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs#L44-L49)):

```rust
pub const TX_PIN: u8 = 0x02;
pub const BUSY_PIN: u8 = 0x01;
```

Both are PIA1 bits you already have the vocabulary for from week 10 —
this chapter adds no new hardware primitive, only a new *use* of one.

### How PA1 actually reaches the decoder

The bus doesn't hand `BitBanger::tick` a raw PIA register — it computes
what a receiver watching the physical wire would see, accounting for
whether the pin is even configured as an output yet
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
register" logic week 10 built into the PIA itself — reused here at the
call site rather than duplicated, because a pin's DDR bit answers "is
this even driven" independently of whatever bit pattern happens to sit
in the output register.

### The RX state machine

`BitBanger::tick` is driven once per instruction from the machine loop
(you saw this call site already in week 6's `step_cpu_unit`:
`self.bus.bitbanger.tick(cycles, self.bus.pia1_tx_mark());`), with a
cycle delta and PA1's level held constant across that delta. It is an
edge-triggered async receiver — the same shape as a real UART's start-bit
hunt — implemented as a two-state enum
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

and the tick function itself ([`crates/coco-core/src/bitbanger.rs:375-432`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs#L375-L432),
elided only where the doc comments already quoted above repeat):

```rust
pub fn tick(&mut self, cycles: u32, pa1_mark: bool) {
    self.state = match self.state {
        RxState::Idle => {
            if self.last_mark && !pa1_mark {
                // Falling edge: mark -> space, a start-bit candidate.
                RxState::Receiving { elapsed: cycles, sample: 0, bits: 0 }
            } else {
                RxState::Idle
            }
        }
        RxState::Receiving { mut elapsed, mut sample, mut bits } => {
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
                } else {
                    // Stop bit read space: framing error. Discard the
                    // byte and resync...
                    self.framing_errors += 1;
                }
                sample += 1;
            }
            if false_start || sample >= TOTAL_SAMPLES {
                RxState::Idle
            } else {
                RxState::Receiving { elapsed, sample, bits }
            }
        }
    };
    self.last_mark = pa1_mark;
}
```

Read the sample-index arithmetic against the frame layout: `START_SAMPLE
= 0` validates the start bit, samples 1 through 8 (`sample <= DATA_BITS`)
pull each data bit in with `bits |= u8::from(pa1_mark) << (sample - 1)` —
notice that's `sample - 1`, so data-bit sample 1 lands in bit position 0
— **LSB first**, matching the framing spec — and sample 9 checks the stop
bit. `TOTAL_SAMPLES = DATA_BITS + 2 = 10`.

### Why mid-cell, not edge-aligned

Every sample happens at the *middle* of its bit cell, not at its edge —
`sample_threshold` computes `(0.5 + k)` bit-times for sample `k`
([`crates/coco-core/src/bitbanger.rs:434-441`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs#L434-L441)):

```rust
/// CPU-cycle offset of sample `sample` (0-indexed) after the start-bit
/// edge: sample times are 0.5 (start validation), 1.5, …, 8.5 (data),
/// 9.5 (stop) bit-times, so sample `k` sits at `(0.5 + k)` bit-times =
/// `bit_period * (2k + 1) / 2`.
fn sample_threshold(&self, sample: u8) -> u32 {
    let scaled = u64::from(self.bit_period) * (2 * u64::from(sample) + 1);
    (scaled / 2) as u32
}
```

This is the same reason a real UART oversamples (typically 16×, sampling
near the theoretical bit center): the transmitter and receiver run off
independent clocks with no shared reference, so their idea of "when a bit
cell starts" drifts apart the longer a frame runs. Sampling at the
*edge* of a cell means the smallest clock disagreement flips you onto the
wrong side of a transition; sampling at the *center* buys a full half a
bit-time of slack before that happens, and that slack is largest exactly
where you need it — at the *last* bit of a 10-bit frame, where drift has
had the longest time to accumulate. Two tests exercise this directly
([`crates/coco-core/src/bitbanger_test.rs:101-121`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger_test.rs#L101-L121)): building a frame
2% faster or 2% slower than the decoder's configured rate still decodes
cleanly, because "by bit 9 (9.5 bit-times in) accumulated drift is under
0.19 bit-times, well inside the 0.5 bit-time margin each mid-cell sample
has."

The same mechanism doubles as glitch rejection. `START_SAMPLE`'s job is
specifically to catch a falling edge that *isn't* a real start bit: if
the line is already back at mark by the half-bit-time mark, the edge that
triggered `Receiving` was too short to be a genuine transmission, and the
frame is silently abandoned — "not a framing error, since no frame ever
began." The doc comment names a concrete real-world trigger for this:
"the ~30-cycle low pulse PA1 emits while the ROM's boot code
reconfigures DDRA (`$A02F`) free-runs into a phantom 0xFF" without the
check. The test `sub_bit_glitch_is_rejected_as_false_start` sends a
30-cycle glitch (against a ~1486-cycle bit period — under 2% of one bit
cell) followed by two full bit-periods of mark, and confirms nothing
decodes and no framing error is counted, then confirms a real byte sent
afterward still decodes cleanly.

### The bit period: 78 + 16×N, and why N = 88 means 600 baud

`BitBanger` doesn't hardcode "600 baud" anywhere — it stores a bit period
in CPU cycles, settable at runtime, because that's what `POKE 150,n`
changes on real hardware:

```rust
pub const DEFAULT_BIT_PERIOD: u32 = 78 + 16 * 88;
```

The formula and the constant `88` both come from an actual ROM
disassembly, not a datasheet — `docs/bitbanger-spec.md` records the
derivation (this project's `docs/` directory holds copyrighted reference
material and isn't part of the public repo, but the finding is: the
bit-bang delay loop is a self-referential `BSR` that runs the countdown
*twice* per bit, so `cycles_per_bit = 78 + 16×N`, where `N` is the live
16-bit value of the ROM variable `LPTBTD` at `$0095`/`$0096` — decimal
149/150, hence `POKE 150,n`). The default N the ROM actually initializes
at boot is `88` (`$0058`) — not the `87` printed in the CoCo 3 Service
Manual's Table 2, which the spec's own cross-check identifies as a stale
pre-1.2 constant. Trust the ROM bytes, not the manual, when the two
disagree; the spec's cross-reference to Color BASIC Unravelled confirms
the 87→88 change happened at Color BASIC 1.2.

The full measured table, straight from the ROM trace:

| Baud label | N | cycles/bit (78 + 16N) | Effective @ 0.894886 MHz |
|---|---|---|---|
| 120 | 458 | 7406 | 120.9 |
| 300 | 180 | 2958 | 302.5 |
| **600 (default)** | **88** | **1486** | **602.2** |
| 1200 | 41 | 734 | 1219.2 |
| 2400 | 18 | 366 | 2445.6 |

Verify the default row by hand: `78 + 16×88 = 78 + 1408 = 1486` — exactly
the source constant. Baud is `CPU_HZ / cycles_per_bit`: `894886 / 1486 =
602.15…`, close enough to nominal 600 baud that "600 baud" is the
label everyone uses, even though the true rate running on real silicon is
602.2. (The chapter's baud-arithmetic exercise, §14.8.1, asks you to
redo this for a different `N`.)

Because the emulator counts *CPU cycles*, not wall-clock milliseconds,
two things fall out with no special-case code at all:

- **`POKE 150,n`** just calls `BitBanger::set_bit_period` with a
  recomputed `78 + 16*n`; the decoder doesn't know or care that BASIC
  changed its mind about the rate mid-session.
- **The `$FFD9` high-speed poke** doubles the CPU clock without touching
  the ROM's cycle-counted delay loop, so it *exactly* doubles the
  effective baud — the delay loop still counts the same number of
  (now-faster) cycles. Test `double_rate_bit_period_decodes`
  ([`crates/coco-core/src/bitbanger_test.rs:150-158`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger_test.rs#L150-L158)) configures the
  decoder at half the default period and confirms a byte sent at that
  rate still decodes — proving the relationship is pure arithmetic, with
  "no special-cased fast mode."

### A driver that doesn't play along: NitrOS-9

Not every piece of CoCo software shares Color BASIC's obliviousness to
clock speed. [`tests/bitbanger_os9.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bitbanger_os9.rs)'s module doc comment records a
genuinely surprising empirical finding, arrived at by instrumenting the
decoder's raw edge timings during a real NitrOS-9 boot rather than
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

In other words: Color BASIC's printer driver counts a fixed number of
*cycles* per bit and lets the speed poke change what that means in
wall-clock time (600 baud becomes 1200 the instant you `POKE 65497,0`
then hit the poke's own semantics — actually the reverse direction, but
the point stands: cycle-count-fixed means baud follows clock speed).
NitrOS-9's `/p` driver does the opposite: it detects the doubled clock
and doubles its own delay-loop count to compensate, holding *true* baud
constant at 600 regardless of speed. Two independently-written 6809
device drivers, two opposite policies for the same hardware fact — and
the emulator's decoder doesn't encode either policy; it just counts
whatever cycles actually elapse, which is why it can decode both without
being told which driver it's listening to. The test itself
([`crates/coco-core/tests/bitbanger_os9.rs:204-237`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bitbanger_os9.rs#L204-L237)) boots the real EOU
disk images to a shell, runs `echo hello >/p`, retunes the decoder to
`2 * DEFAULT_BIT_PERIOD`, and asserts the captured bytes are exactly
`b"hello \r"` with zero framing errors — a second, independently-written
driver validating the same decoder that Color BASIC's `LLIST` exercises.

### BUSY feedback: what's real, what's a stub

PB0 carries BUSY back from the (virtual) printer, and `BitBanger` exposes
it as a plain settable bit:

```rust
pub fn busy(&self) -> bool { self.busy }
pub fn set_busy(&mut self, busy: bool) { self.busy = busy; }
```

Be honest about what this buys you today: nothing in this codebase's
`Dmp105` sink currently calls `set_busy` — the module doc comment says so
plainly, calling out "a (currently unimplemented) DMP-105 buffer model"
and citing the spec's own flag: BUSY's *assertion granularity* (does the
real printer's 134-byte receive buffer assert BUSY per-byte? only when
nearly full?) is marked INFERRED, not VERIFIED, in `dmp105-protocol.md`
— the manual documents the polarity and the existence of a 134-character
buffer but never states the exact byte-count trigger. The wire is real
and tested ([`tests/bitbanger.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bitbanger.rs)'s three bus-level tests confirm PB0
reads 0 by default and reflects `set_busy`), but no code path currently
drives it from print volume — a virtual printer in this emulator is
always "ready," which is a fidelity choice that would only bite if a
program depended on flow control the emulator never actually needs to
apply (an infinite-speed printer never needs to say "slow down").

### Rust corner: the `Rc<RefCell<_>>` handle pattern for pluggable sinks

`BitBanger` owns its decoded-byte destination as `Box<dyn PrinterSink>` —
one trait, several implementations (`NoopSink`, `CaptureSink`, `FileSink`,
and rung 3's `Dmp105Handle`). `CaptureSink`'s definition is worth pausing
on:

```rust
#[derive(Clone, Default)]
pub struct CaptureSink(Rc<RefCell<Vec<u8>>>);
```

Why not a plain `Vec<u8>`? Because once `set_sink` moves a `CaptureSink`
into `BitBanger`'s `Box<dyn PrinterSink>`, the caller has no way to read
the accumulated bytes back out — the `Vec` would be locked inside a
trait object with no getter. `Rc<RefCell<Vec<u8>>>` fixes this by
splitting *ownership* (shared, via `Rc`) from *access* (checked at run
time, via `RefCell`): clone the handle before handing one half to
`set_sink`, keep the other half, and both refer to the same buffer.
`Dmp105Handle` (§14.5) is the same pattern one level richer — a shared
handle to a whole interpreter, not just a `Vec`.

Contrast this with week 11's cross-thread audio ring buffer,
`Arc<Mutex<VecDeque<_>>>`. Both are "shared ownership plus a way to
mutate through a shared reference" — but `BitBanger`, its sink, and the
frontend code that later reads a `Dmp105Handle`'s paper all run on the
*same* thread (the emulator core has no background thread of its own),
so the atomic-refcounting and OS-level locking `Arc`/`Mutex` pay for buys
nothing here. `Rc`/`RefCell` do the identical job — shared mutable
access — at a fraction of the cost, because `Rc`'s reference count is a
plain (non-atomic) integer and `RefCell`'s borrow check is a runtime
comparison, not a kernel-mediated lock. The rule of thumb: reach for
`Arc`/`Mutex` only once you actually cross a thread boundary (week 11's
audio callback genuinely does); everywhere else in this single-threaded
core, `Rc`/`RefCell` is the right — and cheaper — tool. Note too that
this is *not* the architecture week 1 spent a whole section warning you
away from: `Machine`/`SystemBus` themselves stay a plain owned tree with
zero `Rc`/`RefCell` anywhere in them (that's what makes save states
trivial); the pattern shows up only at this one narrow seam, where a
value must legitimately be reachable from two independent owners
(the bus's sink slot, and the frontend's paper-window state) for reasons
that have nothing to do with the core machine's own structure.

---

## 14.3 Rung 2: the 6551 ACIA — a real UART

### Why a second serial device at all

The bit-banger models what stock BASIC already does with zero extra
hardware. The Tandy Deluxe RS-232 Program Pak (26-2226) is different: it
plugs an actual MOS 6551 ACIA (Asynchronous Communications Interface
Adapter — "not a Motorola MC-prefixed part," the module doc is careful to
note, since so much of this codebase's other chips are Motorola parts) into
the cartridge port. A 6551 does in silicon what the printer driver does
in a busy-wait loop: hardware framing (start/data/parity/stop bits shifted
by a real shift register), a programmable baud-rate generator driven off
its own crystal, and genuine modem-control inputs (DCD, DSR) with their
own interrupt sources. This section is the chapter's real UART; rung 1
never had one.

### Register bitmaps

Four registers, offsets 0–3 (`$FF68`–`$FF6B` once you know where the pak
decodes them, §14.4):

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

Two bits are worth flagging by name because they're easy to misread:
`command::RX_IRQ_DISABLE` is *inverted* — clear means enabled, matching
the real 6551 (the doc comment calls this out explicitly: "0 = rx-IRQ
enabled, 1 = disabled") — and `tx_control` (the two bits at
`TX_CONTROL_MASK`) is a four-value enum packed into two bits, not a
simple on/off:

```rust
pub mod tx_control {
    pub const RTS_OFF: u8 = 0;       // RTS off, no TDRE IRQ (tx still runs)
    pub const IRQ_ENABLED: u8 = 1;   // tx on, RTS on, TDRE IRQ enabled
    pub const RTS_ON: u8 = 2;        // tx on, RTS on, TDRE IRQ disabled
    pub const BREAK: u8 = 3;         // forced BREAK; TDR never consumed
}
```

That table is where the register bitmap stops being self-explanatory and
starts encoding real 6551 behavior: only value `1` ever arms the TDRE
IRQ, and only value `3` (BREAK) actually stops the transmitter — value
`0` (`RTS_OFF`) still transmits, just silently. A quick way to internalize
this: `acia6551_test.rs`'s
`consume_at_start_fires_irq_only_when_tx_irq_enabled` writes `DTR` alone
(command's low bits all zero, i.e. `RTS_OFF`) and confirms no IRQ fires
on a TDR write, then writes `DTR | (IRQ_ENABLED << TX_CONTROL_SHIFT)` and
confirms one does.

### Baud math from a crystal, not a table lookup

The 6551 doesn't remember "1200 baud" as a concept — it divides its
reference crystal down by a fixed factor and a per-index divider, and
*that* result happens to equal 1200. The crystal is standard across the
whole 6551 family:

```rust
const ACIA_CRYSTAL_HZ: u64 = 1_843_200;      // 1.8432 MHz
const BAUD_CLOCK_DIVISOR: u64 = 16;
const BAUD_DIVIDER: [u32; 16] = [
    1, 2304, 1536, 1048, 856, 768, 384, 192, 96, 64, 48, 32, 24, 16, 12, 6,
];
```

`baud = ACIA_CRYSTAL_HZ / 16 / BAUD_DIVIDER[index]`. Work three of the
sixteen indices by hand, because the numbers land exactly:

| Baud | Index | Divider | `1843200 / 16 / divider` |
|---|---|---|---|
| 300 | 6 | 384 | `1843200 / 16 / 384 = 300` |
| 1200 | 8 | 96 | `1843200 / 16 / 96 = 1200` |
| 9600 | 14 | 12 | `1843200 / 16 / 12 = 9600` |
| 19200 | 15 | 6 | `1843200 / 16 / 6 = 19200` |

1.8432 MHz is not an arbitrary number — it's the classic UART reference
crystal chosen specifically because dividing it by 16 and then by small
integers lands on every standard RS-232 baud rate with zero rounding
error, which is exactly why 6551s, 8250s, and 16550s across the whole
microcomputer era all used the same crystal value.

### The byte-level frame engine

The interesting fact about this model is that it does *not* shift bits
one at a time. `cycles_per_frame` computes how many CPU cycles one whole
frame (start + data + optional parity + stop bits, at the configured
baud) takes, and a single timer counts down to "the whole byte just
arrived" or "the whole byte just finished transmitting":

```rust
fn frame_bits(&self) -> u32 {
    START_BITS + self.word_length() + u32::from(self.parity_enabled()) + self.stop_bits()
}

pub(super) fn cycles_per_frame(&self) -> u32 {
    let frame_bits = u64::from(self.frame_bits());
    let divider = u64::from(self.baud_divider());
    let cycles = frame_bits * divider * BAUD_CLOCK_DIVISOR * CPU_HZ / ACIA_CRYSTAL_HZ;
    cycles as u32
}
```

Work this by hand for two of the configurations `acia6551_test.rs`
actually asserts against. At 19200 baud (divider 6), 8 data bits, no
parity, 1 stop bit — `frame_bits = 1 + 8 + 0 + 1 = 10`:

```
cycles = 10 * 6 * 16 * 894886 / 1843200
       = 960 * 894886 / 1843200
       = 894886 / 1920           (since 1843200 / 960 = 1920)
       = 466.08...  ->  466
```

and the test confirms it exactly ([`crates/coco-core/src/acia6551_test.rs:41-52`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551_test.rs#L41-L52)):

```rust
const EXPECTED_CYCLES: u32 = 466; // 10 * 6 * 16 * 894_886 / 1_843_200
acia.tick(EXPECTED_CYCLES - 1);
assert_eq!(acia.take_tx_byte(), None);
acia.tick(1);
assert_eq!(acia.take_tx_byte(), Some(0xA5));
```

At 1200 baud (divider 96), same frame shape: `cycles = 10 * 96 * 16 *
894886 / 1843200 = 894886 / 120 = 7457.38… → 7457` — again matching the
test's `EXPECTED_CYCLES` exactly. Both computations simplify the same
way: `frame_bits * divider * 16` always divides `ACIA_CRYSTAL_HZ` evenly
(that's the crystal's whole purpose), leaving `CPU_HZ / <some clean
integer>`, truncated. This is the exercise in §14.8.1's second half:
derive the 9600-baud case yourself (divider 12) and predict the truncated
cycle count before checking it against a test you write.

The transmit and receive sides run near-identical timers
([`crates/coco-core/src/acia6551/frame.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551/frame.rs)). Transmit is worth reading
closely because of *when* it fires TDRE, which is easy to get backwards:

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

TDRE — "Transmit Data Register Empty," the CPU's cue that it's safe to
write the *next* byte — sets the instant the shifter *starts* consuming
the current byte, not when the byte finishes going out the wire. That
matches real UART behavior (the holding register really is free again
the moment its contents move into the shift register) and it's why the
CPU can keep feeding a 6551 back-to-back bytes at close to the wire rate
instead of one-byte-then-wait-a-whole-frame. `write_tdr` calls
`start_tx_frame_if_ready` immediately on every write, so a CPU write
landing while the transmitter happens to be idle is picked up
synchronously rather than waiting for the next `tick`.

### The deliberate fidelity gap, and who would notice

The module doc comment states the trade-off up front, and it's worth
reading in full because it's the chapter's clearest example of "accuracy
is a budget":

```rust
//! MAME's `mos6551_device` is a bit-serial engine: it shifts one bit at a
//! time off a per-bit timer and can therefore generate real parity/framing
//! errors and expose bit-accurate RS-232 waveforms. This model is
//! deliberately **byte-level**: [`Acia6551::tick`] runs a whole-frame timer
//! for the receiver and transmitter, sized from the same baud-rate math MAME
//! uses..., and delivers/consumes a complete byte when that timer expires.
```

Concretely, this model:

- **Never generates a parity or framing error internally.** There's no
  bit shifter to mis-sample a noisy line, so the status bits exist (and
  clear at exactly the moments MAME clears them) but this implementation
  never *sets* them from its own RX process — those errors would only
  ever arrive already-baked into a host-injected byte, and nothing in
  this codebase's `SerialEndpoint` layer injects corrupted frames.
- **Approximates echo mode at the byte boundary.** A real 6551 in echo
  mode retransmits each bit as it's shifted in, live; this model queues
  the *whole* received byte onto the TX wire the instant the RX frame
  completes (`complete_rx_frame`'s last few lines). It also skips a MAME
  nuance — forcing the echoed output to mark while overrun is set.
- **Collapses the 5-bit-word/2-stop-bits corner case.** A real 6551 gives
  that specific combination 1.5 stop bits; this model rounds it to a
  plain 2 (`Acia6551::stop_bits`'s doc comment says so directly).
- **Resolves DCD/DSR IRQ arming once per `tick` instead of on a live
  edge.** The module doc is candid that MAME itself "carries `TODO`
  comments admitting the exact timing is unresolved" here, so tying it
  to this model's own tick boundary "is no worse and is simpler to
  reason about."

Who notices? Practically nobody running ordinary terminal software or
BASIC's `OPEN "S"` I/O over the pak — a byte either arrives correctly or
it doesn't, and correct bytes at the right cadence is all a terminal
emulator or a file-transfer protocol actually checks for. The gap would
only matter to software that deliberately *depends on* bit-level RS-232
misbehavior: a modem-diagnostic program that injects a framing error on
purpose to test its own recovery path, or an oscilloscope-style RS-232
line monitor. Nothing that shipped for the CoCo did either of those
things through this pak, which is exactly the DESIGN.md §5 philosophy
from week 1 ("tighten later only if a game needs it") applied to a
non-video subsystem for the first time in this course.

### DCD/DSR and the IRQ source bitmask

Every 6551 IRQ source is tracked as one bit in a private bitmask, not as
scattered booleans, and the *status register's* IRQ bit is just
"is any source bit set":

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

A status-register *read* clears every armed source at once
(`read_status`: `self.irq_sources = 0; self.update_irq_output();`) —
that's the same "reading has side effects" fact week 1 built the whole
`Bus::read(&mut self)` contract around, showing up in a chip that isn't a
PIA. DCD/DSR arming is gated by DTR, checked once per tick:

```rust
pub(super) fn tick_modem_lines(&mut self) {
    if self.dcd_level != self.dcd_checked {
        self.dcd_checked = self.dcd_level;
        if self.dtr_enabled() {
            self.irq_sources |= irq_source::DCD;
            self.update_irq_output();
        }
    }
    // ...identical shape for DSR
}
```

but the *status bits themselves* (`status::DCD`, `status::DSR`) track the
live input level unconditionally, independent of DTR — `set_dcd`/`set_dsr`
update them immediately, on every call, regardless of whether DTR happens
to be enabled. Only the *IRQ-arming-on-a-change* is DTR-gated. This
distinction — "the status bit is honest about the wire regardless of
software state, but the interrupt is a software-configurable filter on
top of it" — is a pattern worth internalizing, because you'll meet it
again wherever a chip exposes both a live level and a latched/maskable
event derived from that level (PIA1 CB1 in §14.4 is the very next
example).

### Rust corner: `unsafe` with `SAFETY` comments, and the crate that doesn't forbid it

Week 1 flagged `mc6809`'s `#![forbid(unsafe_code)]` as a promise the CPU
crate makes and never needs to break. `coco-core` — the crate this whole
chapter lives in — makes no such promise, and `serial.rs`'s
`PtyEndpoint` is why: allocating a Unix pseudo-terminal pair genuinely
requires raw `libc` calls (`posix_openpt`, `grantpt`, `unlockpt`,
`ptsname_r`, `fcntl`) that have no safe Rust wrapper in the standard
library. Read how the crate handles that honestly rather than reaching
for an external PTY crate to hide it:

```rust
pub fn new() -> io::Result<Self> {
    // SAFETY: each libc call's return value is checked before the next
    // is made; the fd is closed on every early-return error path so no
    // fd is leaked.
    unsafe {
        let master_fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        if master_fd < 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::grantpt(master_fd) != 0 {
            let err = io::Error::last_os_error();
            libc::close(master_fd);
            return Err(err);
        }
        // ...
    }
}
```

Every `unsafe` block in this file carries a `// SAFETY:` comment
explaining *why* the invariants the compiler can't check are actually
upheld — here, that every fallible call's return value is checked before
the next one runs, and that no code path leaks the file descriptor. This
is the discipline that makes `unsafe` in an otherwise-safe codebase
legible: a reviewer doesn't have to re-derive the safety argument from
scratch, because the comment states it. Compare the *shape* of the
justification to `mc6809`'s blanket `forbid`: the CPU crate can promise
"no unsafe, ever" because nothing it does needs a raw pointer or a
syscall; `coco-core` cannot make that same blanket promise once it needs
to talk to the host operating system's PTY subsystem, so instead it
localizes every unsafe operation to the few functions that truly need it
(`PtyEndpoint`'s handful of methods) and documents each one individually.
Neither approach is "more correct" in the abstract — `forbid` is right
where it's achievable, and scoped-`unsafe`-with-`SAFETY`-comments is right
where it isn't; recognizing which situation you're in is the actual skill.

### The wire interface: `SerialEndpoint`

`Acia6551` itself never touches a socket or a file — its host-facing seam
is `take_tx_byte`/`receive_byte`/`rx_ready`/`set_dcd`/`set_dsr`, a pure
chip model exactly like `PrinterSink` was a pure decoded-byte sink in
§14.2. `serial.rs` supplies three implementations of the actual host
wire:

```rust
pub trait SerialEndpoint {
    fn poll_rx(&mut self) -> Option<u8>;
    fn tx(&mut self, b: u8);
    fn dcd(&self) -> bool;
}
```

`Loopback` (every byte handed to `tx` comes straight back out of
`poll_rx`, DCD always true) is the test/CI endpoint and the acceptance
seam — "byte written to `$FF68` reappears at `$FF68`" is a one-line
guarantee once you have this. `TcpEndpoint` binds a listener and serves
one non-blocking client at a time, treating "nothing connected" as
`dcd() == false` rather than an error — a real unplugged RS-232 cable
just eats what you send it, and `TcpEndpoint::tx` does exactly that on a
`WouldBlock`. `PtyEndpoint` is the one just discussed.

---

## 14.4 The RS-232 Pak: completing the CART* interrupt story

### How a UART "rides" a cartridge

The Deluxe RS-232 Program Pak is, electrically, nothing but a 6551 and an
optional 4K EPROM sitting behind the cartridge port's address and data
bus — the same port week 5's I/O map first showed you, and the same port
the whole second half of this chapter (§14.6) is about. Two decode facts
matter:

```rust
pub const ACIA_BASE: u16 = 0xFF68;
pub const ACIA_LAST: u16 = 0xFF6B;
pub const EPROM_LEN: usize = 0x1000; // 4K, mirrored across the CTS window
```

`$FF68`–`$FF6B` sits *outside* the `$FF40`–`$FF5F` SCS* window that most
cartridge I/O decodes (§14.6 covers SCS* properly) — real hardware
doesn't strobe SCS* here at all, but the pak decodes the raw address bus
directly, the same trick the Sound/Speech Cartridge and the Orchestra-90
use at their own addresses. The bus forwards this whole `$FF60`–`$FF7E`
"spare window" to whatever cartridge is installed, whether or not the
address is one it recognizes:

```rust
fn read(&mut self, addr: u16) -> u8 {
    match addr {
        ACIA_BASE..=ACIA_LAST => self.acia.read((addr & 0x03) as u8),
        _ => IO_OPEN_BUS,
    }
}
```

The ROM-image half (the pak's own BASIC/terminal EPROM, `rom_read`)
decodes only 12 address bits, so a real 4K dump mirrors across the whole
CTS window: `image[(addr & 0x0FFF) as usize]` — the same "device doesn't
bother decoding every address line" story you'll see again with ROM
paks proper in §14.6.

### `cart_interrupt` as a level, and `poll_cart_interrupt`'s job

The `Cartridge` trait (fully introduced in §14.6) offers two different
interrupt mechanisms, and the RS-232 pak is the one cartridge in this
codebase that uses the *level* form rather than the Q-burst form:

```rust
/// ACIA `_IRQ` → CART* as a level (plan "Prerequisite B"); the bus
/// converts transitions into the PIA1 CB1 edge / GIME EI0 raise.
fn cart_interrupt(&mut self) -> bool {
    self.acia.irq_asserted()
}
```

This is the function week 6 named but deferred: `poll_cart_interrupt`,
called once per instruction from `step_cpu_unit` (the very function §6.3
walked you through), samples this level and converts a *change* into
what the physical CART* pin actually feeds — PIA1's CB1 input
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

`self.pia1.b.set_c1(!level)` is the one line that closes the loop: CART*
is active-low on the real connector, so an *asserted* level (`true`)
must drive CB1 *low* (`false`) — hence the `!`. `set_c1` is the same PIA
primitive week 10 introduced for horizontal and field sync
([`crates/coco-core/src/pia.rs:66-85`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/pia.rs#L66-L85)); it only latches the CB1 flag on a
genuine transition whose direction matches PIA1's control-register edge
selection, exactly as it did for HS/FS pulses. `Pia::irq()` — the
"either side is asserting" OR you saw wired into `SystemBus::firq_asserted`
back in week 10's coverage of the PIA — is what actually reaches the CPU:

```rust
pub fn firq_asserted(&self) -> bool {
    self.pia1.irq() || self.gime.firq_asserted()
}
```

PIA1's output feeds **FIRQ**, never IRQ — CART* has always been a FIRQ
source on the CoCo, and this is the same wire the game-pak auto-start
mechanism in §14.6 uses, driven a different way (Q-burst instead of a
level change). One instruction-boundary poll, one shared PIA input pin,
two different cartridges asserting it for two different reasons.

### Watching the whole chain fire

[`tests/rs232.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/rs232.rs)'s `rx_irq_fires_firq_via_pia1_cb1` drives every layer of
this in one test, with comments that read as a script for exactly the
chain just described:

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

Notice the test writes a byte to `ACIA_DATA` (the pak's loopback endpoint
echoes it straight back to the receiver), then has to run the bus for
`ROUND_TRIP_BUDGET` cycles before checking anything — the byte-level
frame timer from §14.3 really does take real cycles to complete, even in
a "loopback" test that never touches a socket. And notice the two-step
unwind at the end: reading the ACIA's status register clears *its* IRQ
output (CART* deasserts), but PIA1's *own* CB1 flag is separately latched
and stays set — asserting FIRQ — until something reads PIA1's port B
*data* register, exactly the "reading a data register clears the
interrupt flag" fact from week 1 and week 10, now shown mattering one
device downstream of where it lives. A companion test,
`machine_loop_polls_cart_interrupt`, proves the same chain fires through
the real `Machine::run_field` loop, not just the bus-level `run` helper
this test builds — confirming the per-instruction `poll_cart_interrupt`
wiring inside `step_cpu_unit` is the thing actually doing the work in a
running machine, not an artifact of the test's own harness.

---

## 14.5 Rung 3: the DMP-105 — a protocol on top

### Framing the layering

Nothing about the bit-banger or the 6551 knows a printer is on the other
end of the wire — both are pure byte-in/byte-out transports. `Dmp105` is
the layer that turns a byte stream into ink: it implements `PrinterSink`
(so it can plug into `BitBanger` exactly where `CaptureSink`/`FileSink`
did) and interprets every byte as either printable text or one of the
DMP-105's documented control codes, maintaining a print-head position and
drawing into a shared `Paper` model. `docs/printer-plan.md`'s own framing
for this, quoted in the module doc comment, is worth keeping in mind:
`crate::printer` holds the paper model *shared* across the whole DMP
family (a future DMP-130/Epson dialect would reuse it), while `dmp105.rs`
and its `protocol` submodule hold everything specific to this one
model's control-code dialect.

### The control-code interpreter

Every decoded byte flows through one entry point, `Dmp105::feed`, which
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

Two small `mod`s of named constants stand in for a dispatch table —
`control::` for bare byte codes (`LF`, `CR`, `SELECT_GRAPHICS`, the
`REPEAT` introducer `$1C`, …) and `esc::` for the byte immediately after
an `ESC` (`$1B`) introducer (`ELONGATE_START`, `POSITION`,
`PITCH_CONDENSED`, …). `begin_esc_operands` looks up how many operand
bytes a given selector needs — 0, 1, or 2 — entirely from a `match`, no
runtime table:

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

Character-Print-mode dispatch is a straight `match` over the byte's
value range, ASCII in the middle, undefined codes falling through to a
literal `X` glyph — exactly what the manual specifies ("Undefined codes
... print literal `X` in CP mode"):

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

Graphics mode is the interesting contrast: bit 7 set is *always* data
(the "data marker," never a control code, per the manual), and bit 7
clear falls through to a small set of recognized codes with **no**
undefined-code fallback glyph at all — the manual states outright that
undefined bytes are simply ignored inside Graphics mode, never printed.

### A real bug the tests remember: repeating the repeat introducer

The DMP-105's repeat sequence — `$1C n c`, "repeat byte `c` `n` times" —
looks at first like it should recurse straight through `feed`. It
deliberately doesn't:

```rust
/// The repeated byte is expanded through the per-mode dispatchers, NOT
/// through [`Dmp105::feed`]: inside a repeat, `c` is the datum being
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

The regression test names the exact failure mode this prevents:

```rust
/// Regression: `1C 1C 1C` (repeat of the repeat introducer) once
/// re-entered `feed` and rebuilt its own spawning state without bound —
/// a stack overflow on three bytes of arbitrary traffic. Inside a
/// repeat, `c` is data: an undefined CP code that prints the `X`
/// placeholder glyph, `n` times, and the state machine ends clean.
#[test]
fn repeating_the_repeat_introducer_terminates_and_prints_placeholders() {
    let mut dmp = Dmp105::default();
    feed_str(&mut dmp, &[control::REPEAT, 3, control::REPEAT]);
    assert_eq!(dmp.x, 3 * normal_cell_width());
    assert!(dmp.paper.extent().dot_count > 0); // three X glyphs
    feed_str(&mut dmp, b"A");
    assert_eq!(dmp.x, 4 * normal_cell_width());
}
```

This is exactly the class of hardware-fed vulnerability week 1's "fidelity
is a budget" table doesn't capture: it isn't about accuracy at all, it's
about *robustness against untrusted, hostile-shaped input* — a printer
byte stream is attacker-controlled the moment it comes from a socket
(§14.3's `SerialEndpoint`, or a `.wav`/`.cas` decode in week 12's world),
so the interpreter has to survive arbitrary bytes, not just documented
ones, without the emulator process itself crashing.

### Fixed-point units: why 1/3600" and 1/72"

Every position `Dmp105` tracks — head `x`, head `y` — is a plain integer,
never a float, and the two axes use *different* denominators chosen for
different reasons:

- **Vertical, `Y_UNITS_PER_INCH = 72`.** Every documented vertical fact
  in the DMP-105 manual is already a whole number of 1/72": the three
  text line-feed pitches are 1/6" (= 12 units), 1/8" (= 9 units), and
  1/12" (= 6 units), and the fixed graphics-mode line feed is 7/72" (= 7
  units, exactly). 1/72" is the *finest* unit that keeps every one of
  those an exact integer — no finer resolution is needed.
- **Horizontal, `X_UNITS_PER_INCH = 3600`.** This one isn't a hardware
  register at all — it's derived. The manual's Appendix G gives dots per
  8" print line directly: Normal 960, Compressed (12 CPI) 1152, Condensed
  (16.7 CPI) 1600 — which work out to 120, 144, and 200 dots per inch
  respectively (`960/8`, `1152/8`, `1600/8`). Each pitch's per-dot
  spacing in inches is `1/120`, `1/144`, `1/200`. `3600` is the **least
  common multiple** of 120, 144, and 200, chosen specifically so all
  three spacings become exact integers of the shared unit: `3600/120 =
  30`, `3600/144 = 25`, `3600/200 = 18` — three clean integers, one
  shared grid, and a pitch change mid-line (`1B 13`/`1B 14`/`1B 17`) never
  needs to round.

```rust
const fn dot_spacing(self) -> u32 {
    X_UNITS_PER_INCH / self.dots_per_inch()
}
```

Ask what the alternative would have cost: represent `x`/`y` as `f64`
inches instead. Every glyph column advance, every `LF`, every `1B 5A n`
immediate feed becomes a floating-point addition, and floating-point
addition is not associative — `(a + b) + c` and `a + (b + c)` can
produce different bit patterns. A print job is thousands of sequential
position updates; over that many additions, small representation errors
in fractions like `1/144` (which has no exact binary floating-point
representation, the same problem `0.1 + 0.2 != 0.3` demonstrates in every
language with IEEE floats) would accumulate into visible column drift by
the end of a long line — dots landing a fraction of a pixel off from
where the same byte stream landed the first hundred times, purely from
summation order or how many pitch changes happened along the way. Integer
arithmetic in a shared fixed-point unit has none of that: `30 + 30 + 30`
is exactly `90`, always, regardless of grouping, and the LCM choice
guarantees no pitch's per-dot step is ever a repeating fraction of the
unit in the first place. This is the same reasoning CPU cycle-counting
(§14.2, §14.3, and every timing-sensitive chapter before this one) is
built on, applied to spatial position instead of time: pick an integer
unit fine enough that every value you need to represent is exact, and
every operation on it stays exact by construction.

Position math saturates rather than overflowing, for the same
"survive arbitrary input" reason the repeat-introducer guard exists:

```rust
self.x = self.x.saturating_add(CELL_DOTS * col_step);
```

and marks past the physical 8" print zone are simply dropped
(`PRINT_WIDTH_X_UNITS = 8 * X_UNITS_PER_INCH`) — a stream with no `CR`
in it can't grow the paper model without bound, matching how a real
print head physically cannot move past the platen's edge.

### The paper model: a continuous roll

`Paper` stores marked dots as "bands" — one `BTreeMap` entry per row `y`,
holding the sorted-or-not list of `x` columns marked on it:

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
the choice that makes `dots_in_range` — "give me everything a scrolled
viewport needs to redraw" — an efficient range scan instead of a linear
filter over the whole roll, which matters once a paper roll has
accumulated pages of print history. There is deliberately no page or
form-feed concept anywhere in `Paper`: the module doc comment cites the
protocol spec's own finding that form feed is "VERIFIED ABSENT" from the
DMP-105's firmware entirely — no page-length register, no top-of-form.
An 11" page boundary, if a frontend wants to draw one, is purely a
rendering choice layered on top of an infinite roll, not a fact this
model tracks. `take_dirty` returns (and resets) the row range touched
since the last call — the cheapest possible "what changed" signal for a
live-updating paper-window frontend, relying on the fact that a real
print head's `y` only ever advances forward within one job, so nothing
below the dirty floor could possibly have changed.

### The font: data, and honestly labeled as guesswork

Week 7 built the VDG's MC6847 text font as *verified* hardware data —
real glyph bitmaps traceable to the chip. The DMP-105's font cannot be
that, and the module doc comment says so as its very first line:

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
actually prints (column-by-column, not row-by-row), with bits 0–6 as the
seven body rows and bit 7 as the descender-row dot. Glyphs are authored
as ASCII art and transposed at compile time via a `const fn`:

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

so a source comment like `"..#...#.."` reads visually as the glyph it
produces — far more reviewable than raw hex bit patterns, and the same
"author data the way a human actually checks it" instinct week 7's font
tables used. The honest labeling matters precisely because everything
*else* in this chapter has been hardware-verified against ROM
disassembly or a manufacturer manual; this is the one place the codebase
is candid that it had to guess, and it flags exactly which byte ranges
are guesses (`$A0`–`$BF` European symbols and `$E0`–`$FE` block graphics
are *placeholders*, not even authored guesses at real glyphs, because the
manual never transcribes their bitmaps at all — only the ASCII range
`$20`–`$7E` is a deliberate hand-authored rendering).

### End to end: `LLIST` through the whole stack

[`tests/dmp105_boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/dmp105_boot.rs) is the integration test that proves every layer
of this chapter's first two rungs plus this section actually compose:
boot the real `coco3.rom`, attach a `Dmp105Handle` as the bit-banger's
sink, type a one-line program, `LLIST` it, and check the paper picked up
plausible content:

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

This is deliberately a "shape" test, not a glyph-exact one — the module
doc comment is explicit that "glyph-exact assertions are the unit golden
tests' job," which live back in `dmp105_test.rs`
(`hello_cr_at_normal_pitch_produces_expected_glyph_columns_and_row` checks
`H`'s left stroke lands at exactly the seven expected rows, for
instance). What this integration test buys instead is proof that the
*wiring* works against unmodified ROM code end to end: real PIA1 DDR/CRA
setup, the real ROM's bit-bang transmit loop, the BUSY handshake (never
hanging), `BitBanger`'s decoder, and `Dmp105`'s interpreter, all chained
— the same "does the whole path actually compose" question
`bitbanger_boot.rs`'s `LLIST` test (§14.2) asked one layer down.

---

## 14.6 Elective: the cartridge system

### The trait as a plugin architecture

Every device you've read about since §14.4 — the RS-232 pak, and every
cartridge you're about to read about — implements one trait,
`Cartridge`. DESIGN.md named this seam back before any of it existed:
"make `Cartridge` a trait so a WD1773 floppy controller, plain ROM packs,
and the Multi-Pak slot all plug in." Read the trait's full surface
([`crates/coco-core/src/cart.rs:35–186`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cart.rs#L35-L186), doc comments trimmed for space)
to see exactly how many different kinds of "cartridge" it has learned to
express since that sentence was written:

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

`read`/`write` are the only two required methods with no default body —
everything else has a sane no-op default. That's what actually makes
this a *plugin* seam rather than a heavyweight interface every cartridge
has to fully implement: `RomPak` only needs `rom_read` and
`cart_line_ties_q` above the required two; `Gmc` adds `generator_sample`
for its PSG; only `DiskCart` needs `halt_asserted`/`take_nmi`; only the
SSC needs `audio_sample`. Two attribute categories are already familiar
from earlier chapters, wearing new names: `cart_line_ties_q`/
`cart_interrupt` (this section's whole point, below) are the FIRQ story;
`peek`/`rom_peek`/`peek_control` are week 16's side-effect-free debugger
twins of `read`/`rom_read`/`control_read`, arriving here because a
cartridge is exactly the kind of device (like a PIA, like the GIME) whose
plain reads can have side effects a debugger's memory viewer must not
trigger.

### Rust corner: a closed enum, not `Box<dyn Cartridge>`

Here's a genuine surprise if you've been reading `Cartridge` as "the
thing you'd obviously store as `Box<dyn Cartridge>`": the cartridge
actually sitting in the port is stored as a closed enum.

```rust
#[non_exhaustive]
#[derive(Serialize, Deserialize)]
pub enum Cart {
    Empty(EmptySlot),
    RomPak(RomPak),
    BankedRomPak(BankedRomPak),
    Gmc(Gmc),
    DiskCart(Box<crate::fdc::DiskCart>),
    MultiPak(Box<MultiPak>),
    Orch90(crate::orch90::Orch90),
    DistoRtc(crate::rtc::DistoRtc),
    DeluxeRs232(crate::rs232::DeluxeRs232),
    Ssc(Box<crate::ssc::Ssc>),
    #[serde(skip)]
    Custom(Box<dyn Cartridge>),
}
```

The comment at the top of `cart.rs` gives the reason directly: "an enum
is what lets `SystemBus`/`Machine` derive serde for save-states." This is
week 1's `#[derive(Serialize, Deserialize)]` story (§1.4) showing up at a
new layer: a trait object has no fixed, known-in-advance shape that
`serde` can generate a serializer for — `serde` needs to know, at compile
time, every concrete type that could be behind the pointer. An enum
*is* that closed list, spelled out. `Cart::Custom` is the deliberate
escape hatch for out-of-crate test doubles (you saw one, `TestCart`, in
[`tests/mpi.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/mpi.rs)) — and it's excluded from serialization entirely
(`#[serde(skip)]`), so a save-state attempted with a test double plugged
in fails cleanly (`Cart::contains_custom` checks for exactly this before
`crate::snapshot::save` ever tries).

Dispatch to the concrete type is a macro-generated `match`, not a
vtable call:

```rust
macro_rules! with_each_cart {
    ($self:expr, $cart:ident => $body:expr) => {
        match $self {
            Cart::Empty($cart) => $body,
            Cart::RomPak($cart) => $body,
            // ... one arm per variant ...
            Cart::Custom($cart) => $body,
        }
    };
}

impl Cart {
    pub fn read(&mut self, addr: u16) -> u8 {
        with_each_cart!(self, cart => cart.read(addr))
    }
    // ... eighteen more delegation methods, identical shape
}
```

Every arm except `Custom` resolves to a statically-known concrete type at
the call site — the same monomorphization story week 1's Rust corner
told about `impl Bus`, except here it's a `match` doing the dispatch
instead of generics, because the *set* of cartridge types is closed and
known, not "any type the caller supplies." Only `Cart::Custom` pays for a
genuine virtual call through its `Box<dyn Cartridge>`. Contrast this with
where `dyn Trait` genuinely *is* the right tool in this same codebase:
`BitBanger`'s `Box<dyn PrinterSink>` (§14.2) and `DeluxeRs232`'s
`Box<dyn SerialEndpoint>` (§14.3) both use open trait objects, because
those sinks/endpoints are meant to be extended by consumers of the crate
(a frontend's own printer-capture UI, a future host backend) without
`coco-core` needing to know about them in advance. `Cart`'s cartridge set,
by contrast, is finite and lives entirely inside this one crate — every
cartridge Tandy or a third party ever sold for a CoCo is a fixed,
enumerable list, so a closed enum is not a limitation here, it's a more
precise description of the actual problem, and it's what buys the whole
tree its free serialization.

### Auto-start: the Q-burst that boots a game pak instantly

You already have both halves of this mechanism from earlier chapters —
this section is where they get named as one story. `cart_line_ties_q`
is the *other* interrupt mechanism the `Cartridge` trait offers, sitting
right above `cart_interrupt` in the trait:

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
directly to the `Q` clock pin (pin 7) — a jumper, not a chip: the moment
the cartridge is plugged into a powered-on CoCo, CART* starts toggling at
the CPU's own ~895 kHz quadrature clock rate, continuously, with no
software involved at all on the cartridge side. `SystemBus::hsync`
samples this once per scanline — nowhere near 895 kHz, but the trait doc
comment's own justification is the right one: "the PIA/GIME only care
that *an* edge keeps arriving," and once per scanline is wildly more
than fast enough to keep either interrupt path continuously fed, so
there's no need to actually model sub-scanline timing here (contrast
this against §14.4's `poll_cart_interrupt`, which genuinely *is* polled
per-instruction, because a 6551's interrupt latency at high baud is
short enough that scanline granularity would be visible):

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

A full falling-then-rising edge, back to back, every single scanline —
the same "emit both edges together since we have no sub-line timing
resolution" trick week 10's `hsync` used for the HS pulse itself,
reused verbatim for the cartridge Q-burst.

Now the ROM side of the story, verified against a real disassembly
(`docs/cartridges.md`, cross-checked against Super Extended BASIC
Unravelled II and Bob Russell's memory map — not this codebase's own
invention). BASIC's reset code enables PIA1's CB1 FIRQ (`$FF23` bit 0);
the continuous Q-clock toggling then fires the ROM's default FIRQ
handler at `$A0F6`. On the CoCo 3, that handler's cartridge path
(`$A0FC`) calls a routine at `$8C28` which: clears the interrupt, writes
GIME `INIT0 = COCO|MMUEN|MC3|MC2` (i.e. `MC1=0`: the 16K-internal +
16K-external ROM map, with `MC2` enabling standard SCS), forces ROM mode,
and jumps to `$C000` — the external ROM window a cartridge's `rom_read`
answers. A non-autostarting cartridge (Disk BASIC, most utility paks)
reaches the same `$C000` a different way: BASIC's cold-start code
recognizes the disk controller ROM specifically by its `'D'`,`'K'`
signature bytes at `$C000`/`$C001`, and a human typing `EXEC &HE010`
reaches an arbitrary cartridge's entry point manually.

[`tests/cart.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/cart.rs) proves this whole chain fires against the real ROM, not
a synthetic stub — one autostart test, one negative control:

```rust
/// A pak whose code at $C000 writes [`MARKER_BYTE`] to [`MARKER_ADDR`] and
/// loops forever: `LDA #$A5 ; STA $0400 ; BRA *`
fn marker_pak(autostart: bool) -> RomPak {
    let mut image = vec![0u8; ROM_PAK_MAX_LEN];
    let program = [0x86, 0xA5, 0xB7, 0x04, 0x00, 0x20, 0xFE];
    image[CART_ENTRY_OFFSET..CART_ENTRY_OFFSET + program.len()].copy_from_slice(&program);
    RomPak::from_bytes(&image, autostart).unwrap()
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

The assertion message literally spells out the whole chain this section
just walked, cited by real ROM addresses: `CART* -> PIA1 CB1 -> FIRQ ->
$8C28 -> $C000`. If this were broken — say, `cart_line_ties_q` stopped
being polled, or `set_c1` stopped latching — this test wouldn't just
fail an assertion cleanly; the loop would run all 400 fields and the
`fired` flag would simply never flip, which is exactly why the test
bounds the field count instead of looping forever waiting for a marker
byte that would otherwise never arrive.

### The Multi-Pak Interface: one port, four slots

The MPI (26-3024) is a passive 4-slot expansion adapter — no chip of its
own beyond a 74-series decoder and an 8-bit select register at `$FF7F`.
The key hardware fact, stated once in the module doc comment and worth
internalizing before reading the code: **only three of the connector's
signals are actually switched per-slot** — SCS*, CTS*, and CART*. Every
other line (address bus, data bus, HALT*, NMI*, the clocks) is common to
all four slots simultaneously.

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
CART* line are live — those two lines follow each other, always, because
a real MPI switches them together. `read`/`write` route the SCS window
to `self.slots[self.scs_slot()]`, but `rom_read` and `cart_line_ties_q`
both route to `self.slots[self.cts_slot()]`:

```rust
fn cart_line_ties_q(&self) -> bool {
    self.slots[self.cts_slot()].cart_line_ties_q()
}
fn cart_interrupt(&mut self) -> bool {
    self.slots[self.cts_slot()].cart_interrupt()
}
```

— which is exactly why Disk-BASIC-era software could point SCS at the
disk controller slot while CTS/CART point at a game slot: the two
windows genuinely are independent selectors on real hardware, not a
single "active slot" concept. `HALT*`/`NMI*`/`tick`, by contrast, reach
*every* slot regardless of selection — the trait doc on `MultiPak::tick`
says it outright: "a device doesn't stop just because it isn't currently
addressed." An FD-502 sitting in slot 4 can still hold HALT* even while
the select register currently points SCS/CTS at slot 1; [`tests/mpi.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/mpi.rs)'s
`halt_and_nmi_are_wire_ored_across_all_slots_regardless_of_selection`
proves exactly this with a synthetic `TestCart`.

**Software-write blocking** is the switch/register interaction most
worth internalizing, because it's the kind of stateful hardware quirk
that's easy to get backwards:

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
`$FF7F` itself, at which point the switch is *locked out*: turning the
physical dial afterward still records the new position (so the next
reset picks it up) but has no live effect until RESET* actually fires.
[`tests/mpi.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/mpi.rs)'s `software_write_blocks_the_switch_until_the_next_reset`
walks exactly this sequence: a software write to slot 0, then
`set_switch(1)` (recorded but inert), confirmed still reading slot 0's
value, then `reset()`, which both restores switch control *and* reloads
the now-moved switch's value in one step.

### RomPak: mirror-fill, and the half-swap quirk

A ROM pak dump is a headerless raw file — the emulator community's
`.ccc`/`.rom`/`.bin` convention carries no size or banking metadata at
all; size *is* the metadata. Two facts about how such a dump maps onto
the CPU's 32K external ROM window are easy to get backwards on a first
try.

**Mirror-fill.** Real cartridge ROMs frequently don't decode every
address line — an 8K EPROM in a socket meant for up to 32K repeats every
8K across the window on real silicon, because the chip simply never
looks at the address bits above its own capacity. `mirror_fill` matches
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

[`tests/cart.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/cart.rs)'s `mirror_fill_equals_plain_repetition_for_any_size` is
the test that turns the comment's claim into a checked fact: for a
deliberately non-power-of-two size (5000 bytes), every one of the 32K
window's bytes equals `bytes[i % LEN]` — proving the doubling loop, kept
in MAME's own shape "so the provenance is obvious," really is just a more
efficient way to write plain modular repetition.

**The half-swap.** This is the genuinely surprising one. A 32K pak dump
is conventionally laid out CTS-window-first: file offset 0 is the byte
that should appear at `$C000`, and (for full 32K carts) file offset
`$4000` is the byte that appears at `$8000` once the GIME's INIT0 map
switches to 32K-external. But the GIME doesn't route cartridge banks in
that straightforward an order — its actual bank-routing formula (from
MAME's `gime.cpp`) is `((bank & 3) ^ 2) * 0x2000`, which swaps the two
16K halves relative to a naive `addr - $8000` index. `RomPak::rom_read`
reproduces that swap directly:

```rust
const ROM_PAK_HALF_SWAP: u16 = 0x4000;

fn rom_read(&mut self, addr: u16) -> u8 {
    self.image[((addr - ROM_PAK_BASE) ^ ROM_PAK_HALF_SWAP) as usize]
}
```

[`tests/cart.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/cart.rs)'s `full_32k_image_maps_cts_half_first` checks the
consequence directly: `rom_read(0xC000)` (the byte a `JMP $C000`
autostart landing would execute first) returns `bytes[0x0000]` — file
offset zero — while `rom_read(0x8000)` returns `bytes[0x4000]`, the file's
*second* half. Get the XOR backwards (or omit it) and the symptom is
specific and nasty: any ≤16K pak still works fine, because mirror-fill
makes both halves byte-identical and the bug is invisible — but a real
32K cart like Arkanoid, whose entry code genuinely lives at file offset 0
expecting to be fetched at `$C000`, would boot into garbage. This is
exactly the kind of bug that survives testing against small ROMs and
only surfaces against a specific large, real cartridge — which is why the
test constructs a full 32K image with `bytes[i] = i as u8` (every offset
individually distinguishable) rather than anything smaller.

Banked paks (`BankedRomPak`, the RoboCop/Predator circuit reused by the
GMC below) trade the fixed 32K window for a **16K** window that a
whole-byte write to `$FF40` slides across up to 128K of image — `bank *
16K mod 128K`, wrapping so an undersized image mirrors across the unused
upper banks exactly like a plain pak does. Interestingly, the 16K window
makes the half-swap bit *disappear* rather than needing separate
handling: `BANKED_PAK_WINDOW_LEN - 1` as the address mask discards
exactly the bit `ROM_PAK_HALF_SWAP` would have flipped, so
`window_mirrors_across_both_16k_halves_of_the_external_map` confirms the
same bank shows identically at `$8000`, `$A000`, and `$C000` with no
special-casing required in `BankedRomPak::rom_read` at all.

### The Games Master Cartridge: banked ROM plus a chip you already know

John Linville's GMC is, structurally, nothing but a `BankedRomPak` with
one extra write-only register:

```rust
const GMC_PSG_REG: u16 = 0xFF41;

pub struct Gmc {
    rom: BankedRomPak,
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

The SN76489A itself is week 11's chip, unchanged — the GMC's only
contribution is *plumbing* it into the cartridge port, at its own 4 MHz
crystal, alongside a bank-switched ROM window. One detail is worth
flagging as a documented MAME-fidelity choice rather than an oversight:
GMC audio mixes into the speaker *unconditionally*, bypassing the analog
sound mux's SEL-bit-gated cartridge-sound input entirely (the doc comment
notes MAME routes the GMC's PSG to a dedicated speaker device, ignoring
the SNDEN/mux path "entirely," with no independent schematic settling
what the real cartridge's SND-pin wiring should do instead — so this
model keeps MAME's behavior rather than inventing a mux-gated path with
no evidence behind it). [`tests/gmc.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/gmc.rs)'s `bank_latch_pages_the_16k_window`
confirms the inherited banking behaves exactly like a standalone
`BankedRomPak` — writing each of 8 bank values to `$FF40` and checking
`rom_read($C000)` returns that bank's marker byte — while a separate MPI
test confirms the PSG's audio genuinely plays "from any slot" (the analog
bus is common across an MPI's four slots, just like HALT*/NMI*) even
while a *different* slot answers the register writes.

---

## 14.7 Reading assignment

In this order:

1. **[`crates/coco-core/src/bitbanger.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger.rs), whole file** — the module doc
   comment first, then `tick`/`sample_threshold`. You have every fact
   needed to hand-derive the baud table in §14.2 from this file alone.
2. **[`crates/coco-core/src/acia6551.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551.rs) and its [`frame.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551/frame.rs)/[`registers.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551/registers.rs)/
   [`irq.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/acia6551/irq.rs) submodules** — read the module doc comment's "byte-level
   timing divergence" section slowly; it's the chapter's clearest single
   statement of the fidelity-is-a-budget philosophy applied to a chip
   this course hasn't met before.
3. **[`crates/coco-core/src/rs232.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/rs232.rs)** — small enough to read whole in
   one sitting; it's the chapter's best example of "a cartridge is just
   a chip wired to a bus," reusing every mechanism §14.6 names.
4. **[`crates/coco-core/src/dmp105.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/dmp105.rs) and [`dmp105/protocol.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/dmp105/protocol.rs)** — read
   `feed`/`dispatch_cp`/`execute_repeat` together; the recursion-avoidance
   comment on `execute_repeat` is worth re-reading after you've seen the
   regression test that motivated it.
5. **[`crates/coco-core/src/cart.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cart.rs) and [`cart/cart_enum.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cart/cart_enum.rs)** (elective) —
   the trait first, then `with_each_cart!`; compare against
   `BitBanger`'s `Box<dyn PrinterSink>` to feel the difference between
   "closed set, needs serde" and "open set, extended by consumers."

While reading, run the whole chapter's test surface:

```
cargo test -p coco-core --lib bitbanger:: acia6551:: dmp105:: serial::
cargo test -p coco-core --test bitbanger rs232 dmp105_boot cart mpi gmc
```

The last two integration tests in that second line (`bitbanger_boot`,
`bitbanger_os9`, `dmp105_boot`, and the real-ROM tests inside [`cart.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/cart.rs)/
[`mpi.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/mpi.rs)) need `roms/coco3.rom` (and [`bitbanger_os9.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bitbanger_os9.rs) additionally needs
`roms/disk11.rom` plus disk images under `disks/`) — none of which ship
in this repository or in a fresh worktree; they're git-ignored and
present only on the machine this course was authored on. Every one of
those tests checks for the asset and prints `eprintln!("skipping ...")`
and returns cleanly if it's missing, rather than failing — you'll see
that pattern (`load_rom()`/`try_load_rom()` returning `Option`) at the
top of each file. The pure unit and bus-level tests ([`bitbanger_test.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bitbanger_test.rs),
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
during. Check your trace against a test you write using `Dmp105::feed`
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
`esc::ELONGATE_END` in [`dmp105/protocol.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/dmp105/protocol.rs)) but exercise it yourself:
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

Week 15 leaves the core entirely and spends a week on `coco-egui`, the
frontend: the per-frame loop, input routing, the VM manager, and headless
UI testing with kittest. One thread from this chapter continues there —
the DMP-105's *paper*, as an actual on-screen scrolling view a user can
watch fill up while `LLIST` runs, is week 15's to build; everything about
what goes *onto* that paper (the protocol, the fixed-point coordinate
system, the font) was this chapter's. Week 16 closes the course with the
debugger and save states — and now that you've read `Cart`'s enum-vs-trait-
object story, you already understand *why* the whole cartridge tree
(RomPak images excepted — copyrighted bytes, reattached separately) rides
along for free in every `.ccstate` snapshot.
