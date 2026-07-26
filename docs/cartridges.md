# How CoCo Cartridges Work

Notes for implementing cartridge support in cocovm. Hardware claims are cited
against the local reference PDFs (`docs/`); file-format and banking conventions
that only exist in the emulator community are marked as such.

## Is a cartridge just an address space you can read and write to?

Mostly no — and the ways it isn't are exactly what a `Cartridge` implementation
has to model. A *plain game cartridge* is close: it is a read-only 16K window
that the GIME maps at `$C000–$FDFF`. But the cartridge connector is not a
memory socket; it is a slice of the raw MC6809 bus. A cartridge sees the full
address bus (`A0–A15`), data bus, `R/W`, and the `E`/`Q` clocks, plus two
*select* outputs (`CTS*`, `SCS*`), an *interrupt input* (`CART*`), and even
`HALT*`, `NMI*` and `SLENB*` (Service Manual p. 51, Table 3). So a cartridge
can be ROM, but it can also be an I/O device that decodes any address it
likes, asserts interrupts, halts the CPU mid-instruction (the disk controller
does), or bank-switches far more ROM than the window holds. "An address space"
is the right mental model for the *CTS window*; the rest of the connector is
what breaks the model.

## 1. The physical/electrical model

The cartridge slot is a 40-pin card-edge connector. The signals (CoCo 3
Service Manual, Table 3, p. 51):

| Pin | Signal | Description |
|----:|--------|-------------|
| 1–2 | NC | (were −12V/+12V on CoCo 1/2) |
| 3 | `HALT*` | Halt input to the CPU |
| 4 | `NMI*` | Non-maskable interrupt to the CPU |
| 5 | `RESET*` | Main reset / power-up clear |
| 6 | `E` | Main CPU clock (0.89 MHz / 1.78 MHz) |
| 7 | `Q` | Quadrature clock, leads `E` |
| 8 | `CART*` | "Interrupt input for cartridge detection" |
| 9 | +5V | 300 mA budget |
| 10–17 | `D0–D7` | Data bus |
| 18 | `R/W*` | Read/write |
| 19–31, 37–39 | `A0–A15` | Full address bus |
| 32 | `CTS*` | "Cartridge select signal" (ROM window) |
| 33–34 | GND | |
| 35 | `SND` | Analog sound input, mixed into TV audio |
| 36 | `SCS*` | "Spare select signal" (cartridge I/O) |
| 40 | `SLENB*` | "Input to disable device selection" |

Address decoding: the GIME (TCC1014, called "ACVC" in the service manual)
outputs select lines `S0–S2` into a 74LS138 (IC9) which produces `ROM*`
(internal ROM), `CTS*`, `PIA*`, and `SCS*` ("cartridge I/O"). `SLENB*` gates
the decoder — a cartridge pulling pin 40 low disables *all* internal device
selection and may answer any address itself (Service Manual p. 36, Fig. 5-6).
The manual also notes ROMs are only enabled during the `E`-high portion of a
read cycle.

### The ROM window and INIT0 MC bits

When the machine is in ROM mode (not all-RAM), `$8000–$FDFF` is ROM,
`$FE00–$FEFF` is constant RAM, and `$FF00–$FFFF` is I/O and vectors. Which ROM
answers is chosen by GIME `INIT0` (`$FF90`) bits `MC1`/`MC0` (Unravelled II,
Appendix A, `$FF90`):

| `MC1` | `MC0` | ROM mapping |
|:-:|:-:|---|
| 0 | x | 16K internal (`$8000–$BFFF`) + 16K external via `CTS*` (`$C000–$FDFF`) |
| 1 | 0 | 32K internal |
| 1 | 1 | 32K external via `CTS*` (except the vectors) |

`INIT0` bit 2 (`MC2`) is documented as "standard SCS" (Unravelled II,
Appendix A); BASIC sets it together with the `MC1=0` map when preparing to run
a cartridge ("ENABLE STANDARD SCS", Appendix I, `L8C28`). With `MC2` set,
`SCS*` is asserted for `$FF40–$FF5F` accesses. What exactly changes when
`MC2=0` is *not* spelled out in the local PDFs — I could not verify it, so
treat "SCS = `$FF40–$FF5F`" as the only mode BASIC uses.

On a CoCo 1/2, `CTS*` simply decoded `$C000–$FEFF`; the CoCo 3 GIME made the
external halves selectable via `MC1/MC0` as above, and the stock ROM copies
BASIC (including Disk BASIC, if present) into RAM at cold start and runs from
RAM (Unravelled II, pp. 29–30), so `CTS*` fetches mostly happen during boot
and when a game cart owns the machine.

## 2. Why it's not just flat readable/writable memory

