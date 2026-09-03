# Tandy Sound/Speech Cartridge (SSC, 26-3143) Spec

Sources: MAME master (`src/devices/bus/coco/coco_ssc.cpp` — `coco_ssc_device`;
`src/devices/sound/ay8910.cpp`/`.h` — `ay8910_device`, classic AY-3-8910
mode; `src/devices/sound/sp0256.cpp` — `sp0256_device`, Joe Zbiciak's
core), Super Extended BASIC Unravelled II (SEBU) Appendix A (register
addresses), `docs/cartridges.md` ("Carts can decode addresses outside SCS"),
and the **Tandy Speech/Sound Cartridge Owner's Manual (26-3144)** Appendix A
(the host-byte command protocol itself — see "Host byte protocol" below).
All register-semantics claims below were verified against MAME source or the
26-3144 manual before implementation; three MAME corrections to a naive
reading are called out explicitly, and every place the 26-3144 manual is
silent or ambiguous is called out as a judgment call, not a verified fact.

## What's modelled vs deferred

The SSC is really three chips: a TMS7040 microcontroller running firmware
that speaks a host-byte command protocol over the CoCo's `$FF7D`/`$FF7E`
handshake (documented in 26-3144 Appendix A), an SP0256-AL2 speech
synthesizer the firmware drives, and an AY-3-8913 PSG the firmware also
drives for music/sound effects.

- **Implemented**: the `$FF7D`/`$FF7E` handshake register semantics (busy
  flag, status byte, SP0256 reset, SP0256-reset-edge-triggered AY reset,
  busy-lost byte discarding), a full AY-3-8913 core, an SP0256-AL2 core
  (`crates/coco-core/src/sp0256.rs`, see "SP0256-AL2 speech" below), and
  the 26-3144 host-byte protocol minus text-to-speech: buffer-RAM
  LOAD/EXECUTE for sound data, register strings and allophone streams,
  `$AF` direct AY register access, the sequential sound-data playback
  engine that drives the AY from a timer-scheduled event stream, and the
  allophone feeder that drives the SP0256. Also implemented: bus routing
  (including through the Multi-Pak Interface) and audio mixing into
  `SystemBus::sound_sample`.
- **NOT implemented**: the TMS7040 CPU core itself (this crate interprets the
  protocol directly rather than emulating the microcontroller that runs it)
  and, with it, the firmware's ROM-based English text-to-speech rules.
  Speech-string (ASCII) command bytes are parsed just enough to keep the
  protocol state machine in sync — their LOAD variants still fill buffer
  RAM per the flat-RAM model below — but their EXECUTE variants are no-ops:
  there is no text-to-allophone converter to run them through. See
  "Deferred" at the end.
- **Needs a ROM**: the SP0256-AL2's 2 KB allophone mask ROM,
  `~/.local/share/cocovm/roms/sp0256-al2.rom` (MAME's `sp0256-al2.bin` from
  its `coco_ssc` set, SHA-1 `e60fcb5fa16ff3f3b69d36c7a6e955744d3feafc`,
  renamed). The asset bundle carries it from v3 on, and `ensure_assets`
  re-downloads the bundle whenever any bundled ROM is missing, so older
  installs catch up at launch. The cartridge is built around the image
  (`SoundSpeechCartridge::new(rom)`): inserting one without the file is
  refused with a cartridge error naming it, and snapshots record it under
  `media.cart_roms` like every other cart ROM. There is no ROM-less mode —
  on the real board the chip is soldered in.

**Why the TMS7040 itself isn't emulated, but its protocol now is**: on real
hardware, a byte written to `$FF7E` isn't itself a documented opcode from the
CPU's perspective — it's a byte the TMS7040's *firmware* reads over a port
and interprets. But the 26-3144 manual's Appendix A documents that firmware's
command protocol directly (command byte ranges, buffer LOAD/EXECUTE
semantics, the sound-data event format), so this implementation interprets
the protocol in `crate::ssc::Ssc::dispatch` without needing to emulate the
TMS7040 CPU that would otherwise run it. `Ssc::ay_write`/`Ssc::ay_read` (and
the `Cart::as_ssc` accessor that reaches them through the cartridge enum) remain available directly for tests/debugging, bypassing the
protocol entirely.

## `$FF7D`/`$FF7E` register semantics

Both addresses are outside the standard `$FF40-$FF5F` SCS window; the SSC
decodes them itself off the full expansion-port address bus (`$FF60-$FF7E` is
motherboard-unmapped, not the disk controller's SCS window — see
`docs/cartridges.md`).

### `$FF7D` — SP0256 reset control

