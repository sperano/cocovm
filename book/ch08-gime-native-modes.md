# Chapter 8 — GIME Native Modes: Registers, Text Attributes, Graphics, Palette

*Week 8. Goal: read a `$FF90–$FF9F` register dump and describe the exact
screen it produces — columns or pixels, colours, where in RAM the data
lives — without running the emulator. Week 7 got you to the green BASIC
prompt through the CoCo's oldest trick: pretending to be a VDG. This week
you leave that pretense behind and meet the chip you actually asked to
study. Every register you POKEd as a kid to get `WIDTH 80` or `HSCREEN 2` —
and a few you never touched, because BASIC touched them for you — gets a
name, a bit layout, and a line of Rust that reads it.*

---

## 8.0 Where this picks up

Week 7 left you with a working but slightly deflating fact: the CoCo 3's
power-on BASIC prompt is not drawn by the chip this course is about. It's
VDG-compatible 32×16 text, decoded through the SAM-compat page register,
using MC6847 glyphs. The GIME renders it, but only because `INIT0` bit 7
(`COCO`) tells the GIME "pretend to be the chip you replaced."

Typing `WIDTH 80` or `HSCREEN 2` is what turns that bit off. From that
moment, a completely different register file — `$FF98` through `$FF9F` —
takes over, the GIME's own font replaces the MC6847's, and the video
hardware starts reading physical RAM directly instead of going through the
16-bit logical addresses the CPU sees. This chapter is that register file:
what each bit means, what a legal combination looks like, and how
[`crates/coco-core/src/gime_video.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs) turns eight bytes of registers plus a
block of RAM into pixels.

One boundary to hold in your head from the start, because the codebase
itself holds it as a module boundary: this chapter is about *what a single
scanline looks like given the current registers*. What happens when a 6809
program changes those registers *in the middle of a field* — so the top
half of the screen used one video base and the bottom half used another —
is next week's material, mid-frame splits and all. You'll meet the exact
mechanism that makes that possible (`FieldScan`, §8.5), but the raster
tricks it enables are week 9's.

---

## 8.1 The register file as a video-mode description language

Eight consecutive bytes, `$FF98`–`$FF9F`, completely describe a GIME-native
video mode: how many columns or pixels wide, how many colours, where the
data lives in RAM, and how it's scrolled. Nothing else — no separate
"graphics mode" enum, no derived state computed once and cached. Every
frame, every scanline, the renderer re-reads these bytes and decodes them
fresh. That's a deliberate design point worth sitting with before the bit
tables: **the registers are not configuration for the renderer, they *are*
the mode.** There is no other place in the codebase that says "we are
currently in HSCREEN 2." If you want to know the mode, you read `$FF98` and
`$FF99`, exactly like a piece of 6809 code would.

| Address | Name | Constants module | What it selects |
|---|---|---|---|
| `$FF98` | VMODE | [`vmode`] | graphics/text, burst phase, monochrome, field rate, lines-per-row |
| `$FF99` | VRES  | [`vres`]  | lines-per-field, bytes-per-row, colour depth / attribute enable |
| `$FF9A` | border | — | 6-bit border colour value |
| `$FF9B` | video bank | — | high address bits for >512K machines |
| `$FF9C` | vertical scroll | — | smooth-scroll seed (character-row line to start on) |
| `$FF9D`/`$FF9E` | vertical offset | — | physical video base ×8 |
| `$FF9F` | horizontal offset | [`hoff`] | HVEN + X-scroll, ×2 |

All eight are implemented as plain fields on the `GIME` struct
([`crates/coco-core/src/gime.rs:169-228`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L169-L228)), and the bit layouts live in small
constant modules right above it:

```rust
/// Video Mode Register ($FF98) bit assignments (SEB Unravelled II). Only meaningful
/// when INIT0 COCO=0 (GIME native modes); ignored in CoCo-compatible mode.
pub mod vmode {
    /// Bit-plane / graphics select: 1 = graphics (HSCREEN), 0 = hi-res text.
    pub const BP: u8 = 0x80;
    /// Burst phase invert (alternate composite colour set).
    pub const BPI: u8 = 0x20;
    /// Monochrome on composite output.
    pub const MOCH: u8 = 0x10;
    /// 50 Hz field rate (else 60 Hz).
    pub const H50: u8 = 0x08;
    /// Lines per character row (text modes).
    pub const LPR_MASK: u8 = 0x07;
}
```

([`crates/coco-core/src/gime.rs:70-83`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L70-L83).) That's the whole of `$FF98`: one
mode bit (`BP`), two composite-monitor bits you'll meet properly in week 9
(`BPI`, `MOCH` — this chapter only needs to know they exist and don't affect
RGB output), a field-rate bit, and a 3-bit `LPR` field packed into the low
nibble. Register `$FF99` (VRES) follows the same shape:

```rust
/// Video Resolution Register ($FF99) bit assignments (SEB Unravelled II): rows per
/// field (LPF), bytes per row (HRES), and colour depth (CRES).
pub mod vres {
    /// Lines-per-field select (bits 5–6): 192/200/210/225 rows.
    pub const LPF_MASK: u8 = 0x60;
    pub const LPF_SHIFT: u8 = 5;
    /// Horizontal resolution select (bits 2–4): sets bytes per row, not pixels.
    pub const HRES_MASK: u8 = 0x1C;
    pub const HRES_SHIFT: u8 = 2;
    /// Colour-resolution select (bits 0–1): pixels packed per byte (2/4/16 colours).
    /// In text modes (BP=0) bit 0 instead enables per-character attribute bytes.
    pub const CRES_MASK: u8 = 0x03;
    /// Text-mode attribute enable (CRES bit 0, BP=0 only).
    pub const TEXT_ATTR: u8 = 0x01;
}
```

([`gime.rs:85-99`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L85-L99).) Notice the overloading in the doc comment: `CRES` means
"how many bits per pixel" in graphics mode and "attributes on/off" in text
mode. Same two bits, same register, opposite meaning, switched by `VMODE`
bit 7. This is exactly the kind of "the datasheet says it depends" fact
that makes hand-decoding a register dump feel like archaeology the first
few times — after this chapter it won't.

Three fields — `HRES`, `CRES`, `LPR` — don't decode to a number by
arithmetic; they index a lookup table, because the chip's spacing isn't
linear or formulaic. Here they are, verbatim:

```rust
/// Active display lines per field, indexed by the VRES LPF field.
pub const LPF_LINES: [usize; 4] = [192, 200, 210, 225];

/// Lines per character row, indexed by the $FF98 LPR field. Hardware-verified
/// values from MAME `get_lines_per_row` (SEB's table says 1/2/3/8/9/10/12 but
/// the chip does 1/1/2/8/9/10/11; LPR=%111 repeats one glitched line forever,
/// approximated by a huge count so only the first row ever shows).
pub const LPR_LINES: [usize; 8] = [1, 1, 2, 8, 9, 10, 11, usize::MAX];

/// Text columns per row, indexed by the VRES HRES field (BP=0). HRES bit 1 is
/// ignored by the chip in text modes (MAME dispatches on $FF99 & 0x15), which
/// yields SEB's 32/40/32/40/64/80/64/80 table.
pub const TEXT_COLS: [usize; 8] = [32, 40, 32, 40, 64, 80, 64, 80];

/// Graphics bytes fetched per row, indexed by the VRES HRES field (BP=1).
pub const GFX_BYTES_PER_ROW: [usize; 8] = [16, 20, 32, 40, 64, 80, 128, 160];

/// Graphics bits per pixel, indexed by the VRES CRES field (BP=1): 2, 4, or 16
/// colours. CRES=%11 is undefined on the GIME; 4 bpp is the closest behaviour.
pub const GFX_BPP: [usize; 4] = [1, 2, 4, 4];
```

([`gime.rs:110-133`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L110-L133).) Two footnotes worth flagging immediately because
they'll bite you in the exercises if you don't know them going in:

1. **The SEB Unravelled II reference book's `LPR` table is wrong** — it
   lists 1/2/3/8/9/10/12; the real chip (confirmed against MAME's
   `get_lines_per_row`) does 1/1/2/8/9/10/11. The codebase follows the
   chip, not the book, and says so in the comment. That's the right call —
   but it's also a reminder that even the best available secondary source
   has a documented error, and "verify against `docs/`" (per this repo's
   house rule) sometimes means verifying `docs/` against something else.
2. **`TEXT_COLS` has only four distinct values across eight table entries**
   (32, 40, 32, 40, 64, 80, 64, 80) because `HRES` bit 1 (the middle bit of
   the 3-bit field) is architecturally ignored in text mode. `%000` and
   `%010` both mean 32 columns; `%001` and `%011` both mean 40. Compare
   `GFX_BYTES_PER_ROW`, which uses the same 3-bit `HRES` field and gets
   eight *distinct* values — graphics mode reads all three bits. Same
   register field, same bit width, different chip behaviour depending on
   `BP`. If you ever decode a text-mode register dump and get 32 columns
   when you expected 36 or some other value not in `{32, 40, 64, 80}` —
   you mis-decoded; those are the only four legal answers, by
   construction.

> **Rust corner: an array as a truth table.** Compare this decoding
> strategy to week 2's `cc` module (`ch02-registers-flags-dispatch.md`
> §2.1), which was a flat namespace of `pub const u8` bit masks tested with
> `&`. Here, three of the six register fields (`LPR`, `HRES` in text mode,
> `HRES` in graphics mode) don't decode by *masking a bit* — they decode by
> *indexing an array with the field's numeric value*. `LPR_LINES[3] == 8`
> is the entire truth table for "what does LPR=%011 mean," in one line,
> with no `match` arms to keep in sync with a datasheet table by hand. The
> alternative — a `match field { 0 => 1, 1 => 1, 2 => 2, ... }` — says the
> same thing with six more lines and no guarantee the compiler enforces
> "one arm per possible 3-bit value" (Rust *does* check `match`
> exhaustiveness, but an array literal makes the shape of the table —
> "eight entries, indexed 0–7" — visible at a glance, and a stray 9th arm
> or a gap is a compile error by construction, not a logic bug waiting to
> be found). When a register field's meaning is "look this up," not
> "compute this," an array typically reads better than a `match`. You'll
> use this same trick for the `LPF_LINES`/`vertical_window` pair in
> `raster.rs` — same shape, same reasoning.

### The two registers that bypass the MMU: `$FF9D`/`$FF9E`

Every address the CPU touches on a CoCo 3 goes through the MMU you studied
in week 5 — 8K logical slots, translated through the active task's block
table, `phys = block<<13 | addr&0x1FFF`. The GIME-native video hardware
does not do that. `$FF9D` (high byte) and `$FF9E` (low byte) together form
a 16-bit **vertical offset** register, and the physical video base is that
16-bit value, left-shifted by 3:

```rust
/// Physical start address of the GIME-native video display: the vertical
/// offset registers ×8 (any 8-byte boundary in the 512K space), plus the
/// $FF9B 512K bank on >512K machines. GIME-native scanout bypasses the MMU
/// entirely — this is a physical address (SEB Unravelled II Fig 6).
pub fn video_base(&self) -> usize {
    ((self.video_bank as usize & 0x0F) << 19) | ((self.vertical_offset as usize) << 3)
}
```

([`gime.rs:263-269`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L263-L269).) The write side confirms the byte order — high byte
first, matching the register naming (`VOFFSET1_REG` at `$FF9D` shifts left
8, `VOFFSET0_REG` at `$FF9E` sets the low byte), from
[`crates/coco-core/src/bus/io.rs:137-143`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L137-L143):

```rust
VOFFSET1_REG => {
    self.gime.vertical_offset =
        (self.gime.vertical_offset & 0x00FF) | u16::from(val) << 8;
}
VOFFSET0_REG => {
    self.gime.vertical_offset = (self.gime.vertical_offset & 0xFF00) | u16::from(val);
}
```

**Why physical, not logical?** Think back to week 5: the MMU exists
precisely so a 64K CPU can address up to 2 megabytes of installed RAM (128K
stock, 512K as Tandy actually shipped, 2048K as the GIME's MMU architecture
permits — [`crates/coco-core/src/config.rs:114-115`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/config.rs#L114-L115)), by remapping 8K
windows on demand. A hi-res graphics screen at `HSCREEN 4` occupies 30,720
bytes (you'll compute this exactly in the exercises) — under four 8K MMU
slots. If the video hardware could only read through the CPU's *current*
MMU mapping, then every field, the screen would only be visible if the CPU
happened to have those particular physical blocks mapped into its logical
window *right now* — and BASIC would have to un-map its own code and
variables to make room, or the screen would show whatever unrelated data
happened to be mapped there. Worse: a physical byte deep in bank 6 or 7 (say,
a screen the program built while task register 1 pointed elsewhere) might
never be reachable through the CPU's current logical view at all.

Physical addressing sidesteps all of it. The video base can point *anywhere*
in installed RAM — including banks the CPU isn't currently looking at —
completely independent of what the running BASIC or machine-language
program has mapped into its own 64K. This is exactly the trick that makes
double-buffering and off-screen HSCREEN pages practical on a CoCo 3: you
build the next frame in a bank the CPU can still see and edit through the
MMU, then simply repoint `$FF9D`/`$FF9E` at it — no copy, no re-mapping the
CPU's own address space, and the switch is instantaneous from the video
hardware's point of view. The MMU's whole reason to exist (letting a 64K
CPU reach more than 64K of RAM) would otherwise fight directly against the
video hardware's need to always show *some* fixed patch of that same RAM
regardless of what the CPU is doing with its window into it.

### `$FF9A`–`$FF9C`: border, video bank, smooth scroll

Three simpler registers round out the group:

- **`$FF9A` (border)** is a raw 6-bit colour *value*, not a palette
  register index — `g.color(g.border & 0x3F)` resolves it the same way a
  palette entry would, but it bypasses the 16-entry table entirely. Every
  pixel outside the active display area, every scanline, is this one
  colour.
- **`$FF9B` (video bank)** supplies address bits above bit 18 for machines
  with more than 512K installed — `(self.video_bank as usize & 0x0F) << 19`
  in `video_base()` above. On stock 128K/512K hardware it's always zero and
  irrelevant; it exists because the GIME's MMU architecture reaches 2MB and
  the video base needed a way to follow it there.
- **`$FF9C` (vertical scroll)** holds a 4-bit seed: which scanline *within*
  the first character row the field should start displaying from, for
  smooth (sub-character-row) vertical scrolling. You'll see exactly how
  `FieldScan::latch` consumes it in §8.5.

### `$FF9F`: horizontal offset, HVEN, and the 256-byte seam

The last register in the group is the one with the sharpest edge case.
`$FF9F` packs two things into one byte:

```rust
/// Horizontal Offset Register ($FF9F) bit assignments (SEB Unravelled II).
pub mod hoff {
    /// Horizontal virtual enable: rows are 256 bytes wide; the display is a
    /// scrollable window into them.
    pub const HVEN: u8 = 0x80;
    /// X0–X6 horizontal offset; ×2 gives the byte offset added within each row.
    pub const X_MASK: u8 = 0x7F;
}
```

([`gime.rs:101-108`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L101-L108).) With `HVEN` clear, every displayed row is exactly as
wide as the mode needs (say, 80 bytes for `HSCREEN 3`), and the low 7 bits
of `$FF9F` (×2, giving an even byte count) shift the *fetch* start within
that row — smooth horizontal scrolling within the visible row's own bytes.

With `HVEN` set, the semantics change: every row in RAM is treated as a
fixed 256 bytes wide regardless of how many of those bytes are actually
displayed, and the X offset scrolls a window across that wider virtual row.
This is genuinely a *virtual* screen — more data lives in RAM per row than
ever appears on screen at once, and `$FF9F` pans across it. The row pitch
used to step to the *next* row changes to match:

```rust
fn advance_scan(scan: &mut FieldScan, g: &GIME, row_bytes: usize) {
    let pitch = if g.horizontal_offset & hoff::HVEN != 0 {
        gime::HVEN_ROW_BYTES // 256
    } else {
        row_bytes
    };
    ...
}
```

([`gime_video.rs:247-258`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L247-L258), abbreviated.) And the fetch itself wraps at that
256-byte boundary no matter what:

```rust
let fetch = |i: usize| ram[(row_base + ((x_offset + i) % ROW_FETCH_WRAP)) % ram.len()];
```

([`gime_video.rs:231`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L231), where `ROW_FETCH_WRAP = 0x100`.) That modulo is not
conditioned on `HVEN` — it always wraps every fetch offset at 256 bytes
within the row, even when `HVEN` is off and the row is narrower than that.
SEB Unravelled II calls this "peculiar things happen" without `HVEN`
enabled: shift `$FF9F`'s X offset far enough on a non-`HVEN` mode and the
fetch address wraps back to the *start* of the same 256-byte span rather
than continuing to climb — the "seam." A test drills exactly this
(`horizontal_offset_shifts_fetch_with_seam_wrap`, walked in §8.7).

### Worked example: decoding `WIDTH 80`'s real register image

Time to put the whole table to use on bytes that are not synthetic test
values — this is the *actual* register image Super Extended Color BASIC
writes when you type `WIDTH 80`, verbatim from the ROM's own data table
(SEB Unravelled II, disassembly listing at `$E044`–`$E04B`, confirmed
byte-for-byte against [`crates/coco-core/tests/gime_modes.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/gime_modes.rs)'s replay of
the same sequence):