**Plain carts are read-only.** A Program Pak is an EPROM on `CTS*`. Writes to
`$C000–$FDFF` go nowhere (or, in all-RAM mode, to RAM). Small ROMs typically
don't decode all address lines, so a 4K/8K image *mirrors* through the 16K
window on real hardware.

**Autostart is an interrupt, not a probe.** `CART*` (pin 8) is wired to PIA1
CB1. PIA1 control register `$FF23` bit 0 enables an FIRQ on the CART edge and
bit 7 is the latched flag (Unravelled II, Appendix A, `$FF23`; Bob Russell's
memory map p. 33 notes POKE `$FF23`,54/55 disables/enables cartridge
auto-exec). An autostarting cartridge ties `CART*` to the `Q` clock (pin 7) —
this jumper convention is documented in Tandy ROM-Pak schematics and emulator
sources, not in the local PDFs. BASIC's reset code enables the CB1 FIRQ; the
constant `Q` toggling then fires the default FIRQ handler at `$A0F6` (Bob
Russell p. 32–33). On the CoCo 3 the handler's cartridge path (`$A0FC`) calls
`L8C28`/`$8C28`, which clears the interrupt, writes `INIT0 =
COCO|MMUEN|MC3|MC2` (i.e. `MC1=0`: 16K internal + 16K external, standard SCS),
forces ROM mode, and jumps to `ROMPAK = $C000` (Unravelled II, Appendix I).
Non-autostart carts are entered manually: `EXEC &HE010` (`GOCART`) jumps to
`EXECCART` (`$A05E`), which sets up the same map and jumps to `$C000`.
The GIME also has its own CART interrupt source: `IRQENR`/`FIRQENR`
(`$FF92`/`$FF93`) bit 0 (`EI0`) triggers on the falling edge of pin 8
(Unravelled II, p. 15) — available to software, but stock BASIC autostart uses
the PIA path.

**Disk BASIC detection.** The disk controller's ROM appears at `$C000` via
`CTS*`; BASIC recognizes it by the ASCII signature `DK` (`$44 $4B`) at
`$C000/$C001` (Bob Russell, p. 32) and, on the CoCo 3, copies it to RAM.

**SCS carts expose registers.** The canonical example is the FD-50x disk
controller, which decodes `SCS*` (`$FF40–$FF5F`):

- `$FF40` `DSKREG` — write-only latch: drive selects, motor, double-density,
  write precomp, and bit 7 "halt flag" (Unravelled II, Appendix A).