- **Read**: always `0xFF`, unconditionally.
- **Write**: only bit 0 is decoded.
  - `1`: asserts the SP0256's RESET pin — every write with the bit set
    resets the chip (MAME `coco_ssc_device::ff7d_write`), halting whatever
    it was saying at once and raising SBY. The allophone-stream cursor is
    firmware state and is *not* touched: on the next tick it hands the chip
    the rest of the stream.
  - A **falling edge** (previous write had bit 0 = 1, this write has bit 0 =
    0) resets the AY-3-8913 (all registers and internal generator state) and
    forces `busy` to `false` — this is MAME `coco_ssc_device`'s modelled
    behavior for the SP0256 reset line, not an emulator convenience we added.
  - Any other transition (bit 0 stays 0, or goes 0→1) does nothing to the AY.
  - Edge detection needs a "previous bit 0" to compare against; `SoundSpeechCartridge` inits
    it to `false` at power-on/reset so the very first `$FF7D` write — even if
    it happens to be bit 0 = 0 — is never itself treated as a falling edge.

### `$FF7E` — host command latch (write) / status (read)

- **Write**: if `busy` is already set, **the byte is discarded entirely** —
  not latched into `Ssc::host_latch`, does not restart the busy-hold window,
  never reaches the protocol state machine. This is the manual's own stated
  behavior (26-3144 p.10): "If you try to transfer data to the S/SC while bit
  7 is low, you lose all the data you send until the bit resets." Previously
  this implementation always latched the byte regardless of busy; that was
  wrong and has been corrected (see "Busy-lost bytes" below).

  Otherwise: latches the byte into `Ssc::host_latch` (the "Port A latch" on
  real hardware), sets `busy = true`, and processes the byte synchronously
  through the host-byte protocol state machine (see "Host byte protocol"
  below) — every accepted byte goes through this uniformly, whether it's a
  top-level command, buffer-load data, a `$8F` timer-base postbyte, or a
  `$AF` direct-access register/value byte. Real hardware also asserts the
  TMS7000's INT3 line here, waking the firmware to consume the byte
  asynchronously — not modelled (no TMS7000 core); this implementation
  interprets the byte immediately instead.