| Register | Value | 
|---|---|
| `$FF90` (INIT0) | `$4C` |
| `$FF98` (VMODE) | `$03` |
| `$FF99` (VRES)  | `$15` |
| `$FF9A` (border) | `$12` |
| `$FF9B` (video bank) | `$00` |
| `$FF9C` (scroll) | `$00` |
| `$FF9D`/`$FF9E` (offset) | `$D8`/`$00` |

Decode it register by register:

- **`$FF90 = $4C`** = `0100_1100` = `MMUEN` (`0x40`) + `MC3` (`0x08`) +
  `MC2` (`0x04`). `COCO` (`0x80`) is clear — this is a GIME-native mode,
  not CoCo-compatible.
- **`$FF98 = $03`**: `BP` (`0x80`) clear → text mode. `LPR_MASK` field =
  `%011` = 3 → `LPR_LINES[3] = 8` scanlines per character row.
- **`$FF99 = $15`** = `0001_0101`: `LPF` field (bits 5–6) = `%00` →
  `LPF_LINES[0] = 192` active lines. `HRES` field (bits 4–2) =
  `(0x15 & 0x1C) >> 2 = 5` → `TEXT_COLS[5] = 80` columns. `CRES` bit 0 = 1
  → attributes **on**.
- **`$FF9A = $12`**: border colour value `0x12` = `0b00_01_00_10`. Run it
  through the channel decode (§8.4 has the full derivation): R bits
  (5,2) = (0,0) → 0; G bits (4,1) = (1,1) → 3 → `0xFF`; B bits (3,0) =
  (0,0) → 0. **Pure green** — the exact border colour of the screen you
  remember, confirmed from the actual ROM data, not a guess.
