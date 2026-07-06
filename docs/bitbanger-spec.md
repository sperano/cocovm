# Bit-Banger Serial Printer Port Spec (V1 verification findings)

Sources: CoCo 3 Service Manual (image-only PDF, read as page renders),
Color BASIC Unravelled (CBU) + Super Extended BASIC Unravelled II (SEBU),
real `roms/coco3.rom` bytes (hex-verified at the cited offsets), MAME master
(`src/mame/trs/coco.cpp`, `coco12.cpp`, `coco3.cpp`,
`src/devices/bus/rs232/printer.cpp`), `docs/6x09_Instruction_Sets.pdf`,
`docs/Lomont_CoCoHardware.pdf`. All claims VERIFIED unless flagged.

## Register map

- **TX**: PIA1 Data Register A **bit 1 (PA1)**, `$FF20`. Bit 1 = mark/idle
  (high), 0 = space. ROM init at `$A048` sets `#2` ("make RS232 output
  marking"); MAME `pia1_pa_changed()` masks `0x02`. DIN pin 4.
- **BUSY/status in**: PIA1 Data Register B **bit 0 (PB0)**, `$FF22`.
  **0 = ready, 1 = busy.** BASIC's driver (`$A2C3`/`$A2F3`, byte-identical in
  coco3.rom) spins `LDB $FF22 / LSRB / BCS` before every byte AND again after
  the byte, IRQ+FIRQ masked (`ORCC #$50`) for the whole transmit. No timeout.
  For the printer cable, DIN pin 2 (RS-232C IN) carries the printer's BUSY;
  DIN pin 1 (CD → CA1) is explicitly No Connection and MAME marks CA1 "NYI" —
  do not gate anything on CA1.
- MAME's `radio_shack_serial_printer_device` resets RXD to 0 (not-busy),
  matching the polarity above.

Manual erratum flagged: Fig 5-13 labels CD as going to "PIA IC4-4" (PA2) —
contradicts the manual's own prose (CA1, pin 40) and MAME. Transcription
error; ignore.

## Framing

**1 start bit (space) + 8 data bits LSB-first + 1 stop bit (mark), no
parity.** The ROM shift loop (`$A2BF-$A2FA`) calls the mark routine exactly
once after the 8 data bits. The Service Manual's "two stop bits" (p.48) is
**wrong** — contradicted by the ROM bytes and by MAME's
`RS232_STOPBITS_1` default. (DMP-105 accepts 1 or 2 stop bits anyway.)

## Baud timing

- Constant `LPTBTD` at RAM `$0095` (MSB) / `$0096` (LSB) — decimal 149/150,
  hence "POKE 150,n". Same address on CoCo 3.
- **Actual coco3.rom default: 88 (`$0058`)** (init table at ROM `$A10D`,
  file offset `0x210D`: `... 00 58 ...`). The Service Manual's Table 2 value
  87 (`$0057`) is the stale pre-Color-BASIC-1.2 constant (CBU Appendix I
  documents the 87→88 change). Trust the ROM.
