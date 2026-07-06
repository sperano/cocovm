# DMP-105 Protocol Spec (V2 verification findings)

Source: *DMP-105 Dot Matrix Printer Operation Manual* (Tandy/Radio Shack,
catalog #26-1276), scanned copy from colorcomputerarchive.com, read at page-image
resolution and cross-checked between its own duplicate tables (Appendix A vs the
chapter tables). Cross-check source for family context: *DMP-130 Operation
Manual* (same archive). MAME has **no** Tandy DMP-family printer device
(searched mamedev/mame — zero hits), so there is no emulator prior art to lean
on. Page numbers are the manual's printed page numbers.

Every claim below is VERIFIED against the manual unless explicitly flagged
INFERRED or UNVERIFIABLE. Phase 2 implements only VERIFIED entries.

## 1. Machine basics

| Item | Value | Source |
|---|---|---|
| Glyph matrix | 9 wide x 7 high dots | p.3, p.22 |
| Graphics-mode vertical dots | 7 per column (fixed) | p.33 |
| Descenders/underline | one extra dot row below the 7-dot body (g p q y j; ç µ § ß ƒ) | p.47, p.48 |
| Head pin count | **UNVERIFIABLE** — manual never states it; 9-pin plausible (7+descender+underline) but INFERRED only | p.57 schematic unreadable at scan resolution |
| Columns @ 10 CPI | 80 | Appendix G p.59 |
| Dots/line | Normal 960, Compressed(12 CPI) 1152, Condensed(16.7 CPI) 1600 | Appendix G p.59 |
| Character cell | 12 dots wide at every pitch (9 glyph + 3 gap); no adjustable letter-spacing exists | p.59 (dots/char = 12) |
| Paper | fanfold 4"–9.5" tractor, or friction single sheets; platen lever selects | p.7, p.9, p.59 |
| Carriage | bidirectional minimum-distance, power-on default; `1B 55 01` = unidirectional, `1B 55 00` = bidirectional | p.3, p.32 Table 16 |

## 2. Serial interface (p.44–45)

- DIP switch 1 ON = serial; switch 2: ON = 600 baud, OFF = 2400 baud. Only
  those two rates. No parity/word-format switches exist.
- Framing: 1 start bit (SPACE), 8 data bits, no parity, 1 or 2 stop bits
  (MARK; only the first stop bit is checked).
- Receive buffer: up to 134 characters (separate from the print/dot buffer).
- 4-pin DIN: pin 1 NC, pin 2 BUSY (from printer), pin 3 GND, pin 4 DATA (to
  printer). RS-232 bipolar levels: SPACE = logic 0 = +3..+25 V, MARK = logic 1
  = −3..−25 V.
- BUSY polarity: SPACE = READY, MARK = BUSY (explicit, p.44 + p.45 timing
  diagram). Assertion granularity is not stated; the 134-byte-buffer-overflow
  model is **INFERRED** from p.44 + p.45 remarks.
- Framing error: printer prints one `X` glyph, then stops until the data line
  returns to MARK. In Graphics Mode the `X` is unprintable so nothing prints
  (p.45).
- Integration note: polarity here is the printer-side RS-232 convention; must
  be composed with V1's PIA-pin polarity findings, not assumed identical.

## 3. Control codes (non-ESC)

| Dec | Hex | Name | CP mode | Graphics mode | Source |
|---|---|---|---|---|---|
| 0,1 | 00,01 | — | ignored | ignored | p.39 |
| 10/138 | 0A/8A | LF | print buffer, feed at latched LF pitch (default 1/6") | 0A: fixed 7/72" feed; 8A is graphics data | p.25, p.39 |
| 13/141 | 0D/8D | CR | print buffer; CR-only or CR+LF per NL mode | 0D same; 8D is graphics data | p.26, p.39 |
| 14 | 0E | End Underline | stop underlining | ignored | p.29 T13, p.39 |
| 15 | 0F | Start Underline | start (2-pass: chars then rule) | ignored | p.29 T13, p.39 |
| 18 | 12 | Select Graphics | enter Graphics Mode | ignored | p.39 |
| 28 n c | 1C n c | Repeat | repeat code c, n(1–255) times | only if c has MSB set | p.29, p.40 |
| 30 | 1E | End Graphics | ignored | exit Graphics Mode | p.40 |

- Undefined codes 0–31 outside the set above: print literal `X` in CP mode;
  ignored in Graphics (p.26, p.41 T12).
- $80–$9F and $C0–$DF: undefined, print `X` in CP mode; graphics data in
  Graphics mode (p.40, p.41).
- **FF (0x0C): VERIFIED ABSENT.** No form feed, top-of-form, or page-length
  concept anywhere in the firmware (DMP-130 manual lists FF as its own
  addition). FF from the host is just an undefined code (prints `X`). Page
  handling is entirely host-side; the emulator's 11" page model is a paper
  concept, not a printer command.
- **HT (0x09): VERIFIED ABSENT.** No tab; `1B 10` positioning serves that
  role (p.30).

## 4. Escape sequences (complete list)

| Bytes (hex) | Effect | Source |
|---|---|---|
| 1B 0E / 1B 0F | Start / End Elongation (double-width) | p.22 T6 |
| 1B 10 n1 n2 | Head positioning to dot column n1*256+n2 (n1 0–3; max col 799) — both modes | p.30, p.34 T18 |
| 1B 13 | Normal 10 CPI (default) | p.22 T6 |
| 1B 14 | Condensed 16.7 CPI | p.22 T6 |
| 1B 15 | CR = CR only | p.26 T11 |
| 1B 16 | CR = CR+LF (default) | p.26 T11, p.51 |
| 1B 17 | Compressed/"Elite" 12 CPI (manual uses both names) | p.22 |
| 1B 1C | LF pitch = 1/12" | p.25 T9 |
| 1B 1F / 1B 20 | Start / End Bold | p.22 T6 |
| 1B 36 | LF pitch = 1/6" (default) | p.25 T9 |
| 1B 38 | LF pitch = 1/8" | p.25 T9 |
| 1B 55 00 / 01 | Bidirectional / Unidirectional | p.32 T16 |
| 1B 5A n | n/72" feed, executed immediately (both modes; n 0–255) | p.29 T10 |
| 1B 5B n | n/72" feed, latched only (CP mode; n 0–127) | p.29 T10 |

No `ESC @` reset, no other sequences. Vertical spacing net: 6/8/12 LPI plus
raw n/72" (Appendix G p.59).

## 5. Graphics mode (ch.7 pp.33–35)

- Enter `12`, exit `1E`. Pitch must be selected *before* entry (ignored inside).
- Density follows the prior character pitch: 960/1152/1600 dots/line.
- Data byte = 128 + dot weights: bit0 = **top** dot (weight 1) … bit6 =
  **bottom** dot (weight 64); bit7 always set as the data marker (not an 8th
  pin). `FF` = all 7 dots (manual derives 1+2+…+64=127 explicitly). This is
  LSB-first-top — the **opposite** of Epson ESC/P conventions.
- `1B 10 n1 n2` positioning: n1 = 256-column band (0–3), n2 = offset; CHR$(0)
  must still be sent when n1=0 (worked example p.34).
- Line feed inside Graphics: only `0A`, nominally 7/72". **Manual-internal
  contradiction found during T4 implementation**: Appendix D item 8 (p.51)
  states 11 full-pitch LFs = 18 graphics LFs exactly (and 11 half LFs = 9),
  but 11 × 12/72" = 132/72" ≠ 18 × 7/72" = 126/72". The two claims cannot
  both be exact. The 18:11 ratio implies graphics LF = 11/108" = 22 steps of
  a 1/216" mechanical unit — under which every documented pitch is an integer
  step count (full LF 36, 1/8" 27, 1/12" 18, n/72" = 3n, graphics 22) and
  "7/72" (21 steps) is a rounded nominal. INFERRED, not verified; resolve in
  V3 before Phase 3 graphics ships (check the DMP-130 manual's graphics LF
  wording for corroboration). Current code implements the individually-stated
  facts (graphics LF = 7/72") and carries a test documenting that the p.51
  identity does not hold under them
  (`dmp105.rs::graphics_lf_vs_text_lf_rounding_trap_is_not_reproducible_from_given_facts`).
- Manual has no full-bitmap dump example, but the model above is complete;
  no freehand/joystick mode exists (that's DMP-130 only).

## 6. Character set (Appendix C pp.47–49)

- $20–$7E: standard 94-char ASCII, 1:1.
- $80–$9F: undefined (prints `X`).
- $A0–$BF: 32 European symbols (à ç £ µ § ® © ¼ ¾ ½ ¶ ¥ Å … ß ™).
- $C0–$DF: undefined (prints `X`).
- $E0–$FE: 30 block-graphic chars ($E0 = blank); note says use 1/12" LF for
  seamless diagrams. $FF unused.
- Manual claims "158 patterns in ROM" but its own tables total 156 — internal
  inconsistency in the manual; the three tables (94/32/30) are authoritative.

## 7. Power-on defaults (Appendix D item 1, p.51)

Pitch Normal 10 CPI; LF pitch 1/6"; NL mode (CR = CR+LF); underline off;
elongation off; bold off; bidirectional; buffer cleared. **No software reset
code exists** — defaults restore only on power-cycle. The interpreter's reset
entry point is a power-cycle event, not a byte sequence.

## 8. Family context

DMP-105 and DMP-130 share a common Tandy DMP control-code core (identical
bytes for CR/LF pairs, the 0E=end/0F=start underline ordering — confirmed in
both manuals, not an OCR error — elongation, graphics entry `12`). DMP-130 is
a strict superset adding FF/TOF/page-length, backspace, IBM emulation mode,
hex dump, margins, perforation skip, country sets, DP/WP/BI modes. Architect a
shared DMP core with per-model extensions; do not port any DMP-130-only code
into the 105 interpreter. DMP-100/110/120 not examined (deferred to V3).