- **Read**: a status byte, bits 4-0 always read set (`0x1F`) — MAME's
  real-hardware trace shows these pulled high; their individual meanings (if
  any) aren't documented. On top of that base:
  - **bit 7**: busy/ready, **1 = not busy**, **0 = busy**. Set immediately
    after a `$FF7E` write, cleared by [the synthetic busy-hold
    mechanism](#busy-clearing-a-synthetic-hold) below (or by the `$FF7D`
    falling-edge reset, or by `Ssc::reset`).
  - **bit 6**: SP0256 SBY ("standby" = idle/ready), read straight off the
    chip (MAME `m_spo->sby_r()`): it drops the moment an allophone is
    latched and rises when the sequencer halts with nothing queued. The
    execute-speech-string commands (`$C0`-`$C6`, `$D0`-`$D7`) are no-ops
    (see "Host byte protocol") and never clear it.
  - **bit 5**: Sound Activity Circuit output, **1 = quiet, 0 = sound
    playing** — see [SAC bit 5 is inverted](#sac-bit-5-is-inverted) below.
    Unchanged by this revision: it already tracks the AY's real output
    correctly, including output now driven by the sound-data engine.

## Two corrections to a naive reading

A naive reading of "SSC has a busy flag and a sound-activity flag" gets each
of these wrong; both are cited directly against MAME's
`coco_ssc_device`:

### SAC bit 5 is inverted

It would be natural to guess bit 5 = 1 means "sound is playing" (matching bit
7's "busy = 0 while ready, wait no — actually busy = 0 means NOT busy"
polarity confusion is exactly the trap here). MAME's `coco_ssc_device`
literally returns `!sound_active` into that bit position: **1 means quiet,
0 means sound is playing**. `crate::ssc::status::QUIET` names the bit for
what it actually reports, not what it's usually assumed to mean.

### Busy clearing has no register-level trigger

The real hardware doesn't clear `busy` via any documented `$FF7D`/`$FF7E`
write pattern — the TMS7040 firmware clears it (some port-bit toggle
internal to the firmware, not exposed on the bus) once it has finished
processing the host byte, on its own schedule. Since no firmware runs here,
there's nothing to "finish processing" on — see the next section for the
synthetic stand-in.

## Synthetic choices (not hardware facts)

### Busy-clearing: a synthetic hold

Since real busy-clearing depends on firmware timing this implementation
doesn't have, `Ssc::tick` instead counts down a fixed hold window after every
`$FF7E` write:

```rust
/// Synthetic hold time for `busy` after a `$FF7E` write, in E-clock cycles.
/// Not a hardware fact -- see the module doc comment.
const BUSY_HOLD_CYCLES: u32 = 100;
```

100 E-clock cycles (~112 µs at the normal-speed clock) is long enough that a
driver polling the status byte in a tight loop observes a genuine busy period,
short enough not to stall anything. Busy also clears immediately on the
`$FF7D` falling-edge AY reset and on `Ssc::reset`.

### Busy-lost bytes

Covered above under "`$FF7E` — host command latch": while `busy` is set, an
incoming `$FF7E` write is discarded outright, per 26-3144 p.10. This is a
verified manual fact, not a judgment call.

### Sound-data event duration: an invented formula — **NOT A HARDWARE FACT**

**There is no documented formula, units, or worked example anywhere in the
26-3144 manual for how the `$8F` timer-base value and a sound-data event's
duration byte combine into real elapsed time.** The manual states only that,
independently for each value, 0 is shortest and 255 is longest — no formula,
no units (milliseconds? frames? PSG cycles?), no worked example. This was
searched for exhaustively and is a genuine, flagged gap, not an oversight.
Exactly like [`BUSY_HOLD_CYCLES`](#busy-clearing-a-synthetic-hold) above,
`crate::ssc::timing` invents a stand-in, tuned only to produce audible,
plausible note/effect durations:

```rust
pub mod timing {
    /// `duration_cycles = duration_byte * (timer_base + 1) * CYCLES_PER_DURATION_UNIT`.
    pub const CYCLES_PER_DURATION_UNIT: u32 = 400;
    /// No documented power-on/reset default for the timer-base register.
    /// Chosen as a mid-range value so software that never sends `$8F` still
    /// gets audible, non-instant durations. Not a hardware fact.
    pub const DEFAULT_TIMER_BASE: u8 = 32;

    pub fn duration_cycles(duration_byte: u8, timer_base: u8) -> u32 {
        u32::from(duration_byte) * (u32::from(timer_base) + 1) * CYCLES_PER_DURATION_UNIT
    }
}
```

`timer_base + 1` avoids a permanently-zero-duration engine if software never
sends `$8F`; `duration_byte` is deliberately NOT offset by 1 — a duration of
exactly 0 legitimately means "instant", consistent with the manual's own
"place a silence event with duration 0 at the end" idiom. This function is
`pub` specifically so tests (and any future caller) compute expected cycle
counts from the same formula instead of duplicating the arithmetic — see
`crates/coco-core/tests/ssc.rs`'s `timer_base_scales_sound_event_duration`
and `tone_plus_envelope_pair_uses_the_envelope_groups_own_duration` tests.

### AY clock = 2× the CoCo E-clock

The AY-3-8913 and the (unemulated) TMS7040 CPU share one crystal on the real
cartridge, both clocked at twice the CoCo bus's E-clock rate (NTSC ≈
1,789,773 Hz). `Ssc::tick(cycles)` advances the AY by `cycles *
AY_CLOCK_MULTIPLIER` (`= 2`) master clocks. The TMS7040 side of that shared
clock is irrelevant here since no TMS7040 is emulated.

### Double-speed caveat not modelled

SEB Unravelled II notes the SSC doesn't work in double-speed (POKE 65497)
mode on real hardware — a bus-timing artifact of the real board, not a
register semantic. MAME ignores it; so does this implementation.

## Host byte protocol — `crates/coco-core/src/ssc.rs`

Source: Tandy Speech/Sound Cartridge Owner's Manual (26-3144), Appendix A.
Every command byte/range below was verified exhaustively against the
manual's Appendix A table; the ranges are disjoint and exhaustive over
`0x00, 0x80-0xFF`. `0x01-0x7F` (bit 7 clear) is plain ASCII text-to-speech
data in the default input mode — consumed and discarded, including `0x0D`
arriving in this mode (it has no special effect here since nothing
accumulates or speaks it).

### Command table

| Byte(s) | Meaning | Notes |
|---|---|---|
| `$00` | Stop all sound and speech | does NOT clear buffer RAM; `$CF` plus `$C7` |
| `$80-$87` | LOAD speech string, buffers `N..=7` | terminator `$0D`; buffer-fill only, no-op content |
| `$88-$8E` | LOAD sound data, buffers `N..=7` | terminator `$FF` |
| `$8F` | LOAD timer base (1 postbyte, 0-255) | NOT a buffer write — see timer-base timing below |
| `$90-$97` | LOAD speech string, buffer `N` only | terminator `$0D` |
| `$98-$9F` | LOAD sound data, buffer `N` only | terminator `$FF` |
| `$A0-$A7` | LOAD allophone stream, buffers `N..=7` | terminator `$FF` |
| `$A8-$AE` | LOAD register string, buffers `N..=7` | terminator `$FF` |
| `$AF` | Toggle `$AF` direct-access mode | see below |
| `$B0-$B7` | LOAD allophone stream, buffer `N` only | terminator `$FF` |
| `$B8-$BF` | LOAD register string, buffer `N` only | terminator `$FF` |
| `$C0-$C6` | EXECUTE speech string, buffers `N..=7` | **no-op**: text-to-speech lives in the unemulated firmware |
| `$C7` | Abort all speech | stops feeding the SP0256; the latched allophone plays out |
| `$C8-$CE` | EXECUTE sound data, buffers `N..=7` | runs the sound engine |
| `$CF` | Stop all sound | sound only; speech continues |
| `$D0-$D7` | EXECUTE speech string, buffer `N` only | **no-op**, as `$C0-$C6` |
| `$D8-$DF` | EXECUTE sound data, buffer `N` only | runs the sound engine |
| `$E0-$E7` | EXECUTE allophone stream, buffers `N..=7` | feeds the SP0256 (see "SP0256-AL2 speech") |
| `$E8-$EF` | EXECUTE register string, buffers `N..=7` | writes `(reg,val)` pairs straight to the AY |
| `$F0-$F7` | EXECUTE allophone stream, buffer `N` only | feeds the SP0256 |
| `$F8-$FF` | EXECUTE register string, buffer `N` only | writes `(reg,val)` pairs straight to the AY |

`N` for every range above is `byte - <range start>`. Note the asymmetric
widths: `$80-$87`/`$90-$97` and their allophone/register-string counterparts
are full 8-wide (`N` 0-7), but `$88-$8E`/`$A8-$AE`/`$C0-$C6`/`$C8-$CE` are
only 7-wide because `$8F`/`$AF`/`$C7`/`$CF` carve a single byte out of what
would otherwise be an 8-wide range for a different purpose (timer-base load,
direct-access toggle, abort-speech, stop-sound respectively).

### Buffer RAM (`crate::ssc::ram`)

Flat 512-byte RAM = 8 buffers × 64 bytes (`ram::BUFFER_COUNT *
ram::BUFFER_SIZE`), buffer `N` at offset `N*64..(N+1)*64`. Buffer contents
are untyped raw bytes: whatever LOAD command filled a buffer, any later
EXECUTE reads it back the same way, with no cross-checking of "was this
loaded as sound data".

- **Consecutive** loads/executes (`N..=7` variants) span from buffer `N`'s
  start offset through the end of all RAM (`ram::SIZE`) — they can spill
  into later buffers.
- **Individual** loads/executes (buffer `N` only) are confined to
  `N*64..(N+1)*64`.
- **LOAD state machine** (byte-by-byte, group-*unaware*): tracks
  `(terminator, cursor, cap)`. If the incoming byte equals the terminator,
  the load ends (byte not stored). Else if `cursor >= cap`, the load ends
  *without* storing this byte, and the byte is immediately re-dispatched as
  if freshly received at the top level (the manual: the protocol "reverts to
  normal input mode"). Else the byte is stored and `cursor` advances.
  Crucially, this scan is flat and has **no notion of sound-data group
  boundaries** — an `$FF` value anywhere in a sound-data byte stream (even
  mid-group, e.g. as a duration byte) ends the LOAD right there. This is
  different from EXECUTE-time scanning (below), which only checks for the
  terminator at group-start positions.

#### RAM prefill to `$FF`, not zero — **judgment call**

The manual doesn't state a reset/power-on fill value for buffer RAM. This
implementation prefills to `RAM_RESET_BYTE = 0xFF` (same as
`terminator::SOUND`) — inferred as the only value consistent with how
EXECUTE works: EXECUTE keeps **no separate "how many bytes did the LOAD
actually write" bookkeeping**. It just re-scans from the buffer's start,
stopping at its own terminator/incomplete-group condition. Since `$FF` is
also the untouched-since-reset RAM value, an EXECUTE naturally stops exactly
where a shorter-than-capacity LOAD stopped, with no extra state needed. If
RAM were zero-filled instead, an unwritten byte one past a short sound-data
load would misparse as a spurious `$00`-opcode (tone A, amplitude 0) group
and corrupt playback — verified against a load+execute test scenario
requiring the `0xFF` fill to behave correctly.

**Known unhandled edge case, not worth engineering around**: re-loading a
*shorter* stream into a buffer region previously filled by a *longer* one can
leave genuine stale non-terminator bytes just past the new cursor (from the
earlier, longer load), which a later EXECUTE could misparse as real data.
Not fixed — buffers are cheap to reset via a full reload or `$FF7D`
falling-edge/`Ssc::reset` in practice.

### Sound-data event format (`crate::ssc::group`)

Each group's first byte: bits 7-5 = 3-bit opcode, bit 4 = M (envelope-mode
flag; unused for envelope's own first byte, where bits 3-0 are shape bits
S3-S0 instead), bits 3-0 = amplitude (tone/noise) or shape.

| opcode (bits 7-5) | Meaning | Length |
|---|---|---|
| `000` | Tone A | 4 |
| `001` | Tone B | 4 |
| `010` | Tone C | 4 |
| `011` | Envelope (low encoding) | 4 |
| `100` | Noise A | 3 |
| `101` | Noise B | 3 |
| `110` | Noise C | 3 |
| `111` | Envelope (high encoding) | 4 |

**Tone group** (`[op|M|amp, coarse, fine, duration]`): `coarse & 0x0F` →
`TONE_x_COARSE`; `fine` (full 8 bits) → `TONE_x_FINE`; `amp | (M ? 0x10 :
0)` → `VOL_x` (matches the AY's own R8-R10 bit layout). Updates
`last_amplitude_nibble` to this group's raw `amp` unconditionally (tone
groups have no reuse flag).

**Noise group** (`[op|M|amp, R|period, duration]`): `period & 0x1F` →
`NOISE_PERIOD`. Bit 7 of byte 1 is the "R" reuse flag (manual, quoted: "If
this bit is low, the last 4 bits of the previous byte determine amplitude...
If this bit is set, however, the amplitude of the preceding data group is
used, and the amplitude bits in the first byte are ignored"). Implemented as
`last_amplitude_nibble`, tracked across groups and reset to 0 at the start of
every EXECUTE-sound-data command; updated after every group that does NOT set
the reuse flag.

**Envelope group** (`[op|shape, coarse, fine, duration]`), only ever appears
per the manual immediately after a tone/noise group with M=1: `shape &
0x0F` → `ENV_SHAPE`; `coarse` → `ENV_COARSE`; `fine` → `ENV_FINE`.

**Terminator rule for sound-data groups at EXECUTE time**: unlike the flat
LOAD scanner above, EXECUTE's re-scan only checks for `$FF` at
group-*start* positions (i.e., wherever it's about to read a fresh opcode
byte) — never at arbitrary offsets within a group. This falls out naturally
from `Ssc::advance_engine` only ever checking the byte at `Engine::cursor`
before parsing a group, never mid-group. Same principle applies to
register-string EXECUTE (`Ssc::execute_register_string`): `$FF` only ends the
stream when it appears where a fresh register byte is expected, not as a
value byte.

**"Last incomplete event never executes"**: not special-cased at LOAD time.
At EXECUTE time, when about to parse a group at the current cursor, if fewer
than the group's required byte count remain before the EXECUTE-time cap, the
engine stops there — that partial group is never parsed or programmed to the
AY. Covers both a genuinely truncated LOAD and a group that happens to
straddle a buffer's fixed capacity boundary.

#### Judgment calls (not literally stated by the manual)

- **Mixer auto-enable**: processing a tone/noise group ensures that
  channel's mixer bits in AY register 7 (`ay8913::mixer` — bumped to
  `pub(crate)` so `ssc.rs` can reuse the same named shift constants) have the
  matching generator enabled and the other disabled, read-modify-write, only
  touching that one channel's 2 bits. Not literally stated by the manual, but
  required: the manual's own "TUNE1"/"TUNE2" demo DATA streams are tone-A-only
  and never poke R7 directly, so without this they'd play silently.
- **Combined tone/noise+envelope event duration**: when a tone/noise group
  has M=1 and is immediately followed by its envelope group, the pair is
  treated as one compound engine event — both groups' registers are
  programmed together (before scheduling anything), and the **envelope
  group's own duration byte** (not the tone/noise group's) becomes the
  event's scheduled duration. Genuinely ambiguous per the manual; this is the
  interpretation implemented and tested
  (`tone_plus_envelope_pair_uses_the_envelope_groups_own_duration`).
