# Chapter 5 — The Bus: Memory Maps, the SAM, and the GIME MMU

*Week 5. Goal: given any address on any CoCo — 1, 2, or 3 — say with
certainty what the CPU actually touches. By the end of this chapter
`SystemBus::read`/`write` will read as plainly as a memory map poster,
you will know why the reset vector is the one address that can never be
banked away, and you will have watched a one-line reordering break four
tests for a reason you can explain in a sentence.*

Weeks 1–4 built a CPU that can execute anything, provided something on
the other end of `Bus::read`/`write` answers honestly. This week builds
that something. `mc6809` doesn't know a CoCo exists; from here on,
`coco-core`'s `SystemBus` is the entire outside world as far as the CPU
can tell — one flat 64K address space that, depending on four bits in a
register you're about to meet, might be RAM, might be one of two ROMs,
might be a chip register that clears itself when read, or might be
nothing at all.

---

## 5.1 Two chips, one seam

The CPU only ever calls `bus.read(addr)` or `bus.write(addr, val)` with a
16-bit `addr`. Everything this chapter covers is the answer to "which
physical byte, or which chip register, does that address actually name?"
— and the honest answer is "it depends," because the CoCo shipped in two
generations with genuinely different chips doing the deciding:

- **CoCo 1 and CoCo 2**: a MC6883 **SAM** (Synchronous Address
  Multiplexer) holds a handful of latched bits and maps the 64K space to
  ROM, RAM, or the I/O page in one lookup. No MMU — what you see is what
  you get, banked only by a single P1 bit for 64K machines.
- **CoCo 3**: the **GIME** absorbs the SAM's job (in a compatibility
  layer, so old software still works) and adds a real MMU: eight 8K
  logical slots, each independently pointed at any 8K physical block in
  up to 2 MB of RAM.