- **`$FF9D`/`$FF9E` = `$D8`/`$00`**: `vertical_offset = 0xD800`.
  `video_base = 0xD800 << 3 = 0x6C000` — physical, bypassing the MMU.

Put together: **an 80-column, 24-row (192 ÷ 8) hi-res text screen, with
per-character attribute bytes, a pure-green border, living at physical
`$6C000`.** Attributes mean 2 bytes per cell, so each row is `80 × 2 = 160`
bytes and the whole screen occupies `160 × 24 = 3,840` bytes — physical
`$6C000` through `$6CEFF`. `HRES = 5` has its low bit set, which (§8.2)
means this is a *wide* mode: the full 640-pixel canvas, no border columns
left or right. Hold onto that `160`-byte row pitch — you'll recognize it
directly in a `render_gime.rs` test in §8.7, and it is *not* a coincidence.

---

## 8.2 Text with attributes

With `BP` clear, `$FF98`/`$FF99` describe a text mode: some number of
columns (32/40/64/80, from `TEXT_COLS`), some number of scanlines per
character row (`LPR_LINES`), and — if `CRES` bit 0 is set — a second
*attribute* byte following every character byte.

### The attribute byte

```rust
/// Attribute-byte fields (SEB Unravelled II Fig 4).
const ATTR_BLINK: u8 = 0x80;
const ATTR_UNDERLINE: u8 = 0x40;
const ATTR_FG_SHIFT: u8 = 3;
const ATTR_COLOR_MASK: u8 = 0x07;
/// Foreground colours come from palette registers 8–15.
const ATTR_FG_BASE: usize = 8;
```

([`gime_video.rs:30-36`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L30-L36).) One byte, four fields: bit 7 blink, bit 6
underline, bits 5–3 a 3-bit foreground colour (0–7, added to `ATTR_FG_BASE`
= 8 to land in palette registers 8–15), bits 2–0 a 3-bit background colour
(0–7, palette registers 0–7 directly — no offset needed since it's already
at the bottom). Sixteen palette registers, eight for background, eight for
foreground, selected by one byte per character cell. Without attributes
(`CRES` bit 0 clear), every character on screen uses one fixed pair —
palette register 1 for foreground, register 0 for background:

```rust
/// Palette registers for text without attributes: background 0, foreground 1
/// (MAME `emit_gime_text_samples`).
const NO_ATTR_BG: usize = 0;
const NO_ATTR_FG: usize = 1;
```

### The GIME's own font

Legacy VDG text (week 7) drew glyphs from `font6847.rs`, an MC6847-derived
table, 8×12. GIME-native text draws from a completely different table —
[`crates/coco-core/src/font_gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font_gime.rs), `GIME_FONT`: 128 glyphs, each exactly
8 rows tall, 8 pixels wide. The header comment is precise about what "128
glyphs" covers:

> 128 glyphs indexed by the character byte's low 7 bits. The layout is
> ASCII from `$20` up (lowercase with descenders at `$60-$7F`); `$00-$1F`
> are accented and special characters. Each glyph is 8 pixels wide by 8
> rows; text modes with more than 8 lines per row (LPR 9-12) pad below
> with blank lines.

So codes `$20`–`$7F` are exactly the ASCII printable range you'd expect —
type `'A'` (`$41`) into the text buffer and glyph index `$41` (which is
what you'd get anyway, since the font is indexed by `code & 0x7F` and `$41`
is already ≤ `$7F`) draws a capital A. Codes `$00`–`$1F` are a block of 32
special glyphs — accented letters and symbols — that have no ASCII
meaning; a raw dump of a hi-res text buffer that shows bytes in this range
is not showing you control characters, it's showing you the CoCo 3's
extended character set (the debug helper `text_lines` in the same file
prints `.` for these, rather than guessing at an ASCII interpretation —
you'll read that function in §8.6).

Every glyph is fixed at 8 rows regardless of `LPR`. When `LPR` selects a
taller cell — 9, 10, or 11 scanlines — the extra rows below the glyph are
simply blank (`line_in_row < GLYPH_ROWS` guards the lookup in
`paint_text_row`, falling back to an all-zero row otherwise). An
8-line-per-row mode (`LPR = %011`, exactly what `WIDTH 80` uses) has *no*
spare rows — which is why that mode is also the one with a defined
underline position, as you're about to see.

### The underline row, and why it depends on `LPR`

```rust
/// The scan line within a character row that the underline attribute lights,
/// per LPR — only defined for 8/9/10-line rows (SockMaster via MAME).
fn underline_line(lines_per_row: usize) -> Option<usize> {
    match lines_per_row {
        8 => Some(7),
        9 | 10 => Some(8),
        11 => Some(9),
        _ => None,
    }
}
```

([`gime_video.rs:172-179`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L172-L179).) An underline is one specific scanline — the
second-to-last or last row of the cell, depending on how tall the cell is —
forced fully lit regardless of the glyph's own pixels. For `LPR = 8` (the
`WIDTH 80` case above), that's row index 7: the very bottom scanline of the
8-tall cell. `LPR` values of 1 or 2 (the tiny cells used by non-attribute,
low-row-count modes) have no defined underline position at all — `None` —
because there's no spare scanline below the glyph to draw one on.

### Blink: driven by the GIME's own timer

Bit 7 of the attribute byte marks a character as blinking. Whether it's
*currently* visible or blanked is not decided per-character or per-frame by
the renderer — it's one shared boolean, `blink_state`, toggled elsewhere by
the GIME's 12-bit interval timer every time it underflows
([`crates/coco-core/src/gime.rs:378-394`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L378-L394), `tick_timer`, briefly: each
underflow flips `blink_state` and reloads). The timer itself, its reload
math, and its interrupt wiring are week 6's clock and (in more depth) a
later chapter's subject — what matters here is only the one line at the
call site, [`crates/coco-core/src/machine/render.rs:52-54`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L52-L54):

```rust
// Blink phase is toggled by the GIME interval timer, which BASIC
// programs at hi-res text setup (SEB Unravelled II).
let blink_on = self.bus.gime.blink_state;
```

`paint_text_row` receives that single `bool` and, per character cell,
blanks any glyph whose attribute byte has bit 7 set while `blink_on` is
true:

```rust
if attr & ATTR_BLINK != 0 && blink_on {
    code = BLANK_CHAR; // $20, a space
}
```

[`tests/gime_irq.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/gime_irq.rs)'s `rom_programs_the_timer_and_blink_phase_toggles` (not
walked in depth here — it belongs with week 6/11's timer material) confirms
the real ROM programs the timer with `$FFFF` at cold start specifically so
blinking works the instant a program sets the blink attribute, without
BASIC ever touching the timer registers itself. Note for later: that test
boots the real `coco3.rom`, so — like `gime_modes.rs` in §8.8 — it can't
actually run in a `roms/`-less checkout; more on that honestly in §8.8.

### The `WIDTH 80` HRES quirk, restated precisely

§8.1 already showed you `TEXT_COLS`'s duplicated entries
(`[32, 40, 32, 40, 64, 80, 64, 80]`). Concretely: SEB Unravelled II's own
disassembly gives `WIDTH 40` the register value `$05` for `$FF99` — `HRES`
field = `(0x05 & 0x1C) >> 2 = 1`. `WIDTH 80` uses `$15` — `HRES` field =
`(0x15 & 0x1C) >> 2 = 5`. `1` and `5` differ only in the bit-1 position of
the 3-bit `HRES` field (`0b001` vs `0b101`) — and `TEXT_COLS[1] == 40`,
`TEXT_COLS[5] == 80`, exactly the two values you'd hope for. But
`TEXT_COLS[0] == TEXT_COLS[2] == 32` and `TEXT_COLS[3] == TEXT_COLS[1] ==
40` too — the middle bit of `HRES` is architecturally don't-care in text
mode. `decodes_text_modes` in `render_gime.rs` asserts this directly:
`$FF99 = 0x08` (`HRES` field = `%010`) still decodes to 32 columns, the
same as `%000`.

Test the boundary yourself once you've read §8.7's walkthrough: nothing in
the source *computes* "ignore bit 1" as an explicit `& !0x02` — it falls
out entirely from `TEXT_COLS`'s duplicated table entries. That's a case
where "the table already encodes the quirk" beats "an explicit bit mask
plus a comment explaining why," precisely because a reader decoding a
register value never has to remember the exception — they just index the
table and get the right answer, wrong bit or not.

### Walking `paint_text_row`

Here's the function in full, with the framing already established: this
paints *one scanline* of *one text row* into the active pixel span.

