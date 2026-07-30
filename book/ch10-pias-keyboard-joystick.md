# Chapter 10 — The PIAs, the Keyboard Matrix, and the Joystick ADC-by-Comparator

*Week 10. Goal: understand the chip that mediates almost all CoCo I/O. Parts
I–III gave you a CPU, a bus, and a GIME that can paint a screen — an
impressive machine that still can't hear you. Every key pressed and every
joystick wiggle on a real CoCo passes through a pair of 1977-vintage
parallel-port chips before the CPU ever sees it, and one of those chips also
happens to generate the interrupt that keeps stock BASIC's idle loop alive.
This week you finally deliver on the promise made in Chapter 1: reading a PIA
data register clears an interrupt flag, and that single fact is why
`Bus::read` takes `&mut self`. By the end you'll be able to trace a
keypress from a finger on a keycap to a character on the screen, and explain
why the CoCo's "joystick port" contains no analog-to-digital converter at
all.*

---

Nine weeks in, the emulator has an odd asymmetry. It can execute
every 6809 instruction, resolve every indexed addressing mode, decode a
64K address space through an MMU, keep time to the scanline, and paint a
raster in half a dozen video modes. What it cannot do is notice that a
human being exists. Everything built so far flows outward: bytes become
pixels, cycles become fields. Nothing flows in.

This week reverses the arrow, and the chip that does the reversing is
almost comically humble compared to the GIME. The MC6821 has no video
scanout, no memory management, no timer, no palette. It has sixteen
pins of general-purpose parallel I/O, four registers, and a
single genuinely clever trick involving a bit that decides, instruction
by instruction, which of two registers a given address means. Tandy
bought two of them and wired essentially everything to them.

There is a reason this chapter sits here in the course rather than in
Chapter 3. A PIA in isolation is a twenty-minute read. A PIA understood as
the thing that makes a keypress become a character on a BASIC screen
requires a CPU that executes the ROM's scan loop, a bus that decodes
`$FF00`, a scanline clock that fires the field-sync interrupt, and a
video path that shows the result. All four of those now exist. What
follows is the week where the pieces connect, and where several loose
threads left dangling since Chapter 1 finally get tied off: why `Bus::read`
takes `&mut self`, what the ROM's interrupt handler is actually
acknowledging, and how software with no analog-to-digital converter
anywhere in the machine still manages to read an analog joystick.

The chapter also introduces a habit worth naming in advance. Almost every
claim below can be checked twice, once against this repository's source
and once against the disassembly of the ROM that ran on the real machine.
When the two agree, you know the model is right for the right reason. The
listings quoted throughout come from `color-basic-unravelled.pdf`, the
commented disassembly of Color BASIC 1.2. They are worth reading slowly:
they are what the other end of every wire in this chapter was actually
doing in 1981.

---

## 10.1 The chip that mediates almost all I/O

Start with the problem the chip exists to solve, because the shape of the
solution follows directly from it. The MC6809E has sixteen address pins
and eight data pins. That vocabulary is perfect for talking to memory and
useless for talking to anything else. A key switch is not a byte at an
address. A potentiometer is not a byte at an address. A cassette motor
relay is a coil that is either energized or not, and a printer's BUSY
line is a single wire that is either high or low. Somewhere between the
CPU's world of addressed bytes and the peripheral's world of raw voltages,
something has to translate.

