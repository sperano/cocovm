# Chapter 8 — GIME Native Modes: Registers, Text Attributes, Graphics, Palette

*Week 8. Goal: decode the GIME-native video registers. Given `$FF98–$FF9F`,
this chapter determines the text or graphics geometry, physical RAM source,
palette use, and per-scanline rendering behavior.*

---

## 8.0 Where this picks up

The power-on BASIC prompt uses VDG-compatible 32×16 text decoded through the
SAM-compatible page register,
using MC6847 glyphs. The GIME renders it, but only because `INIT0` bit 7
(`COCO`) tells it to pretend to be the chip it replaced. Everything
the machine does before a program says otherwise is a 1980 video mode
running on 1986 silicon.

Typing `WIDTH 80` or `HSCREEN 2` is what turns that bit off, and the
moment it clears, three separate things change at once. A completely
different register file takes over: `$FF98` through `$FF9F`, eight bytes
that were being ignored a microsecond earlier. The GIME's own internal
character generator replaces the MC6847-derived font, with a different
glyph shape and a different code-point layout. And the video hardware
stops reading through the CPU's 16-bit logical address space and starts
reading *physical* RAM directly. That last change has the most
far-reaching consequences, and §8.1 spends the most time on it.

This chapter explains that register file: what each bit means, what a legal
combination looks like, and how
[`crates/coco-core/src/gime_video.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs)
turns eight bytes of registers plus a block of RAM into pixels. The module
opens with a compact summary:

```rust
//! GIME-native hi-res text and graphics scanout (INIT0 COCO=0; `DESIGN.md` §6).
//!
//! Text (`$FF98` BP=0): 32/40/64/80 columns from the GIME's internal character
//! generator (`font_gime`), optionally with per-character attribute bytes
//! (foreground palette regs 8–15, background regs 0–7, blink, underline).
//! Graphics (BP=1, HSCREEN): 16–160 bytes per row unpacked at 1/2/4 bits per
//! pixel through the palette registers.
//!
//! Unlike the CoCo-compatible modes, GIME-native scanout addresses *physical*
//! RAM directly — the vertical offset registers give the start address and the
//! MMU is bypassed (SEB Unravelled II). Rendering is per scanline into the
//! canonical 640×240 raster (`raster.rs`, Option B): [`paint_scanline`] paints
//! one canvas row from the LIVE registers plus the per-field latched state in
//! [`FieldScan`], so mid-frame register writes take effect on the next line —
//! except the field-latched group ($FF9D/$FF9E base, $FF9C smooth-scroll
//! seed), which MAME `gime.cpp` `new_frame` samples once per field.
//! Register semantics verified against SEB Unravelled II and MAME `gime.cpp`
//! (see memory `gime-scanline-verified-facts`).
```

([`gime_video.rs:1-18`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L1-L18).)
Every phrase in that comment becomes a section of this chapter. The
32/40/64/80 columns and their attribute bytes are §8.2. The 16-to-160
bytes per row at 1, 2, or 4 bits per pixel are §8.3. The palette registers
those bits index are §8.4. And the last sentence — live registers plus a
small latched group — is §8.5, the part that makes next week possible.

One boundary is worth keeping in mind from the start, because the codebase
itself draws it as a module boundary. This chapter is about *what a single
scanline looks like given the current registers*. What happens when a 6809
program changes those registers *in the middle of a field*, so that the top
half of the screen used one video base and the bottom half used another, is
next week's material. The exact mechanism that makes such a split possible
appears here — `FieldScan`, in §8.5, and the deliberate decision to re-read
most registers every line — but the raster tricks it enables belong to
Chapter 9. Learn the still photograph first; the motion picture comes after.

---

## 8.1 The register file as a video-mode description language

Eight consecutive bytes, `$FF98` through `$FF9F`, completely describe a
GIME-native video mode: how many columns or pixels wide it is, how many
colors it can show, where the data lives in RAM, and how it is scrolled.
There is nothing else. No separate "graphics mode" enum, no derived state
computed once at mode-switch time and cached for the rest of the field.
Every frame, every scanline, the renderer re-reads these bytes and decodes
them fresh.

The registers are the renderer's current mode. There is no place in this codebase that records
"the machine is currently in `HSCREEN 2`." If you want to know the mode,
you read `$FF98` and `$FF99` and decode them, exactly as a piece of 6809
code would have to. That fidelity costs a few microseconds of redundant
decoding per scanline and buys something considerably more valuable: there
is no cached copy of the mode that can fall out of sync with the registers
a program actually wrote. A whole category of emulator bug — the screen
that stays in the old mode because someone forgot to invalidate a cache —
cannot exist here.

Here is the whole group, with the constants module describing each
register's bit layout where one exists:

| Address | Name | Constants module | What it selects |
|---|---|---|---|
| `$FF98` | VMODE | [`vmode`] | graphics/text, burst phase, monochrome, field rate, lines-per-row |
| `$FF99` | VRES  | [`vres`]  | lines-per-field, bytes-per-row, color depth / attribute enable |
| `$FF9A` | border | — | 6-bit border color value |
| `$FF9B` | video bank | — | high address bits for >512K machines |
| `$FF9C` | vertical scroll | — | smooth-scroll seed (character-row line to start on) |
| `$FF9D`/`$FF9E` | vertical offset | — | physical video base ×8 |
| `$FF9F` | horizontal offset | [`hoff`] | HVEN + X-scroll, ×2 |

All eight are plain fields on the `GIME` struct
([`crates/coco-core/src/gime.rs:169-228`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L169-L228)),
sitting alongside the MMU task registers from Chapter 5 and the timer and
interrupt state that Chapters 6 and 11 deal with. There is no `VideoRegisters`
sub-struct, and the flat layout is the honest one: on the real chip these
are eight addresses in the same I/O page as everything else the GIME owns.

The bit layouts live in small constant modules immediately above the
struct. Start with `$FF98`, the video mode register:

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

([`crates/coco-core/src/gime.rs:70-83`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L70-L83).)
That is the whole of `$FF98`, and it is smaller than its reputation. One
bit decides between text and graphics: `BP`, the bit-plane select, which
this chapter's two big sections split on. Two bits, `BPI` and `MOCH`, exist
only for composite output and have no effect whatsoever on the RGB path
that every worked example in this chapter uses. Chapter 9 gives them their
own treatment; until then it is enough to know they are there and not in
the way. `H50` picks a 50 Hz field rate instead of 60 Hz, which matters
for PAL machines and for the scanline budget of Chapter 6 rather than for the
geometry of a single line. And the bottom three bits are a packed field,
`LPR`, that says how many scanlines tall a character row is.

Register `$FF99` has the same shape, a few packed fields with a mask
apiece, but it carries considerably more of the mode:

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

([`gime.rs:85-99`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L85-L99).)
Two details in that comment block repay a slow read. The first is the note
on `HRES`: it "sets bytes per row, not pixels." That distinction is easy to
skim past and is the source of more confusion than any other fact in the
register file. `HRES` controls how much *data* the video hardware fetches
for each displayed row. How many pixels that data turns into depends
entirely on how many bits each pixel takes, which is a different field
altogether.

The second is the overloading of `CRES`. In graphics mode it means "how
many bits per pixel." In text mode, bit 0 of the very same two-bit field
means something with no relationship at all to color depth: it turns
per-character attribute bytes on. Same register, same bits, opposite
meaning, switched by a bit in a *different* register. This is the kind of
"the datasheet says it depends" fact that makes hand-decoding a register
dump feel like archaeology the first few times through. After this chapter
it will not.

### Fields that don't decode by arithmetic

Four of the packed fields — `LPF`, `HRES`, `CRES`, and `LPR` — do not turn
into a useful number by any formula. You cannot shift them, scale them, or
add an offset and arrive at the answer. They index a lookup table, because
the chip's own spacing of values is neither linear nor formulaic. Here are
all five tables, verbatim, doc comments included, because in this particular
block the comments carry as much information as the code:

```rust
/// Active display lines per field, indexed by the VRES LPF field.
///
/// LPF=%10 is documented as 210 lines (SEB Unravelled II); on real hardware it
/// is a glitched "infinite" count (MAME `update_geometry`) — 210 is the sane
/// approximation.
pub const LPF_LINES: [usize; 4] = [192, 200, 210, 225];

/// Lines per character row, indexed by the $FF98 LPR field. Hardware-verified
/// values from MAME `get_lines_per_row` (SEB's table says 1/2/3/8/9/10/12 but
/// the chip does 1/1/2/8/9/10/11; LPR=%111 repeats one glitched line forever,
/// approximated by a huge count so only the first row ever shows).
pub const LPR_LINES: [usize; 8] = [1, 1, 2, 8, 9, 10, 11, usize::MAX];

/// Text columns per row, indexed by the VRES HRES field (BP=0). HRES bit 1 is
/// ignored by the chip in text modes (MAME dispatches on $FF99 & $15), which
/// yields SEB's 32/40/32/40/64/80/64/80 table.
pub const TEXT_COLS: [usize; 8] = [32, 40, 32, 40, 64, 80, 64, 80];

/// Graphics bytes fetched per row, indexed by the VRES HRES field (BP=1).
pub const GFX_BYTES_PER_ROW: [usize; 8] = [16, 20, 32, 40, 64, 80, 128, 160];

/// Graphics bits per pixel, indexed by the VRES CRES field (BP=1): 2, 4, or 16
/// colours. CRES=%11 is undefined on the GIME; 4 bpp is the closest behaviour.
pub const GFX_BPP: [usize; 4] = [1, 2, 4, 4];
```

([`gime.rs:110-133`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L110-L133).)
Four of those five tables carry a footnote about hardware that misbehaves —
only `GFX_BYTES_PER_ROW` is clean. Three of them deserve unpacking before
the worked examples arrive, because they are exactly the entries that will
bite during the exercises. The fourth, `GFX_BPP`'s undefined `CRES = %11`,
needs no more than its comment: no `HSCREEN` mode selects it, and 4 bpp is
the closest thing to a defensible answer.

Start with `LPR_LINES`, which contains a small scandal. Super Extended
Color BASIC Unravelled II is the best secondary source available for this
chip, and its table for the lines-per-row field lists 1/2/3/8/9/10/12. The
real silicon, verified against MAME's `get_lines_per_row`, does
1/1/2/8/9/10/11. Three of the eight entries in the reference book are
wrong. This codebase follows the chip rather than the book and says so in
the comment. That is the right call, and it doubles as a standing reminder
about method. This repository's house rule is to verify hardware claims
against the PDFs in `docs/` instead of guessing. That rule holds, with the
caveat this table documents: sometimes verifying against `docs/` means
verifying `docs/` against something else. When a book and a
hardware-derived implementation disagree, the disagreement itself is the
interesting artifact, and the honest thing to do is record both and pick
one for a stated reason.

The eighth entry of that same table, `usize::MAX`, is not a lines-per-row
value in any meaningful sense. `LPR = %111` on the real chip produces a
glitch: one scanline repeats forever, and the display never advances to a
second character row. Rather than special-case that anywhere in the render
path, the table encodes the glitch as an absurdly large row height. The
counter in `advance_scan` compares `line_in_row` against it, never reaches
it, and therefore never steps the row pointer — so only the first row is
ever fetched, which is precisely the visible behavior. A sentinel value
that makes ordinary arithmetic produce the correct pathological result is
often cheaper and clearer than a branch, and this is a textbook case.

`LPF_LINES` carries a milder version of the same story. Its `%10` entry is
documented as 210 lines and is, on real silicon, a glitched count that
behaves as zero or infinity depending on where in the raster the write
lands. There is no correct number to put there. The table puts 210, the
comment says why, and the approximation is visible to anyone reading the
constant rather than buried in a renderer.

`TEXT_COLS` has the strangest shape of the five: eight entries, only four
distinct values, arranged as 32, 40, 32, 40, 64, 80, 64, 80. The reason is
that `HRES` bit 1 — the middle bit of the three-bit field — is
architecturally ignored by the chip in text mode. Index `%000` and index
`%010` both mean 32 columns; `%001` and `%011` both mean 40. Compare
`GFX_BYTES_PER_ROW`, which is indexed by the very same three-bit field and
produces eight *distinct* values, because graphics mode reads all three
bits. One register field, one bit width, two different chip behaviors
depending on `BP`.

The practical consequence is a decoding sanity check worth memorizing. In
text mode there are exactly four legal column counts: 32, 40, 64, and 80.
If a hand-decode of a text-mode register dump produces 36, or 48, or any
other number, the decode is wrong. Those four are the only answers the
table can return, by construction.

The tables are consumed through two small helper methods on `GIME`, and
seeing them used teaches as much as seeing them declared:

```rust
    /// Lines per character row from the $FF98 LPR field (also applied to
    /// graphics rows, where BASIC's HSCREEN setup selects 1).
    pub fn lines_per_row(&self) -> usize {
        LPR_LINES[(self.vmode & vmode::LPR_MASK) as usize]
    }

    /// Active display lines in the current field from the VRES LPF bits.
    pub fn lines_per_field(&self) -> usize {
        LPF_LINES[((self.vres & vres::LPF_MASK) >> vres::LPF_SHIFT) as usize]
    }
```

([`gime.rs:271-280`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L271-L280).)
Each is one line: mask the field out of its register, shift it down to a
plain integer if it does not already sit at the bottom, index the table.
`lines_per_row` needs no shift because `LPR` occupies bits 0–2 already;
`lines_per_field` needs one because `LPF` sits at bits 5–6. That is the
entire decoding vocabulary of this register file, repeated with different
masks. Note the parenthetical in the first comment: `LPR` applies to
graphics rows too, where a "character row" is one scanline tall and the
concept degenerates into "advance the row pointer every line." §8.3's
worked example depends on that.

> **Rust corner: an array as a truth table.** Compare this decoding
> strategy to Chapter 2's `cc` module (`ch02-registers-flags-dispatch.md`
> §2.1), which was a flat namespace of `pub const u8` bit masks tested
> with `&`. That style suits a register whose bits are independent
> booleans. Here, four of these registers' eight fields decode by *indexing
> an array with the field's numeric value* rather than by testing a bit, and
> the array literal is the whole specification. `LPR_LINES[3] == 8` is the
> entire truth table for "what does `LPR = %011` mean," in one line, with
> no `match` arms to keep in sync with a datasheet table by hand.
>
> The alternative would be `match field { 0 => 1, 1 => 1, 2 => 2, ... }`,
> which says the same thing in six more lines. Rust *does* check `match`
> exhaustiveness, so that version is not unsafe. What the array literal
> adds is a visible shape: eight entries, indexed 0 through 7, lined up
> where a reader can compare them against the reference table
> side by side. A stray ninth entry or a missing one is a compile error
> from the declared length, not a logic bug waiting to be found in a
> renderer six months later. When a register field's meaning is "look this
> up" rather than "compute this," an array usually reads better than a
> `match`. The same shape recurs in `raster.rs` for the
> `LPF_LINES`/`vertical_window` pair you will meet in §8.5.

### The two registers that bypass the MMU: `$FF9D`/`$FF9E`

Every address the CPU touches on a CoCo 3 goes through the MMU studied in
Chapter 5. Eight-kilobyte logical slots, translated through the active task's
block table, `phys = block << 13 | addr & 0x1FFF`. That translation is
unconditional for the CPU: there is no addressing mode, no instruction, and
no privilege level that lets 6809 code reach around it.

The GIME-native video hardware does not go through it at all. Registers
`$FF9D` (high byte) and `$FF9E` (low byte) together form a 16-bit
*vertical offset* value, and the physical address where the video hardware
begins fetching is that value shifted left by three:

```rust
    /// Physical start address of the GIME-native video display: the vertical
    /// offset registers ×8 (any 8-byte boundary in the 512K space), plus the
    /// $FF9B 512K bank on >512K machines. GIME-native scanout bypasses the MMU
    /// entirely — this is a physical address (SEB Unravelled II Fig 6).
    pub fn video_base(&self) -> usize {
        ((self.video_bank as usize & 0x0F) << 19) | ((self.vertical_offset as usize) << 3)
    }
```

([`gime.rs:263-269`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L263-L269).)
Sixteen bits shifted up by three reach 19 bits of address, which is 512K —
exactly the largest machine Tandy shipped. The `×8` is not arbitrary
scaling for its own sake: it is how a 16-bit register covers a 19-bit
address space, at the price of only being able to point at 8-byte
boundaries. Screens are thousands of bytes long and always start at round
addresses anyway, so the constraint costs nothing real.

The write side confirms the byte order, and it is the order the register
names imply rather than the one 6809 habits might suggest. `VOFFSET1_REG`
at `$FF9D` supplies the high byte, `VOFFSET0_REG` at `$FF9E` the low, from
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

Each write preserves the other half. A program can therefore change the top
byte of the video base without disturbing the bottom, which is exactly what
a page-flipping routine wants: one `STA $FF9D` moves the entire display
somewhere else in physical RAM, atomically as far as the video hardware is
concerned.

Now the question that matters. Why physical addressing at all, when the
whole machine is otherwise built around the MMU?

Think back to Chapter 5. The MMU exists precisely so that a CPU with sixteen
address lines can reach far more than 64K of installed RAM — 128K on a
stock machine, 512K as Tandy shipped it, and up to 2048K as the GIME's MMU
architecture permits
([`crates/coco-core/src/config.rs:114-115`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/config.rs#L114-L115)) —
by remapping eight-kilobyte windows on demand. A hi-res graphics screen at
`HSCREEN 4` occupies 30,720 bytes, a figure the exercises ask you to derive
rather than accept. That is nearly four full MMU slots out of the eight the
CPU has.

Suppose the video hardware could only read through the CPU's *current* MMU
mapping. Then the screen would be visible only while the CPU happened to
have those particular physical blocks mapped into its logical window at
that instant. Every interrupt handler, every BASIC statement that touched a
different bank, every disk driver that borrowed a slot for a sector buffer
would have to restore the mapping before the next scanline or the picture
would fill with whatever unrelated data now lived at those logical
addresses. Worse, a screen built in a bank that the current task's block
table does not reference at all would be unreachable by the video hardware
entirely — invisible not because it was scrolled off, but because there was
no path from the video fetch to those physical bytes.

Physical addressing removes the problem rather than managing it. The video
base can point anywhere in installed RAM, including banks the CPU is not
currently looking at, entirely independent of what the running BASIC or
machine-language program has mapped into its own 64K.

That independence is what makes double-buffering practical on this machine.
A program builds the next frame in a bank it *can* still see and edit
through the MMU, and when the frame is ready it repoints `$FF9D`/`$FF9E` at
it. No copy, no remapping of the CPU's own address space, no tearing beyond
the one-field granularity §8.5 will pin down precisely. The alternative
world, where video and CPU shared one address translation, would have had
the MMU's reason for existing — letting a 64K CPU reach more than 64K —
pitted directly against the video hardware's need to always show one
fixed patch of that same RAM regardless of what the CPU is doing. Two
requirements, one mechanism, permanent conflict. Splitting the addressing
paths dissolves it.

### `$FF9A`–`$FF9C`: border, video bank, smooth scroll

Three simpler registers round out the group, and they are simpler in
different ways.

`$FF9A` holds the border color, and the important word in that sentence is
*color*, not *index*. Unlike every foreground and background color in
this chapter, the border does not name one of the sixteen palette
registers. It carries a raw six-bit color value of its own, resolved by
the same function a palette register's contents would pass through. The
renderer's `resolve_colors` in §8.4 handles it as a separate return value
for exactly this reason. Every pixel outside the active display area, on
every scanline, is this one color — the top and bottom border rows in
their entirety, and the left and right strips on non-wide modes. A program
that wants a striped border does not write sixteen registers; it writes
this one register repeatedly, timed against the beam, which is Chapter 9's
opening trick.

`$FF9B`, the video bank register, is the least interesting byte in the
file on any machine anyone actually owned. It supplies physical address
bits above bit 18 for machines with more than 512K installed, appearing in
`video_base()` as `(self.video_bank as usize & 0x0F) << 19`. On a stock
128K or 512K CoCo 3 it is always zero and completely irrelevant. It exists
because the GIME's MMU architecture reaches two megabytes and the video
base needed some way to follow it there. Emulating it costs one masked
shift, which is cheaper than explaining why it was omitted.

`$FF9C`, the vertical scroll register, holds a four-bit value with a
specific and slightly unusual meaning: not "how many pixels down," but
"which scanline *within* the first character row should the field start
displaying from." Setting it to 3 in an eight-line-per-row text mode means
the top row of the screen shows only its bottom five scanlines, and every
row below it shifts up correspondingly. That is smooth vertical scrolling
at sub-character-row granularity, achieved without moving a single byte of
screen memory. §8.5 walks the exact three lines of `FieldScan::latch` that
consume it, including the guard that handles a value the current row height
cannot honor.

### `$FF9F`: horizontal offset, HVEN, and the 256-byte seam

The last register in the group has the sharpest edge case in the chapter.
`$FF9F` packs two unrelated things into one byte:

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

([`gime.rs:101-108`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L101-L108).)
With `HVEN` clear, each displayed row in RAM is exactly as wide as the mode
needs — 80 bytes for a 640-pixel two-color graphics screen, 160 bytes for
80-column text with attributes — and the low seven bits of `$FF9F`, doubled
to give an even byte count, shift where within that row the fetch begins.
The result is horizontal scrolling within the row's own bytes.

With `HVEN` set, the semantics change substantially. Every row in RAM is
now treated as a fixed 256 bytes wide, regardless of how few of those bytes
are actually displayed, and the X offset scrolls a window across the wider
virtual row. This is a genuinely *virtual* screen: more data lives in RAM
per row than can ever appear on screen at once, and `$FF9F` pans across it
two bytes at a time. A game can hold a wide level map in RAM and scroll it
horizontally by writing one register per frame.

The row pitch — the number of bytes to add when stepping from one displayed
row to the next — has to change to match, and it does:

```rust
/// Advance `scan`'s shared vertical counter after painting a body row of
/// `row_bytes` bytes: the row pointer steps by the CURRENT line's live pitch
/// once per LPR lines (MAME `record_full_body_scanline`; LPR=%111's huge
/// count never wraps).
fn advance_scan(scan: &mut FieldScan, g: &GIME, row_bytes: usize) {
    let pitch = if g.horizontal_offset & hoff::HVEN != 0 {
        gime::HVEN_ROW_BYTES
    } else {
        row_bytes
    };
    scan.line_in_row += 1;
    if scan.line_in_row >= g.lines_per_row() {
        scan.line_in_row = 0;
        scan.row_base += pitch;
    }
}
```

([`gime_video.rs:243-258`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L243-L258).)
`HVEN_ROW_BYTES` is 256
([`gime.rs:135-136`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L135-L136)).
Note what else this function reveals, since it will matter in §8.5: the row
pointer does not advance on every scanline. It advances once every
`lines_per_row()` scanlines, which is why an eight-line-tall text row
fetches its 160 bytes once and then re-reads them for eight consecutive
canvas rows, drawing a different glyph row from each byte each time.

The fetch itself carries the register's real oddity. Every read wraps at a
256-byte boundary within the row, whether or not `HVEN` is set:

```rust
    let fetch = |i: usize| ram[(row_base + ((x_offset + i) % ROW_FETCH_WRAP)) % ram.len()];
```

([`gime_video.rs:231`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L231),
where `ROW_FETCH_WRAP` is `0x100`.) Read that modulo carefully. It is not
conditioned on `HVEN` at all: the fetch offset always wraps at 256 bytes
within the row, even when `HVEN` is off and the row itself is narrower than
that. Super Extended Color BASIC Unravelled II describes the consequence as
"peculiar things happen" when the horizontal offset is used without `HVEN`
enabled: shift the X offset far enough on a non-`HVEN` mode and the fetch
address wraps back to the start of the same 256-byte span instead of
continuing to climb into the next row's data. That wrap point is the
*seam*, and the test named for it,
`horizontal_offset_shifts_fetch_with_seam_wrap`, is walked in §8.7.

The outer `% ram.len()` is a second, independent safety net: it keeps a
video base pointing past the end of installed RAM from indexing out of
bounds, wrapping it into the installed range instead. A 128K machine whose
video base was left at a 512K address shows *something* rather than
crashing the emulator, which is the correct failure mode for a renderer.

### Wide and non-wide: where the 640-pixel canvas comes from

One more piece of geometry has to be in place before the worked examples,
because both of them turn on it. Every GIME-native mode in this codebase
renders into the same fixed-size canvas:

```rust
/// Canonical canvas width: MAME's coco3 visible width.
pub const CANVAS_W: usize = 640;
/// Canonical canvas height: MAME's coco3 visible lines.
pub const CANVAS_H: usize = 240;

/// Active-content width of non-wide modes; the rest of the 640 is border.
pub const NON_WIDE_ACTIVE_W: usize = 512;
/// Horizontal border width each side of a non-wide mode's 512 px body.
pub const NON_WIDE_BORDER_X: usize = (CANVAS_W - NON_WIDE_ACTIVE_W) / 2;
```

([`crates/coco-core/src/raster.rs:15-23`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/raster.rs#L15-L23).)
The module's own doc comment calls this "Option B": one fixed-size RGBA
canvas for every GIME-native mode, with every legal mode reaching it by an
*integer* horizontal scale. A 320-pixel-wide mode doubles each pixel; a
640-pixel-wide mode copies it once. Nothing ever needs a resampling filter,
and the frontend never has to cope with a texture that changes size when a
program types `WIDTH 80`.

Within that canvas, a mode is either *wide* or it is not, and one bit
decides:

```rust
/// Mask for the HRES field's low bit ($FF99 bit 2): the "wide" flag in
/// MAME's pixel path (`render_scanline`: `wide = !legacy && (ff99 & 0x04)`).
/// Wide modes fill the full 640 canvas px with no border; non-wide modes
/// fill the centre 512. (MAME's `update_geometry` tests bit 3 instead, but
/// only for field-sync timing — the emitted pixel widths follow bit 2.)
const WIDE_HRES_MASK: usize = 0x01;
```

([`gime_video.rs:54-59`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L54-L59).)
The mask is `0x01` rather than `0x04` because it is applied to the `HRES`
field *after* it has been shifted down to a plain 0–7 value, where bit 2 of
the register has become bit 0 of the field. The function that uses it does
two jobs at once — paint the side borders if there are any, and report back
where the active span starts and how wide it is:

```rust
fn paint_side_borders(g: &GIME, row_px: &mut [u8], border: [u8; 4]) -> (usize, usize) {
    let hres = ((g.vres & vres::HRES_MASK) >> vres::HRES_SHIFT) as usize;
    let wide = hres & WIDE_HRES_MASK != 0;
    let (x0, active_w) = if wide {
        (0, CANVAS_W)
    } else {
        (NON_WIDE_BORDER_X, NON_WIDE_ACTIVE_W)
    };
    if !wide {
        fill(&mut row_px[..x0 * BYTES_PER_PIXEL], border);
        fill(&mut row_px[(x0 + active_w) * BYTES_PER_PIXEL..], border);
    }
    (x0, active_w)
}
```

([`gime_video.rs:201-214`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L201-L214).)
A wide mode returns `(0, 640)` and paints no side border at all, because
there is none: the active picture reaches both edges of the visible raster.
A non-wide mode returns `(64, 512)`, having first filled the 64-pixel strip
on each side with the `$FF9A` color. Everything downstream — the text
painter, the graphics painter, and their `xscale` computations — works
inside that returned span and never needs to know which case it got.

Two rules of thumb fall out of this and will be used repeatedly below.
Every `HRES` field value with its low bit set is wide, and every `HSCREEN`
mode BASIC can set up turns out to be one. A 40-column text screen is wide
too, which surprises people who expect narrower to mean smaller: 40 columns
of 8-pixel glyphs is 320 native pixels, doubled to fill all 640, with no
border strip. The 64-column mode, by contrast, is *not* wide — 512 native
pixels, centered, with 64 pixels of border on each side.

### Worked example: decoding `WIDTH 80`'s real register image

Time to put the whole table to use on bytes that are not synthetic test
values. What follows is the actual register image Super Extended Color
BASIC writes when a program executes `WIDTH 80`, taken from the ROM's own
data table (SEB Unravelled II, disassembly listing at `$E044`–`$E04B`) and
confirmed byte for byte against
[`crates/coco-core/tests/gime_modes.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/gime_modes.rs)'s
replay of the same sequence against a real booted machine:

| Register | Value |
|---|---|
| `$FF90` (INIT0) | `$4C` |
| `$FF98` (VMODE) | `$03` |
| `$FF99` (VRES)  | `$15` |
| `$FF9A` (border) | `$12` |
| `$FF9B` (video bank) | `$00` |
| `$FF9C` (scroll) | `$00` |
| `$FF9D`/`$FF9E` (offset) | `$D8`/`$00` |

Decode it register by register, in the order the tables were introduced.

`$FF90 = $4C` is `0100_1100`, which sets `MMUEN` (`0x40`), `MC3` (`0x08`),
and `MC2` (`0x04`). The bit that matters most here is the one that is
*clear*: `COCO` (`0x80`). This is a GIME-native mode. Everything below
applies precisely because that bit is zero.

`$FF98 = $03` has `BP` (`0x80`) clear, so this is text rather than
graphics, and its `LPR` field is `%011` = 3, giving `LPR_LINES[3] = 8`
scanlines per character row.

`$FF99 = $15` is `0001_0101`, and it carries three separate fields. The
`LPF` field in bits 5–6 is `%00`, so `LPF_LINES[0] = 192` active lines. The
`HRES` field is `(0x15 & 0x1C) >> 2 = 5`, so `TEXT_COLS[5] = 80` columns.
And `CRES` bit 0 is set, which in a text mode means per-character attribute
bytes are enabled.