```rust
fn paint_text_row(
    mode: &TextMode,
    palette: &[[u8; 4]; PALETTE_LEN],
    blink_on: bool,
    line_in_row: usize,
    active_w: usize,
    fetch: impl Fn(usize) -> u8,
    out: &mut [u8],
) {
    let bytes_per_char = if mode.attributes { 2 } else { 1 };
    let underline = underline_line(mode.lines_per_row);
    let native_w = mode.cols * CHAR_W;
    let xscale = (active_w / native_w).max(1);

    let mut x = 0;
    for col in 0..mode.cols {
        let mut code = fetch(col * bytes_per_char);
        let (fg, bg, underlined) = if mode.attributes {
            let attr = fetch(col * bytes_per_char + 1);
            if attr & ATTR_BLINK != 0 && blink_on {
                code = BLANK_CHAR;
            }
            (
                palette[ATTR_FG_BASE + ((attr >> ATTR_FG_SHIFT) & ATTR_COLOR_MASK) as usize],
                palette[(attr & ATTR_COLOR_MASK) as usize],
                attr & ATTR_UNDERLINE != 0 && !(attr & ATTR_BLINK != 0 && blink_on),
            )
        } else {
            (palette[NO_ATTR_FG], palette[NO_ATTR_BG], false)
        };

        let glyph = &GIME_FONT[(code & CHAR_CODE_MASK) as usize];
        let row_bits = if line_in_row < GLYPH_ROWS {
            glyph[line_in_row]
        } else {
            0
        };
        let underline_here = underlined && Some(line_in_row) == underline;
        for cx in 0..CHAR_W {
            let on = underline_here || row_bits & (0x80 >> cx) != 0;
            let color = if on { fg } else { bg };
            fill(&mut out[x * BYTES_PER_PIXEL..][..xscale * BYTES_PER_PIXEL], color);
            x += xscale;
        }
    }
}
```

([`gime_video.rs:303-348`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L303-L348).) Walk it in the order it executes:

1. **`bytes_per_char`** is 1 or 2 depending on whether this mode has
   attributes — everything downstream indexes `fetch` in units of this
   stride.
2. **`xscale = active_w / native_w`.** The renderer always draws at the
   font's native 8-pixel-per-character resolution, then integer-duplicates
   each pixel to fill however wide the active span actually is on the
   canonical 640-px canvas. An 80-column wide mode has `native_w = 640`,
   so `xscale = 1` — no duplication. A 40-column mode has `native_w = 320`,
   `active_w = 640` (wide modes always fill the full canvas — §8.1's
   `WIDTH 80` example established that HRES bit 0 set means wide), so
   `xscale = 2`: every native pixel becomes a 2-pixel-wide block. This is
   the same integer-scale-to-a-canonical-canvas idea `raster.rs`'s doc
   comment names "Option B" — one fixed-size framebuffer for every mode,
   geometry differences absorbed by scale factors instead of by resizing
   the canvas per mode.
3. **Per column**, `fetch(col * bytes_per_char)` reads the character code.
   If attributes are on, a second `fetch` one byte later reads the
   attribute byte, decodes `fg`/`bg`/`underlined` from it, and — this is
   the blink implementation in full — overwrites `code` to `BLANK_CHAR`
   (`$20`, a space) *before* the glyph lookup if the cell should currently
   be blanked. Blink doesn't hide pixels after the fact; it substitutes a
   different (blank) character before the glyph is even looked up.
4. **The glyph row**: `line_in_row` (which text scanline of the cell this
   is, 0-based) indexes straight into the 8-row glyph, or contributes an
   all-zero row if the cell is taller than the 8-row font (the `LPR`-9/10/11
   padding from §8.2's font discussion).
5. **The underline override**: `underline_here` is true only when this
   *specific* scanline matches the mode's defined underline row *and* the
   cell isn't currently blanked by blink (`!(attr & ATTR_BLINK != 0 &&
   blink_on)` — notice this repeats the blink condition rather than
   reusing a variable; a blinked-off cell shows nothing, not even its
   underline).
6. **Per pixel**, `on` is true if the underline forces it or the glyph's
   bit is set (`0x80 >> cx` walks MSB-first — bit 7 is the leftmost
   pixel), and `fill` writes `xscale` copies of `fg` or `bg` before
   advancing `x`.

> **Rust corner: a closure as the fetch strategy, monomorphized.** `fetch:
> impl Fn(usize) -> u8` is the same principle you met in week 1's `Bus`
> trait — `impl Trait` in a function signature resolves to one concrete,
> inlined type per call site, not a runtime-dispatched trait object. Here
> it buys something specific: `paint_text_row` and `paint_graphics_row`
> don't need to know *how* a byte gets from RAM to the caller — whether
> that's a straight index, the 256-byte seam-wrapped index from §8.1's
> `$FF9F` discussion, or (as you'll see in `Scanout` in §8.6) a
> struct-based fetch used only by the debug text dump. `paint_body_row`
> builds exactly one closure, capturing `row_base`, `x_offset`, and `ram`
> by reference, and hands it to whichever of the two paint functions the
> mode selects. One trait bound, three call sites, zero indirection at
> runtime.

---

## 8.3 Graphics: HSCREEN modes

With `BP` set, `$FF99` describes a packed-pixel bitmap instead of a
character grid: `HRES` sets bytes fetched per row (`GFX_BYTES_PER_ROW`),
`CRES` sets bits per pixel (`GFX_BPP`), and pixel width follows
arithmetically: `width = bytes_per_row * 8 / bpp`.

```rust
pub fn decode_graphics(g: &GIME) -> GraphicsMode {
    let hres = ((g.vres & vres::HRES_MASK) >> vres::HRES_SHIFT) as usize;
    let bytes_per_row = gime::GFX_BYTES_PER_ROW[hres];
    let mut bpp = gime::GFX_BPP[(g.vres & vres::CRES_MASK) as usize];
    if bytes_per_row > gime::GFX_BYTES_PER_ROW[5] && bpp == 1 {
        bpp = 2;
    }
    GraphicsMode {
        bytes_per_row,
        bpp,
        width: bytes_per_row * 8 / bpp,
        lines: g.lines_per_field(),
    }
}
```

([`gime_video.rs:102-115`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L102-L115).) That `if` is not decorative — it's a
documented hardware quirk. `HRES = %110`/`%111` (128/160 bytes per row)
combined with `CRES = %00` (1 bpp) is, per SEB Unravelled II Figure 5, "not
a guaranteed combination": the chip does not actually produce a
1024-or-1280-pixel-wide picture. MAME's `gime.cpp` aliases that specific
combination to the `CRES = %01` (2 bpp) renderer instead, and this codebase
follows suit — a test, `cres0_at_128_and_160_bytes_aliases_to_2bpp`, pins
it down (walked in §8.7). This is the graphics-mode analogue of the
text-mode `HRES`-bit-1 quirk from §8.2: a documented hardware edge case
that the decode function must special-case explicitly, because — unlike
the text case — there's no way to bake it into the lookup tables alone
(the aliasing is conditional on *two* fields at once, `HRES` and `CRES`
together, not a single field's own don't-care bit).

### Bits per pixel, MSB first: walking `paint_graphics_row`

```rust
fn paint_graphics_row(
    mode: &GraphicsMode,
    palette: &[[u8; 4]; PALETTE_LEN],
    active_w: usize,
    fetch: impl Fn(usize) -> u8,
    out: &mut [u8],
) {
    let pixels_per_byte = 8 / mode.bpp;
    let value_mask = (1u8 << mode.bpp) - 1;
    let xscale = (active_w / mode.width.max(1)).max(1);

    let mut x = 0;
    for bx in 0..mode.bytes_per_row {
        let byte = fetch(bx);
        for j in 0..pixels_per_byte {
            // Pixels are packed MSB-first within the byte.
            let shift = 8 - mode.bpp * (j + 1);
            let color = palette[((byte >> shift) & value_mask) as usize];
            if x + xscale > active_w {
                return; // defensive: never paint past the active span
            }
            fill(&mut out[x * BYTES_PER_PIXEL..][..xscale * BYTES_PER_PIXEL], color);
            x += xscale;
        }
    }
}
```

([`gime_video.rs:352-377`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L352-L377).) Compare its shape to `paint_text_row` — same
`xscale` idea, same `fetch`/`fill` vocabulary — but the inner loop is pure
bit arithmetic instead of a font lookup, because a graphics pixel *is* its
own palette index; there's no glyph between the byte and the screen.

Trace `j` for a 4-bpp byte (`HSCREEN 2`, `mode.bpp = 4`): `pixels_per_byte
= 8/4 = 2`, `value_mask = 0b1111`. For `j = 0`: `shift = 8 - 4*1 = 4` — the
*high* nibble. For `j = 1`: `shift = 8 - 4*2 = 0` — the *low* nibble. So
pixel 0 of the byte comes from bits 7–4, pixel 1 from bits 3–0: **MSB
first**, exactly matching how you'd hand-encode HSCREEN pixels by shifting
the first pixel of a pair into the top nibble. For a 1-bpp byte (`HSCREEN
3`), `pixels_per_byte = 8`, and the eight shifts run `7, 6, 5, ..., 0` —
each individual bit, left to right, same MSB-first convention. Each decoded
`value` (0–15, 0–3, or 0–1 depending on `bpp`) indexes directly into
`palette` — no offset, no `ATTR_FG_BASE`-style adjustment; graphics mode
uses the palette registers starting at 0, and how many of the 16 are
actually reachable is purely a function of `bpp` (a 1-bpp mode can only
ever produce indices 0 or 1, so registers 2–15 are simply never selected).

The defensive `if x + xscale > active_w { return; }` is worth noting for
what it's *not*: it's not part of the hardware's own behaviour, it's a
guard against a decode producing more pixels than the active span has room
for (which shouldn't happen for any of the legal `HRES`/`CRES`
combinations this decode table produces, given the aliasing fix above —
but the guard costs one comparison and turns a would-be out-of-bounds
panic into a silently-truncated row instead, which is the right failure
mode for a renderer: better a slightly wrong picture than a crashed
emulator).

### HSCREEN 1–4: the ROM's own register images

`HSCREEN n` in BASIC doesn't compute a register value at runtime — it
looks one up. SEB Unravelled II's disassembly shows the exact table
(`$E06C`, labelled `RESTABLE`) the ROM indexes with `HSCREEN`'s argument
minus one:

```
* VIDEO RESOLUTION MODE REGISTER (FF99) DATA FOR HSCREEN MODES
RESTABLE FCB   $15        320 PIXELS, 4 COLORS
         FCB   $1E        320 PIXELS, 16 COLORS
         FCB   $14        640 PIXELS, 2 COLORS
         FCB   $1D        640 PIXELS, 4 COLORS