- Bit period from full instruction-level trace of the delay path
  (`LA2FD → BSR LA302` uses a self-referential BSR so the `LDX LPTBTD` +
  LEAX/BNE countdown runs **twice** per bit):

  **cycles_per_bit = 78 + 16 × N**, N = live 16-bit value of `LPTBTD`.

  | Baud label | N | Cycles/bit | Effective @0.894886 MHz |
  |---|---|---|---|
  | 120 | 458 | 7406 | 120.9 |
  | 300 | 180 | 2958 | 302.5 |
  | 600 (default) | **88** | **1486** | 602.2 |
  | 1200 | 41 | 734 | 1219.2 |
  | 2400 | 18 | 366 | 2445.6 |

  (Service Manual Table 2 also has an internal 300-baud mismatch: "180 dec /
  $BE hex" printed side-by-side — unresolved, not needed for default path.)
- The delay loop is a pure fixed-cycle busy-wait with no clock compensation:
  the CoCo 3 high-speed poke (`$FFD9`, 1.789772 MHz) **exactly doubles** the
  effective baud. Therefore the emulator's sampler must measure bit cells in
  **CPU cycles at the current clock**, never wall time, so it automatically
  tracks POKE 150 changes and speed pokes like real hardware.

## CoCo 3 differences

None. Printer ROM code is byte-identical to Color BASIC 1.2 (hex-compared at
`$A2BF-$A307`), PIA1 DDR setup identical (DDRA=$FE, DDRB=$F8), GIME not
involved (`$FF20-$FF23` remains a real MC6821 block; MAME coco3 wires the
same handlers and the same 600/8-N-1 printer default as coco12).

## NitrOS-9 `/p` driver (T3 finding, empirical, not ROM-disassembled)

NitrOS-9 EOU 1.0.1 boots to a shell with the CoCo 3 GIME already in
high-speed mode (`GIME::cpu_fast == true`, i.e. the `$FFD9` SAM register
write has already happened — confirmed by reading `Machine`'s live GIME
state right after reaching the shell prompt, not inferred).

Unlike Color BASIC's bit-bang driver (a fixed-cycle-count busy-wait that does
**not** compensate for the speed poke — `bitbanger-spec.md` "Baud timing":
"the CoCo 3 high-speed poke... exactly doubles the effective baud"),
NitrOS-9's `/p` path keeps the **true wall-clock baud at 600 regardless of
`cpu_fast`**. Measured by instrumenting `BitBanger::tick`'s raw PA1
edge-to-edge cycle deltas during a live boot of the real EOU disk images and
running `dir /dd >/p` (619 captured bytes decode cleanly, matching the known
directory listing byte-for-byte): the fundamental bit-cell quantum observed
is ~2977-2993 CPU cycles (small jitter from MC6809 instruction-boundary
quantization/scheduler timing, not measurement error) — i.e. **twice**
`DEFAULT_BIT_PERIOD` (1486). At the doubled 1.789772 MHz clock, `2 *
DEFAULT_BIT_PERIOD` (2972 cycles) = 1.661 ms/bit = 601.7 baud, i.e. real 600
baud. Setting the decoder's `bit_period` to `2 * DEFAULT_BIT_PERIOD` decodes
both a short `echo hello >/p` and the 619-byte `dir /dd >/p` listing with
**zero framing errors**.

Conclusion: NitrOS-9's bit-bang driver reads the CPU-speed state and doubles
its own delay-loop cycle count when `cpu_fast` is set, to hold wall-clock
baud constant — the opposite compensation behavior from Color BASIC's driver
(which has no such awareness). This is an empirical finding from direct
instrumentation against unmodified NitrOS-9/EOU code, not a disassembly of
the `/p` driver itself — the driver's source/module wasn't inspected, so the
*mechanism* (e.g. which OS-9 kernel call or GIME register read it uses to
detect speed) is unverified; only the resulting bit rate is. See
`crates/coco-core/tests/bitbanger_os9.rs`.

## Decoder spec for `bitbanger.rs`

- Sample PA1 transitions with CPU-cycle timestamps (per-instruction tick,
  same pattern as cassette/FDC).
- Async RX state machine: idle(mark) → falling edge = start-bit candidate →
  re-sample the line at 0.5 bit-times and abandon the frame if it is back at
  mark (standard UART false-start-bit rejection; the ROM's boot-time DDRA
  reconfiguration at $A02F emits a ~30-cycle low glitch on PA1 that would
  otherwise decode as a phantom 0xFF) → sample mid-bit at 1.5, 2.5, …
  bit-times → 8 data bits LSB-first → verify stop bit (mark) else framing
  error.
- Bit period: configurable, default 1486 CPU cycles (600 baud at normal
  clock); expressed in cycles so speed pokes double the rate naturally. A
  robust decoder should tolerate a few percent deviation (real printers did).
- Drive PB0 (BUSY) back into PIA1 as an input: 0=ready normally; a sink may
  assert 1=busy (e.g. DMP-105 model with its 134-byte buffer) and BASIC will
  pace itself, since it polls before and after every byte.
- Byte sink is pluggable: text-capture file first, DMP-105 interpreter later.