- **Standalone envelope group fallback**: an envelope-opcode group NOT
  preceded by an M=1 tone/noise group is not a documented manual scenario.
  Defensively treated as its own one-group event (registers programmed, own
  duration used) purely so a malformed stream can't desync or infinite-loop
  the engine — not a modelled hardware behavior.
- **`$AF` direct-access `$FF`-only-at-pair-start rule**: the manual (quoted):
  "The byte pairs you poke in (register # followed by data) are transferred
  'on the fly' into the sound generator until you send a terminator (FF
  hex)." Implemented as: in the `Register`-expecting state, `$FF` exits
  direct-access mode; in the `Value`-expecting state, `$FF` is a completely
  ordinary data byte (NOT a terminator). Cross-checked independently against
  a partial TMS7040 firmware disassembly.
- **Register-string EXECUTE with a dangling odd byte**: if the window ends
  with a register byte but no paired value byte, that trailing byte is
  dropped rather than misapplied — the manual doesn't address this case for
  register strings specifically; treated the same as sound-data's "incomplete
  trailing group never executes" rule as the smallest consistent choice.

### Concurrency model: sequential-per-stream

The sound-data engine is a single active cursor through **one** linear byte
stream at a time — one group "playing" (gating the next group's processing)
for its scheduled duration. A new EXECUTE-sound-data command **replaces**
whatever stream was previously running; there is no queueing and no
concurrent-channel scheduling in this engine. Real hardware achieves
simultaneous multi-channel playback via the separate, un-timed
register-string LOAD/EXECUTE mechanism instead (`(reg, val)` pairs applied
instantly, no timing) — already covered above, and orthogonal to the
sequential engine.

**End-of-stream does NOT silence the AY.** When a stream ends (terminator
found, or an incomplete trailing group), the engine simply stops advancing.
Whatever registers the last successfully-processed group programmed remain
exactly as set, indefinitely, until something else overwrites them — a new
EXECUTE command, or an explicit `$00`/`$CF` stop
(`Ssc::stop_all_sound`, which *does* zero `VOL_A`/`VOL_B`/`VOL_C` — a true
off, unlike natural stream end).

**Starting an EXECUTE-sound-data command** resets the engine
(`last_amplitude_nibble = 0`, cursor/cap per the consecutive/individual
rule) and *synchronously* parses and programs the first group at
command-dispatch time — not deferred to the next tick. Subsequent groups
advance from `Cartridge::tick` via `Ssc::tick_engine`: if the elapsed cycles
meet or exceed the current group's remaining duration, `advance_engine` runs
immediately; no remainder is carried into the next event's countdown (same
simplicity tradeoff as `BUSY_HOLD_CYCLES` elsewhere in this file).