```

Decode each through the same `decode_graphics` machinery §8.3 just walked,
and the ROM's own comments check out exactly:

| HSCREEN | `$FF99` | `HRES` field | bytes/row | `CRES` field | bpp | colours | width |
|---|---|---|---|---|---|---|---|
| 1 | `$15` | `%101` (5) | 80 | `%01` | 2 | 4 | 320 |
| 2 | `$1E` | `%111` (7) | 160 | `%10` | 4 | 16 | 320 |
| 3 | `$14` | `%101` (5) | 80 | `%00` | 1 | 2 | 640 |
| 4 | `$1D` | `%111` (7) | 160 | `%01` | 2 | 4 | 640 |

Every `HRES` field here (`%101` and `%111`) has its low bit set — every
`HSCREEN` mode is a *wide* mode (§8.1's `WIDTH 80` example already showed
you what that means: full 640-px canvas, no side border). And notice
`HSCREEN 1` and `HSCREEN 3` share the same `HRES` field (5, 80 bytes/row)
despite different pixel widths (320 vs 640) — because `bpp` differs (2 vs
1); the *byte* bandwidth is identical, only how those bytes get sliced into
pixels changes. Same relationship between `HSCREEN 2` and `HSCREEN 4`
(both `HRES` field 7, 160 bytes/row). Bytes per row is set by `HRES` alone
— `CRES` trades that fixed byte bandwidth for either more colours per pixel
(fewer, wider pixels) or more pixels (narrower, fewer colours). Keep that
relationship in mind for Exercise 8.2.

The ROM's video-mode RAM images (`IM.GRAPH`/`E079`-labelled block, SEB
Unravelled II) confirm the video base too: both the 320-wide and 640-wide
hi-res graphics images write `$FF9D = $C0`, `$FF9E = $00` —
`vertical_offset = 0xC000`, `video_base = 0xC000 << 3 = 0x60000`. Every
`HSCREEN` mode BASIC sets up lives at the same fixed physical address; only
the resolution registers change which byte-and-bit-depth interpretation is
applied to the data found there.

### Worked example: `HSCREEN 3`'s register image

Full register dump this time, same treatment as `WIDTH 80` got in §8.1:

| Register | Value |
|---|---|
| `$FF90` | `$4C` |
| `$FF98` | `$80` |
| `$FF99` | `$14` |
| `$FF9D`/`$FF9E` | `$C0`/`$00` |

- **`$FF98 = $80`**: `BP` set → graphics. `LPR` field = `%000` →
  `LPR_LINES[0] = 1` — one scanline per fetched row, unlike text's 8. This
  makes sense once you see it: a graphics "row" is a single line of
  pixels, redrawn fresh every scanline; there's no multi-scanline
  character cell to repeat.
- **`$FF99 = $14`**: from the table above — 80 bytes/row, 1 bpp (2
  colours), 640 px wide, `LPF` field `%00` → 192 active lines.
- **`$FF9D`/`$FF9E` = `$C0`/`$00`**: `video_base = 0xC000 << 3 = 0x60000`.

**640×192, 2-colour (1 bpp) graphics, physical base `$60000`.** Total
screen memory: `80 bytes/row × 192 rows = 15,360 bytes` (`$3C00`) —
physical `$60000` through `$63BFF`. Palette registers 0 and 1 are the only
ones a 2-colour mode can ever select (§8.3's "no offset, indices 0–1 only"
point). This is a genuinely tiny picture by modern standards — fifteen
kilobytes for a full monochrome hi-res screen — and yet, per §8.1's MMU
discussion, still needed the physical-addressing trick to be usable
alongside a running BASIC program without a constant remapping dance.

---

## 8.4 The palette

Sixteen registers, `$FFB0`–`$FFBF`, each holding one 6-bit colour value.
Every foreground, background, and border colour the GIME-native renderer
ever draws is one of these sixteen entries (plus the border register's own
independent 6-bit value, which isn't a palette index at all — §8.1).

### Which registers a mode actually uses

SEB Unravelled II's Figure 13 tabulates it precisely, and it matches
exactly what §8.2's `ATTR_FG_BASE`/`NO_ATTR_FG`/`NO_ATTR_BG` constants and
§8.3's un-offset graphics indexing already told you in code form:

| Mode | Registers used |
|---|---|
| Hi-res text, no attributes | bg = reg 0, fg = reg 1 |
| Hi-res text, with attributes | bg = regs 0–7, fg = regs 8–15 |
| Hi-res graphics, 16 colour | 0–15 |
| Hi-res graphics, 4 colour | 0–3 |
| Hi-res graphics, 2 colour | 0–1 |

(The legacy 32×16 VDG-compatible text mode from week 7 is a special case
outside this table entirely — it fixes background to register 12 and
foreground to register 13, regardless of `BP`/attributes, because it isn't
reading these decode paths at all.)

### The 6-bit `RGBrgb` format

```rust
/// Convert a 6-bit GIME palette value to RGBA. The register format is
/// `RGBrgb` (two bits per channel); each channel scales `0..3` to `0..0xFF`
/// via `×0x55` — matching the GIME's RGB output (MAME `gime.cpp`).
pub fn rgb_color(value: u8) -> [u8; 4] {
    let chan = |hi_bit: u8, lo_bit: u8| {
        let v = ((value >> hi_bit) & 1) << 1 | ((value >> lo_bit) & 1);
        v * 0x55
    };
    [chan(5, 2), chan(4, 1), chan(3, 0), 0xFF]
}
```

([`crates/coco-core/src/gime/palette.rs:56-62`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs#L56-L62).) Six bits, three channels,
two bits each — hence "`RGBrgb`": the *high* half of each channel's 2-bit
value comes from one bit group (bits 5, 4, 3 — capital `RGB`), the *low*
half from another (bits 2, 1, 0 — lowercase `rgb`). SEB Unravelled II
states the bit assignment directly:

```
Bit 5   (R1) High Order Red
Bit 4   (G1) High Order Green
Bit 3   (B1) High Order Blue
Bit 2   (R0) Low Order Red
Bit 1   (G0) Low Order Green
Bit 0   (B0) Low Order Blue
```

— which is exactly `chan(5, 2)` for red (bit 5 high, bit 2 low), `chan(4,
1)` for green, `chan(3, 0)` for blue, matching the code's argument order
precisely. Each channel's 2-bit result (0–3) scales to a full byte by
multiplying by `0x55` (`0x55 × 3 = 0xFF`) — the only way to spread 4 evenly
spaced values across 0–255 using integer multiplication with no gaps at
either end.

**Worked example**, straight from SEB Unravelled II's own colour-derivation
walkthrough: build "purple" as Red=2, Green=1, Blue=3 (each 0–3). Per the
bit table, that's bit5,bit2 = the 2-bit pattern for R=2 (`10`), bit4,bit1 =
G=1 (`01`), bit3,bit0 = B=3 (`11`) — assembled as `bit5 bit4 bit3 bit2 bit1
bit0` = `1 0 1 0 1 1` = `0b101011` = **43 decimal**, exactly the value SEB
gives. Running `43` through `rgb_color`: `chan(5,2)`: bit5=1, bit2=0 → `v =
1<<1|0 = 2` → `R = 2×0x55 = 0xAA`. `chan(4,1)`: bit4=0, bit1=1 → `v =
0<<1|1 = 1` → `G = 0x55`. `chan(3,0)`: bit3=1, bit0=1 → `v = 1<<1|1 = 3` →
`B = 0xFF`. Result: `(0xAA, 0x55, 0xFF)` — a genuinely purple-leaning blue,
matching "mostly blue, a little red, a little green" exactly as the
strengths said. This is not a contrived example — it's the book's own
illustration, verified line-for-line against the function that actually
ships.

This is the **RGB-monitor** path only. A composite monitor resolves the
same 6-bit value through an entirely different, hand-measured 64-entry
lookup table (`COMPOSITE_PALETTE`/`COMPOSITE_PALETTE_180` in the same
file) — because composite colour isn't a linear function of the register
bits at all, it's an artifact of how the CoCo's composite encoder
modulates those bits onto a colour subcarrier. That table, the `BPI`
burst-phase-invert bit `$FF98` mentioned back in §8.1, and the `MOCH`
greyscale averaging are all week 9's material in full; this chapter's job
was only to establish that the RGB path — the one every worked example
above used — is a clean, derivable formula, and the composite path is not.

### Caching: `resolve_colors`, once per scanline

```rust
fn resolve_colors(g: &GIME) -> ([[u8; 4]; PALETTE_LEN], [u8; 4]) {
    let mut palette = [[0u8; 4]; PALETTE_LEN];
    for (entry, &reg) in palette.iter_mut().zip(&g.palette) {
        *entry = g.color(reg);
    }
    (palette, g.color(g.border & BORDER_COLOR_MASK))
}
```

([`gime_video.rs:162-168`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L162-L168).) `paint_scanline` calls this once, at the top of
every scanline ([`gime_video.rs:273`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L273)), converting all sixteen palette
registers plus the border to resolved RGBA up front, then hands the
resulting sixteen-entry array down into whichever of `paint_text_row` /
`paint_graphics_row` the mode selects — which is why those functions take
`palette: &[[u8; 4]; PALETTE_LEN]` rather than the raw 6-bit register
array plus a `GIME` reference. Per-*pixel* colour resolution would mean
running the `×0x55` arithmetic (or a 64-entry composite lookup) up to 640
times per scanline for what's actually only ever 16 distinct answers.
Once per scanline is still "live" in the sense that matters for this
chapter — change a palette register mid-field and the *next* scanline
picks it up — but it's the coarsest grain that still gets that right.

### What `COLOR`/`PALETTE` actually write

BASIC's `PALETTE` command (SEB Unravelled II, `$E60C` on) is a direct
register poke with bounds checking, and its own comments are worth reading
verbatim because of one detail that ties directly back to §8.5's live/latch
distinction:

```
LDA   BINVAL+1      GET THE NUMBER OF THE PALETTE REGISTER TO CHANGE
CMPA  #16           16 PALETTE REGISTERS MAXIMUM
LBCC  ILLFUNC       ILLEGAL FUNCTION CALL ERROR IF PALETTE REGISTER > 15
...
LDB   VERBEG+1      GET THE NEW COLOR FOR THE PALETTE REGISTER
CMPB  #63           MAXIMUM OF 64 COLORS (ZERO IS A LEGIT COLOR)
BLS   LE62A         BRANCH IF LEGITIMATE COLOR SELECTED
LDB   #63           USE COLOR 63 IF BAD COLOR NUMBER SELECTED
LE62A ORCC  #$50    DISABLE INTERRUPTS
SYNC                WAIT FOR AN INTERRUPT TO CHANGE PALETTE REGISTERS - THIS WILL
                     PREVENT THE SCREEN FROM FLASHING WHEN THE CHANGE IS MADE.