Open [`crates/coco-core/src/pia.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/pia.rs). Its header comment is the whole chapter
in miniature:

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

Three sentences, and they name every mechanism this chapter spends nine
sections unpacking: the two sides, the DDR/data access split, the
read-clears-the-flag protocol, and the sync lines that drive the CPU's
IRQ. Read that comment again when you finish the chapter and it should
read as a summary rather than a preview.

The Motorola MC6821 is not a CoCo-specific chip. It's a general-purpose
peripheral interface adapter that shipped in an enormous fraction of
1977–1985 8-bit hardware, because it solved the translation problem
generically and cheaply. On the CPU side it presents four addressable
registers. On the peripheral side it presents sixteen general-purpose
pins, arranged as two independent 8-bit *sides* named A and B, plus four
interrupt and handshake lines called CA1, CA2, CB1, and CB2. Each of the
sixteen data pins can be configured, individually, as an input or an
output. Whatever you hang on those pins, the chip's job is the same: make
pin voltages look like bits in a register, and make bits written to a
register look like pin voltages.

Tandy wired two of them into the CoCo, PIA0 at `$FF00` and PIA1 at
`$FF20`, and hung almost every non-video, non-disk peripheral off those
thirty-two pins: the keyboard matrix, both joystick ports, the 6-bit
sound DAC, the cassette line, the printer's bit-banged serial line, and
on the CoCo 1 and 2 the VDG's mode-select bits. That inventory is worth
pausing on, because it explains why this one small chapter is load-bearing
for four later ones. When a BASIC program did `PRINT PEEK(65280)` — and
`65280` is `$FF00` — it was reading PIA0 directly, which is to say it was
reading the keyboard matrix and the joystick comparator in the same byte.

One CPU-facing register layout serves both sides, mirrored:

| Offset | Register | Side |
|--------|----------|------|
| `+0` | Port A data/DDR | A |
| `+1` | Control register A (CRA) | A |
| `+2` | Port B data/DDR | B |
| `+3` | Control register B (CRB) | B |

That's four registers, decoded by two address bits (`addr & 0x03`). Two
details in that table matter more than they appear to. The
first is that offsets `+0` and `+2` each name *two* different registers
depending on a control bit, which is §10.2's subject. The second is that
four registers decoded by two address bits means the chip physically
cannot tell `$FF00` from `$FF04`, a fact a real ROM exploits and the
emulator must reproduce, also in §10.2.

The Rust model mirrors the hardware's shape exactly, one struct per side
and one struct wrapping the pair:

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

Four `u8`s and a `bool` per side. Three of those four bytes are real
registers the data sheet names; the fourth, `input`, is not a register at
all but a mirror of what the outside world is currently driving onto the
pins. On the real chip there is nothing to store there, because the pins
*are* the storage — whatever voltage a key switch or a comparator happens
to be applying is simply present, continuously, and the chip samples it
when the CPU reads. An emulator has no continuous voltages, so it needs
somewhere to put the sampled value, and `input` is that somewhere. Section
10.7 shows the discipline that keeps it honest: the bus refreshes `input`
immediately before every read, so nothing stale can ever be observed.

`MC6821` is one type instantiated twice, as `pia0` and `pia1` on
`SystemBus`. That is the entire seam between "generic 1977 parallel-port
chip" and "specific CoCo peripheral": everything Tandy wired to a PIA pin,
in this codebase, ends up either setting `port.input` before a read or
reading `port.output` and `port.c2_output()` after a write. Nothing else
crosses the boundary. The chip does not know a keyboard exists, and the
keyboard does not know a PIA exists; the bus knows both, and introduces
them at the moment of an actual bus access.

That narrowness is why one 188-line file underlies the keyboard (this
chapter), the sound DAC (Chapter 11), the cassette relay and record line
(Chapters 11 and 12), and the printer's busy line (Chapter 14). PIA1 in
particular is the front door every one of those subsystems walks through.
This chapter builds the door; later chapters walk through it without
re-deriving how it opens.

> **A note on what's *not* modeled.** The doc comment above is explicit:
> Cx2 interrupt-input mode and handshake/pulse-strobe mode don't exist in
> this emulator. The real MC6821 can configure Cx2 as an *input* that
> latches its own interrupt flag, or as an output that auto-pulses on a
> data-register access, which is a complete hardware handshake protocol
> for printers and similar devices. The CoCo's ROM never uses those modes.
> Cx2 on both PIAs is always programmed as a plain set/reset output
> driving mux selects and enable lines, so the emulator doesn't pay for
> logic nothing exercises.
>
> This is the fidelity-budget discipline from Chapter 1 (§1.6) in miniature:
> model what software can observe, not what the data sheet allows. It is
> also falsifiable in the way Chapter 1 insisted every fidelity choice should
> be. If some cartridge ROM turned up tomorrow that programmed CA2 as a
> pulse strobe, the fix would be local — one more branch in the control
> register write path — and it would arrive with a test proving it was
> needed. Until then, the unimplemented modes are documented rather than
> pretended away, which is the difference between a scope decision and a
> bug.

---

## 10.2 One address, two registers: the DDR-access bit

Here's the first real puzzle a PIA presents, and it's a design problem
worth appreciating before seeing the answer. Port A needs to be
*configurable*. Some pins are inputs, such as the keyboard rows and the
joystick comparator on PIA0's port A. Some are outputs, such as the
keyboard column strobes on PIA0's port B and the sound DAC on PIA1's port
A. And the configuration cannot be per-port, because at least one real
CoCo port is genuinely mixed: PIA1's port A has a cassette input on bit 0
and seven output bits above it. So direction has to be selected *per bit*,
which means a whole byte of configuration per side, held in a *data
direction register* (DDR).

That's the requirement. The constraint is that the chip only has four
CPU-visible addresses total, two per side, and both are already spoken
for: one for data and one for control. There is no fifth address to put
the DDR at. Two registers, one address, and no room to grow.

The 6821's answer is a steal bit in the control register. One bit of CRA
decides, at any given moment, whether the port A data address means the
data register or the data direction register. The full control-register
layout, with the steal bit third from the bottom, is spelled out as named
constants:

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

Eight bits, and every one of them earns its place. Bits 0 and 1 configure
the Cx1 interrupt line, which is §10.3's subject. Bits 3 through 5
configure the Cx2 pin, which §10.6 uses as the joystick multiplexer
select. Bits 6 and 7 are the two interrupt flags, and they are read-only
from the CPU's side. Bit 2 is the steal bit. Note in passing that the
module is called `cr` and the constants are unprefixed, so call sites read
as `cr::DDR_ACCESS` and `cr::C1_FLAG`. That is the same named-mask
discipline the `cc` module applied to the CPU's condition codes back in
Chapter 2, applied here to a peripheral chip: no magic bit numbers, and every
mask carries its meaning to the call site.

With the constants in hand, the mechanism is four lines of `if`:

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

Bit clear, and the data address means the DDR. Bit set, and it means the
live data register, which senses pins on a read and drives them on a
write. Same address, two registers, one bit deciding which one an
instruction is actually touching. Ignore the flag-clearing line in
`read_side` for now; it's the subject of §10.3 and it is the single most
consequential line in the file.

What does a "live data register" read actually return, though, when some
of the pins are outputs and some are inputs? That question has exactly one
sensible answer and the chip gives it:

```rust
    /// Value seen when reading the data register: output bits on output pins,
    /// live input on the rest.
    fn data(&self) -> u8 {
        (self.output & self.ddr) | (self.input & !self.ddr)
    }
```

Read the two halves against the DDR's meaning, where a 1 bit marks an
output. `self.output & self.ddr` keeps the bits of the output register
that correspond to pins the software configured as outputs, and discards
the rest. `self.input & !self.ddr` keeps the bits of the sampled pin state
that correspond to pins configured as inputs, and discards the rest. The
two masks are exact complements, so every bit position in the result comes
from exactly one source, and the byte the CPU sees is a blend: on output
pins you read back what you wrote, and on input pins you read what the
world is doing. This one expression is why a mixed-direction port like
PIA1's port A works at all, and it is used unchanged by both the real read
path and the debugger's non-destructive peek path in §10.3.

### A real ROM sequence, decoded byte by byte

That's the theory. Here's the real thing: Color BASIC's cold-start PIA
initialization, disassembled directly from the ROM. After `LEAY` seeds the
warm-start pointer, sixteen instructions
configure both chips completely, and every one of them is doing something
this section has just explained.

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

Before decoding what it does, notice *how* it addresses things, because
there's a hardware fact hiding in the indexing. `LDX #PIA1` points X at
`$FF20`, and then the first six instructions use **negative offsets**,
`-4,X` through `-1,X`, to reach `$FF1C` through `$FF1F`. Those are not
PIA1 registers. They are not, on the face of it, PIA0 registers either:
PIA0 lives at `$FF00`–`$FF03`.

The resolution is that the chip only decodes two address lines. Every
other address bit in the range is simply not connected to it, so PIA0
answers at `$FF00`, and also at `$FF04`, `$FF08`, and so on, all the way
up to `$FF1C`–`$FF1F`. This is *incomplete address decode*, and in 1980 it
was not sloppiness but economics: routing more address lines and adding
the gates to compare them cost silicon and board area, and if nothing else
lives in the intervening addresses, there is nothing to conflict with.
Chapter 5 (§5.3) laid out the resulting I/O page map; [`bus/regs.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/regs.rs) states the
consequence in a comment and three constants:

```rust
// I/O page device ranges (`DESIGN.md` §3). PIA0/PIA1 mirror every 4 bytes.
pub(super) const IO_BASE: u16 = 0xFF00;
pub(super) const PIA0_LAST: u16 = 0xFF1F;
pub(super) const PIA1_BASE: u16 = 0xFF20;
```

and `io_read`/`io_write` ([`crates/coco-core/src/bus/io.rs:61-66`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L61-L66) and
[`:100-103`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L100-L103)) mask every address in `IO_BASE..=PIA0_LAST` down to
`addr & 0x03` before dispatching to the PIA, which reproduces the
mirroring exactly.

The ROM's author knew this and used it. With X already pointing at PIA1,
`-4,X` through `-1,X` reach PIA0's last mirror for free, saving the three
bytes of a second `LDX #PIA0` and the cycles to execute it. That is not an
emulator implementation detail. It is a hardware fact that real shipped
code depends on, and an emulator that decoded PIA0 only at
`$FF00`–`$FF03` would watch this exact sequence write its configuration
into the void and then fail to boot.

Now walk what the sequence actually does, register by register. The
mirrored addresses behave exactly like the canonical ones, so `$FF1D`
masks to 1 and means CRA, `$FF1F` masks to 3 and means CRB, `$FF1C` masks
to 0 and means port A, and `$FF1E` masks to 2 and means port B.

1. `CLR $FF1D` sets PIA0 **CRA** to 0. The `DDR_ACCESS` bit is now clear,
   so the port A data address means the DDR.
2. `CLR $FF1F` sets PIA0 **CRB** to 0, the same move for side B.
3. `CLR $FF1C` sets PIA0 **port A's DDR** to 0, making every port A pin an
   input. This is the keyboard-row and joystick-comparator side, and it
   must be input-only.
4. `LDD #$FF34` loads `A = $FF` and `B = $34` in a single instruction, a
   common 6809 trick when two different byte constants are needed back to
   back.
5. `STA $FF1E` sets PIA0 **port B's DDR** to `$FF`, making every port B
   pin an output. This is the keyboard column-strobe side.
6. `STB $FF1D` sets PIA0 **CRA** to `$34`, which is `0b0011_0100`. Decode
   it against the `cr` constants one bit at a time. `DDR_ACCESS` (bit 2)
   is now **set**, so the data address flips back to meaning live data,
   and every subsequent access to `$FF00` reads or drives real pins rather
   than configuration. `C2_OUTPUT` and `C2_SET_RESET` (bits 5 and 4) are
   both set, selecting CA2 as a static set/reset output, and `C2_SET`
   (bit 3) is clear, so that output starts low. `C1_EDGE_HIGH` (bit 1) is
   clear, selecting the falling edge for CA1. And `C1_IRQ_ENABLE` (bit 0)
   is clear, so CA1 will latch its flag but will not pull the CPU's IRQ
   line. That last point becomes important in §10.4.
7. `STB $FF1F` sets PIA0 **CRB** to `$34`, the same shape for side B: CB2
   a set/reset output starting low, CB1 falling-edge selected, CB1
   interrupt disabled, data-register access restored.

Then PIA1 goes through the same three-step dance, and the DDR values it
lands on are the interesting part. Port A's DDR becomes `$FE`, so bit 0
stays an input while bits 1 through 7 become outputs. Bit 0 is the cassette
input, and bits 2 through 7 are the 6-bit DAC that §10.6 sweeps and week
11 turns into audio. Port B's DDR becomes `$F8`, so bits 0 through 2 stay
inputs while bits 3 through 7 become outputs. Bits 0 and 2 are the
printer's BUSY line and the RAMSZ memory-size sense switch, and the upper
five are the legacy VDG mode bits. Neither `$FE` nor `$F8` is `$00` or
`$FF`, which is the concrete proof that per-bit direction control was not
over-engineering: the CoCo genuinely needs it on both of PIA1's ports.

That three-step shape — clear the control register to expose the DDR,
program the direction bits, then write the control register again to
select the data register and configure Cx1 and Cx2 — is the canonical PIA
initialization idiom. Every 6821-based machine's ROM does some version of
it, and having decoded it once you'll recognize it instantly in any
listing for the rest of the course.

> **Rust corner: overriding `Default` for hardware truth, not zero.**
> `PIAPort` does not derive `Default`; it has a hand-written `impl`:
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
> `#[derive(Default)]` would have given every field `0`, including
> `input`, and a `0x00` input register means "every pin sensed low." Feed
> that through §10.5's active-low matrix and it decodes as every key on
> every row simultaneously pressed at power-on, which is a memorable way
> for a boot to fail. Real input pins with nothing driving them don't
> settle at ground. They float, and on the CoCo they read as logic high
> until something actively pulls a line low, so `0xFF` is the honest reset
> value. The `c1_level: true` field follows the same logic for the
> interrupt line: it idles high, and §10.3 explains why the starting level
> matters as much as the current one.
>
> The derive macro cannot know any of this. It knows only that zero is a
> valid value for every primitive, which is a statement about types rather
> than about hardware. The general rule is worth carrying: whenever the
> zero value of a type isn't the hardware-true reset state, write
> `Default` by hand and put the reason in a comment beside it. The same
> reasoning shows up in Chapter 11 for the idle audio DAC level and in week
> 13 for idle disk-controller status bits. When reading unfamiliar device
> code in this codebase, check the field comments rather than assuming
> that a struct deriving `Default` has a meaningful one.

---

## 10.3 Edge detection, and the promise from Chapter 1

Now, in full, the part Chapters 1 and 5 promised: *reading a PIA data register
clears an interrupt flag*. That single sentence has been cited three times
already in this course as the reason for a type signature, and this is the
section that earns it. The mechanism has three moving parts, and it's
worth building them in order, from the wire inward.

CA1 and CB1 are single-bit interrupt-request inputs, which is to say
actual wires from the outside world into the PIA. The CoCo wires PIA0's
CA1 to the horizontal sync pulse and PIA0's CB1 to the field, or vertical,
sync pulse. The PIA has no idea what "sync" means. All it sees is a
digital line transitioning between high and low, and its job is to decide,
per transition, whether *that specific edge* is the one software asked to
be told about. The direction it cares about is selected by control
register bit 1, `cr::C1_EDGE_HIGH`, and the answer is recorded in bit 7,
`cr::C1_FLAG`.

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

Read it in three steps. First, if the new level equals the level already
stored, nothing happened electrically, so the function returns without
touching the flag. A wire that is high and stays high has not had an edge,
and callers are allowed to be sloppy about calling repeatedly. Second, if
the levels differ, a real edge occurred, and the new level is recorded
either way. That assignment happens *before* the early return specifically
so the level is tracked even when the edge doesn't match; skip it and the
next transition in the opposite direction would look like "no change" and
be silently swallowed.

Third, and most compactly, `level == rising_selected` asks whether the
direction of *this* edge matched the direction the control register asked
for. `rising_selected` is true when bit 1 selected low-to-high.
`level` is true exactly when the transition that just completed was a
rise. If the two booleans agree, the edge is the one software cares about,
and bit 7 latches. Comparing two booleans for equality is doing the work
of a four-case truth table here, which is worth reading twice the first
time and is perfectly clear the second.

The flag is read-only from the CPU's perspective. Software can never
directly set or clear it with a write, and the write path enforces that:

```rust
    fn write_control(port: &mut PiaPort, val: u8) {
        // Bits 7/6 are read-only interrupt flags; the CPU can't set them.
        const WRITABLE: u8 = !(cr::C1_FLAG | cr::C2_FLAG);
        port.control = (port.control & !WRITABLE) | (val & WRITABLE);
    }
```

`WRITABLE` is a compile-time constant naming the six bits software owns.
The expression keeps the current values of the two flag bits and takes the
written values of everything else, which is the standard shape for "merge
a write into a register with read-only bits." It's a `const` rather than a
literal precisely so the two masks in the expression can never drift apart
from the two flags in the comment.

So how does software ever clear the flag? There is exactly one way, and
it's the hardware protocol the whole first chapter of this course was
leading up to:

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
"clear interrupt" register, not a magic value written to the control
register: the mere act of the CPU reading `$FF00` or `$FF02` resets both
interrupt flags for that side, as a side effect of the load instruction.
This is real 1977 silicon behavior rather than an emulator convenience.
Inside the chip, the flag flip-flops are wired to reset during a
peripheral-register read cycle, so the acknowledgment is a property of the
bus transaction itself. Software doesn't handle the interrupt and then
separately acknowledge it. Reading the data is simultaneously *getting the
value that changed* and *telling the chip you got it*.

Notice also which branch the clearing lives in. A read of the DDR, taken
when `DDR_ACCESS` is clear, does not acknowledge anything, and neither
does a read of the control register, which never enters this function at
all. Only a read of the live data register counts. That distinction is not
pedantry; §10.4 shows the ROM's interrupt handler reading the control
register *first*, precisely because it needs to inspect the flag without
clearing it, and then reading the data register second to clear it.

One more piece completes the chain from a latched flag to an actual
interrupt on the CPU:

```rust
    /// True when this side is asserting IRQ (Cx1 flag set and its enable on).
    fn irq(&self) -> bool {
        self.control & cr::C1_FLAG != 0 && self.control & cr::C1_IRQ_ENABLE != 0
    }
```

Two bits, both required. The flag records that the edge happened; the
enable records whether software wants to be interrupted about it. This is
the standard separation between *status* and *mask* that every interrupt
controller in this course uses, including the GIME's own in Chapter 8, and it
is why the cold-start `$34` from §10.2 leaves the machine latching sync
flags every single line and field without ever raising IRQ. The flags are
there to be polled until software opts in.

This whole mechanism is exactly why `Bus::read` in
[`crates/mc6809/src/lib.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs) takes `&mut self` (Chapter 1, §1.3). `LDA $FF02`
looks, syntactically, like a pure load. On real hardware and in this
emulator it mutates `PiaPort.control`. If `read` took `&self`, this
method would need `Cell` or `RefCell` to compile, and the honest fact that
this load has a side effect would be hidden inside a wrapper type instead
of being visible in the signature of every read in the system. The reason
you learned about `&mut self` reads in Chapter 1 was this exact chip.

That decision has a cost, and Chapter 1 named it as well: sometimes a read
genuinely must not disturb anything. A debugger's memory viewer
hovering over `$FF02` and refreshing sixty times a second cannot be
allowed to eat pending interrupts. So the PIA offers a second, explicitly
side-effect-free path:

```rust
    /// Side-effect-free read for the debugger ([`crate::SystemBus::peek`]): the
    /// value [`MC6821::read`] would return for `reg`, but WITHOUT clearing the
    /// Cx1/Cx2 interrupt flags. `a_input`/`b_input` are the freshly sampled
    /// input-pin states — the bus recomputes them the same way a real read
    /// refreshes `PiaPort::input` first.
    pub fn peek(&self, reg: u8, a_input: u8, b_input: u8) -> u8 {
        match reg & 0x03 {
            0 => Self::peek_side(&self.a, a_input),
            1 => self.a.control,
            2 => Self::peek_side(&self.b, b_input),
            _ => self.b.control,
        }
    }
```

The signature carries the entire contract. It takes `&self`, so the
compiler guarantees it cannot mutate a flag. It takes the sampled input
bytes as parameters rather than reaching for them, because a `&self`
method has no way to ask the bus to refresh `PiaPort::input` first — the
caller does that and hands the result in. Two functions, two contracts,
both enforced by types rather than by a comment nobody reads. Chapter 16
builds the debugger panel on top of this; for now it is worth noting that
the awkwardness lives in exactly one place, in the method whose entire
purpose is to be the exception.

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
> `reg & 0x03` can only ever produce `0`, `1`, `2`, or `3`, and a human
> reading the expression can see that instantly. The Rust compiler cannot.
> Applying `&` to a `u8` yields a `u8`, whose type-level range is still
> `0..=255`, and nothing in the type system narrows it after a bitwise
> and. So exhaustiveness checking still demands a catch-all arm, and
> `_ => self.b.control` is it. That arm does double duty: it handles the
> legitimate `3` case and absorbs the 252 values the mask can never
> produce, which is harmless because they would all read the same register.
>
> This is worth internalizing as a pattern rather than filing as a quirk.
> Whenever you mask an integer down to a known-small range for a `match`,
> you are asserting a fact the compiler can't verify, so the wildcard arm
> is not dead code to be trimmed. It's the compiler's insurance policy,
> and it should return something sane rather than `unreachable!()`,
> because on real, buggy, or adversarial input `reg` really can be
> anything. Compare the shape to the `peek` method above, which needs the
> identical arm for the identical reason: when two functions have the same
> unreachable case, that's a hint the mask belongs at the boundary, and
> indeed the bus applies `addr & 0x03` before either is called.

---

## 10.4 The interrupt story completed: two heartbeats

Chapter 6 (§6.6) introduced the field-sync-on-PIA0-CB1 IRQ path as the thing
that breaks stock BASIC out of its idle loop, and told the debugging war
story of what an emulator looks like without it: instruction-perfect and
functionally comatose. What Chapter 6 deliberately deferred was the chip-level
half of the story. This section is where the raster timing events of Chapter 6
meet the edge-detection logic of §10.3, and where the ROM's own interrupt
handler is finally read line by line.

There are two heartbeats, one fast and one slow, and they arrive on
different PIA0 sides. `SystemBus::hsync` fires once per scanline and
carries the fast one:

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

The two lines to look at first are `self.pia0.a.set_c1(false)` followed by
`self.pia0.a.set_c1(true)`: a falling edge and then a rising edge, back to
back, once per scanline. That pair models the real horizontal-sync pulse,
which idles high and drops low for roughly 4.5 µs at the end of each line,
without requiring the sub-scanline timing resolution the emulator doesn't
have. The trick works because of how `set_c1` gates on edge direction.
Whichever direction CRA selected, exactly one of those two calls matches
it, so every scanline produces exactly one CA1 flag no matter which
polarity software asked for. Stock BASIC's `$34` selects the falling edge
and gets its flag from the first call; NitrOS-9's rising-edge convention
gets its flag from the second. Neither gets two, and neither gets zero.

That is worth dwelling on as a general emulation technique. The honest
model would be "assert CA1 low at cycle N, raise it at cycle N + 4," which
requires a timing resolution finer than the scanline the entire renderer is
built around. The cheaper model emits both edges at the same instant and
relies on the fact that the *observable consequence*, a single flag per
line, is identical. Software cannot distinguish the two models unless it
can read the raw CA1 level, and no CoCo software can, because the level
isn't exposed anywhere in the register map. Chapter 14 uses the same
compress-both-edges trick for a different signal.

How fast is this heartbeat? An NTSC scanline is about 63.5 µs, so PIA0's
CA1 flag sets roughly 15,700 times a second. The ROM's own disassembly
calls it "the 63.5 microsecond interrupt," which is a nice confirmation
that 1981 thought about it in exactly those terms. That is far too fast to
be useful as a general-purpose interrupt source for BASIC, and indeed the
cold-start CRA value of `$34` from §10.2 leaves `C1_IRQ_ENABLE` clear, so
the flag latches fifteen thousand times a second and reaches the CPU
exactly never. It is available to be polled by anything that wants
scanline-rate timing, and ignored by everything that doesn't.

The slow heartbeat is field sync, and it does not live in `hsync`. It
belongs to the scanline loop from Chapter 6, in `Machine::end_of_line`
([`crates/coco-core/src/machine/run.rs:132-140`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs#L132-L140)):

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

Every line gets an `hsync`. Two specific lines additionally get a
field-sync edge. The PIA side of those two edges is as small as it should
be:

```rust
    /// Field-sync falling edge (~60/50 Hz vertical, at
    /// [`crate::config::VideoStandard::fs_falling_line`] scanlines into the
    /// field, not at end-of-field): latches PIA0 CB1 (control reg $FF03, port
    /// B) — the interrupt that drives BASIC's housekeeping loop — per its
    /// selected edge, and raises the GIME VBORD source (Lomont: "VBORD
    /// generated on falling edge of VSYNC").
    pub fn fs_falling(&mut self) {
        self.pia0.b.set_c1(false);
        // No GIME (hence no VBORD source) on the plain-SAM path — Phase 4.
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

Unlike the horizontal pulse, these two edges genuinely happen at different
times, and the emulator has the resolution to place them.
[`config.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/config.rs#L65-L111) derives NTSC line 244 for the CoCo 3's falling edge and line
248 for the rising edge, so there is a real four-line interval during
which CB1 sits low, matching the shape of the actual vertical blanking
interval. Both functions' doc comments name their sources, which is the
house style for any timing number that came from outside the repository.

From the flag to the CPU pin is one more short hop, and it is a
wired-OR of two independent interrupt sources:

```rust
    pub fn irq_asserted(&self) -> bool {
        self.pia0.irq() || self.gime.irq_asserted()
    }
```

`MC6821::irq()` is the two-sided combination of the per-side `irq()` from
§10.3, so this expression is really three conditions: CA1's flag-and-enable,
CB1's flag-and-enable, and whatever the GIME's own interrupt block has to
say. Chapter 6 (§6.6) walked the consequences of that OR in detail; what's
new here is that you can now see all the way down to the two control-register
bits at the bottom of it.

### What the ROM's handler actually does

Here is Color BASIC's IRQ service routine, reached through the interrupt
vector, disassembled from the ROM. It is eight instructions long, and the
first three are pure PIA protocol:

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

Read it against everything above. `LDA PIA0+3` reads `$FF03`, PIA0's
control register B. Reading *the control register* never clears a flag:
`MC6821::read`'s `1 =>` and `_ =>` arms just return `port.control`
verbatim, and only a *data*-register read goes through `read_side`. So
this instruction is purely diagnostic. It fetches CRB's bits, and `BPL`,
branch if plus, tests whether bit 7 is clear. Bit 7 is `cr::C1_FLAG`. If
it isn't set, the interrupt that woke the CPU wasn't the field sync, so
the handler returns immediately with `RTI`. The listing's own comment
names the alternative it's rejecting: the every-scanline 63.5 µs interrupt
from CA1.

If bit 7 *is* set, the very next instruction, `LDA PIA0+2`, reads PIA0's
**port B data register** at `$FF02`, and per `read_side` that single load
clears both `C1_FLAG` and `C2_FLAG` on side B. The ROM's comment says it
outright: "RESET PIA0, PORT B INTERRUPT FLAG." What makes this the
clearest illustration in the entire codebase of a memory read that changes
machine state is what happens to the value it loads: nothing. `A` holds the
keyboard column-strobe byte for about one instruction and is then clobbered
by the `LDX` on the next line. The read exists *purely for its side
effect*. An optimizing compiler would delete it; a 6809 programmer in 1981
wrote it on purpose, and an emulator whose `Bus::read` took `&self`
couldn't model it without ceremony.

Everything after that is a two-byte counter being decremented, and it is
tempting to dismiss it as bookkeeping unrelated to the PIA. It isn't, and
the next subsection explains why.

### Who turns the interrupt on

Section 10.2 established that cold start leaves both of PIA0's
`C1_IRQ_ENABLE` bits clear. Something has to set one before any of this
handler ever runs. In Color BASIC, one of the somethings is the `SOUND`
statement:

```
A956 B6 FF 03              LDA     PIA0+3     GET CONTROL REGISTER OF PIA0, PORT B
A959 8A 01                 ORA     #1         *
A95B B7 FF 03              STA     PIA0+3     * ENABLE 60 HZ INTERRUPT (PIA0 IRQ)
```

Read, set bit 0, write back. Bit 0 is `cr::C1_IRQ_ENABLE`, and the
read-modify-write shape is deliberate: `SOUND` has no business disturbing
the edge selection or the CA2 mux state that other code owns, so it ORs in
one bit and leaves the other seven alone. And now the tail of `BIRQSV`
makes sense. `SNDDUR` is the note duration `SOUND` computed from its
second argument, and the field-sync interrupt is the clock that counts it
down while the main code sits in a tone-generating loop. `SOUND` enables
the interrupt because `SOUND` is what needs it.

This is a good moment to connect back to Chapter 6's synthetic boot ROM,
which contained the sequence `LDA #$05` / `STA $FF03` and was described
there as "enable the CB1 field-sync IRQ." `$05` is `C1_IRQ_ENABLE |
DDR_ACCESS`, which is precisely what `SOUND`'s `ORA #1` produces when the
running CRB already has `DDR_ACCESS` set. The hand-written test ROM from
four weeks ago was doing, bit for bit, what the real ROM does when a BASIC
program asks for a beep.

There is a broader lesson in that arrangement, and it's the reason this
section exists rather than stopping at `RTI`. The field-sync interrupt on
a stock CoCo doesn't do much *work*. Its entire job, from BASIC's point of
view, is to fire on a dependable schedule so that code which needs to
measure elapsed time can do so without counting its own instructions.
That's the "it's alive" heartbeat from Chapter 6, and you have now seen both
ends of the wire: the raster hardware asserting CB1 in `fs_falling`, and
the ROM's own handler acknowledging it with the read this section exists
to explain.

---

## 10.5 The keyboard: a 7×8 matrix, electrically

Switch from interrupts to input. The CoCo keyboard is not sixty
independent switches wired to sixty pins, because sixty pins is not a
budget the machine has. It's a **matrix**: 56 keys wired at the
intersections of 7 row lines and 8 column lines, so 15 pins can sense 56
switches. The arithmetic generalizes, and it's why every keyboard from
this era through today's is scanned rather than wired directly: `r + c`
pins buy `r × c` keys, so the savings grow as the keyboard does.

`keyboard.rs`'s header comment gives the authentic layout, cross-checked
against MAME's `coco3_keyboard`:

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

Take a moment with the table, because its structure is not arbitrary. The
letters run linearly from `@` at (0,0) through `Z` at (3,2), which means a
key's matrix position and its position in the ASCII alphabet differ by a
constant. That is not a coincidence; it's what lets the ROM convert a
matrix hit into a character with an add and a shift rather than a lookup
table, and it's what lets this codebase's `char_key` do the same in three
lines of Rust. The digits then run linearly across rows 4 and 5, the
punctuation fills out row 5, and everything that isn't a printable
character lands in row 6.

A key at position `(row, col)` is a switch bridging row wire `row` to
column wire `col` when pressed. Nothing energizes the matrix on its own.
The CPU has to *drive* one side and *sense* the other, one column at a
time, and then repeat the whole procedure often enough to catch a
keystroke. That's the tradeoff a scanned matrix makes: far fewer pins, at
the cost of software having to run a loop instead of reading one register.

Both sides are **active low**, which was a near-universal convention in
TTL-era hardware for a good electrical reason. Pulling a line down to
ground is cheap and definite, while driving it up requires a source of
current, so the standard arrangement is a pull-up resistor holding the
line high by default and a switch that shorts it to ground when closed.
Applied here: a column is "selected" by driving its PB bit to 0, and a row
reads 0 exactly when a held key connects it through a strobed-low column.
`PA7` is carved out of all of this. It isn't a key at all; it's the
joystick comparator bit, which §10.6 takes up.

The live matrix state is about as simple a struct as this codebase gets:

```rust
/// The live matrix state: `rows[r]` has bit `c` set when key `(r, c)` is held.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Keyboard {
    rows: [u8; ROWS],
}
```

Seven bytes, one per row, each bit a column, and — unlike `PIAPort` — a
derived `Default` is correct here, because "no bits set" really does mean
"no keys held." The field is private, so the only way in is `set`:

```rust
    /// Press or release the key at `(row, col)`.
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

Bounds-check, then set or clear one bit. The early return on an
out-of-range position is worth a note: it makes a bad position a no-op
rather than a panic, which is the right call for a function whose callers
include a frontend translating host key events. A typo in a keymap table
should produce a key that does nothing, not a crash. This is the frontend's
entire write surface into the emulated keyboard. `coco-egui` never touches
a PIA register directly; it calls `set` (§10.8).

Beside the matrix state lives a pure function that the frontend and the
tests both use to go the other way, from a character to the key that
produces it:

```rust
pub fn char_key(c: char) -> Option<(Pos, bool)> {
    // Letters: @ A..Z live linearly from (0,0); uppercase needs CoCo shift.
    if c.is_ascii_alphabetic() {
        let upper = c.to_ascii_uppercase();
        let p = 1 + (upper as u8 - b'A'); // '@' is position 0, 'A' is 1
        let pos = (p / COLS as u8, p % COLS as u8);
        return Some((pos, c.is_ascii_uppercase()));
    }
```

That's the linear-letters observation from the matrix table cashed in:
subtract `'A'`, add one for the `@` that occupies position zero, then
divide and modulo by the column count to get row and column. The rest of
the function is an explicit `match` over digits and punctuation, because
those are not linear in any useful way, and every arm returns a `bool`
saying whether the CoCo's SHIFT key must be held. Section 10.8 explains
why that `bool` is not the same thing as "the host key was shifted."

### `sense()`, walked bit by bit

Everything the CPU actually experiences funnels through one function, and
it is eight lines long:

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

Trace it for one key. Suppose the user is holding position (0, 1), which
the matrix table says is the `A` key, and the ROM strobes column 1 by
writing `$FD`, or `0b1111_1101`, to port B — every bit high except bit 1,
which is driven low.

1. `strobe = 0b1111_1101`, so `selected = !strobe = 0b0000_0010`. The
   complement turns the active-low strobe into a positive-logic mask that
   names exactly the column currently pulled low, which makes every
   subsequent test a plain `&`.
2. `pa` starts at `0xFF`, every row idle high, matching the no-key-pressed
   default and the floating-pin reasoning from §10.2's Rust corner.
3. The loop visits row 0 first. `pressed` is `self.rows[0]`, which has bit
   1 set because `A` lives at column 1 of row 0. `pressed & selected` is
   `0b10 & 0b10`, which is nonzero: this row has a held key on the strobed
   column. So `pa &= !(1 << 0)` clears bit 0 and `pa` becomes
   `0b1111_1110`.
4. Rows 1 through 6 either hold no key at all, or hold a key on some
   column that isn't the strobed one. Either way `pressed & selected` is
   zero, and those bits of `pa` stay high.
5. The result is `pa = 0xFE`. Row 0 reads low; every other row reads high,
   and PA7, never touched by the loop, keeps the high level it started
   with.

That's the entire electrical story of one keypress: strobe a column low,
read back which rows went low, done. The test
`sense_pulls_row_low_only_for_strobed_column` (§10.9) is this exact trace
encoded as assertions.

Two properties of this function are worth naming explicitly because later
sections lean on them. First, it takes `&self` and returns a value,
touching nothing. That makes it safe to call from anywhere, including the
per-scanline GIME keyboard-interrupt sampling in `hsync` that §10.7 comes
back to. Second, the strobe is a full byte, not a column index, so
"multiple columns selected at once" is representable and behaves
sensibly. The ROM uses that, as §10.9's third assertion shows.

### Rollover and ghosting: an honest matrix, and an honest emulator

A real scanned matrix has a well-known failure mode called **ghosting**.
If three simultaneously held keys happen to occupy three corners of a
rectangle in the matrix, sharing two rows and two columns between them,
current can sneak from the strobed column through one held key, along a
row, back through a second held key, and out along another column. The
fourth corner of the rectangle then senses as pressed even though nobody
touched it. Keyboards that care about this put a diode in series with
every key so current can only flow one way; keyboards built to a price
often didn't.

This emulator does not model that failure. `sense()` computes each row
independently and exactly, with no leakage between rows or columns, so it
implements a matrix with **perfect n-key rollover**: every combination of
held keys senses correctly, always. That is a real difference from some
physical CoCo keyboards, and it is the right call for an emulator.
Ghosting is a mechanical property of a particular piece of keyboard
hardware, not a fact about the 6821 or about the ROM's scanning algorithm,
and nothing in the fidelity table from Chapter 1 (§1.6) asks for it. The
question that table trains you to ask is "who notices?", and the answer
here is: a user pressing three specific keys at once, who would experience
the real machine's behavior as a bug.

If you ever did want to model it, and an exercise below invites you to
sketch how, you'd add it in exactly this function. `sense()` is the one
place in the whole codebase where "how the matrix behaves electrically"
lives, which is itself a design property worth noticing: the fact that
there is a single such place is what makes the question answerable at all.

### The ROM's own scan algorithm: `KEYIN`, walked

Here is what the software on the other side of those keycaps was actually
doing. Color BASIC's keyboard entry point is at `$A1C1`, and the first
thing it does is not a scan at all but a fast rejection:

```
A1C1 7F FF 02      LA1C1   CLR   PIA0+2       CLEAR COLUMN STROBE
A1C4 B6 FF 00              LDA   PIA0         READ KEY ROWS
A1C7 43                    COMA                COMPLEMENT ROW DATA
A1C8 48                    ASLA                SHIFT OFF JOYSTICK DATA
A1C9 27 79                 BEQ   LA244        RETURN IF NO KEYS OR FIRE BUTTONS DOWN
```

Five instructions, and every one of them is this chapter's material.
`CLR PIA0+2` writes `$00` to the column strobe, which in active-low terms
selects *every column simultaneously*. `LDA PIA0` then reads the rows, and
because every column is selected, any held key anywhere in the matrix
pulls its row low. `COMA` flips the active-low reading into positive logic
so that "something is down" becomes "some bit is set." `ASLA` shifts the
byte left by one, which pushes bit 7 out of the register entirely — the
comment calls that "shift off joystick data," because bit 7 is the
comparator, not a key. And `BEQ` returns if what's left is zero.

The cost of that test is five instructions. The cost of the full
column-by-column scan below is roughly eight times that, plus debounce.
Since the overwhelmingly common case at a BASIC prompt is that no key is
down at all, paying five instructions to avoid the other forty is an
excellent trade, and it's why the routine is structured as a cheap
rejection falling through into an expensive scan rather than as one loop.

Note also what the comment on the final branch says: "RETURN IF NO KEYS
**OR FIRE BUTTONS** DOWN." The ROM's author knew that a joystick fire
button is electrically indistinguishable from a key at this point in the
code. Section 10.6 returns to that.

When something *is* down, execution falls through into `KEYIN` proper:

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

This is the software side of the electrical picture above, and there are
three ideas in it worth extracting.

The first is how it walks the columns. `STA 2,U` writes `$FF` to the
strobe register, selecting no column at all, which is a safe idle state.
Then `ROL 2,U` **rotates the strobe register left through the carry flag**,
one position per iteration. The carry was cleared by the `CLRA` before the
loop, so the first rotation shifts a 0 into bit 0 and the strobe becomes
`$FE`, selecting column 0. The next rotation moves that 0 to bit 1, and so
on. Eight rotations later the 0 has been rotated out into the carry and
the strobe is back to all ones, at which point `BCC` no longer branches
and the loop ends. One instruction advances the scan and one branch
terminates it, with no separate counter needed for either. A modern
compiler would produce something considerably less elegant.

The second is the debounce and edge detection, which happen entirely in
software. `BSR LA23A` senses the current column's rows into `A`. `EORA ,X`
exclusive-ORs that against the *previous* scan of this same column, stored
in a small RAM buffer called `KEYBUF`, which leaves a 1 bit wherever the
key's state changed. `ANDA ,X` then ANDs the changes against the old
reading. Work through the four cases and you'll find the only surviving
bits are those that went from "not pressed" to "pressed" in the active-low
sense, so releases are filtered out. The freshly read byte then replaces
the old one in `KEYBUF` for next time.

The third idea is the one to carry forward: `KEYBUF` exists because the
hardware has no memory. The PIA and the matrix only ever report the
*current* level of every line. They cannot report an edge, a repeat, or a
release, because none of those is a voltage. Any notion of "a new key was
pressed" has to be constructed by comparing two samples over time, and in
this machine that construction lives in the ROM, in eight bytes of RAM.
This emulator's `Keyboard.rows` array is `KEYBUF`'s direct descendant in
purpose, though it lives on the other side of the fence: `rows` is the
*truth* about what's held, and `KEYBUF` is the ROM's *memory* of what it
last saw.

The row-read subroutine `KEYIN` calls is seven instructions and contains
two surprises:

```
A238 E7 42         LA238   STB   2,U          SAVE NEW COLUMN STROBE VALUE
A23A A6 C4         LA23A   LDA   ,U           READ PIA0, PORT A TO SEE IF KEY IS DOWN
A23C 8A 80                 ORA   #$80         MASK OFF THE JOYSTICK COMPARATOR INPUT
A23E 6D 42                 TST   $02,U        ARE WE STROBING COLUMN 7?
A240 2B 02                 BMI   LA244        NO
A242 8A C0                 ORA   #$C0         YES, FORCE ROW 6 TO BE HIGH - THIS WILL CAUSE
                                              THE SHIFT KEY TO BE IGNORED
A244 39            LA244   RTS                RETURN
```

The first surprise is `ORA #$80`, which forces bit 7 high before the value
is used. That is the ROM defending itself against PA7, the joystick
comparator, which may be at either level depending on where the DAC and
the pots happen to sit. The emulator's `sense()` leaves bit 7 high for the
same reason from the other direction, and the comment in `sense()` says so
in as many words. Two independent implementations, forty-five years apart,
agreeing that PA7 must not be allowed to look like a key.

The second surprise is the `TST`/`BMI`/`ORA #$C0` sequence. `TST $02,U`
tests the column strobe register itself, and `BMI` branches when its bit 7
is set, meaning column 7 is *not* the one being strobed. When column 7 *is*
selected, the ROM ORs in `$C0`, forcing row 6 high and thereby ignoring
whatever key sits at (6, 7). Look that position up in the matrix table:
it's SHIFT. The scan loop deliberately blinds itself to SHIFT so that
holding SHIFT doesn't register as a keystroke in its own right. SHIFT is
instead checked separately when a real key has been found, by a small
routine that strobes column 7 on purpose:

```
A22E 86 7F         LA22E   LDA   #$7F         COLUMN STROBE
A230 A7 42                 STA   2,U          STORE TO PlA
A232 A6 C4                 LDA   ,U           READ KEY DATA
A234 43                    COMA                *
A235 84 40                 ANDA #$40          * SET BIT 6 IF SHIFT KEY DOWN
A237 39                    RTS                RETURN
```

`$7F` is `0b0111_1111`, column 7 low and everything else high. Read, flip
to positive logic, keep bit 6, and the result is nonzero exactly when the
key at row 6, column 7 is held. That is SHIFT's matrix position, arrived
at from the ROM side, and it is an independent confirmation of the layout
table this codebase copied from MAME. When two sources that never talked
to each other agree on a coordinate, the coordinate is right.

Everything in this section has assumed that PA7 belongs to something other
than the keyboard. It's time to find out what.

---

## 10.6 Joystick: no ADC, a 6-bit DAC and a comparator

The CoCo joystick ports look, from the outside, like they read an analog
potentiometer's position as a number. `JOYSTK(0)` in BASIC returns a value
from 0 to 63, which is exactly what an analog-to-digital converter would
give you. There is no analog-to-digital converter anywhere in the machine.

`joystick.rs`'s header states the trick outright, and cites where each
piece was verified:

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

The physical circuit is worth picturing, because the software makes no
sense without it. A joystick is a pair of potentiometers, one per axis,
each wired as a voltage divider, so the shaft position becomes an analog
voltage somewhere between ground and the supply rail. On the motherboard,
a comparator chip compares that voltage against a second voltage. The
second voltage comes from the 6-bit sound DAC — the same resistor ladder
that plays every game's sound effects, driven by PIA1's port A output
register at `$FF20`, bits 2 through 7. The comparator's output is one bit
answering one question: is the DAC's voltage at or below the pot's? That
bit lands on PIA0 PA7, the very bit `keyboard::sense` deliberately never
touches and the ROM's `LA23A` deliberately masks off.

So there is no register that "contains" the stick position. Software has
to *find* it by trial: drive the DAC to some voltage, read the one
comparator bit, and decide whether to try higher or lower next. Repeat
until the DAC voltage converges on the pot voltage. That is
**successive approximation**, the same algorithm a real SAR-type ADC chip
implements in hardware, except that here it runs as 6809 instructions.
Tandy shipped the resistor ladder they already needed for sound, one
comparator, and a few lines of ROM, instead of an ADC chip. That is the
"ADC-by-comparator" in this week's title, and it is a fine example of the
era's cost engineering: the missing chip's function still exists; it has
just been relocated into software.

### Selecting which pot: the analog mux

One comparator has to serve four potentiometers, since there are two ports
of two axes each. A small analog multiplexer picks which pot is currently
connected, and its two select inputs are wired to PIA pins — not data
pins this time, but the Cx2 lines that §10.2's control-register walk
configured as static outputs. Reading such an output is a single bit test:

```rust
    pub fn c2_output(&self) -> bool {
        self.control & cr::C2_SET != 0
    }
```

The bus turns those two bools into array indices:

```rust
        let axis = usize::from(self.pia0.a.c2_output()); // SEL1: 0 = X, 1 = Y
        let stick = usize::from(self.pia0.b.c2_output()); // SEL2: 0 = right
```

CA2, side A's Cx2 pin, is SEL1 and chooses X against Y. CB2 is SEL2 and
chooses the right port against the left. Both were programmed as
set/reset outputs by the `$34` written at cold start, so changing a
selection means rewriting the control register with bit 3 flipped, which
is exactly what the ROM's mux routine does:

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

`ANDA #$F7` clears bit 3, `cr::C2_SET`, unconditionally, then `ORA #$08`
conditionally puts it back depending on a bit shifted out of `B`. That is
the same clear-then-conditionally-set shape as the `SOUND` routine's
read-modify-write in §10.4, and for the same reason: the other seven bits
of the control register belong to other subsystems and must survive.

The clever part is the addressing. `STA ,U++` writes the control register
and *then* advances `U` by two, from `$FF01` to `$FF03`. Combined with the
`BSR LA9A7` two instructions above, which calls the code immediately
following it, the routine runs its own body twice: once with `U` pointing
at CRA and the low bit of `B` in the carry, and once with `U` pointing at
CRB and the next bit of `B` in the carry. One small subroutine, two
control registers, two bits of a joystick-number argument, no loop
counter. The listing's own header states the contract: "THIS ROUTINE WILL
TRANSFER BIT 0 OF ACCB TO SEL 1 OF THE ANALOG MULTIPLEXER AND BIT 1 OF
ACCB TO SEL 2."

### The comparator and the successive-approximation loop

Now the software A/D conversion itself. `GETJOY` is what `JOYSTK(0)`
calls, and its very first instruction is a reminder that the DAC has a day
job:

```
A9DE 8D 94         GETJOY  BSR   LA974       TURN OFF AUDIO
```

`LA974` clears bit 3 of PIA1's control register B, which is the CB2 output
the ROM's listing calls the sound-mux enable. The conversion is about to
sweep the DAC through a range of values as fast as the CPU can write them,
and if that reached the speaker it would be audible as a click or a buzz.
So the ROM disconnects the audio path first, converts, and — in the
`SOUND` statement's case — reconnects it afterwards. That single `BSR` is
the whole reason Chapters 10 and 11 are adjacent chapters: the joystick and
the sound output are the same six pins, time-shared by convention.

Then the conversion:

```
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

This is textbook binary search in silicon-era clothing. `B` starts at
`$80`, the midpoint of the DAC's range, and holds the current guess. `A`
starts at `$40` and is the step size, halved on every pass. Each iteration
writes the guess to the DAC with `STB DA`, where `DA` is PIA1's port A
data register at `$FF20`, reads the comparator with `LDA PIA0`, and tests
bit 7 with `BMI`, branch if minus, which on the 6809 means the sign bit is
set.

The two outcomes are symmetric. If the comparator reads high, the DAC is
at or below the pot, meaning the guess is too low, so `BMI` branches to
`ADDB ,S` and raises the guess by the current step. If the comparator
reads low, the DAC has overshot, so execution falls through to `SUBB ,S`
and lowers it. The `FCB SKP2` between them is a period trick worth
recognizing: `$8C` is the opcode for `CMPX` immediate, which consumes the
next two bytes as an operand, so placing it before `EB E4` causes those
two bytes to be swallowed as data rather than executed. It's a two-byte
unconditional skip that costs less than a `BRA`, and it appears throughout
this ROM.

The step size halves each pass via `LSRA`, and after six passes `A` has
been shifted down to 1 and the loop exits. Six comparisons rather than
sixty-three linear steps is the entire value of binary search, and here
each comparison costs a DAC write and a PIA read, so the saving is
measured in real microseconds during which BASIC isn't doing anything
else. The two `LSRB`s at the end shift the six meaningful bits down from
the top of the byte to the bottom, producing the 0-to-63 value `JOYSTK`
eventually returns.

One detail in that listing has no counterpart in the emulator, and it's
instructive. `LDA #10` sets up "10 TRIES TO GET STABLE READING," and after
each conversion the ROM compares the result against the previous
conversion's value and retries if they differ, up to ten times. That loop
exists because the real thing being measured is an analog voltage on a
mechanical potentiometer, and analog voltages have noise. Two consecutive
conversions of a physically motionless stick can legitimately disagree by
a count. In this emulator the pot is a `u8` that does not move between
conversions, so the second reading always equals the first and the retry
loop exits after two passes, every time. The ROM's defensive code is
harmless and invisible. That's a nice illustration of a general principle:
an emulator is frequently *more* deterministic than the hardware, and
software written to cope with analog reality finds reality unusually
agreeable.

Compare the ROM's comparator test to the emulator's model of the same bit:

```rust
    /// Comparator output for the mux-selected pot: high while the DAC level
    /// is at or below the pot (MAME `coco.cpp`: `dac_output() <= joyval`).
    pub fn compare(&self, stick: usize, axis: usize, dac: u8) -> bool {
        dac <= self.axes[stick & 1][axis & 1]
    }
```

One comparison operator, and the `<=` rather than `<` is load-bearing: it
is what MAME's `coco.cpp` implements, and it decides the behavior at the
exact boundary where DAC equals pot. The colocated test
`comparator_is_high_while_dac_at_or_below_pot` pins all three cases, below,
equal, and above. The `& 1` masks on both indices are the same defensive
habit as `Keyboard::set`'s bounds check: a caller that passes a bad stick
number gets a wrong answer rather than a panic.

Here is how the bus assembles that bit onto PA7 on every port A read
([`bus/pins.rs:14-27`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/pins.rs#L14-L27)):

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
PIA1's output register, masking off the low two bits and shifting down,
which matches the module comment's "PIA1 PA2–PA7." Those discarded low
bits are not padding, incidentally: bit 1 is the RS-232 serial output that
`GETJOY`'s `ORB #2` was carefully preserving on every DAC write, so that
sweeping the joystick doesn't transmit garbage to a printer.

The most important property of this function is when it runs. There is no
"joystick device" the emulator polls each frame and caches. The comparator
bit is *computed fresh on every PIA0 port A read*, exactly the way the real
comparator chip continuously compares two voltages and the real PIA
samples whatever is on its pin at the instant the CPU reads. Nothing is
cached between reads, because on real hardware there is nothing to cache.

> **Rust corner: `usize::from(bool)` instead of `if`/`else`.** Both
> `pia0_pa_pins` and the mux-select code above lean on `usize::from`
> converting a `bool` directly into `0` or `1`:
>
> ```rust
> let axis = usize::from(self.pia0.a.c2_output());
> let stick = usize::from(self.pia0.b.c2_output());
> let dac = (self.pia1.a.output & 0xFC) >> 2;
> if self.joysticks.compare(stick, axis, dac) { ... }
> ```
>
> `From<bool> for usize`, and for every other integer type, is in the
> standard library specifically because "true or false as an array index
> or a 0/1 count" is common enough to deserve a conversion rather than a
> branch. The alternative, `if self.pia0.a.c2_output() { 1 } else { 0 }`,
> is not wrong; it's four more tokens for the same fact, and it invites a
> reader to check whether the two branches might one day diverge. They
> can't, because there is only one bool.
>
> There's a small correctness argument too. The conversion is defined to
> map `false` to `0` and `true` to `1`, which means the mapping can't be
> silently inverted by an editing mistake the way a hand-written `if` can.
> When you see `T::from(some_bool)` for a numeric `T` in this codebase,
> read it as "an index or count derived from a hardware select line." It
> is a recurring shape anywhere a chip's output pin picks between two
> alternatives, and you'll meet it again in Chapter 11's sound-source
> selection.

### Fire buttons bypass the strobe

Joystick fire buttons don't sit behind the analog mux at all. They are
wired straight onto four of the keyboard's row lines, active low, exactly
as a key switch would be, but **without needing any column strobed**:

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

The nested table indexing is `[stick][button]`, and reading the constant
carefully is the only way to get the bit assignments right: the outer
array is indexed by stick, the inner by button, so right-stick button 1 is
`0x01` and right-stick button 2 is `0x04`, while left-stick button 1 is
`0x02` and button 2 is `0x08`. The colocated test
`button_rows_match_seb_wiring` asserts exactly that interleaving, which is
the kind of fact that is easy to transpose and impossible to notice once
transposed.

Back in `pia0_pa_pins`, note the order of operations:
`pa &= !self.joysticks.button_rows()` runs *after*
`self.keyboard.sense(...)` has computed the keyboard's own view of the same
rows. A held fire button pulls PA0 through PA3 low unconditionally, column
strobe or no column strobe, and the `&=` composes the two sources without
either knowing about the other.

That composition produces a real, documented quirk: **a fire button is
electrically indistinguishable from whatever key sits at its row and the
currently strobed column.** The ROM's own comments admit it twice. The
fast-rejection test at `LA1C1` says "RETURN IF NO KEYS **OR FIRE BUTTONS**
DOWN," and the debounce path deliberately probes for the confusion:

```
A213 C6 FF                 LDB    #$FF        SET COLUMN STROBE TO ALL ONES (NO
A215 8D 21                 BSR    LA238       STROBE) AND READ KEYBOARD
A217 4C                    INCA                = INCR ROW DATA, ACCA NOW 0 IF NO JOYSTICK
A218 26 06                 BNE    LA220       = BUTTON DOWN. BRANCH IF JOYSTICK BUTTON DOWN
```

With the strobe set to `$FF`, no column is selected, so *no key can
possibly be sensed*. `LA238` reads the rows and forces bit 7 high, so a
completely idle read returns `$FF`, and `INCA` turns that into zero. Any
nonzero result therefore means something pulled a row low without a column
being strobed, and the only thing on the board that can do that is a fire
button. The ROM uses this as a deliberate test, and the emulator's
`fire_buttons_pull_rows_low_regardless_of_strobe` test (§10.9) sets up the
identical conditions in Rust.

This is not an emulator bug to fix. It's a fact about the real board's
wiring, stated in `SEB Unravelled II` as the warning that the buttons
"cannot be masked off," meaning software has no way to tell the PIA to
ignore them for the duration of a keyboard scan. The codebase models it
faithfully by composing the two sources with the same bitwise machinery
rather than special-casing buttons out of the keyboard read path, which is
why the quirk falls out for free instead of having to be reintroduced.

---

## 10.7 The pins composition: one byte, many devices

Step back and look at [`bus/pins.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/pins.rs) as a whole, because the file is a
worked example of a design idea that recurs everywhere in this codebase.
Its header names what it is plainly:

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

One `u8`, and by the time the function returns it, bits 0 through 6 have
been set by two entirely separate emulated devices, the 7×8 keyboard
matrix and the joystick fire buttons. Bit 7 comes from a third, the
DAC-and-comparator pair, which itself reaches across into `self.pia1`, the
*other* PIA entirely, to read a DAC value that some unrelated piece of
code wrote there.

No single device in this codebase "owns" `$FF00`. The byte that a
`LDA $FF00` returns is **assembled fresh on every read** from whatever
every relevant emulated device currently believes about its own pins. That
is exactly how the real PIA0's port A behaves: each pin's voltage is
independently driven by whichever circuit is physically wired to it, a key
switch or a button switch or the comparator output, and the chip itself
does nothing but latch and present whatever voltages happen to be there at
read time. The emulator's composition function is not an abstraction over
the hardware. It is a transcription of it.

This is called from `io_read` at the point of an actual bus access, never
speculatively and never on a timer:

```rust
            IO_BASE..=PIA0_LAST => {
                // Refresh port A's input pins (keyboard rows + joystick
                // comparator/buttons) before the PIA read.
                self.pia0.a.input = self.pia0_pa_pins();
                self.pia0.read((addr & 0x03) as u8)
            }
```

`self.pia0.a.input` is written immediately before `self.pia0.read(...)`
consumes it inside `PiaPort::data()`, whose DDR-blending formula §10.2
walked through. The PIA's own state doesn't know or care where `input`
came from; it trusts that whoever calls `read` refreshed it first. That
one-line contract is what allows keyboard and joystick state to live in
plain, independent structs with no live reference into the PIA and no
reference from the PIA back to them.

It's the same "assemble reality at the point of observation, from disjoint
pieces" instinct that the borrow checker forced onto `Machine`'s field
layout in Chapter 1 (§1.4). Here it shows up as a decision about *when* to
compute a value rather than *where* to store it, but the underlying rule is
identical: don't let one piece of state need to know about another, and let
something above both of them combine them exactly when combining is needed.

The `&self` on `pia0_pa_pins` is doing real work in that arrangement,
because the bus read is not its only caller. Look back at `hsync` in
§10.4:

```rust
        // Buttons are included: SEB warns joystick fire buttons always trip EI1.
        let line_low = self.pia0_pa_pins() & 0x7F != 0x7F;
        if is_gime && line_low && !self.kbd_line_low {
            self.gime.raise(gime::intr::EI1);
        }
        self.kbd_line_low = line_low;
```

The GIME has its own keyboard-interrupt source, EI1, which fires when any
of the row lines PA0 through PA6 goes low. Sampling it means asking the
same question the CPU asks when it reads `$FF00`, once per scanline,
*without* performing a bus read and therefore without clearing anybody's
interrupt flags. Because `pia0_pa_pins` is a pure function of `&self`, the
GIME can call it freely. Had the pin composition been folded into the PIA
read path as a side effect, this second consumer would have needed a
duplicate implementation, and the two would eventually have disagreed. The
`& 0x7F` masks off PA7 so the comparator can't masquerade as a keypress,
which is the same defensive move as the ROM's `ORA #$80` in §10.5.
`kbd_line_low` is the previous sample, retained so that only a *transition*
raises the interrupt rather than every scanline of a held key. Edge
detection from stored levels, one more time; the pattern is everywhere in
this chapter once you start looking.

PIA1's two input-pin functions are smaller but follow the same shape, and
they are worth a glance because Chapters 11, 12, and 14 lean on them without
re-deriving the pattern:

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

Bit 0 is the cassette data-in line, which is Chapter 12's entire subject.
Every other bit floats high, matching `PiaPort::default`'s `0xFF` idle
state from §10.2's Rust corner, because nothing is wired to them. Writing
`0xFF` and `!CASSETTE_IN` rather than `0x01` and `0x00` keeps the floating
pins visible in the source: the reader can see that seven bits are
deliberately idle rather than accidentally zero.

`pia1_pb_pins` is the same idea with two real signals composed onto an
otherwise floating `0xFF`: the printer's BUSY line on bit 0, which Chapter 14
uses, and the RAMSZ memory-size sense switch on bit 2, which exists only
on the CoCo 1 and 2. Its doc comment is one of the longer ones in the
codebase and tells a genuinely interesting story about why a CoCo 1 needed
a ROM upgrade before it could use 64K of RAM. That story is a detour from
this week's throughline, and the comment tells it better than a paraphrase
would; read it in the source.

---

## 10.8 Host-side, briefly: two keymaps and a type-ahead queue

Chapter 15 owns the frontend in full. What follows is just enough to close
the loop from a human's actual keyboard to the matrix `sense()` scans,
because a chapter about input that stops at the emulator boundary leaves
the most obvious question unanswered.

`coco-egui` supports two input philosophies, and the choice between them
matters more than it might seem. **Positional** mode maps each host
physical key directly to the CoCo matrix position that sits in roughly the
same place on a real CoCo keyboard. Press the key labeled `-` on a modern
keyboard and what reaches the matrix is whatever CoCo key occupies that
physical spot, regardless of what symbol either keyboard prints there.
[`keymap.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/keymap.rs#L4-L43):

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

Note the comment on `K::Minus => (5, 2)`. A host `-` key lands on the CoCo
position that produces `:`, because that is where MAME's reference layout
puts it. Positional mode is about matching the *layout* of a real CoCo
keyboard, not the symbols printed on a modern one. It's the right mode for
games, which overwhelmingly read the matrix directly and bypass BASIC's
symbolic layer entirely; a game that checks for "the key at row 3, column
3" wants that key to be where the CoCo's arrow key was.

Positional mode has one more responsibility that the table doesn't show.
Modifiers are not matrix positions in egui's event model but a separate
bitfield, so they're applied first, every frame, from the current
modifier state
([`app/input.rs:115-135`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/input.rs#L115-L135)):

```rust
        let kb = &mut self.machine.bus.keyboard;
        kb.set(kbd::SHIFT, mods.shift);
        kb.set(kbd::CTRL, mods.ctrl);
        kb.set(kbd::ALT, mods.alt);
```

Three matrix positions driven straight from the host's modifier state, and
then the key events on top. That's why the positional keymap table has no
entries for SHIFT, CTRL, or ALT: they arrive by a different road.

**Symbolic** mode goes the other way. It wants the *character* that was
typed, whatever host key produced it, and looks it up with the same
`char_key` function from `keyboard.rs` that §10.5 introduced. The
important part of `char_key`'s return type is the `bool` riding alongside
the position: it says whether the CoCo's own SHIFT key must be held to
produce that character. That is not the same question as whether the host
user held shift. The CoCo's case convention is inverted from a modern
keyboard's, so an unshifted host `a` and a shifted host `A` map to the
same matrix position with *different* CoCo shift requirements, and
`char_key` is the one place that conversion lives. It's what makes
clipboard paste and typed text work without the user thinking about the
CoCo's SHIFT key at all, and it's what every headless test in this course
that types at a BASIC prompt calls too.

There's a timing problem left, and it's a good illustration of why
emulator frontends can't treat input as instantaneous. A real key press
has to be *held* for multiple emulated fields before the ROM's scan loop
will register it, because §10.5's `KEYIN` only runs when BASIC's main loop
gets around to calling it. A single-field tap can land entirely between
two calls and never be seen at all. So symbolic mode doesn't set and
immediately clear a matrix position. It queues taps and drains them over
real emulated time, one field at a time
([`typeahead.rs:47-69`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/typeahead.rs#L47-L69)):

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

The state machine has three phases, and each call advances it by one field.
Hold the key down for `TYPE_HOLD_FIELDS` fields, release it, wait
`TYPE_GAP_FIELDS` fields, then move to the next queued tap. The gap is not
decoration: without it, two identical consecutive characters would look to
the ROM's debounce logic like one continuous hold, and `HELLO` would echo
as `HELO`. Section 10.5's walk through `EORA ,X` / `ANDA ,X` is precisely
the code that would swallow the second `L`.

This is the production-code twin of the `tap` and `tap_char` helpers that
this chapter's tests use directly (§10.9). Same shape, same reason, one
written for a human typing through an egui window and one written for a
test asserting on a screen buffer.

---

## 10.9 Reading the tests

Four tests, across three files, each pinning a fact this chapter has
already walked through in the source. The habit from Chapter 1 (§1.7) applies
here more than anywhere: read the test first, and it tells you what the
hardware does in five lines that the implementation spreads across three
functions.

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

Three assertions covering the three cases that matter: the strobed column
senses the key, a different column doesn't, and — the interesting one —
strobing *every* column at once still senses it. That third case works
because `sense()`'s `pressed & selected` test only needs *some* overlap
between the held key's column and the set of currently selected columns.
Nor is it a hypothetical: it's the exact operation Color BASIC's
`LA1C1` fast path performs with `CLR PIA0+2` before deciding whether to
pay for a full column-by-column scan (§10.5). A `sense()` that took a
column *index* instead of a strobe *byte* would have been simpler to write
and would have made that ROM path impossible to model.

**`falling_edge_selected_port_flags_only_on_high_to_low`**
([`crates/coco-core/tests/pia_sync.rs:26-33`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/pia_sync.rs#L26-L33)) exercises §10.3's edge logic
through the real bus entry point, `hsync()`, rather than through
`PiaPort::set_c1` directly:

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

The altitude is the point. This test asserts that the whole per-scanline
call sequence in `hsync()` produces the flag that stock BASIC's default
control-register setup expects, not merely that `set_c1` in isolation
obeys its contract. That narrower claim has its own test,
`pia::tests::falling_edge_selected_flags_only_on_high_to_low`, colocated
in [`pia.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/pia.rs) itself via `#[path = "pia_test.rs"] mod tests;`. Two tests,
two altitudes, and both genuinely needed — the sabotage exercise below
demonstrates exactly why, by breaking the shared logic underneath them and
watching only one of the two notice.

**`cb1_falling_flag_first_appears_at_fs_falling_line_not_before`**
([`crates/coco-core/tests/pia_sync.rs:50-70`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/pia_sync.rs#L50-L70)) checks a fact that is easy
to get subtly wrong: the field-sync flag must appear at the *real*
scanline the field-sync pulse occurs, not merely somewhere during the
field.

```rust
#[test]
fn cb1_falling_flag_first_appears_at_fs_falling_line_not_before() {
    let mut b = bus();
    // Default CRB ($FF03=0): falling edge selected, matching stock BASIC's
    // $34/$35 ROM setup.
    let falling_line = VideoStandard::NTSC.fs_falling_line(MachineVariant::Coco3);
    for _ in 0..falling_line {
        b.hsync(); // drives CA1 only; CB1 must stay untouched all field
        assert_eq!(
            b.pia0.b.control & cr::C1_FLAG,
            0,
            "CB1 flag must not appear before the field-sync falling edge"
        );
    }
    b.fs_falling();
    assert_ne!(
        b.pia0.b.control & cr::C1_FLAG,
        0,
        "CB1 flag must appear exactly at the falling-edge scanline"
    );
}
```

This is a negative-space test. Since `fs_falling_line` is 244 for NTSC on
a CoCo 3, the loop spends 244 iterations asserting the flag is *still not
there*, followed by one assertion that it now is. That shape, proving an
invariant holds continuously right up to the instant it's supposed to
change, is how you test "this fires at the *right time*" as opposed to
merely "this fires eventually." Code that accidentally raised CB1 early,
say by confusing `fs_falling_line` with a different line count, would fail
on some iteration long before the loop reached `fs_falling()`, and the
failure message would tell you which iteration.

The loop also quietly asserts something else worth noticing: that
`hsync()` touches side A and *only* side A. Two hundred and forty-four
calls to a function that pulses CA1 must leave CB1's flag alone, and if
someone ever wired the horizontal sync to the wrong side, this test fails
on the first iteration.

**`fire_buttons_pull_rows_low_regardless_of_strobe`**
([`crates/coco-core/tests/joystick_bus.rs:65-79`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/joystick_bus.rs#L65-L79)) is §10.6's
"buttons bypass the strobe" claim, verified through the real bus:

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

`b.pia0.b.output = 0xFF` deliberately puts the matrix into the state where
*no key* could possibly be sensed, which is the same setup the ROM's own
`LDB #$FF` probe at `$A213` uses (§10.6), and the test still finds two row
lines pulled low. The final assertion is the one that makes it a real
test rather than a smoke test: rows 1 and 2 must stay *high*, so a buggy
`button_rows()` that returned an over-broad mask would fail here even
though the first two assertions passed.

That's the whole reason `button_rows()` appears in `pia0_pa_pins` as a
separate `&=` step composed after `keyboard.sense(...)`, rather than being
folded into the keyboard's own logic. Buttons genuinely don't care what
the strobe register says, and the code says so structurally.

> One honest caveat before you run any of these yourself: `keyboard.rs`'s
> two end-to-end tests (`typing_at_prompt_echoes_to_screen`,
> `typing_multiple_keys_with_irqs_active`) boot the real `roms/coco3.rom`
> and will panic with a "file not found" error in a worktree that lacks
> `roms/` (Chapter 1, §1.7 warned you about exactly this — ROMs live only in
> the main checkout). The four tests walked above need no ROM at all:
> `cargo test -p coco-core --test pia_sync --test joystick_bus`, the
> `pia::tests::*` set, and the first two tests in `keyboard.rs` all run
> clean anywhere.

---

## 10.10 Reading assignment

In this order. The sequence is deliberate: the chip first, then the two
devices hanging off it, then the composition layer that joins them, then
the tests that pin the whole thing down.

1. **[`crates/coco-core/src/pia.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/pia.rs)**, the whole file (188 lines) — you've
   now seen nearly every line of it quoted in this chapter, but read it once
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
`Keyboard::sense` returns for each strobe, assuming 'K' is the only key
held. Which single strobe value produces a non-`0xFF` result, and what
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
implementing the core-side half of Chapter 15's frontend feature.)

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
Someone proposes: "just give every PIA port a fixed hardware direction —
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

Next week stays inside PIA1 but follows a different pin. The 6-bit DAC
this chapter used only as a comparator reference voltage becomes, in week
11, an actual audio signal: cycle-timestamped writes rendered into a
sample grid a sound card can play. The PIA-side plumbing has already made
its cameo here, in `GETJOY`'s opening `BSR LA974` that mutes the sound mux
before sweeping the DAC, and in the CA2/CB2 select bits that pick between
sound sources. Chapter 11 is where that plumbing gets followed all the way to
a waveform, and where the awkward fact that the joystick and the speaker
share six pins turns from a footnote into a scheduling problem.

Chapter 12, after that, follows PIA1's *other* pins — the ones this chapter
only named — into a cassette deck, where a single input bit and a single
output bit carry a complete frequency-shift-keyed modem, demodulated in
software by a ROM that has no idea it is being emulated.