`$FF9A = $12` is a border color value, not an index, so it goes straight
through the channel decode that §8.4 derives in full. Briefly: `0x12` is
`0b00_01_00_10`; the red channel takes bits 5 and 2, which are both zero;
green takes bits 4 and 1, which are both one, giving the maximum 3 and
therefore `0xFF`; blue takes bits 3 and 0, both zero. The result is pure
green. That is the border color of the CoCo 3's 80-column screen, read out
of the actual ROM data rather than remembered.

`$FF9D`/`$FF9E` = `$D8`/`$00` gives `vertical_offset = 0xD800`, so
`video_base = 0xD800 << 3 = 0x6C000`. Physical, bypassing the MMU
entirely — a location well above the 64K the CPU can see at once.

Put it together and the register image describes one specific screen: an
80-column, 24-row hi-res text display with per-character attribute bytes, a
pure-green border, living at physical `$6C000`. The 24 rows come from
dividing the 192 active lines by the 8-line character row. Attributes mean
two bytes per cell, so each row of 80 characters occupies 160 bytes and the
whole screen occupies `160 × 24 = 3,840` bytes, running from physical
`$6C000` through `$6CEFF`. And because `HRES = 5` has its low bit set, this
is a wide mode: the full 640-pixel canvas, no border columns left or right,
`xscale = 1`, one native pixel per canvas pixel.

Hold on to that 160-byte row pitch. It reappears, as a literal `160`, in a
`render_gime.rs` test walked in §8.7 — but by a different route: there it
is `GFX_BYTES_PER_ROW[7]`, the bandwidth of an `HSCREEN 2` graphics row,
rather than 80 columns times two bytes. Same number, two independent
reasons for it, which is worth knowing before the coincidence misleads you.

---

## 8.2 Text with attributes

With `BP` clear, `$FF98` and `$FF99` describe a text mode: some number of
columns drawn from `TEXT_COLS`, some number of scanlines per character row
drawn from `LPR_LINES`, and — when `CRES` bit 0 is set — a second byte
following every character byte in memory. That second byte is the
attribute, and it is what separates the GIME's text modes from every text
mode the CoCo had before. A CoCo 1 text screen had one foreground color
and one background color for the entire display. A CoCo 3 hi-res text
screen can give every single character cell its own pair, plus underline
and blink, for the price of doubling the screen's memory footprint.

The decode itself is four lines, and it does exactly what §8.1's tables
promise:

```rust
/// Decode the text mode from the GIME video registers ($FF98/$FF99).
pub fn decode_text(g: &GIME) -> TextMode {
    let hres = ((g.vres & vres::HRES_MASK) >> vres::HRES_SHIFT) as usize;
    TextMode {
        cols: gime::TEXT_COLS[hres],
        attributes: g.vres & vres::TEXT_ATTR != 0,
        lines: g.lines_per_field(),
        lines_per_row: g.lines_per_row(),
    }
}
```

([`gime_video.rs:73-82`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L73-L82).)
No branching on mode, no validation, no error case. Mask the field, index
the table, test one bit, call the two helper methods. Any of the eight
`HRES` values is legal and produces one of the four legal column counts;
there is no illegal register image to reject, which is exactly the property
that lets §8.10's first exercise hand over an arbitrary dump and expect a
definite answer.

### The attribute byte

Five constants describe the layout of that second byte:

```rust
/// Attribute-byte fields (SEB Unravelled II Fig 4).
const ATTR_BLINK: u8 = 0x80;
const ATTR_UNDERLINE: u8 = 0x40;
const ATTR_FG_SHIFT: u8 = 3;
const ATTR_COLOR_MASK: u8 = 0x07;
/// Foreground colours come from palette registers 8–15.
const ATTR_FG_BASE: usize = 8;
```

([`gime_video.rs:30-36`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L30-L36).)
One byte, four fields, no wasted bits. Bit 7 marks the cell as blinking.
Bit 6 marks it as underlined. Bits 5 through 3 hold a three-bit foreground
color, 0 to 7, which is added to `ATTR_FG_BASE` to land in palette
registers 8 through 15. Bits 2 through 0 hold a three-bit background
color, also 0 to 7, which needs no offset because it indexes palette
registers 0 through 7 directly.

That split is the design's whole cleverness. Sixteen palette registers,
eight reserved for backgrounds and eight for foregrounds, addressed by one
byte per cell. A programmer sets up the palette once, then paints the
screen with attribute bytes that never touch a hardware register again.
The cost is that a character cell cannot use an arbitrary pair from the
sixteen — a foreground color must live in the upper eight registers and a
background in the lower eight — which is exactly the sort of constraint
that vanishes into a program's palette-setup routine and is never thought
about again.

Without attributes, the same screen becomes far simpler and far more
limited:

```rust
/// Palette registers for text without attributes: background 0, foreground 1
/// (MAME `emit_gime_text_samples`).
const NO_ATTR_BG: usize = 0;
const NO_ATTR_FG: usize = 1;
```

([`gime_video.rs:37-40`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L37-L40).)
Every character on the screen uses palette register 1 for its foreground
and register 0 for its background. Fourteen of the sixteen palette
registers are unreachable in that mode. In exchange, the screen
takes half the memory: an 80-column, 24-row screen without attributes is
1,920 bytes instead of 3,840.

### The GIME's own font

Legacy VDG text in Chapter 7 drew its glyphs from `font6847.rs`, an
MC6847-derived table with 8×12 cells. GIME-native text draws from a
completely different table in
[`crates/coco-core/src/font_gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font_gime.rs):
`GIME_FONT`, 128 glyphs, each exactly 8 rows tall and 8 pixels wide. The
module header is precise about what those 128 glyphs cover, and precision
matters here because the layout is not quite what a programmer used to
ASCII would assume:

> 128 glyphs indexed by the character byte's low 7 bits. The layout is
> ASCII from `$20` up (lowercase with descenders at `$60-$7F`); `$00-$1F`
> are accented and special characters. Each glyph is 8 pixels wide by 8
> rows; text modes with more than 8 lines per row (LPR 9-12) pad below
> with blank lines.

Codes `$20` through `$7F` are the ASCII printable range, arranged exactly
where ASCII puts them. Writing `'A'` (`$41`) into the text buffer selects
glyph index `$41`, which draws a capital A. That is true whether or not the
high bit is set, because the font is indexed by `code & 0x7F`, so `$C1`
draws the same A that `$41` does.

Codes `$00` through `$1F` are where the assumption breaks. In ASCII those
are control codes. In this font they are 32 accented letters and symbols
with no ASCII meaning at all. A raw hex dump of a hi-res text buffer
showing bytes in that range is not showing control characters that
somehow leaked into video memory; it is showing the CoCo 3's extended
character set. The codebase's own debug helper takes this seriously enough
to refuse to guess, printing a placeholder for such codes rather than a
misleading ASCII interpretation. That function is `text_lines`, and §8.6
reads it.

The fixed 8-row height interacts with `LPR` in a way that is easy to state
and easy to forget. Every glyph is 8 rows regardless of how tall the
character cell is. When `LPR` selects a 9-, 10-, or 11-scanline cell, the
extra scanlines below the glyph are blank; the guard `line_in_row <
GLYPH_ROWS` in `paint_text_row` falls back to an all-zero row rather than
indexing past the end of the glyph. An 8-line-per-row mode — `LPR = %011`,
which is exactly what `WIDTH 80` selects — has no spare rows at all. That
turns out to explain something about underlines.

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

([`gime_video.rs:170-179`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L170-L179).)
An underline is one specific scanline of the character cell, forced fully
lit regardless of what the glyph's own pixels say. Which scanline depends
on how tall the cell is, and the mapping is not simply "the last one." For
an 8-line cell it is row index 7, the very bottom scanline, which is the
only choice available. For a 9- or 10-line cell it is row 8, the first
scanline below the 8-row glyph. For an 11-line cell it is row 9, leaving
one blank scanline beneath.

The `None` case is the interesting one. `LPR` values that produce 1- or
2-scanline cells have no defined underline position, because there is no
room for one: a cell that is one scanline tall cannot both show a glyph row
and an underline. The function returns `Option<usize>` rather than picking
an arbitrary fallback, and the caller compares `Some(line_in_row) ==
underline`, which is false for every possible `line_in_row` when the
underline is `None`. The absence of a defined behavior is modeled as an
absent value, and the comparison that consumes it needs no special case at
all.

### Blink: driven by the GIME's own timer

Bit 7 of the attribute byte marks a character as blinking. Whether a
blinking character is *currently* visible is not decided per character, and
not even decided by the renderer. It is one shared boolean on the `GIME`
struct, `blink_state`, toggled by the chip's own 12-bit interval timer:

```rust
    /// Advance the 12-bit timer by `ticks` input clocks. Each underflow raises
    /// the TMR interrupt source, toggles the text blink phase, and reloads
    /// (SEB Unravelled II; MAME `timer_elapsed`). Inhibited while the
    /// programmed value is zero.
    pub fn tick_timer(&mut self, ticks: u32) {
        if self.timer_reload == 0 {
            return;
        }
        let mut remaining = ticks;
        while remaining >= u32::from(self.timer_count) {
            remaining -= u32::from(self.timer_count);
            self.blink_state = !self.blink_state;
            self.raise(intr::TMR);
            self.restart_timer();
        }
        self.timer_count -= remaining as u16;
    }
```

([`gime.rs:378-394`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L378-L394).)
The timer's reload arithmetic, its two selectable input clocks, and the
interrupt it raises belong to Chapter 6 and a later chapter. Only one
line concerns this chapter: `self.blink_state = !self.blink_state`, on
every underflow. Blink rate is therefore not a video property at all. It is
whatever the interval timer's programmed period happens to be, and a
program that reprograms the timer for its own purposes changes the blink
rate as a side effect.

The renderer's involvement is a single read at the call site, from
[`crates/coco-core/src/machine/render.rs:52-54`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L52-L54):

```rust
        // Blink phase is toggled by the GIME interval timer, which BASIC
        // programs at hi-res text setup (SEB Unravelled II).
        let blink_on = self.bus.gime.blink_state;
```

That one `bool` travels down through `paint_scanline` into
`paint_text_row`, where it does its work in two lines:

```rust
            if attr & ATTR_BLINK != 0 && blink_on {
                code = BLANK_CHAR;
            }
```

`BLANK_CHAR` is `0x20`, a space
([`gime_video.rs:41-42`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L41-L42),
whose comment credits MAME's `get_data_with_attributes`). Note what this
is *not*: blink is not implemented by drawing the glyph and then hiding it,
nor by swapping foreground for background. The character code itself is
replaced with a space before the font is even consulted. Everything
downstream — glyph lookup, pixel loop, background fill — proceeds
completely unaware that anything unusual happened. That is the cheapest
possible implementation and, as it happens, exactly what the hardware
documentation describes.

[`tests/gime_irq.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/gime_irq.rs)'s
`rom_programs_the_timer_and_blink_phase_toggles` confirms that the real ROM
programs the timer with `$FFFF` at cold start, specifically so that
blinking works the instant a program sets a blink attribute, without BASIC
ever having to touch the timer registers itself. That test belongs with the
timer material rather than here, and it comes with a caveat worth flagging
now: it boots the real `coco3.rom`, so like `gime_modes.rs` in §8.8 it
cannot run in a checkout without a `roms/` directory. §8.8 addresses that
situation honestly rather than pretending it away.