STB   ,X            SAVE THE NEW COLOR IN THE PALETTE REGISTER
```

The ROM validates the register index (error on ≥16) and clamps an
out-of-range colour to 63 rather than erroring — then deliberately masks
interrupts and executes `SYNC` (a full CPU halt-until-interrupt, the same
instruction from week 4) *before* writing the register, specifically so the
write lands during vertical blanking instead of mid-scanline. The comment
says exactly why: writing a palette register while the beam is partway
through drawing it *would* be visible — a colour flash or tear at whatever
scanline the write happened to land on — because §8.4's own
`resolve_colors` re-reads live registers every scanline, on purpose, so
that mid-frame changes *do* take effect immediately. That immediacy is a
feature (week 9's raster tricks depend on exactly this), but it means a
*careless* write is visible too, and here's the ROM's authors, in 1986,
already reaching for the same "sync your writes to blanking" discipline
every raster-effects programmer eventually learns. `COLOR`, similarly,
resolves to writes at fixed register pairs (background/foreground for
whichever mode is active per Figure 13 above) rather than a general
register+value pair like `PALETTE`.

[`crates/coco-core/examples/palette_trace.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/palette_trace.rs) is a small debugging harness
built around exactly this fact — it boots a cartridge, single-steps the
machine, and logs every change to `bus.gime.palette` together with the PC
that caused it, specifically to let you watch a real program's palette
discipline (or lack of it) scroll past. It needs a real ROM and cartridge
image to run, so it's a skim-only mention here, not a lab exercise — but
it's worth knowing it exists the next time you're debugging a colour glitch
in a real program.

---

## 8.5 `FieldScan`: what's latched, what's live

