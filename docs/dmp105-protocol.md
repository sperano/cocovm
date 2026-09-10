# DMP-105 Protocol Spec (V2 verification findings)

Source: *DMP-105 Dot Matrix Printer Operation Manual* (Tandy/Radio Shack,
catalog #26-1276), scanned copy from colorcomputerarchive.com, read at page-image
resolution and cross-checked between its own duplicate tables (Appendix A vs the
chapter tables). Cross-check source for family context: *DMP-130 Operation
Manual* (same archive). MAME has **no** Tandy DMP-family printer device
(searched mamedev/mame — zero hits), so there is no emulator prior art to lean
on. Page numbers are the manual's printed page numbers.

Every claim below is VERIFIED against the manual unless explicitly flagged
INFERRED or UNVERIFIABLE. Behavior follows the explicit command tables and worked examples. Source conflicts and implementation choices are called out below.

## 1. Machine basics

| Item | Value | Source |
|---|---|---|
| Glyph matrix | 9 wide x 7 high dots | p.3, p.22 |
| Graphics-mode vertical dots | 7 per column (fixed) | p.33 |
| Descenders/underline | one extra dot row below the 7-dot body (g p q y j; ç µ § ß ƒ) | p.47, p.48 |
| Head pin count | **UNVERIFIABLE** — manual never states it; 9-pin plausible (7+descender+underline) but INFERRED only | p.57 schematic unreadable at scan resolution |
| Columns @ 10 CPI | 80 | Appendix G p.59 |
| Text dots/line | Normal 960, Compressed(12 CPI) 1152, Condensed(16.7 CPI) 1600 | Appendix G p.59 |
| Graphics and positioning columns/line | Normal 480, Compressed 576, Condensed 800; only every second text dot is addressable | p.30 Table 14, p.33 Table 17 |
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
| 13/141 | 0D/8D | CR | print buffer; CR-only or CR+LF per NL mode | 0D returns home and, in NL mode, feeds the fixed graphics pitch; 8D is data | p.26, p.39 |
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
| 1B 10 n1 n2 | Head positioning to dot column n1*256+n2 (n1 0–3; every second text dot) — both modes | p.30, p.34 T18 |
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

- Enter `12`, exit `1E`. Pitch must be selected *before* entry. Pitch commands inside graphics are ignored, including their effect on subsequent text.
- Density follows the prior character pitch: **480/576/800 graphics columns/line** (60/72/100 DPI over 8 inches). Text uses twice as many horizontal dot positions. The earlier specification incorrectly used the text-dot counts for graphics.
- Data byte = 128 + dot weights: bit0 = **top** dot (weight 1) … bit6 =
  **bottom** dot (weight 64); bit7 always set as the data marker (not an 8th
  pin). `FF` = all 7 dots (manual derives 1+2+…+64=127 explicitly). This is
  LSB-first-top — the **opposite** of Epson ESC/P conventions.
- `1B 10 n1 n2` positioning: n1 = 256-column band (0–3), n2 = offset; CHR$(0)
  must still be sent when n1=0 (worked example p.34). At condensed pitch,
  position 800 wraps to column zero of the next graphics line (p.34);
  other out-of-range position commands are ignored (p.27).
- Graphics `LF` and the feed portion of `CR` use **7/72 inch**, as stated
  explicitly on p.25 and in Appendix A p.39. Text feed-pitch commands do
  not change this. Select CR-only or CR+LF before graphics entry: those
  escape commands are also ignored inside graphics (p.39).
- **Conflicting source, explicit implementation choice:** Appendix D item 8,
  p.51 says 11 full-pitch feeds equal 18 graphics feeds. That implies
  11/108 inch, not 7/72 inch. Reinspection of the scan confirms the conflict
  is in the manual. DMP-130 p.63 independently specifies 7/72 inch; it does
  not establish DMP-105 mechanics. Use the repeated, explicit DMP-105 feed
  definitions on pp.25/39. Do not adopt the speculative 22/216-inch feed or
  claim a verified motor-step size. Hardware measurement or firmware could
  justify revising this choice later.
- Graphics-active escapes: elongation on/off, absolute positioning, and
  immediate `ESC Z n` feed. Other documented escapes are consumed and
  ignored, including direction and latched pitch commands (pp.39–40).
- Manual has no full-bitmap dump example, but the model above is complete;
  no freehand/joystick mode exists (that's DMP-130 only).

## 6. Character set (Appendix C pp.47–49)

- $20–$7E: standard 94-char ASCII, 1:1.
- $80–$9F: undefined (prints `X`).
- $A0–$BF: 32 European symbols, in code order (p.48):
  ``´ à ç £ ` µ ° ▼ † § ® © ¼ ¾ ½ ¶`` for $A0–$AF and
  ``¥ Ä Ö Ü ¢ ‾ ä ö ü ß ™ é ù è ¨ ƒ`` for $B0–$BF. The typeset table leaves
  $A4, $B5 and $BE unreadable; those three are INFERRED from the DMP-130's
  printout of the same table (DMP-130 manual p.81), which prints grave,
  overline and diaeresis there. Descenders: ç µ § ß ƒ (p.48 note 2).
- $C0–$DF: undefined (prints `X`).
- $E0–$FE: 30 block-graphic chars (p.49), a 6×6 dot matrix (p.22) spread
  over the 12-dot cell so neighbours join (INFERRED: each column struck on
  both of its dot positions so areas print solid); use 1/12"
  LF for seamless diagrams (p.49 note). $E0 = blank. $E1–$EF are the 15
  combinations of a 2×2 quadrant grid in this order: the four single
  quadrants (top-left, top-right, bottom-left, bottom-right), the two
  diagonals (top-left+bottom-right, top-right+bottom-left), top row, bottom
  row, left column, right column, the four three-quarter blocks (missing
  bottom-right, bottom-left, top-right, top-left), then the full block.
  $F0–$FA are thin box-drawing pieces `┌ ─ ┐ ┬ ├ │ └ ┘ ┴ ┤ ┼`. $FB–$FE are
  filled right triangles with the right angle at top-left, bottom-right,
  top-right, bottom-left. $FF unused.
- Manual claims "158 patterns in ROM" but its own tables total 156 — internal
  inconsistency in the manual; the three tables (94/32/30) are authoritative.
- **Dot patterns are UNVERIFIABLE.** The controller is a mask-ROM
  microcontroller (`EP-106`, Appendix F schematic p.57) and no dump exists;
  every glyph bitmap in the emulator is an artistic approximation. Only the
  code-to-symbol mapping above is verified. See `docs/dmp-font-sources.md`.

## 7. Power-on defaults (Appendix D item 1, p.51)

Pitch Normal 10 CPI; LF pitch 1/6"; NL mode (CR = CR+LF); underline off;
elongation off; bold off; bidirectional; buffer cleared. **No software reset
code exists** — defaults restore only on power-cycle. The interpreter's reset
entry point is a power-cycle event, not a byte sequence.

## 8. Family context

DMP-105 and DMP-130 share several Tandy control codes, including underline,
elongation, and graphics entry. They require separate interpreters:
DMP-130 is **not a strict protocol superset**. Its graphics density is
fixed at 480 columns, its Tandy direction operands are reversed, and it
does not provide DMP-105's `ESC Z n` / `ESC [ n` feeds in Tandy mode.
See `dmp130-protocol.md` for the model-specific commands and source conflicts.
Both interpreters share the paper representation, display, and exports.