### The `WIDTH 80` HRES quirk, restated precisely

§8.1 established that `TEXT_COLS` has duplicated entries. Seeing that
duplication land on real ROM values matters, because it is the difference
between a curiosity and a fact that changes how a register dump is read.

SEB Unravelled II's disassembly gives `WIDTH 40` the `$FF99` value `$05`,
whose `HRES` field is `(0x05 & 0x1C) >> 2 = 1`. `WIDTH 80` uses `$15`,
whose `HRES` field is `(0x15 & 0x1C) >> 2 = 5`. Those two field values,
`%001` and `%101`, differ only in the top bit, and `TEXT_COLS[1] == 40`
while `TEXT_COLS[5] == 80` — precisely the two answers a reader would hope
for.

The quirk is what happens to the *middle* bit. `TEXT_COLS[0]` and
`TEXT_COLS[2]` are both 32; `TEXT_COLS[1]` and `TEXT_COLS[3]` are both 40.
Setting `HRES` bit 1 in a text mode changes nothing at all. The test
`decodes_text_modes` in `render_gime.rs` asserts this directly, using
`$FF99 = 0x08`, an `HRES` field of `%010`, and expecting the same 32
columns that `%000` gives.

What makes this a design point rather than a footnote is *where* the quirk
lives in the source. Nothing computes "ignore bit 1." There is no `& !0x02`
anywhere in the decode path, and no comment explaining an exception. The
behavior falls out entirely from two duplicated pairs of entries in a
table. A reader decoding a register dump never has to remember the
exception, because indexing the table with the raw field value produces the
right answer whether or not they noticed the wrong bit was set. Encoding a
hardware quirk in data beats encoding it in control flow, whenever the
quirk is a function of one field's value — and §8.3 shows what happens when
it is not.

### Walking `paint_text_row`

Here is the function that turns all of the above into pixels. The framing
matters: this paints *one scanline* of *one text row* into the active pixel
span that `paint_side_borders` handed back. It has no idea which canvas row
it is on, no idea how many rows precede it, and no access to RAM — only to
a `fetch` closure that answers "give me byte *i* of the current row."

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

([`gime_video.rs:303-348`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L303-L348).)
Walk it in the order it executes.

The four setup lines establish everything that is constant across the row.
`bytes_per_char` is 1 or 2, and every subsequent `fetch` index is scaled by
it. `underline` is the `Option<usize>` from a page ago, computed once
rather than per character. `native_w` is how wide this row would be at one
pixel per font pixel: columns times the fixed 8-pixel cell width.

`xscale` is the line that makes Option B work. The renderer always draws at
the font's native resolution and then duplicates each pixel an integer
number of times to fill whatever active span it was given. An 80-column
wide mode has `native_w = 640` and `active_w = 640`, so `xscale = 1` and
nothing is duplicated. A 40-column mode has `native_w = 320` and, because
40-column modes are wide, `active_w = 640`, so `xscale = 2` and every
native pixel becomes a two-pixel block. A 64-column mode has `native_w =
512` and `active_w = 512`, so `xscale = 1` again, with the difference
showing up as border rather than scaling. Geometry differences between
modes are absorbed entirely by this one integer, which is what "one fixed
framebuffer for every mode" costs in practice.

The per-column body reads the character code first, then branches on
whether this mode has attributes. In the attribute case a second `fetch`
one byte later reads the attribute, and three things come out of it as a
tuple: a resolved foreground color, a resolved background color, and a
boolean saying whether the underline applies. Notice that the tuple's third
element repeats the blink condition rather than reusing a variable:
`attr & ATTR_UNDERLINE != 0 && !(attr & ATTR_BLINK != 0 && blink_on)`. That
repetition is deliberate and encodes a real rule. A cell that is currently
blinked off shows nothing at all, not even its underline. Without that
clause, an underlined blinking character would blink its glyph while its
underline stayed stubbornly lit.

The glyph row lookup is where the 8-row font meets a possibly taller cell.
`line_in_row` indexes straight into the glyph when it is in range and
contributes an all-zero row when it is not, which is the padding the font's
header comment describes.

`underline_here` combines the two conditions: this cell is underlined, and
this particular scanline is the mode's underline row. For an eight-line
cell that is true on exactly one of the eight scanlines the cell occupies.

Finally the per-pixel loop. `0x80 >> cx` walks the glyph byte from the most
significant bit down, so bit 7 is the leftmost pixel on screen — the
convention anyone who has hand-drawn a character bitmap on graph paper will
expect. A pixel is lit if the underline forces it or the glyph bit is set,
and `fill` writes `xscale` copies of the chosen color before `x` advances
by the same amount. There is no separate "background pass": every pixel in
the span is written exactly once, in this loop, in either the foreground or
the background color.

> **Rust corner: a closure as the fetch strategy, monomorphized.**
> `fetch: impl Fn(usize) -> u8` is the same principle as Chapter 1's `Bus`
> trait. `impl Trait` in a function's parameter position resolves to one
> concrete, inlined type per call site rather than a runtime-dispatched
> trait object. Here it buys something specific: `paint_text_row` and
> `paint_graphics_row` do not need to know *how* a byte gets from RAM to
> them. The caller decides, and there are three different callers with
> three different answers — a straight index, the 256-byte seam-wrapped
> index from §8.1, and the struct-based `Scanout::fetch` used by the debug
> text dump in §8.6.
>
> The construction site is `paint_body_row`, which builds exactly one
> closure and hands it to whichever painter the mode bit selects
> ([`gime_video.rs:231-240`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L231-L240)):
>
> ```rust
>     let fetch = |i: usize| ram[(row_base + ((x_offset + i) % ROW_FETCH_WRAP)) % ram.len()];
>     if g.vmode & vmode::BP != 0 {
>         let mode = decode_graphics(g);
>         paint_graphics_row(&mode, palette, active_w, fetch, active);
>         mode.bytes_per_row
>     } else {
>         let mode = decode_text(g);
>         paint_text_row(&mode, palette, blink_on, line_in_row, active_w, fetch, active);
>         mode.cols * if mode.attributes { 2 } else { 1 }
>     }
> ```
>
> The closure captures `row_base`, `x_offset`, and `ram` by reference and
> is passed by value into the painter, which monomorphizes against its
> anonymous type and inlines the body. One trait bound, three call sites,
> zero indirection at runtime. The return value matters too: each branch
> reports how many bytes the row consumed, which is what `advance_scan`
> needs for its non-`HVEN` pitch. The `BP` bit is tested here and nowhere
> else in the painting path.

---

## 8.3 Graphics: HSCREEN modes

With `BP` set, the same two registers describe something structurally
different: a packed-pixel bitmap instead of a character grid. There is no
font, no cell height, and no attribute byte. `HRES` sets how many bytes are
fetched per row, `CRES` sets how many bits each pixel occupies, and the
pixel width follows arithmetically from the two.

That last relationship is the one to internalize, because it explains the
whole `HSCREEN` family. Bytes per row is *bandwidth*: how much data the
video hardware pulls out of RAM for each displayed line. Bits per pixel is
how that fixed bandwidth is spent. Spend more bits per pixel and you get
more colors and fewer pixels; spend fewer and you get more pixels in
fewer colors. The chip does not offer a way to have both.

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

([`gime_video.rs:102-115`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L102-L115).)
The first three lines are the same table-indexing pattern as `decode_text`,
and `width: bytes_per_row * 8 / bpp` is the arithmetic just described:
eight bits per byte, divided among pixels.

The `if` in the middle is not decorative. It is a documented hardware
quirk, and the function's own doc comment states the provenance:

```rust
/// Decode the graphics mode from the GIME video registers ($FF98/$FF99).
///
/// HRES=%110/%111 (128/160 bytes per row) with CRES=%00 is not a guaranteed
/// combination (SEB Unravelled II Fig 5) and the chip does not produce a
/// 1024/1280-px picture: MAME `gime.cpp` (cases `0x18/0x19`, `0x1c/0x1d`)
/// aliases CRES=0 to the CRES=1 renderer there, so this decode does too.
```

([`gime_video.rs:96-101`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L96-L101).)
Follow the arithmetic that the quirk prevents. At 160 bytes per row and one
bit per pixel, `width` would come out as `160 × 8 / 1 = 1280` pixels — five
times the width of a 256-pixel VDG screen, on a machine whose canonical
raster is 640 pixels wide. The chip does not do this. The combination is
listed as "not guaranteed" in the reference book, MAME aliases it to the
two-bits-per-pixel renderer, and this codebase follows MAME. The comparison
`bytes_per_row > gime::GFX_BYTES_PER_ROW[5]` is written against the table
rather than against a literal 80, so that the condition stays correct if
anyone ever revises the table.

This is the graphics-mode analogue of §8.2's text-mode `HRES` quirk, and
the contrast between how the two are handled is the interesting part. The
text quirk depends on one field's value alone, so it was baked into
`TEXT_COLS`'s duplicated entries and disappeared from the code entirely.
This one depends on *two* fields at once, `HRES` and `CRES` together, and
no single-field lookup table can express a conditional relationship between
two independent indices. So it becomes an explicit `if`, with a comment
citing both sources. The rule is not "always put quirks in tables"; it is
"put a quirk in a table when the table's own index determines it, and in
code with a citation when it does not."

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

([`gime_video.rs:352-377`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L352-L377).)
Compare its shape to `paint_text_row`: the same `xscale` computation, the
same `fetch`/`fill` vocabulary, the same single pass writing every pixel
once. What is gone is the font. A graphics pixel *is* its own palette
index; there is nothing between the byte in RAM and the color on screen
except a shift and a mask.

Trace `j` through a four-bits-per-pixel byte, the `HSCREEN 2` case.
`pixels_per_byte` is `8 / 4 = 2` and `value_mask` is `0b1111`. For `j = 0`,
`shift = 8 - 4 × 1 = 4`, which selects the high nibble. For `j = 1`,
`shift = 8 - 4 × 2 = 0`, the low nibble. So the first pixel of the pair
comes from bits 7–4 and the second from bits 3–0: most significant bits
first, exactly matching how anyone hand-encoding `HSCREEN` pixel data would
shift the leftmost pixel into the top nibble.