## SP0256-AL2 speech (`crates/coco-core/src/sp0256.rs`, `ssc/speech.rs`)

The SP0256-AL2 is GI's "Narrator" speech processor with the 2 KB
allophone mask ROM: a microsequencer walks bit-packed instructions in that
ROM (LSB-first, at a bit-granular PC) to load a 12-pole LPC lattice filter —
six cascaded second-order stages excited by a periodic impulse train
(voiced) or a 15-bit LFSR (noise) — and re-runs after each frame's repeat
count expires. `crates/coco-core/src/sp0256.rs` (+ `sp0256/micro.rs`,
`lpc.rs`, `datafmt.rs`) is a port of MAME `sp0256.cpp`: same opcode
semantics, operand-block layout tables, quantization table, wrapping
16-bit filter arithmetic, `HIGH_QUALITY` limiter, and `PER_PAUSE`/`PER_NOISE`
equivalents. The SPB640 speech FIFO is omitted — nothing on the SSC drives
it. A stray `STEP_BUDGET` caps instructions per sequencer run so a garbage
ROM that jumps to itself can't hang the emulator (not a hardware fact; the
real AL2 ROM never comes near it).

- **ROM**: 2 KB at chip address `$1000` (MAME `ROM_LOAD(..., 0x1000, ...)`),
  the first 128 bytes being a 64-entry jump table. ALD value `n` lands the
  sequencer at byte `$1000 + 2n` (MAME `m_ald = data << 4` in bit
  addresses). Reads outside the image return 0 = RTS/HLT.
