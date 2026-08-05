# Chapter 5 — The Bus: Memory Maps, the SAM, and the GIME MMU

*Week 5. Goal: resolve a CPU address on each CoCo model. This chapter follows
reads and writes through decode priority, the SAM, the GIME's ROM controls,
and the MMU, including the hardwired reset-vector range.*

The CPU chapters used a flat 64K test bus. A CoCo bus must instead decide
whether each address reaches RAM, ROM, a device register, or no responding
device. `SystemBus` implements that decision.

The `mc6809` crate does not know a CoCo exists. From the CPU's perspective,
`SystemBus` is one 64K address space. Control bits can make the same logical
address select RAM, one of two ROM sources, a side-effecting device register,
or open bus.

The ranges overlap and several move under software control. Decode priority
resolves those overlaps. One small range at the top of memory remains mapped
to internal ROM regardless of the other controls so the CPU can always fetch
its vectors.

The CoCo 1/2 and CoCo 3 use different address-decoding hardware, so the
codebase keeps two independent decode paths.
Section 5.5 tells the older, simpler story; §§5.6–5.10 tell the newer,
more elaborate one; and §5.4, in between, is the argument for why the two
are deliberately never merged.

---

## 5.1 Two chips, one seam

The CPU calls `bus.read(addr)` or `bus.write(addr, val)` with a 16-bit address.
The bus must resolve that logical address to a physical byte or device
register.

The result depends on the machine model, software-controlled mapping bits,
and the priority of overlapping ranges.

Start with the generational split, because it is the coarsest of the
three dependencies and the one that structures the rest of the chapter.
The CoCo 1 and CoCo 2 delegate the decision to an MC6883 SAM, the
Synchronous Address Multiplexer introduced in Chapter 1's tour of the
machine. The SAM holds a handful of latched bits and maps the 64K space
to ROM, RAM, or the I/O page in a single lookup. There is no MMU
anywhere in that picture: what the CPU sees is what the board has, banked
only by a single `P1` bit that swaps the upper half of RAM into the lower
half on 64K machines.

The CoCo 3 replaces both halves of that arrangement with the GIME. It
absorbs the SAM's job — in a compatibility layer, so that software
written for the older machines keeps working — and then adds the feature
that actually distinguishes the machine: a real memory management unit.
Eight logical 8K slots, each independently pointed at any 8K physical
block in up to 2 MB of RAM, with two complete sets of those eight
registers so that an entire 64K view can be swapped in a single write.
Sections 5.7 and 5.8 are devoted to that mechanism and to what real
software did with it.

