# Tandy Sound/Speech Cartridge (SSC, 26-3144) Spec

Sources: MAME master (`src/devices/bus/coco/coco_ssc.cpp` — `coco_ssc_device`
and its port glue; `src/devices/cpu/tms7000/` — `tms7000_device`;
`src/devices/sound/ay8910.cpp`/`.h` — `ay8910_device`, classic AY-3-8910
mode; `src/devices/sound/sp0256.cpp` — `sp0256_device`, Joe Zbiciak's core),
Super Extended BASIC Unravelled II (SEBU) Appendix A (register addresses),
`docs/cartridges.md` ("Carts can decode addresses outside SCS"), and the
**Tandy Speech/Sound Cartridge Owner's Manual (26-3144)** Appendix A (the
host-byte command protocol). Register semantics are verified against MAME
and the manual; protocol *behaviour* is no longer specified here at all —
the cartridge's own firmware produces it, and this document records what
that firmware was observed to do where the manual is silent.

## What's modelled

The SSC is three chips plus RAM: a TMS7040 microcontroller running a 4 KB
firmware that speaks the host-byte protocol over `$FF7D`/`$FF7E`, an
SP0256-AL2 speech synthesizer, an AY-3-8913 PSG, and 2 KB of static RAM the
firmware uses for its receive queue and the eight 64-byte buffers the manual
describes. All of it is emulated:

- **TMS7040** (`crates/tms7000`, a port of MAME's core): the firmware runs
  as on the board, so text-to-speech, buffer loads and executes, sound-data
  pacing, and BUSY* handling are the real thing. See "TMS7040 firmware".
- **SP0256-AL2** (`crates/coco-core/src/sp0256.rs`, a port of MAME's
  core): allophone synthesis from the 2 KB AL2 ROM. See "SP0256-AL2".
- **AY-3-8913** (`crates/coco-core/src/ay8913.rs`): full PSG core.
- **Board glue** (`crates/coco-core/src/ssc/board.rs`): the four TMS7040
  ports wired to the RAM, the AY, the SP0256, and the host handshake
  exactly as MAME's `coco_ssc.cpp` decodes them.
- **`$FF7D`/`$FF7E`**, bus routing (including through the Multi-Pak), the
  Sound Activity Circuit, and audio mixing into `SystemBus::sound_sample`.

**Needs two ROMs**, installed as `~/.local/share/cocovm/roms/ssc-tms7040.rom`
(MAME's `pic-7040-510.bin`, CRC32 `a8e2eb98`) and `sp0256-al2.rom` (MAME's
`sp0256-al2.bin`, CRC32 `b504ac15`). The asset bundle carries both from v3
on and `ensure_assets` re-downloads it whenever a bundled ROM is missing.
The cartridge is built around the images (`SoundSpeechCartridge::new(firmware,
sp0256_rom)`): inserting one without either file is refused with a cartridge
error naming it, and snapshots record both under `media.cart_roms`. There is
no ROM-less mode — on the real board the chips are soldered in.

## The board

```text
TMS7040 port A: input  — the host byte a `$FF7E` write latched
        port B: output — A0-A7 of the 2 KB static RAM
        port C: output — bit 0 RAM A8 / AY BC1, 1 A9, 2 A10,
                         3 RAM R/W* / AY BDIR, 4 RAM CS*, 5 SP0256 ALD*,
                         6 AY CS*, 7 BUSY* to the host
        port D: bidirectional — the data bus shared by RAM, AY, SP0256
INT1 ← SP0256 load request (DRQ, which MAME's core equals to LRQ)
INT3 ← a `$FF7E` write; dropped when the firmware reads port A
```

Every strobe decodes on a port C write, in MAME's statement order: a RAM
write when CS* and R/W* are both low (address = port C bits 0-2 : port B,
data = the bus as last driven); an AY address latch when AY CS* is low with
BDIR and BC1 high, an AY data write with BDIR high and BC1 low; an SP0256
ALD strobe on ALD*'s falling edge, only for data-bus values below 64 (the
AL2 ROM's 64-entry jump table); and BUSY* released on bit 7's rising edge.
A port D read returns RAM when CS* is low and R/W* high, the AY's latched
register when AY CS* is low with BDIR low and BC1 high, else the bus as
last driven.

Clocks: the AY and the TMS7040 share one crystal at 2× the CoCo's E-clock
(MAME `DERIVED_CLOCK(2, 1)`); the TMS7040 divides it by two internally, so
it executes exactly one cycle per E-cycle. The SP0256 has its own 3.12 MHz
crystal. SEBU notes the cartridge doesn't work in double-speed (POKE 65497)
mode on real hardware; MAME ignores that, and so does this implementation
(everything simply runs 2× faster).

## `$FF7D`/`$FF7E` register semantics

Both addresses are outside the standard `$FF40-$FF5F` SCS window; the SSC
decodes them itself off the full expansion-port address bus.

### `$FF7D` — reset control

- **Read**: always `0xFF`.
- **Write**: only bit 0 is decoded.
  - `1`: asserts the SP0256's RESET pin — every write with the bit set
    resets the chip (MAME `ff7d_write`), halting whatever it was saying at
    once and raising SBY. The TMS7040 is not reset by this.
  - A **falling edge** (bit 0 was 1, now 0) resets the TMS7040 (its reset
    sequence runs on the next tick), resets the AY, and clears BUSY*. The
    host-byte latch and INT3 are *not* cleared: a byte written just before
    the reset is read by the rebooted firmware, as in MAME.
  - The line is primed high at power-on (MAME `m_reset_line = 1`), so the
    manual's `POKE 1 : POKE 0` works even as the very first access, and a
    lone first write of 0 is itself a falling edge.

### `$FF7E` — host command latch (write) / status (read)

- **Write**: latches the byte into port A, sets BUSY*, asserts INT3. The
  latch is unconditional: a byte the firmware hasn't read yet is
  overwritten. That is the manual's "if you try to transfer data while bit 7
  is low, you lose all the data you send" (p.10) — data is lost by being
  overwritten, not refused.
- **Read**: bits 4-0 always read set (`0x1F`; MAME's real-hardware trace
  shows them pulled high). Then:
  - **bit 7**: BUSY*, **1 = not busy**. Cleared by the write, set again when
    the firmware raises port C bit 7. Observed: the firmware's INT3 handler
    queues the byte and releases BUSY* about 370 cycles after the write; it
    acts on the byte from its main loop later (~1,200 cycles for a simple
    byte, more for a burst — a queue of ten bytes drains in ~25,000). This is
    the manual's note that "the speech and sound status bits are not valid
    immediately following a speech or sound execution command".
  - **bit 6**: SP0256 SBY, 1 = idle, straight from the chip.
  - **bit 5**: Sound Activity Circuit, **1 = quiet, 0 = sound playing**.

### Corrections to a naive reading

- **SAC bit 5 is inverted**: MAME `coco_ssc_device` returns `!sound_active`;
  `crate::ssc::status::QUIET` names the bit for what it reports.
- **Busy has no register-level clear**: nothing the host writes clears it;
  only the firmware's port C toggle does. The old interpreter's fixed
  `BUSY_HOLD_CYCLES` is gone.
- **The latch never discards**: see the `$FF7E` write above; the old
  interpreter refused bytes while busy, which was the opposite of MAME.

## Host byte protocol (26-3144 Appendix A)

The firmware interprets it; `crates/coco-core/src/ssc/commands.rs` only
names the constants so tests can build streams. Byte ranges, `N = byte -
range start`:

| Byte(s) | Meaning |
|---|---|
| `$00` | Stop all sound and speech (buffers kept) |
| `$01-$7F` | Text: spoken when `$0D` arrives (default mode) |
| `$80-$87` / `$90-$97` | LOAD speech string, buffers `N..=7` / `N` only, terminator `$0D` |
| `$88-$8E` / `$98-$9F` | LOAD sound data, terminator `$FF` |
| `$8F` | LOAD timer base (one postbyte) |
| `$A0-$A7` / `$B0-$B7` | LOAD allophone stream, terminator `$FF` |
| `$A8-$AE` / `$B8-$BF` | LOAD register string, terminator `$FF` |
| `$AF` | Direct AY access: `(register, value)` pairs until `$FF` where a register is expected |
| `$C0-$C6` / `$D0-$D7` | EXECUTE speech string (text-to-speech) |
| `$C7` | Abort all speech |
| `$C8-$CE` / `$D8-$DF` | EXECUTE sound data |
| `$CF` | Stop all sound |
| `$E0-$E7` / `$F0-$F7` | EXECUTE allophone stream |
| `$E8-$EF` / `$F8-$FF` | EXECUTE register string |

Buffers are 8 × 64 bytes; "consecutive" loads spill from buffer `N` into
the following ones. Sound-data events are 4-byte tone / envelope groups and
3-byte noise groups (`commands::group`), paced by the timer base.

### Observed firmware behaviour (not in the manual)

Facts established by running the firmware; they are what the tests assert
and are the same in MAME by construction.

- **Where things live in RAM**: buffer 0 starts at `$200` of the 2 KB RAM
  (buffer `N` at `$200 + 64N`); the receive queue starts at `$400`.
- **Text mode**: bytes below `$80` in the default mode accumulate as text
  until `$0D`, which speaks them; a command byte (`$80` and up) is honoured
  even between text bytes.
- **Direct access `$FF`**: `$FF` where a *value* is expected is a plain
  value; only `$FF` where a register number is expected ends the mode.
- **Sound-data durations** (tone A, duration byte `D`, measured at the AY's
  volume register, 894,886 cycles/s): at the power-on timer base, `D = 10`
  lasts 708,800 cycles and `D = 30` 2,029,960 — about 66,000 cycles per
  duration unit plus ~48,000 of overhead, i.e. roughly one timer-1 period
  (the firmware runs timer 1 with prescaler 31, an INT2 every 65,536
  cycles). With `$8F` bases 8/16/32 and `D = 10`: 26,816 / 50,240 / 94,496
  cycles. **Cross-checked against MAME**: the same `D = 100` event sounds
  for 7.36 s in a MAME `-wavwrite` capture and 7.38 s here.
- **After `$00`**: the AY channel volumes are written to 0.
- **Text-to-speech** of `"I CAN TALK "` + `$0D` speaks for well under a
  second after a short conversion delay; the manual's demo works as printed.

## TMS7040 firmware (`crates/tms7000`, `crates/coco-core/src/ssc.rs`)

The core is a port of MAME's `tms7000` (register file, peripheral file with
IOCNT0 / timer 1 / ports A-D, INT1/INT2/INT3 with MAME's level-tracked
flags, MAME's cycle costs and reset quirks). Verified instruction for
instruction and cycle for cycle against MAME's `pic7040` debugger trace over
its whole 580,000-instruction capture, from reset through the host's first
byte and its INT3 handling (`scripts/ssc-trace-diff.py`); the only
differences are one-instruction shifts at timer interrupts, where MAME's
fractional timeslicing moves its timer phase by up to a cycle.

- **Stepping**: `Cartridge::tick(cycles)` adds the E-cycles to a budget and
  runs whole instructions (or interrupt entries, or the reset sequence)
  while it is positive, carrying the overshoot into the next tick. The
  firmware runs first, then the AY and SP0256 step over the same cycles, so
  strobes issued within a tick land before the chips advance. INT3 is
  re-synced after every step (the firmware's port A read drops it); INT1
  follows the SP0256's load request, sampled after each step and again after
  the chip steps.
- **Reset**: MAME's `device_reset` writes ports B, C and D through the
  peripheral file *before* clearing state, with DDR C/D at 0, so the board
  sees port B all ones and ports C/D all zeros (one RAM write and BUSY* low);
  then TRAP 0 vectors through `$FFFE`, pushing the old PC into R0/R1. The
  core reproduces this; a `$FF7D` falling edge or a machine reset queues it
  for the next tick (`TMS7040::assert_reset`).
- **Firmware facts**: vectors RESET `$F000`, INT1 `$F21C`, INT2 `$F033`,
  INT3 `$F012`; boot sets SP to `$4A`, IOCNT0 to `$3C` (INT2 and INT3
  enabled, INT1 off), DDR C to `$FF`, timer 1 to `T1DATA = $FF`, `T1CTL =
  $9F`; its idle loop ORs bit 4 into IOCNT0 every ~1,600 cycles (which, as a
  write-1-clears, also drops a pending INT2 flag — so the first timer
  interrupt the firmware actually takes depends on that phase, in MAME
  too). No illegal opcode is executed.
- **Tracing**: `SoundSpeechCartridge::enable_firmware_trace` /
  `drain_firmware_trace` record instructions; `cargo run -p coco-core
  --example ssc_trace` prints them in MAME's tracelog format.

## SP0256-AL2 (`crates/coco-core/src/sp0256.rs`)

GI's "Narrator" speech processor with the 2 KB allophone mask ROM: a
microsequencer walks bit-packed instructions in that ROM (LSB-first, at a
bit-granular PC) to load a 12-pole LPC lattice filter — six cascaded
second-order stages excited by a periodic impulse train (voiced) or a 15-bit
LFSR (noise) — and re-runs after each frame's repeat count expires. The
port mirrors MAME `sp0256.cpp`: opcode semantics, operand-block layout
tables, quantization table, wrapping 16-bit filter arithmetic,
`HIGH_QUALITY` limiter, `PER_PAUSE`/`PER_NOISE`. The SPB640 FIFO is omitted
(nothing on the SSC drives it); a `STEP_BUDGET` caps instructions per
sequencer run so a garbage ROM can't hang the emulator.

- **ROM**: 2 KB at chip address `$1000` (MAME `ROM_LOAD(..., 0x1000, ...)`),
  the first 128 bytes a 64-entry jump table; ALD value `n` lands at byte
  `$1000 + 2n`. Reads outside the image return 0 = RTS/HLT.
- **Clocking**: 3.12 MHz crystal, one output sample per 312 clocks = 10 kHz.
  `SP0256::step(e_cycles)` accrues samples at the fixed ratio 10 000 /
  894 886 per E-cycle, so its status lines advance deterministically even
  with no audio device draining output.
- **Output**: linearly interpolated between the last two samples — a cheap
  stand-in for the board's RC low-pass — and mixed into the cartridge's
  mux-10 line at `SPEECH_GAIN = 1.75 / 2.0` relative to the PSG (MAME
  `SP0256_GAIN`/`AY8913_GAIN`), **after** the Sound Activity Circuit tap:
  MAME routes only the AY through the SAC, so speech never clears bit 5.
- **Handshake**: `ald_write` is dropped while LRQ is low; LRQ rises again as
  soon as the sequencer picks the command up, so one allophone queues behind
  the one playing — the firmware services that through INT1. SBY drops on
  `ald_write` and rises when the sequencer halts with nothing latched, at
  the next period boundary (up to 64 samples, ~7 ms, after the last
  instruction), same as MAME.
- **Trailing pause**: the sequencer keeps re-exciting the last frame after
  it halts; only a PAUSE (`PA1`-`PA5`) zeroes it. That is the manual's own
  instruction ("You must end allophone data with a pause") and MAME's
  behaviour.
- **Timing vs Appendix C**: measured ALD-to-SBY durations (the golden table
  in `sp0256_test.rs`) run 25-40 % shorter than the manual's nominal
  per-allophone durations, which are garbled in places (`/OY/` "42 ms"). MAME
  speaking the manual's page-13 "Color Computer" stream spans ~320 ms and
  ~990 ms for the two words with a ~680 ms pause; this port ~300, ~905 and
  ~640 ms. The core matches its source; the datasheet figures are not what
  the core produces.

## AY-3-8913 core (`crates/coco-core/src/ay8913.rs`)

The AY-3-8913 is an AY-3-8910 PSG with the two I/O ports (registers 14/15)
absent — no pins on the package. `AY8913` mirrors MAME `ay8910.cpp`'s classic
(non-AY8930-expanded, non-YM2149) mode:

- **Registers**: R0-R5 tone A/B/C fine/coarse (12-bit combined period; coarse
  registers R1/R3/R5 masked to 4 bits at write time). R6 noise period
  (5-bit). R7 mixer: bits 0-2 tone disable A/B/C, bits 3-5 noise disable
  A/B/C, both *active-low enable*; bits 6-7 ignored. R8-R10 channel volumes:
  bits 0-3 fixed level, bit 4 selects envelope mode. R11/R12 envelope period
  (16-bit). R13 envelope shape — writing it always restarts the envelope.
  The bus-side address latch (`write_address`/`write_data`/`read_data`)
  decodes 4 bits, as MAME's `address_w`.
- **Internal step clock** = master clock / 8 (MAME `stream_alloc(0,
  m_streams, master_clock / 8)`). `Ay8913::step` accumulates a
  fractional-clock remainder across calls.
- **Tone generators**: classic mode reduces MAME's duty-cycle down-counter to
  a plain toggle every `period` internal steps (clamped to at least 1).
- **Noise**: a 17-bit LFSR, feedback = bit 0 XOR bit 3 shifted into bit 16,
  output = bit 0, with MAME's second prescaler halving the noise period.
  Seeded to 1 at power-on/reset.
- **Envelope**: 16 levels (`ENV_STEP_MASK = 0x0F`), 2 internal steps per
  level on the classic AY-3-8910. Shape decode mirrors MAME's
  `envelope_t::set_shape`/pacing loop exactly.
- **Volume DAC table**: MAME's resistor-network formula from Matthew
  Westcott's ZX Spectrum measurements (`ay8910_param`), min-max normalized
  to `[0.0, 1.0]` instead of MAME's legacy `-0.25 * 0.5` rescale.
- **Mixing (deliberate deviation from MAME)**: real AY output mixing runs
  all three channels through one shared resistor network (MAME's `mix_3D`
  table). This implementation sums the three channels' already-gated,
  already-DAC'd levels and divides by three (`SINGLE_OUTPUT` style) — not
  bit-accurate against a chip analyzer capture, good enough for a sound
  cartridge.
- **Output/downsampling**: `Ay8913::drain` returns the average of the mixed
  per-internal-step output since the last call — a box-filter downsample
  from the ~223.7 kHz internal-step rate to whatever rate the caller drains at.

## Audio integration

`SystemBus::sound_sample` point-samples the speaker level per audio-grid
slot — see `crates/coco-core/src/bus.rs`. The mux's cartridge-input arm
(SEL2:SEL1 = 10) calls `Cartridge::audio_sample` and mixes the result in
with `CARTRIDGE_GAIN = 0.75`, matched to the 6-bit DAC's own gain.

`Cartridge::audio_sample` is called **exactly once per `sound_sample`
invocation, regardless of mux selection**. `Ssc::audio_sample` drains the
AY, feeds the Sound Activity Circuit unconditionally (bit 5 must reflect the
cartridge's own output even while the CoCo's speaker is listening to the DAC
or cassette), then adds the SP0256's interpolated output.

### Sound Activity Circuit (SAC)

An envelope follower on the AY's output, purely so `$FF7E` bit 5 can report
activity. Per MAME `coco_ssc.cpp`: a one-pole DC-blocking high-pass filter
(`y = 0.99 * (y_prev + x - x_prev)`), rectification through an asymmetric
leaky integrator (attack `0.0026`, decay `0.0003`), and hysteresis
(`sound_active` on above `0.05`, off below `0.01`). MAME runs this per
host-audio-sample; this implementation runs it per `audio_sample` call,
close enough in rate that the coefficients serve unmodified.

Because `Ay8913::drain`'s box filter averages over a slot's worth of internal
steps, only a tone slow enough that consecutive drained samples swing
between near-silent and near-full-scale registers as activity — true of any
audible-range tone, worth knowing when writing a test (see
`tests/ssc/audio_sac.rs`'s `TEST_TONE_PERIOD`).

## Bus routing

`$FF40-$FF5F` is the "standard" SCS* window; `$FF60-$FF7E` is
motherboard-unmapped, but the full address bus reaches the expansion
connector, so cartridges decode registers there too (`docs/cartridges.md`).
`SystemBus` routes the whole `$FF40-$FF7E` band to `Cartridge::read`/`write`;
`$FF7F` stays carved out for the Multi-Pak's own select register.

**Multi-Pak Interface forwarding**: the MPI only switches SCS*/CTS*/CART*
between slots — the address and data buses are common. `$FF40-$FF5F` goes
only to the SCS-selected slot, but `$FF60-$FF7E` is broadcast to **every**
slot on write, and a read returns the first non-open-bus response. An SSC in
a non-SCS-selected slot still receives its `$FF7D`/`$FF7E` traffic
(`tests/ssc/bus_routing.rs`).

## Snapshots

The cartridge serializes its chips and board latches; neither ROM image is
carried. `media.cart_roms` records two `SlotROMRef`s for an SSC slot,
distinguished by `role` (`Primary` = the SP0256-AL2 ROM, `SSCFirmware` =
the TMS7040 firmware), and restore reattaches both through the core's
cart-ROM step. A snapshot from before the firmware was emulated carries only
the `Primary` ref and none of the `tms`/`board` fields: those default to a
chip with its reset pending, the frontend supplies the installed firmware
with a warning, and the firmware boots on the first tick — mid-command state
from the old interpreter is not carried over. A budget outside what one
TMS7040 step can leave is rejected as an invalid payload.

## Deferred

- **Debugger integration**: `crates/coco-core/src/debug.rs` and the
  frontend's debugger panes are 6809-only. Firmware inspection goes through
  `SoundSpeechCartridge::firmware()` and the trace hook.
