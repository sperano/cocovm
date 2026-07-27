# Chapter 10 — The PIAs, the Keyboard Matrix, and the Joystick ADC-by-Comparator

*Week 10. Goal: understand the chip that mediates almost all CoCo I/O. Parts
I–III gave you a CPU, a bus, and a GIME that can paint a screen — an
impressive machine that still can't hear you. Every key you type and every
joystick wiggle you make on a real CoCo passes through a pair of 1977-vintage
parallel-port chips before the CPU ever sees it, and one of those chips also
happens to generate the interrupt that keeps stock BASIC's idle loop alive.
This week you finally deliver the promise made in week 1: reading a PIA data
register clears an interrupt flag, and that single fact is why `Bus::read`
takes `&mut self`. By the end you'll be able to trace a keypress from a
finger on a keycap to a character on the screen, and explain why the CoCo's
"joystick port" contains no analog-to-digital converter at all.*

---

## 10.1 The chip that mediates almost all I/O

Open [`crates/coco-core/src/pia.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/pia.rs). Its header comment is the whole chapter
in five lines:

```rust
//! MC6821 Peripheral Interface Adapter. See `DESIGN.md` §7.
//!
//! Models the two independent sides (A/B), each with an output register, a data
//! direction register, a control register, and the Cx1 interrupt line. The
//! DDR/data access split (control bit 2) and the Cx1 flag (bit 7) cleared by
//! reading the data register are what the CoCo relies on: the 60 Hz field sync
//! and the horizontal sync are wired to PIA0's CB1/CA1 and drive the CPU IRQ.
//!
//! Cx2 is modelled as a set/reset output only (`c2_output`, the mode the CoCo
//! uses for the joystick mux and sound enable); Cx2 interrupt-input and
//! handshake/pulse strobe modes are not modelled.
```

The Motorola MC6821 is not a CoCo-specific chip — it's a general-purpose
"peripheral interface adapter" that shipped in an enormous fraction of
1977–1985 8-bit hardware, because it solved a problem every home computer
had: the CPU has 16 address pins and 8 data pins, and *everything else* —
keyboards, printers, joysticks, cassette decks — speaks in raw parallel
signals or single bits, not memory addresses. The PIA is the adapter: on one
side, four CPU-addressable registers; on the other, sixteen general-purpose
pins (two 8-bit "sides," A and B) plus four interrupt/handshake lines (CA1,
CA2, CB1, CB2). Tandy wired two of them into the CoCo — PIA0 at `$FF00` and
PIA1 at `$FF20` — and hung almost every non-video, non-disk peripheral off
those thirty-two pins: the keyboard matrix, both joystick ports, the 6-bit
sound DAC, the cassette line, the printer's bit-banged serial line, and (on
CoCo 1/2) the VDG's mode-select bits. When your BASIC program did `PRINT
PEEK(65280)` — `65280` is `$FF00` — it was reading PIA0 directly.

One CPU-facing register layout serves both sides, mirrored:

| Offset | Register | Side |
|--------|----------|------|
| `+0` | Port A data/DDR | A |
| `+1` | Control register A (CRA) | A |
| `+2` | Port B data/DDR | B |
| `+3` | Control register B (CRB) | B |

That's four registers, decoded by two address bits (`addr & 0x03`) — and
that detail matters more than it looks like it should, as you'll see in
§10.2. The Rust model mirrors the hardware shape exactly:

```rust
/// One side (A or B) of an MC6821.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiaPort {
    /// Output register (drives the pins selected as outputs by `ddr`).
    pub output: u8,
    /// Data direction register — 1 bit = output pin, 0 = input pin.
    pub ddr: u8,
    /// Control register (CRA/CRB).
    pub control: u8,
    /// State of the input pins (what the outside world drives).
    pub input: u8,
    /// Current level of the Cx1 line, tracked so [`PiaPort::set_c1`] can tell
    /// an edge from a repeated level. Idle high (MAME `6821pia.cpp`).
    c1_level: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MC6821 {
    pub a: PiaPort,
    pub b: PiaPort,
}
```

`MC6821` — one type, instantiated twice (`pia0`, `pia1` on `SystemBus`).
Everything Tandy wired to a PIA pin, in this codebase, ends up either
setting `port.input` before a read or reading `port.output`/`port.c2_output()`
after a write. That's the entire seam between "generic 1977 parallel-port
chip" and "specific CoCo peripheral" — and it's why the same 180-line file
you're about to finish reading underlies the keyboard (this chapter), the
sound DAC (week 11), the cassette relay and record line (week 11–12), and
the printer busy line (week 14). PIA1 is the front door every one of those
subsystems walks through; this chapter builds the door, and later chapters
walk through it without re-deriving how it opens.

> **A note on what's *not* modelled.** The doc comment above is explicit:
> Cx2 interrupt-input mode and handshake/pulse-strobe mode don't exist in
> this emulator. The real MC6821 can configure Cx2 as an *input* that
> latches its own interrupt flag, or as an output that auto-pulses on a
> data-register access (a full handshake protocol for printers). The CoCo's
> ROM never uses those modes — Cx2 is always programmed as a plain
> set/reset output (mux selects, DAC/relay enables) — so the emulator
> doesn't pay for logic nothing exercises. This is the fidelity-budget
> discipline from week 1 (§1.6) in miniature: model what software can
> observe, not what the datasheet allows.

---

## 10.2 One address, two registers: the DDR-access bit

Here's the first real puzzle a PIA presents. Port A needs to be
*configurable* — some pins are inputs (keyboard rows, the joystick
comparator), some are outputs (keyboard column strobes on port B, the sound
DAC on PIA1 port A) — and that configuration is a per-bit **data direction
register (DDR)**. But the PIA only has one CPU-visible address per side.
Two registers, one address. The 6821's answer is a steal bit in the control
register:

```rust
pub mod cr {
    /// Cx1 interrupt enable — 1 = an active Cx1 edge asserts IRQ.
    pub const C1_IRQ_ENABLE: u8 = 0x01;
    /// Cx1 active-edge select — 0 = high→low, 1 = low→high.
    pub const C1_EDGE_HIGH: u8 = 0x02;
    /// Data/DDR access select — 1 = data register, 0 = data-direction register.
    pub const DDR_ACCESS: u8 = 0x04;
    /// Cx2 output level when bits 5:4 select set/reset output mode.
    pub const C2_SET: u8 = 0x08;
    /// Cx2 output-mode select — with [`C2_OUTPUT`], 1 = set/reset (static level
    /// from [`C2_SET`]), 0 = handshake/pulse strobes (unmodelled).
    pub const C2_SET_RESET: u8 = 0x10;
    /// Cx2 direction — 1 = Cx2 is an output pin.
    pub const C2_OUTPUT: u8 = 0x20;
    /// Cx2 interrupt flag (read-only). Modelled as storage only.
    pub const C2_FLAG: u8 = 0x40;
    /// Cx1 interrupt flag (read-only) — set by an active Cx1 edge.
    pub const C1_FLAG: u8 = 0x80;
}
```

Bit 2 of the control register (`DDR_ACCESS`) decides what a read or write
to the *data* address actually touches:

```rust
fn read_side(port: &mut PiaPort) -> u8 {
    if port.control & cr::DDR_ACCESS != 0 {
        // Reading the peripheral data register clears the interrupt flags.
        port.control &= !(cr::C1_FLAG | cr::C2_FLAG);
        port.data()
    } else {
        port.ddr
    }
}

fn write_side(port: &mut PiaPort, val: u8) {
    if port.control & cr::DDR_ACCESS != 0 {
        port.output = val;
    } else {
        port.ddr = val;
    }
}
```

Bit clear → the data address means the DDR. Bit set → it means the live
data register (reads sense pins / drive outputs). Same address, two
registers, one bit deciding which one you're actually touching this
instruction.

### A real ROM sequence, decoded byte by byte

That's the theory. Here's the real thing — Color BASIC's cold-start PIA
initialization, from `color-basic-unravelled.pdf`, disassembled directly
from the ROM:

```
A027 31 8C E4      RESVEC  LEAY LA00E,PC        POINT Y TO WARM START CHECK CODE
A02A 8E FF 20      LA02A   LDX   #PIA1           POINT X TO PIA1
A02D 6F 1D                 CLR   -3,X            CLEAR PIA0 CONTROL REGISTER A
A02F 6F 1F                 CLR   -1,X            CLEAR PIA0 CONTROL REGISTER B
A031 6F 1C                 CLR   -4,X            SET PIA0 SIDE A TO INPUT
A033 CC FF 34              LDD   #$FF34          *
A036 A7 1E                 STA   -2,X            * SET PIA0 SIDE B TO OUTPUT
A038 E7 1D                 STB   -3,X            * ENABLE PIA0 PERIPHERAL REGISTERS, DISABLE PIA0
A03A E7 1F                 STB   -1,X            * MPU INTERRUPTS, SET CA2, CA1 TO OUTPUTS
A03C 6F 01                 CLR   1,X             CLEAR CONTROL REGISTER A ON PIA1
A03E 6F 03                 CLR   3,X             CLEAR CONTROL REGISTER B ON PIA1
A040 4A                    DECA                  A REG NOW HAS $FE
A041 A7 84                 STA   ,X              BITS 1-7 ARE OUTPUTS, BIT 0 IS INPUT ON PIA1 SIDE A
A043 86 F8                 LDA   #$F8            =
A045 A7 02                 STA   2,X             = BITS 0-2 ARE INPUTS, BITS 3-7 ARE OUTPUTS ON B SIDE
A047 E7 01                 STB   1,X             * ENABLE PERIPHERAL REGISTERS, DISABLE PIA1 MPU
A049 E7 03                 STB   3,X             * INTERRUPTS AND SET CA2, CB2 AS OUTPUTS
```

Notice the addressing: X points at `PIA1` (`$FF20`), and the *first* six
instructions use **negative offsets** — `-4,X` through `-1,X` — to reach
`$FF1C`–`$FF1F`. Those aren't PIA1 registers. `$FF00`–`$FF03` is PIA0's real
address, but the chip only decodes two address lines (`addr & 0x03`); every
other address bit is unwired, so PIA0 also answers at `$FF04`, `$FF08`, …
all the way to `$FF1C`–`$FF1F`. [`bus/regs.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/regs.rs) documents exactly this:

```rust
// I/O page device ranges (`DESIGN.md` §3). PIA0/PIA1 mirror every 4 bytes.
pub(super) const IO_BASE: u16 = 0xFF00;
pub(super) const PIA0_LAST: u16 = 0xFF1F;
pub(super) const PIA1_BASE: u16 = 0xFF20;
```

and `io_read`/`io_write` ([`crates/coco-core/src/bus/io.rs:61,100`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L61-L100)) mask every
address in `IO_BASE..=PIA0_LAST` down to `addr & 0x03` before dispatching to
the PIA. The ROM's author knew this and used it: with X already pointing at
PIA1, `-4,X` through `-1,X` reach PIA0's last mirror for free, saving four
bytes of `LDX #PIA0` setup. This is not an emulator implementation detail —
it's a real hardware fact (incomplete address decode, cheaper in 1980
silicon than routing two more address lines) that a real ROM exploits, and
the emulator has to get the mirroring right or this exact sequence would
misbehave.

Now walk what it actually does, register by register, using PIA0's
canonical addresses (`$FF1D`&`0x03`=1=CRA, `$FF1F`&`0x03`=3=CRB, `$FF1C`&
`0x03`=0=port A, `$FF1E`&`0x03`=2=port B — the mirror behaves exactly like
the real address):

1. `CLR $FF1D` → PIA0 **CRA** = 0. `DDR_ACCESS` bit clear: the data address
   now means the DDR.
2. `CLR $FF1F` → PIA0 **CRB** = 0. Same, for side B.
3. `CLR $FF1C` → PIA0 **port A DDR** = 0 → every port A pin is an input.
   This is the keyboard-row/joystick-comparator side; it must be
   input-only.
4. `LDD #$FF34` loads `A=$FF, B=$34` in one instruction — a common 6809
   trick when you need two different byte constants immediately after each
   other.
5. `STA $FF1E` → PIA0 **port B DDR** = `$FF` → every port B pin is an
   output. This is the keyboard column-strobe side.
6. `STB $FF1D` → PIA0 **CRA** = `$34` (`0b0011_0100`). Decode it against
   the `cr` constants: `DDR_ACCESS` (bit 2) is now **set** — the data
   address flips back to meaning live data, not the DDR, so every
   subsequent access to `$FF00` reads/drives real pins, not
   configuration. `C2_SET_RESET`+`C2_OUTPUT` (bits 5:4) select CA2 as a
   static set/reset output; `C2_SET` (bit 3) clear means that output
   starts low. `C1_EDGE_HIGH` clear selects the falling edge for CA1 (the
   value the rest of this chapter will assume for CA1/CB1 unless a test
   says otherwise).
7. `STB $FF1F` → PIA0 **CRB** = `$34`, same shape for side B (CB2 low,
   falling-edge CB1, data-register access restored).

Then PIA1, the same three-step dance (clear CR → program DDR while it's
still exposed → write CR again to flip back to data mode and configure
Cx1/Cx2), landing on port A DDR `$FE` (bit 0 — cassette data in — stays
input; bits 1–7 — serial out, DAC — become outputs) and port B DDR `$F8`
(bits 0–2 — printer busy, RAMSZ — stay input; bits 3–7 — VDG mode bits —
become outputs).

That three-step shape — **clear CR to expose the DDR, program direction,
write CR again to select the data register and arm Cx1/Cx2** — is the
canonical PIA initialization idiom, and you'll recognize it instantly
now in every ROM listing for the rest of the course.

> **Rust corner: overriding `Default` for hardware truth, not zero.**
> `PiaPort` derives nothing for its `Default`; it has a hand-written
> `impl`:
>
> ```rust
> impl Default for PiaPort {
>     fn default() -> Self {
>         // Idle input pins float high on the CoCo (keyboard rows read $FF = no key).
>         Self { output: 0, ddr: 0, control: 0, input: 0xFF, c1_level: true }
>     }
> }
> ```
>
> `#[derive(Default)]` would have given every field `0` — including
> `input`, and a `0x00` input register means "every pin sensed low," i.e.
> every key on every row simultaneously pressed at power-on. Real CMOS
> input pins with nothing driving them don't settle at ground; they float,
> and on the CoCo they read as logic-high (`0xFF`) until something pulls a
> line low. The derive macro can't know that — it only knows "zero is a
> valid default for every primitive." Whenever the zero value of a type
> isn't the hardware-true reset state, you write `Default` by hand. You'll
> see the identical pattern reasoning in week 11 (idle audio DAC level) and
> week 13 (idle disk-controller status bits) — check the field comments,
> not just whether a struct derives `Default`.

---

## 10.3 Edge detection, and the promise from week 1

Now the part `ch01`/`ch05` promised in full: *reading a PIA data register
clears an interrupt flag*. Here is the mechanism from the ground up.

CA1 and CB1 are single-bit interrupt-request inputs — real wires from the
outside world into the PIA. The CoCo wires PIA0's CA1 to the horizontal
sync pulse and PIA0's CB1 to the field (vertical) sync pulse. The PIA
doesn't care about "sync" — all it sees is a digital line transitioning
high or low, and it has to decide, per transition, whether *that specific
edge* (rising or falling, whichever the control register selected) should
raise a flag the CPU can see and optionally an interrupt:

```rust
/// Drive the Cx1 line to `level`, latching the Cx1 flag only on a real
/// transition whose direction matches CRA/CRB bit 1 ([`cr::C1_EDGE_HIGH`]):
/// bit1=1 selects low→high, bit1=0 selects high→low. A call that repeats
/// the current level (no edge) never sets the flag, and an edge in the
/// non-selected direction doesn't either — matching MAME `6821pia.cpp`
/// `c1_low_to_high`/`c1_high_to_low` (~line 1107): the flag sets iff
/// `(m_in_c1 != state) && ((state && rising_selected) || (!state &&
/// falling_selected))`. The CoCo wires horizontal sync to PIA0 CA1 and
/// field sync to PIA0 CB1.
pub fn set_c1(&mut self, level: bool) {
    let transitioned = self.c1_level != level;
    self.c1_level = level;
    if !transitioned {
        return;
    }
    let rising_selected = self.control & cr::C1_EDGE_HIGH != 0;
    if level == rising_selected {
        self.control |= cr::C1_FLAG;
    }
}
```

Read it in three steps: (1) if the new level equals the level already
stored, nothing happened electrically — return, no flag. (2) Otherwise a
real edge occurred; remember the new level either way (the level must be
tracked even when the edge doesn't match, or the next opposite transition
would look like "no change"). (3) `level == rising_selected` is a compact
way to ask "did the direction of *this* edge match the direction the
control register asked for?" — `rising_selected` is `true` when CRA/CRB bit
1 asked for low→high; `level` is `true` exactly when the transition just
completed was a rise. If they agree, the edge the software cares about just
happened, so set bit 7, the Cx1 interrupt flag.

That flag is read-only from the CPU's perspective — software can never
directly clear it with a write:

```rust
fn write_control(port: &mut PiaPort, val: u8) {
    // Bits 7/6 are read-only interrupt flags; the CPU can't set them.
    const WRITABLE: u8 = !(cr::C1_FLAG | cr::C2_FLAG);
    port.control = (port.control & !WRITABLE) | (val & WRITABLE);
}
```

So how does software ever clear it? Only one way — and it's the hardware
protocol the whole first chapter of this course was leading up to:

```rust
fn read_side(port: &mut PiaPort) -> u8 {
    if port.control & cr::DDR_ACCESS != 0 {
        // Reading the peripheral data register clears the interrupt flags.
        port.control &= !(cr::C1_FLAG | cr::C2_FLAG);
        port.data()
    } else {
        port.ddr
    }
}
```

**Reading the data register is the acknowledgment.** Not a separate
"clear interrupt" register, not a write to the control register — the mere
act of the CPU reading `$FF00` (or `$FF02`) resets both interrupt flags for
that side, as a side effect of the load instruction. This is real 1977
silicon behavior, not an emulator convenience: the real 6821's internal
flag flip-flops are wired to reset on the trailing edge of the E clock
during a peripheral-register read cycle. Software doesn't "handle" the
interrupt and then separately "acknowledge" it — reading the data is
simultaneously *getting the value that changed* and *telling the chip you
got it*.

This is exactly why `Bus::read` in [`crates/mc6809/src/lib.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs) takes
`&mut self` (week 1, §1.3): `LDA $FF02` looks, syntactically, like a pure
load — but on real hardware and in this emulator it mutates `PiaPort.control`.
If `read` took `&self`, this method would need `Cell` or `RefCell` to
compile, and the honest fact "this load has a side effect" would be hidden
from the type signature instead of visible in it. Full circle: the reason
you learned about `&mut self` reads in week 1 was this exact chip.

> **Rust corner: a masked match still needs a wildcard arm.** Look at
> `MC6821::read`:
>
> ```rust
> pub fn read(&mut self, reg: u8) -> u8 {
>     match reg & 0x03 {
>         0 => Self::read_side(&mut self.a),
>         1 => self.a.control,
>         2 => Self::read_side(&mut self.b),
>         _ => self.b.control,
>     }
> }
> ```
>
> `reg & 0x03` can only ever produce `0`, `1`, `2`, or `3` — a human reading
> the expression can see that instantly. The Rust compiler cannot: `&` on a
> `u8` still has type `u8`, whose full range is `0..=255`, and nothing in
> the type system narrows it after a bitwise-and. So exhaustiveness
> checking still demands a catch-all arm, and `_ => self.b.control` is it
> (the fourth, unreachable-in-practice case is folded into the same arm as
> `3`, which is harmless since they're identical). This is worth
> internalizing as a pattern, not a quirk: whenever you mask an integer
> down to a known-small range for a `match`, you're asserting a fact the
> compiler can't verify, so the wildcard arm is not dead code to trim —
> it's the compiler's insurance policy, and it should return something
> sane (here, the same thing arm `3` would) rather than `unreachable!()`,
> because on real, buggy, or adversarial input `reg` really can be
> anything.

---

## 10.4 The interrupt story completed: two heartbeats

Week 6 (§6, [`bus/sync.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sync.rs)) introduced the field-sync-on-PIA0-CB1 IRQ path as
the thing that breaks stock BASIC out of its idle loop; this section is
where you learn exactly *how* the PIA turns a raster timing event into a
CPU interrupt, and what the ROM does about it.

`SystemBus::hsync` fires once per scanline. Read it now that you know what
`set_c1` does:

```rust
pub fn hsync(&mut self) {
    // No GIME on the plain-SAM path (CoCo 1/2): the PIA0/PIA1 Cx1 pulses
    // below stay exactly as-is, but the GIME border/keyboard/cart-EI0
    // interrupt sources it would also raise here don't exist — the GIME
    // struct must stay completely inert on that path
    // (`docs/coco12-plan.md` Phase 4).
    let is_gime = self.variant == MachineVariant::Coco3;
    self.pia0.a.set_c1(false);
    if is_gime {
        self.gime.raise(gime::intr::HBORD);
    }
    self.pia0.a.set_c1(true);
    // Buttons are included: SEB warns joystick fire buttons always trip EI1.
    let line_low = self.pia0_pa_pins() & 0x7F != 0x7F;
    if is_gime && line_low && !self.kbd_line_low {
        self.gime.raise(gime::intr::EI1);
    }
    self.kbd_line_low = line_low;
    if self.cart.cart_line_ties_q() {
        self.pia1.b.set_c1(false);
        if is_gime {
            self.gime.raise(gime::intr::EI0);
        }
        self.pia1.b.set_c1(true);
    }
}
```

`self.pia0.a.set_c1(false)` then `set_c1(true)` — a falling edge
immediately followed by a rising edge, back to back, once per scanline.
This models the real HS pulse (idle high, low for ~4.5 µs at line end)
without needing sub-scanline timing resolution: whichever edge direction
CRA selected — stock BASIC's default falling edge, or NitrOS-9's
rising-edge convention — exactly one of those two calls sets the flag, so
every scanline produces exactly one CA1 flag regardless of which polarity
software asked for. A CoCo's horizontal line period is about 63.5 µs (the
NTSC line rate), so PIA0 CA1 ticks at roughly 15.7 kHz — far too fast for
software to want as an interrupt source on its own, which is exactly why
BASIC doesn't enable CA1's interrupt; it only polls it (more on that
below).

Field sync is the other heartbeat, and it's `Machine::run_field`
([`crates/coco-core/src/machine/run.rs:132-139`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs#L132-L139), week 6's file) that fires
it at the right line, not `hsync` itself:

```rust
let fs_falling_line = self.config.video.fs_falling_line(self.config.variant);
let fs_rising_line = self.config.video.fs_rising_line(self.config.variant);
self.bus.hsync();
if self.line == fs_falling_line {
    self.bus.fs_falling();
}
if self.line == fs_rising_line {
    self.bus.fs_rising();
}
```

and the two edges the PIA side sees:

```rust
/// Field-sync falling edge (~60/50 Hz vertical, at
/// [`crate::config::VideoStandard::fs_falling_line`] scanlines into the
/// field, not at end-of-field): latches PIA0 CB1 (control reg $FF03, port
/// B) — the interrupt that drives BASIC's housekeeping loop — per its
/// selected edge, and raises the GIME VBORD source (Lomont: "VBORD
/// generated on falling edge of VSYNC").
pub fn fs_falling(&mut self) {
    self.pia0.b.set_c1(false);
    if self.variant == MachineVariant::Coco3 {
        self.gime.raise(gime::intr::VBORD);
    }
}

/// Field-sync rising edge, at
/// [`crate::config::VideoStandard::fs_rising_line`] scanlines into the
/// field: latches PIA0 CB1 per its selected edge (e.g. NitrOS-9-style
/// rising-edge polling). No GIME border source is tied to this edge.
pub fn fs_rising(&mut self) {
    self.pia0.b.set_c1(true);
}
```

Note both edges genuinely happen at different scanlines, not back-to-back
like `hsync`'s HS pulse — `config.rs` derives NTSC line 244 for the CoCo 3's
falling edge and line 248 for the rising edge, so there's a real gap where
CB1 sits low, matching the real vertical-blank interval's shape. This is
the field-sync interrupt week 6 told you stock BASIC idles on
(`self.pia0.irq()` feeds `SystemBus::irq_asserted`, wired-OR with the
GIME's own IRQ output):

```rust
pub fn irq_asserted(&self) -> bool {
    self.pia0.irq() || self.gime.irq_asserted()
}
```

### What the ROM's handler actually does

Here is Color BASIC's IRQ service routine, reached through the interrupt
vector, disassembled from the ROM:

```
A9B3 B6 FF 03      BIRQSV LDA   PIA0+3      CHECK FOR 60HZ INTERRUPT
A9B6 2A 0D                BPL   LA9C5       RETURN IF 63.5 MICROSECOND INTERRUPT
A9B8 B6 FF 02              LDA   PIA0+2     RESET PIA0, PORT B INTERRUPT FLAG
A9BB BE 00 8D              LDX   >SNDDUR    GET INTERRUPT TIMER (SOUND COMMAND)
A9BE 27 05                 BEQ   LA9C5      RETURN IF TIMER = 0
A9C0 30 1F                 LEAX  -1,X       DECREMENT TIMER IF NOT = 0
A9C2 BF 00 8D              STX   >SNDDUR    SAVE NEW TIMER VALUE
A9C5 3B            LA9C5   RTI              RETURN FROM INTERRUPT
```

Read it against everything above. `LDA PIA0+3` (`$FF03`, PIA0's control
register B) is a plain read of the control register — reading *the control
register* never clears a flag (`read`'s `1 =>`/`_ =>` arms just return
`port.control` verbatim); only a *data*-register read does. So this
instruction is purely diagnostic: it fetches CRB's bits, and `BPL`
(branch-if-plus, i.e. bit 7 clear) tests whether the CB1 flag — bit 7,
`cr::C1_FLAG` — is set. If it's *not* set, the interrupt that woke the CPU
wasn't the 60 Hz field sync; it must have been the every-scanline CA1
horizontal sync instead, which BASIC has no work to do for, so it returns
immediately with `RTI`.

If bit 7 *is* set, the very next instruction, `LDA PIA0+2`, reads PIA0's
**port B data register** (`$FF02`) — and per `read_side`, that single load
clears both `C1_FLAG` and `C2_FLAG` on side B. The comment in the ROM
listing says it outright: "RESET PIA0, PORT B INTERRUPT FLAG." The value
loaded (the keyboard column-strobe byte, since port B's data register is
what this whole chapter's keyboard-scan machinery drives) isn't even used —
`A` is immediately clobbered by the next `LDX`. The read exists *purely for
its side effect*. This is the ROM depending on exactly the mechanism §10.3
just walked through, and it's the single clearest illustration in the whole
codebase of "a memory read that changes machine state."

Everything after that is bookkeeping unrelated to the PIA (decrementing a
sound-duration timer) — the field-sync interrupt on a stock CoCo doesn't
do much *work*; its entire job, from BASIC's point of view, is to fire
sixty times a second and thereby let a `BRA *`-style idle loop notice time
has passed. That's the "it's alive" heartbeat from week 6, and now you've
seen both ends of the wire: the raster hardware asserting CB1 in
`fs_falling`, and the ROM's own handler acknowledging it with the read this
section exists to explain.

---

## 10.5 The keyboard: a 7×8 matrix, electrically

The CoCo keyboard is not sixty independent switches wired to sixty pins —
that would need sixty I/O pins the machine doesn't have. It's a **matrix**:
56 keys wired at the intersections of 7 row lines and 8 column lines, so 15
pins (7+8) can sense 56 switches. `keyboard.rs`'s header comment gives the
authentic layout (cross-checked against MAME's `coco3_keyboard`):

```rust
//! CoCo 3 keyboard matrix (`DESIGN.md` §7).
//!
//! The keyboard is a 7-row × 8-column matrix. The CPU strobes columns by writing
//! PIA0 port B ($FF02, active low) and senses rows by reading PIA0 port A ($FF00,
//! active low; PA7 is the joystick comparator, not a key). Layout and the CoCo
//! shift semantics are the authentic matrix (cross-checked against MAME's
//! `coco3_keyboard`):
//!
//! ```text
//!         PB0    PB1    PB2    PB3   PB4    PB5   PB6    PB7
//! PA0:    @      A      B      C     D      E     F      G
//! PA1:    H      I      J      K     L      M     N      O
//! PA2:    P      Q      R      S     T      U     V      W
//! PA3:    X      Y      Z      up    down   left  right  space
//! PA4:    0      1      2      3     4      5     6      7
//! PA5:    8      9      :(*)   ;(+)  ,(<)   -(=)  .(>)   /(?)
//! PA6:    ENTER  CLEAR  BREAK  ALT   CTRL   F1    F2     SHIFT
//! ```
```

A key at position `(row, col)` is a switch bridging row wire `row` to
column wire `col` when pressed. Nothing energizes the matrix on its own —
the CPU has to *drive* one side and *sense* the other, one column at a
time. That's the classic tradeoff of a scanned matrix: fewer pins, at the
cost of software having to do the scanning in a loop instead of reading
one register.

Both sides are **active-low**, a near-universal convention in TTL-era
hardware because pulling a line to ground is electrically cheap and a
pull-up resistor holds it high by default: a column is "selected" by
driving its PB bit to 0, and a row reads 0 exactly when a held key
connects it through a strobed-low column. `PA7` is carved out — it's not a
key at all, it's the joystick comparator bit (§10.7).

The live matrix state is about as simple a struct as this codebase gets:

```rust
/// The live matrix state: `rows[r]` has bit `c` set when key `(r, c)` is held.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Keyboard {
    rows: [u8; ROWS],
}
```

Seven bytes, one per row, each bit a column. `set` flips a bit:

```rust
pub fn set(&mut self, pos: Pos, down: bool) {
    let (row, col) = (pos.0 as usize, pos.1);
    if row >= ROWS || col as usize >= COLS {
        return;
    }
    if down {
        self.rows[row] |= 1 << col;
    } else {
        self.rows[row] &= !(1 << col);
    }
}
```

This is the frontend's entire write surface into the emulated keyboard —
`coco-egui` never touches a PIA register directly; it calls `set` (§10.8).

### `sense()`, walked bit by bit

Everything the CPU actually experiences funnels through one function:

```rust
/// Compute the PIA0 port-A row sense for a given port-B column strobe.
///
/// Both are active low: a column is selected when its `strobe` bit is 0, and a
/// sensed row reads 0 when a held key connects it to a selected column. PA7 is
/// left high (joystick comparator).
pub fn sense(&self, strobe: u8) -> u8 {
    let selected = !strobe; // 1 = column currently strobed low
    // Rows 0..6 sense keys; PA7 (bit 7) stays high (joystick comparator).
    let mut pa = 0xFF;
    for (r, &pressed) in self.rows.iter().enumerate() {
        if pressed & selected != 0 {
            pa &= !(1 << r);
        }
    }
    pa
}
```

Trace it for one key. Say the user is holding `(row=0, col=1)` — the 'A'
key — and the ROM strobes column 1 (writes `$FD` = `0b1111_1101` to port
B, every bit high except bit 1, which is driven low):

1. `strobe = 0b1111_1101`. `selected = !strobe = 0b0000_0010` — a 1-bit
   mask naming exactly the one column currently pulled low.
2. `pa` starts at `0xFF` — every row idle-high, matching the "no key
   pressed" default.
3. Loop over rows. At `r=0`, `pressed = self.rows[0]`, which has bit 1 set
   (the 'A' key lives at column 1 of row 0). `pressed & selected` = `0b10 &
   0b10` = `0b10`, nonzero — this row has a held key on the strobed
   column. So `pa &= !(1 << 0)` clears bit 0: `pa` becomes `0b1111_1110`.
4. Every other row (`r=1..6`) either has no key held at all, or has a key
   held on a *different* column that isn't the strobed one — `pressed &
   selected` is `0` either way, so those bits of `pa` stay high.
5. Result: `pa = 0xFE`. Row 0 reads low; every other row (and PA7, never
   touched) reads high.

That's the entire electrical story of one keypress: strobe a column low,
read back which rows go low, done. `sense_pulls_row_low_only_for_strobed_column`
(§10.9) is this exact trace, encoded as a test.

### Rollover and ghosting — an honest matrix, and an honest emulator

A real scanned matrix has a well-known failure mode: **ghosting**. If three
keys held simultaneously happen to occupy three corners of a rectangle in
the matrix (two shared rows, two shared columns), the diode-free CoCo
matrix can make a fourth, unpressed key at the rectangle's last corner
appear pressed too — current sneaks through the two held keys' shared wires.
This emulator does not model that failure: `sense()` computes each row
independently and exactly, with no leakage between rows or columns, so it
implements a matrix with **perfect n-key rollover** — every combination of
held keys senses correctly, always. That's a real, honest difference from
some physical CoCo keyboards (rollover quality varied by keyboard revision
and how many keys you actually held at once), and it's the right call for
an emulator: ghosting is a mechanical defect of specific keyboard
hardware, not a fact about the 6821 or the ROM's scanning algorithm, and
nothing in the fidelity table from week 1 (§1.6) asks for it. If you ever
wanted to model it (an exercise below invites you to sketch this), you'd
add it at exactly this function — `sense()` is the one place in the whole
codebase where "how the matrix behaves electrically" lives.

### The ROM's own scan algorithm — `KEYIN`, walked

You typed on one of these as a kid; here's what the ROM you typed into was
actually doing, taken directly from `color-basic-unravelled.pdf`'s
disassembly of `KEYIN` (`$A1CB`) — the exact routine `POLCAT`/`INKEY$`
calls, still present unchanged in the CoCo 3's Super Extended Color BASIC
for backward compatibility:

```
A1CB 34 54         KEYIN   PSHS  U,X,B         SAVE REGISTERS
A1CD CE FF 00              LDU   #PIA0         POINT U TO PIA0
A1D0 8E 01 52              LDX   #KEYBUF       POINT X TO KEYBOARD MEMORY BUFFER
A1D3 4F                    CLRA                * CLEAR CARRY FLAG, SET COLUMN COUNTER (ACCA)
A1D4 4A                    DECA                * TO $FF
A1D5 34 12                 PSHS  X,A            SAVE COLUMN CTR & 2 BLANK (X REG) ON STACK
A1D7 A7 42                 STA   2,U            INITIALIZE COLUMN STROBE TO $FF
A1D9 69 42         LA1D9   ROL   2,U            * ROTATE COLUMN STROBE DATA LEFT 1 BIT, CARRY
A1DB 24 43                 BCC   LA220          * INTO BIT 0 - BRANCH IF 8 SHIFTS DONE
A1DD 6C 60                 INC   ,S             INCREMENT COLUMN COUNTER
A1DF 8D 59                 BSR   LA23A          READ KEYBOARD ROW DATA
A1E1 A7 61                 STA   1,S            TEMP STORE KEY DATA
A1E3 A8 84                 EORA  ,X              SET ANY BIT WHERE A KEY HAS MOVED
A1E5 A4 84                 ANDA  ,X              ACCA=0 IF NO NEW KEY DOWN, <>0 IF KEY WAS RELEASED
A1E7 E6 61                 LDB   1,S            GET NEW KEY DATA
A1E9 E7 80                 STB   ,X+             STORE IT IN KEY MEMORY
A1EB 4D                    TSTA                 WAS A NEW KEY DOWN?
A1EC 27 EB                 BEQ   LA1D9          NO-CHECK ANOTHER COLUMN
```

This is the software side of §10.5's electrical picture. `LDU #PIA0` +
`STA 2,U` (`STA PIA0+2`, i.e. `$FF02`) writes the column strobe; the
initial `STA 2,U` sets it to `$FF` (no column selected — a safe idle
state), and the loop's `ROL 2,U` **rotates a single zero bit through the
strobe register one position per iteration** — an elegant way to walk
"select column 0, then 1, then 2, … then 7" using one instruction and the
carry flag as the loop terminator (`BCC` — when the rotated-out bit was
originally a 1, not the marker 0, eight rotations have completed and the
carry stays clear). Each iteration calls `LA23A` (`read_side`'s hardware
counterpart) to sense that column's rows, XORs against the *previous* scan
stored in `KEYBUF` to isolate bits that *changed*, ANDs that against the
new reading to keep only newly-*pressed* bits (a released key produces a
`1→0` change that `EOR` also flags, but `AND`ing with the fresh, now-0 bit
filters it back out — only a `0→1`-then-still-1 pattern survives), and
loops to the next column if nothing new happened. `KEYBUF` is exactly this
emulator's `Keyboard.rows` array's real-hardware ancestor: eight bytes (one
per column here, since Color BASIC scans by column rather than by row) that
remember last scan's state, so debouncing and edge-detection are ROM-level
concerns layered on top of the raw matrix `sense()` gives you — the PIA and
the matrix only ever report the *current* level, never an edge; software
supplies the memory.

---

## 10.6 Joystick: no ADC, a 6-bit DAC and a comparator

The CoCo joystick ports look, on the box, like they read an analog
potentiometer position as a number. There is no analog-to-digital
converter anywhere in the machine. `joystick.rs`'s header states the trick
outright:

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

The real circuit: the joystick is a potentiometer wired as a voltage
divider, so its shaft position is an analog voltage between 0 and 5V. A
comparator chip on the CoCo's motherboard compares that voltage against a
second voltage produced by feeding the same 6-bit sound DAC (the one that
plays every game's sound effects — PIA1's `$FF20`, bits 2–7) through a
resistor ladder. The comparator's single-bit output — "is the DAC's
voltage above or below the pot's voltage?" — lands on PIA0 PA7, the very
bit `keyboard::sense` deliberately never touches (§10.5).

That's the whole trick: there's no register that "contains" the stick
position. Software has to *find* it, by trial: drive the DAC to some
voltage, read the one comparator bit, and decide whether to try higher or
lower next. Repeat until the DAC voltage converges on the pot voltage —
**successive approximation**, implemented entirely in 6809 assembly, which
is exactly what "ADC by comparator" in this week's title means.

### Selecting which pot: the analog mux

One comparator, four potentiometers (two axes × two ports) — so a small
analog multiplexer chip picks which one is currently connected to the
comparator, and *that* selection is wired to two more PIA pins, this time
as plain digital outputs rather than data-register bits:

```rust
pub fn c2_output(&self) -> bool {
    self.control & cr::C2_SET != 0
}
```

```rust
let axis = usize::from(self.pia0.a.c2_output()); // SEL1: 0 = X, 1 = Y
let stick = usize::from(self.pia0.b.c2_output()); // SEL2: 0 = right
```

CA2 (PIA0 side A's Cx2 pin, set-reset output mode — recall the `$34`/`$3C`
control values from §10.2) is SEL1, choosing X vs Y. CB2 is SEL2, choosing
right vs left port. Real ROM code for this — the mux-select routine, from
`color-basic-unravelled.pdf`:

```
A9A2 CE FF 01      LA9A2   LDU   #PIA0+1     POINT U TO PIA0 CONTROL REG
A9A5 8D 00                 BSR   LA9A7       PROGRAM 1ST CONTROL REGISTER
A9A7 A6 C4         LA9A7   LDA   ,U          GET PIA CONTROL REGISTER
A9A9 84 F7                 ANDA  #$F7        RESET CA2 (CB2) OUTPUT BIT
A9AB 57                    ASRB               SHIFT ACCB BIT 0 TO CARRY FLAG
A9AC 24 02                 BCC   LA9B0       BRANCH IF CARRY = ZERO
A9AE 8A 08                 ORA   #$08        FORCE BIT 3=1; SET CA2(CB2)
A9B0 A7 C1         LA9B0   STA   ,U++        PUT IT BACK IN THE PIA CONTROL REGISTER
```

`ANDA #$F7` clears bit 3 (`cr::C2_SET`) unconditionally, then conditionally
re-sets it depending on a bit shifted out of `B` — the same "clear then
conditionally OR" shape you'll now recognize from `c2_output()`'s single
bit read. `STA ,U++` writes CRA and then, because of the post-increment on
`U`, the second call through the same code (`BSR LA9A7` reached via `U`
already advanced) programs CRB right after — one small subroutine sets
both mux-select lines from the two low bits of a joystick-number argument.

### The comparator and the successive-approximation loop

Here's the software A/D conversion itself, `GETJOY`, from the same
disassembly — read this as a real binary search over a 6-bit space:

```
A9DE 8D 94         GETJOY  BSR   LA974       TURN OFF AUDIO
A9E0 8E 01 5E              LDX   #POTVAL+4   POINT X TO JOYSTICK DATA BUFFER
A9E3 C6 03                 LDB   #3          GET FOUR SETS OF DATA (4 JOYSTICKS)
A9E5 86 0A         LA9E5   LDA   #10         10 TRIES TO GET STABLE READING
A9E7 ED E3                 STD   ,--S        STORE JOYSTICK NUMBER AND TRY NUMBER ON STACK
A9E9 8D B7                 BSR   LA9A2       SET THE SELECT INPUTS ON ANALOG MULTIPLEXER
A9EB CC 40 80      LA9EB   LDD   #$4080      ACCA = SHIFT COUNTER (6 BITS); ACCB = 1/2 TRIAL DIFFERENCE
A9EE A7 E2         LA9EE   STA   ,-S         TEMP STORE SHIFT COUNTER ON STACK
A9F0 CA 02                 ORB   #2          KEEP RS-232 SERIAL OUT MARKING
A9F2 F7 FF 20              STB   DA          STORE IN D/A CONVERTER
A9F5 C8 02                 EORB  #2          PUT RS-232 OUTPUT BIT BACK TO ZERO
A9F7 B6 FF 00              LDA   PIA0        HIGH BIT IS FROM COMPARATOR
A9FA 2B 03                 BMI   LA9FF       BRANCH IF COMPARATOR OUTPUT IS HIGH
A9FC E0 E4                 SUBB  ,S          SUBTRACT 1/2 THE CURRENT TRIAL DIFFERENCE
A9FE 8C                    FCB   SKP2        SKIP NEXT TWO BYTES
A9FF EB E4         LA9FF   ADDB  ,S          ADD 1/2 OF THE CURRENT TRIAL DIFFERENCE
AA01 A6 E0                 LDA   ,S+         PULL SHIFT COUNTER OFF THE STACK
AA03 44                    LSRA               SHIFT IT RIGHT ONCE
AA04 81 01                 CMPA  #1          HAVE ALL THE SHIFTS BEEN DONE?
AA06 26 E6                 BNE   LA9EE       NO
AA08 54                    LSRB               YES - DATA IS IN THE TOP SIX BITS OF ACCB
AA09 54                    LSRB               PUT IT INTO THE BOTTOM SIX
```

This is textbook binary search, in silicon-era clothing. `B` starts at
`$80` — the midpoint of the DAC's range — and represents "the current
guess." Each iteration: write the guess to the DAC (`STB DA`, `DA` being
PIA1's data register, `$FF20`), read the comparator on PIA0 PA7 (`LDA
PIA0`; `BMI` tests the sign bit, i.e. bit 7), and adjust: if the comparator
reads *high* (DAC ≤ pot — the guess wasn't big enough yet), `BMI` branches
to `ADDB ,S` and pushes the guess up by half the remaining step size; if
the comparator reads *low* (DAC > pot — overshot), execution falls through
to `SUBB ,S` instead, pulling the guess down (and then jumps over the
`ADDB` it would otherwise fall into, via the two-byte `FCB SKP2` skip
trick). The step size itself halves every iteration (`LSRA`
on the shift counter, six times for six bits), so six comparisons — not
sixty-three linear steps — converge the DAC's 6-bit value onto the pot's
true position. That's the "successive approximation" of an SAR
(successive-approximation-register) ADC, done entirely by a general-purpose
CPU toggling one output register and testing one input bit, because the
CoCo shipped no ADC hardware at all.

Compare the ROM's comparator test to the emulator's model of the same bit,
`Joysticks::compare`:

```rust
/// Comparator output for the mux-selected pot: high while the DAC level
/// is at or below the pot (MAME `coco.cpp`: `dac_output() <= joyval`).
pub fn compare(&self, stick: usize, axis: usize, dac: u8) -> bool {
    dac <= self.axes[stick & 1][axis & 1]
}
```

and how the bus assembles the comparator bit onto PA7 every time port A is
read ([`bus/pins.rs:14-27`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/pins.rs#L14-L27)):

```rust
pub(super) fn pia0_pa_pins(&self) -> u8 {
    const COMPARATOR_BIT: u8 = 0x80;
    let mut pa = self.keyboard.sense(self.pia0.b.output);
    pa &= !self.joysticks.button_rows();
    let axis = usize::from(self.pia0.a.c2_output()); // SEL1: 0 = X, 1 = Y
    let stick = usize::from(self.pia0.b.c2_output()); // SEL2: 0 = right
    let dac = (self.pia1.a.output & 0xFC) >> 2;
    if self.joysticks.compare(stick, axis, dac) {
        pa |= COMPARATOR_BIT;
    } else {
        pa &= !COMPARATOR_BIT;
    }
    pa
}
```

`dac = (self.pia1.a.output & 0xFC) >> 2` recovers the 6-bit DAC value from
PIA1's output register — bits 2–7, matching the module comment
("PIA1 PA2–PA7"). There is no separate "joystick device" the emulator
polls each frame; the comparator bit is *computed fresh on every PIA0 port
A read*, exactly like the real comparator chip continuously compares two
voltages and the PIA continuously samples whatever's on its pin. Nothing
is cached between reads, because on real hardware nothing is either.

> **Rust corner: `usize::from(bool)` instead of `if`/`else`.** Both
> `pia0_pa_pins` and the mux-select comment above lean on `usize::from`
> converting a `bool` directly into `0` or `1`:
>
> ```rust
> let axis = usize::from(self.pia0.a.c2_output());
> let stick = usize::from(self.pia0.b.c2_output());
> let dac = (self.pia1.a.output & 0xFC) >> 2;
> if self.joysticks.compare(stick, axis, dac) { ... }
> ```
>
> `From<bool> for usize` (and for every other integer type) is in the
> standard library specifically because "true/false as an array index or
> a 0/1 count" is common enough to deserve a conversion, not a branch. The
> alternative, `if self.pia0.a.c2_output() { 1 } else { 0 }`, is not wrong
> — it's just four more tokens to express the same fact, and it invites a
> reader to wonder if the two branches might diverge (they can't; there's
> only one bool). When you see `T::from(some_bool)` for a numeric `T` in
> this codebase, read it as "index or count derived from a hardware select
> line," a recurring shape anywhere a chip's output pin picks between two
> alternatives — you'll meet it again choosing between analog mux inputs
> in week 11's sound-source selection.

### Fire buttons bypass the strobe

Joystick fire buttons don't sit behind the analog mux at all — they're
wired straight onto four of the keyboard's row lines, active-low, exactly
like a key would be, but **without needing any column strobed**:

```rust
/// PIA0 port-A row bits pulled low by fire buttons, per SEB Unravelled II:
/// PA0 = right button 1, PA1 = left button 1, PA2/PA3 = the CoCo 3 second
/// buttons (right/left).
const BUTTON_ROW_BITS: [[u8; 2]; 2] = [[0x01, 0x04], [0x02, 0x08]];
```

```rust
/// Mask of PIA0 PA row lines the held buttons pull low. Buttons bypass
/// the keyboard column strobe (and so also trip the GIME EI1 source —
/// SEB: they cannot be masked off).
pub fn button_rows(&self) -> u8 {
    let mut mask = 0;
    for (stick, rows) in BUTTON_ROW_BITS.iter().enumerate() {
        for (button, &bit) in rows.iter().enumerate() {
            if self.buttons[stick][button] {
                mask |= bit;
            }
        }
    }
    mask
}
```

and back in `pia0_pa_pins`, note the order: `pa &= !self.joysticks.button_rows()`
happens *after* `self.keyboard.sense(...)` computes the keyboard's own
view of the same rows. A held fire button pulls PA0–PA3 low unconditionally
— column strobe or no column strobe. This produces a real, documented
quirk any CoCo owner who played joystick games will recognize: **the fire
button electrically looks exactly like whatever key lives at row
0–3/column-currently-strobed.** If the keyboard scan happens to be strobing
a column at the moment you fire, the ROM's keyboard-scan routine (§10.5's
`KEYIN`, which — remember — reads PA and can't distinguish "row pulled low
by a key" from "row pulled low by a button") can register a phantom
keypress. That's not an emulator bug to fix; it's a documented fact about
the real board's wiring (`SEB Unravelled II` warns exactly this: buttons
"cannot be masked off," meaning software has no way to tell the PIA
"ignore buttons while I'm scanning keys"), and this codebase models it
faithfully by composing the two sources with the same bitwise `&=`/`sense`
machinery rather than special-casing buttons away from the keyboard read
path. §10.9's `fire_buttons_pull_rows_low_regardless_of_strobe` test is
this exact fact, verified.

---

## 10.7 The pins composition: one byte, many devices

Step back and look at [`bus/pins.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/pins.rs) as a whole. Its header names what it
is plainly:

```rust
//! Input-pin sampling for the two PIAs: keyboard/joystick on PIA0 port A,
//! cassette/RAMSZ/printer-busy on PIA1.
```

and `pia0_pa_pins`, assembled one more time in full, is worth reading as a
single object now that every ingredient is familiar:

```rust
pub(super) fn pia0_pa_pins(&self) -> u8 {
    const COMPARATOR_BIT: u8 = 0x80;
    let mut pa = self.keyboard.sense(self.pia0.b.output);
    pa &= !self.joysticks.button_rows();
    let axis = usize::from(self.pia0.a.c2_output()); // SEL1: 0 = X, 1 = Y
    let stick = usize::from(self.pia0.b.c2_output()); // SEL2: 0 = right
    let dac = (self.pia1.a.output & 0xFC) >> 2;
    if self.joysticks.compare(stick, axis, dac) {
        pa |= COMPARATOR_BIT;
    } else {
        pa &= !COMPARATOR_BIT;
    }
    pa
}
```

One `u8`, and by the time this function returns it, bits 0–6 have been set
by *two entirely separate emulated devices* (the 7×8 keyboard matrix and
the joystick fire buttons — themselves two separate `Joysticks`-port
states) and bit 7 by a *third* (the DAC/comparator, which itself reaches
back into `self.pia1`, the other PIA entirely, to read the DAC value
someone else's code wrote there). No single device in this codebase "owns"
`$FF00`. The byte a `LDA $FF00` returns is **assembled fresh on every read**
from whatever every relevant emulated device currently believes about its
own pins — exactly the way the real PIA0 chip's port A pin voltages are
each independently driven by whichever real circuit is physically wired to
that pin (a key switch, a button switch, or the comparator output), with
the chip itself doing nothing but latching and presenting whatever
voltages happen to be there at read time.

This is called from `io_read` at the point of an actual bus access,
never speculatively or on a timer:

```rust
IO_BASE..=PIA0_LAST => {
    // Refresh port A's input pins (keyboard rows + joystick
    // comparator/buttons) before the PIA read.
    self.pia0.a.input = self.pia0_pa_pins();
    self.pia0.read((addr & 0x03) as u8)
}
```

`self.pia0.a.input` is written immediately before `self.pia0.read(...)`
consumes it inside `PiaPort::data()` (`(self.output & self.ddr) | (self.input
& !self.ddr)`, §10.2's DDR-masking formula) — the PIA's own state doesn't
know or care where `input` came from; it just trusts that whoever calls
`read` refreshed it first. That single-line contract — "the bus refreshes
input pins immediately before a PIA read, every time" — is the entire
reason keyboard/joystick state can live in plain, independent structs
(`Keyboard`, `Joysticks`) instead of every device needing a live reference
into the PIA. It's the same "assemble reality at the point of observation,
from disjoint pieces" idea you saw the borrow checker force onto
`Machine`'s field layout in week 1 (§1.4) — here it shows up as a design
choice about *when* to compute a value, not *where* to store it, but it's
the identical instinct: don't let one piece of state need to know about
another; let something above both of them combine them exactly when
combining is needed.

PIA1's two input-pin functions are smaller but follow the same shape —
worth a glance, because week 11 and week 14 will lean on them without
re-deriving this pattern:

```rust
pub(super) fn pia1_pa_pins(&self) -> u8 {
    const CASSETTE_IN: u8 = 0x01;
    if self.cassette.input_bit() {
        0xFF
    } else {
        !CASSETTE_IN
    }
}
```

Bit 0 is the cassette data-in line (week 12's subject); every other bit
floats high, matching `PiaPort::default`'s `0xFF` idle state (§10.2's Rust
corner) rather than being wired to anything at all. `pia1_pb_pins` is the
same idea with two real signals (printer BUSY on bit 0, the RAMSZ
memory-size sense switch on bit 2 for CoCo 1/2 only) composed onto an
otherwise-floating `0xFF` — read its doc comment in the source for the
Color-BASIC memory-sizing history it explains; it's a genuinely interesting
fact about why the CoCo 1 needed a ROM upgrade to use 64K, but it's a
detour from this week's throughline and the comment tells the whole story
on its own.

---

## 10.8 Host-side, briefly: two keymaps and a type-ahead queue

Week 15 owns the frontend in full; here's just enough to close the loop
from a human's actual keyboard to the matrix `sense()` scans.

`coco-egui` supports two input philosophies, and the choice matters more
than it might look. **Positional** mode maps each host physical key
directly to the CoCo matrix position that sits in roughly the same place
on a real CoCo keyboard — press the key labeled `;` on your host keyboard
and you're pressing whatever key is physically at that matrix position,
regardless of what symbol is actually printed on the CoCo key at that spot.
`keymap.rs`:

```rust
/// Positional map: host physical key → CoCo matrix position (MAME's layout).
pub(crate) fn key_to_pos(key: egui::Key) -> Option<Pos> {
    use egui::Key as K;
    let pos = match key {
        // Letters: @ A..Z run linearly from (0,0).
        K::A => (0, 1), K::B => (0, 2), K::C => (0, 3), K::D => (0, 4),
        // ...
        // Punctuation (host physical key → CoCo key at that position, per MAME).
        K::Minus => (5, 2),      // CoCo ':'
        K::Semicolon => (5, 3),  // CoCo ';'
        // ...
        _ => return None,
    };
    Some(pos)
}
```

Note the comment on `K::Minus => (5, 2)`: a host `-` key lands on the CoCo
position that produces `:`, because that's where MAME's reference layout
puts it — positional mode is about matching muscle memory for someone who
used a real CoCo keyboard's *layout*, not about matching the symbol
printed on a modern keyboard. It's the right mode for games that read the
matrix directly (most do, bypassing BASIC's symbolic layer entirely).

**Symbolic** mode goes the other way: it wants the *character* you typed,
regardless of which host key produced it, and looks it up with the same
`char_key` function from `keyboard.rs` you met in §10.5's exercises:

```rust
pub fn char_key(c: char) -> Option<(Pos, bool)> {
    if c.is_ascii_alphabetic() {
        let upper = c.to_ascii_uppercase();
        let p = 1 + (upper as u8 - b'A'); // '@' is position 0, 'A' is 1
        let pos = (p / COLS as u8, p % COLS as u8);
        return Some((pos, c.is_ascii_uppercase()));
    }
    // ...
}
```

returning both the matrix position *and* whether the CoCo's SHIFT key must
be held to produce that character — because the CoCo's own case convention
is inverted from a modern keyboard's (unshifted keys default to uppercase
letters on a stock CoCo screen; §10.5's matrix diagram's row 0–3 columns
are the bare letters, and shift toggles case the *opposite* way ASCII
users expect). This is what makes clipboard paste and typed text "just
work" without the user thinking about the CoCo's SHIFT key at all — and
it's exactly the function every headless test in this course that types at
a BASIC prompt (`tap_char`/`type_line` in `coco1_boot.rs`, `tap` in
`keyboard.rs`, walked next) calls too.

Because a real key press needs to be *held* for multiple emulated fields
for the ROM's 60 Hz scan loop to register it (a single-field tap can land
entirely between two `KEYIN` calls and never be seen), symbolic mode
doesn't set-and-immediately-clear a matrix position — it queues taps and
drains them over real emulated time, one field at a time:

```rust
/// Advance one field, driving the CoCo matrix for the current tap.
pub(crate) fn advance(&mut self, kb: &mut kbd::Keyboard) {
    match self.phase {
        TypePhase::Idle => {
            if let Some(entry) = self.queue.pop_front() {
                self.current = entry;
                kb.set(entry.0, true);
                if entry.1 {
                    kb.set(kbd::SHIFT, true);
                }
                self.phase = TypePhase::Hold(TYPE_HOLD_FIELDS);
            }
        }
        TypePhase::Hold(0) => {
            kb.set(self.current.0, false);
            kb.set(kbd::SHIFT, false);
            self.phase = TypePhase::Gap(TYPE_GAP_FIELDS);
        }
        TypePhase::Hold(n) => self.phase = TypePhase::Hold(n - 1),
        TypePhase::Gap(0) => self.phase = TypePhase::Idle,
        TypePhase::Gap(n) => self.phase = TypePhase::Gap(n - 1),
    }
}
```

Hold for `TYPE_HOLD_FIELDS` fields, release, gap for `TYPE_GAP_FIELDS`
fields (so two identical consecutive characters read as two separate
keystrokes, not one long hold), then move to the next queued tap. This is
the production-code twin of the `tap`/`tap_char` helpers this chapter's
tests use directly (§10.9) — same shape, same reason (give the ROM's scan
loop real field boundaries to notice the key across), one written for a
human typing through an egui window and one written for a test asserting
on a screen buffer.

---

## 10.9 Reading the tests

Four tests, across three files, each teaching a fact this chapter has
already walked through in the source — now watch them assert it.

**`sense_pulls_row_low_only_for_strobed_column`**
([`crates/coco-core/tests/keyboard.rs:13-26`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/keyboard.rs#L13-L26)) is exactly the §10.5 trace,
written as a test instead of prose:

```rust
#[test]
fn sense_pulls_row_low_only_for_strobed_column() {
    let mut kb = Keyboard::new();
    kb.set((0, 1), true); // 'A' at row 0, column 1

    // Strobe column 1 low (active low): row 0 must read low (0xFE), PA7 stays high.
    const ROW0_LOW: u8 = 0xFE;
    let strobe_col1 = !(1u8 << 1);
    assert_eq!(kb.sense(strobe_col1), ROW0_LOW);
    // Strobe a different column: no key sensed.
    let strobe_col2 = !(1u8 << 2);
    assert_eq!(kb.sense(strobe_col2), 0xFF);
    // Strobe all columns low: the key is still sensed.
    assert_eq!(kb.sense(0x00), ROW0_LOW);
}
```

The third assertion is worth pausing on: strobing *every* column low
(`0x00`) still senses the key, because `sense()`'s `pressed & selected`
test only needs *any* overlap between the held key's column and the
currently-selected set of columns — real ROM code sometimes strobes all
columns at once specifically to answer "is *any* key down at all" (recall
§10.5's `A1C1: CLR PIA0+2` — strobing all columns low — as the very first
thing Color BASIC's cold-start code does after `KEYIN`'s caller checks for
*any* pending keystroke before paying for a full column-by-column scan).

**`falling_edge_selected_port_flags_only_on_high_to_low`**
([`crates/coco-core/tests/pia_sync.rs:26-33`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/pia_sync.rs#L26-L33)) exercises §10.4's edge logic
through the real bus entry point, `hsync()`, rather than `PiaPort::set_c1`
directly:

```rust
#[test]
fn falling_edge_selected_port_flags_only_on_high_to_low() {
    let mut b = bus();
    // Default CRA ($FF01=0): C1_EDGE_HIGH clear -> falling edge selected.
    assert_eq!(b.pia0.a.control & cr::C1_EDGE_HIGH, 0);
    b.hsync(); // emits set_c1(false) then set_c1(true): falling edge matches
    assert_ne!(b.pia0.a.control & cr::C1_FLAG, 0);
}
```

This test is deliberately at the *bus* level, not the `PiaPort` level —
it's asserting that the whole per-scanline call sequence in `hsync()`
(§10.4) produces the flag stock BASIC's default control-register setup
expects, not just that `set_c1` in isolation obeys its contract (that
narrower claim is `pia::tests::falling_edge_selected_flags_only_on_high_to_low`,
colocated in [`pia.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/pia.rs) itself via `#[path = "pia_test.rs"] mod tests;`). Two
tests, two altitudes, both needed — the sabotage exercise below will show
you exactly why the difference matters.

**`cb1_falling_flag_first_appears_at_fs_falling_line_not_before`**
([`crates/coco-core/tests/pia_sync.rs:50-70`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/pia_sync.rs#L50-L70)) checks a fact that's easy to
get subtly wrong: the field-sync flag must appear at the *real* scanline
the field-sync pulse occurs, not merely "eventually, sometime during the
field":

```rust
#[test]
fn cb1_falling_flag_first_appears_at_fs_falling_line_not_before() {
    let mut b = bus();
    let falling_line = VideoStandard::NTSC.fs_falling_line(MachineVariant::Coco3);
    for _ in 0..falling_line {
        b.hsync(); // drives CA1 only; CB1 must stay untouched all field
        assert_eq!(
            b.pia0.b.control & cr::C1_FLAG, 0,
            "CB1 flag must not appear before the field-sync falling edge"
        );
    }
    b.fs_falling();
    assert_ne!(
        b.pia0.b.control & cr::C1_FLAG, 0,
        "CB1 flag must appear exactly at the falling-edge scanline"
    );
}
```

This is a negative-space test: 244 iterations (`fs_falling_line` is 244 for
NTSC CoCo 3) of asserting the flag is *still not there*, then one final
assertion that it now is. That shape — proving an invariant holds
continuously right up to the exact instant it's supposed to change — is
how you test "this fires at the *right time*," as opposed to merely "this
fires eventually." Any code that accidentally raised CB1 early (say, by
confusing `fs_falling_line` with some other line count) would fail on
some iteration well before the loop even reaches `fs_falling()`.

**`fire_buttons_pull_rows_low_regardless_of_strobe`**
([`crates/coco-core/tests/joystick_bus.rs:65-79`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/joystick_bus.rs#L65-L79)) is §10.6's "buttons bypass
the strobe" claim, verified through the real bus:

```rust
#[test]
fn fire_buttons_pull_rows_low_regardless_of_strobe() {
    let mut b = bus();
    // Deselect every keyboard column (strobe all high), as BUTTON does.
    b.pia0.b.output = 0xFF;

    let idle = b.read(PIA0_PA);
    assert_eq!(idle & 0x0F, 0x0F, "no buttons: rows idle high");

    b.joysticks.set_button(RIGHT, 0, true);
    b.joysticks.set_button(LEFT, 1, true);
    let held = b.read(PIA0_PA);
    assert_eq!(held & 0x01, 0, "right button 1 pulls PA0");
    assert_eq!(held & 0x08, 0, "left button 2 pulls PA3");
    assert_eq!(held & 0x06, 0x06, "other rows stay high");
}
```

`b.pia0.b.output = 0xFF` deliberately puts the matrix in the state BASIC's
`BUTTON` function statement uses — no column strobed, so *no key* could
possibly be sensed — and the test still finds the button rows pulled low.
That's the whole point of `button_rows()` living in `pia0_pa_pins` as a
separate `&=` step from `keyboard.sense(...)`, composed after it rather
than folded into the keyboard's own logic: buttons genuinely don't care
what the strobe register says.

> One honest caveat before you run any of these yourself: `keyboard.rs`'s
> two end-to-end tests (`typing_at_prompt_echoes_to_screen`,
> `typing_multiple_keys_with_irqs_active`) boot the real `roms/coco3.rom`
> and will panic with a "file not found" error in a worktree that lacks
> `roms/` (week 1, §1.7 warned you about exactly this — ROMs live only in
> the main checkout). The four tests walked above need no ROM at all;
> `cargo test -p coco-core --test pia_sync --test joystick_bus` and the
> `pia::tests::*`/first-two-`keyboard.rs`-tests run clean anywhere.

---

## 10.10 Reading assignment

In this order:

1. **[`crates/coco-core/src/pia.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/pia.rs)**, the whole file (188 lines) — you've
   now seen nearly every line quoted in this chapter, but read it once
   more start to finish without the surrounding narration, and check that
   you can predict what each function does before reading its body.
2. **[`crates/coco-core/src/keyboard.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/keyboard.rs)** and **[`crates/coco-core/src/joystick.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/joystick.rs)**
   — both short; read the module doc comments first, then `sense`/`compare`/
   `button_rows` closely.
3. **[`crates/coco-core/src/bus/pins.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/pins.rs)** and the PIA-relevant parts of
   **[`crates/coco-core/src/bus/sync.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sync.rs)** (`hsync`, `fs_falling`, `fs_rising`
   — skip the GIME/cartridge interrupt plumbing around them, that's weeks
   8/13's territory).
4. **[`crates/coco-core/src/bus/io.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs)**, the `IO_BASE..=PIA0_LAST`/
   `PIA1_BASE..=PIA1_LAST` match arms in `io_read`/`io_write` only.
5. Tests: [`crates/coco-core/src/pia_test.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/pia_test.rs), [`tests/pia_sync.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/pia_sync.rs),
   [`tests/joystick_bus.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/joystick_bus.rs) in full; skim [`tests/keyboard.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/keyboard.rs) (see the
   caveat above about the ROM-dependent tests).

```
cargo test -p coco-core --lib pia::
cargo test -p coco-core --test pia_sync --test joystick_bus
```

should both be green with no ROM present.

---

## 10.11 Exercises

**10.1 — Trace a keypress, strobe by strobe (recall).** The 'K' key lives
at `(row=1, col=3)` (§10.5's matrix table). Write out, as a table of eight
rows (one per column strobe `KEYIN` tries, columns 0–7), what byte
`Keyboard::sense` returns for each strobe, assuming only 'K' is held and no
other key. Which single strobe value produces a non-`0xFF` result, and what
is that result exactly? Check your table against `sense`'s source — you
should be able to predict all eight rows without running any code.

**10.2 — Sabotage the edge match, run the suite, revert precisely
(sabotage).** In [`crates/coco-core/src/pia.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/pia.rs), inside `PiaPort::set_c1`,
change

```rust
if level == rising_selected {
```

to

```rust
if level != rising_selected {
```

Predict, before running anything, which tests you expect to fail. Then
run:

```
cargo test -p coco-core --lib pia::
cargo test -p coco-core --test pia_sync
```

You should see exactly two failures in each: `pia::tests::falling_edge_selected_flags_only_on_high_to_low`
and `pia::tests::rising_edge_selected_flags_only_on_low_to_high` in the
first run; `cb1_falling_flag_first_appears_at_fs_falling_line_not_before`
and `cb1_rising_edge_selected_polls_high_at_fs_rising_line` in the second.
Now look closely at what *doesn't* fail: `pia::tests::repeated_level_never_flags`
still passes (why — which code path does it exercise that never reaches
the line you changed?), and — more interesting — `pia_sync`'s two
*bus-level* edge tests, `falling_edge_selected_port_flags_only_on_high_to_low`
and `rising_edge_selected_port_flags_only_on_low_to_high`, **also still
pass**, despite testing the same broken logic. Work out why: `hsync()`
fires `set_c1(false)` then `set_c1(true)` back-to-back every call, and with
the comparison inverted, exactly one of those two calls still ends up
setting the flag (just the *wrong* one, for the wrong reason) — so a test
that only checks "the flag ends up set after one `hsync()` call" can't
tell the difference. That's a genuine coverage gap this sabotage exposes,
not a mistake in the test. Once you've confirmed the failures match, revert
with an exact `Edit` back to `==` (don't `git checkout` — that's a shared
path) and confirm `git status` is clean.

**10.3 — Add a third joystick axis mode (build).** `Joysticks` currently
models exactly two ports × two axes, matching the CoCo's two DE-9 joystick
ports. Add a `set_axis_from_mouse`-style helper (name it what you like)
that takes a normalized `f32` in `[-1.0, 1.0]` and converts it to the 0–63
pot range around `AXIS_CENTER`, clamped with `AXIS_MAX`. Write a test that
sets an axis this way and confirms `compare()` produces the same
comparator bit a hand-computed 6-bit value would. (This is, in miniature,
what `coco-egui`'s mouse-as-joystick input source does — you're
implementing the core-side half of week 15's frontend feature.)

**10.4 — Read and predict a `pia_sync` test (read + predict).** Without
running it, predict the output of this modification to
`cb1_rising_edge_selected_polls_high_at_fs_rising_line`
([`tests/pia_sync.rs:72-98`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/pia_sync.rs#L72-L98)): what happens if you delete the
`b.fs_falling()` call inside the loop (line 84) entirely, leaving
`falling_line` computed but unused? Does the test still pass? Does the
*assertion inside the loop* (`no flag before the rising-edge scanline`)
still hold at every iteration, and why does the test's own comment call
`fs_falling()` "wrong direction for this port: must not flag" — what is
this line actually there to prove, if not to make the test pass?

**10.5 — Add a convenience host key to the positional keymap (build).**
Check `key_to_pos` against every one of the 56 positions in §10.5's matrix
diagram — you'll find every CoCo key is already reachable (SHIFT/CTRL/ALT
via `drive_matrix_positionally`'s modifier handling rather than
`key_to_pos` itself, everything else through the match arms). So instead
of filling a gap, add a *second* host binding for an existing CoCo
position: map `egui::Key::Delete` as an alternate route to `kbd::BREAK`
alongside the existing `Escape`. Before wiring it, check both
`control_key_pos` and `is_joystick_key` to make sure `Delete` isn't already
claimed by something else in the input pipeline (§10.8) — a silent double
mapping is a real class of bug in a matrix with several independent
consumers of the same host event stream. Add the arm, and write a small
test (or trace it by hand) confirming both `Escape` and `Delete` now
produce `kbd::BREAK`.

**10.6 — Sketch ghosting, don't implement it (build, optional/advanced).**
§10.5 explained why this emulator's `sense()` has perfect rollover instead
of real hardware's ghosting. Sketch (in comments or a design note, not
necessarily working code) how you would detect a "ghost" condition inside
`sense()`: given the full `rows` array and a strobe, when would a fourth
key need to appear pressed that the user never touched? What data would
`sense()` need that it doesn't have access to today (hint: ghosting is a
property of the *whole* matrix's rectangle structure, not of one row in
isolation)?

**10.7 — Why does the DDR exist at all? (essay, three sentences max).**
A friend proposes: "just give every PIA port a fixed hardware direction —
port A is always input, port B is always output — and delete the DDR
registers entirely; the CoCo never reconfigures a PIA port's direction
after boot anyway, so why pay for the flexibility?" Give the two strongest
reasons the real MC6821 (and this emulator) rejects that design. (One is
about what a *general-purpose* chip has to support that a single
CoCo-specific use doesn't yet exhaust — the 6821 shipped in machines that
used PIA pins very differently; one is about a specific pin on the CoCo's
own two PIAs that genuinely does need per-bit, not per-port, direction
control — look again at PIA1's DDRs from §10.2's walk-through: `$FE` and
`$F8` are not `0x00` or `0xFF`.)

---

## What's next

Next week stays inside PIA1, but follows a different pin: the 6-bit DAC
this chapter used only as a comparator reference voltage becomes, in week
11, an actual audio signal — cycle-timestamped writes rendered into a
sample grid your speakers can play. You've already met the PIA-side
plumbing (`note_audio_write`, the CA2/CB2 mux-select bits selecting DAC vs.
cassette vs. cartridge sound) in passing this chapter; week 11 is where you
finally follow it all the way to a waveform. Week 12, after that, follows
PIA1's *other* pins — the ones this chapter only named — into a cassette
deck's own FSK modem, decoded in reverse by software that has no idea it's
being emulated.