Both chips live in the same file,
[`crates/coco-core/src/bus.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs), and inside
the same struct. `SystemBus` implements the
`Bus` trait from Chapter 1, which makes it the concrete thing sitting on the
far side of the CPU's seam. Read the field list below less as a data
structure and more as an inventory of the machine: nearly every field is
a device that gets its own chapter later in the course.

```rust
pub struct SystemBus {
    pub variant: MachineVariant,
    #[serde(with = "serde_bytes")]
    pub ram: Box<[u8]>,
    #[serde(skip)]
    pub rom: Box<[u8]>,
    pub gime: GIME,
    pub sam: SAM,
    pub pia0: MC6821,
    pub pia1: MC6821,
    pub cart: Cart,
    pub vhd: VHD,
    // ...
}
```
*([`crates/coco-core/src/bus.rs:36-110`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L36-L110), trimmed to the fields this
chapter needs.)*

The first field, `variant`, is the machine's identity, and everything
else in the struct is either memory or a device. `ram` and `rom` are the
two `Box<[u8]>` buffers Chapter 1 dissected. Then come the decode chips,
then the two PIAs, then the cartridge slot and the virtual hard disk, and
in the full declaration a further dozen fields covering the keyboard,
joysticks, cassette, serial port, and the debugger's watchpoint table.

Both `gime: GIME` and `sam: SAM` are present on every machine. A CoCo 1
allocates a `GIME` it does not use;
a CoCo 3 allocates a `SAM` it will never once look at. Two decode chips
that never coexisted in any real machine are both sitting in the struct
at the same time, which looks like waste until you price the
alternatives.

This keeps `SystemBus` a single concrete type. There is no
generic parameter, no trait object, and no enum-of-machines standing
between the CPU's hottest path and a memory access — `bus.read(addr)`
compiles to a direct call every time, exactly as Chapter 1's discussion of
monomorphization promised. Just as importantly, and this is the payoff
you'll recognize from Chapter 1's argument about trees versus graphs, a
single `#[derive(Serialize, Deserialize)]` covers the whole thing for
save states. Nothing has to ask "which variant am I" at serialization
time, because there is only ever one shape of struct to write out. A few
dozen bytes of unused device state is a cheap price for that, and it is
paid once at construction rather than per access.

What remains is the mechanism by which the CPU picks between the two
decoders on any given access, and the ordered set of rules each decoder
applies once chosen. The next section takes the second of those first,
because the ordering is the part that most rewards being internalized
before any of the individual rules.

---

## 5.2 Decode order: what wins when address ranges overlap

Decode priority must be applied before individual ranges can be interpreted.
Several ranges overlap. `$FFFE`, the
reset vector, is inside the I/O page, inside the ROM window, and inside
whatever the MMU happens to be mapping. Three plausible answers; the
machine has exactly one.

The CoCo 3 path is implemented by
[`crates/coco-core/src/bus.rs:267-294`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L267-L294). This is the CoCo 3 path; the
`variant != Coco3` branch near the top peels off to an entirely
different function discussed in §5.5. The CoCo 3 path is a sequence of
guarded early returns:

```rust
impl Bus for SystemBus {
    fn read(&mut self, addr: u16) -> u8 {
        // Debugger watch hook: a single null-check when no watchpoints are
        // installed (the common case), so the hot path is unchanged.
        if self.watch.is_some() {
            self.note_watch(addr, crate::debug::WatchKind::Read);
        }
        // Two independent concrete decode paths, branched once up front
        // (`docs/coco12-plan.md` Phase 2) — not a trait object, so both stay
        // cycle-honest and the GIME path is untouched by the plain-SAM one.
        if self.variant != MachineVariant::Coco3 {
            return self.sam_read(addr);
        }
        // Hardwired to internal ROM ahead of everything else — I/O decode,
        // ROM mapping, and MMU state all take a back seat here (fact behind
        // `HARDWIRED_ROM_BASE`).
        if addr >= HARDWIRED_ROM_BASE {
            return self.rom_read(addr);
        }
        if self.io_enabled && addr >= IO_BASE {
            return self.io_read(addr);
        }
        if self.is_rom_window(addr) {
            return self.rom_read(addr);
        }
        let p = self.phys(addr);
        self.ram[p]
    }
    // write() mirrors this order — see below.
}
```

Strip away the debugger hook at the top and the variant branch below it,
and what's left is four tiers, checked strictly top to bottom. Each tier
is a *precondition* rather than a partition: once its condition is true,
the search ends there and no lower tier is ever consulted, whether or not
the address also falls inside that lower tier's range.

The first tier is `$FFE0–$FFFF`, hardwired to internal ROM. The constant
is `HARDWIRED_ROM_BASE = 0xFFE0`, and what is striking about it is that
it is checked *before* the I/O-page test even though `$FFE0` is
numerically inside `$FF00–$FFFF` as well. That ordering is not a stylistic
preference. If the I/O test came first, the six 6809 hardware vectors —
including the reset vector at `$FFFE` — would be swallowed by the
I/O-page branch and read back as unmapped I/O garbage, and the machine
would never execute a single useful instruction. Section 5.13 traces
exactly this address through reset, and exercise 5.6 asks you to justify
the ordering from first principles rather than from the code.

The constant's doc comment lists the controls that cannot move the range and
cites the evidence, from
[`crates/coco-core/src/bus/regs.rs:85-92`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/regs.rs#L85-L92):

```rust
/// `$FFE0–$FFFF` — the top 32 bytes of the `$8000–$FFFF` window, including the
/// 6809 hardware vectors — is hardwired to internal ROM on every read,
/// regardless of INIT0 MC1:MC0, the SAM TY map-type bit (`$FFDE`/`$FFDF`,
/// all-RAM mode), MMU state, or any inserted cartridge. MAME `coco3.cpp:53-58`
/// documents this as verified by William Astle's real-hardware test, which
/// refutes SEB Unravelled II p.28's claim that this range aliases `$BFFx`.
/// Writes here are dropped (`SystemBus::write`) — it isn't backed by RAM.
pub(super) const HARDWIRED_ROM_BASE: u16 = 0xFFE0;
```

That comment names four separate mechanisms, none of which can touch
this range, and every one of them is a mechanism this chapter goes on to
build. Note also the shape of the citation: MAME's source, a
real-hardware test by a named person, and an explicit statement that a
published reference book gets this wrong. That is what a hardware claim
looks like when someone has actually checked, and it is the standard the
rest of this chapter holds its own claims to.

The second tier is the I/O page itself, `$FF00–$FFDF`, gated on
`io_enabled`. That flag is a debugger convenience and is always `true` on
a running machine, so for now read the condition as simply "the address is
at or above `$FF00`." Because tier 1 already returned for everything from
`$FFE0` up, this tier can never contest the hardwired vectors no matter
how its range is written.

The third tier is the ROM window, and it is the only one of the four
whose condition is a function call rather than a comparison:
`is_rom_window(addr)` covers `$8000–$FDFF`, but gated on whether ROM is
currently mapped in at all — the SAM's map-type bit, or the GIME's
equivalent — and with a special case for the `$FE00–$FEFF` page that
§5.7 unpacks. Section 5.6 is a full deep dive into what happens *inside*
this tier once it has been entered, because "ROM" turns out to mean one
of two entirely different chips depending on two bits in `INIT0`.

The fourth tier is everything else, which is to say ordinary memory.
`phys(addr)` runs the MMU translation — or the fixed map used when the
MMU is switched off — and the result indexes straight into `self.ram`.
Most accesses a running machine makes land here, which is a good reminder
that the exotic tiers above are exceptions carved out of an otherwise
simple story.

Writes use the same priority tiers, from
[`crates/coco-core/src/bus.rs:296-315`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L296-L315):

```rust
    fn write(&mut self, addr: u16, val: u8) {
        if self.watch.is_some() {
            self.note_watch(addr, crate::debug::WatchKind::Write);
        }
        if self.variant != MachineVariant::Coco3 {
            self.sam_write(addr, val);
            return;
        }
        // $FFE0–$FFFF is ROM, not RAM: writes there are dropped.
        if addr >= HARDWIRED_ROM_BASE {
            return;
        }
        if self.io_enabled && addr >= IO_BASE {
            self.io_write(addr, val);
            return;
        }
        // ROM is read-only; writes to the ROM window reach the RAM mapped beneath it.
        let p = self.phys(addr);
        self.ram[p] = val;
    }
```

Two asymmetries jump out, and both are hardware facts rather than
conveniences. Tier 1 has become a bare `return` with no side effect at
all: `$FFE0–$FFFF` isn't backed by RAM anywhere in the machine, so there
is nothing for a write to land on and no "write-through" concept to
implement. A program that stores to the reset vector is not making a
mistake the emulator needs to report; it is simply doing something the
silicon quietly ignores.

The second asymmetry is more interesting, and it is the reason the ROM
tier has no counterpart in `write` at all. On the read side, an address
in a mapped ROM window returns a ROM byte. On the write side, that same
address falls straight through to `phys()` and lands on the RAM sitting
physically underneath the ROM overlay. ROM is read-only silicon, but the
RAM behind it is real, still addressable by the MMU, and still there when
the ROM is later switched out. Software can therefore write a byte to
`$C000`, read `$C000` back, and get a completely different value — not
because the write failed, but because the read and the write resolved to
two different chips. Section 5.14's second worked example demonstrates
exactly this, and §5.5 shows that the CoCo 1/2 path deliberately does
*not* behave this way.

> **Rust corner: precedence as a stack of early returns.** Notice what
> isn't in either function. There is no `match` on address ranges, no
> priority field on a device record, no sorted list of decoders consulted
> in turn. There are four `if ... return` statements in a fixed order,
> and that is the entire dispatch mechanism. This is a common Rust idiom
> for "first matching rule wins": each guard either returns immediately
> or falls through to the next, and the order in the source *is* the
> semantics.
>
> The reason to prefer it here, over the `match` that a Rust programmer's
> instincts might reach for first, is that a `match` carries an implicit
> promise its arms are mutually exclusive. Here they emphatically are
> not: `$FFE0` legitimately belongs to two of the four ranges, and a
> reader needs to see which one wins. Written as a `match`, that fact
> would be buried in arm ordering — invisible to anyone skimming, and
> silently reshuffled the first time someone tidies the arms into
> numerical order. Written as early returns, the overlap and its
> resolution are both on the page. Reach for a `match` when the arms
> really do partition the space, and for an ordered guard chain when they
> genuinely contest it.

With the priority order established, the individual ranges can be taken
one at a time — starting with the busiest 256 bytes in the machine.

---

## 5.3 The I/O page, wall to wall

Of the 65,536 addresses the 6809 can name, 256 of them account for every
conversation the CPU ever has with a device. `$FF00–$FFFF` is the I/O
page, and it is the one region of the address map that is fixed on every
CoCo ever built: it does not move when the MMU is reprogrammed, it does
not move when ROM is switched in or out, and it is the same page on a
CoCo 1 as on a CoCo 3. The only unusual thing inside it is the
hardwired-vector carve-out at the top, which is tier 1 from the previous
section and which the table below marks accordingly.

That stability is what makes the I/O page worth learning as a unit. Every
week of this course from here on lands somewhere in this table — the
keyboard in Chapter 10, the sound DAC in Chapter 11, the cassette in Chapter 12,
the disk controller in Chapter 13, the serial port in Chapter 14 — and each of
those chapters will assume the address is already familiar. Chapter 1
introduced a short version of this map, seven rows deep, as a bookmark.
Here is the complete one, taken not from DESIGN.md's summary but from
the literal `match` arms that
[`crates/coco-core/src/bus/io.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs)'s `io_read` and `io_write`
dispatch on:

| Range | Device | Notes |
|---|---|---|
| `$FF00–$FF1F` | PIA0 | Keyboard rows, joystick comparator, sync IRQs. Only 4 registers exist; `addr & 0x03` mirrors them across the whole 32-byte range. |
| `$FF20–$FF3F` | PIA1 | 6-bit DAC, cassette, VDG-legacy mode bits. Same 4-register mirror. |
| `$FF40–$FF7E` | Cartridge / FDC (SCS\*) | The "standard" SCS window is `$FF40–$FF5F`; some carts (RS-232 Pak, Orchestra-90, the Sound/Speech Cartridge) decode further registers out to `$FF7E` — the full address bus reaches the expansion connector regardless of what the motherboard "intends." |
| `$FF41` / `$FF42` | Becker port (DriveWire) | Intercepts **ahead of** cartridge dispatch, on both decode paths, whenever a `DWServer` is installed — mirrors MAME's handler-install order. |
| `$FF7F` | Multi-Pak Interface select | Only meaningful with an MPI inserted; decoded by the MPI itself, never by a plugged-in cart. |
| `$FF80–$FF86` | VHD (virtual hard disk, NitrOS-9 `emudsk`) | `$FF87–$FF8F` is unmapped/open bus. |
| `$FF90` | INIT0 | MMU enable, ROM map bits, MC3, IRQ/FIRQ master enables, CoCo-compat select. |
| `$FF91` | INIT1 | Timer clock select, MMU task select. |
| `$FF92` | IRQENR | IRQ source enable (write) / latched status, clear-on-read (read). |
| `$FF93` | FIRQENR | FIRQ twin of `$FF92`. |
| `$FF94`/`$FF95` | Timer MSB/LSB | 12-bit interval timer; write-only (reads as 0 on hardware). |
| `$FF96`/`$FF97` | reserved | Unused on the GIME. |
| `$FF98`–`$FF9F` | GIME video: VMODE, VRES, BORDER, VBANK, VSCROLL, VOFFSET1/0, HOFFSET | Chapter 8 territory; write-only like the timer regs (`io.rs`'s `TIMER_MSB_REG..=GIME_LAST => 0` catch-all). |
| `$FFA0–$FFAF` | MMU task registers | `$FFA0–$FFA7` = task 0 slots 0–7, `$FFA8–$FFAF` = task 1 slots 0–7. §5.7/§5.8. |
| `$FFB0–$FFBF` | Palette | 16 registers, 6-bit RGB each. Chapter 8. |
| `$FFC0–$FFDF` | SAM-compatibility strobes | Write-only, even/odd set-clear pairs. §5.4/§5.5. |
| `$FFE0–$FFFF` | Hardwired internal ROM | Tier 1 from §5.2 — bypasses everything else on this table. |

Two details in that table trip people up when reading the source cold,
and both are worth internalizing now because they recur in every device
chapter that follows.

The first is the expression `addr & 0x03`, which is how both PIA arms
pick which of four registers an access names — `io_read`'s
`IO_BASE..=PIA0_LAST => { ...; self.pia0.read((addr & 0x03) as u8) }`
([`bus/io.rs:61-66`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L61-L66)). A PIA has exactly four registers, so
it decodes exactly two address lines, and every other line reaching the
chip is a "don't care" that no internal logic ever examines. The
consequence is that `$FF00`, `$FF04`, `$FF08`, and every other address in
`$FF00–$FF1F` whose bottom two bits are zero all reach the *same*
register. That mirroring is not a shortcut the emulator took to save a
match arm; it is a literal description of how the chip's address pins are
wired, and software of the period cheerfully relied on it.

The second is that most GIME video and timer registers are write-only on
real hardware, which is why `io_read` disposes of the entire
`$FF94–$FF9F` span with a single arm returning zero
(`TIMER_MSB_REG..=GIME_LAST => 0`, [`bus/io.rs:83`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L83)). That arm looks
like an unfinished stub and is nothing of the kind: a real GIME's video
registers genuinely do not drive the data bus when read, so the value the
CPU sees is whatever the bus happens to supply. Anyone who has written
6809 assembly that tries to read-modify-write `$FF98` — load the current
mode byte, set one bit, store it back — will recognize the resulting
bug immediately. The important thing to understand is that it is not the
emulator's bug. It is period-accurate. Note that the emulator does
keep the written value — `io_write`'s `VMODE_REG => self.gime.vmode =
val` arm latches it into the `GIME` struct, where week 8's renderer will
read it — so the information is not lost. It is simply not available to
the 6809 through a bus read, which is exactly the situation on the real
chip.

### Walking the dispatch: PIA, cassette, cart, VHD

The table above says *where* each device lives. Reading the dispatch
itself is a different exercise: it tells you what the *shape* of a device
match arm is, and that shape is remarkably consistent across a set of
devices that have almost nothing else in common. Chapters 10 through 14 dig
into these devices one at a time, and each of those chapters will go
faster if the arm's structure is already familiar. Three patterns recur
across nearly every arm in `io_read` and `io_write`, and one worked
example of each is enough to recognize the rest.

The first pattern is that a device's *input pins are computed at the
moment of access, not stored between accesses*. A PIA is a deliberately
dumb chip: it latches whatever voltage happens to be on its input pins
when the CPU reads it, and it holds a direction register saying which
pins are inputs in the first place. Nothing in this emulator pushes a
live voltage into `pia0` or `pia1` between accesses — no per-cycle update,
no device pumping values in. Instead the bus computes the right byte on
demand, immediately before handing the read to the PIA:

```rust
IO_BASE..=PIA0_LAST => {
    // Refresh port A's input pins (keyboard rows + joystick
    // comparator/buttons) before the PIA read.
    self.pia0.a.input = self.pia0_pa_pins();
    self.pia0.read((addr & 0x03) as u8)
}
```
*([`bus/io.rs:61-66`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L61-L66))*

The interesting half of that arm is the assignment on the first line, not
the `read` call on the second. `self.pia0.a.input` is being overwritten
with a freshly computed byte on every single read, so whatever was in it
before is irrelevant — the PIA is not remembering its inputs, it is being
told them. `pia0_pa_pins` is the function that does the computing, and it
reaches into three unrelated devices to do it:

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
*([`bus/pins.rs:14-27`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/pins.rs#L14-L27))*

That single byte on PIA0 port A is doing three unrelated jobs at once.
Bits 0 through 6 report which keys are pressed in whichever keyboard
column PIA0's port B is currently strobing. Those same bits get pulled
low by joystick fire buttons regardless of the strobe, which is why the
`&= !button_rows()` line comes second and unconditionally. And bit 7,
`COMPARATOR_BIT`, is an analog comparator output: high while the six-bit
DAC value read out of PIA1 sits at or below the joystick potentiometer
that the two `c2_output()` select lines have chosen. Three subsystems,
one byte, because on the real board that is exactly what is wired to
those eight pins.

Chapter 10 spends its time inside `keyboard.rs` and `joystick.rs` and will
make sense of the individual pieces. The lesson to take from it now is
structural rather than electrical. A PIA's "input" is not state that the
PIA owns and maintains; it is a snapshot the bus takes of everything else
in the machine, recomputed from scratch on every single read. Nothing can
go stale, because nothing is ever stored. PIA1's port A gets the same
treatment for the cassette input line, at the opposite extreme of
complexity — one bit, one device, and a function short enough to quote
whole:

```rust
pub(super) fn pia1_pa_pins(&self) -> u8 {
    const CASSETTE_IN: u8 = 0x01;
    if self.cassette.input_bit() { 0xFF } else { !CASSETTE_IN }
}
```
*([`bus/pins.rs:33-40`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/pins.rs#L33-L40))*

Note the polarity: a set cassette bit produces `0xFF`, and a clear one
produces `!CASSETTE_IN`, which is `0xFE` — every bit high except bit 0.
Unused CoCo input pins float high, so "all ones except the one bit we
actually model" is the honest answer rather than a lazy one. Chapter 12 will
care intensely about the timing of that single bit; this week only cares
that it is computed rather than stored, exactly like PIA0's far busier
port A.

The second pattern is that *some writes fan out to more than one device*.
Cassette output is the mirror image of cassette input, and it does not
live behind an address of its own at all. It is tapped directly off the
PIA1 Port A write that just happened, because the six-bit sound DAC and
the cassette recording circuit are physically the same port pins:

```rust
    pub(super) fn write_pia1(&mut self, addr: u16, val: u8) {
        let reg = addr & PIA1_REG_MASK;
        self.pia1.write(reg as u8, val);
        // Cassette record-out is a direct, unconditional tap of the DAC
        // (not gated by SNDEN/the mux — `cassette-verified-facts`), but
        // it only samples on Port A output/DDR writes, not CRA ($FF21)
        // writes: MAME's `update_cassout()` is called exclusively from
        // `pia1_pa_changed()`, never from `pia1_ca2_w()` (the CA2
        // motor-relay callback) — and `write_control()`'s CRA path
        // never touches `port.output`/`port.ddr` anyway, so a CRA-only
        // write can't change the DAC value. Sampling on CRA writes
        // would just re-announce the unchanged level as a spurious
        // transition right after motor-off resets `last_level`.
        if reg == PIA1_PORT_A_OFFSET {
            let dac = (self.pia1.a.output & self.pia1.a.ddr & 0xFC) >> 2;
            self.cassette.record_dac(dac, self.pia1.a.c2_output());
        }
        self.note_audio_write(); // DAC / PB1 / SNDEN / relay
    }
```
*([`bus/io.rs:154-172`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L154-L172); both bus
paths' `PIA1_BASE..=PIA1_LAST` write arms dispatch here)*

Every PIA1 write lands in this helper, and a write to Port A's data or
direction register re-derives the current six-bit DAC value and feeds it
to `cassette.record_dac`. A write to the output register obviously
changes what the recording circuit sees. So does a write to the
data-direction register, since a pin switched from input to output
starts driving whatever the output register already held — which is why
the mask is `output & ddr` rather than `output` alone. A write to the
control register, by contrast, cannot change the DAC value at all — the
CRA path never touches the output or direction registers — so the tap
does not sample there, a boundary the comment pins to MAME's model of
the same wiring. The `note_audio_write` call still runs for every
register, because the sound path cares about more than the DAC. Chapters
11 and 12 both build directly on this tap.

The third pattern is *a device group with its own small sub-dispatch*.
Where the PIAs mirror four registers across a 32-byte range, other
devices claim a short run of consecutive addresses and give each one its
own line. The virtual hard disk at `$FF80–$FF86` — NitrOS-9's `emudsk`
device, Chapter 13's material — is a compact example of the shape:

```rust
VHD_LRN_HI | VHD_LRN_MID | VHD_LRN_LO | VHD_BUFFER_HI | VHD_BUFFER_LO => {
    self.vhd.read_lrn_or_buffer()
}
VHD_COMMAND_STATUS => self.vhd.read_status(),
VHD_SELECT => OPEN_BUS, // always open bus, unconditionally (spec)
```
*([`bus/io.rs:74-78`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L74-L78), read side)*

Seven addresses, three behaviors. Three of the addresses are the bytes of a
*logical record number*, the sector index the host is being asked to
fetch, and two more are a pointer to the buffer the result should land
in; all five share one read arm because reading any of them returns the
same thing. `$FF83` is the command and status register, the address that
makes something actually happen. And `$FF86`, the drive-select register,
always reads back open bus no matter what has been written to it — which,
again, is the device's own specification rather than an unimplemented
stub. Section 5.11 collects every open-bus convention in the codebase,
VHD's included, into one place, because "reads back a fixed value nothing
wrote" turns out to be a surprisingly common answer.

One overlap inside the I/O page deserves naming before Chapter 13 meets the
rest of the cartridge range, because it is the precedence question from
§5.2 recurring at a smaller scale. The Becker port — `$FF41` and `$FF42`,
the DriveWire-over-serial interface — sits *inside* the cartridge's
`$FF40–$FF7E` range. Two devices, two addresses, one range, and both
`io_read` and the plain-SAM path's I/O sub-decode resolve it the same
way: check the Becker port first, unconditionally, before the cartridge
is offered the address at all.

```rust
pub(super) fn io_read(&mut self, addr: u16) -> u8 {
    // Becker-port precedence over cartridge dispatch — mirrors MAME's
    // handler-installation order over the SCS window.
    if let Some(v) = self.becker_read(addr) {
        return v;
    }
    match addr {
        // ... cartridge and everything else ...
    }
}
```
*([`bus/io.rs:54-60`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L54-L60), abbreviated)*

The comment cites MAME's handler-installation order as the reason, which
is a specific and checkable claim rather than an appeal to plausibility:
MAME installs the Becker handlers over the top of the cartridge window,
so whichever emulator you compare against, the same address resolves the
same way. The intercept itself returns an `Option`, and that choice is
what lets one function answer two different questions at once:

```rust
pub(super) fn becker_read(&mut self, addr: u16) -> Option<u8> {
    let dw = self.drivewire.as_mut()?;
    match addr {
        BECKER_STATUS => Some(dw.status_read()),
        BECKER_DATA => Some(dw.data_read()),
        _ => None,
    }
}
```
*([`bus/io.rs:26-33`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L26-L33))*

The first line does the heavy lifting. `self.drivewire.as_mut()?` uses
the question-mark operator on an `Option`, so if no DriveWire server is
installed the function returns `None` immediately and the `match` below
never runs. A `None` result therefore means one of two things — "not my
address" or "the Becker port isn't enabled on this machine" — and the
caller does not have to distinguish them, because the response is the
same either way: fall through to the ordinary cartridge decode
underneath. Only a `Some` diverts the access.

This is the same "first matching rule wins, but only when it actually
matches" idea as §5.2's Rust corner, applied one level down and inside a
single device's address range rather than across the whole 64K. It is
also a small demonstration of why the four-tier chain up top is not
merely a stylistic choice: precedence between overlapping claimants is a
recurring structural problem in a memory map, and it wants a recurring
structural answer.

That is the I/O page end to end, on the CoCo 3. The remaining question
from §5.1 is how the machine gets into `io_read` in the first place —
what happens on the other side of that `variant` check at the top of
`read`.

---

## 5.4 Two machines, two decode paths

At the top of `read` and `write`, one variant check routes the access to the
SAM or GIME decoder. The struct documents that choice:

```rust
/// Which machine this bus decodes addresses for. `Bus::read`/`Bus::write`
/// branch on this once, up front, into two independent concrete decode
/// paths (GIME vs plain SAM) rather than a trait object — see
/// `docs/coco12-plan.md` Phase 2.
pub variant: MachineVariant,
```
*([`bus.rs:38-42`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L38-L42))*

The alternatives move complexity elsewhere.

The first is a `dyn Decoder` trait object, one implementation per
variant, stored in a field and called polymorphically. This is the
obvious object-oriented move, and it loses here for precisely the reason
Chapter 1's `impl Bus` beat `dyn Bus` on the CPU's hot path. Every memory
access — several per instruction, tens of millions per second of emulated
time — would pay a vtable indirection to re-answer a question that was
settled the moment the machine was constructed. The variant cannot change
mid-run. Paying anything at all, repeatedly, for a constant is the wrong
trade, and a predictable branch on a field the CPU cache has held hot for
the last million accesses is about as close to free as a runtime check
gets.

The second is a generic `SystemBus<V: Variant>`, which would recover the
speed through monomorphization. The cost this time is not performance but
contagion. `Machine` would need a type parameter, and so would every
function signature that touches a `Machine`, in the core and the frontend
and the test suite alike. The quieter cost is the one Chapter 16 would pay:
`#[derive(Serialize, Deserialize)]` for save states is pleasant precisely
because `Machine` is one plain, ordinary, singular type. Introduce a type
parameter and every snapshot has to know which instantiation produced it.
That is a tax levied on every chapter after this one, in exchange for
eliminating a branch that costs nothing.

The third temptation is the strongest, because it appeals to a genuine
engineering virtue: merge `sam_read` and the CoCo 3 `read` body into one
function and sprinkle `if variant == Coco3` through the tiers, on the
grounds that duplication is bad. The codebase rejected this explicitly,
and said why in the module doc comment of the file that would have
disappeared,
[`bus/sam_path.rs:1-7`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sam_path.rs#L1-L7):

```rust
//! Plain-SAM path (CoCo 1/2, no GIME): `Sam::map` does the whole-address
//! decode (RAM/ROM/cart/I/O/open-bus) in one step, unlike the GIME path's
//! separate ROM-window/I/O-page/MMU layers, so there's no need for
//! `phys`/`is_rom_window`/`rom_read` equivalents here. This path never
//! touches `self.gime` — no MMU translate, no interrupt raises, no timer
//! (`docs/coco12-plan.md` Phase 2; the field-loop gating that keeps it that
//! way for `hsync`/`fs_*` is Phase 4).
```

The claim in that comment is stronger than "these two chips have
different registers," which would be an argument for a shared function
with different constants. The claim is that they have differently *shaped
decode algorithms*. The SAM resolves an entire address to a target in a
single `match`, as §5.5 is about to show. The GIME needs multiple ordered
tiers because ROM mapping and MMU translation are separate,
independently configurable stages that can each be reprogrammed without
touching the other. There is no shared skeleton to factor out; there is
only one algorithm and a second, unrelated algorithm.

Combining them would make the CoCo 1/2 path carry
machinery it fundamentally does not have — a `phys()` call it must skip,
an MMU it must not consult, a `rom_enabled()` gate that means something
different — for the sake of a code-reuse ideal the two chips never
shared. A little duplication that mirrors two genuinely different pieces
of hardware is more honest than a unification that papers over the
difference, and it is also easier to change later, because a fix to one
decoder cannot possibly break the other.

This is the same "load-bearing abstraction" instinct from Chapter 1,
applied in the opposite direction: there, one seam (`Bus`) was worth
generalizing because every device behind it really does share one
two-method contract. Here, two decoders are worth *not* generalizing
because the CoCo 1/2 and CoCo 3 memory systems are different machines
wearing the same 64K clothes.

---

## 5.5 The CoCo 1/2 path: `SAM::map` and the strobe registers

The CoCo 1/2 decoder provides a compact contrast with the GIME's ordered
stages. The GIME also retains SAM-compatible controls, so the older path
establishes the behavior that its compatibility layer reproduces.

The MC6883 SAM resolves an address in one function. Track where it returns
early and what each `SAMTarget` variant carries
([`crates/coco-core/src/sam.rs:140-176`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/sam.rs#L140-L176)):

```rust
    pub fn map(&self, addr: u16) -> SAMTarget {
        // The vector mirror and the $FF00+ fixed page win regardless of TY —
        // "the mirror region stays ROM" even in all-RAM mode.
        if addr >= VECTOR_MIRROR_BASE {
            return SAMTarget::RomBas(BAS_MIRROR_OFFSET + (addr - VECTOR_MIRROR_BASE) as usize);
        }
        if addr >= STROBE_BASE {
            return SAMTarget::Io; // $FFC0-$FFDF: SAM control strobes.
        }
        if (OPEN_BUS_BASE..=OPEN_BUS_LAST).contains(&addr) {
            return SAMTarget::OpenBus; // $FF7F-$FFBF.
        }
        if addr >= IO_BASE {
            return SAMTarget::Io; // $FF00-$FF7E: PIA0/PIA1/cart SCS (+ extension).
        }
        if self.ty && self.is_64k() {
            // All-RAM mode extends the RAM decode through $FEFF.
            return SAMTarget::Ram(addr as usize);
        }
        match addr {
            0x0000..=0x7FFF => {
                // P1 only matters when TY=0 (guaranteed by this branch) and
                // 64K: it ORs $8000 into the RAM address for CPU accesses in
                // this range.
                let ram_addr = if self.p1 && self.is_64k() {
                    addr | 0x8000
                } else {
                    addr
                };
                SAMTarget::Ram(ram_addr as usize)
            }
            EXT_ROM_BASE..=EXT_ROM_LAST => SAMTarget::RomExt((addr - EXT_ROM_BASE) as usize),
            BAS_ROM_BASE..=BAS_ROM_LAST => SAMTarget::RomBas((addr - BAS_ROM_BASE) as usize),
            CART_ROM_BASE..=CART_ROM_LAST => SAMTarget::Cart((addr - CART_ROM_BASE) as usize),
            _ => unreachable!("address {addr:#06x} not covered by the SAM decode"),
        }
    }
```

The early returns come first, and they encode the same principle §5.2
spent so long on: some things win regardless. The vector mirror at
`$FFE0` and above returns ROM before anything else is considered, which
is this machine's version of the hardwired-vector rule — different
constant, different implementation, identical purpose. Then the strobe
range, then the open-bus hole where a CoCo 3 would have its GIME
registers and this machine has nothing at all, then the rest of the I/O
page. Only after all four of those does the map consult a single bit of
chip state, `self.ty`, and only then does the `match` on address ranges
run.

The `match` itself is the whole ROM decode, and the striking thing about
it is that it is made of constants. `EXT_ROM_BASE..=EXT_ROM_LAST` and
`BAS_ROM_BASE..=BAS_ROM_LAST` are fixed at compile time; no register in
the machine can move them. Extended Color BASIC is at `$8000`, Color
BASIC is at `$A000`, the cartridge is at `$C000`, and that is simply
where those chips are. The only thing `TY` can do is take the whole
arrangement away at once, replacing it with RAM. Section 5.6 is entirely
about the CoCo 3 giving up that fixedness in exchange for two
configuration bits — and, as the price of the trade, needing a whole
extra decode tier to express the result.

One function, one enum result, no separate tiers to re-check on each
access. [`bus/sam_path.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sam_path.rs)'s `sam_read` and `sam_write` do nothing but
`match` on what `map` handed back and act on it. The structure is simpler
than the CoCo 3 path because the hardware genuinely is simpler: no MMU,
no independently configurable ROM-map stage, just a handful of latched
bits collapsing straight to one target per address.

The write side is where the difference from the CoCo 3 shows up most
sharply, and it is short enough to read whole
([`bus/sam_path.rs:46-63`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sam_path.rs#L46-L63)):

```rust
    pub(super) fn sam_write(&mut self, addr: u16, val: u8) {
        match self.sam.map(addr) {
            SAMTarget::Ram(phys) => {
                if let Some(i) = self.sam_ram_index(phys) {
                    self.ram[i] = val;
                }
            }
            // ROM/cart/open-bus targets: while TY=0 writes to $8000-$FEFF do
            // not write through to the RAM underneath (MAME gates
            // write-through on TY) — there's no RAM there at all in our
            // model, so these are simply dropped.
            SAMTarget::RomExt(_)
            | SAMTarget::RomBas(_)
            | SAMTarget::Cart(_)
            | SAMTarget::OpenBus => {}
            SAMTarget::Io => self.sam_io_write(addr, val),
        }
    }
```

Compare that to the CoCo 3's `write` from §5.2, where a store into the
ROM window falls through to the RAM mapped physically beneath it. Here,
four of the six targets are handled by a single empty block: the write
is dropped entirely, with nothing underneath to catch it. The comment
gives both the hardware reason and the modeling reason, which are
usefully distinct. MAME gates ROM-window write-through on the `TY` bit,
so on real hardware in ROM mode there is no write-through either; and in
this emulator's model there is not even a RAM byte at that physical
address to write to, because a CoCo 1/2's RAM is indexed essentially by
CPU address — plus the single `P1` bank bit — rather than through a
physical space that the CPU views through a shifting window. The same
programmer action, a `STA` into the ROM window, silently does nothing on
one machine and quietly modifies hidden RAM on the other.

> **Rust corner: an `enum` as a decode result.** `SAMTarget` — `Ram`,
> `RomExt`, `RomBas`, `Cart`, `Io`, `OpenBus` — is a textbook use of an
> algebraic data type as a *typed, exhaustive* answer to "what is this
> address." Compare this to how you'd likely do it in C: an integer tag
> plus an offset, with the compiler powerless to stop you reading the
> offset when the tag says `Io`. Here, `SAMTarget::Ram(usize)` carries
> its payload *only* in the variant where a payload makes sense, and
> every `match` on a `SAMTarget` that omits a variant is a compile
> error, not a runtime surprise the day someone adds `OpenBus`. This
> pattern — "decode to an enum, then match exhaustively" — recurs
> throughout this codebase; get comfortable reading it now.

### The strobe registers: a first "weird hardware" interface

Here's the detail that startles people who've only ever programmed
against friendly memory-mapped registers: `$FFC0–$FFDF` isn't sixteen
independent bit-fields you read and write like normal bytes. It's
sixteen *pairs* of write-only addresses. **The data byte you write is
completely ignored — only the address matters**, and within each pair,
the *even* address clears one specific bit and the *odd* address sets it:

```rust
/// Apply a SAM control-strobe write ($FFC0–$FFDF): the even address in
/// each pair clears the bit, the odd one sets it, and the data written is
/// irrelevant.
pub fn write_strobe(&mut self, addr: u16) {
    if !(STROBE_BASE..=STROBE_LAST).contains(&addr) { return; }
    let index = (addr - STROBE_BASE) / 2;
    let set = (addr - STROBE_BASE) & 1 != 0;
    match index {
        0..=2 => set_bit(&mut self.v, index as u8, set),
        3..=9 => set_bit(&mut self.f, (index - 3) as u8, set),
        10 => self.p1 = set,
        11 => self.r0 = set,
        12 => self.r1 = set,
        13 => self.m0 = set,
        14 => self.m1 = set,
        15 => self.ty = set,
        _ => unreachable!("SAM strobe index {index} out of range"),
    }
}
```
*([`crates/coco-core/src/sam.rs:119-136`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/sam.rs#L119-L136))*

Sixteen bits, sixteen pairs, 32 addresses: V0–V2 (`$FFC0–$FFC5`), F0–F6
(`$FFC6–$FFD3`), P1 (`$FFD4`/`$FFD5`), R0 (`$FFD6`/`$FFD7`), R1
(`$FFD8`/`$FFD9`), M0 (`$FFDA`/`$FFDB`), M1 (`$FFDC`/`$FFDD`), TY
(`$FFDE`/`$FFDF`). `TY` is the important one for this chapter: clear
means "system ROM mapped at `$8000+`" (the everyday state), set means
**all-RAM** — the ROM is switched out and whatever RAM sits underneath
it becomes visible and writable, all the way through `$FEFF`. Real
Color BASIC actually *uses* this: it copies and self-patches a working
image of itself into RAM and flips TY to run from there, which is why
"the ROM you're looking at" during a CoCo 1/2 session is sometimes a RAM
copy the ROM itself installed.

Why would hardware designers build a register interface this
inconvenient? Because a strobe *is* the primitive a 6809's bus cycle
naturally produces — a falling chip-select edge at a specific address —
and turning "set bit N" and "clear bit N" into two neighboring addresses
means the SAM needs no data-bus latch at all for these bits, just an
address decoder wired straight to sixteen flip-flops. It's cheaper
silicon at the cost of software ergonomics, and it is not unique to the
SAM: you'll meet the same even/odd convention again in the GIME's own
compatibility overlay.

> **Rust corner: deriving two facts from one number.** `(addr -
> STROBE_BASE) / 2` and `(addr - STROBE_BASE) & 1` pull the "which bit"
> and "set or clear" facts out of a single offset with two integer
> operations instead of a 32-entry lookup table or a match with 32 arms.
> This is a common trick when hardware imposes a systematic
> address-to-meaning mapping: notice the *structure* (even/odd pairs,
> sequential index) and compute rather than enumerate. It only works
> because the mapping really is that regular — don't reach for this if
> the pairing has exceptions, which is exactly why `SAM::map`'s vector
> mirror and open-bus carve-outs above are handled as explicit early
> returns instead of folded into the arithmetic.

The CoCo 3's GIME keeps an independent copy of this exact even/odd idea
for backward compatibility — you'll meet [`gime/sam_compat.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/sam_compat.rs)'s version
in §5.7, right after §5.6 covers the ROM window's own banking tricks,
which the SAM (as you just saw) doesn't have at all.

---

## 5.6 The GIME's ROM window: MC1:MC0 and what a cartridge sees

You just read `SAM::map`'s ROM decode: two hard-coded 8K windows,
`EXT_ROM_BASE..=EXT_ROM_LAST` and `BAS_ROM_BASE..=BAS_ROM_LAST`, that
never move. The CoCo 3's `$8000–$FDFF` window is the same 32K of address
space doing the same job, but the GIME adds one thing the SAM never had:
two configuration bits, `MC1` and `MC0` in `INIT0` (`$FF90`), that decide
*which chip* answers each half of that window — the internal ROM you've
already seen (`coco3.rom`), or an external cartridge's ROM arriving over
the `CTS*` pin. This is the tier-3 decode from §5.2 opened up.

### The four states

Two bits give four combinations, and the mapping from combination to
behavior is not a tidy encoding of two independent choices — it is a
table Tandy chose, with one redundant entry. The function that implements
it is six lines long, but its doc comment is the more valuable half,
because it names the source (the ROM-map table in SEB Unravelled II) and
then adds the one fact the table does not cover: what the cold-start code
actually writes, and why that explains the behavior of every diskless
CoCo 3 ever switched on.

```rust
/// True when `addr` in the ROM window maps to the *external* (cartridge)
/// ROM, per INIT0 MC1:MC0 (SEB Unravelled II ROM-map table):
/// `00`/`01` = 16K internal + 16K external at `$C000`; `10` = 32K
/// internal; `11` = 32K external (the CPU vectors stay internal — the bus
/// handles those separately). The cold-start writes INIT0 with MC=`10`
/// before its `JMP $C000`, which is why a diskless boot runs internal ROM.
pub fn rom_is_external(&self, addr: u16) -> bool {
    match self.init0 & (init0::MC1 | init0::MC0) {
        0b10 => false,
        0b11 => true,
        _ => addr >= EXTERNAL_ROM_BASE,
    }
}
```
*([`gime.rs:288-300`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L288-L300); `EXTERNAL_ROM_BASE = 0xC000`)*

| `MC1` | `MC0` | Mapping | What the emulator does |
|:-:|:-:|---|---|
| 0 | 0 | 16K internal (`$8000–$BFFF`) + 16K external (`$C000–$FDFF`) | `_` arm: `addr >= 0xC000` |
| 0 | 1 | Same as `00` — `MC0` is a documented don't-care while `MC1` is clear | `_` arm, identical code path to `00` |
| 1 | 0 | 32K internal — the whole window reads `coco3.rom` | `0b10 => false` |
| 1 | 1 | 32K external — the whole window (except the hardwired vectors) reads the cartridge | `0b11 => true` |

Look closely at the `match`: `0b00` and `0b01` both fall through to the
same wildcard arm. That isn't a gap in the emulator's coverage — it's a
faithful rendering of the documented hardware table (`docs/cartridges.md`
marks `MC0` as `x`, "don't care," whenever `MC1` is `0`). A real MC0
bit-flip with MC1 still clear genuinely does nothing observable, on real
silicon and in this code alike. Exercise 5.7 asks you to prove that with
a test of your own.

The power-on/cold-start state matters enough to name: `INIT0` resets to
`$00` (which is `MC=00`, 16K+16K split — an *empty* cartridge slot until
BASIC's own cold-start code runs), and that cold-start code immediately
writes `MC1` alone (`MC=10`, 32K internal) before jumping into the upper
half of ROM — you'll trace exactly this in [`tests/boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/boot.rs)'s
`cold_start_configures_rom_and_jumps_into_upper_half`, and §5.13 walks
the reset sequence that leads up to it. A diskless CoCo 3 runs entirely
out of `coco3.rom` because BASIC *chose* `MC=10`, not because that's
the only option — insert a Program Pak with autostart wired to `CART*`
and the FIRQ handler flips `INIT0` back to `MC=00` before jumping to
`$C000` (`docs/cartridges.md`'s account of the `$A0FC`/`L8C28` autostart
path), and the upper 16K becomes the cartridge's ROM instead.

### Two select lines, two jobs

A cartridge slot isn't a memory socket wired straight to the address
bus; it's a tap into the *whole* MC6809 bus, including two independent
chip-select outputs the GIME's address decoder drives (CoCo 3 Service
Manual, Table 3, via `docs/cartridges.md`):

- **`CTS*`** — asserted for the ROM window (`$8000/$C000` through
  `$FDFF`, per the table above). This is what `rom_read`/`rom_is_external`
  model.
- **`SCS*`** — asserted for the I/O window, `$FF40–$FF5F` (plus the
  unstrobed extension some carts decode anyway, `$FF60–$FF7E` — the
  cartridge row of §5.3's table).

Same physical connector, two different jobs, and the emulator keeps them
exactly as separate as the pins are: `rom_read` and `cart.read`
(dispatched from `io_read`'s `CART_BASE..=CART_LAST` arm) are two
different functions, because on real hardware they're two different
select lines that could, in principle, be driven by two different chips
on an elaborate cartridge.

### Why the reset vector can never belong to a cartridge

Notice `rom_is_external`'s doc comment: "the CPU vectors stay internal —
the bus handles those separately." Even under `MC=11` — the whole
`$8000–$FDFF` window handed to an external cartridge — tier 1 from §5.2
(`HARDWIRED_ROM_BASE`) still intercepts `$FFE0–$FFFF` before `rom_read`
is ever asked whether the address is external. `rom_read` itself encodes
the boundary explicitly:

```rust
fn rom_read(&mut self, addr: u16) -> u8 {
    if addr < HARDWIRED_ROM_BASE && self.gime.rom_is_external(addr) {
        return self.cart.rom_read(addr);
    }
    let off = (addr - ROM_WINDOW_BASE) as usize;
    self.rom.get(off).copied().unwrap_or(OPEN_BUS)
}
```
*([`bus.rs:252-258`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L252-L258))*

The practical consequence: **no cartridge, however aggressively it
banks itself in, can ever own the reset vector.** A power-up CoCo 3
always, unconditionally, starts executing internal ROM — there is no
hardware path for a cartridge to "boot first." Autostart carts only run
because BASIC's own cold-start code chooses to jump to `$C000` after an
interrupt, not because the machine handed them control directly at
reset.

### The 512 bytes `CTS*` can't reach

One further consequence is worth working through slowly, because it
explains a real, documented CoCo 3 quirk that looks at first like an
off-by-one bug in the emulator. The `CTS*` window stops at `$FDFF`, not
at `$FFFF`. A "16K" cartridge ROM logically spans `$C000–$FFFF`, which is
16,384 bytes, but the GIME only ever asserts `CTS*` across
`$C000–$FDFF`, which is 512 bytes short of that. Those last 512 bytes —
`$FE00` through `$FFFF` — are addresses the cartridge connector's select
line simply never fires for, regardless of how `MC1:MC0` are programmed.

Now split those 512 bytes in two, using rules already established. The top
32 bytes, `$FFE0–$FFFF`, are tier 1: hardwired internal ROM, immune to
everything, never available to a cartridge under any circumstances. The
`$FF00–$FFDF` span below it holds the I/O page proper and the
SAM-compatibility strobes, all of it tier 2. That leaves
`$FE00–$FEFF`, the page §5.2 flagged as MC3-gated, as the only part of
the missing 512 bytes whose fate is still open. With `MC3` set it is
pinned to constant RAM and the question is closed. With `MC3` clear, the
doc comment on `CONSTANT_RAM_BASE` says the page "follows the normal map
like the rest of the `$8000+` window."

Follow that through for a cartridge. "The normal map" here means the
`MC1:MC0` rule, and `is_rom_window` together with `rom_read` keep
applying it uninterrupted through `$FEFF` — the code has no special case
that stops at `$FDFF`, because the *emulator* is modeling the address
decode rather than the physical select line. So under an external-ROM
setting with `MC3` clear, `$FE00–$FEFF` reads as the tail of the
cartridge's own ROM image, even though `CTS*` never fires for it. This
is the only way a Program Pak's last 512 bytes of image data are
addressable at all, and real software depends on it: Sokoban keeps its
palette tables there and copies them out via `$FE88` reads, because
`$FDFF` really is the last byte `CTS*` reaches and the pak's linker put
data past it anyway.

### Two more `bus_map.rs` tests, walked

```rust
#[test]
fn mc_32k_internal_keeps_upper_half_internal() {
    let mut b = bus(MemorySize::K512);
    b.cart = Cart::custom(MarkerCart);
    // The cold-start value: MC=10 (32K internal) — what a diskless boot runs.
    b.write(0xFF90, init0::MC1);
    assert_eq!(b.read(0xC123), 0x23, "upper half reads internal ROM");
}

#[test]
fn mc_32k_external_maps_whole_window_except_vectors() {
    let mut b = bus(MemorySize::K512);
    b.cart = Cart::custom(MarkerCart);
    b.write(0xFF90, init0::MC1 | init0::MC0);
    assert_eq!(b.read(0x8123), 0xAA, "lower half external under MC=11");
    assert_eq!(b.read(0xFDFF), 0xAA, "top of window external");
    assert_eq!(b.read(0xFFFE), 0xFE, "vectors always internal ROM");
}
```
*([`tests/bus_map.rs:79-95`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bus_map.rs#L79-L95))*

The first proves `MC=10`'s "cold-start" behavior directly: even with a
`MarkerCart` installed and ready to answer `0xAA`, `$C123` reads
`marked_rom`'s own offset `0x23` — the cartridge is completely invisible
under this mode, exactly as the table above says. The second flips to
`MC=11` and shows the *entire* window, `$8123` through `$FDFF`, reading
the cartridge — `0xAA` at both ends — while `$FFFE` in the very same
test, on the very same bus, still reads `marked_rom`'s `0xFE`. One
`SystemBus`, one `read` call each time, two completely different answers
511 bytes apart (`$FDFF` vs `$FFFE`), because tier 1 doesn't care what
tier 3 decided.

---

## 5.7 The GIME MMU: 8K slots and two task sets

This section has two subjects, and it takes them in the order the address
map does. First the CoCo 3's imitation of the chip §5.5 just dismantled,
which occupies `$FFC0–$FFDF` and is mostly a matter of historical
obligation. Then the register range immediately below it, `$FFA0–$FFAF`,
which is the reason anyone bought a CoCo 3 in the first place. The
juxtaposition is not accidental: the GIME's designers put a
backward-compatibility shim and the machine's headline new feature
sixteen bytes apart, and both were reached by exactly the same kind of
store instruction.

The compatibility layer is a *second*, separate implementation of the
same even/odd strobe convention, deliberately not shared with `sam.rs`.
Its own module doc comment says so explicitly, in the same terms §5.4
used: the duplication exists so the CoCo 3 path stays completely
untouched by anything the CoCo 1/2 path does. It implements only the
strobes the CoCo 3 actually honors — V0–V2, F0–F6, R1, and TY — and
does not model P1, M0, M1, or the CoCo 1/2 `R0` pair, because on real
CoCo 3 hardware those addresses do nothing:

```rust
pub fn write_sam(&mut self, addr: u16) {
    match addr {
        SAM_VDG_BASE..=SAM_VDG_LAST => { /* V0-V2, same even/odd pattern */ }
        SAM_TY_CLEAR => self.all_ram = false,
        SAM_TY_SET => self.all_ram = true,
        SAM_R1_CLEAR => self.cpu_fast = false,
        SAM_R1_SET => self.cpu_fast = true,
        SAM_PAGE_BASE..=SAM_PAGE_LAST => { /* F0-F6, same pattern */ }
        _ => {}
    }
}
```
*([`crates/coco-core/src/gime/sam_compat.rs:44-70`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/sam_compat.rs#L44-L70), elided to the bits
that matter here)*

`POKE 65497,0` was the CoCo 3's standard double-speed poke, with
`POKE 65496,0` to put the machine back: decimal 65497 and 65496 are
`$FFD9`/`$FFD8` — exactly `SAM_R1_SET`/`SAM_R1_CLEAR` above. The
*older* pair from the CoCo 1/2 era, `POKE 65495,0`/`POKE 65494,0`
(`$FFD7`/`$FFD6`, the SAM's own `R0` strobe from §5.5's list), fares
differently on a CoCo 3: `write_sam` has **no arm at all** for those two
addresses — they fall straight through the `match`'s `_ => {}` catch-all
with no effect whatsoever, not even latched into an unused field (the
GIME struct has no `r0`-equivalent field to latch into). That matches
the real chip: SEB Unravelled II's own register figure lists only `R1` as
active on the GIME. The CoCo 1/2 `R0` pair being "address-dependent"
(nominally fast for ROM fetches, slow for RAM) was already a soft-edged
feature on the original SAM; the GIME's designers apparently decided one
clean, unconditional double-speed bit was enough and didn't bother
wiring the old pair through at all.

That is `$FFC0–$FFDF` handled: SAM compatibility, present and faithful,
but a sideshow. Sixteen bytes lower down sits the register range that
justifies the whole chip.

### The mental model

The problem the MMU exists to solve is stated most clearly as a
contradiction. The 6809 has sixteen address lines and can therefore name
65,536 bytes, full stop — there is no wider addressing mode, no segment
register, no bank byte anywhere in the instruction set. A CoCo 3 shipped
with 128K or 512K of RAM, and could be upgraded further. Those two facts
cannot both be accommodated without something standing between the CPU's
address pins and the memory chips, rewriting addresses in flight. That
something is the MMU, and every design decision in it follows from
keeping the rewrite cheap enough to happen on every single bus cycle.

The mechanism is a lookup table with eight entries. Split the CPU's 64K
logical space into eight 8K windows — `$0000–$1FFF`, `$2000–$3FFF`, and
so on up to `$E000–$FFFF` — and give each window one byte of storage
holding a *physical block number*, meaning which 8K chunk of installed
RAM that window is currently showing. An access to a logical address
looks up its window's block number, and the block number supplies the
high bits of the physical address while the address's own low bits pass
through untouched. Eight bytes of state, and the entire 64K view of
memory is described.

Sixteen registers occupy `$FFA0–$FFAF` rather than eight, because the
GIME provides two complete and independent sets of those eight bytes,
conventionally called task 0 and task 1. A single bit elsewhere — `TR` in
`INIT1` — decides which set is live. Software can therefore program task
1's entire mapping at leisure while task 0 is still running, then switch
the whole 64K view across in one register write, with no per-slot
reprogramming and no window of inconsistency in between. Section 5.8 is
the story of what real software did with that.

All of that structure shows up in the source as two constants and three
fields. The constants are the ones the rest of the code computes from,
rather than open-coded 2s and 8s scattered around
([`gime.rs:20-23`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L20-L23)):

```rust
/// Number of MMU task register sets ($FFA0–A7 and $FFA8–AF).
pub const TASK_COUNT: usize = 2;
/// Logical 8K slots per task (the 64K CPU space / 8K).
pub const SLOTS_PER_TASK: usize = 8;
```

The state itself is correspondingly small. Note that the register file is
a two-dimensional array indexed exactly the way the hardware is described
— by task, then by slot — rather than a flat sixteen-byte array with the
task folded into the index. The type says what the chip is
([`gime.rs:174-176`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L174-L176)):

```rust
pub mmu: [[u8; SLOTS_PER_TASK]; TASK_COUNT],
pub task: usize,
pub mmu_enabled: bool,
```

Three fields: the sixteen register bytes, which of the two sets is
currently selected, and whether the MMU is switched on at all. That last
one matters more than it looks, and §5.9 comes back to it — a CoCo 3
powers up with the MMU *disabled*, which is how a machine with a memory
management unit manages to boot software that has never heard of one.

Getting from an address in `$FFA0–$FFAF` to a slot in that array is
arithmetic rather than a match arm per register, for the same reason
§5.5's strobe decode was arithmetic: the mapping is perfectly regular, so
computing beats enumerating.

```rust
/// Decode an `$FFA0–$FFAF` MMU register address to `(task, slot)`.
fn mmu_index(addr: u16) -> (usize, usize) {
    let idx = (addr - MMU_BASE) as usize;
    (idx / gime::SLOTS_PER_TASK, idx % gime::SLOTS_PER_TASK)
}
```
*([`bus.rs:262-265`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L262-L265))*

### The translation itself

Everything above is setup. The function below is the MMU — the actual
address rewrite, performed on every access that reaches tier 4 of §5.2's
decode, which is to say on the overwhelming majority of all memory
accesses a CoCo 3 ever makes. It is nine lines long and contains no
loops, no lookups beyond one array index, and no branches beyond the
enable check. That economy is worth noticing rather than skimming past:
address translation on this machine is a shift, a mask, an array index
and an OR, and any model that needed more machinery than that would be
modeling something the GIME doesn't do:

```rust
/// Translate a CPU logical address to a physical RAM offset.
pub fn translate(&self, addr: u16) -> usize {
    if self.mmu_enabled {
        let slot = (addr as usize >> BLOCK_SHIFT) & (SLOTS_PER_TASK - 1);
        let block = self.mmu[self.task][slot] as usize;
        (block << BLOCK_SHIFT) | (addr as usize & (BLOCK_SIZE - 1))
    } else {
        DISABLED_MMU_BASE | (addr as usize)
    }
}
```
*([`gime.rs:241-249`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L241-L249), `BLOCK_SHIFT = 13`, i.e. `log2(8192)`)*

One formula is worth becoming fluent in, because the rest of this chapter
and a good deal of Chapter 8 assume it: `phys = (block << 13) | (addr &
0x1FFF)`. Take its three pieces in turn. `addr >> 13` picks which of the
eight 8K windows the address falls in; shifting a 16-bit value right by
thirteen leaves only three bits, so the `& (SLOTS_PER_TASK - 1)` mask
alongside it can never actually change the answer and is there purely as
defense in depth. `addr & 0x1FFF` is the byte offset *within* that 8K
window, and it passes into the physical address completely unchanged —
the MMU relocates blocks, never bytes within them. And `block << 13`
puts that offset at the right place in physical memory.

Work one example by hand, because the formula is much easier to trust
after you've done it once. Suppose the active task's slot 5 holds block
`$07`, and the CPU reads logical `$A123`. The slot index is `$A123 >> 13`,
which is 5, so slot 5's contents apply. The in-block offset is `$A123 &
$1FFF`, which is `$0123`. And the block base is `$07 << 13`, which is
`$E000`. The byte actually read is physical `$E123`. Notice that the
low thirteen bits of the answer are identical to the low thirteen bits of
the question; the MMU only ever changed the high end.

The `else` branch deserves as much attention as the `if`. When the MMU is
disabled, `translate` doesn't fall back to an identity map — it ORs the
whole 64K logical space into a fixed physical window starting at
`DISABLED_MMU_BASE`, which is `0x70000`. The GIME does not have an "MMU
off" mode in the sense of "no translation"; it has a hardwired
translation that puts the CPU's 64K at the top of a 512K space. Section
5.9 shows why that particular constant, and what it does to a machine
that doesn't have 512K to put it in.

A block number can be anything in `0..=255`, since it is a full byte.
That means the GIME can address a full 2 MB of physical space, `256 × 8K`,
even though Tandy never shipped a machine with more than 512K in it.
Nothing stops software from programming a block number that points past
the end of the RAM actually installed, so `SystemBus::phys` finishes the
job by folding the result back into range:

```rust
fn phys(&self, addr: u16) -> usize {
    if (CONSTANT_RAM_BASE..=CONSTANT_RAM_LAST).contains(&addr)
        && self.gime.init0 & gime::init0::MC3 != 0
    {
        return (CONSTANT_RAM_PHYS | (addr as usize & 0xFF)) % self.ram.len();
    }
    self.gime.translate(addr) % self.ram.len()
}
```
*([`bus.rs:220-228`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L220-L228))*

That `% self.ram.len()` is not a cosmetic bounds-check — it is the whole
mechanism by which a 128K or 512K machine survives block numbers that
would otherwise point off the end of physical RAM. Section 5.9 is a
full treatment of exactly what that means for a small machine.

> **Rust corner: `%` as intentional hardware fidelity, not a bug
> smell.** In application code, an unexplained `%` on an index usually
> means "someone forgot to bounds-check and is papering over it." Here
> it's the opposite: the modulo *is* the documented hardware behavior
> (DESIGN.md §3 calls it "the mask relocates it to the top 64K"), and
> [`tests/bus_map.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bus_map.rs)'s `small_machine_aliases_high_window_into_top_blocks`
> pins the exact aliasing a 128K machine produces. When you see
> deliberate wraparound arithmetic in emulator code, look for the
> comment or test that says *why* — it's very often "this is what the
> chip does," not an oversight. Contrast with `wrapping_add` from
> Chapter 1's `Bus::read_u16`: same instinct (embrace fixed-width
> wraparound instead of fighting it), different operator, same reason
> a `checked_*` or panicking method would be *wrong* here.

### MC3: the one address range the MMU can't touch

One wrinkle sits inside `phys` and gets checked before the MMU is
consulted at all. When bit `MC3` of `INIT0` is set, the page
`$FE00–$FEFF` is pinned to physical `$7FE00` no matter what the active
task's slot 7 register says. Two hundred and fifty-six bytes of the
address space are simply exempt from the banking mechanism the previous
few pages were spent building.

The reason is worth working out rather than accepting, because it is
the same reason tier 1 exists and it generalizes to every system with
both interrupts and banked memory. An interrupt can arrive at any
instruction boundary, including the boundary in the middle of a
context switch, when the address space is halfway between two
configurations. Whatever code the interrupt vectors into has to be
present in the CPU's view of memory at that instant, and the only way to
guarantee that across arbitrary MMU programming is to make one region
unbankable. BASIC keeps its interrupt trampolines in this page for
exactly that reason: a jump table at a fixed address that is always
there, whichever task is active and whatever else has been swapped
underneath it.

The read side has to make the matching promise, or the guarantee would be
half a guarantee. `is_rom_window` is where it lives
([`crates/coco-core/src/bus.rs:234-242`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L234-L242)):

```rust
    fn is_rom_window(&self, addr: u16) -> bool {
        if !self.gime.rom_enabled() {
            return false;
        }
        if (CONSTANT_RAM_BASE..=CONSTANT_RAM_LAST).contains(&addr) {
            return self.gime.init0 & gime::init0::MC3 == 0;
        }
        (ROM_WINDOW_BASE..CONSTANT_RAM_BASE).contains(&addr)
    }
```

Read the three returns in order, because between them they are the whole
of tier 3. The first says that if ROM is not currently mapped — that is,
if the SAM-compatible all-RAM bit is set, the same `TY` bit §5.5
introduced — then nothing in the window is ROM, full stop. The second
handles the constant page: inside `$FE00–$FEFF`, the address is ROM if
and only if `MC3` is clear, which is the exact complement of the rule in
`phys`. Set `MC3` and the page is unconditionally RAM on both the read
and the write side; clear it and the page follows the ordinary ROM/RAM
map like any other byte in the window, which is the case §5.6 traced
through to a cartridge's last 512 bytes. The third return is the
straightforward one: everything from `$8000` up to but not including
`$FE00` is ROM whenever ROM is mapped at all.

Note the deliberate asymmetry between the two functions. `phys` decides
where a byte *is*; `is_rom_window` decides whether the CPU is allowed to
see ROM there instead. They consult the same bit and must agree about
it, and the fact that agreement is spread across two functions in two
different tiers is exactly the sort of thing a test suite exists to pin
down. Section 5.14's second worked example walks both states through
`bus_map.rs`, and exercise 5.3 asks you to write the companion test for
the case that example doesn't cover.

---

## 5.8 Task switching: BASIC's own trick, and what OS-9 built on it

You now know the mechanism — two independent 8-register sets, one bit
in `INIT1` picking which is live. This section is about what that
mechanism is actually *for*, because "eight registers you could just as
well have had one of" is a strange thing to put in silicon unless real
software leans on it hard. Two pieces of real software did, at very
different scales.

### BASIC's own use: a second, private view of memory

Super Extended BASIC Unravelled II's memory-management chapter spells
out the theory behind the two register sets in almost the same words
this chapter has been using: each set may be allocated to a different
"task," and switching between them is nothing more than flipping
`INIT1` bit 0 — but the manual is emphatic about what that switch does
*not* do for you. To paraphrase its warning: swapping task sets **does
nothing to preserve the CPU's registers**, and if an interrupt lands
mid-switch, or the currently active stack pointer or program counter
happens to live in a page that just got swapped out from under it, the
machine can crash outright. The register swap is instantaneous and
total; keeping the machine coherent across it is entirely the
programmer's job.

BASIC's own ROM uses exactly this mechanism, at a much smaller scale
than "run a different program": SEB Unravelled II's disassembly names
two small subroutines, `SELTASK0` and `SELTASK1`, that do nothing but
mask interrupts, write `INIT1`, and return — called from a couple dozen
places throughout the hi-res graphics routines (`HGET`/`HPUT`, the
secondary stack, screen paging). The pattern in the disassembly is
always the same shape: select task 1 just long enough to reach a block
of memory that task 0's mapping doesn't currently expose — a graphics
buffer, a second stack — do the access, then select task 0 again.
BASIC never uses this to run two "processes"; it uses the second task
set as a *private trapdoor* to memory it doesn't want to permanently
bank into its main 64K view, entering and leaving it in a handful of
instructions with interrupts masked the whole time — precisely the
discipline the manual's warning demands.

### Operating-system use

The same primitive can support an operating system that prepares an inactive
map before switching tasks. The GIME changes address translation; software
must still save CPU registers, arrange stable kernel mappings, and control the
point at which the task-select bit changes. This chapter does not rely on a
specific OS-9 task-register convention because that convention has not been
verified here against a primary source.

### What the codebase actually tests

`coco-core` doesn't run OS-9 — it implements the hardware primitive OS-9
and BASIC both rode, and that primitive is exactly what
[`tests/bus_map.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bus_map.rs) pins down, one register write at a time:

```rust
#[test]
fn enabled_mmu_uses_task_block() {
    let mut b = bus(MemorySize::K512);
    b.write(0xFFA0, 0x05); // task0 slot0 -> physical block 5
    b.write(INIT0_REG, init0::MMUEN); // enable MMU
    b.write(0x0000, 0x99);
    assert_eq!(b.ram[5 * BLOCK_SIZE], 0x99);
}

#[test]
fn init1_selects_second_task_set() {
    let mut b = bus(MemorySize::K512);
    b.write(0xFFA8, 0x07); // task1 slot0 -> block 7
    b.write(INIT0_REG, init0::MMUEN);
    b.write(INIT1_REG, init1::TR); // select task 1
    assert_eq!(b.gime.task, 1);
    b.write(0x0000, 0x77);
    assert_eq!(b.ram[7 * BLOCK_SIZE], 0x77);
}
```
*([`tests/bus_map.rs:230-248`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bus_map.rs#L230-L248), walked in full in §5.14)*

The second test is the one that matters here: it leaves task 0's slot 0
completely unprogrammed (still `0` from `GIME::default()`), programs
*only* task 1's slot 0, flips the task-select bit, and shows the same
logical address `$0000` resolving through an entirely different
register — proof that the two sets really are independent storage, not
two names for the same eight bytes. Exercise 5.9 asks you to go one
step further and prove **isolation**: that writing through task 1 while
task 0 is inactive genuinely cannot disturb whatever task 0's own
mapping is quietly holding.

---

## 5.9 Sizing the MMU: 128K, 512K, and the high-block mirror

DESIGN.md §3 is candid about a gap in its own specification, and it's
worth quoting the hedge directly before looking at what the code
actually does:

> ¹ **Smaller machines map RAM into the *high* blocks**, not `0..N`. A
> 128K machine doesn't use banks `0x00–0x0F`; its RAM lives at the top
> of the block space… The exact 128K valid range is **not yet pinned**
> here — verify against the Super Extended BASIC Unravelled docs and a
> reference emulator before coding it. Do not assume a low-bit mask.

That's DESIGN.md telling a future implementer "don't guess here." So:
what did the implementer actually write? Go back to `phys` from §5.7 —
there is no 128K-specific branch anywhere in it, no lookup table keyed
by `MemorySize`, no explicit "valid bank range" check at all. The entire
behavior for every RAM size, small or large, is the single generic line
you already read:

```rust
self.gime.translate(addr) % self.ram.len()
```

No special case *is* the design: whatever `translate()` computes — a
number that can be as large as `(255 << 13) | 0x1FFF`, nearly 2 MB — is
simply taken modulo however many bytes are actually installed. The
question this section answers is whether that generic rule happens to
reproduce the specific "high blocks" behavior DESIGN.md flagged as
unverified, or whether it's a different (and possibly wrong) aliasing
scheme wearing the same clothes.

### Working the arithmetic

A 128K machine has `ram.len() == 0x20000` (131072 bytes) — exactly 16
physical 8K blocks. Compute `phys` for a few MMU block numbers by hand:

| Block | `block << 13` | `% 0x20000` |
|---|---|---|
| `$00` | `$00000` | `$00000` |
| `$01` | `$02000` | `$02000` |
| `$0F` | `$1E000` | `$1E000` |
| `$30` | `$60000` | `$00000` |
| `$31` | `$62000` | `$02000` |
| `$3F` | `$7E000` | `$1E000` |

Block `$30` and block `$00` land on the *identical* physical offset.
So do `$31`/`$01`, and any two block numbers that differ by a multiple of
sixteen — because `0x20000
/ 0x2000 = 16` exactly, `% self.ram.len()` on a 128K machine is
arithmetically identical to `block % 16`. This is precisely the
behavior an independently published reference (Chris Lomont's CoCo
hardware notes) documents from the real chip: on a 128K machine, MMU
pages `$00`–`$2F` are copies of the sixteen pages `$30`–`$3F` — there is no 512K
worth of distinct physical storage behind the low block numbers, so the
GIME's own address decoder (or, faithfully, this codebase's plain
modulo) simply wraps. Nobody wrote a "128K special case" into
`coco-core` — the generic sizing rule already reproduces it, which is
either a happy accident or evidence the generic rule is the *right*
level of fidelity. Be appropriately careful here: this is a consistency
check against a secondary source, not a MAME trace-diff — DESIGN.md's
"not yet pinned" hedge should stay in force until someone does that
harder verification. Exercise 5.10 asks you to extend the same table to
a 512K machine.

The two-way aliasing has been verified directly rather than left to the
arithmetic on paper alone. A temporary test enabled the MMU on a 128K
machine, programmed one slot to block `$00` and a second slot to block
`$30`, wrote a marker through the first slot, and read it back through
the second, and vice versa. It passed against the real code before being
deleted — the two block numbers really do address the same bytes, both
ways, exactly as the table predicts.

`DISABLED_MMU_BASE = 0x7_0000` (§5.7's fixed "MMU off" map) is itself
block `$38` in this scheme (`0x70000 / 0x2000 = 0x38`) — which lines up
with the same reference's documented power-on convention for a diskless
boot: BASIC's memory occupies blocks `$38`–`$3F`, one block per 8K
window of the CPU's disabled-MMU 64K. Whether BASIC's cold-start code
later reprograms the MMU's own task-0 registers to that identical
`$38`–`$3F` sequence when it turns `MMUEN` on — so nothing visibly
changes for a diskless boot at the moment the MMU switches from "off" to
"on" — is a genuinely good question to trace yourself with a debugger;
this chapter's own reset trace (§5.13) only follows the first five
instructions, well before `MMUEN` is ever set.

### The documented ceiling, for the larger machines

For 512K and above, DESIGN.md §3 gives the valid range directly, and
here the mask is a hard ceiling rather than a periodic wrap, because
these are real shipped or owner-confirmed configurations, not an
aliasing artifact of a smaller board:

| RAM    | 8K blocks | Valid bank range | Block-number bits | Notes                          |
|--------|-----------|------------------|--------------------|--------------------------------|
| 128K   | 16        | high blocks ¹    | (see ¹)           | RAM sits at the *top* of space |
| 512K   | 64        | `0x00–0x3F`      | 6                 | Tandy's shipped maximum        |
| 1024K  | 128       | `0x00–0x7F`      | 7                 | confirmed real config          |
| 2048K  | 256       | `0x00–0xFF`      | 8 (full register) | confirmed real config          |

`MemorySize::blocks()` ([`config.rs:155-157`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/config.rs#L155-L157)) reports exactly these block
counts (`bytes() / BLOCK_SIZE`), but — same observation as above —
nothing in `SystemBus`/`GIME` ever consults `blocks()` to reject an
out-of-range MMU write; the emulator lets you write any of the 256
possible block numbers into any MMU register on any machine, and lets
`phys`'s modulo sort out what that means physically. `config.rs`'s
`MachineConfig::validate` only ever rejects a RAM *size* the real
hardware never shipped — it has nothing to say about what a program
does with the MMU registers once a valid size is chosen.

### Two more `bus_map.rs` tests, walked

```rust
#[test]
fn disabled_mmu_maps_to_high_window() {
    let mut b = bus(MemorySize::K512);
    assert!(!b.gime.mmu_enabled);
    // Logical $0000 -> physical $70000 in the disabled-MMU window.
    b.write(0x0000, 0xAB);
    assert_eq!(b.ram[DISABLED_MMU_BASE], 0xAB);
    assert_eq!(b.read(0x0000), 0xAB);
}

#[test]
fn small_machine_aliases_high_window_into_top_blocks() {
    // 128K has no physical $70000; the mask relocates it to the top 64K ($10000).
    let mut b = bus(MemorySize::K128);
    b.write(0x0000, 0xCD);
    assert_eq!(b.ram[DISABLED_MMU_BASE % b.ram.len()], 0xCD);
}
```
*([`tests/bus_map.rs:212-228`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bus_map.rs#L212-L228))*

The first establishes the baseline on a 512K machine, where `$70000` is
a perfectly ordinary in-range address and `DISABLED_MMU_BASE` needs no
folding at all. The second is the one this section has been building
toward: on a 128K machine, that exact same logical address `$0000`
lands at `$10000` instead — `0x70000 % 0x20000`, worked by hand two
paragraphs up — proving in one assertion that the "MMU disabled" fixed
map isn't a separate code path with its own small-machine handling; it
runs through the identical `% self.ram.len()` line every translated
address does, and inherits the exact same high-block relocation this
whole section has been deriving from first principles.

---

## 5.10 The write-8/read-6 asymmetry

DESIGN.md §3 documents a real hardware quirk: writing an MMU register
stores a full 8-bit block number (letting software address the full
2 MB range), but **reading it back only reliably returns the low 6
bits** — the top two bits are bus bleedover on most real GIMEs, not the
value you wrote. `io_write`/`io_read` implement exactly this split:

```rust
MMU_BASE..=MMU_LAST => {
    let (task, slot) = mmu_index(addr);
    self.gime.mmu[task][slot] = val; // full 8 bits stored on write
}
```
```rust
MMU_BASE..=MMU_LAST => {
    let (task, slot) = mmu_index(addr);
    self.gime.mmu[task][slot] & gime::MMU_READ_MASK
}
```
*([`bus/io.rs:146-149`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L146-L149) and [`:84-87`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L84-L87); `MMU_READ_MASK = 0x3F`,
[`gime.rs:40`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L40))*

The `gime.rs` module header is stale:

```rust
//! STATUS: MMU translate, SAM compatibility strobes, and the video registers
//! ($FF98–$FF9F) are modelled; native scanout lives in `gime_video`. The timer,
//! GIME-sourced interrupts, and the write-8/read-6 register asymmetry are TODO.
```
*([`gime.rs:1-5`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L1-L5), emphasis on the last clause)*

That comment says the asymmetry is *still TODO* — but `MMU_READ_MASK`
exists, is applied on every MMU register read, and [`tests/bus_map.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bus_map.rs)
has a passing test, `mmu_register_write_8_read_low_6`, proving the
low-6-bits behavior works today:

```rust
#[test]
fn mmu_register_write_8_read_low_6() {
    let mut b = bus(MemorySize::K512);
    b.write(0xFFA3, 0xFF); // full 8 bits stored
    assert_eq!(b.gime.mmu[0][3], 0xFF);
    assert_eq!(b.read(0xFFA3), MMU_READ_MASK); // only low 6 read back
}
```
*([`tests/bus_map.rs:252-258`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bus_map.rs#L252-L258))*

The implementation and test show that the feature now exists. The header
should therefore be treated as outdated rather than as the current feature
list.

The implementation also makes a deterministic simplification. DESIGN.md's
fuller description is "return
`stored & 0x3F | (bus_garbage & 0xC0)`" — i.e., the top two bits on real
silicon aren't just *zero*, they're whatever noise happened to be on the
bus, which varies by individual chip and isn't deterministically
reproducible. The code here does the simpler, deterministic thing —
`& MMU_READ_MASK` alone, which leaves the top two bits at a clean `0`
rather than modeling any bleedover — and doesn't yet implement
DESIGN.md's mentioned "fixed-readback machine" configuration some real
memory upgrades provide. For an emulator whose job is running real
software reproducibly and passing a deterministic test suite, "always
read back 0 in the top two bits" is a defensible simplification of
"unpredictable analog noise" — software that depended on the *specific*
garbage value would be relying on undefined behavior on real hardware
too. But it is a simplification, and now you know exactly where it is
if you ever need to model a specific real machine's readback more
faithfully.

---

## 5.11 Open bus: where the `$FF` — and the `$00` — come from

Several decode paths return an open-bus value: unclaimed I/O, an empty
cartridge slot, a truncated ROM image, or an address beyond installed RAM.
The chosen value depends on the path.

### What "open bus" means

*Open bus* names the situation where no chip is driving the data bus for
the address the CPU just put on the address lines. Every other read in
this chapter has an answer because some device answers it; this is the
case where nothing does. The CPU has no way to know that. It asserts an
address, waits its documented number of cycles, and latches whatever
voltage the eight data lines happen to be sitting at.

On a real 6809 system that voltage is not nothing. It is whatever charge
remains on the bus lines, pulled toward a resting state by the bus's own
passive electrical characteristics, and different bus segments on the
CoCo rest at different levels depending on what is wired to them. An
emulator cannot model that analog behavior honestly and should not
pretend to try. What it can do instead is pick a fixed, documented
stand-in value for each region that can go unanswered, chosen to match
what real hardware returns — or what MAME returns, where the local
reference documents are silent. The interesting consequence, and the
reason this section exists, is that those stand-in values are not all the
same.

### Every open-bus constant in the codebase

| Constant | Value | Where it fires |
|---|---|---|
| `bus::regs::OPEN_BUS` | `0xFF` | The I/O page's final catch-all (`io_read`'s `_ => OPEN_BUS`); `VHD_SELECT` unconditionally; a ROM image shorter than the window it's mapped into (`rom.get(off).copied().unwrap_or(OPEN_BUS)` in `rom_read`, §5.6). |
| `sam::SAMTarget::OpenBus` region | `0xFF` | CoCo 1/2 only: `$FF7F–$FFBF`, the range that would be GIME registers on a CoCo 3 but simply doesn't exist without one ([`tests/sam.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/sam.rs)'s `ff7f_to_ffbf_is_open_bus_on_coco1_2`). |
| plain-SAM small-RAM reads | `0xFF` | `sam_path.rs`'s `sam_ram_index` returns `None` for an address past the installed RAM size on a 4K/16K/32K machine; the caller's `.unwrap_or(OPEN_BUS)` supplies `0xFF` ([`tests/sam.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/sam.rs)'s `small_ram_reads_open_bus_and_drops_writes_past_installed_size`). |
| `cart::IO_OPEN_BUS` | `0xFF` | The cartridge's `$FF40–$FF7E` (`SCS*`) window when no cartridge is installed — "floats high, like an unstrobed PIA input pin" ([`cart.rs:30-32`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cart.rs#L30-L32)). |
| `vhd`'s local `OPEN_BUS` | `0xFF` | VHD registers that don't answer while their drive is deselected. |
| `cart::ROM_OPEN_BUS` | **`0x00`** | The cartridge's `$C000–$FDFF` (`CTS*`) window when no cartridge is installed. |

Five of the six agree on `0xFF`. `ROM_OPEN_BUS` doesn't, and the
codebase is explicit that this isn't a guess:

```rust
/// Value read from the external ROM window when nothing drives the bus:
/// $00, matching real hardware / MAME coco3 (verified by MAME trace-diff,
/// 2026-07-02 — an empty slot's `LDD $C000` yields $0000, not $FFFF).
pub const ROM_OPEN_BUS: u8 = 0x00;

/// Value read from the cartridge I/O window ($FF40–$FF5F, SCS*) when nothing
/// drives the bus: floats high, like an unstrobed PIA input pin.
pub const IO_OPEN_BUS: u8 = 0xFF;
```
*([`cart.rs:25-32`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cart.rs#L25-L32))*

Notice what the doc comments do and don't claim. `IO_OPEN_BUS` gets a
one-line electrical rationale ("floats high, like an unstrobed PIA
input pin"). `ROM_OPEN_BUS` gets no rationale at all — just a citation
to a specific trace-diff, dated. That's the honest version of "we don't
know *why* the `CTS*` line rests low while the `SCS*` line rests high;
we know *that* it does, because we compared this emulator's behavior
against MAME's and against what a real machine's `LDD $C000` reports,
and matched it." Resist the temptation, reading or writing emulator
code, to invent a plausible-sounding electrical explanation you haven't
actually verified — "matches the trace" is a complete and honest reason
on its own, and it's a stronger claim than a guessed rationale would be.
Section 5.14's first worked example puts the same instinct to work in a
test rather than a comment: `mc_16k_split_routes_upper_half_to_cartridge`
asserts that `b.read(0xC123)` is `0x00` with an empty cartridge slot. A
reader who assumed open bus always reads back `0xFF` would take that
assertion for a typo. It is `ROM_OPEN_BUS` doing precisely what its
comment says, pinned by a test so that nobody can later "fix" it into
consistency.

### The debugger already assumes you'll get this wrong

One more place open bus shows up, foreshadowing Chapter 16: every
`Cartridge` method has a side-effect-free `peek` twin —
`rom_peek`/`peek`/`peek_control` — and every one of *those* defaults to
open bus rather than trying to guess a "probably harmless" real value:

```rust
/// Side-effect-free twin of [`Cartridge::rom_read`] for the debugger's
/// disassembly/memory views ([`crate::SystemBus::peek`]). Overridden by
/// cartridges whose ROM read is a pure array fetch (ROM paks, the FD-502
/// controller ROM); the default is open bus so a device that can't read
/// its ROM without side effects safely reports nothing rather than
/// perturbing state.
fn rom_peek(&self, _addr: u16) -> u8 {
    ROM_OPEN_BUS
}
```
*([`cart.rs:47-55`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cart.rs#L47-L55))*

This is the same `read`-vs-`peek` split Chapter 1 introduced for PIA
interrupt flags, applied to cartridges: a debugger memory view that
can't safely read a device's ROM without a side effect reports open bus
rather than risk corrupting the machine it's supposed to be inspecting
— "we don't know" is a legitimate answer for a decode function to give,
as long as every caller agrees on what "we don't know" looks like in
that specific region of the bus.

---

## 5.12 ROM composition and CRC validation

Every section so far has treated `self.rom` as a given: a `Box<[u8]>`
that exists, is the right length, and contains the right bytes. That is a
comfortable assumption inside a decode function and an unsafe one
everywhere else. ROM images arrive as files on somebody's disk, they
arrive under names that may or may not describe their contents, and on
the older machines they do not arrive as a single file at all. This
section is about the seam between "a file the user supplied" and "the
array `rom_read` indexes into," which turns out to be a place where two
generations of hardware once again disagree.

### Two very different ROM stories

The CoCo 3's story is the simple one. There is a single 32K image,
`coco3.rom`, loaded whole and mapped verbatim: `SystemBus::rom_read`
computes `off = addr - 0x8000` and indexes straight into it
([`bus.rs:252-258`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L252-L258)). One file, one array, one subtraction. That image
is Super Extended Color BASIC, and it is what came up every time a
CoCo 3 was switched on, occupying the whole `$8000–$FFFF` window whenever
`INIT0`'s `MC1:MC0` bits select 32K-internal. Since that is what the
cold-start code writes, it is also the reason `PEEK` above 32767 on a
diskless CoCo 3 always returned a ROM byte and never open cartridge bus.
Section 5.6 has the full four-state table for every other combination of
those two bits.

The CoCo 1/2 story has no single "the ROM" at all. Real machines shipped
several separate mask ROM chips — Extended Color BASIC answering
`$8000–$9FFF`, plain Color BASIC answering `$A000–$BFFF` — and a machine
with only Color BASIC installed, which many were, has literally nothing
responding in the Extended BASIC range. `SAM::map` reflects that chip
boundary directly, returning two distinct `SAMTarget` variants
(`RomExt` and `RomBas`, §5.5) rather than pretending there is one flat
image.

But `SystemBus::new` takes exactly one `Box<[u8]>`, and the plain-SAM
read path indexes into it with a fixed offset for the Color BASIC half.
So something has to compose a single array out of however many ROM files
are actually present, and decide what to put where a chip is missing.
[`tests/coco1_boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/coco1_boot.rs) does the minimal version of this, and the choice
of filler byte is the interesting part:

```rust
/// Compose a Color-BASIC-only flat image: `OPEN_BUS_FILLER` for the extbas
/// half, `bas12.rom` at `BAS_OFFSET`.
fn boot_machine() -> Option<(Machine, Vec<u8>)> {
    let Some(bas) = try_load("bas12.rom") else { /* ... */ };
    let mut image = vec![OPEN_BUS_FILLER; BAS_OFFSET]; // $FF filler, no Extended BASIC
    image.extend_from_slice(&bas);
    // ...
}
```
*([`tests/coco1_boot.rs:26,38-46`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/coco1_boot.rs); `BAS_OFFSET = 8*1024`,
`OPEN_BUS_FILLER = 0xFF`)*

`0xFF` is not an arbitrary padding byte. It is what an empty,
unconnected bus line reads as, which is §5.11's whole subject, and it is
the same value `bus.rs`'s own `OPEN_BUS` constant supplies for every
other unmapped range in this chapter's table. Filling the missing chip's
address range with open-bus bytes means the composed image behaves the
same way the real machine does: the addresses are there, they answer, and
what they answer is "nothing is here."

The payoff is that the emulator does not need to know whether Extended
BASIC is installed. Nothing branches on it, no configuration flag records
it, and `SAM::map` decodes the `RomExt` range identically either way. The
absence is represented as data rather than as a case. A test that boots
this composed image
(`ty0_reads_rom_at_extbas_and_bas_windows`-adjacent coverage, and
`coco1_boot.rs` proper) proves the machine boots into the plain "COLOR
BASIC" banner, not "EXTENDED COLOR BASIC". The reason is that
`$8000–$9FFF` genuinely reads back `$FF` bytes, which don't disassemble
into working BASIC startup code, so the ROM's own startup sequence
detects the absence and skips straight to the Color BASIC banner.
`coco-egui`'s `compose_coco12_rom` does the general version of this same
layout at runtime, picking whichever `extbas*.rom`/`bas*.rom` files are
present and filling the gap the same way when they aren't.

### Knowing what you actually loaded

Before any of this, you generally want to know *which* dump you've got
— homebrew patches and mislabelled files are common in the wild.
`rom_db.rs` keeps a manifest of every known-good CoCo ROM (copied from
MAME's own romset definitions) and validates by content, not filename:

```rust
pub enum Validation {
    Verified(&'static KnownROM),
    Mismatch { expected: &'static KnownROM, actual_crc32: u32, actual_size: usize },
    Unknown,
}

pub fn validate(file_name: &str, bytes: &[u8]) -> Validation {
    if let Some(known) = identify(bytes) {
        return Validation::Verified(known);
    }
    match KNOWN_ROMS.iter().find(|r| r.file == file_name) {
        Some(expected) => Validation::Mismatch {
            expected,
            actual_crc32: crc32(bytes),
            actual_size: bytes.len(),
        },
        None => Validation::Unknown,
    }
}
```
*([`rom_db.rs:38-91`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/rom_db.rs#L38-L91), elided)*

`identify` checks content (size + CRC32) against every known ROM
*regardless of the claimed file name* — so a `coco3.rom` that's
secretly `bas12.rom` renamed is still `Verified`, correctly identified
by content. Only when content doesn't match anything known does the
claimed file name matter at all, to decide between "this claims to be a
ROM we know, but it's been patched" (`Mismatch`, with the concrete CRC
so you can tell a hex editor's worth of difference from total garbage)
and "never heard of this file" (`Unknown`, e.g. a fan-made homebrew
image). The important design decision is in the module doc comment:
**validation is advisory, never a gate** — an `Unknown` or `Mismatch`
ROM still boots. Patched and homebrew ROMs are legitimate CoCo software,
not errors to reject.

---

## 5.13 Reset, traced end to end

Every decode rule in this chapter converges on one address at the very
first instant the machine exists, and it is worth following that single
address all the way through, because it exercises tier 1, the ROM
mapping, the endianness convention from Chapter 1, and the power-on register
state, all before a single instruction has executed. If any one of those
is wrong, the machine does not boot; if all of them are right, the CoCo 3
is already running real code.

Construction is where it starts. `Machine::new` builds a CPU and a bus,
applies whatever monitor setting the configuration asked for, and then —
the line that matters here — resets the CPU while the bus is already
alive and answering ([`machine.rs:167-176`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine.rs#L167-L176)):

```rust
pub fn new(config: MachineConfig, rom: Box<[u8]>) -> Self {
    let mut cpu = MC6809::new();
    let mut bus = crate::SystemBus::new(config.variant, config.memory, rom);
    if let Some(monitor) = config.monitor {
        bus.gime.monitor = monitor;
    }
    cpu.reset(&mut bus);
    Self { cpu, bus, /* ... */ }
}
```

That ordering is a requirement, not a convenience. `MC6809::reset` does
not invent a starting program counter; it *reads* one out of memory, so
the bus has to be fully constructed and the ROM already attached before
reset is allowed to run. Here is the whole of it, from Chapter 4's
territory, unchanged
([`crates/mc6809/src/lib.rs:177-183`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L177-L183)):

```rust
pub fn reset(&mut self, bus: &mut impl Bus) {
    self.dp = 0;
    self.cc |= cc::IRQ_MASK | cc::FIRQ_MASK;
    self.pc = bus.read_u16(VECTOR_RESET);   // VECTOR_RESET = 0xFFFE
    self.state = State::Running;
    self.nmi_armed = false;
}
```

Walk `bus.read_u16(0xFFFE)` through everything you now know:

1. `read_u16` (Chapter 1's default trait method) issues two big-endian
   `read` calls: `read(0xFFFE)` then `read(0xFFFF)`.
2. Each hits `SystemBus::read`. `variant == Coco3` (default config), so
   we're in the GIME path, not `sam_read`.
3. Both `0xFFFE` and `0xFFFF` are `>= HARDWIRED_ROM_BASE (0xFFE0)` —
   **tier 1 wins**, before the I/O-page check, before the ROM-window
   check, before the MMU is ever consulted. This is the entire reason
   tier 1 exists: at this exact moment, `INIT0` is still at its
   power-on value (`$00` — MMU disabled, no ROM-map bits programmed
   yet), so if the hardwired-vector carve-out didn't exist, `$FFFE`
   would fall through to the I/O-page tier and read back whatever
   `io_read`'s catch-all returns for an unclaimed address — `$FF`, not
   a ROM byte.
4. `rom_read(0xFFFE)`: since `0xFFFE` is *not* `< HARDWIRED_ROM_BASE`,
   the external-cartridge carve-out in `rom_read` is skipped entirely
   (that carve-out only applies below `$FFE0` — see the doc comment on
   `HARDWIRED_ROM_BASE`, which cites MAME's `coco3.cpp:53-58` and a
   real-hardware test by William Astle refuting an older, incorrect
   claim in SEB Unravelled II that this range aliases `$BFFx`). Straight
   to `rom[addr - 0x8000]`.
5. `coco3.rom` offset `0xFFFE - 0x8000 = 0x7FFE`, the last two bytes of
   the 32K image.

[`tests/boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/boot.rs)'s `reset_vector_points_into_rom` pins the real values:

```rust
const RESET_ENTRY: u16 = 0x8C1B;

#[test]
fn reset_vector_points_into_rom() {
    let mut m = boot_machine();
    assert_eq!(m.bus.read(0xFFFE), 0x8C);
    assert_eq!(m.bus.read(0xFFFF), 0x1B);
    assert_eq!(m.cpu.pc, RESET_ENTRY);
}
```
*([`tests/boot.rs:13,27-35`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/boot.rs))*

`coco3.rom[0x7FFE..=0x7FFF] = {0x8C, 0x1B}` — the vector *stored* at the
very end of the ROM image points to `$8C1B`, an address near the
*start* of the mapped ROM (offset `0x0C1B`). That's completely ordinary
for a 6809 reset vector: the vector table lives at the top of the
address space by architecture convention, but nothing says the code it
points to has to live near there too. `PC = 0x8C1B` after `reset()`
returns is your first, cleanest proof that decode order, ROM mapping,
and the hardwired-vector carve-out are all wired correctly before a
single instruction has executed.
`cold_start_configures_rom_and_jumps_into_upper_half` (same file) carries
the trace five instructions further: the cold-start code immediately does
`ORCC`, then `LDA #$0A / STA $FF90` — writing `INIT0` with `MC1` and
`MC3` set (32K internal ROM, constant vector page — the `MC=10` row of
§5.6's table) — then `CLR $FF91`, then jumps to `$C000`, now reading the
*upper* half of the same 32K image. That INIT0 write is what makes tier 3
(`is_rom_window`) start returning results that actually matter — before
it, the MMU-disabled RAM tier would have answered for everything below
`$FFE0`.

---

## 5.14 Four worked examples from `bus_map.rs`

[`tests/bus_map.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bus_map.rs) is, deliberately, the single best teaching artifact
for this chapter — every test builds a `SystemBus` directly against a
*synthetic* ROM, `marked_rom()`, whose every byte equals its own
low-address byte:

```rust
fn marked_rom() -> Box<[u8]> {
    (0..ROM_SIZE).map(|i| i as u8).collect::<Vec<_>>().into_boxed_slice()
}
```

That trick means any test can assert on the exact ROM offset a read
resolved to just by checking the returned byte — no need to track real
BASIC opcodes. Four examples here, each chosen to exercise a different
tier from §5.2 (§5.6 and §5.9 each walked another pair of tests, right
where those concepts were introduced — the ROM-window MC1:MC0 states and
the 128K/512K sizing tests, respectively).

### Example 1: the INIT0 MC bits choosing between internal and external ROM

```rust
#[test]
fn mc_16k_split_routes_upper_half_to_cartridge() {
    let mut b = bus(MemorySize::K512);
    // Power-on INIT0 = $00 -> MC=00: 16K internal + 16K external. The empty
    // slot answers open-bus $00 (MAME trace-diff verified), not internal ROM.
    assert_eq!(b.read(0x8123), 0x23, "lower half stays internal");
    assert_eq!(b.read(0xC123), 0x00, "upper half is the (empty) cartridge");

    b.cart = Cart::custom(MarkerCart);
    assert_eq!(b.read(0xC123), 0xAA, "upper half reads the cartridge ROM");
    assert_eq!(b.read(0x8123), 0x23, "lower half still internal");
}
```
*([`tests/bus_map.rs:65-76`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bus_map.rs#L65-L76))*

`MarkerCart` is a two-line fake cartridge that always answers `0xAA` on
its ROM window (`rom_read`) — a reminder that the `Cartridge` trait
(Chapter 13 territory) is the same kind of narrow seam as `Bus` itself, and
that swapping it in a test is exactly how you exercise the "what if
external ROM is present" branch of `GIME::rom_is_external` without ever
touching a real cartridge image. At power-on, `INIT0`'s `MC1:MC0` bits
are `00`, which `rom_is_external` ([`gime.rs:294-300`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L294-L300), §5.6's full table)
maps to "16K internal + 16K external": `$8000–$BFFF` still reads
`marked_rom` (you can see the low byte pass straight through, `0x8123 →
0x23`), but `$C000–$FDFF` routes to `self.cart.rom_read` instead —
reading `0x00` (`ROM_OPEN_BUS`, §5.11) until a cartridge is actually
installed, then `0xAA` once `MarkerCart` answers.

### Example 2: MC3 pinning `$FE00–$FEFF` to constant RAM

```rust
#[test]
fn constant_page_fe00_is_ram_when_mc3_set() {
    let mut b = bus(MemorySize::K512);
    b.write(0xFF90, init0::MC3);
    b.write(0xFE00, 0x5A);
    b.write(0xFEFF, 0xA5);
    assert_eq!(b.read(0xFE00), 0x5A);
    assert_eq!(b.read(0xFEFF), 0xA5);
    // The byte just below still reads ROM (writes fall through to shadow RAM).
    b.write(0xFF90, init0::MC3 | init0::MC1);
    b.write(0xFDFF, 0x11);
    assert_eq!(b.read(0xFDFF), marked_rom()[0xFDFF - 0x8000]);
}
```
*([`tests/bus_map.rs:150-166`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bus_map.rs#L150-L166))*

This is §5.7's MC3 rule made concrete: with `MC3` set, `$FE00` and
`$FEFF` round-trip a write/read exactly like plain RAM — `phys()`'s
special case routes them to `CONSTANT_RAM_PHYS = 0x7FE00` regardless of
any MMU state — while the byte one address lower, `$FDFF`, is *outside*
the pinned page and still obeys the ordinary ROM-window rule. The second
half of the test writes to it and confirms the write was silently
writes through the ROM overlay into shadow RAM. The subsequent read still
returns `marked_rom`'s value because `MC1` keeps the read path mapped to
internal ROM. Switching ROM out would expose the byte written underneath.
The adjacent `$FDFF` and `$FE00` addresses therefore have different read
routing even though both writes can reach RAM.

### Example 3: enabling the MMU and switching task sets

```rust
#[test]
fn enabled_mmu_uses_task_block() {
    let mut b = bus(MemorySize::K512);
    b.write(0xFFA0, 0x05); // task0 slot0 -> physical block 5
    b.write(INIT0_REG, init0::MMUEN); // enable MMU
    b.write(0x0000, 0x99);
    assert_eq!(b.ram[5 * BLOCK_SIZE], 0x99);
}

#[test]
fn init1_selects_second_task_set() {
    let mut b = bus(MemorySize::K512);
    b.write(0xFFA8, 0x07); // task1 slot0 -> block 7
    b.write(INIT0_REG, init0::MMUEN);
    b.write(INIT1_REG, init1::TR); // select task 1
    assert_eq!(b.gime.task, 1);
    b.write(0x0000, 0x77);
    assert_eq!(b.ram[7 * BLOCK_SIZE], 0x77);
}
```
*([`tests/bus_map.rs:230-248`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bus_map.rs#L230-L248))*

The first test programs *only* `$FFA0` (task 0, slot 0) and enables the
MMU; a write to logical `$0000` — slot 0 of the 64K window — lands at
physical `5 * 8192`, exactly `phys()`'s formula from §5.7 with `block =
5`, `addr & 0x1FFF = 0`. The second test proves the *task* half of "two
task sets" — §5.8's whole subject — programming task 1's slot 0 to block
7, leaving task 0's slot 0 completely unprogrammed, flipping `INIT1`'s
`TR` bit, and confirming both that `gime.task` actually became `1` and
that the *same* logical address `$0000` now resolves through the
newly active task's mapping instead. If you ever need to convince
yourself the "context switch by flipping one bit" claim from §5.7/§5.8
is real and not aspirational prose, this pair of tests is the proof.

### Example 4: the disabled-MMU map, and how a small machine survives it

```rust
#[test]
fn disabled_mmu_maps_to_high_window() {
    let mut b = bus(MemorySize::K512);
    assert!(!b.gime.mmu_enabled);
    b.write(0x0000, 0xAB);
    assert_eq!(b.ram[DISABLED_MMU_BASE], 0xAB);
    assert_eq!(b.read(0x0000), 0xAB);
}

#[test]
fn small_machine_aliases_high_window_into_top_blocks() {
    let mut b = bus(MemorySize::K128);
    b.write(0x0000, 0xCD);
    assert_eq!(b.ram[DISABLED_MMU_BASE % b.ram.len()], 0xCD);
}
```
*([`tests/bus_map.rs:212-228`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bus_map.rs#L212-L228); walked in full, with the hand-worked
arithmetic behind the second test, in §5.9)*

Two RAM sizes, one logical address, two different physical
destinations — `$70000` on a 512K machine, `$10000` on a 128K one —
both produced by the same unconditional `% self.ram.len()` in `phys()`.
If §5.9's table left any doubt that "no special-cased 128K logic"
really does reproduce the documented high-block behavior, this pair of
already-passing tests is where that claim gets checked by the compiler
on every single `cargo test` run, not just by hand.

---

## 5.15 Reading assignment

In this order:

1. **[`crates/coco-core/src/bus.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs), whole file** — you've now seen most
   of it in fragments; read it start to finish once so the four-tier
   decode order in `read`/`write` sits in your head as one continuous
   shape, not four separate quotes.
2. **[`crates/coco-core/src/bus/io.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs)** — the full I/O dispatch. Check
   every `match` arm off against the table in §5.3 as you go; find the
   one register this chapter didn't mention (there's at least one).
3. **[`crates/coco-core/src/bus/pins.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/pins.rs)** — short, and it recasts every
   PIA "read" you'll do from Chapter 10 onward: nothing is stored, it's all
   computed fresh from other devices' state at the moment of access.
4. **[`crates/coco-core/src/gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs), lines 1–70 and 230–300** — the
   register bit constants (skim; you'll be back for these in Chapter 8)
   and `translate`/`write_init0`/`write_init1`/`rom_is_external` in
   full.
5. **[`crates/coco-core/src/sam.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/sam.rs), whole file** — short enough to read
   end to end, and doing so makes explicit just how much simpler the
   CoCo 1/2 memory story is next to the GIME's.
6. **[`crates/coco-core/src/bus/sam_path.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sam_path.rs)** — the thin adapter that
   turns `SAM::map`'s `SAMTarget` into actual reads and writes; note
   how little code it takes once `SAM::map` has already done the real
   work.
7. **[`crates/coco-core/src/cart.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cart.rs#L1-L70), lines 1–70** — the `Cartridge`
   trait and its two open-bus constants (§5.11); a preview of Chapter 13
   that only takes a few minutes now.
8. **`docs/cartridges.md`** — a tracked, in-repo reference doc (not one
   of the gitignored copyrighted PDFs) covering the physical cartridge
   connector, the `CTS*`/`SCS*` split, and the autostart interrupt path
   §5.6 summarized; read it in full if the electrical side of §5.6
   interested you.

While reading, run the two test files that exercise everything above
with no real ROM required:

```
cargo test -p coco-core --test bus_map --test sam
```

([`tests/boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/boot.rs) and [`tests/coco1_boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/coco1_boot.rs) need real ROM dumps in
`./roms/` — run them too if you have that directory populated; if not,
`coco1_boot.rs` skips itself with a message rather than failing.)

**On §5.8's OS-9 aside:** the local reference PDFs (SEB Unravelled II,
the CoCo 3 Service Manual, Bob Russell's memory map) cover BASIC's own
use of the two task-register sets in detail but don't document OS-9's
multi-tasking use of the same mechanism. If you want to go further than
this chapter did, a forum thread collecting real OS-9 kernel-source
knowledge is a reasonable starting point: ["Ein paar Informationen zu
OS-9"](https://forum.classic-computing.de/forum/index.php?thread%2F26063-ein-paar-informationen-zu-os-9%2F=)
(German; the relevant passages describe the kernel/system task and
per-process DAT-image swap). Treat it, as this chapter did, as
secondhand software history rather than a verified hardware spec.

---

## 5.16 Exercises

**5.1 — Break the decode order, on purpose (sabotage).** In your own
checkout, swap the order of the two `if` blocks at the top of the CoCo 3
branch of `SystemBus::read` — check `io_enabled && addr >= IO_BASE`
*before* `addr >= HARDWIRED_ROM_BASE`, instead of after. Predict, before
running anything, which tests in `bus_map.rs` will fail and why (think
about which addresses are members of *both* ranges). Then run
`cargo test -p coco-core --test bus_map` and check yourself — you should
see four failures, all reading back `0xFF` (open bus) at addresses in
`$FFE0–$FFFF` where a ROM byte was expected. Revert the change and
confirm the suite is green again. (This is not a hypothetical: the four
failures above were observed by making exactly this edit and reverting
it, before the claim was written down here.)

**5.2 — MMU arithmetic (drill).** Using `phys = (block << 13) | (addr &
0x1FFF)`:
   (a) MMU task 0, slot 2 (i.e. logical `$4000–$5FFF`) is programmed to
   physical block `$3A`. What physical address does logical `$4321`
   resolve to?
   (b) The active task is task 1; slot 7 (logical `$E000–$FFFF`) is
   programmed to block `$12`. What physical address does logical
   `$E777` resolve to, and which of `mmu[0]`/`mmu[1]` did you have to
   consult to answer that?
   (c) On a 512K machine (64 physical blocks), what actually happens if
   slot 3 is programmed to block `200`? Compute the aliased physical
   address for logical `$6000` and name the `SystemBus` function
   responsible for that behavior.

**5.3 — Write the MC3 test the chapter didn't (build).** §5.14's second
worked example proved MC3 pins `$FE00–$FEFF` to constant RAM *while ROM
is mapped*. Write a new `bus_map.rs`-style test proving the companion
claim from `vector_page_is_mapped_ram_in_all_ram_mode_when_mc3_clear`-
adjacent coverage: with `MC3` clear and `gime.all_ram = true` (SAM
TY-equivalent), `$FE00–$FEFF` behaves as ordinary MMU-mapped RAM, not
as the constant page. Your test should fail against a deliberately
broken `is_rom_window`/`phys` (try hard-coding the constant-page branch
to always apply regardless of `MC3`) and pass against the real code —
prove that to yourself before moving on.

**5.4 — Map the I/O page from memory (recall).** Without looking back
at §5.3, write down what device or register lives at `$FF00`, `$FF20`,
`$FF41`, `$FF7F`, `$FF80`, `$FF90`, `$FFA0`, `$FFB0`, `$FFC0`, and
`$FFE0`. For each, name which tier of §5.2's decode order it belongs
to. Then check yourself against the table.

**5.5 — Read the test, predict the result (read + predict).** Before
running anything, predict what this test from [`tests/sam.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/sam.rs) asserts,
address by address, and *why* — in particular, explain in one sentence
why the assertion about `$BFFE` at the end is necessary for the test to
mean what it claims:

```rust
#[test]
fn vector_mirror_stays_rom_even_in_all_ram_mode() {
    let mut b = bus(MemorySize::K64);
    let rom_byte = b.read(0xBFFE);
    assert_eq!(b.read(0xFFFE), rom_byte, "mirrors $BFFE under TY=0");

    b.write(M1_SET, 0);
    b.write(TY_SET, 0);
    b.write(0xFFFE, 0x00);
    assert_eq!(b.read(0xFFFE), rom_byte, "vector mirror must stay the same ROM byte under TY=1");
    assert_ne!(b.read(0xBFFE), rom_byte, "sanity: $BFFE itself really did flip to RAM under TY=1");
}
```

Then run it and confirm.

**5.6 — Why must `$FFFE` bypass the MMU, even in all-RAM mode? (essay,
four sentences max).** Both the GIME path (`HARDWIRED_ROM_BASE`) and
the CoCo 1/2 SAM path (`VECTOR_MIRROR_BASE`) special-case the top of the
address space so the reset vector is unconditionally ROM, regardless of
TY/all-RAM state, MMU programming (CoCo 3 only — the SAM has no MMU),
or cartridge presence. Walk §5.13's reset trace and explain concretely
what would happen on power-on if this carve-out didn't exist and
`$FFFE` instead read through the normal MMU/RAM path with the machine's
actual power-on register state (MMU disabled, all RAM zeroed). Where
would `PC` end up, and what would the CPU try to execute next?

**5.7 — The don't-care bit, proven (build).** §5.6 claims `MC0` has no
observable effect whenever `MC1` is clear — `rom_is_external`'s `match`
sends both `0b00` and `0b01` to the same wildcard arm. Write a
`bus_map.rs`-style test that programs `INIT0` with `MC0` alone (`MC1`
clear) and asserts the ROM window behaves identically to power-on
(`INIT0 = $00`): internal ROM below `$C000`, cartridge above it. Then
write the opposite test for `MC1` set: confirm `MC0` *does* matter now —
`MC=10` keeps the whole window on internal ROM while `MC=11` hands it
to the cartridge, so the probe that read identically in the `MC1`-clear
case must read differently here (make sure your test is actually
distinguishing that pair of states before you trust it).

**5.8 — Two open-bus values, one sentence each (recall + sabotage).**
Name the two constants from §5.11 that answer an empty cartridge slot,
state which value each returns, and explain in one sentence each why
they're allowed to disagree — hint: they're gated by different physical
select lines. Then actually swap `ROM_OPEN_BUS`'s and `IO_OPEN_BUS`'s
values in `cart.rs` (`0x00` becomes `0xFF` and vice versa), predict
which `bus_map.rs`/`sam.rs` tests will fail, run
`cargo test -p coco-core --test bus_map --test sam`, and check your
prediction. Revert before moving on.

**5.9 — Prove task isolation (build).** §5.8 claims the two MMU task
register sets are genuinely independent storage — writing through one
cannot disturb the other. `enabled_mmu_uses_task_block` and
`init1_selects_second_task_set` come close but each only touches *one*
task's registers. Write a test that: programs task 0 slot 0 to block
`$05` and writes a marker byte through it; switches to task 1, programs
*its* slot 0 to a *different* block, and writes a *different* marker
through it; switches back to task 0; and asserts task 0's original
marker is still exactly what you wrote, undisturbed by anything that
happened while task 1 was active. Run it and confirm it passes against
the real code before moving on. Then temporarily hard-code `translate()` to
read `self.mmu[0]` regardless of `self.task` and confirm that the new test
catches the defect.

**5.10 — Sizing drill, one machine larger (drill).** §5.9 hand-computed
the 128K block-aliasing table. Do the same for a **512K** machine (64
physical blocks, `ram.len() == 0x80000`): what physical address does
MMU block `$47` alias to, and what physical address does block `$7F`
alias to? For each, name which *other*, smaller block number produces
the identical physical address — and explain in one sentence why a 512K
machine's aliasing pattern is a genuine hardware ceiling (per
DESIGN.md's confirmed-real-config table) rather than the same kind of
"no distinct storage back there" wraparound the 128K case is.

---

## What's next

This chapter resolved addresses for a fixed machine state. Chapter 6 adds
time. `run_field()` ([`machine/run.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs)) advances the CPU, converts cycles into
scanlines, and schedules horizontal and vertical synchronization. It also
turns the `$FFD8`/`$FFD9` speed control from a stored bit into a change in the
machine's execution rate.