The one-bit-per-pixel case works the same way with eight iterations instead
of two: `shift` runs 7, 6, 5, down to 0, taking each bit left to right.
Every depth this chip supports uses the same convention, which is worth
noting because it is one of the two or three facts most likely to be got
backwards on a first implementation.

Each decoded value indexes `palette` directly, with no offset and no
`ATTR_FG_BASE`-style adjustment. Graphics modes use the palette registers
starting from 0, and how many of the sixteen are reachable is purely a
function of `bpp`. A one-bit-per-pixel mode can only ever produce indices 0
and 1, so registers 2 through 15 are never selected no matter what they
contain. A four-bit mode reaches all sixteen. This is the whole of §8.4's
"which registers does a mode use" table, expressed as arithmetic.

The defensive early `return` deserves a note for what it is *not*. It does
not model any hardware behavior. It is a guard against a decode producing
more pixels than the active span has room for, which should not happen for
any legal `HRES`/`CRES` combination given the aliasing fix above. It costs
one comparison per pixel and converts a would-be out-of-bounds panic into a
silently truncated row. For a renderer that is the right trade: a slightly
wrong picture is a far better failure than a crashed emulator, and a
truncated row is visible enough that nobody will ship it by accident.

### HSCREEN 1–4: the ROM's own register images

`HSCREEN n` in BASIC does not compute a register value at run time. It
looks one up. SEB Unravelled II's disassembly shows the table, at `$E06C`
and labeled `RESTABLE`, that the ROM indexes with `HSCREEN`'s argument
minus one:

```
* VIDEO RESOLUTION MODE REGISTER (FF99) DATA FOR HSCREEN MODES
RESTABLE FCB   $15        320 PIXELS, 4 COLORS
         FCB   $1E        320 PIXELS, 16 COLORS
         FCB   $14        640 PIXELS, 2 COLORS
         FCB   $1D        640 PIXELS, 4 COLORS
```

Four bytes, four modes, and the ROM's own comments claim a pixel width and
a color count for each. Running those four bytes through the
`decode_graphics` machinery just walked confirms every one of them:

| HSCREEN | `$FF99` | `HRES` field | bytes/row | `CRES` field | bpp | colors | width |
|---|---|---|---|---|---|---|---|
| 1 | `$15` | `%101` (5) | 80 | `%01` | 2 | 4 | 320 |
| 2 | `$1E` | `%111` (7) | 160 | `%10` | 4 | 16 | 320 |
| 3 | `$14` | `%101` (5) | 80 | `%00` | 1 | 2 | 640 |
| 4 | `$1D` | `%111` (7) | 160 | `%01` | 2 | 4 | 640 |

Three observations, in increasing order of usefulness.

Every `HRES` field in the table, `%101` and `%111`, has its low bit set.
Every `HSCREEN` mode is therefore a wide mode in the §8.1 sense: the full
640-pixel canvas, no side borders, with `xscale` doing the work of turning
320 native pixels into 640 canvas pixels where necessary.

`HSCREEN 1` and `HSCREEN 3` share an `HRES` field, and so share a bytes-per-row
figure of 80, despite producing pictures of different widths. `HSCREEN 2`
and `HSCREEN 4` do the same thing at 160 bytes per row. In each pair, the
byte bandwidth is identical and only `CRES` differs. The chip fetches
exactly as much data for a 640×2 screen as for a 320×4 one; what changes is
how those bytes are sliced.

Which means the four `HSCREEN` modes are really two bandwidth choices
crossed with a colors-versus-resolution choice. That relationship is the
subject of Exercise 8.2, and it has a consequence worth predicting before
computing: two modes with the same bytes per row and the same line count
occupy exactly the same amount of screen memory, regardless of how
different they look.

The ROM's video-mode RAM images (the `IM.GRAPH` block at `$E079` in SEB
Unravelled II) settle the video base as well. Both the 320-wide and the
640-wide hi-res graphics images write `$FF9D = $C0` and `$FF9E = $00`,
giving `vertical_offset = 0xC000` and `video_base = 0xC000 << 3 = 0x60000`.
Every `HSCREEN` mode BASIC sets up lives at the same fixed physical
address. Only the resolution registers change, which is to say: only the
*interpretation* applied to bytes at a fixed location changes. The same
30,720 bytes are a 640×4-color picture or a 320×16-color picture
depending on two bits in `$FF99`.

### Worked example: `HSCREEN 3`'s register image

Same treatment `WIDTH 80` got in §8.1, on the mode that produces the
highest-resolution picture the machine can show:

| Register | Value |
|---|---|
| `$FF90` | `$4C` |
| `$FF98` | `$80` |
| `$FF99` | `$14` |
| `$FF9D`/`$FF9E` | `$C0`/`$00` |

`$FF98 = $80` sets `BP`, so this is graphics. Its `LPR` field is `%000`,
giving `LPR_LINES[0] = 1`: one scanline per fetched row, rather than the 8
a text mode used. That makes sense the moment it is stated plainly. A
graphics "row" is a single line of pixels, redrawn from fresh bytes on
every scanline. There is no multi-scanline character cell to repeat, so the
row pointer advances every line — which is exactly what `advance_scan` does
when `lines_per_row()` returns 1.

`$FF99 = $14` decodes through the table above: 80 bytes per row, one bit
per pixel, 640 pixels wide, two colors, with an `LPF` field of `%00`
giving 192 active lines.

`$FF9D`/`$FF9E` gives `video_base = 0xC000 << 3 = 0x60000`, the fixed
`HSCREEN` address.

The full description: a 640×192 two-color picture at physical `$60000`,
occupying `80 × 192 = 15,360` bytes, or `$3C00` — physical `$60000` through
`$63BFF`. Palette registers 0 and 1 are the only ones this mode can ever
select, for the reason `paint_graphics_row` made arithmetic: a one-bit
value cannot index higher than 1.

Fifteen kilobytes for a full-screen monochrome picture is a genuinely small
number by any modern standard, and it is still larger than the entire
screen memory of most machines this one competed with. It is also, per
§8.1, still large enough that the physical-addressing trick earns its keep:
15,360 bytes spans two 8K MMU slots, and a program that had to keep both
mapped into its own 64K at all times would be giving up a quarter of its
address space just to keep the screen visible.

---

## 8.4 The palette

Sixteen registers, `$FFB0` through `$FFBF`, each holding one six-bit
color value. Every foreground, background, and pixel color the
GIME-native renderer draws is one of those sixteen entries. The border is
the single exception, carrying its own independent six-bit value in
`$FF9A` rather than an index into the table, as §8.1 established.

Sixteen is a small number, and the constraint shaped how CoCo 3 software
looked. A 16-color `HSCREEN 2` screen uses every register the chip has,
which means a program that wants a different palette for a different part
of the screen has exactly one option: change the registers while the beam
is between the two parts. That is Chapter 9's material, but the pressure that
motivates it is visible here, in the size of the table.

### Which registers a mode actually uses

SEB Unravelled II's Figure 13 tabulates the mapping precisely, and it
matches what §8.2's `ATTR_FG_BASE`, `NO_ATTR_FG`, and `NO_ATTR_BG`
constants and §8.3's un-offset graphics indexing already said in code:

| Mode | Registers used |
|---|---|
| Hi-res text, no attributes | bg = reg 0, fg = reg 1 |
| Hi-res text, with attributes | bg = regs 0–7, fg = regs 8–15 |
| Hi-res graphics, 16 color | 0–15 |
| Hi-res graphics, 4 color | 0–3 |
| Hi-res graphics, 2 color | 0–1 |

Notice that no mode except 16-color graphics uses all sixteen registers
as a flat pool. Attributed text splits them into two halves of eight.
Low-color graphics modes use a prefix of the table and leave the rest
untouched. A program switching between `HSCREEN 3` and attributed text
therefore has to think about the palette twice: registers 0 and 1 mean
"the two graphics colors" in one mode and "the first background and the
second background" in the other.

The legacy 32×16 VDG-compatible text mode from Chapter 7 sits outside this
table entirely. It fixes background to register 12 and foreground to
register 13 regardless of `BP` or attributes, because it does not go
through any of these decode paths — it is the compatibility renderer, and
it answers to a different set of rules.

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