- **Clocking**: its own 3.12 MHz crystal (MAME `XTAL(3'120'000)`), one
  output sample per 312 clocks = 10 kHz. `SP0256::step(e_cycles)` accrues
  samples at the fixed ratio 10 000 / 894 886 per E-cycle, so status lines
  advance deterministically even with no audio device draining output (and
  speech would run 2× fast in double-speed mode — the real cartridge doesn't
  work there at all, per SEBU).
- **Output**: `SP0256::output` linearly interpolates between the last two
  10 kHz samples by the fraction of a sample period elapsed — a cheap stand-in
  for the board's RC low-pass on the chip's digital output. Mixed into the
  cartridge's mux-10 line in `Ssc::audio_sample` at `SPEECH_GAIN = 1.75 /
  2.0` relative to the PSG (MAME `SP0256_GAIN`/`AY8913_GAIN`), **after** the
  Sound Activity Circuit tap: MAME routes only the AY through the SAC, so
  speech never clears status bit 5.
- **Handshake lines**: `ald_write` is dropped while LRQ is low (MAME
  `ald_w`); LRQ rises again as soon as the sequencer picks the command up,
  so one allophone can be queued behind the one playing. SBY drops on
  `ald_write` and rises when the sequencer halts with nothing latched — at
  the next period boundary, up to 64 samples (~7 ms) after the last
  instruction, same as MAME.
- **Trailing pause**: the sequencer keeps re-exciting the last frame's
  parameters after it halts; only a PAUSE (`PA1`-`PA5`) zeroes them. That is
  the manual's own instruction ("You must end allophone data with a pause
  ... to ensure that you silence the speech processor") and MAME's
  behaviour, so it is kept.
- **Timing vs Appendix C**: measured ALD-to-SBY durations (the golden table in
  `sp0256_test.rs`) run 25-40 % shorter than the manual's nominal per-allophone
  durations (which are themselves garbled in places: `/OY/` "42 ms", `/AY/`
  "26 ms"). MAME's coco3 + S/SC speaking the manual's page-13 "Color
  Computer" stream spans ~320 ms and ~990 ms for the two words with a
  ~680 ms pause between — this port: ~300, ~905 and ~640 ms — so the core
  matches its source; the datasheet figures are simply not what the core
  produces.

### Allophone feeder (`ssc/speech.rs`)

`$E0-$E7`/`$F0-$F7` set an independent cursor (`Speech`) over the usual
consecutive/individual window. On every cart tick (and once synchronously at
EXECUTE time) `feed_speech` hands the chip bytes while LRQ is high: `$FF`
at the cursor or reaching the cap ends the stream. This mirrors the
firmware servicing the chip's load-request interrupt (MAME wires DRQ to
`TMS7000_INT1_LINE`). Speech and the sound-data engine run concurrently.

Judgment calls (not literally stated by the manual):

- **Bytes ≥ 64 are skipped**: the board only strobes ALD for port-D values
  below 64 (MAME `ssc_port_c_w`: `m_tms7000_portd < 64`); what the firmware
  itself does with such a byte is undocumented, so the feeder drops it and
  moves on rather than stalling.
- **`$C7`/`$00` stop feeding, nothing more**: the SP0256's RESET pin is
  wired to `$FF7D` bit 0, not to the TMS7040, so the firmware has no way to
  cut an allophone short — the one latched (and the one already queued)
  play out. `$CF` is sound-only, per its manual entry.
- **`$FF7D` bit 0 resets only the chip**: the cursor survives and resumes
  on the next tick (see `$FF7D` above).

## AY-3-8913 core (`crates/coco-core/src/ay8913.rs`)

The AY-3-8913 is an AY-3-8910 PSG with the two I/O ports (registers 14/15)
absent — no pins on the package. `AY8913` mirrors MAME `ay8910.cpp`'s classic
(non-AY8930-expanded, non-YM2149) mode:

- **Registers**: R0-R5 tone A/B/C fine/coarse (12-bit combined period; coarse
  registers R1/R3/R5 masked to 4 bits at write time — only that many bits
  exist in silicon). R6 noise period (5-bit, masked at write time). R7 mixer:
  bits 0-2 tone disable A/B/C, bits 3-5 noise disable A/B/C, both *active-low
  enable* (bit set = that generator's contribution to the channel is
  disabled); bits 6-7 (port direction) ignored. R8-R10 channel volumes: bits
  0-3 fixed level, bit 4 selects envelope mode. R11/R12 envelope period
  (full 16-bit, no masking). R13 envelope shape — writing it always restarts
  the envelope at the top of a fresh ramp.
- **Internal step clock** = master clock / 8 (MAME
  `stream_alloc(0, m_streams, master_clock / 8)`). `Ay8913::step` accumulates
  a fractional-clock remainder across calls so a `cycles` argument that isn't
  a multiple of 8 doesn't lose clocks.
- **Tone generators**: classic (non-expanded) mode reduces MAME's duty-cycle
  down-counter to a plain toggle every `period` internal steps (clamped to at
  least 1 — with period 0 the comparison loop would never terminate).
- **Noise**: a 17-bit LFSR, feedback = bit 0 XOR bit 3 shifted into bit 16
  (`rng = (rng >> 1) | ((bit0 ^ bit3) << 16)`), output = bit 0. A second
  prescaler halves the noise-period register's effective rate before each
  LFSR shift (MAME `m_prescale_noise`). Seeded to 1 at power-on/reset (a
  zero seed would never toggle, since the feedback taps are also zero).
- **Envelope**: 16 levels (`ENV_STEP_MASK = 0x0F`), paced at 2 internal steps
  per level on the classic AY-3-8910 (`ENVELOPE_STEP_MULTIPLIER`; the YM2149
  paces twice as fast — not relevant here, this chip is always classic mode).
  Shape decode (hold/alternate/attack bits, with CONT=0 shapes folded to
  their CONT=1 equivalent) mirrors MAME's `envelope_t::set_shape`/pacing loop
  exactly, including the `step ^ attack` XOR trick that turns a plain
  down-ramp into attack (rising) and alternating (triangle) shapes.
- **Volume DAC table**: MAME derives its per-step output level from resistor
  values Matthew Westcott measured off a ZX Spectrum's AY circuit in Dec
  2001 (`ay8910_param` in `ay8910.cpp`): a shared pull-down resistor
  (8MΩ), a pull-up switched in for every step except 0 (800kΩ — volume 0 is
  a true 0V off, not merely the quietest AC level), and 16 measured
  per-step resistors in parallel with a 1kΩ load. `build_volume_table` in
  `ay8913.rs` reproduces that same resistor-network formula
  (`rw/rt`, the "on" vs "total" parallel conductance at each step) and
  min-max normalizes the 16 results to `[0.0, 1.0]` — a cleaner
  normalization than MAME's legacy `-0.25 * 0.5` rescale, which exists only
  to match old non-normalized emulator output levels. Because the raw
  resistor-network fraction is monotonically increasing in the step index,
  this normalization lands volume 0 at exactly 0.0 and volume 15 at exactly
  1.0 (asserted by a unit test).
- **Mixing (deliberate deviation from MAME)**: real AY output mixing runs
  all three channels through one shared resistor network — a genuinely
  nonlinear combination MAME reproduces with an `8×32×32×32`-entry
  precomputed table (`mix_3D`). This implementation instead sums the three
  channels' already-gated, already-DAC'd levels and divides by three
  (`SINGLE_OUTPUT` style). This keeps full-scale output comparable
  regardless of how many channels are active, but does not reproduce the
  real chip's channel-interaction nonlinearity — good enough for a sound
  cartridge's music/SFX, not bit-accurate against a chip analyzer capture.
- **Output/downsampling**: `Ay8913::step` accumulates a running sum of the
  mixed per-internal-step output; `Ay8913::drain` returns the average since
  the last call and resets the accumulator — a box-filter downsample from
  the AY's ~223.7 kHz internal-step rate to whatever rate the caller drains
  at (see "Audio integration" below).

## Audio integration

`SystemBus::sound_sample` point-samples the speaker level once per scanline
(~15.7 kHz) — see `crates/coco-core/src/bus.rs`. The mux's cartridge-input
arm (SEL2:SEL1 = 10), previously always silent, now calls
`Cartridge::audio_sample` and mixes the result in with a gain constant
(`CARTRIDGE_GAIN = 0.75`) matched to the 6-bit DAC's own gain, so an
AY at full scale is comparably loud to the DAC at full scale.

`Cartridge::audio_sample` is called **exactly once per `sound_sample`
invocation, regardless of mux selection** — not just when the mux happens to
be pointed at the cartridge input. `Ssc::audio_sample` drains the AY (see
"Output/downsampling" above) and feeds the Sound Activity Circuit
unconditionally, since `$FF7E` bit 5 must reflect the cartridge's own output
even while the CoCo's speaker is listening to the DAC or cassette instead,
then adds the SP0256's interpolated output (see "SP0256-AL2 speech").

### Sound Activity Circuit (SAC)

An envelope follower on the cartridge's own (pre-mux) audio output, purely so
`$FF7E` bit 5 can report activity. Per MAME `coco_ssc.cpp`:

1. One-pole DC-blocking high-pass filter: `y = 0.99 * (y_prev + x - x_prev)`.
2. Rectify (`|y|`) and run an asymmetric leaky integrator: attack coefficient
   `0.0026` when the rectified sample exceeds the current envelope, decay
   coefficient `0.0003` otherwise.
3. Hysteresis: `sound_active` becomes `true` once the envelope exceeds `0.05`
   (`THRESH_ON`), `false` once it drops below `0.01` (`THRESH_OFF`) — the gap
   between the two thresholds avoids the status bit chattering right at a
   single threshold.

MAME runs this per host-audio-sample (its own audio stream's native rate,
tens of kHz). This implementation instead runs it once per
`Ssc::audio_sample` call, i.e. once per `sound_sample` scanline sample
(~15.7 kHz) — close enough in order of magnitude that the coefficients above
are used unmodified, but the effective attack/decay *time constants* shift
somewhat relative to real hardware. Fine for driving a status bit a program
polls; not tuned to match a real cartridge's response time exactly.

**Practical consequence for driving audible activity**: because
`Ay8913::drain`'s box filter averages over a whole scanline's worth of
internal steps, a tone whose period is short relative to that window
averages out to a nearly constant per-sample level — which a DC-blocking
filter can't distinguish from silence. Detecting activity relies on the tone
being slow enough (relative to the ~15.7 kHz sampling rate) that consecutive
drained samples swing between near-silent and near-full-scale as the tone's
own square wave transitions — true of any audible-range tone in practice,
but worth knowing when writing a test around it (see
`crates/coco-core/tests/ssc.rs`'s `TEST_TONE_PERIOD` comment).

## Bus routing

`$FF40-$FF5F` is the "standard" SCS* window; `$FF60-$FF7E` is
motherboard-unmapped, but the full address bus reaches the expansion
connector regardless, so cartridges are free to decode registers there too
(`docs/cartridges.md` "Carts can decode addresses outside SCS" — the Deluxe
RS-232 Pak at `$FF68-$FF6B`, Orchestra-90 at `$FF7A/$FF7B`, the SSC at
`$FF7D/$FF7E`). `SystemBus` now routes the whole `$FF40-$FF7E` band to
`Cartridge::read`/`write` (previously only `$FF40-$FF5F`); `$FF7F` stays
carved out separately for the Multi-Pak's own select register
(`Cartridge::control_read`/`control_write`), since it must reach the MPI
itself even when a cartridge's own decode could alias it.

**Multi-Pak Interface forwarding**: the MPI only switches SCS*/CTS*/CART*
between slots — the address and data buses are common to every slot. So
`$FF40-$FF5F` still goes only to the SCS-selected slot, but `$FF60-$FF7E` is
broadcast to **every** slot on write, and read returns the first
non-open-bus response among all four slots (real hardware would bus-fight if
two plugged-in carts both decoded the same extension address; in practice at
most one ever does). This means an SSC plugged into a non-SCS-selected MPI
slot still receives its `$FF7D`/`$FF7E` traffic — verified by
`crates/coco-core/tests/ssc.rs`.

## Deferred (not built here)

- **TMS7040 CPU core**: a full second CPU emulation of the microcontroller
  that runs the cartridge's own firmware. Not needed for `$FF7E` writes to
  have real meaning any more — the 26-3144 manual's Appendix A documents the
  protocol directly, and this implementation interprets it without emulating
  the CPU that would otherwise run it (see "Host byte protocol" above). A
  TMS7040 core would only be needed to run the *actual* firmware image
  bit-for-bit (e.g. to reproduce undocumented edge cases or bugs), not to get
  correct SOUND-side behavior.
- **Text-to-speech** (`$80-$87`/`$90-$97` LOAD + `$C0-$C6`/`$D0-$D7` EXECUTE,
  and the default-mode ASCII-until-`$0D` path): the manual is explicit that
  the English letter-to-allophone rules are "ROM-based phonetic rules" in
  the TMS7040's 4 KB firmware (`pic-7040-510.bin` in MAME's `coco_ssc`
  set). Reproducing them faithfully means either running that firmware on
  a TMS7000 core — which would also replace the invented
  `BUSY_HOLD_CYCLES`/`timing` constants above with the real thing — or
  reverse-engineering the rule tables out of its disassembly. Either is a
  separate piece of work; a generic English-to-allophone ruleset would not
  match the cartridge's pronunciations. These commands are
  recognized-and-ignored, not silently misrouted: `status::SPEECH_READY`
  stays as the chip reports it, and the LOAD variants still fill buffer RAM.

The remaining seam is documented (see "What's modelled") rather than a silent
gap.
