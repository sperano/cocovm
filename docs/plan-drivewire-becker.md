# Plan: DriveWire (Becker port) — emulator as server

Make the emulator itself a **DriveWire server**, exposing host disk images (and,
later, host directories) to DriveWire-aware **HDB-DOS** and **NitrOS-9** running
inside the emulated CoCo. Primary transport: the fast **Becker port**.

## Why "Becker port"? (answer to the question)

Named after **Gary Becker**, author of **CoCo3FPGA** (a CoCo 3 reimplemented on
an Altera/DE1 FPGA). DriveWire was originally bit-banged over the real serial
port ($FF20) at 38400/57600 baud — slow and interrupt-jitter-sensitive. Becker
added a **2-register virtual serial port** to CoCo3FPGA so DriveWire bytes move
at full bus speed. VCC and XRoar adopted that interface, MAME added it in 0.156
(2014, MAMEtesters #5660), and everyone named the device "becker" after him. It
is **not a real Tandy peripheral** — it's a virtual/FPGA-era fast path.
(Sources: MAMEtesters #5660; CoCo3FPGA/Davebiz wiki; VCC `becker.c`.)

## Verified facts

Sources: MAME `coco_dwsock.cpp` + `coco.cpp`; VCC `becker.c`; DriveWire
Specification (boisy/DriveWire); DriveWire 3 Spec PDF. All register/opcode/
checksum values cross-agree across MAME + VCC + spec.

### Becker register interface (no $FF40)
- **`$FF41` status (read-only)**: `0x02` if ≥1 RX byte is available from the
  server, else `0x00`. Only bit 1 defined. Write ignored.
- **`$FF42` data**: read = next server→client byte (FIFO); write = send one byte
  to the server immediately.
- MAME/VCC connect this to a DriveWire server over **TCP 127.0.0.1:65504**.
  Since our emulator *is* the server, Becker bytes feed an **in-process**
  DriveWire state machine — no socket, single-threaded, no concurrency. (Still
  worth optionally exposing the TCP port so external tools like DriveWire4 /
  pyDriveWire can drive it.)
- **⚠ $FF41 collides with the Games Master Cartridge's SN76489** (see
  `plan-games-master-cartridge.md`) — MAME intercepts becker *before* the cart,
  so the two cannot coexist in one slot. Model the same precedence.

### DriveWire protocol (server side)
- **Sector = 256 bytes; LSN = 24-bit big-endian (3 bytes, MSB first).**
- **Checksum = plain 16-bit sum of ALL 256 sector bytes**, sent big-endian.
  **CORRECTION (2026-07-08):** the earlier "255-byte" claim was wrong — re-traced
  against three independent sources that all sum 256 bytes: DW4 Java
  `DWProtocolHandler.computeChecksum` (`while (numbytes > 0) { numbytes--; … }`
  touches indices 255..0), pyDriveWire `dwutil.dwCrc16` (`sum(bytearray(data))`),
  and NitrOS-9's own 6809 code (`dwcheck.asm` `DoCSum`: 8-bit counter
  pre-decremented from 0 = 256 iterations; `dwread_becker.asm` accumulates every
  byte read). MAME `coco_dwsock.cpp` contains no checksum logic at all (pure TCP
  bridge) and was never a valid source for the old claim; VCC `becker.c` could
  not be located to re-check. Max sum 65280 fits u16 exactly — no wrapping.
- **Transaction timeout 250 ms** (both sides abort a stalled transfer). We
  implement it lazily in CPU cycles: 447,443 cycles = 250 ms at the max
  1.7897725 MHz clock, checked when the next byte arrives.