Every register this chapter has covered gets read by `paint_scanline`
*fresh, every single scanline* — `$FF98`, `$FF99`, `$FF9A`, `$FF9F` are all
consulted live, straight off the `GIME` struct, no caching beyond §8.4's
per-scanline colour resolve. A 6809 program that changes the border colour,
the mode bits, or the horizontal offset between one scanline and the next
sees that change take effect starting on the very next line painted. This
is the mechanism (not yet the *use* of the mechanism — that's next week)
behind raster splits: because nothing about per-line painting requires the
registers to hold still for a whole field, nothing stops a program from
changing them mid-field either.

But not everything is read live. Three pieces of state are sampled exactly
once, at the very start of a field, and held fixed no matter what the
registers do afterward — mirroring, deliberately, what MAME's `gime.cpp`
calls `new_frame`:

```rust
/// Per-field video scanout state, latched at field start — the register group
/// MAME `gime.cpp` `new_frame` samples once per field and never re-reads
/// mid-frame: the video base address ($FF9D/$FF9E), the INIT0 COCO
/// legacy-vs-GIME switch, and the VSC smooth-scroll seed ($FF9C). Everything
/// else ($FF98/$FF99 mode bits, $FF9F offset/HVEN, $FF9A border) is read live
/// per line by [`paint_scanline`].
#[derive(Serialize, Deserialize)]
pub struct FieldScan {
    pub legacy: bool,
    pub(crate) row_base: usize,
    pub(crate) line_in_row: usize,
}
```

([`gime_video.rs:117-140`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L117-L140), doc comment and fields.) `legacy` is the `INIT0
COCO` bit's value *at the moment the field started* — if a program flips
into or out of CoCo-compatible mode mid-field, the switch doesn't take
effect until the *next* field begins, so a field never has to answer "am I
painting legacy VDG-style or GIME-native halfway through my own body."
`row_base` seeds from `video_base()` (§8.1's physical-address computation)
exactly once. `line_in_row` seeds from the `$FF9C` vertical-scroll register
— but only if it's a valid starting point for the *current* `LPR`:

```rust
pub fn latch(g: &GIME, legacy: bool) -> Self {
    let vsc = (g.vertical_scroll & 0x0F) as usize;
    let lpr = g.lines_per_row();
    Self {
        legacy,
        row_base: if legacy { g.sam_display_base() as usize } else { g.video_base() },
        line_in_row: if legacy || vsc >= lpr { 0 } else { vsc },
    }
}
```

([`gime_video.rs:146-158`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L146-L158).) That guard — `vsc >= lpr` falls back to 0 — is
what §8.7's `vertical_scroll_starts_field_mid_character_row` test exercises:
setting `$FF9C` to a value the current `LPR` can't honour (say, scrolling 5
lines into a 3-line-per-row cell) doesn't panic or wrap, it's simply
ignored for that field.

`row_base` and `line_in_row` aren't read-only after latching, either —
`advance_scan` (§8.1's `$FF9F` discussion already showed you its pitch
calculation) mutates them as painting proceeds down the field, stepping
`row_base` forward by the current row's pitch every `lines_per_row(g)`
scanlines. So `FieldScan` is best understood as *two* different things
bundled in one struct: the truly field-frozen part (`legacy`, and the
*starting* values `row_base`/`line_in_row` were seeded from) and a running
cursor that the per-line painter is free to advance using registers it
reads live. The freezing happens exactly once, at latch time; the
advancing happens every line, using whatever `$FF98`/`$FF9F` say *right
now*.

The machine loop's own call site pins down exactly when "field start"
means, in scanline terms — [`crates/coco-core/src/machine/render.rs:29-40`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L29-L40):

```rust
pub(super) fn render_scanline(&mut self) {
    if self.config.variant != MachineVariant::Coco3 {
        return;
    }
    if self.line == 0 {
        let legacy = self.bus.gime.init0 & gime::init0::COCO != 0;
        self.field_scan = Some(gime_video::FieldScan::latch(&self.bus.gime, legacy));
        ...
    }
    ...
}
```

`render_scanline` runs once per scanline, called from `end_of_line`
(week 6's territory) — and the latch happens inline, on line 0's own call,
not in some separate "start of field" hook. The doc comment on this
function is candid about the one-line-time consequence: "one line-time
later than MAME's field start, within the plan's line-granular contract."
That's exactly the kind of small, documented divergence-from-a-reference-
implementation this course has flagged before (week 4's testing strategy
chapter, Appendix A) — a deliberate simplification, not an oversight, and
one line of vertical blanking timing difference has never mattered for any
test in this codebase.

This is as far as this chapter goes with `FieldScan`. What you now know:
which registers freeze at field start and which don't, and the exact
function that does the freezing. What you don't yet know — and won't,
until next week — is what a program can *do* with that knowledge: a
horizontal border-colour split timed off a scanline-count interrupt, a
mid-field `HSCREEN` mode change, or any of the raster tricks that made CoCo
3 demos worth watching. [`tests/scanline_split.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/scanline_split.rs), next week's material, is
where that story is told through actual 6809 code running in ROM.

---

## 8.6 The lab: `gime_demo.rs`

[`crates/coco-core/examples/gime_demo.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/gime_demo.rs) is a from-scratch harness — no
ROM, no ROM-derived RAM state, just a `GIME` struct configured directly
from Rust and a block of RAM you fill by hand. It exercises exactly the two
paths this chapter covered: an 80-column attribute text screen, and an
`HSCREEN 2` colour-bar graphics screen. Run it yourself:

```
cargo run -p coco-core --example gime_demo /tmp
```

This genuinely runs with nothing but `cargo` and the crate — no `roms/`
directory needed, confirmed by actually running it while writing this
chapter. It prints:

```
wrote /tmp/text80.ppm (640x240)
wrote /tmp/hscreen2.ppm (640x240)
```

Both PPMs are the full canonical 640×240 raster you'd get from the real
machine loop, produced by the same `render_field` you've already read
(§8.1's `WIDTH 80` and §8.3's `HSCREEN 3` worked examples both used the
functions this calls). Look at the text-mode setup first:

```rust
let mut g = GIME::new();
g.vmode = 0x03; // BP=0, LPR=8
g.vres = 0x15; // 80 cols, attributes
g.vertical_offset = (base >> 3) as u16;
g.border = 0x12;
g.palette = [0, 9, 18, 27, 36, 45, 54, 63, 0, 63, 46, 26, 12, 5, 38, 56];
```

`vmode`/`vres`/`border` are exactly the `WIDTH 80` values you decoded by
hand in §8.1 — this example uses the real ROM's own register image, not a
made-up one. The message string is written 24 rows deep, one attribute
byte per character choosing a foreground colour that cycles with the row
number, an underline forced on row 4, blink forced on row 5:

```rust
let mut attr = (fg << 3) | bg;
if row == 4 { attr |= 0x40; } // underline
if row == 5 { attr |= 0x80; } // blink
```

Rendered, this produces a green-bordered screen with 24 rows of the phrase
"cocovm GIME 80-column text ABCDEFGHIJKLMNOPQRSTUVWXYZ ..." scrolling
across each row at a one-character offset, each row a different foreground
colour, row 4 underlined, row 5 blinking (rendered with `blink_on = false`
in the call, so it appears normally in this particular PPM — flip that
argument and row 5 goes blank, exactly like `blink_attribute_blanks_...`
in §8.7 tests). The `HSCREEN 2` half switches `vmode`/`vres` to the exact
values from §8.3's HSCREEN table and fills a 320×192 grid with vertically
drifting colour bars — sixteen columns of the full palette, each column's
colour index nudged by `y / 12` to make the drift visible.

### Try it yourself: put your name on screen, with blink

Modify the harness to render your own message instead of the canned
string, and — this is the point of the exercise, not incidental — make
your own name (not the rest of the line) blink, by controlling exactly
which columns get the blink attribute bit rather than applying it to a
whole row:

```rust
let msg = b"YOUR NAME HERE is typing on a CoCo 3";
let blink_start = 0; // column where your name begins
let blink_end = 14;  // column just past your name
for row in 0..24 {
    for col in 0..80 {
        let i = base + (row * 80 + col) * 2;
        ram[i] = msg[(col + row) % msg.len()];
        let fg = (row % 8) as u8;
        let bg = if row >= 16 { (row % 8) as u8 } else { 0 };
        let mut attr = (fg << 3) | bg;
        if (blink_start..blink_end).contains(&col) {
            attr |= 0x80;
        }
        ram[i + 1] = attr;
    }
}
```

Render it twice — once with `render_field(&g, &ram, false, &mut fb)`, once
with `true` for the blink argument — and diff the two PPMs (or just look at
them): only the columns you marked should change between the two images.
If the whole screen goes blank instead, you've marked the attribute byte
for every column, not just the name — go back to §8.2's attribute-byte
walkthrough and check which bits you're actually setting per column versus
per row.

---

## 8.7 Reading the tests: `render_gime.rs`, predicted and checked

[`crates/coco-core/tests/render_gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_gime.rs) is deterministic, ROM-free coverage
for everything this chapter has walked through — decode functions, RAM
scanout, attribute colours, blink, underline, bit unpacking. Read three of
them the way the test file itself was meant to be read: predict the pixel,
then check the assertion.

### `text_without_attributes_uses_palette_0_and_1`

```rust
let g = gime_with(TEXT_LPR8, VRES_TEXT40); // 40 cols, no attributes
let mut ram = vec![0u8; RAM_LEN];
ram[BASE] = b'A';

let mut fb = Vec::new();
let (fb_w, fb_h) = render_field(&g, &ram, false, &mut fb);
```

40 columns is a *non-wide*... no — check `VRES_TEXT40 = 0x04`: `HRES`
field = `(0x04 & 0x1C) >> 2 = 1`, which per §8.1's `TEXT_COLS` table means
40 columns, and `HRES` field 1 (`%001`) *does* have its low bit set — this
is wide (§8.1 established wide = `HRES` field's bit 0). So `native_w = 40 ×
8 = 320`, `active_w = 640` (full canvas, no border), `xscale = 2`. `'A'` is
`$41`; `GIME_FONT[0x41]`'s row 0 is `0x10` — bit 4 set, meaning pixel index
3 (0-based from the left, since bit 7 is pixel 0) is lit. At `xscale = 2`,
native pixel 3 covers canvas x = 6 and 7. Predict: `px(&fb, 6, TOP)` and
`px(&fb, 7, TOP)` should both be the foreground colour (palette register 1,
since no attributes means the fixed `NO_ATTR_FG`/`NO_ATTR_BG` pair from
§8.2), and `px(&fb, 0, TOP)` should be background (register 0). The test
confirms exactly this — `assert_eq!(px(&fb, 3 * 2, TOP), fg)` and `px(&fb,
3 * 2 + 1, TOP)`, i.e. canvas x 6 and 7, both `GIME::rgb_color(1)`. It also
checks the border, `px(&fb, 0, 0)` (canvas row 0, well above `TOP = 25`,
the first active row for `LPF = 0`'s 192-line body) equals
`GIME::rgb_color(0)` — border defaults to whatever `g.border` was left at
(`0` in `gime_with`'s helper), which happens to equal palette register 0's
value under this test's identity-ish palette setup (`register i holds
colour value i`), but conceptually it's the independent `$FF9A` value from
§8.1, not the background palette register — a coincidence of this
particular test's setup, not a hardware relationship.

### `graphics_unpacks_4bpp_msb_first_from_physical_base`

```rust
let g = gime_with(GFX, VRES_320X16); // HSCREEN 2 shape: 160 bytes/row, 4bpp, 320px
let mut ram = vec![0u8; RAM_LEN];
ram[BASE] = 0x5C; // pixels 5, 12
ram[BASE + 160] = 0x70; // row 1 first pixel = 7
```

`0x5C = 0b0101_1100`. Per §8.3's MSB-first walk for 4 bpp: pixel 0 is the
high nibble (`0101 = 5`), pixel 1 is the low nibble (`1100 = 12`). At
`HSCREEN 2`'s shape, `width = 320`, `active_w = 640`, `xscale = 2` — so
native pixel 0 (value 5) covers canvas x 0–1, native pixel 1 (value 12)
covers canvas x 2–3. Predict `px(&fb, 0, TOP) == px(&fb, 1, TOP) ==
GIME::rgb_color(5)` and `px(&fb, 2, TOP) == GIME::rgb_color(12)` — which is
exactly what the test asserts, plus one more check: `ram[BASE + 160]`
(second row, since a 4-bpp/160-bytes-per-row `HSCREEN 2` row is 160 bytes
and `LPR = 1` for graphics means every scanline is a new row — §8.3's
worked example established `LPR_LINES[0] = 1` for graphics) should appear
at canvas row `TOP + 1`, confirming the row pitch is exactly 160 — not a
guess, the byte offset the test itself chose to prove it.

### `horizontal_offset_shifts_fetch_with_seam_wrap`

```rust
let mut g = gime_with(GFX, VRES_320X16);
g.horizontal_offset = 0x80 | 0x01; // HVEN on, X=1 -> byte offset 2
let mut ram = vec![0u8; RAM_LEN];
ram[BASE + 2] = 0xF0;         // lands at pixel 0 with the 2-byte shift
ram[BASE + 256 + 2] = 0x90;   // row 1: HVEN pitch 256, same 2-byte shift
```

This is §8.1's `HVEN` seam mechanism, exercised directly. `X_MASK` field =
1, and the offset is "×2 gives the byte offset" per the `hoff` module doc
— so `x_offset = 1 * 2 = 2` bytes. With `HVEN` on, `fetch(i)` reads
`ram[row_base + (x_offset + i) % 256]` — the byte at `row_base + 2`
(`i = 0`) is what lands at pixel 0. `0xF0`'s high nibble is `0xF = 15`,
predict `px(&fb, 0, TOP) == GIME::rgb_color(15)` — confirmed. The *row
pitch* under `HVEN` is fixed at `HVEN_ROW_BYTES = 256` regardless of the
mode's actual `bytes_per_row` (80 for this shape) — so row 1's data lives
at `row_base + 256`, and the test plants it at `+256 +2` to match the same
2-byte X shift, predicting `px(&fb, 0, TOP + 1) == GIME::rgb_color(9)`
(`0x90`'s high nibble) — also confirmed. Nothing here demonstrates the
*wrap* itself failing to happen (that would need an X offset large enough
to walk past byte 255 within the row, which this test doesn't push) — but
it does confirm the two load-bearing facts §8.1 claimed: the offset is in
2-byte units, and `HVEN` changes the row-to-row pitch independently of the
mode's own byte width.

---

## 8.8 `gime_modes.rs`: the real ROM, honestly

[`crates/coco-core/tests/gime_modes.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/gime_modes.rs) is the test this chapter's syllabus
entry calls out by name — it boots the actual `coco3.rom`, runs it to the
BASIC prompt, then pokes the *exact* register sequences from §8.1 and
§8.3's worked examples (the `WIDTH 80` and `HSCREEN 2` images, verbatim)
directly at the live machine, and checks the framebuffer dimensions stay
locked to the canonical 640×240 canvas across the switch. It's an
integration test in the fullest sense available to this course: real ROM
bytes, a real boot, a real register-write sequence a real BASIC program
would issue.

Be honest about what you can do with it right now, in *this* checkout:
`roms/` is one of the two directories this repository deliberately keeps
out of version control (`docs/`, PDFs, is the other) because their
contents are copyrighted — Tandy's ROM images, not this project's code.
Running it here:

```
$ cargo test -p coco-core --test gime_modes
```

fails immediately, not on an assertion but on the fixture load itself:

```
cannot read .../crates/coco-core/../../roms/coco3.rom: No such file or directory (os error 2)
```

That's not a bug to chase — it's the expected result of working in a
checkout (or, as here, a course-writing worktree) that never had `roms/`
populated. On a machine where you've legitimately obtained a CoCo 3 ROM
image and placed it at `roms/coco3.rom` (repository root, sibling to
`crates/`), the same command boots the real 32K Super Extended Color BASIC,
runs 120 fields to reach the idle prompt, then executes exactly the
register writes you already hand-decoded in §8.1 and §8.3:

```rust
// The ROM's WIDTH 80 register image: COCO off, BP=0 LPR=8, 80 cols with
// attributes, video base $6C000 ($FF9D:$FF9E = $D80:0 ×8).
m.bus.write(0xFF90, 0x4C);
m.bus.write(0xFF98, 0x03);
m.bus.write(0xFF99, 0x15);
m.bus.write(0xFF9D, 0xD8);
m.bus.write(0xFF9E, 0x00);
m.run_field();
```

If you have the ROM available, run it — there is real pedagogical value in
watching the *exact* bytes you decoded by hand actually flip a live,
booted machine's video mode. If you don't, you've still done the
substantive work: everything this test *checks* (framebuffer stays
640×240 across the mode switch — "one stable texture size across every
CoCo 3 mode is the point of Option B," per the test's own comment) is a
narrower claim than what §8.1's worked example already walked by hand
(exactly which columns, colours, and physical address those bytes
produce). [`tests/gime_irq.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/gime_irq.rs)'s ROM-dependent blink test from §8.2 is in
the same position — real, valuable, and unavailable in a `roms/`-less
tree; don't let a missing fixture read as a missing feature.

---

## 8.9 Reading assignment

In this order:

1. **[`crates/coco-core/src/gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs), lines 42–228.** The register constant
   modules (`init0`, `init1`, `vmode`, `vres`, `hoff`, `intr`), the lookup
   tables (`LPF_LINES`, `LPR_LINES`, `TEXT_COLS`, `GFX_BYTES_PER_ROW`,
   `GFX_BPP`), and the `GIME` struct's video-relevant fields. Everything in
   §8.1 lives here.
2. **[`crates/coco-core/src/gime_video.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs), in full (477 lines).** `decode_text`
   and `decode_graphics` first, then `FieldScan` and `resolve_colors`, then
   `paint_scanline` and its two callees. This is the chapter's centre of
   gravity; read the module doc comment at the top again once you've read
   the rest — it will make more sense the second time.
3. **[`crates/coco-core/src/gime/palette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs), in full.** Read `rgb_color`
   against §8.4's worked example with a calculator in hand; don't just
   trust the chapter's arithmetic.
4. **[`crates/coco-core/src/font_gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font_gime.rs), lines 1–17 and skim the table.**
   You don't need to memorize glyph bitmaps — notice the shape (128
   entries, 8 rows each) and the `$00`–`$1F` special-character note.
5. **[`crates/coco-core/tests/render_gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_gime.rs), in full.** Predict every test
   this chapter didn't walk (there are eleven more) before reading its
   assertions.

Run the ROM-free suite while you read:

```
cargo test -p coco-core --test render_gime
cargo run -p coco-core --example gime_demo /tmp
```

If (and only if) you have `roms/coco3.rom` available:

```
cargo test -p coco-core --test gime_modes
cargo test -p coco-core --test gime_irq
```

---

## 8.10 Exercises

**8.1 — Register dump to screen description (recall).** Given this
register dump — not one used as a worked example anywhere in this chapter
— describe the screen precisely: mode (text/graphics), columns or pixel
width, colours available, whether the display is wide or has side borders,
and the physical video base address.

```
$FF90 = $4C
$FF98 = $03
$FF99 = $10
$FF9A = $3F
$FF9D = $E4
$FF9E = $00
```

Work every field from the tables in §8.1 and §8.2 by hand before checking
yourself against `decode_text`/`decode_graphics` in code. (Hint: this
`$FF99` value is deliberately *not* one of the four you've already seen
decoded in this chapter — don't pattern-match against §8.1's or §8.3's
worked examples, actually compute the `HRES`/`CRES`/`LPF` fields.)

**8.2 — Bytes-per-row and total screen memory for `HSCREEN 4` (arithmetic
drill).** Using `$FF99 = $1D` (§8.3's HSCREEN table) and the fact that its
`$FF98` image sets `LPF = %00` (192 active lines, same as every `HSCREEN`
mode this chapter covered): compute `bytes_per_row`, `bpp`, pixel width,
and total screen memory in bytes. Then answer: how does the *total screen
memory* compare to `HSCREEN 2`'s (worked out in §8.3's "HSCREEN 1–4" table
discussion, though not given as a final byte count there — compute that
one too), and why is the relationship what it is, given that the two modes
have different pixel widths and colour counts? Express the total both in
decimal bytes and as a count of 8K MMU blocks (week 5) it would take to
map the whole screen into the CPU's logical space at once.

**8.3 — Sabotage `paint_text_row`'s foreground base, and watch it fail
(build + verify).** In [`crates/coco-core/src/gime_video.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs), change:

```rust
const ATTR_FG_BASE: usize = 8;
```

to `0`. Predict, before running anything, which `render_gime.rs` tests
will fail and why (hint: think about which tests read the foreground
colour of an *attributed* character versus tests that never touch
attributes at all — non-attribute text and every graphics test read no
`ATTR_FG_BASE`-derived index and should be unaffected). Then actually run:

```
cargo test -p coco-core --test render_gime
```

and compare the failure list to your prediction. (For reference — don't
peek until you've made your own prediction — sabotaging this constant
this way produces exactly 5 failures out of 14 tests, all of them the ones
that assert on an *attributed* foreground colour: blink, underline, the
second-row pitch check, the vertical-scroll check, and the direct
`text_attributes_select_fg_bg_palettes` test. Everything else — the
no-attribute test, every graphics test, the border tests — stays green,
because none of them ever read a foreground colour through
`ATTR_FG_BASE`.) Revert the constant, re-run the suite to confirm all 14
pass again, and check `git status` shows no changes to the file before you
move on — leaving a sabotaged constant in the tree is the one way to turn
this exercise into tomorrow's very confusing bug report.

**8.4 — Extend `gime_demo.rs`: a third PPM (build).** Add a third block to
`gime_demo.rs`'s `main` that renders an `HSCREEN 1` (320×192, 4-colour)
screen — you worked out its exact `$FF99` value in §8.3's HSCREEN table —
filled with a simple pattern of your choosing (vertical stripes, a
gradient, anything that isn't solid one colour). Write it to a third PPM
and confirm by eye that it's genuinely 4-colour, 320-pixel-wide content
occupying the full 640-canvas width (no border columns) — cross-check
against the `HSCREEN 1` row in your own §8.3 table before you conclude
you've got the register value right.

**8.5 — Why physical, not logical? (essay, five sentences max).** §8.1
argued that `$FF9D`/`$FF9E` must address physical RAM directly, bypassing
the MMU, because the video hardware needs to reach RAM the CPU's *current*
MMU mapping might not include. Make the argument precise: describe a
concrete scenario, using an installed-RAM size from `config.rs`'s
`MemorySize` enum (128K, 512K, or 2048K) and a specific video mode from
this chapter, where a *logical*-addressed video base would force the CPU
to give up access to its own running code or variables just to keep the
screen visible. What CoCo 3 technique (mentioned in §8.1) becomes
straightforward once the video base is physical, that would otherwise
require an active copy every frame?

**8.6 — Read and predict: `underline_lights_bottom_line_of_8_line_rows`
(read + predict).** Before reading past this sentence, open
`render_gime.rs` and find `underline_lights_bottom_line_of_8_line_rows`.
Read only the test's setup — the register values it configures and the RAM
bytes it writes — and write down, on paper, which exact canvas pixel(s)
you predict will be lit by the underline and which will not, at both
`TOP + 6` and `TOP + 7`. Then read the assertions and check yourself. If
you predicted wrong, trace back through `underline_line` (§8.2) and
`paint_text_row`'s `underline_here` computation to find exactly which fact
you were missing — don't just note that you were wrong, identify the
specific line of reasoning that broke.

---

## What's next

Next week keeps `gime_video.rs` open but changes the question: not "what
does one scanline look like," but "what happens when the registers this
chapter taught you to read *change* while the beam is still partway down
the screen." You already have every piece of machinery week 9 needs —
`FieldScan`'s latched-versus-live split (§8.5), the fact that
`paint_scanline` re-reads mode, palette, and border fresh every line — you
just haven't yet seen a real 6809 program *use* that machinery on purpose.
[`tests/scanline_split.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/scanline_split.rs) has the receipts: a timer-driven FIRQ handler
splitting the border colour mid-field, verified against a real interrupt
firing from real ROM code. You'll also finally learn what composite output
actually is — not a simplified RGB, but a genuinely different palette
table this chapter kept deferring — and meet the CoCo 1/2's own graphics
modes, the ones GIME-native scanout never touches at all.
