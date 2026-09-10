# DMP-130 protocol and rendering

The source is the Tandy **DMP-130 Operation Manual**, using printed page numbers.
The local scan is copyrighted and is not distributed with the source code.
The interpreter supports the Tandy data-processing (DP), word-processing (WP),
and bit-image modes, plus the printer's separate IBM emulation grammar.

## Defaults and state

The emulated physical configuration uses the factory defaults from pp. 19–20:
Tandy grammar, DP mode, Tandy characters, 10 CPI, CR with LF, LF without CR,
1/6-inch full feed, an 11-inch form, and perforation skipping disabled.
The serial baud rate is configured by the machine's serial decoder.
The printer exposes a continuous paper roll. Form feed and perforation skipping
move the print position on that roll instead of cutting separate pages.

Text is buffered until a printing control, style change, position change, or
full buffer prints it. IBM CAN discards only buffered impressions. Line feed
retains the horizontal position. Full text buffers print and perform the
configured carriage return, including the selected CR-only behavior.
Reset discards pending commands and unprinted ink but retains printed paper.
Snapshots preserve pending operands, counted graphics, buffered ink, and styles.

The interface has no physical bell, print head, or paper-out sensor. Bell,
direction, and paper-out controls are accepted without an audible or mechanical
effect. The parallel interface's inactivity flush does not apply to the serial
interface: the manual only documents that feature for parallel input on p. 48.

## Coordinates and metrics

Paper coordinates use 3,600 horizontal units and 432 vertical units per inch.
The vertical denominator represents 1/48, 1/72, 1/144, and 1/216 inch exactly.
These denominators are software choices, not claims about the stepper motor.
An internal horizontal fraction preserves condensed and NLQ coordinates without
accumulating per-character rounding. Impressions round down to paper coordinates.

The following text metrics come from pp. 44 and 53:

| Font | Dots across 8 inches | Dots per cell | Position columns |
|---|---:|---:|---:|
| Standard 10 CPI | 960 | 12 | 480 |
| Standard 12 CPI | 1,152 | 12 | 576 |
| Condensed | 1,918 | 14 | 959 |
| NLQ 10 CPI | 1,920 | 24 | 960 |
| NLQ 12 CPI | 2,304 | 24 | 1,152 |

Text position addresses every second dot column. Margins retain their physical
positions when the font changes. Superscript, subscript, and microfont preserve
the selected horizontal resolution. Microfont halves the DP line feed and uses
1/12 inch in WP mode. Full, half, and three-quarter feed selectors refer to the
base 1/6-inch setting, not to the preceding selected feed (pp. 39–42).

Tandy graphics uses 480 columns across 8 inches and seven pins. Bit 7 marks a
graphics byte; bit 0 is the top pin. `FF` prints all seven pins. Graphics LF is
7/72 inch, and graphics POS 480 advances to the next graphics line. Leaving
graphics restores the preceding DP or WP mode and text styles (pp. 59–64).

IBM graphics uses eight pins with bit 7 at the top and little-endian byte
counts. K selects 60 DPI, L and Y select 120 DPI, and Z selects 240 DPI.
Every counted payload byte is data, including escape and carriage return.
Columns beyond the printable zone are consumed without producing impressions.

## Tandy commands

Numbers in this section are hexadecimal unless identified otherwise.
`ESC` is `1B`. The command inventory follows pp. 89–92, checked against the
more detailed programming chapters on pp. 27–64.

| Command | Operation |
|---|---|
| `00`, `01`, `07` | Ignore nulls; accept bell |
| `08 n` | Backspace n text dots; graphics ignores `08` without consuming n |
| `0A`, `8A` | Flush and LF; `8A` is data in graphics |
| `0C` | Flush and feed to next top of form |
| `0D`, `8D` | Flush and configured CR; `8D` is data in graphics |
| `0E`, `0F` | Underline off, on |
| `12`, `13`, `14` | Graphics, DP, WP |
| `1C count data` | Repeat printable text or graphics data |
| `1E` | Leave graphics and restore preceding text mode |
| `7F`, `FF` | Ignore in text; `FF` is graphics data |
| `ESC 01`–`ESC 09` | Insert the specified number of text dot spaces |
| `ESC 0A`, `1C`, `1E`, `36`, `38` | Reverse full, forward half, reverse half, forward full, forward three-quarter feed |
| `ESC 0E`, `0F` | Double width on, off |
| `ESC 10 hi lo` | Absolute print position |
| `ESC 11`, `12`, `13`, `14`, `17`, `1D` | Proportional NLQ, NLQ 10 CPI, standard 10 CPI, condensed, standard 12 CPI, NLQ 12 CPI |
| `ESC 15`, `16` | CR-only, CR with LF |
| `ESC 1A`, `32`, `33`, `39` | Immediate 1/48, 1/72, 1/216, 1/144-inch feed |
| `ESC 1F`, `20` | Bold on, off |
| `ESC 21` | Flush and reset into IBM grammar |
| `ESC 34 n` | Set form to n/6 inch, with a minimum n of two |
| `ESC 3A`, `3B` | IBM character set 2, Tandy character set |
| `ESC 40 n` | Immediate n/144-inch feed |
| `ESC 42 n` | Italic on for one, off for zero |
| `ESC 48 n` | Skip n full lines at perforation; zero disables |
| `ESC 4D` | Microfont |
| `ESC 51 n`, `52 n` | Left, right margins in current font cells |
| `ESC 53 n`, `58` | Superscript for zero or subscript for one; end script |
| `ESC 55 n` | Unidirectional for zero, bidirectional for one |
| `ESC 59 n` | Select country, decimal 32–42 |