Minimum viable opcode set to boot NitrOS-9 + serve HDB-DOS (**corrected
2026-07-08 against DW4 Java + pyDriveWire + NitrOS-9 dwio/rbdw asm**):
| Op | Hex | Layout |
|----|-----|--------|
| OP_NOP | 0x00 | ignore |
| OP_TIME | 0x23 | reply 6 bytes: y−1900, month **1–12**, day, hr, min, sec (no DOW byte) |
| OP_INIT | 0x49 | session begin, single byte, **no reply** |
| OP_TERM | 0x54 | session end, single byte, **no reply** |
| OP_RESET | 0xF8/0xFE/0xFF | defensive no-op, aborts any in-flight transaction |
| OP_DWINIT | 0x5A | `[5A][drv_ver]` → reply exactly **0x04** (DW_PROTOCOL_VERSION; NitrOS-9 accepts only 0x04 or 0xFF and aborts driver install otherwise — it's a version number, not a feature bitfield) |
| OP_READ / OP_REREAD | 0x52 / 0x72 | req `52 drv L2 L1 L0` → **1 status byte first**; only if `00`: then `<256 data>` then `ckHi ckLo` (old "`00 ckHi ckLo <256>`" order was wrong). On error the status byte is the whole reply (`F6` unmounted, `F4` bad LSN / I/O) |
| OP_READEX / OP_REREADEX | 0xD2 / 0xF2 | req same → server **always** sends 256 bytes (real data, or 256 **zero** bytes on read failure, remembering the pending error); client sends `ckHi ckLo`; server → final status: `F3` if client checksum mismatches (CRC **overrides** the pending read error), else pending error (`00` if none) |
| OP_WRITE / OP_REWRITE | 0x57 / 0x77 | req `57 drv L2 L1 L0 <256> ckHi ckLo` — server always consumes all 262 bytes; reply 1 byte: `F3` bad checksum (checked before touching the drive), `F6` unmounted, `F5` write error, `00` OK |
| OP_GETSTAT/SETSTAT | 0x47/0x53 | `[op][drv][statcode]` → **no reply** (reference servers log & ignore; NitrOS-9 never reads a reply) |
Error codes: `00` OK, `F3` CRC, `F4` read, `F5` write, `F6` not-ready.
Unknown opcodes: consume only the opcode byte, no reply (matches DW4 +
pyDriveWire). **HDB-DOS mode** is a *global* server flag in both reference
servers, not per-drive: drive byte in the packet is ignored and
`drive = LSN / 630`, `lsn = LSN % 630` (630 = 35 tracks × 18 sectors).

Virtual serial ops (0x43/0x63/0xC3/0x80–0x8F/0x45/0xC5/0x44/0xC4) multiplex
~16 virtual serial channels for OS-9 `/N` networking/telnet. Real channels are
**phase 2** — but a minimal **idle/no-op subset IS required to boot the stock
NitrOS-9 becker disk** (its startup runs inetd + scdwv, which poll OP_SERREAD
every ~40 clock ticks and freeze forever on no reply). Verified idle subset
(NitrOS-9 scdwv.asm/dwio.asm + DW4 Java + pyDriveWire, 3-way):
SERREAD 0x43 (no payload) → always reply `00 00`; SERINIT 0x45 / SERTERM 0xC5
consume 1 byte; SERGETSTAT 0x44 / SERWRITE 0xC3 consume 2; SERSETSTAT 0xC4
consumes 2 **plus exactly 26 more bytes iff statcode = 0x28 (SS.ComSt, the SCF
PD.OPT table — scdwv OPTCNT = DW4 `comRead(26)`)**; FASTWRITE 0x80–0x8F
(channel = op−0x80) consume 1; SERREADM 0x63 consumes 2, replies `count` zeros
(unreachable while SERREAD reports idle). None reply otherwise; scdwv never
reads a reply to INIT/TERM/GETSTAT/SETSTAT.

### "Expose local directories" — the honest answer
DriveWire moves **256-byte sectors of a block device only** — it has **no
native concept of serving a host directory**. Realistic options:
1. **Serve a disk image** (`.dsk`/`.os9` RBF / `.vhd`) — the standard path; the
   CoCo's OS provides the filesystem, the server just does `offset = LSN*256`.
   This is all that's needed to boot and run. **Phase 1.**
2. **Pack a host directory into a synthesized image** (RBF for NitrOS-9, or
   RS-DOS for Disk BASIC) and mirror changes back — our own host-side layer,
   **not** part of DriveWire. Non-trivial (must implement the on-disk filesystem
   format). **Phase 2/3.**
3. **Host access over DriveWire virtual serial** (OS-9 networking) — most
   flexible, needs client-side drivers. **Phase 2.**

## Architecture integration

- **Becker port is NOT a `Cartridge`** — it's a host-side virtual port that
  intercepts `$FF41`/`$FF42` *before* the cart. Add a `becker: Option<BeckerPort>`
  field on `SystemBus`; in `io_read`/`io_write` (bus.rs:367/401), when enabled,
  intercept those two addresses ahead of the `CART_BASE..=CART_LAST` arm
  (bus.rs:380/413), mirroring MAME's precedence.
- **DriveWire server** lives in `coco-core` as a synchronous state machine fed
  byte-by-byte from Becker writes, emitting reply bytes into the Becker RX FIFO.
  No thread needed. A `DwServer { drives: Vec<Option<DwDrive>> }` where
  `DwDrive` wraps a host file (flat LSN image) or `.vhd`.
- **OP_TIME** reads the host clock (the machine already has no wall-clock dep;
  inject a time source).
- Optional TCP bridge (phase 2) runs the same state machine over a socket for
  external DriveWire tools.

## Task breakdown & model assignment

| # | Task | Model | Rationale |
|---|------|-------|-----------|
| 1 | **Becker port** device + bus interception at `$FF41`/`$FF42` (RX FIFO, status bit, precedence over cart) | **Sonnet** | Small, but the pre-cart interception + GMC-conflict precedence needs care. |
| 2 | **DriveWire protocol core**: opcode state machine, LSN→offset sector I/O, **255-byte checksum**, READEX handshake, error codes, OP_DWINIT/OP_TIME/OP_INIT/TERM/GETSTAT/SETSTAT | **Sonnet** | Fully specified; a clear stateful protocol. The checksum quirk and READEX two-phase handshake are the traps — pin them with test vectors. |
| 3 | **Disk backends**: flat `.dsk`/`.os9` LSN images + `.vhd`; per-drive mount table; HDB-DOS "absolute sector" mode flag | **Sonnet** | Straightforward file I/O; the mode flag is the one subtlety. |
| 4 | **egui UI**: enable Becker, mount/unmount images per drive, activity indicator | **Haiku** | Mechanical, mirrors existing disk-mount UI. |
| 5 | **Optional TCP bridge** for external DriveWire tool interop (same state machine over a socket) | **Sonnet** | Contained; keep behind a config flag. |
| 6 | **Phase 2 — directory serving**: synthesize an RBF (and/or RS-DOS) image from a host directory + write-back | **Opus** | Implementing the OS-9 RBF / RS-DOS on-disk format correctly is genuinely hard and easy to corrupt; wrong-fix-expensive. Scope separately; do not bundle with phase 1. |
| 7 | **Phase 2 — virtual serial channels** (OS-9 `/N`) | **Sonnet** | Additive opcode set; independent of disk path. |

## Testing / acceptance
- **Protocol unit tests** with captured byte sequences: OP_READEX round-trip
  (including the 255-byte checksum and a forced `F3` mismatch → REREADEX),
  OP_WRITE, OP_DWINIT feature byte, OP_TIME.
- **Boot NitrOS-9** from a DriveWire-served boot disk over the Becker port and
  reach a shell — the real integration test.
- **HDB-DOS**: `DIR`/`LOAD` from a becker-served `.dsk` in Disk BASIC.
- Interop (optional): pyDriveWire client against our TCP bridge reads a sector
  matching the file.

## Implementation status (updated 2026-07-13, branch `worktree-drivewire-becker`)
Phase 1 implemented, green, and **acceptance-tested**: `coco-core/src/drivewire.rs`
(DwServer state machine + DwImage backends + injected clock + vserial idle
subset, 23 unit tests), Becker intercept in `bus.rs` at $FF41/$FF42 ahead of
cart dispatch on both GIME and SAM paths (`tests/drivewire_bus.rs`, 6 tests),
egui DriveWire submenu + status bar + `--becker/--dw0..3/--hdbdos` flags,
chrono wall clock. Acceptance (`tests/drivewire_boot.rs`, 3 tests, assets:
`roms/hdbdw3bc3.rom` + `disks/spetris.dsk`/`blank02.dsk`/
`nos96809l2v030300coco3_becker.dsk`):
- HDB-DOS 1.4 Becker boots, `DIR` lists a served .dsk, `SAVE` writes sectors;
  mounting before cold start even auto-runs `AUTOEXEC.BAS` → `LOADM`/`EXEC`
  over Becker. Caveat: HDB-DOS cold-start autoruns AUTOEXEC.BAS if present, so
  the DIR test mounts after reaching OK.
- NitrOS-9 3.3.0 L2 becker disk: HDB-DOS `DOS` → kernel + modules load over
  Becker (hdbdos mode OFF; DOS reads track 34 = LSN 612–629 where drive-0
  addressing coincides), clock2_dw fetches time via OP_TIME (no time prompt),
  Shell+ prompt `{Term|02}/DD:`, `dir` lists the root. unknown_opcodes = 0.
Not yet done: TCP bridge (task 5), phase 2 (real vserial channels, directory
serving).

## Risks
- **Checksum is a plain 256-byte sum** (see correction above) — the earlier
  255-byte claim would have broken every transfer; pinned by unit tests.
- **$FF41 GMC collision** — document; enforce becker-before-cart precedence.
- **HDB-DOS vs NitrOS-9 drive addressing** (absolute-sector vs drive-number
  mode) — make it a config flag, test both.
- **Directory serving (phase 2)** is much bigger than it sounds — keep phase 1
  (image serving) shippable on its own.

## Cartridge-I/O address contention (shared reference)
The third-party `$FF40–$FF7E` space is crowded; only one cart physically
occupies a slot (MPI switches SCS). Master map for all plans:
`$FF40/41` GMC (bank/PSG) · `$FF41/42` **Becker** · `$FF50–53` Disto RTC ·
`$FF50–58` / `$FF70–78` SuperIDE · `$FF5A–5F` CoCo PSG · `$FF5C`/`$FF7C` DS1315 ·
`$FF68–6B` Deluxe RS-232 · `$FF7A/7B` Orchestra-90 · `$FF7D/7E` Sound/Speech.