- `$FF48–$FF4B` — FDC status/command, track, sector, data registers
  (mirrored at `$FF4C–$FF4F`). The command set listed in Unravelled II is the
  WD17xx set; the actual chip in the FD-502 is a WD1773 (community/MAME
  documentation — the local PDFs don't name the part).

The halt flag is the strongest counterexample to "just memory": when set, the
controller asserts `HALT*` (pin 3) to stall the CPU until the FDC raises DRQ,
turning polled sector I/O into cycle-exact hardware flow control.

**Carts can decode addresses outside SCS.** Because the full address bus is on
the connector, paks claim addresses the motherboard leaves unmapped
(`$FF60–$FF7F`): the Deluxe RS-232 Pak sits at `$FF68–$FF6B`, the
Sound/Speech Cartridge at `$FF7D–$FF7E`, and the Multi-Pak's own register at
`$FF7F` (all Unravelled II, Appendix A). The Orchestra-90 CC stereo pak has
two 8-bit DACs (XRoar manual §4.2); emulators place them at `$FF7A/$FF7B`
(MAME/XRoar convention — not verified in the local PDFs).

**Banked carts.** Some late commercial carts hold more than 16K: Tandy's
RoboCop (128K) and Predator (64K) used a "super cartridge" board with a bank
(offset) register selecting which 16K page of ROM appears in the `CTS*`
window. Community documentation (hackup.net "Bank Switching Cartridges";
CoCoFLASH user guide) describes the register as latched by *writes anywhere in
`$FF40–$FF5F`* (i.e. any SCS write). This is an emulator/community convention
reverse-engineered from the boards — no local PDF covers it.

**The Multi-Pak Interface (MPI).** Four slots; register at `$FF7F`: bits 1–0
select which slot receives `SCS*`, bits 5–4 select which slot gets `CTS*` and
`CART*` (`$FF7F` location: Unravelled II, Appendix A; bit layout: Tandy MPI
Service Manual 26-3024 / CoCopedia). Disk-BASIC-era software routinely points
SCS at the disk controller slot while CTS/CART point at a game slot.

## 3. The `.ccc` / `.rom` file formats

Both are **headerless raw dumps** of the cartridge ROM — no metadata at all.
There is no format difference between `.ccc`, `.rom`, and `.bin`; `.ccc` is
just a naming convention for Color Computer Cartridges (comp.emulators.misc;
XRoar accepts both extensions for ROM carts). Consequences:

- **Size is the metadata.** Typical sizes are 2K/4K/8K/16K; 32K images are
  either CoCo 3 "32K external" carts or banked. 64K/128K images (Predator,
  RoboCop) imply the bank-register scheme above. MAME's `coco_cart` softlist
  encodes per-title banking hints; a raw file alone can't distinguish "32K
  flat" from "2×16K banked" — emulators use size heuristics or per-title
  metadata.
- **Mirroring.** Images smaller than 16K should be mirrored (repeated) through
  the `CTS*` window, matching real carts' partial address decoding.
- **Autostart** is not encoded either. Emulators default ROM carts to
  autostart (asserting CART/FIRQ) and provide an override (e.g. XRoar's
  `-no-cart-autorun`), since a minority of carts (notably the disk controller)
  must not autostart.

## 4. Implications for cocovm

Current state (`crates/coco-core/src/cart.rs`, `bus.rs`): the `Cartridge`
trait has `read`/`write`, and `SystemBus` routes only `CART_BASE..=CART_LAST`
(`$FF40–$FF5F`) to it. `rom_read()` always serves internal ROM, with a comment
deferring the external overlay; `SystemBus::firq_asserted()` is hardwired
`false`. To support cartridges:

1. **CTS reads.** When `INIT0` has `MC1=0`, route `$C000–$FDFF` reads to the
   cartridge (and all of `$8000–$FDFF` when `MC1=1, MC0=1`). The trait's
   single `read(addr)` can serve both windows, but the cart must be able to
   distinguish a CTS access from an SCS access — either by address range
   convention or by splitting the trait into `read_cts(offset)` /
   `read_scs(addr)`. The offset-based form makes mirroring and banking
   trivial (`rom[(bank_base + offset) % rom.len()]`).
2. **SCS reads/writes** already flow through; keep them, since the FDC,
   banked carts, and the MPI all live there. Carts that decode `$FF60–$FF7F`
   (RS-232, Orchestra-90, SSC) need the bus to forward that range too instead
   of returning `OPEN_BUS`.
3. **CART line.** The trait needs a way to drive pin 8 — e.g.
   `fn cart_line(&self) -> bool` sampled by the machine, or an explicit
   "tied to Q" flag. Wire it to PIA1 CB1 (`pulse_c1` on the toggling edge,
   respecting `$FF23` bit 0/edge-polarity bits) so BASIC's `$A0F6` autostart
   path works, and to GIME `EI0` for completeness. `firq_asserted()` must then
   OR in PIA1's FIRQ output.
4. **Banking and state.** `EmptySlot`, `RomPak { rom, bank }`, and later
   `DiskController { fdc, dskreg }` — the enum-instead-of-trait-object note in
   `cart.rs` (for serde save-states) applies here.
5. **HALT (later).** The disk controller's `$FF40` bit-7 halt flag needs a
   CPU halt line; fine to defer, but don't design the trait so it can't be
   added.

## Sources

- CoCo 3 Service Manual (Tandy 26-3334): Table 3 p. 51 (connector pinout),
  Fig. 5-6 p. 36 (74LS138 decode, SLENB, E-gated ROM enables), Fig. 1-2 p. 8.
- Super Extended BASIC Unravelled II: pp. 29–30 (ROM configurations, RAM
  copy), p. 15 (GIME EI0 / pin 8), Appendix A (`$FF23`, `$FF40–$FF5F` FDC,
  `$FF68–$FF6B`, `$FF7D–$FF7F`, `$FF90` MC bits table), Appendix I (`$8C1B`,
  `$8C28`, `$A05E`, `GOCART $E010`, `ROMPAK $C000`).
- Color Computer Memory Map v2.0 (Bob Russell): pp. 32–33 (`$A0F6` FIRQ
  handler, `$FF23` autostart pokes, `DK` signature).
- Community: [hackup.net — Bank Switching Cartridges](https://www.hackup.net/2019/07/bank-switching-cartridges/),
  [CoCoFLASH User Guide](https://usermanual.wiki/Pdf/Coco20Flash20Guide.1352680095.pdf),
  [XRoar manual §4.2](https://www.6809.org.uk/xroar/doc/xroar.shtml),
  [CoCopedia — Multi-Pak Interface](https://www.cocopedia.com/wiki/index.php/Multi-Pak_Interface),
  [comp.emulators.misc on .ccc format](https://comp.emulators.misc.narkive.com/XRopwaM1/trs-80-coco-rom-format-question),
  [MAME coco_cart softlist](https://github.com/mamedev/mame/blob/master/hash/coco_cart.xml).