Text-only controls retain their mode restrictions. Graphics does not inherit
DMP-105-specific commands such as `ESC Z` or `ESC [`.

## IBM commands

The inventory follows pp. 67–77 and 93–99. BEL, BS, HT, LF, VT, FF, CR, SO, SI,
DC2, DC4, and CAN accept plain, high-bit, and escape-prefixed forms.
BS flushes pending text before moving back one character. HT selects the next
physical tab stop, and underline includes the skipped space. SO selects transient
double width; LF or DC4 clears it. SI and DC2 select and clear condensed printing.

| Command | Operation |
|---|---|
| `ESC !` | Flush and reset into Tandy grammar |
| `ESC - n` | Underline on for one, off for zero |
| `ESC 0`, `1` | Select 1/8-inch or 7/72-inch feed |
| `ESC A n`, `ESC 2` | Stage n/72-inch feed, then activate it |
| `ESC 3 n`, `ESC J n`, `ESC ]` | Select n/216-inch feed, immediately feed n/216 inch, reverse 1/6 inch |
| `ESC 4` | Set current position as top of form |
| `ESC C n`, `ESC C 0 n` | Set form to n lines or n inches and clear perforation skip |
| `ESC 5 n` | CR automatic LF off for zero, on for one |
| `ESC 6`, `7` | IBM character set 2, character set 1 |
| `ESC 8`, `9` | Disable, enable paper-out sensing |
| `ESC :`, `M` | 12 CPI, 10 CPI |
| `ESC D stops 0`, `R` | Replace horizontal stops; restore every eighth-column stops |
| `ESC E`, `F`, `G`, `H` | Emphasized on, off; double strike on, off |
| `ESC I n` | Standard for one, NLQ for two or three |
| `ESC P n` | Proportional on for one, off for zero |
| `ESC S n`, `T` | Superscript for zero or subscript for one; end script |
| `ESC U n` | Unidirectional for one, bidirectional for zero |
| `ESC W n` | Persistent double width on for one, off for zero |
| `ESC N n`, `O` | Skip n selected lines at perforation; clear skip |
| `ESC X left right` | Set physical margins from one-based font columns |
| `ESC <` | Home print head and select unidirectional operation until CR |
| `ESC d lo hi`, `e lo hi` | Move forward, backward in 1/120-inch units |
| `ESC ^ n` | Print assigned control-position symbol; otherwise a space |
| `ESC K/L/Y/Z lo hi payload` | Counted 60/120/120/240-DPI graphics |

## Explicit approximations and disputed documentation

The manuals provide character samples and dimensions, not ROM bitmaps. Text
uses artistic glyphs derived from the existing DMP-105 font. Normal, condensed,
and NLQ output uses the documented 9×9, 11×9, and 19×18 geometry. The 1/72-inch
normal pin spacing is inferred from p. 34's conversion of 1/6 inch to 12 dots.
The NLQ interlace, italic slant, emphasized offset, double-strike offset, and
accent shapes are rendering approximations.

The ASCII width tables are visually transcribed from Appendix A pp. 82–83.
Proportional output treats the listed widths as advances; the manual does not
independently explain their blank-column accounting. The interpreter retains
suspended style flags so that changing the applicable font can restore them.

Code-to-symbol mappings for the whole Tandy set are verified from p. 81 (the
grid and the printout below it) and p. 57 (Table 26): the European symbols
$A0–$BF are the DMP-105 table; the extended symbols $C0–$DF are
`â ê î ô û ^ ë ï á í ó ú ¡ ñ ã õ` then `Æ æ Å å Ø ø Ñ É Á Í Ó Ú ¿ Ù È Â`; the
block graphics $E0–$FE are the DMP-105 set (quadrants, box pieces,
triangles) printed six dots across the cell. All eleven country
substitutions (USA, Germany, France, Norway, Sweden, Denmark, Finland,
Italy, Spain, England, Belgium) come from Table 26. INFERRED readings, where
the typeset table drops a diacritic or leaves a cell blank: Denmark `@` É,
Norway `^` Ä, France `~` blank, and the long dash at `~` for Finland,
England and Belgium as an overline; bare A O U a o u in umlaut positions
are read as Ä Ö Ü ä ö ü, as Germany's row proves the typesetter dropped
them. IBM's extended symbol tables (pp. 84–85) remain untranscribed and
print the undefined-character placeholder. Dot patterns for every symbol
are artistic: the font lives in mask ROM and no dump exists
(`docs/dmp-font-sources.md`).

The following policies resolve contradictions explicitly:

- `ESC @ n` feeds immediately in every mode, following p. 42. Appendix p. 91
  instead describes a latched DP feed.
- `ESC 36` performs immediate full feed in WP, following p. 41's general feed
  description. Appendix p. 91 says that WP ignores it.
- IBM relative-position overflow returns to the left margin, following p. 72.
  Appendix p. 99 instead says to ignore an out-of-bounds command.
- Repeat ignores function-code operands, following pp. 92 and 107. Page 46's
  undefined-marker language is less specific.
- Tandy `ESC :` selects IBM set 2, following repeated statements on p. 49.
  Appendix p. 91 labels it set 1.
- Tandy graphics `FF` prints all pins, following the worked examples on
  pp. 61–63. The ignore entry on p. 92 is inconsistent with those examples.
- Full forms use the configured 11-inch length, following pp. 20 and 45.
  The unconditional 56-line claim on p. 107 applies poorly to fanfold paper.
- IBM Y and Z print the supplied dot masks. The manual disagrees about which
  mode forbids consecutive same-row dots, so no speculative dot suppression
  is applied.

No DMP-130 feed value establishes the disputed DMP-105 motor-step identity.
The two printers have independent command interpreters.