([`crates/coco-core/src/gime/palette.rs:53-62`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs#L53-L62).)
Six bits, three channels, two bits each. The name `RGBrgb` describes the
bit order and is worth spelling out, because the two bits of a channel are
not adjacent. The *high* bit of each channel lives in the upper half of the
register — bits 5, 4, and 3, the capital `RGB` — and the *low* bit lives in
the lower half, bits 2, 1, and 0, the lowercase `rgb`. SEB Unravelled II
states the assignment directly:

```
Bit 5   (R1) High Order Red
Bit 4   (G1) High Order Green
Bit 3   (B1) High Order Blue
Bit 2   (R0) Low Order Red
Bit 1   (G0) Low Order Green
Bit 0   (B0) Low Order Blue
```

That is exactly `chan(5, 2)` for red, `chan(4, 1)` for green, and
`chan(3, 0)` for blue, matching the code's argument order pair by pair. The
closure reassembles each channel by shifting its high bit up one position
and ORing in its low bit, producing a value from 0 to 3.

The `× 0x55` that follows is the only sensible way to spread four evenly
spaced levels across a full byte using integer multiplication. `0x55 × 0`
is 0, `0x55 × 3` is `0xFF`, and the two intermediate values land at `0x55`
and `0xAA`. No gap at the black end, no gap at the white end, and equal
spacing between. A naive `v << 6` would top out at `0xC0` and leave the
brightest color visibly dim; a `v * 85` written as a decimal literal would
say the same thing less legibly.

A worked example, taken from SEB Unravelled II's own color-derivation
walkthrough rather than invented here, makes the whole path concrete.
Build "purple" as Red = 2, Green = 1, Blue = 3, each on the 0-to-3 scale.
Per the bit table, red's two bits are `10`, green's are `01`, and blue's
are `11`. Assemble them in register order — bit 5, bit 4, bit 3, bit 2, bit
1, bit 0 — and the pattern is `1 0 1 0 1 1`, which is `0b101011`, or 43 in
decimal. That is the value SEB gives.

Now run 43 back through `rgb_color`. `chan(5, 2)` sees bit 5 set and bit 2
clear, so `v = 1 << 1 | 0 = 2` and red is `2 × 0x55 = 0xAA`. `chan(4, 1)`
sees bit 4 clear and bit 1 set, so `v = 1` and green is `0x55`.
`chan(3, 0)` sees both bits set, so `v = 3` and blue is `0xFF`. The result
is `(0xAA, 0x55, 0xFF)`: mostly blue, a fair amount of red, a little green.
A purple-leaning blue, exactly as the channel strengths promised. The
book's illustration and the shipping function agree line for line, which is
the kind of cross-check worth doing once for a color format before
trusting it for a whole chapter.

This is the RGB-monitor path, and only that. A composite monitor resolves
the very same six-bit value through an entirely different mechanism: a
hand-measured 64-entry lookup table, `COMPOSITE_PALETTE` and its
burst-phase-inverted twin `COMPOSITE_PALETTE_180`, both in the same file.
There is no formula for those, and the table's own comment says so
plainly. Composite color is not a linear function of the register bits
but an artifact of how the CoCo's composite encoder modulates those bits
onto a color subcarrier. That table, the `BPI` bit from `$FF98`, and the
`MOCH` grayscale averaging are Chapter 9's subject in full. This chapter's job
was only to establish the contrast: the RGB path is a clean derivable
formula, the composite path is measured data, and both are reached through
the same six bits.

### Caching: `resolve_colors`, once per scanline

```rust
/// Resolve the 16 GIME palette registers and the $FF9A border to RGBA.
fn resolve_colors(g: &GIME) -> ([[u8; 4]; PALETTE_LEN], [u8; 4]) {
    let mut palette = [[0u8; 4]; PALETTE_LEN];
    for (entry, &reg) in palette.iter_mut().zip(&g.palette) {
        *entry = g.color(reg);
    }
    (palette, g.color(g.border & BORDER_COLOR_MASK))
}
```

([`gime_video.rs:161-168`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L161-L168).)
`paint_scanline` calls this once, at the top of every scanline, converting
all sixteen palette registers plus the border into resolved RGBA up front.
The resulting array is what gets handed down to `paint_text_row` and
`paint_graphics_row`, which is why those functions take
`palette: &[[u8; 4]; PALETTE_LEN]` rather than a raw register array plus a
`GIME` reference. By the time a painter runs, no color arithmetic remains
to be done; every possible answer is already sitting in a sixteen-entry
lookup.

The grain of that caching is a deliberate choice with consequences in both
directions. Resolving per *pixel* would mean running the `× 0x55`
arithmetic, or a 64-entry composite lookup, up to 640 times per scanline to
produce what are only ever sixteen distinct answers. Resolving per *field*
would be cheaper still, and would be wrong: a program that changes a
palette register partway down the screen expects the change to show up
below that point, and Chapter 9 is built on exactly that expectation. Once per
scanline is the coarsest grain that still gets mid-field palette changes
right, and it is no accident that it matches the granularity of everything
else in this renderer.

Note also that `resolve_colors` calls `g.color(...)`, not
`GIME::rgb_color(...)`. The former dispatches on the machine's configured
monitor type and is the reason composite support required no changes to any
painter — a fact Chapter 9 opens with.

### What `COLOR` and `PALETTE` actually write

BASIC's `PALETTE` command, at `$E60C` in SEB Unravelled II's disassembly,
is a direct register poke with bounds checking. Its own comments are worth
reading verbatim, because one of them ties straight back to the live-versus-
latched distinction §8.5 is about to make:

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

Two different validation policies sit in that fragment, which is itself
informative about 1986 ROM design. An out-of-range *register number* is an
error: `?FC ERROR`, refuse the operation. An out-of-range *color value* is
silently clamped to 63. The register index has to be right because there is
no sensible seventeenth register; the color is a matter of degree, and the
ROM's authors evidently decided that giving the programmer the brightest
available color beat interrupting them with an error message.

The three instructions before the store are the part that matters here.
`ORCC #$50` masks both interrupt lines. `SYNC` halts the CPU until an
interrupt asserts — the same instruction Chapter 4 built, used here for its
halting behavior rather than for the interrupt itself, since interrupts
are masked and the handler will not run. Then, and only then, the store.

The comment explains the purpose without hedging: this prevents the screen
from flashing when the change is made. The reason the flash exists at all
is precisely the design decision described two paragraphs above.
`resolve_colors` re-reads the live registers every scanline, on purpose, so
that a mid-frame change takes effect immediately. That immediacy is a
feature and Chapter 9 depends on it. But a feature that makes deliberate
mid-frame changes possible also makes *careless* mid-frame changes visible,
as a color flash or tear at whatever scanline the write happened to land
on. So here are the ROM's authors, in 1986, already reaching for the same
"sync your writes to blanking" discipline that every raster programmer
eventually arrives at independently.

`COLOR` is the less interesting sibling: rather than taking a register
number and a value, it resolves to writes at fixed register pairs — the
background and foreground registers for whichever mode is currently active,
per Figure 13 above. Convenience over generality, which is also why a
program doing anything unusual with the palette reaches for `PALETTE`
instead.

[`crates/coco-core/examples/palette_trace.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/palette_trace.rs)
is a small debugging harness built around exactly this behavior. It boots
a cartridge, single-steps the machine, and logs every change to
`bus.gime.palette` alongside the program counter that caused it, so a real
program's palette discipline — or lack of it — scrolls past as it runs. It
needs a real ROM and a cartridge image, so it is a skim-only mention here
rather than a lab exercise — but well worth remembering the next time a
color glitch in a real program needs explaining.

---

## 8.5 `FieldScan`: what's latched, what's live

Nearly every register this chapter has covered is read *fresh, on every
single scanline*. `$FF98`, `$FF99`, `$FF9A`, and `$FF9F` are all consulted
live, straight off the `GIME` struct, with no caching beyond §8.4's
per-scanline color resolve. A 6809 program that changes the border color,
the mode bits, or the horizontal offset between one scanline and the next
sees that change take effect on the very next line painted.

That is the mechanism behind raster splits, though not yet their use.
Because nothing about painting one line requires the registers to hold
still for a whole field, nothing stops a program from changing them
mid-field. The renderer did not need a feature added to support
splits; it needed a feature *not* added — no snapshot, no per-field cache,
no "mode change takes effect next frame" logic.

But three pieces of state are not read live. They are sampled exactly once,
at the very start of a field, and held fixed no matter what the registers
do afterwards. That grouping matches the model in MAME's `gime.cpp`, whose
`new_frame` routine samples the same registers. The agreement is useful
evidence for the model; it is not a complete account of the physical chip.

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

([`gime_video.rs:117-140`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L117-L140),
doc comment and fields.) Three fields, and each one answers a question that
a mid-field register change would otherwise make unanswerable.

`legacy` holds the value of the `INIT0` `COCO` bit at the instant the field
started. If a program flips into or out of CoCo-compatible mode partway
down the screen, the change waits for the next field. This is the one
latch that is clearly a *simplification of the physical chip in service of
sanity*: a field never has to decide, halfway down its own body, whether
it is painting VDG-compatible or GIME-native. The two paths fetch data
in fundamentally different ways — one through the bus honoring the MMU,
the other straight out of physical RAM. Splitting a field between them
would require reconciling two incompatible addressing models mid-line.
MAME's `m_legacy_video` makes the same choice.

`row_base` is seeded once from `video_base()`, §8.1's physical-address
computation. A program that repoints `$FF9D`/`$FF9E` mid-field does not
move the picture it is currently in the middle of drawing; the new base
takes effect at the next field's latch. That is what makes double-buffering
by register write safe: the switch is atomic with respect to a displayed
frame, so a viewer never sees the top half of one buffer above the bottom
half of another.

`line_in_row` is seeded from the `$FF9C` vertical-scroll register, but only
when that value makes sense for the current row height:

```rust
    /// Latch the per-field register group (MAME `new_frame`). Legacy fields
    /// seed from the SAM-compat page base with `line_in_row` 0 (MAME:
    /// `m_line_in_row = COCO ? 0 : vsc`).
    pub fn latch(g: &GIME, legacy: bool) -> Self {
        let vsc = (g.vertical_scroll & 0x0F) as usize;
        let lpr = g.lines_per_row();
        Self {
            legacy,
            row_base: if legacy {
                g.sam_display_base() as usize
            } else {
                g.video_base()
            },
            line_in_row: if legacy || vsc >= lpr { 0 } else { vsc },
        }
    }
```

([`gime_video.rs:143-158`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L143-L158).)
The guard is the clause `vsc >= lpr`, and it handles a register combination
that is easy to write and impossible to honor: a smooth-scroll seed of 5
in a mode whose character rows are only 2 scanlines tall. Starting at line
5 of a 2-line row is not a position that exists. The code does not panic,
does not wrap the value into range, and does not clamp it to the last valid
line — it falls back to 0 and ignores the register for that field. The
in-range seed path is pinned by the test
`vertical_scroll_starts_field_mid_character_row`; the guard itself has no
test yet, and it is exactly the sort of decision worth writing one for even
when it seems obvious, because "what does the hardware do with a nonsensical
value" has three plausible answers and only one right one. (Consider that a
standing invitation.)

One more thing in `row_base`: a legacy field seeds from `sam_display_base()`,
the 16-bit logical SAM page base from Chapter 7, rather than from the physical
`video_base()`. One struct, two addressing models, selected by the same
latched bit that picks the painter.

### Where the latch actually happens

The machine loop pins down what "field start" means in scanline terms:

```rust
    pub(super) fn render_scanline(&mut self) {
        if self.config.variant != MachineVariant::Coco3 {
            return;
        }
        if self.line == 0 {
            let legacy = self.bus.gime.init0 & gime::init0::COCO != 0;
            self.field_scan = Some(gime_video::FieldScan::latch(&self.bus.gime, legacy));
            self.framebuffer
                .resize(raster::CANVAS_W * raster::CANVAS_H * BYTES_PER_PIXEL, 0);
            self.fb_width = raster::CANVAS_W as u32;
            self.fb_height = raster::CANVAS_H as u32;
        }
```

([`crates/coco-core/src/machine/render.rs:29-40`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L29-L40).)
`render_scanline` runs once per scanline, called from `end_of_line` in
Chapter 6's timing loop, and the latch happens inline on line 0's own call
rather than in a separate "start of field" hook. Fewer moving parts, one
less thing to forget to call, and the framebuffer resize sits in the same
branch for the same reason.

The function's doc comment is candid about the one consequence: this is
"one line-time later than MAME's field start, within the plan's
line-granular contract." A deliberate, documented divergence from the
reference implementation rather than an oversight — the same species of
decision this course flagged in Chapter 4's testing chapter and in Appendix A.
One scanline of vertical-blanking timing difference has never mattered for
any test in this codebase, and when it eventually does, the comment tells
the next reader exactly where to look.

### `paint_scanline`: the whole per-line contract in one function

Everything in this chapter meets in one place. This is the function the
machine loop calls once per scanline, and reading it top to bottom is the
best available summary of the chapter:

```rust
pub fn paint_scanline(
    g: &GIME,
    ram: &[u8],
    scan: &mut FieldScan,
    blink_on: bool,
    row: usize,
    fb: &mut [u8],
) {
    debug_assert!(row < CANVAS_H);
    let (palette, border) = resolve_colors(g);
    let row_px = &mut fb[row * CANVAS_W * BYTES_PER_PIXEL..][..CANVAS_W * BYTES_PER_PIXEL];

    if !in_active_rows(g, row) {
        fill(row_px, border);
        return;
    }

    let (x0, active_w) = paint_side_borders(g, row_px, border);
    let active = &mut row_px[x0 * BYTES_PER_PIXEL..][..active_w * BYTES_PER_PIXEL];

    // Per-line live fetch parameters ($FF9F offset).
    let x_offset = (g.horizontal_offset & hoff::X_MASK) as usize * 2;
    let row_bytes = paint_body_row(
        g,
        ram,
        scan.row_base,
        x_offset,
        &palette,
        blink_on,
        scan.line_in_row,
        active_w,
        active,
    );

    advance_scan(scan, g, row_bytes);
}
```

([`gime_video.rs:264-299`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L264-L299).)
Count the live register reads. `resolve_colors` reads all sixteen palette
registers and `$FF9A`. `in_active_rows` reads `$FF99`'s `LPF` field.
`paint_side_borders` reads `$FF99`'s `HRES` field. The `x_offset`
computation reads `$FF9F`. `paint_body_row` reads `$FF98`'s `BP` bit and
then the whole of whichever mode it selects. `advance_scan` reads `$FF9F`
again for `HVEN` and `$FF98` for `LPR`. Four of the eight bytes in the
group — `$FF98`, `$FF99`, `$FF9A`, and `$FF9F` — are therefore consulted
on every single scanline, several of them more than once. The ones that
never are, `$FF9C` and the `$FF9D`/`$FF9E` pair, are precisely the ones
`FieldScan` latched, and `$FF9B` latches with them because it contributes
only through `video_base()`.

The only state that crosses from one call to the next is `scan`, and it
carries exactly two numbers: where the current data row starts and how far
into that row's scanlines the beam has got. There is no "current mode"
carried forward, no cached geometry, no dirty flag. Each call re-derives
everything else from registers.

The early return for non-body rows is where the border gets painted for the
top and bottom of the screen, and its guard reads the vertical geometry
live:

```rust
/// True when canvas `row` falls within the active (non-border) vertical
/// window, from the LIVE LPF bits (applies even mid-frame; the glitched %10
/// value is approximated, see [`vertical_window`]).
fn in_active_rows(g: &GIME, row: usize) -> bool {
    let lpf = ((g.vres & vres::LPF_MASK) >> vres::LPF_SHIFT) as usize;
    let (top, body) = vertical_window(lpf);
    row >= top && row < top + body
}
```

([`gime_video.rs:188-195`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L188-L195).)
And `vertical_window` is the vertical counterpart of the wide/non-wide
table from §8.1 — another lookup that places an active body of a given
height inside a fixed 240-row canvas:

```rust
pub const fn vertical_window(lpf: usize) -> (usize, usize) {
    match lpf {
        0 => (25, 192),
        1 => (23, 200),
        2 => ((CANVAS_H - 210) / 2, 210),
        _ => (8, 225),
    }
}
```

([`raster.rs:33-40`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/raster.rs#L33-L40).)
Each pair is a top-border row count and a body row count, and the doc
comment above it records that the 192, 200, and 225 cases come from MAME's
`update_geometry`, where each sums with its bottom border to exactly 240.
The `LPF = %10` case is the glitched one from §8.1 and gets a centered
approximation rather than a measured number.

That first pair, `(25, 192)`, is where the constant `TOP = 25` in
`render_gime.rs` comes from. Every pixel assertion in §8.7 is written
relative to it, and now it is not a magic number.

> **Rust corner: the `[a..][..b]` slice window.** The expression
> `&mut fb[row * CANVAS_W * BYTES_PER_PIXEL..][..CANVAS_W * BYTES_PER_PIXEL]`
> appears in `paint_scanline`, in `paint_side_borders`, and twice in each
> painter's inner loop, so it repays one careful reading. It is two
> slicing operations chained: first take everything from the row's starting
> byte to the end of the buffer, then take the first `CANVAS_W ×
> BYTES_PER_PIXEL` bytes of *that*. The result is one row's worth of
> pixels, bounds-checked at both ends.
>
> The alternative spelling, `&mut fb[start..start + len]`, means the same
> thing and requires the reader to verify that `start` appears identically
> in both halves of the range. The chained form states the offset once, so
> a typo in it cannot produce a window of the wrong length at the wrong
> place — only a correctly-sized window at the wrong place, which is a far
> easier bug to see. In a renderer where every function receives a
> sub-slice of the framebuffer and never the whole thing, that discipline
> compounds: `paint_text_row` genuinely cannot scribble on another row,
> because the slice it holds does not reach one. Rust's bounds checking
> turns "the renderer wrote past the end of a row" from a class of memory
> corruption into a panic with a line number.

That containment is what makes `paint_scanline` safe to call one line at a
time: a call can only touch its own row, and the only state it carries
forward is `scan`. Both callers walk the rows in order — `render_field` in
§8.7 in a tight loop, the machine loop with a scanline's worth of CPU
execution between calls — and it is containment, not any whole-frame
bookkeeping, that lets the second of those work at all.

### What this chapter does not do with `FieldScan`

`row_base` and `line_in_row` are not read-only after latching.
`advance_scan`, from §8.1, mutates both as painting proceeds down the
field, stepping `row_base` forward by the current line's live pitch every
`lines_per_row()` scanlines. So `FieldScan` is best understood as two
different things bundled into one struct: a genuinely field-frozen part —
`legacy`, and the *initial* values the other two were seeded from — and a
running cursor that the per-line painter advances using registers it reads
live. The freezing happens exactly once, at latch time. The advancing
happens every line, using whatever `$FF98` and `$FF9F` say at that moment.

That is as far as this chapter goes. What is now established: which
registers freeze at field start, which do not, the exact function that does
the freezing, and the exact function that consumes the result. What remains
unestablished — and stays that way until next week — is what a program can
*do* with the knowledge: a border-color split timed off a scanline-count
interrupt, a mid-field mode change, or any of the raster tricks that made
CoCo 3 demos worth watching.
[`tests/scanline_split.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/scanline_split.rs)
is where that story gets told, through actual 6809 code running in ROM, and
it is Chapter 9's opening act.

---

## 8.6 The lab: `gime_demo.rs`

[`crates/coco-core/examples/gime_demo.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/gime_demo.rs)
is a from-scratch harness with no ROM, no boot sequence, and no
ROM-derived RAM state — just a `GIME` struct configured directly from Rust
and a block of RAM filled by hand. It exercises the two paths this chapter
covered, an 80-column attribute text screen and an `HSCREEN 2` color-bar
graphics screen, and writes each to a PPM file for eyeballing.

```
cargo run -p coco-core --example gime_demo /tmp
```

This runs with nothing but `cargo` and the crate — confirmed by running it
in a checkout that has no `roms/` directory at all. It prints:

```
wrote /tmp/text80.ppm (640x240)
wrote /tmp/hscreen2.ppm (640x240)
```

Both files are the full canonical 640×240 raster the real machine loop
produces, generated by the same `render_field` the tests use. Being able to
produce a real frame with six lines of setup and no ROM is worth pausing
on: it means every claim in §8.1 through §8.4 can be checked visually in
about four seconds, which is a substantially better debugging loop than
booting a machine and typing `WIDTH 80` at it.

The text-mode setup is where the chapter's arithmetic shows up as literals:

```rust
    let mut g = GIME::new();
    g.vmode = 0x03; // BP=0, LPR=8
    g.vres = 0x15; // 80 cols, attributes
    g.vertical_offset = (base >> 3) as u16;
    g.border = 0x12;
    // A CoCo-ish palette: mimic the ROM's defaults loosely.
    g.palette = [0, 9, 18, 27, 36, 45, 54, 63, 0, 63, 46, 26, 12, 5, 38, 56];
```

([`gime_demo.rs:25-31`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/gime_demo.rs#L25-L31).)
`vmode`, `vres`, and `border` are exactly the `WIDTH 80` values decoded by
hand in §8.1. This example uses the ROM's own register image rather than a
made-up one, which means the picture it produces is the picture a real
machine produces for those registers, modulo the palette and the screen
contents. Note `vertical_offset = (base >> 3) as u16` — the inverse of
§8.1's `video_base()`, dividing a physical address by 8 to get the register
value. Notice the palette's shape too: the first eight entries climb steadily
(0, 9, 18, …, 63) because they are backgrounds, and the second eight are
scattered because they are foregrounds, per §8.4's Figure 13 split.

Filling the screen is two nested loops, and the attribute byte is assembled
per cell:

```rust
    let msg = b"cocovm GIME 80-column text  ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789";
    for row in 0..24 {
        for col in 0..80 {
            let i = base + (row * 80 + col) * 2;
            ram[i] = msg[(col + row) % msg.len()];
            let fg = (row % 8) as u8;
            let bg = if row >= 16 { (row % 8) as u8 } else { 0 };
            let mut attr = (fg << 3) | bg;
            if row == 4 {
                attr |= 0x40; // underline
            }
            if row == 5 {
                attr |= 0x80; // blink
            }
            ram[i + 1] = attr;
        }
    }
```

([`gime_demo.rs:33-49`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/gime_demo.rs#L33-L49).)
The address arithmetic `base + (row * 80 + col) * 2` is §8.1's 160-byte row
pitch written a different way: 80 columns at 2 bytes each. `(fg << 3) | bg`
is §8.2's attribute layout assembled by hand — the foreground shifted into
bits 5–3, the background left in bits 2–0. Row 4 gets `0x40` for underline
and row 5 gets `0x80` for blink, matching the same constants
`paint_text_row` tests against.

Rendered, this produces a green-bordered screen showing 24 rows of the
message, each row shifted one character from the one above and drawn in a
different foreground color, row 4 underlined, row 5 marked as blinking.
The call passes `blink_on = false`, so row 5 appears normally in this
particular PPM. Passing `true` instead blanks it entirely, which is the
same behavior `blink_attribute_blanks_character_during_blink_phase`
asserts in §8.7's test file.

The `HSCREEN 2` half switches `vmode` and `vres` to the values from §8.3's
table and fills a 320×192 grid with drifting color bars: sixteen columns
of the full palette, each column's color index nudged by `y / 12` so the
drift is visible as a diagonal rather than as flat vertical stripes. Both
nibbles of each byte are written independently — `c << 4 | ((c + y as u8 /
12) & 0x0F)` — which is §8.3's MSB-first packing done by hand, the left
pixel of each pair in the high nibble.

### The text dump: reading a screen without rendering it

There is a second way to inspect a GIME text screen, and it is the one
worth reaching for when the question is "what characters are on screen"
rather than "what does it look like." `text_lines` decodes a hi-res text
field straight to strings:

```rust
pub fn text_lines(g: &GIME, ram: &[u8]) -> Vec<String> {
    let mode = decode_text(g);
    let bytes_per_char = if mode.attributes { 2 } else { 1 };
    let mut scan = Scanout::new(g, ram, mode.cols * bytes_per_char, mode.lines_per_row);
    let rows = mode.lines.checked_div(mode.lines_per_row).unwrap_or(0);

    let mut out = Vec::with_capacity(rows);
    for _ in 0..rows {
        let line = (0..mode.cols)
            .map(|col| {
                let code = scan.fetch(col * bytes_per_char) & CHAR_CODE_MASK;
                if (0x20..0x7F).contains(&code) { code as char } else { UNPRINTABLE_CHAR }
            })
            .collect();
        out.push(line);
        for _ in 0..mode.lines_per_row {
            scan.next_line();
        }
    }
    out
}
```

([`gime_video.rs:456-476`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L456-L476).)
Three details connect back to earlier sections. `mode.lines.checked_div(
mode.lines_per_row)` is the 192 ÷ 8 = 24 arithmetic from §8.1's worked
example, written defensively — `checked_div` returning `None` on the
`LPR = %111` sentinel yields zero rows rather than a division-by-zero
panic. The range test `(0x20..0x7F).contains(&code)` is §8.2's font layout
turned into a filter: codes below `$20` are the accented and special
glyphs, and rather than print a misleading control character the function
substitutes `UNPRINTABLE_CHAR`, which is `'.'`
([`gime_video.rs:441-444`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L441-L444)).
And `scan.next_line()` is called `lines_per_row` times per text row, which
is how a function that renders no scanlines still walks the same addresses
a scanline renderer would.

That address-walking is the reason `Scanout` exists as a separate struct
rather than the function computing addresses inline. Its doc comment states
the goal plainly:

```rust
/// Walks the GIME's video fetch addresses for the text-dump probe: a physical
/// row base advancing by the row pitch, with per-byte offsets (including the
/// $FF9F X offset ×2) wrapping at the 256-byte seam. Matches MAME
/// `record_scanline_res` / `new_frame`.
```

([`gime_video.rs:393-396`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L393-L396).)
`Scanout::fetch` implements the same 256-byte seam wrap as
`paint_body_row`'s closure, and `Scanout::new` applies the same `HVEN`
pitch rule and the same vertical-scroll guard as `FieldScan::latch`. The
duplication is real but bounded, and the function's own doc comment is
explicit that sharing `decode_text` between the probe and the renderer is
what keeps the two from drifting apart. It is also explicit about what the
probe deliberately does *not* do: it ignores attribute bytes' color,
blink, and underline fields entirely, and it fetches each text row once
instead of once per scanline. A debug dump that reported blinked-off cells
as blank would be actively unhelpful.

### Try it yourself: put your name on screen, with blink

Modify the harness to render your own message instead of the canned
string, then make your own name — and not the rest of the line — blink.
Controlling exactly which columns get the blink attribute bit, rather than
applying it to a whole row, is the point of the exercise and not an
incidental detail:

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

[`crates/coco-core/tests/render_gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_gime.rs)
is deterministic, ROM-free coverage for everything this chapter has walked
through: the decode functions, RAM scanout, attribute colors, blink,
underline, and bit unpacking. Fourteen tests, no fixtures, no boot
sequence, and a whole-file run measured in milliseconds.

Two helpers make every test in the file readable, and they are worth
meeting first because the assertions are unintelligible without them:

```rust
fn gime_with(vmode_val: u8, vres_val: u8) -> GIME {
    let mut g = GIME::new();
    g.vmode = vmode_val;
    g.vres = vres_val;
    g.vertical_offset = VOFF;
    // Identity-ish palette: register i holds colour value i (0–15 fit in 6 bits).
    for (i, reg) in g.palette.iter_mut().enumerate() {
        *reg = i as u8;
    }
    g
}

fn px(fb: &[u8], x: usize, y: usize) -> [u8; 4] {
    let i = (y * CANVAS_W + x) * BYTES_PER_PIXEL;
    fb[i..i + 4].try_into().unwrap()
}
```

([`render_gime.rs:47-62`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_gime.rs#L47-L62).)
The identity-ish palette is the trick that makes the assertions legible:
palette register *i* holds color value *i*, so an assertion that reads
`GIME::rgb_color(9)` is saying "this pixel came from palette register 9"
without any indirection to trace. `px` turns a canvas coordinate into the
four RGBA bytes at that point, so a test can assert on a single pixel by
position.

Both revolve around `render_field`, the whole-field convenience wrapper
that tests use instead of stepping the machine loop:

```rust
pub fn render_field(g: &GIME, ram: &[u8], blink_on: bool, fb: &mut Vec<u8>) -> (usize, usize) {
    fb.resize(CANVAS_W * CANVAS_H * BYTES_PER_PIXEL, 0);
    let mut scan = FieldScan::latch(g, false);
    for row in 0..CANVAS_H {
        paint_scanline(g, ram, &mut scan, blink_on, row, fb);
    }
    (CANVAS_W, CANVAS_H)
}
```

([`gime_video.rs:384-391`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L384-L391).)
Latch once, then call `paint_scanline` for all 240 canvas rows in order.
Because the registers never change during that loop, the result is exactly
what a real field would look like if no program touched a video register
mid-frame. The function's own doc comment is careful about this: it is "the
whole-field equivalent of stepping `paint_scanline` over every visible
row," and the machine loop deliberately does *not* use it, because
line-by-line painting is what lets mid-frame changes split the raster.

Read three of the tests the way the file was meant to be read: predict the
pixel, then check the assertion.

### `text_without_attributes_uses_palette_0_and_1`

```rust
    let g = gime_with(TEXT_LPR8, VRES_TEXT40);
    let mut ram = vec![0u8; RAM_LEN];
    ram[BASE] = b'A';

    let mut fb = Vec::new();
    let (fb_w, fb_h) = render_field(&g, &ram, false, &mut fb);
```

Start by decoding the registers, since the test's constants hide them.
`VRES_TEXT40` is `0x04`, so the `HRES` field is `(0x04 & 0x1C) >> 2 = 1`,
which `TEXT_COLS` maps to 40 columns. Field value 1 is `%001`, whose low
bit is set, so this is a *wide* mode despite having fewer columns than the
64-column mode that is not. Forty columns of 8-pixel glyphs is
`native_w = 320`, the active span is the full `active_w = 640`, and
therefore `xscale = 2`.

Now the glyph. `'A'` is `$41`, and `GIME_FONT[0x41]`'s row 0 is `0x10`.
Only bit 4 is set, and since `0x80 >> cx` walks from the left, bit 4
corresponds to native pixel index 3. At `xscale = 2`, native pixel 3 covers
canvas x = 6 and x = 7.

So the prediction is: canvas pixels (6, TOP) and (7, TOP) are foreground,
canvas pixel (0, TOP) is background, and because this mode has no attribute
bytes, foreground means palette register 1 and background means register 0
— the fixed `NO_ATTR_FG`/`NO_ATTR_BG` pair from §8.2. The test asserts
exactly that, writing the coordinates as `3 * 2` and `3 * 2 + 1` so the
derivation stays visible in the source.

There is a fourth assertion, on `px(&fb, 0, 0)`. Canvas row 0 is well above
`TOP = 25`, the first active row for `LPF = 0`'s 192-line body, so it is
border. The test expects `GIME::rgb_color(0)`, because `gime_with` leaves
`g.border` at its default of 0. That happens to equal palette register 0's
value under this test's identity palette, which is a coincidence of the
fixture and not a hardware relationship: the border is the independent
`$FF9A` value from §8.1, and the next test proves it by setting it to
something else.

### `graphics_unpacks_4bpp_msb_first_from_physical_base`

```rust
    let g = gime_with(GFX, VRES_320X16);
    let mut ram = vec![0u8; RAM_LEN];
    ram[BASE] = 0x5C; // pixels 5, 12
    ram[BASE + 160] = 0x70; // row 1 first pixel = 7
```

`VRES_320X16` is `0x1E`, which §8.3's table identifies as `HSCREEN 2`: 160
bytes per row, 4 bits per pixel, 320 pixels wide, 16 colors. Wide, so
`active_w = 640` and `xscale = 2`.

`0x5C` is `0b0101_1100`. Per §8.3's MSB-first walk at 4 bpp, pixel 0 is the
high nibble, `0101` = 5, and pixel 1 is the low nibble, `1100` = 12. At
`xscale = 2`, native pixel 0 covers canvas x 0–1 and native pixel 1 covers
canvas x 2–3. Predict `px(&fb, 0, TOP)` and `px(&fb, 1, TOP)` both equal
`GIME::rgb_color(5)`, and `px(&fb, 2, TOP)` equals `GIME::rgb_color(12)`.
All three hold.

The fourth assertion is the interesting one, because it tests something the
first three cannot. The test plants a second byte at `ram[BASE + 160]` and
expects its high nibble, 7, to appear at canvas row `TOP + 1`. That
assertion is really two claims at once: that the row pitch is exactly 160
bytes, and that a graphics mode advances to a new data row on *every*
scanline. The second follows from `LPR_LINES[0] = 1` — §8.3's worked
example established that graphics modes set `LPR` to `%000` — and the first
follows from `GFX_BYTES_PER_ROW[7] = 160`. The byte offset the test chose
is the proof; had the pitch been anything else, the byte would have landed
somewhere other than the second visible row.

### `horizontal_offset_shifts_fetch_with_seam_wrap`

```rust
    let mut g = gime_with(GFX, VRES_320X16);
    g.horizontal_offset = 0x80 | 0x01;
    let mut ram = vec![0u8; RAM_LEN];
    ram[BASE + 2] = 0xF0; // lands at pixel 0 with the 2-byte shift
    ram[BASE + 256 + 2] = 0x90; // row 1: HVEN pitch 256, same 2-byte shift
```

This is §8.1's `HVEN` mechanism exercised directly. The register value
`0x80 | 0x01` sets `HVEN` and gives an X offset field of 1, and per the
`hoff` module's doc comment the offset is doubled, so `x_offset = 2` bytes.

With `HVEN` on, `fetch(i)` reads `ram[row_base + (x_offset + i) % 256]`, so
the byte at `row_base + 2` is what `i = 0` returns and therefore what lands
at pixel 0. `0xF0`'s high nibble is 15, so predict
`px(&fb, 0, TOP) == GIME::rgb_color(15)`. Confirmed.

The row pitch under `HVEN` is fixed at 256 bytes regardless of the mode's
actual `bytes_per_row`, which for this shape would otherwise be 160. So
row 1's data lives at `row_base + 256`, and the test plants its byte at
`+256 +2` to account for the same 2-byte X shift, predicting
`px(&fb, 0, TOP + 1) == GIME::rgb_color(9)` from `0x90`'s high nibble. Also
confirmed.

One honest limitation is worth naming, because a reader who takes the test
name at face value will look for something that is not there. Nothing in
this test demonstrates the seam *wrap* actually firing. Triggering it would
need an X offset large enough to walk past byte 255 within a row, and this
test does not push that far. What it does confirm are the two load-bearing
facts §8.1 claimed: the offset is measured in 2-byte units, and `HVEN`
changes the row-to-row pitch independently of the mode's own byte width.
Naming what a test does *not* cover is as useful as naming what it does,
and the gap here is a legitimate place to add coverage.

---

## 8.8 `gime_modes.rs`: the real ROM, honestly

[`crates/coco-core/tests/gime_modes.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/gime_modes.rs)
is the test this chapter's syllabus entry calls out by name. It boots the
actual `coco3.rom`, runs it to the BASIC prompt, then pokes the exact
register sequences from §8.1 and §8.3's worked examples — the `WIDTH 80`
and `HSCREEN 2` images, verbatim — at the live machine, and checks that the
framebuffer dimensions stay locked to the canonical 640×240 canvas across
the switch. It is an integration test in the fullest sense this course has
available: real ROM bytes, a real boot, a real register-write sequence that
a real BASIC program would issue.

It pays to be precise about what that test can do in *this* checkout.
The `roms/` directory is one of two directories this repository
deliberately keeps out of version control, the other being `docs/` and its
reference PDFs, because their contents are copyrighted — Tandy's ROM
images, not this project's code. Running the test here:

```
$ cargo test -p coco-core --test gime_modes
```

fails immediately, and not on an assertion. It fails on the fixture load:

```
cannot read .../crates/coco-core/../../roms/coco3.rom: No such file or directory (os error 2)
```

That is not a bug to chase. It is the expected result of working in a
checkout — or a git worktree split off from one — that never had `roms/`
populated. On a machine where a CoCo 3 ROM image has been legitimately
obtained and placed at `roms/coco3.rom`, at the repository root as a
sibling of `crates/`, the same command boots the real 32K Super Extended
Color BASIC, runs 120 fields to reach the idle prompt, and then executes
exactly the register writes decoded by hand in §8.1:

```rust
    // The ROM's WIDTH 80 register image: COCO off, BP=0 LPR=8, 80 cols with
    // attributes, video base $6C000 ($FF9D:$FF9E = $D80:0 ×8).
    m.bus.write(0xFF90, 0x4C); // INIT0: COCO=0, MMU on, MC3/MC2
    m.bus.write(0xFF98, 0x03);
    m.bus.write(0xFF99, 0x15);
    m.bus.write(0xFF9D, 0xD8);
    m.bus.write(0xFF9E, 0x00);
    m.run_field();
```

([`gime_modes.rs:38-45`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/gime_modes.rs#L38-L45).)
Five register writes and one field. A nice detail sits a few lines
further down, too: after switching back to CoCo-compatible mode the test
runs *two* fields before asserting, with the comment "the COCO flip latches
at the NEXT field start." That is §8.5's `legacy` latch, observed from the
outside by a test that would fail if the latch were not there.

If the ROM is available, run it. There is real pedagogical value in
watching the exact bytes decoded by hand actually flip a live, booted
machine's video mode. If it is not available, the substantive work has
still been done, and it is worth being clear about why. Everything this
test *checks* — that the framebuffer stays 640×240 across the mode switch,
because "one stable texture size across every CoCo 3 mode is the point of
Option B," per the test's own comment — is a narrower claim than what
§8.1's worked example already established by hand. The hand decode said
which columns, which colors, and which physical address those bytes
produce. The test says the canvas did not change size. The first is the
harder result.

[`tests/gime_irq.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/gime_irq.rs)'s
ROM-dependent blink test from §8.2 sits in the same position: real,
valuable, and unavailable in a `roms/`-less tree. Do not let a missing
fixture read as a missing feature, in this codebase or any other.

---

## 8.9 Reading assignment

In this order. Each file makes the next more legible, and the sequence
deliberately goes constants, then renderer, then color, then font, then
tests — narrowing from "what the registers say" to "what the pixels do."

1. **[`crates/coco-core/src/gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs), lines 42–228.** The register constant
   modules (`init0`, `init1`, `vmode`, `vres`, `hoff`, `intr`), the lookup
   tables (`LPF_LINES`, `LPR_LINES`, `TEXT_COLS`, `GFX_BYTES_PER_ROW`,
   `GFX_BPP`), and the `GIME` struct's video-relevant fields. Everything in
   §8.1 lives here. Read the doc comments as carefully as the code — three
   of the five tables document a hardware misbehaviour that the constant
   alone does not reveal.
2. **[`crates/coco-core/src/gime_video.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs), in full (477 lines).** `decode_text`
   and `decode_graphics` first, then `FieldScan` and `resolve_colors`, then
   `paint_scanline` and its two callees. This is the chapter's centre of
   gravity; read the module doc comment at the top again once you've read
   the rest — it will make more sense the second time.
3. **[`crates/coco-core/src/gime/palette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs), in full.** Read `rgb_color`
   against §8.4's worked example with a calculator in hand; don't just
   trust the chapter's arithmetic. Skim the two composite tables without
   trying to make sense of them — Chapter 9 does that.
4. **[`crates/coco-core/src/font_gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font_gime.rs), lines 1–17 and skim the table.**
   You don't need to memorize glyph bitmaps — notice the shape (128
   entries, 8 rows each) and the `$00`–`$1F` special-character note. Look
   up `GIME_FONT[0x41]` and confirm that its eight bytes really do draw a
   capital A when read as a bitmap.
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
width, colors available, whether the display is wide or has side borders,
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
have different pixel widths and color counts? Express the total both in
decimal bytes and as a count of 8K MMU blocks (Chapter 5) it would take to
map the whole screen into the CPU's logical space at once.

**8.3 — Sabotage `paint_text_row`'s foreground base, and watch it fail
(build + verify).** In [`crates/coco-core/src/gime_video.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs), change:

```rust
const ATTR_FG_BASE: usize = 8;
```

to `0`. Predict, before running anything, which `render_gime.rs` tests
will fail and why (hint: think about which tests read the foreground
color of an *attributed* character versus tests that never touch
attributes at all — non-attribute text and every graphics test read no
`ATTR_FG_BASE`-derived index and should be unaffected). Then actually run:

```
cargo test -p coco-core --test render_gime
```

and compare the failure list to your prediction. (For reference — don't
peek until you've made your own prediction — sabotaging this constant
this way produces exactly 5 failures out of 14 tests, all of them the ones
that assert on an *attributed* foreground color: blink, underline, the
second-row pitch check, the vertical-scroll check, and the direct
`text_attributes_select_fg_bg_palettes` test. Everything else — the
no-attribute test, every graphics test, the border tests — stays green,
because none of them ever read a foreground color through
`ATTR_FG_BASE`.) Revert the constant, re-run the suite to confirm all 14
pass again, and check `git status` shows no changes to the file before you
move on — leaving a sabotaged constant in the tree is the one way to turn
this exercise into tomorrow's very confusing bug report.

**8.4 — Extend `gime_demo.rs`: a third PPM (build).** Add a third block to
`gime_demo.rs`'s `main` that renders an `HSCREEN 1` (320×192, 4-color)
screen — you worked out its exact `$FF99` value in §8.3's HSCREEN table —
filled with a simple pattern of your choosing (vertical stripes, a
gradient, anything that isn't solid one color). Write it to a third PPM
and confirm by eye that it's genuinely 4-color, 320-pixel-wide content
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

Next week keeps `gime_video.rs` open but changes the question. Not "what
does one scanline look like," but "what happens when the registers this
chapter taught you to read *change* while the beam is still partway down
the screen."

Every piece of machinery Chapter 9 needs is already in hand: `FieldScan`'s
latched-versus-live split from §8.5, and the fact that `paint_scanline`
re-reads mode, palette, and border fresh on every line. What is missing is
a real 6809 program using that machinery on purpose.
[`tests/scanline_split.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/scanline_split.rs)
has the receipts: a timer-driven FIRQ handler splitting the border color
mid-field, verified against a real interrupt firing from real ROM code.

Two other threads this chapter deliberately left hanging get picked up
there as well. Composite output turns out not to be a simplified RGB but a
genuinely different, hand-measured palette table reached through the same
six bits — the `BPI` and `MOCH` bits from `$FF98` finally do something. And
the CoCo 1 and 2's own graphics modes, the PMODEs that GIME-native scanout
never touches at all, get the treatment Chapter 7's text-only tour did not
have room for.
