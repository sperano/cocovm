# Plan: Real-Time Clock cartridge

> **Status (2026-07-08): implemented** — `crates/coco-core/src/rtc.rs`
> (`Msm6242` + `DistoRtc`), tests in `crates/coco-core/tests/rtc.rs`, egui
> Machine-menu items + `--rtc` CLI flag. The register-map discrepancy below
> is **resolved**: `clock2_disto2.asm` (2-N-1) and `clock2_disto4.asm`
> (4-N-1) are different Disto products with different select offsets
> ($FF52 vs $FF51), and MAME's `meb_rtime.cpp` is a superset that satisfies
> both — `$FF51`, `$FF52`, and `$FF53` all latch the register address, data
> at `$FF50`. That superset is what's implemented, so both NitrOS-9 drivers
> work unmodified. One deliberate deviation from MAME: time-register writes
> update the clock (real-chip behavior; MAME drops them), so NitrOS-9
> `setime` sticks. Time is injected (`rtc::TimeSource`), never `std::time`
> in coco-core; the clock tracks the host through a seconds offset, immune
> to pause/double-speed drift. Not automated: a real NitrOS-9 boot-track
> with a clock2_disto module (EOU ships the DriveWire clock by default) —
> verify `date` manually against a custom boot disk. Option B (DS1216
> phantom clock) remains unimplemented.

Give NitrOS-9 a real-time clock so it stops prompting for the date and
timestamps files. Recommended target: the **Disto RTC (OKI MSM6242)**, with the
**Dallas DS1216 SmartWatch** phantom-clock as a simpler alternative.

## Verified facts

No local PDF covers RTC cartridges. Sources are MAME (Disto path) and NitrOS-9
driver source (Dallas path) — cited below.

### Option A — Disto RTC / OKI MSM6242 (MAME-modeled)
- MAME `meb_rtime.cpp` (`disto_rtime_device`): `MSM6242(config, m_rtc, 32'768)`.
  Not a standalone cart — it plugs into the **Disto Mini Expansion Bus (MEB)**,
  a header inside the Disto **Super Controller II** floppy controller
  (`COCO_SCII`) and the **Disto RAM cartridge** (`COCO_PAK_RAM`). MEB addresses
  `$FF50–$FF57` (SCS offsets `0x10–0x17`).
- Register interface (MAME):
  - **`$FF50`** = MSM6242 data, indexed by the address latch (`read/write(addr & 0x0F)`).
  - **`$FF51`** = address-select latch.
  - `$FF52`/`$FF53` = Centronics parallel port (busy readback / strobe) — the
    board also carries a printer port; out of scope unless wanted.
- MSM6242 register file (16 nibbles, addr 0–15): S1/S10, MI1/MI10, H1/H10,
  D1/D10, MO1/MO10, Y1/Y10, W (weekday), CD/CE/CF (control). BCD.
- **⚠ Discrepancy to resolve before locking the register map:** NitrOS-9's
  `clock2_disto2.asm`/`clock2_disto4.asm` also target base `$FF50` but write the
  register-select to **`$FF52`** and read/write data at `$FF50` — which does
  **not** match MAME's `$FF51`-latch layout. Either these are different Disto
  products, or one source is off by one. **Trace the actual NitrOS-9 clock
  module you intend to boot** (disassemble its `GetTime`/`SetTime`) and match
  *that*, rather than assuming MAME's or the driver's layout.

### Option B — Dallas DS1216 SmartWatch phantom clock (NitrOS-9-source only)
- **MAME has no DS1315/DS1216 CoCo device** — the only spec is NitrOS-9
  `clock2_smart.asm` ("Dallas DS1216 SmartWatch"). Classic phantom clock under
  the ROM socket:
  - `$C000` read = send bit 0; `$C001` read = send bit 1; `$C004` = read byte.
    Bits are toggled by **reads** (TST), not writes (the chip has no CS/write).
  - **64-bit wakeup magic** `C5 3A A3 5C C5 3A A3 5C` must be clocked in before
    the clock responds; then a BCD time stream is read back.
  - On CoCo 3 the driver uses MMU reg `$FFA6` to page RAM at `$C000` so its
    bit-banging code isn't fetched from the page it's overlaying.