Both live in [`crates/coco-core/src/bus.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs), inside one struct,
`SystemBus`, which is what actually implements the `Bus` trait from
week 1:

```rust
pub struct SystemBus {
    pub variant: MachineVariant,
    #[serde(with = "serde_bytes")]
    pub ram: Box<[u8]>,
    #[serde(skip)]
    pub rom: Box<[u8]>,
    pub gime: GIME,
    pub sam: Sam,
    pub pia0: MC6821,
    pub pia1: MC6821,
    pub cart: Cart,
    pub vhd: Vhd,
    // ...
}
```
*([`crates/coco-core/src/bus.rs:36-110`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L36-L110), trimmed to the fields this
chapter needs.)*

Notice both `gime: GIME` and `sam: Sam` are always present, on every
machine. A CoCo 1 allocates a `GIME` it never looks at; a CoCo 3
allocates a `Sam` it never looks at. That's a few dozen wasted bytes in
exchange for something worth more: `SystemBus` is one concrete type, so
there is no generic parameter and no `dyn` anything standing between the
CPU's hot path and a memory access, and — the payoff you'll recognize
from week 1 — a single `#[derive(Serialize, Deserialize)]` covers the
whole thing for save states, with no "which variant am I" branching
required at (de)serialization time. You'll see exactly how the CPU
chooses between the two decoders in §5.4.

---

## 5.2 Decode order: what wins when address ranges overlap

Before the branch tables, internalize the *order of precedence*, because
several device ranges physically overlap and the order is the whole
story. Here is the real function, [`crates/coco-core/src/bus.rs:267-294`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L267-L294),
for the CoCo 3 path (the `variant != Coco3` branch peels off to a
completely different function, §5.5):

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

Four tiers, checked strictly top to bottom, each one a *precondition*
that, once true, ends the search:

1. **`$FFE0–$FFFF` — hardwired internal ROM.** `HARDWIRED_ROM_BASE =
   0xFFE0`. This is checked *before* the I/O-page test even though
   `$FFE0` is numerically inside `$FF00–$FFFF` too. If it weren't first,
   the six 6809 hardware vectors — including the reset vector at
   `$FFFE` — would be swallowed by the I/O-page branch below and read
   back as unmapped I/O garbage. §5.13 traces exactly this address
   through reset; exercise 5.6 asks you to justify the ordering from
   first principles.
2. **`$FF00–$FFBF` — the I/O page**, gated by `io_enabled` (a debugger
   convenience, always `true` on a running machine). Below `$FFE0`, so
   it never contests the hardwired vectors.
3. **The ROM window**, `is_rom_window(addr)` — `$8000–$FDFF`, gated on
   whether ROM is currently mapped in at all (SAM/GIME "map type" bit)
   and on the `$FE00–$FEFF` MC3 special case (§5.7). §5.6 is a full
   deep-dive on what happens *inside* this tier once it's entered.
4. **Everything else** falls through to `phys(addr)` — the MMU (or the
   fixed disabled-MMU map) translating into `self.ram`.

`write` ([`bus.rs:296-315`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L296-L315)) walks the identical four tiers with one
asymmetry worth flagging now and expanding in §5.10's neighbor: writes to
tier 1 are simply dropped (`$FFE0–$FFFF` isn't backed by RAM at all —
there's no "write-through" concept there), and writes that fall through
tier 3 (the ROM window when ROM *is* mapped) land on the RAM sitting
underneath it, not on the ROM itself — because ROM is read-only silicon
and the physical translation via `phys()` still resolves to a real RAM
address even while the CPU can't read it back.

> **Rust corner: precedence as a stack of early returns.** Notice there
> is no `match` here, no priority field, no sorted list of ranges —
> just four `if ... return` statements in a fixed order. This is a
> common Rust idiom for "first matching rule wins": each guard either
> returns immediately or falls through to the next. It reads
> top-to-bottom exactly the way the silicon's own priority encoder
> would, which is the point — when you're modeling a decode order that
   *is* meaningfully ordered (not just a disjoint partition), resist the
   urge to reach for a `match` on address ranges; a `match` implies the
   arms are mutually exclusive, and here `$FFE0` legitimately belongs to
   two overlapping ranges. The early-return chain makes the overlap and
   its resolution visible in the code, not hidden behind arm ordering
   that a future editor could silently reshuffle.

---

## 5.3 The I/O page, wall to wall

`$FF00–$FFFF` is fixed on every CoCo — it never moves regardless of
MMU or ROM-mapping state (only the hardwired-vector carve-out inside it
does anything unusual, and that's tier 1 above). Here is the complete
map as [`crates/coco-core/src/bus/io.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs)'s `io_read`/`io_write` actually
dispatch it — not just DESIGN.md's summary, the literal `match` arms:

| Range | Device | Notes |
|---|---|---|
| `$FF00–$FF1F` | PIA0 | Keyboard rows, joystick comparator, sync IRQs. Only 4 registers exist; `addr & 0x03` mirrors them across the whole 32-byte range. |
| `$FF20–$FF3F` | PIA1 | 6-bit DAC, cassette, VDG-legacy mode bits. Same 4-register mirror. |
| `$FF40–$FF7E` | Cartridge / FDC (SCS\*) | The "standard" SCS window is `$FF40–$FF5F`; some carts (RS-232 Pak, Orchestra-90, the Sound/Speech Cartridge) decode further registers out to `$FF7E` — the full address bus reaches the expansion connector regardless of what the motherboard "intends." |
| `$FF41` / `$FF42` | Becker port (DriveWire) | Intercepts **ahead of** cartridge dispatch, on both decode paths, whenever a `DwServer` is installed — mirrors MAME's handler-install order. |
| `$FF7F` | Multi-Pak Interface select | Only meaningful with an MPI inserted; decoded by the MPI itself, never by a plugged-in cart. |
| `$FF80–$FF86` | VHD (virtual hard disk, NitrOS-9 `emudsk`) | `$FF87–$FF8F` is unmapped/open bus. |
| `$FF90` | INIT0 | MMU enable, ROM map bits, MC3, IRQ/FIRQ master enables, CoCo-compat select. |
| `$FF91` | INIT1 | Timer clock select, MMU task select. |
| `$FF92` | IRQENR | IRQ source enable (write) / latched status, clear-on-read (read). |
| `$FF93` | FIRQENR | FIRQ twin of `$FF92`. |
| `$FF94`/`$FF95` | Timer MSB/LSB | 12-bit interval timer; write-only (reads as 0 on hardware). |
| `$FF96`/`$FF97` | reserved | Unused on the GIME. |
| `$FF98`–`$FF9F` | GIME video: VMODE, VRES, BORDER, VBANK, VSCROLL, VOFFSET1/0, HOFFSET | Week 8 territory; write-only like the timer regs (`io.rs`'s `TIMER_MSB_REG..=GIME_LAST => 0` catch-all). |
| `$FFA0–$FFAF` | MMU task registers | `$FFA0–$FFA7` = task 0 slots 0–7, `$FFA8–$FFAF` = task 1 slots 0–7. §5.7/§5.8. |
| `$FFB0–$FFBF` | Palette | 16 registers, 6-bit RGB each. Week 8. |
| `$FFC0–$FFDF` | SAM-compatibility strobes | Write-only, even/odd set-clear pairs. §5.4/§5.5. |
| `$FFE0–$FFFF` | Hardwired internal ROM | Tier 1 from §5.2 — bypasses everything else on this table. |

Two details worth internalizing because they trip people up reading the
source cold:

- **`addr & 0x03`** is how both PIA read/write arms pick a register —
  `io_read`'s `IO_BASE..=PIA0_LAST => { ...; self.pia0.read((addr &
  0x03) as u8) }` ([`bus/io.rs:61-66`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L61-L66)). A real 6821 only decodes its
  bottom two address lines; every other line is "don't care," which is
  why `$FF00`, `$FF04`, `$FF08`, … all reach the *same* register. This
  isn't a shortcut the emulator took — it's literally how the chip's
  address pins are wired.
- **Most GIME video/timer registers are write-only on real hardware.**
  `io_read`'s `TIMER_MSB_REG..=GIME_LAST => 0` arm ([`bus/io.rs:83`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L83))
  isn't a stub; a real GIME's video registers genuinely don't drive the
  data bus on a read. If you ever wrote 6809 assembly that tried to
  read-modify-write `$FF98`, that bug is not your emulator's fault —
  it's period-accurate.

### Walking the dispatch: PIA, cassette, cart, VHD

The table above tells you *where* each device lives; it's worth reading
the actual dispatch once so the *shape* of a device match arm is
familiar before weeks 10–13 dig into any one of them individually. Three
patterns recur across almost every arm in `io_read`/`io_write`.

**Pattern 1: input pins are computed, not stored.** A PIA is a dumb
20-pin chip — it latches whatever voltage is on its input pins at the
moment of a read, plus a direction register per pin. In this emulator
nothing *pushes* a live voltage into `pia0`/`pia1` between accesses; the
bus computes the right byte on demand, immediately before handing the
read to the PIA:

```rust
IO_BASE..=PIA0_LAST => {
    // Refresh port A's input pins (keyboard rows + joystick
    // comparator/buttons) before the PIA read.
    self.pia0.a.input = self.pia0_pa_pins();
    self.pia0.read((addr & 0x03) as u8)
}
```
*([`bus/io.rs:61-66`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L61-L66))*

`pia0_pa_pins` is the function that does the computing:

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

That single byte on PIA0 port A is doing three unrelated jobs at once —
keyboard row sense, joystick fire buttons, and the joystick's analog
comparator bit — because on real hardware, that's exactly what's wired
to those eight pins. You'll spend week 10 inside `keyboard.rs` and
`joystick.rs`; for now, the lesson is structural: **a PIA's "input"
isn't state the PIA owns, it's a snapshot the bus takes of everything
else in the machine, taken fresh on every single read.** PIA1's port A
gets the same treatment for the cassette input line, at the opposite
extreme of complexity — one bit, one device:

```rust
pub(super) fn pia1_pa_pins(&self) -> u8 {
    const CASSETTE_IN: u8 = 0x01;
    if self.cassette.input_bit() { 0xFF } else { !CASSETTE_IN }
}
```
*([`bus/pins.rs:33-40`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/pins.rs#L33-L40))*

**Pattern 2: some writes fan out to more than one device.** Cassette
*output* is the mirror image of cassette input, and it doesn't live
behind its own address at all — it's tapped directly off whatever PIA1
write just happened, because the DAC and the cassette relay share the
same physical port:

```rust
PIA1_BASE..=PIA1_LAST => {
    self.pia1.write((addr & 0x03) as u8, val);
    // Cassette record-out is a direct, unconditional tap of the DAC
    // (not gated by SNDEN/the mux) fed on every PIA1 write since any of
    // them (port A output/DDR or CRA, which carries the motor relay)
    // can change it.
    let dac = (self.pia1.a.output & self.pia1.a.ddr & 0xFC) >> 2;
    self.cassette.record_dac(dac, self.pia1.a.c2_output());
    self.note_audio_write(); // DAC / PB1 / SNDEN / relay
}
```
*([`bus/io.rs:104-113`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L104-L113))*

Every single PIA1 write — even one that has nothing to do with the
cassette, like flipping a completely unrelated control bit — re-derives
the current 6-bit DAC value and feeds it to `cassette.record_dac`,
because from the cassette's perspective *any* PIA1 write might have
just changed the analog level it's supposed to be recording. Week 11
and week 12 build on exactly this tap.

**Pattern 3: a device group with its own small sub-dispatch.** VHD
(`$FF80–$FF86`, NitrOS-9's virtual hard disk) is a good miniature of
"one device, several registers, one line of dispatch each":

```rust
VHD_LRN_HI | VHD_LRN_MID | VHD_LRN_LO | VHD_BUFFER_HI | VHD_BUFFER_LO => {
    self.vhd.read_lrn_or_buffer()
}
VHD_COMMAND_STATUS => self.vhd.read_status(),
VHD_SELECT => OPEN_BUS, // always open bus, unconditionally (spec)
```
*([`bus/io.rs:74-78`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L74-L78), read side)*

Three LRN (logical record number) bytes, two buffer-pointer bytes, one
command/status register, and one drive-select register that — per the
device's own spec, not a stub — always reads back open bus regardless
of what's written to it. §5.11 collects every open-bus convention in
the codebase, VHD's included, into one place.

**Precedence, again.** One more overlap worth naming before you meet
the rest of the cart range in week 13: the Becker port ($FF41/$FF42,
DriveWire-over-serial) sits *inside* the cartridge's `$FF40–$FF7E`
range, and both `io_read` and `sam_read`'s I/O sub-decode check it
*first*, unconditionally:

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

`becker_read` returns `None` — "not my address, or the Becker port isn't
even enabled" — and lets the caller fall through to the ordinary
cartridge decode underneath it. This is the exact same "first matching
rule wins, but only when it actually matches" idea from §5.2's Rust
corner, applied one level down and inside a single device's own address
range rather than across the whole 64K.

---

## 5.4 Two machines, two decode paths

Go back to the top of `read`/`write` in §5.2: `if self.variant !=
MachineVariant::Coco3 { return self.sam_read(addr); }`. That's the
*entire* CoCo-1/2-vs-CoCo-3 decision — one branch, evaluated once per
access, immediately routing to one of two independent, unrelated
functions. The struct's own doc comment explains why this shape and not
something fancier:

```rust
/// Which machine this bus decodes addresses for. `Bus::read`/`Bus::write`
/// branch on this once, up front, into two independent concrete decode
/// paths (GIME vs plain SAM) rather than a trait object — see
/// `docs/coco12-plan.md` Phase 2.
pub variant: MachineVariant,
```
*([`bus.rs:38-42`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L38-L42))*

Think about the alternative designs and why each loses:

- **A `dyn Decoder` trait object**, one impl per variant. This is the
  "obvious" OOP move, and it's wrong here for the same reason week 1's
  `impl Bus` beat `dyn Bus` on the CPU's hot path: every single memory
  access — several per instruction, tens of millions per second of
  emulated time — would pay a vtable indirection for a decision that is
  *already known and fixed* the moment the machine is constructed. The
  variant never changes mid-run.
- **Generic `SystemBus<V: Variant>`.** Monomorphization would give you
  the speed back, but now `Machine` itself needs a type parameter, it
  infects every function signature that touches a `Machine`, and (the
  quieter cost) `#[derive(Serialize, Deserialize)]` for save states gets
  much less pleasant to write once the concrete type of `Machine` isn't
  singular. Week 16 leans hard on `Machine` being one plain, ordinary
  struct; a generic bus would tax every chapter after this one to save
  a branch that costs nothing.
- **One unified decode function with `if variant == Coco3 { ... } else {
  ... }` sprinkled through every tier.** This is what you'd get if you
  tried to *merge* `sam_read` and the CoCo 3 `read` body into one
  function "to avoid duplication." Read [`bus/sam_path.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sam_path.rs)'s own module
  doc comment for why the codebase explicitly rejected this:

  ```rust
  //! Plain-SAM path (CoCo 1/2, no GIME): `Sam::map` does the whole-address
  //! decode (RAM/ROM/cart/I/O/open-bus) in one step, unlike the GIME path's
  //! separate ROM-window/I/O-page/MMU layers, so there's no need for
  //! `phys`/`is_rom_window`/`rom_read` equivalents here. This path never
  //! touches `self.gime` — no MMU translate, no interrupt raises, no timer.
  ```
  *([`bus/sam_path.rs:1-7`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sam_path.rs#L1-L7))*

  The two chips don't just have different registers — they have
  **differently shaped decode algorithms**: the SAM resolves a whole
  address to a target in one `match` (§5.5); the GIME needs multiple
  ordered tiers because it has ROM-mapping and MMU translation as
  separate, independently-configurable stages. Forcing them into one
  function would mean the CoCo 1/2 path pays mental (and possibly
  runtime) overhead for machinery it fundamentally doesn't have, for the
  sake of a code-reuse ideal the two chips don't actually share. **A
  little duplication that mirrors two genuinely different pieces of
  hardware is more honest than a unification that papers over the
  difference.**

This is the same "load-bearing abstraction" instinct from week 1,
applied in the opposite direction: there, one seam (`Bus`) was worth
generalizing because every device behind it really does share one
two-method contract. Here, two decoders are worth *not* generalizing
because the CoCo 1/2 and CoCo 3 memory systems are different machines
wearing the same 64K clothes.

---

## 5.5 The CoCo 1/2 path: `Sam::map` and the strobe registers

The MC6883 SAM predates the GIME by half a decade and does its whole job
in one function, `Sam::map` ([`crates/coco-core/src/sam.rs:140-176`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/sam.rs#L140-L176)):

```rust
pub fn map(&self, addr: u16) -> SamTarget {
    // The vector mirror and the $FF00+ fixed page win regardless of TY —
    // "the mirror region stays ROM" even in all-RAM mode.
    if addr >= VECTOR_MIRROR_BASE {
        return SamTarget::RomBas(BAS_MIRROR_OFFSET + (addr - VECTOR_MIRROR_BASE) as usize);
    }
    if addr >= STROBE_BASE {
        return SamTarget::Io; // $FFC0-$FFDF: SAM control strobes.
    }
    if (OPEN_BUS_BASE..=OPEN_BUS_LAST).contains(&addr) {
        return SamTarget::OpenBus; // $FF7F-$FFBF.
    }
    if addr >= IO_BASE {
        return SamTarget::Io; // $FF00-$FF7E: PIA0/PIA1/cart SCS (+ extension).
    }
    if self.ty && self.is_64k() {
        // All-RAM mode extends the RAM decode through $FEFF.
        return SamTarget::Ram(addr as usize);
    }
    match addr {
        0x0000..=0x7FFF => {
            let ram_addr = if self.p1 && self.is_64k() { addr | 0x8000 } else { addr };
            SamTarget::Ram(ram_addr as usize)
        }
        EXT_ROM_BASE..=EXT_ROM_LAST => SamTarget::RomExt((addr - EXT_ROM_BASE) as usize),
        BAS_ROM_BASE..=BAS_ROM_LAST => SamTarget::RomBas((addr - BAS_ROM_BASE) as usize),
        CART_ROM_BASE..=CART_ROM_LAST => SamTarget::Cart((addr - CART_ROM_BASE) as usize),
        _ => unreachable!("address {addr:#06x} not covered by the SAM decode"),
    }
}
```

One function, one `enum` result (`SamTarget`), no separate tiers to
re-check on every call — [`bus/sam_path.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sam_path.rs)'s `sam_read`/`sam_write` just
`match` on what `map` handed back and act. Structurally simpler than the
CoCo 3 path because the hardware genuinely is simpler: no MMU, no
independent ROM-map stage — the SAM's handful of latched bits collapse
straight to one target per address. Notice too that the SAM's ROM decode
is completely **fixed**: `EXT_ROM_BASE..=EXT_ROM_LAST` and
`BAS_ROM_BASE..=BAS_ROM_LAST` are constants, not something a register can
reprogram. §5.6 is entirely about the CoCo 3 giving up exactly that
fixedness in exchange for two extra configuration bits.

> **Rust corner: an `enum` as a decode result.** `SamTarget` — `Ram`,
> `RomExt`, `RomBas`, `Cart`, `Io`, `OpenBus` — is a textbook use of an
> algebraic data type as a *typed, exhaustive* answer to "what is this
> address." Compare this to how you'd likely do it in C: an integer tag
> plus an offset, with the compiler powerless to stop you reading the
> offset when the tag says `Io`. Here, `SamTarget::Ram(usize)` carries
> its payload *only* in the variant where a payload makes sense, and
> every `match` on a `SamTarget` that omits a variant is a compile
> error, not a runtime surprise the day someone adds `OpenBus`. This
> pattern — "decode to an enum, then match exhaustively" — recurs
> throughout this codebase; get comfortable reading it now.

### The strobe registers: your first "weird hardware" interface

Here's the detail that startles people who've only ever programmed
against friendly memory-mapped registers: `$FFC0–$FFDF` isn't sixteen
independent bit-fields you read and write like normal bytes. It's 16
*pairs* of write-only addresses. **The data byte you write is completely
ignored — only the address matters**, and within each pair, the *even*
address clears one specific bit and the *odd* address sets it:

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
compatibility overlay next.

> **Rust corner: deriving two facts from one number.** `(addr -
> STROBE_BASE) / 2` and `(addr - STROBE_BASE) & 1` pull the "which bit"
> and "set or clear" facts out of a single offset with two integer
> operations instead of a 32-entry lookup table or a match with 32 arms.
> This is a common trick when hardware imposes a systematic
> address-to-meaning mapping: notice the *structure* (even/odd pairs,
> sequential index) and compute rather than enumerate. It only works
> because the mapping really is that regular — don't reach for this if
> the pairing has exceptions, which is exactly why `Sam::map`'s vector
> mirror and open-bus carve-outs above are handled as explicit early
> returns instead of folded into the arithmetic.

The CoCo 3's GIME keeps an independent copy of this exact even/odd idea
for backward compatibility — you'll meet [`gime/sam_compat.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/sam_compat.rs)'s version
in §5.7, right after §5.6 covers the ROM window's own banking tricks,
which the SAM (as you just saw) doesn't have at all.

---

## 5.6 The GIME's ROM window: MC1:MC0 and what a cartridge sees

You just read `Sam::map`'s ROM decode: two hard-coded 8K windows,
`EXT_ROM_BASE..=EXT_ROM_LAST` and `BAS_ROM_BASE..=BAS_ROM_LAST`, that
never move. The CoCo 3's `$8000–$FDFF` window is the same 32K of address
space doing the same job, but the GIME adds one thing the SAM never had:
two configuration bits, `MC1` and `MC0` in `INIT0` (`$FF90`), that decide
*which chip* answers each half of that window — the internal ROM you've
already seen (`coco3.rom`), or an external cartridge's ROM arriving over
the `CTS*` pin. This is the tier-3 decode from §5.2 opened up.

### The four states

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
half of ROM — you traced exactly this in [`tests/boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/boot.rs)'s
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
`$8000–$FDFF` *and* the rest of the window handed to an external
cartridge — tier 1 from §5.2 (`HARDWIRED_ROM_BASE`) still intercepts
`$FFE0–$FFFF` before `rom_read` is ever asked whether the address is
external. `rom_read` itself encodes the boundary explicitly:

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

One more consequence worth internalizing, because it explains a real,
documented CoCo 3 quirk: `CTS*`'s external window stops at `$FDFF`, not
`$FFFF`. A "16K" cartridge ROM logically spans `$C000–$FFFF`, but the
GIME only ever asserts `CTS*` for `$C000–$FDFF` — `0x1E00` bytes short
of a full 16K (`$FE00–$FFFF` is 512 bytes the cartridge bus simply
cannot see, ever, regardless of MC1:MC0). Recall from §5.2 tier 3 that
`$FE00–$FEFF` is the MC3-gated constant-RAM page, and `$FFE0–$FFFF` is
the hardwired vector tier — between the two, that leaves exactly
`$FE00–$FEFF` as a *maybe*: with `MC3` clear, that page "follows the
normal map like the rest of the `$8000+` window" (the doc comment on
`CONSTANT_RAM_BASE`), which for an external-ROM cartridge means it reads
as the **tail of the cartridge's own ROM image**, even though `CTS*`
itself never fires for it — the emulator's `is_rom_window`/`rom_read`
just keep applying the same MC1:MC0 rule uninterrupted through `$FEFF`.
This is the *only* way a Program Pak's last `$200` bytes of image data
are addressable at all, and real software depends on it: Sokoban keeps
its palette tables there and copies them out via `$FE88` reads, because
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
seven bytes apart (`$FDFF` vs `$FFFE`), because tier 1 doesn't care what
tier 3 decided.

---

## 5.7 The GIME MMU: 8K slots and two task sets

The GIME's compatibility layer for `$FFC0–$FFDF` is a *second*,
separate implementation of the same even/odd strobe convention —
deliberately not shared code with `sam.rs` (its own module doc comment
says so explicitly, "so the CoCo 3 path stays completely untouched").
It only implements the strobes the CoCo 3 actually uses — V0–V2, F0–F6,
R1, TY — and explicitly does *not* model P1, M0, M1, or the CoCo 1/2 R0
pair, because those don't do anything on real CoCo 3 hardware:

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

If you ever typed `POKE 65497,0` on a real CoCo 3 to get double speed
and `POKE 65496,0` to put it back, decimal 65497 and 65496 are
`$FFD9`/`$FFD8` — exactly `SAM_R1_SET`/`SAM_R1_CLEAR` above. If you also
remember an *older* pair, `POKE 65495,0`/`POKE 65494,0` (`$FFD7`/
`$FFD6`, the SAM's own `R0` strobe from §5.5's list), the CoCo 3's
`write_sam` has **no arm at all** for those two addresses — they fall
straight through the `match`'s `_ => {}` catch-all with no effect
whatsoever, not even latched into an unused field (the GIME struct has
no `r0`-equivalent field to latch into). That matches the real chip:
SEB Unravelled II's own register figure lists only `R1` as active on
the GIME. The CoCo 1/2 `R0` pair being "address-dependent" (nominally
fast for ROM fetches, slow for RAM) was already a soft-edged feature on
the original SAM; the GIME's designers apparently decided one clean,
unconditional double-speed bit was enough and didn't bother wiring the
old pair through at all.

That's `$FFC0–$FFDF` handled — SAM compatibility, present but a
sideshow. The GIME's actual headline feature lives at `$FFA0–$FFAF`:
a real **MMU**.

### The mental model

Split the CPU's 64K logical space into eight 8K windows (`$0000–$1FFF`,
`$2000–$3FFF`, … `$E000–$FFFF`). The MMU holds, per window, an 8-bit
**physical block number** — which 8K chunk of up to 2 MB of installed
RAM that window currently shows. Sixteen registers, `$FFA0–$FFAF`,
because there are *two independent sets* of eight — "task 0" and "task
1" — and one bit elsewhere (`INIT1` TR) picks which set is active.
Software can prepare task 1's mapping while task 0 is running, then
switch the whole 64K view over in a single register write — no per-slot
reprogramming needed for a context switch. §5.8 is the whole story of
what real software actually did with that trick.

```rust
/// Number of MMU task register sets ($FFA0–A7 and $FFA8–AF).
pub const TASK_COUNT: usize = 2;
/// Logical 8K slots per task (the 64K CPU space / 8K).
pub const SLOTS_PER_TASK: usize = 8;
```
*([`gime.rs:20-23`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L20-L23))*

```rust
pub mmu: [[u8; SLOTS_PER_TASK]; TASK_COUNT],
pub task: usize,
pub mmu_enabled: bool,
```
*([`gime.rs:174-176`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L174-L176))*

`$FFA0–$FFA7` decode to `mmu[0][0..8]`, `$FFA8–$FFAF` to `mmu[1][0..8]`.
The decode is arithmetic, not a match arm per register:

```rust
/// Decode an `$FFA0–$FFAF` MMU register address to `(task, slot)`.
fn mmu_index(addr: u16) -> (usize, usize) {
    let idx = (addr - MMU_BASE) as usize;
    (idx / gime::SLOTS_PER_TASK, idx % gime::SLOTS_PER_TASK)
}
```
*([`bus.rs:262-265`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L262-L265))*

### The translation itself

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

This is the formula the syllabus wants you fluent in: **`phys = (block
<< 13) | (addr & 0x1FFF)`**. `addr >> 13` picks which of the eight
8K windows the address falls in (equivalently, `& 7` — the top three
bits of a 16-bit address beyond bit 13 don't exist, so the mask is
almost decorative, but it's there for defense-in-depth); `addr & 0x1FFF`
is the byte offset *within* that 8K window, preserved unchanged into the
physical address; `block << 13` places that offset at the right spot in
physical memory. A block number can be anywhere in `0..=255` — the GIME
addresses a full 2 MB (`256 × 8K`) even though Tandy only ever shipped
up to 512K — which is why `SystemBus::phys` finishes the job with a
modulo against whatever RAM is actually installed:

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
would otherwise point off the end of physical RAM. §5.9 is a full
treatment of exactly what that means for a small machine.

> **Rust corner: `%` as intentional hardware fidelity, not a bug
> smell.** In application code, an unexplained `%` on an index usually
> means "someone forgot to bounds-check and is papering over it." Here
> it's the opposite: the modulo *is* the documented hardware behaviour
> (DESIGN.md §3 calls it "the mask relocates it to the top 64K"), and
> [`tests/bus_map.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bus_map.rs)'s `small_machine_aliases_high_window_into_top_blocks`
> pins the exact aliasing a 128K machine produces. When you see
> deliberate wraparound arithmetic in emulator code, look for the
> comment or test that says *why* — it's very often "this is what the
> chip does," not an oversight. Contrast with `wrapping_add` from
> week 1's `Bus::read_u16`: same instinct (embrace fixed-width
> wraparound instead of fighting it), different operator, same reason
> a `checked_*` or panicking method would be *wrong* here.

### MC3: the one address range the MMU can't touch

One wrinkle sits inside `phys` before the MMU is even consulted: when
`INIT0` bit `MC3` is set, `$FE00–$FEFF` is **pinned** to physical
`$7FE00` regardless of what the active task's MMU slot says. Why would
you want a 256-byte page immune to the very banking mechanism you just
built? Because interrupt vectors have to be reachable no matter what
task is active or what's banked into the rest of the address space —
BASIC keeps its interrupt trampolines here specifically so an interrupt
firing mid-context-switch still lands on working code. `is_rom_window`
([`bus.rs:234-242`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L234-L242)) makes the corresponding read-side promise: when MC3
is set, `$FE00–$FEFF` is *never* treated as ROM either, even if the rest
of the `$8000+` window currently is — it's unconditionally the constant
RAM page. When MC3 is clear, that page just follows the ordinary
ROM/RAM map like any other byte in the window (§5.6 walked exactly this
case for an external cartridge); §5.14's second worked example walks
both states through `bus_map.rs`.

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
*not* do for you. Paraphrasing its warning: swapping task sets **does
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

### OS-9: the same primitive, a genuinely different use

Real multi-tasking OS-9 (Level II, the CoCo 3's native multi-user
operating system) builds something much bigger on the identical
hardware feature. In broad strokes — this is software history, not a
claim about anything `coco-core` itself implements, so take it as
context rather than a spec — OS-9's kernel keeps its own code, drivers,
and file managers permanently resident in **one** task register set,
so that no matter which user process is currently running, an interrupt
or a system call always lands on stable, unbanked kernel code with
nothing more than an `INIT1` bit-flip needed to reach it. The **other**
task register set is the one that actually changes: each time OS-9's
scheduler dispatches a different process, it copies that process's own
8-block memory map into the currently-inactive register set — while the
*other* set (wherever the previous process or the kernel currently
sits) keeps running completely undisturbed — and only then flips the
task-select bit. The new process's entire 64K view becomes live in one
write, with no memory actually copied and no process's data ever
touched by another process's mapping. This is precisely BASIC's
`SELTASK0`/`SELTASK1` trick, generalized from "reach a graphics buffer
for a few instructions" to "this is how an entire multi-user operating
system switches between running programs without a device driver or an
MMU fault ever knowing the difference." (Source: forum documentation of
OS-9's task-register usage; see the citation at the end of this
chapter's Reading assignment. This is not something the local reference
PDFs cover, and this chapter has not verified the exact polarity of
which GIME task-register set OS-9 calls "system" versus "user" against
a primary source — treat the mechanism as solid and the labeling as
secondhand.)

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
So do `$31`/`$01`, and every other pair sixteen apart — because `0x20000
/ 0x2000 = 16` exactly, `% self.ram.len()` on a 128K machine is
arithmetically identical to `block % 16`. This is precisely the
behavior an independently published reference (Chris Lomont's CoCo
hardware notes) documents from the real chip: on a 128K machine, MMU
pages `$00`–`$2F` are copies of pages `$30`–`$3F` — there is no 512K
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

I verified the two-way aliasing directly rather than trust the
arithmetic on paper alone: a temporary test that enabled the MMU on a
128K machine, programmed one slot to block `$00` and a second slot to
block `$30`, wrote a marker through the first slot, and read it back
through the second (and vice versa) passed against the real code before
being deleted — the two block numbers really do address the same bytes,
both ways, exactly as the table predicts.

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

Be precise about what "modeled" means here, because `gime.rs`'s own
module header is stale and will mislead you if you trust the prose over
the code:

```rust
//! STATUS: MMU translate, SAM compatibility strobes, and the video registers
//! ($FF98–$FF9F) are modelled; native scanout lives in `gime_video`. The timer,
//! GIME-sourced interrupts, and the write-8/read-6 register asymmetry are TODO.
```
*([`gime.rs:1-5`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L1-L5), emphasis on the last clause)*

That comment says the asymmetry is *still TODO* — but `MMU_READ_MASK`
exists, is applied on every MMU register read, and [`tests/bus_map.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/bus_map.rs)
has a passing test, `mmu_register_write_8_read_low_6`, proving the
low-6-bits behaviour works today:

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

The comment is simply out of date;
someone implemented the feature and didn't update the file banner.
**Trust the code and the tests over prose comments when they disagree —
comments don't get exercised by `cargo test`.** This is a useful
lesson independent of this specific chip: doc comments describe intent
at the moment they were written and silently rot as the code around
them changes, while a passing test is a claim the compiler and test
runner both actively re-verify on every run.

There's also a real simplification worth being honest about, separate
from the stale comment. DESIGN.md's fuller description is "return
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
garbage value would be relying on undefined behaviour on real hardware
too. But it is a simplification, and now you know exactly where it is
if you ever need to model a specific real machine's readback more
faithfully.

---

## 5.11 Open bus: where the `$FF` — and the `$00` — come from

You've now seen `OPEN_BUS` fall out of half a dozen unrelated decode
paths: the I/O page's catch-all, an empty cartridge slot, a truncated
ROM image, a RAM address past the end of a small machine. It's worth
collecting every one of these into a single place, because they are
*not* all the same value, and the difference is a real hardware fact,
not an inconsistency to paper over.

### What "open bus" means

No chip is driving the data bus for that address. On a real 6809 system
the CPU still reads *something* — whatever charge happens to be sitting
on the bus lines, typically pulled toward a resting state by the bus's
own passive electrical characteristics — and different bus segments on
the CoCo rest at different levels. The emulator can't (and shouldn't try
to) model the analog physics; instead, each region that can go
unanswered picks a **fixed, documented stand-in value**, matched to what
real hardware (or MAME, where the local docs are silent) actually
returns.

### Every open-bus constant in the codebase

| Constant | Value | Where it fires |
|---|---|---|
| `bus::regs::OPEN_BUS` | `0xFF` | The I/O page's final catch-all (`io_read`'s `_ => OPEN_BUS`); `VHD_SELECT` unconditionally; a ROM image shorter than the window it's mapped into (`rom.get(off).copied().unwrap_or(OPEN_BUS)` in `rom_read`, §5.6). |
| `sam::SamTarget::OpenBus` region | `0xFF` | CoCo 1/2 only: `$FF7F–$FFBF`, the range that would be GIME registers on a CoCo 3 but simply doesn't exist without one ([`tests/sam.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/sam.rs)'s `ff7f_to_ffbf_is_open_bus_on_coco1_2`). |
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
You met the identical instinct in §5.14's first worked example:
`mc_16k_split_routes_upper_half_to_cartridge` asserts `b.read(0xC123) ==
0x00` for an empty cartridge slot — that `0x00`, not `0xFF`, is
`ROM_OPEN_BUS` doing
exactly what its comment says.

### The debugger already assumes you'll get this wrong

One more place open bus shows up, foreshadowing week 16: every
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

This is the same `read`-vs-`peek` split week 1 introduced for PIA
interrupt flags, applied to cartridges: a debugger memory view that
can't safely read a device's ROM without a side effect reports open bus
rather than risk corrupting the machine it's supposed to be inspecting
— "we don't know" is a legitimate answer for a decode function to give,
as long as every caller agrees on what "we don't know" looks like in
that specific region of the bus.

---

## 5.12 ROM composition and CRC validation

### Two very different ROM stories

**CoCo 3**: one 32K image, `coco3.rom`, loaded whole and mapped
verbatim — `SystemBus::rom_read` computes `off = addr - 0x8000` and
indexes straight into it ([`bus.rs:252-258`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L252-L258)). This is the ROM you saw
every time you turned your CoCo 3 on: Super Extended Color BASIC,
occupying the full `$8000–$FFFF` window when INIT0's `MC1:MC0` bits
select 32K-internal (the machine's cold-start default, and why
`PEEK` above 32767 on a diskless CoCo 3 always read ROM, never open
cartridge bus — §5.6 has the full four-state table for every other
combination of those two bits).

**CoCo 1/2**: no single "the ROM." Real machines shipped multiple
separate mask ROM chips — Extended Color BASIC at `$8000–$9FFF`, plain
Color BASIC at `$A000–$BFFF` — and a machine with only Color BASIC
installed (many did) simply has *nothing* answering the Extended BASIC
range. `Sam::map` reflects the chip boundary directly as two separate
`SamTarget` variants (`RomExt`/`RomBas`, §5.5) rather than one flat
image, but at load time `coco-egui` and the test suite still need
*something* to hand `SystemBus::new` for that missing half. The
approach, verified in [`tests/coco1_boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/coco1_boot.rs):

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

`0xFF` isn't arbitrary — it's what an empty, unconnected bus line reads
as (§5.11's whole subject), the same convention `bus.rs`'s own
`OPEN_BUS: u8 = 0xFF` constant uses for every other unmapped range in
this chapter's table. A test that boots this composed image
(`ty0_reads_rom_at_extbas_and_bas_windows`-adjacent coverage, and
`coco1_boot.rs` proper) proves the machine boots into the plain "COLOR
BASIC" banner, not "EXTENDED COLOR BASIC" — because `$8000–$9FFF`
genuinely reads back `$FF` bytes, which don't disassemble into working
BASIC startup code, so the ROM's own startup sequence detects the
absence and skips straight to the Color BASIC banner. `coco-egui`'s
`compose_coco12_rom` does the general version of this same layout at
runtime, picking whichever `extbas*.rom`/`bas*.rom` files are present
and filling the gap the same way when they aren't.

### Knowing what you actually loaded

Before any of this, you generally want to know *which* dump you've got
— homebrew patches and mislabeled files are common in the wild.
`rom_db.rs` keeps a manifest of every known-good CoCo ROM (copied from
MAME's own romset definitions) and validates by content, not filename:

```rust
pub enum Validation {
    Verified(&'static KnownRom),
    Mismatch { expected: &'static KnownRom, actual_crc32: u32, actual_size: usize },
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

Every decode rule in this chapter converges on one address the very
first instant the machine exists. `Machine::new` ([`machine.rs:167-176`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine.rs#L167-L176)):

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

`cpu.reset` ([`crates/mc6809/src/lib.rs:177-183`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L177-L183)):

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

1. `read_u16` (week 1's default trait method) issues two big-endian
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

`coco3.rom[0x7FFE..0x7FFF] = {0x8C, 0x1B}` — the vector *stored* at the
very end of the ROM image points to `$8C1B`, an address near the
*start* of the mapped ROM (offset `0x0C1B`). That's completely ordinary
for a 6809 reset vector: the vector table lives at the top of the
address space by architecture convention, but nothing says the code it
points to has to live near there too. `PC = 0x8C1B` after `reset()`
returns is your first, cleanest proof that decode order, ROM mapping,
and the hardwired-vector carve-out are all wired correctly before a
single instruction has executed. `cold_start_configures_rom_and_jumps
_into_upper_half` (same file) carries the trace five instructions
further: the cold-start code immediately does `ORCC`, then `LDA
#$0A / STA $FF90` — writing `INIT0` with `MC1` and `MC3` set (32K
internal ROM, constant vector page — the `MC=10` row of §5.6's table) —
then `CLR $FF91`, then jumps to `$C000`, now reading the *upper* half of
the same 32K image. That INIT0 write is what makes tier 3
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
BASIC opcodes. Four examples here, chosen to each exercise a different
tier from §5.2 (§5.6 and §5.9 walked two more pairs of tests each, right
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
(week 13 territory) is the same kind of narrow seam as `Bus` itself, and
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
the pinned page and still obeys the ordinary ROM-window rule: the second
half of the test writes to it and confirms the write was silently
dropped, because `$FDFF` is still ROM (`MC1` selects 32K-internal, so
the write to it has no effect and the subsequent read still returns
`marked_rom`'s value). One test, two adjacent addresses, two entirely
different memory semantics, decided by three bits.

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
newly-active task's mapping instead. If you ever need to convince
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
2. **[`crates/coco-core/src/bus/io.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs)** — the full I/O dispatch. Cross
   the table in §5.3 off against every `match` arm as you go; find the
   one register this chapter didn't mention (there's at least one).
3. **[`crates/coco-core/src/bus/pins.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/pins.rs)** — short, and it recasts every
   PIA "read" you'll do from week 10 onward: nothing is stored, it's all
   computed fresh from other devices' state at the moment of access.
4. **[`crates/coco-core/src/gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs), lines 1–70 and 230–300** — the
   register bit constants (skim; you'll be back for these in week 8)
   and `translate`/`write_init0`/`write_init1`/`rom_is_external` in
   full.
5. **[`crates/coco-core/src/sam.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/sam.rs), whole file** — short enough to read
   end to end, and doing so makes explicit just how much simpler the
   CoCo 1/2 memory story is next to the GIME's.
6. **[`crates/coco-core/src/bus/sam_path.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sam_path.rs)** — the thin adapter that
   turns `Sam::map`'s `SamTarget` into actual reads and writes; note
   how little code it takes once `Sam::map` has already done the real
   work.
7. **[`crates/coco-core/src/cart.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cart.rs#L1-L70), lines 1–70** — the `Cartridge`
   trait and its two open-bus constants (§5.11); a preview of week 13
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
confirm the suite is green again. (This is not a hypothetical: making
exactly this edit and reverting it is how this chapter's own author
verified the claim before writing it down.)

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
   responsible for that behaviour.

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
write the mirror-image test for `MC1` set: confirm `MC0` still doesn't
matter (`MC=10` and — hypothetically — an `MC=11`-adjacent `MC1`-alone
state are *not* the same thing, unlike the `MC1`-clear case; make sure
your test is actually distinguishing the right pair of states before
you trust it).

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
the real code before moving on — and then, just to feel what "isolation"
actually buys you, temporarily hard-code `translate()` to always read
`self.mmu[0]` regardless of `self.task` and watch your own test catch
it.

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

Everything in this chapter answers "what does address X touch" for a
*static* snapshot of the machine's registers. Week 6 asks the next
question: *when*. `run_field()` ([`machine/run.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs)) is the loop that
actually drives the CPU forward one instruction at a time, converts
elapsed CPU cycles into scanlines, and decides when hsync and vsync
fire — the heartbeat that makes "double-speed poke" mean something more
concrete than "the `cpu_fast` bit you learned to flip in §5.5 is now
`true`." You already know exactly which register that poke sets
(`$FFD8`/`$FFD9`, §5.5) and exactly what a scanline eventually reads out
of RAM through the bus you just mastered (weeks 7–9); next week is what
ties the *rate* at which any of that happens back to a wall clock.
