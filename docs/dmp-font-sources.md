# DMP-105 / DMP-130 font sources

Research notes for replacing the hand-drawn printer fonts with verified
data. Everything cited here is on this machine under `docs/` (PDFs and
`docs/orig-scans/dmp/` page renders are git-ignored; `docs/txt/` holds OCR
text with `===== PAGE n =====` markers giving the PDF page index).

## Bottom line

- **No ROM dump exists for either printer, and none is likely to appear.**
  The DMP-105 schematic (Operation Manual Appendix F, printed p. 57,
  `orig-scans/dmp/dmp105-p57-schematic.png`) shows the controller as a
  single 64-pin custom-masked microcontroller labelled `EP-106` (IC5; pins
  MR, HALT, STBY, IRQ1, EXTAL/XTAL, ports P1–P7, i.e. an HD6301Y/HD63701Y-class
  part). There is no external character ROM. The DMP-100 service manual
  likewise lists its CPU as a mask-ROM `MBL8049-NM162`. The font lives in
  on-chip mask ROM; recovering it needs a physical unit and a mode-switch or
  decap dump. MAME has no Tandy DMP device. The DMP-130 manual's schematic
  pages (TOC says p. 117) are missing from every scan found.
- **The manuals do give complete code-to-glyph mappings**, and the DMP-130
  manual contains **real printouts** of the whole character set and of every
  style combination, scanned at 300 dpi. That is the best evidence available:
  glyph *identity* is verifiable for every code; glyph *bitmaps* can be
  approximated from print samples for the DMP-130 only.
- The GitHub project GmEsoft/DMP105toTiff (clone in `docs/DMP105toTiff/`,
  GPL-3.0, Michel Bernard, 2024) has a full 9×9 DMP-105 Tandy font plus IBM
  set 2 and a "Robotron" font as ASCII art (`DmpFontGen/DMP105_Font_*.cpp`).
  No provenance is stated anywhere in the repo; it is hand-drawn, so it is a
  second artistic approximation, not evidence.
- Gissio/font_DotMatrix on GitHub reproduces the Epson FX-80 set, not a
  Tandy design; it is only relevant to a future FX-80 emulation.

## Local files

| File | What it is |
| --- | --- |
| `DMP-105 Operation Manual (Tandy).pdf` | 64 pp., 300 dpi bilevel scan, no text layer (OCR in `txt/`). colorcomputerarchive.com |
| `DMP-130 Operation Manual (Tandy).pdf` | 122 pp. (printed pages 1–116), 300 dpi bilevel, no text layer (OCR in `txt/`). Printed page p = PDF page p+6 |
| `DMP-106 Operation Manual (Tandy).pdf` | 111 pp., not yet examined (DMP-105 successor with IBM set) |
| `DMP-100 Service Manual (Tandy).pdf` | 55 pp., has text layer. Different single-hammer mechanism; only p. 8 "Character pattern A" (5×7) is font-related |
| `orig-scans/dmp/` | 300 dpi renders of every page named below |
| `DMP105toTiff/` | GmEsoft clone (git-excluded via `.git/info/exclude`) |

No DMP-105 or DMP-130 *service* manual exists online (checked archive.org,
colorcomputerarchive.com, classiccmp.org/cini, manualslib, tandy.wiki,
trs-80.com, vcfed). The DMP-105 Operation Manual contains no real print
samples at all; every character in it is typeset.

## DMP-105: verified mappings (Appendix C, printed pp. 47–49)

Manual states "158 dot-matrix patterns in the ROM": 94 ASCII + 32 European +
30 block graphic + space + blank. Descenders: `g p q y j` and underline
(p. 47 note); `ç µ § ß ƒ` (p. 48 note 2).

**32 European symbols, `$A0–$BF`** (`orig-scans/dmp/dmp105-p48-european-table.png`),
cross-checked against the DMP-130's real printout of the same table
(`dmp130-p81-printout-3x.png`), which settles the cells the typeset table
leaves unreadable:

| Code | Glyph | Code | Glyph |
| --- | --- | --- | --- |
| A0 | ´ | B0 | ¥ |
| A1 | à | B1 | Ä |
| A2 | ç | B2 | Ö |
| A3 | £ | B3 | Ü |
| A4 | ` (printout; typeset cell unreadable) | B4 | ¢ |
| A5 | µ | B5 | ‾ overline (printout) |
| A6 | ° | B6 | ä |
| A7 | ▼ | B7 | ö |
| A8 | † | B8 | ü |
| A9 | § | B9 | ß |
| AA | ® | BA | ™ |
| AB | © | BB | é |
| AC | ¼ | BC | ù |
| AD | ¾ | BD | è |
| AE | ½ | BE | ¨ (printout; typeset cell blank) |
| AF | ¶ | BF | ƒ |

**30 block graphics, `$E0–$FE`** (`orig-scans/dmp/dmp105-p49-block-graphic-table.png`).
The manual says they are "composed of six vertical dots" and the set is
"6×6" (p. 22). The table decodes as:

- `E0` blank.
- `E1–EF`: the 15 non-empty combinations of a 2×2 quadrant grid, in the
  manual's order: four single quadrants (top-left, top-right, bottom-left,
  bottom-right), two diagonals (top-left+bottom-right, top-right+bottom-left),
  top row, bottom row, left column, right column, four three-quarter blocks
  (missing bottom-right, bottom-left, top-right, top-left), full block.
- `F0` ┌, `F1` ─, `F2` ┐, `F3` ┬, `F4` ├, `F5` │, `F6` └, `F7` ┘, `F8` ┴,
  `F9` ┤, `FA` ┼ (thin box-drawing lines).
- `FB` ◤, `FC` ◢, `FD` ◥, `FE` ◣ (filled right triangles; right angle at
  top-left, bottom-right, top-right, bottom-left respectively).
- `FF` is not in the table (matches the "30 defined" count: E1–FE).

## DMP-130: verified material

- **Tandy character set table** p. 81 (`dmp130-p81-tandy-table-with-printout.png`):
  full 16×16 grid for `$00–$FF`, *plus three lines of real DMP-130 output*
  printing codes 33–254 at the bottom of the page. `dmp130-p81-printout-3x.png`
  is a 3× crop. At 300 dpi one draft-font dot column is ~2.5 px, so
  silhouettes and serif shapes are clear but individual dots merge; use it
  to check shapes, not to transcribe bitmaps blindly.
- **Extended symbols `$C0–$DF`** read from the grid and printout together:
  `â ê î ô û ^ ë ï á í ó ú ¡ ñ ã õ` then `Æ æ Å å Ø ø Ñ É Á Í Ó Ú ¿ Ù È Â`.
  The typeset grid drops diacritics on capitals; the printout shows them.
  An earlier transcription read the acute accents at `$C8–$DB` as umlauts;
  the printout is unambiguous (Spanish set: á í ó ú ¡ ¿ Á Í Ó Ú).
- **IBM character set 1 / 2** pp. 84–85 (`dmp130-p84-ibm-set1.png`,
  `dmp130-p85-ibm-set2.png`), each also with a real printout line. Not yet
  transcribed.
- **Dot-column width tables** pp. 82–83 (Tandy standard/proportional and
  correspondence/proportional) and pp. 86–87 (IBM). These are the
  proportional advance figures the task asks to verify.
- **Country character table** p. 57, Table 26 (`dmp130-p57-country-table.png`):
  fully legible for all 11 countries (n = 32 USA, 33 Germany, 34 France,
  35 Norway, 36 Sweden, 37 Denmark, 38 Finland, 39 Italy, 40 Spain,
  41 England, 42 Belgium) at ASCII 23, 24, 40, 5B, 5C, 5D, 5E, 60, 7B, 7C,
  7D, 7E. The typesetter dropped umlauts: Germany's row prints bare
  `A O U a o u` where Ä Ö Ü ä ö ü are certain, so bare vowels in the Nordic
  columns are read the same way. Remaining INFERRED cells: Denmark `@` É,
  Norway `^` Ä, France `~` (blank cell, read as no character), and the long
  dash at `~` for Finland, England and Belgium (read as an overline).
- **Character category priority table** Appendix C pp. 101–105
  (`dmp130-p101..p105-style-priority-printouts.png`): real printouts of
  `ABCDefghijk` in every combination of proportional, condensed, NLQ,
  double-strike, italic, super/subscript, elongated. This is the reference
  for style rendering (italic slant, emphasized offset, NLQ interlacing),
  though the samples are small.
- Fonts (p. 32, p. 38): standard 9×9, condensed 11×9, correspondence 19×18,
  proportional n×18, microfont = superscript-size text with half line feed.

## Status

Done from local evidence (`crates/coco-core/src/dmp_charset.rs`):

1. DMP-105 `$A0–$BF` code→symbol mapping.
2. DMP-105 `$E0–$FE` block set as 2×2 quadrants + box lines + triangles,
   printed six dots across the cell so neighbours join.
3. DMP-130 country substitutions for all 11 countries.
4. DMP-130 extended Tandy `$C0–$DF` symbols.

Still open:

5. DMP-130 proportional advances from pp. 82–83 and 86–87 (verify the
   transcribed widths and blank-column accounting).
6. Shape review of every DMP-130 glyph against the p. 81/84/85 printouts,
   and style rendering against pp. 101–105.
7. IBM sets 1 and 2 (pp. 84–85).

Cannot be done without hardware: exact per-dot bitmaps for either printer.
All glyph bitmaps in `dmp105_font.rs` and `dmp_symbols.rs` stay artistic,
and the protocol docs say so. The only route to real data is dumping the
mask ROM of an `EP-106` (105) or the DMP-130's controller from a physical
unit.

Not yet examined: `DMP-106 Operation Manual (Tandy).pdf` (may carry better
print samples of the same 105-family font), Rainbow magazine reviews of the
DMP-105 (1984) and DMP-130 (1986), which typically reproduced print samples.