- Burke & Burke / Cloud-9 variants (`clock2_ds1315.asm`) expose the same
  protocol through **SCS decode** instead: `$FF5C` read / `$FF58` zero-bit /
  `$FF59` one-bit (B&B), or `$FF7C`/`$FF78`/`$FF79` (Cloud-9 "fully decoded").
- **Don't assert a specific Dallas part number** in our docs — the NitrOS-9
  sources disagree on packaging (DS1315 vs DS1216); call it "a Dallas
  phantom-clock family part, protocol per NitrOS-9 driver source."

### Recommendation
Target **Option A (MSM6242 at `$FF50/$FF51`)** as primary: it's the only RTC
MAME models (so there's a cross-checkable reference — MAME's `msm6242.cpp`), and
NitrOS-9 disto drivers exist. **But** the register-offset discrepancy means the
real gate is *matching the specific NitrOS-9 clock module*, so make the plan's
first task a trace/verify pass. Option B is a good fallback — its bit-serial
protocol is trivial to emulate — if the user's NitrOS-9 build ships the
SmartWatch driver instead.

## Architecture integration

- **Address contention:** `$FF50–$FF57` collides with SuperIDE's default
  `$FF50–$FF58` (see `plan-superide.md`) — only one per slot; note it.
- MSM6242 core: a `Msm6242 { regs, address_latch }` reading a host/injected
  time source (BCD conversion). Pure logic, no audio/video coupling.
- Decode: route `$FF50`/`$FF51` through the cartridge `read`/`write`. If the RTC
  is modeled as its own simple `Cartridge`, this rides the existing SCS window
  (`CART_BASE..=CART_LAST`, bus.rs:380/413) — **no bus change needed** (unlike
  RS-232/Orch-90). The DS1216 `$C000` phantom variant instead needs `rom_read`
  interception at `$C000/$C001/$C004`.
- Time source: inject a `fn now() -> DateTime` (host clock, or a settable
  emulated clock) — keep `coco-core` free of a hard `std::time` dependency in
  the same style as the rest of the machine.

## Task breakdown & model assignment

| # | Task | Model | Rationale |
|---|------|-------|-----------|
| 1 | **Verify precondition**: disassemble/trace the target NitrOS-9 clock module (disto2/disto4 or smart) to pin the exact register offsets and access order | **hw-verify / trace-debug** | The MAME-vs-driver discrepancy must be resolved against the *actual* driver before coding, or the clock silently reads garbage. |
| 2 | **MSM6242 core** (`msm6242.rs`): 16 BCD registers, address latch, control bits, host time source | **Sonnet** | Well-specified chip; contained pure logic. |
| 3 | **`DistoRtc` cart** wiring `$FF50/$FF51` (+ optional Centronics stub) through the SCS window | **Sonnet** | Thin `Cartridge` impl; no bus change. |
| 4 | **(Alternative) DS1216 phantom clock**: `$C000/$C001/$C004` `rom_read` interception, 64-bit magic-pattern state machine, BCD readback | **Sonnet** | Simple bit-serial state machine, but the ROM-socket overlay + MMU-page interaction needs care. |
| 5 | **egui UI**: enable RTC, set/sync clock | **Haiku** | Trivial. |
| 6 | **Tests**: register read yields correct BCD for a fixed injected time; NitrOS-9 `setime`/`date` reads it back | **Sonnet** | Deterministic with an injected clock. |

## Testing / acceptance
- Inject a fixed time; assert each MSM6242 register returns the right BCD nibble.
- Boot NitrOS-9 with the matching clock module and confirm `date` shows the
  injected time without the manual date prompt.
- DS1216 path: assert the 64-bit magic unlocks the clock and the BCD stream
  decodes to the injected time.

## Risks
- **Register-offset discrepancy** (task 1) is the whole ballgame — do not code
  the map from MAME alone.
- **$FF50 contention** with SuperIDE / RTC-bearing disk controllers — one per
  slot.
- **Driver-specific**: NitrOS-9 support is only as good as matching the exact
  clock module the user's OS build ships; document which module the emulation
  targets.
