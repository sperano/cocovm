# Chapter 7 — How a Raster Works, and the Legacy VDG Text Mode

*Week 7. Goal: go from "a TV scans lines" to a fully decoded green BASIC
prompt, with zero GPU involved anywhere. Weeks 2–4 gave you a CPU; week 5
gave you a bus; week 6 gave you a clock that calls `render_scanline()` once
per line. This week that function stops being an empty promise and starts
actually painting pixels — and the first thing it paints turns out not to be
what you assumed for forty years. You told us to "start with the GIME."
Here is why that instruction led here, to week 7, and not to week 1: nothing
the GIME draws is visible until a CPU executes ROM code over a bus on a
clock. Now all three previous parts of the course pay off at once, on the
exact screen you stared at as a kid.*

---

## 7.1 Why this is where video finally starts

Quick recap of the machinery already built, because this chapter plugs
directly into it. Week 6's `end_of_line()` ([`crates/coco-core/src/machine/run.rs:130`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs#L130))
runs once per scanline, in cycle-accurate lockstep with the CPU:

```rust
pub(super) fn end_of_line(&mut self) -> bool {
    let lines = self.config.video.lines_per_field();
    // ...hsync, field-sync edges...
    self.render_scanline();
    // ...audio, GIME timer...
    self.line += 1;
    if self.line >= lines {
        self.line = 0;
        self.render_field();
        true
    } else {
        false
    }
}
```

`render_scanline()` ([`crates/coco-core/src/machine/render.rs:29`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L29)) was already
called every line in week 6 — you just didn't look inside it, because inside
it was, functionally, nothing yet: geometry bookkeeping with no pixels. This
week that changes. By the end of this chapter, calling it 192 times (once
per visible line of a 32×16 text screen) will produce the exact frame a real
CoCo 3 puts on a TV at cold boot.

Two things this chapter is *not*: it is not the GIME's native 40/80-column
text or bitmapped graphics registers (`$FF98`–`$FF9F`) — those are week 8.
It is not composite artifact colour, mid-frame splits, or PMODE bitmap
graphics — those are week 9. This week is deliberately narrow: one video
mode, the one every CoCo 3 owner saw first and most, decoded down to
individual bits and pixels.

---

## 7.2 Raster fundamentals, from zero

You've never done graphics programming, so this section owes you the whole
picture before any Rust. Skip nothing here — every later section leans on
this arithmetic.

### 7.2.1 What a CRT actually does

A CRT television doesn't have pixels the way an LCD does. It has an electron
beam and a magnet that steers it. The steering follows one fixed, boring
path, over and over, sixty times a second on NTSC:

```
  ┌────────────────────────────────────────────┐  ← top of screen
  │ ────────────────────────────────────────►  │  scanline 0 (left→right)
  │ ◄┘                                          │  retrace (beam off, snaps back)
  │ ────────────────────────────────────────►  │  scanline 1
  │ ◄┘                                          │
  │                    ⋮                        │
  │ ────────────────────────────────────────►  │  scanline 261
  └────────────────────────────────────────────┘  ← bottom, beam snaps back to top
```

The beam sweeps left to right, brightness modulated by the incoming video
signal, painting one horizontal strip — a **scanline**. At the right edge
the beam is blanked (turned off) and steered back to the left edge: that's
**horizontal sync** (hsync), a pulse in the signal telling the TV's
electronics "start a new line now." After the last scanline the beam is
blanked and steered all the way back to the top-left: **vertical sync**
(vsync), meaning "start a new field now." One complete top-to-bottom sweep
is a **field**; NTSC does 262 scanlines per field, roughly 59.94 of them
every second ([`crates/coco-core/src/config.rs:52`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/config.rs#L52), [`:60`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/config.rs#L60) — you derived this
exact number in week 6).

Not every one of those 262 lines carries a picture. The beam needs time to
travel back up to the top (**vertical blanking**) and, more subtly, real
consumer TVs never showed the full 262-line sweep as picture — a chunk at
the top and bottom was always cropped by the set's own bezel and
overscan. The chip designers accounted for this by drawing a smaller
**active area** in the middle of the field and painting a solid **border**
colour around it — border on the sides (during visible lines, before/after
the active pixels) and border-colored blank lines above/below the active
body. That border is not a rendering nicety; it's a required part of every
real video signal, and it's exactly what you remember as the colored strip
around the CoCo's picture.

So: a field is 262 potential scanlines; not all are visible; of the visible
ones, only a rectangle in the middle is "the picture," everything else is
border or blanking. Every video chip this course will meet — the MC6847,
the SAM, the GIME — is a machine that decides, scanline by scanline, "is
this a border line or a picture line, and if picture, what colour is each
pixel across it."

### 7.2.2 The framebuffer: a screen is just bytes

Real hardware doesn't have a framebuffer — the beam paints directly from a
running decode of RAM, live, as it sweeps. An emulator can't do that (there
is no beam), so it fakes the whole field's output into an ordinary array in
memory and hands that array to the host to display once the field is done
(or, as you'll see in §7.3, one scanline at a time as it's computed — same
idea, finer granularity).

The array is a flat `Vec<u8>`, and the convention this codebase uses —
practically universal in graphics work — is **RGBA**: four bytes per pixel,
red, green, blue, alpha (opacity; always `0xFF`, fully opaque, since this
emulator never needs transparency). `BYTES_PER_PIXEL` is `4`
([`crates/coco-core/src/video.rs:40`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video.rs#L40)). A single white pixel is the four bytes
`[0xFF, 0xFF, 0xFF, 0xFF]`; black is `[0x00, 0x00, 0x00, 0xFF]`; the pure
green you'll see so much of this chapter is `[0x00, 0xFF, 0x00, 0xFF]`.

Pixels are stored **row-major**: all of row 0's pixels, left to right, then
all of row 1's, and so on — which is just "the same order the CRT beam
paints in," §7.2.1's scanline sweep translated directly into array layout.
For a framebuffer of width `W` pixels, the byte offset of pixel `(x, y)`'s
red channel is:

```
offset = (y * W + x) * BYTES_PER_PIXEL
```

Walk it once by hand so the formula stops being abstract. Take a small
16×4 framebuffer (`W = 16`) and find pixel `(3, 2)` — column 3, row 2:

```
offset = (2 * 16 + 3) * 4
       = (32 + 3) * 4
       = 35 * 4
       = 140
```

Byte 140 is that pixel's red channel; 141 green, 142 blue, 143 alpha. Every
single pixel-writing function you'll read this chapter — `paint_px`,
`blit_cell`, `blit_semigraphics4` — is this one multiplication, sometimes
with a stride added for a border offset. You now know 100% of the
arithmetic of 2D graphics as this codebase practices it. There is no more
"graphics math" to learn; everything else is bookkeeping about *which*
color to write at *which* offset.

Here it is verbatim, from the actual renderer ([`crates/coco-core/src/video/text.rs:260`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L260)):

```rust
fn blit_cell(fb: &mut [u8], row: usize, col: usize, glyph: &[u8; CELL_H], fg: [u8; 4], bg: [u8; 4]) {
    let x0 = BORDER + col * CELL_W;
    let y0 = BORDER + row * CELL_H;
    for (cy, &bits) in glyph.iter().enumerate() {
        for cx in 0..CELL_W {
            let on = bits & (0x80 >> cx) != 0;
            let color = if on { fg } else { bg };
            let idx = ((y0 + cy) * FB_W + (x0 + cx)) * BYTES_PER_PIXEL;
            fb[idx..idx + BYTES_PER_PIXEL].copy_from_slice(&color);
        }
    }
}
```

`(y0 + cy) * FB_W + (x0 + cx)` is exactly `y * W + x` from the formula
above, with `x0`/`y0` shifting into the right character cell first. Once
you see this, every renderer in the video subsystem — this week's text
mode, next week's GIME graphics, week 9's PMODE — is legible: they are all
this same loop with a different rule for *which* color to compute.

### 7.2.3 Connecting the two: one call per scanline

Week 6 built the clock; this chapter fills in what it drives. The call
chain, concretely:

```
run_field()                        (machine/run.rs)
  └─ per line: end_of_line()        — hsync, then:
       └─ render_scanline()         (machine/render.rs) ← THIS CHAPTER
            └─ paint_legacy_scanline(row)   for legacy VDG text (this week)
                 └─ paint_legacy_text_line(...)  → blit one row of glyphs
                      └─ paint_px(...)      → the (y*W+x)*4 arithmetic
```

`render_scanline` is called 262 times per field (once per line, whether or
not that line is visible). This week's job is one branch deep inside it:
`paint_legacy_scanline`, which decides, for the current line, "is this
border or active, and if active, what glyph row goes where." That's the
whole chapter, stated as one sentence — everything from here is filling in
the "what glyph row" part correctly.

---

## 7.3 The canonical 640×240 canvas

A modern display has no fixed relationship to a CoCo's video timing, so
*something* has to decide the pixel dimensions the emulator core hands to
the frontend. The codebase's answer, for the CoCo 3, is one fixed-size
canvas that every video mode — this week's legacy text, next week's GIME
native text/graphics, week 9's advanced modes — renders into. The whole
module is 40 lines; read it in full
([`crates/coco-core/src/raster.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/raster.rs)):

```rust
//! Canonical 640×240 raster geometry (Option B, `docs/plan-per-scanline-video.md`).
//!
//! One fixed-size RGBA canvas for every GIME-native mode, matching MAME's
//! coco3 visible window (`coco3.cpp` `set_raw(..., 912, 0, 640, 262, 1, 240)`).
//! Every legal mode reaches it by an INTEGER horizontal scale: active content
//! is 512 px (non-wide, 64 px border each side) or 640 px (wide, no border) —
//! MAME `gime.cpp` `render_scanline` (`wide = !legacy && ($FF99 & 0x04)`; ...

/// Canonical canvas width: MAME's coco3 visible width.
pub const CANVAS_W: usize = 640;
/// Canonical canvas height: MAME's coco3 visible lines.
pub const CANVAS_H: usize = 240;

/// Active-content width of non-wide modes; the rest of the 640 is border.
pub const NON_WIDE_ACTIVE_W: usize = 512;
/// Horizontal border width each side of a non-wide mode's 512 px body.
pub const NON_WIDE_BORDER_X: usize = (CANVAS_W - NON_WIDE_ACTIVE_W) / 2;

pub const fn vertical_window(lpf: usize) -> (usize, usize) {
    match lpf {
        0 => (25, 192),
        1 => (23, 200),
        2 => ((CANVAS_H - 210) / 2, 210),
        _ => (8, 225),
    }
}
```

Read this slowly, because the design decision it embodies is worth more
than the 40 lines suggest.

**Why one fixed size, ever?** The CoCo 3 can be in dozens of legal video
configurations — 32-column legacy text, 40/80-column GIME text, four bit
depths of HSCREEN graphics at four widths, each with its own native pixel
count. A naive design renders each mode into its own natively-sized buffer
and makes the *frontend* juggle N different aspect ratios and aim for the
right one. Instead, `raster.rs` fixes one target — 640×240, chosen because
it's MAME's own CoCo 3 visible window, so the two can be screenshot-diffed
directly — and every mode is responsible for **integer-scaling itself up**
to fill it. A 256-pixel-wide legacy text row (this week) scales ×2 into 512
active pixels; an 80-column GIME text row (week 8) might not scale at all.
The frontend (week 15) then does exactly one thing forever: take a 640×240
RGBA buffer, upload it as a texture, letterbox it at 4:3. No mode-specific
frontend code, ever. That's the "headless-testable payoff" the syllabus
promised: a test can allocate a 640×240 `Vec<u8>`, call the renderer, and
assert on pixel colours with no window, no GPU, no scaling logic to get
wrong twice.

**Wide vs. non-wide.** `NON_WIDE_ACTIVE_W = 512` — most modes, including
this week's legacy text, draw a 512-pixel-wide active body with a 64-pixel
border on each side (`NON_WIDE_BORDER_X = (640 − 512) / 2 = 64`). A small
number of GIME modes (week 8's widest HSCREEN resolutions) use the full 640
with no side border at all — "wide" mode. You'll meet the register bit that
selects it next week; for now, know that legacy VDG text is *always*
non-wide, which is why `paint_legacy_scanline` (below) unconditionally uses
`NON_WIDE_BORDER_X`/`NON_WIDE_ACTIVE_W` and never checks a wide flag.

> **Rust corner: `chunks_exact_mut` for a run of identical pixels.** The
> integer scale this section keeps mentioning — 256 native pixels stretched
> to fill 512 canvas pixels — is implemented by one small function, `paint_px`
> ([`crates/coco-core/src/video.rs:148`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video.rs#L148)):
> ```rust
> fn paint_px(out: &mut [u8], x: &mut usize, xscale: usize, color: [u8; 4]) {
>     for px in out[*x * BYTES_PER_PIXEL..][..xscale * BYTES_PER_PIXEL].chunks_exact_mut(BYTES_PER_PIXEL)
>     {
>         px.copy_from_slice(&color);
>     }
>     *x += xscale;
> }
> ```
> `out[*x * BYTES_PER_PIXEL..][..xscale * BYTES_PER_PIXEL]` is two range
> indexes chained: the first slices from the cursor to the end of the
> buffer, the second takes the first `xscale * BYTES_PER_PIXEL` bytes of
> *that* — "start here, then take this many," which reads more directly
> than folding both bounds into one expression. `.chunks_exact_mut(4)` then
> walks that slice four bytes at a time, handing back one `&mut [u8]` per
> pixel for `copy_from_slice` to stamp the same colour into. `chunks_exact`
> (rather than plain `chunks`) guarantees every chunk is a full
> `BYTES_PER_PIXEL` long and silently drops any short trailing remainder —
> the right call here specifically *because* the slice length is
> `xscale * BYTES_PER_PIXEL` by construction, always an exact multiple, so a
> short last chunk could only mean a bug upstream, never a case worth
> handling gracefully. This one function is "draw a native pixel `xscale`
> times" for every legacy scanline in the codebase — nothing more clever is
> needed.

**The vertical window.** `vertical_window(lpf)` answers "given the $FF99 LPF
(lines-per-field) register's 2-bit value, how many border rows sit above
the active body, and how tall is the body?" LPF=0 gives 25 border rows then
a 192-row body (25 + 192 + 23 = 240 — the remaining 23 rows are the
*bottom* border, computed by whoever calls this, not stored here). This is
a live hardware register (you'll manipulate it directly starting week 8);
this week you only need to know it exists and that even the legacy VDG path
reads it — real CoCo 3 hardware runs the GIME's vertical geometry
unconditionally, whether or not you're in VDG-compatible mode. `LPF=2` is
the interesting row: the doc comment calls out that real silicon glitches
at this value (a "zero/infinite" line count that depends on exactly when in
the raster you wrote the register), and this emulator picks a defined,
centered approximation (210 rows) rather than modeling the glitch — an
explicit, documented fidelity trade-off, the same kind you'll see justified
throughout DESIGN.md.

**A caveat worth stating plainly, since this codebase supports real CoCo
1/2 hardware too:** `raster.rs`'s canonical canvas is a *CoCo 3* artifact —
it exists because the GIME defines one visible window shared by every mode
it can produce. A CoCo 1 or 2 has no GIME and no such unification; those
machines render into their own fixed, smaller buffer
(`video::FB_W`/`FB_H`, 288×224 — you'll see it in §7.4) via a whole-field
snapshot rather than the per-scanline canonical path. That split is set up
in [`crates/coco-core/src/machine.rs:17-18`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine.rs#L17-L18) (`FB_WIDTH`/`FB_HEIGHT` default
to the VDG geometry) and in `machine/render.rs`'s two entry points,
`render_scanline` (CoCo 3 only, early-returns otherwise) and `render_field`
(CoCo 1/2 only, early-returns for CoCo 3). This chapter's worked examples
are CoCo 3, since that's this course's machine, but keep the split in mind
when you read a test and see two different framebuffer sizes in play.

---

## 7.4 The reveal: your BASIC prompt was never GIME-native

Here is the correction `DESIGN.md` §6 records, verified against a real ROM
boot and a MAME screenshot, and it is worth sitting with for a moment if
you owned a CoCo 3: **the green `OK` prompt you stared at for years was
drawn by the GIME pretending to be the old MC6847 VDG chip from the CoCo
1/2 — not by any of the GIME's own native video modes.** A real CoCo 3 has
no MC6847 chip soldered to the board at all; on power-up, the ROM
configures the GIME to imitate one, in a 32-column-by-16-row text mode that
is bit-for-bit compatible with what CoCo 1/2 software expects at `$0400`.
Native 40/80-column GIME text only appears once you type `WIDTH 40` or
`WIDTH 80` — something most BASIC programmers rarely did outside a handful
of applications. So the "first visible pixel" milestone in this codebase's
own history required building this legacy-compatibility path *before* any
native GIME video register mattered, and the same ordering makes sense
pedagogically: this is week 7, GIME-native is week 8.

### 7.4.1 How the machine decides which path it's on

`Machine::video_mode()` ([`crates/coco-core/src/machine/video_mode.rs:19`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/video_mode.rs#L19)) is
the dispatcher — read it in full, it's short and it's the map of the entire
video subsystem for weeks 7–9:

```rust
fn video_mode(&self) -> VideoMode {
    match self.config.variant {
        MachineVariant::Coco1 | MachineVariant::Coco2 => {
            if self.bus.pia1.b.output & video::VDG_AG != 0 {
                VideoMode::CocoGraphics
            } else {
                VideoMode::CocoText
            }
        }
        MachineVariant::Coco3 => {
            let g = &self.bus.gime;
            if g.init0 & gime::init0::COCO != 0 {
                // PIA1 $FF22 bit 7 selects VDG graphics (PMODE) vs alphanumerics/semigraphics.
                if self.bus.pia1.b.output & video::VDG_AG != 0 {
                    VideoMode::CocoGraphics
                } else {
                    VideoMode::CocoText
                }
            } else if g.vmode & gime::vmode::BP != 0 {
                VideoMode::GIMEGraphics
            } else {
                VideoMode::GIMEText
            }
        }
    }
}
```

A CoCo 1/2 has no GIME at all, so it *always* runs this week's path —
picking text vs. graphics off a single PIA1 bit (`VDG_AG`, `$FF22` bit 7).
A CoCo 3 checks one more gate first: `INIT0`'s `COCO` bit (`$FF90` bit 3).
If it's set, the machine ignores every native GIME video register and falls
into the *exact same* `VDG_AG` branch the CoCo 1/2 uses — this is "GIME
pretending to be a VDG." Only when `COCO` is clear does the GIME's own
`$FF98` `BP` (bits-per-pixel-vs-text) bit get consulted at all. Stock CoCo 3
BASIC sets `COCO` at cold start and never clears it unless you ask for a
wider screen — which is precisely the fact this section opened with.

### 7.4.2 The screen byte, bit by bit

The legacy text screen is 512 bytes — `COLS * ROWS = 32 * 16`
([`crates/coco-core/src/video.rs:29-43`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video.rs#L29-L43)) — living in RAM at a base address
the SAM (or, on CoCo 3, the GIME's SAM-compatibility page register) points
at (default `$0400`, the address every CoCo BASIC programmer memorized).
Each byte is a **character cell** that packs three independent things,
decoded by three named bit masks
([`crates/coco-core/src/video/text.rs:16-18`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L16-L18)):

```rust
const SEMIGRAPHICS_BIT: u8 = 0x80; // bit 7 — 1 = semigraphics 4, 0 = alphanumeric
const INVERSE_BIT: u8 = 0x40;      // bit 6 — inverse video (alphanumeric only)
const GLYPH_CODE_MASK: u8 = 0x3F;  // bits 5-0 — alphanumeric glyph code
```

```
   bit:  7   6   5 4 3 2 1 0
        ┌───┬───┬─────────────┐
        │ S │ I │  glyph code │
        └───┴───┴─────────────┘
          │   │
          │   └─ inverse video (alpha mode only)
          └───── 0 = alphanumeric   1 = semigraphics-4
```

If bit 7 is 0, this is a plain text cell: bits 5–0 select one of 64 glyphs
(`decode_alpha_char`, [`crates/coco-core/src/video/text.rs:37`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L37), maps code
`$00-$1F` to `'@'..'_'` and `$20-$3F` to a second copy of the ASCII block
from space upward), and bit 6 flips foreground and background for that one
cell. If bit 7 is 1, none of that applies — the byte switches to an
entirely different interpretation, semigraphics-4, covered in §7.6.

> **Rust corner: `usize` on the framebuffer side, `u16` on the bus side.**
> `paint_legacy_scanline` ([`crates/coco-core/src/machine/render.rs:75`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L75))
> straddles two address spaces with two different integer types, and the
> boundary between them is exactly where the cast lives:
> ```rust
> let mut buf = [0u8; video::COLS];
> for (i, byte) in buf.iter_mut().take(row_bytes).enumerate() {
>     *byte = self.bus.read(base.wrapping_add(i as u16));
> }
> // ...
> let active = &mut self.framebuffer[(row * raster::CANVAS_W
>     + raster::NON_WIDE_BORDER_X)
>     * BYTES_PER_PIXEL..][..raster::NON_WIDE_ACTIVE_W * BYTES_PER_PIXEL];
> ```
> `base` (the screen row's start address) and the loop's `i as u16` are
> `u16` because that's a real constraint, not a style choice: the 6809's
> address bus is sixteen bits wide, full stop, and `Bus::read` (week 1)
> takes a `u16` for exactly that reason — `wrapping_add` on `i as u16` is
> the same "must wrap at `$FFFF` → `$0000`" discipline you saw in week 1's
> `Bus::read_u16`. `row`, `CANVAS_W`, `BYTES_PER_PIXEL`, and the framebuffer
> index built from them are `usize` because *that's* a real constraint too:
> `Vec<u8>`/slice indexing in Rust is defined in terms of `usize` — it's the
> type that's guaranteed large enough to index anything the host's address
> space can hold, which a fixed 16-bit type is not required to be (and on
> some hosts isn't). The two integer types aren't interchangeable
> stylistic choices; each one is the correct width for the address space it
> names, and the explicit `as u16` cast on the loop counter is the one spot
> in this function where a `usize` loop index (`i`, from `.enumerate()`)
> crosses into 6809 address space and must be narrowed, deliberately, to
> match. Losing that cast — or writing `i as u16` where a framebuffer index
> was needed instead — is the kind of silent-truncation bug `clippy`'s
> `cast_possible_truncation` lint exists to catch; watch for the direction
> of every `as` cast at a boundary like this one.

### 7.4.3 Decoding a real screen byte, end to end

Take the byte `$41`. Split it into bits: `0100 0001`. Bit 7 is 0, so this is
alphanumeric. Bit 6 is 1 — inverse. Bits 5–0 are `000001` = 1. Glyph code 1,
via `decode_alpha_char`, is `'@' + 1` = `'A'`. So `$41` is: *the letter 'A',
inverse video.*

Notice something satisfying here: `$41` is *also* the ASCII code for
capital `'A'`. That's not a coincidence dressed up as one — it falls
straight out of the arithmetic. For any code `c < $20`, the non-inverse
decoded character is `'@' + c` = `$40 + c`; the *inverse* raw byte for that
same code is `c | $40` = (since `c`'s top two bits are always clear)
`$40 + c` — the identical value. For codes under `$20`, "ASCII value of the
decoded character" and "raw screen byte with the inverse bit forced on"
are literally the same number. It's a cute artifact of the VDG's code
space overlapping ASCII's uppercase block, not a rule you need to memorize
— but it's why `$41` reads naturally as both "screen byte" and "the letter
A" at once.

Now the part that matters for the "black-on-green" reveal. The stock
Color BASIC ROM fills the entire 512-byte text screen with the *inverse*
form of every character it prints — bit 6 set on every single screen byte,
always, not just for a blinking cursor or a highlighted word. Feed that
fact into `resolve_alpha_cell`
([`crates/coco-core/src/video/text.rs:88-116`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L88-L116)), the function every text
pixel in this codebase routes through:

```rust
fn resolve_alpha_cell(
    generator: AlphaGenerator,
    ff22: u8,
    code: u8,
    fg: [u8; 4],
    bg: [u8; 4],
) -> ([u8; 4], [u8; 4], &'static [u8; CELL_H]) {
    let glyph_code = code & GLYPH_CODE_MASK;
    let inverse = code & INVERSE_BIT != 0;
    // ...true-lowercase branch, §7.5...
    let (cell_fg, cell_bg) = if inverse { (bg, fg) } else { (fg, bg) };
    (cell_fg, cell_bg, glyph)
}
```

When `inverse` is true, the *background* colour paints where the glyph's
"on" bits are, and the *foreground* colour paints everywhere else. So every
character on the stock boot screen draws its strokes in the **background**
palette colour and everything else in the **foreground** palette colour —
backwards from what "foreground/background" suggests in every other
context you've used those words. Combine that with where those two colours
actually come from (§7.7: green foreground, black background by default)
and you get exactly what you remember: **black letters, on a field of solid
green** — with the green being what "background" resolves to and the black
letters being "foreground," inverted onto the page by that one bit, on
every cell, all the time. This is the single strangest fact this chapter
teaches, and it is exactly correct: go boot the real ROM (or read
[`tests/coco1_boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/coco1_boot.rs) from week 6) and check for yourself that every
non-blank screen byte has bit 6 set.

---

## 7.5 Fonts as data

Nothing about how these glyphs are drawn is special-cased in the renderer —
`blit_cell` (§7.2.2) doesn't know or care what letter it's drawing. All the
"font" knowledge lives in plain `const` arrays. This section is about
reading those arrays like a human would, because you will need to eyeball
one at some point when a glyph looks wrong.

### 7.5.1 The MC6847's internal ROM, as a Rust array

`MC6847_FONT` ([`crates/coco-core/src/font6847.rs:37`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font6847.rs#L37)) has 64 entries — one
per glyph code — each a `[u8; 12]`: twelve bytes, one per raster row of the
8×12 character cell. Here is the entry for `'A'` (glyph code `$01`, the
second row of the table):

```rust
[0x00, 0x00, 0x00, 0x08, 0x14, 0x22, 0x22, 0x3E, 0x22, 0x22, 0x00, 0x00,], // A
```

Each byte's top 8 bits (of a `u8`, all 8 bits) correspond to the cell's 8
columns, most-significant bit first — the exact same `bits & (0x80 >> cx)`
test you already read in `blit_cell`. Decode all twelve rows by hand, `#`
for a set bit and `.` for clear, and the letter appears:

```
row 0:  0x00  ........
row 1:  0x00  ........
row 2:  0x00  ........
row 3:  0x08  ....#...
row 4:  0x14  ...#.#..
row 5:  0x22  ..#...#.
row 6:  0x22  ..#...#.
row 7:  0x3E  ..#####.
row 8:  0x22  ..#...#.
row 9:  0x22  ..#...#.
row 10: 0x00  ........
row 11: 0x00  ........
```

An 'A': apex at row 3, crossbar at row 7, legs down to row 9. Rows 0–2 and
10–11 are blank on *every* entry in `MC6847_FONT` — the doc comment
explains why: "the plain MC6847 [draws glyphs at] rows 3-10 (top 2 and
bottom 2 rows always blank — MAME `vdg_fontdata8x12`)." Six rows of actual
strokes, centered in a twelve-row cell, with generous blank padding above
and below for line spacing — that padding is why CoCo text never looks
cramped even though it's only a 32×16 grid.

> **Rust corner: a font is just a `const`, and that's the whole design.**
> `MC6847_FONT` is declared `pub const MC6847_FONT: [[u8; 12]; 64] = [...]`
> — a nested array literal, not a lazily-built value and not a loaded
> resource. Two more common-looking alternatives were available, and both
> are worse here. `include_bytes!("font.bin")` would embed the identical
> bytes but as an opaque `&'static [u8; 768]` with no structure — every
> glyph lookup would need hand-rolled index math (`bytes[code * 12 + row]`)
> instead of `font[code][row]`, reintroducing exactly the kind of
> off-by-one risk week 3's indexed-postbyte decoder went to such lengths to
> eliminate. `lazy_static!`/`once_cell` exist to defer initialization of
> values that genuinely *can't* be computed until runtime — a `HashMap`
> built from config, a value needing an allocator. Nothing here needs
> deferring: every glyph byte is known before the program even starts, so
> paying a first-access initialization check (and, for some such crates, an
> `unsafe` cell underneath) buys nothing. A `const` array of primitive data
> is baked straight into the binary's read-only section at compile time; it
> requires no init step, no allocation, and no synchronization, and
> `MC6847_FONT[code][row]` compiles to a plain indexed load — provably as
> cheap as indexing a local array, because to the generated code, that's
> exactly what it is.

### 7.5.2 Two more fonts, same shape, different rows

`MC6847T1_FONT` ([`font6847.rs:112`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font6847.rs#L112)) is 96 entries: the same 64
uppercase/symbol glyphs (index 0–63, same code space, same meaning), *plus*
32 more (index 64–95) that only exist on the newer T1 chip — true lowercase
letters, reached only through the special path in §7.5.3. Compare the T1's
`'A'` to the plain chip's:

```rust
// MC6847_FONT[1]  — rows 3-10
[0x00, 0x00, 0x00, 0x08, 0x14, 0x22, 0x22, 0x3E, 0x22, 0x22, 0x00, 0x00,]
// MC6847T1_FONT[1] — rows 1-8
[0x00, 0x08, 0x14, 0x22, 0x22, 0x3E, 0x22, 0x22, 0x00, 0x00, 0x00, 0x00,]
```

Same six strokes, shifted two rows higher in the cell. That's not
cosmetic — the T1 needed rows 8–11 free for lowercase **descenders** (the
tails on 'g', 'j', 'p', 'q', 'y' that dip below the baseline), so its
uppercase glyphs had to move up to make room. `GIME_LOWRES_FONT`
([`crates/coco-core/src/font_gime.rs:162`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font_gime.rs#L162)) — the *third* font, the one a
real CoCo 3 actually uses for this mode, since there's no VDG chip on the
board at all — follows the same T1-style layout (rows 1–8, lowercase with
descenders in 64–95):

```rust
[0x00, 0x10, 0x28, 0x44, 0x44, 0x7C, 0x44, 0x44, 0x00, 0x00, 0x00, 0x00], // A  (GIME_LOWRES_FONT[1])
```

Visibly a different glyph shape again (compare the stroke widths), but the
same 12-row/8-column/rows-1-8 packing convention. Three real font ROMs,
three slightly different letterforms, one shared array shape — which is
exactly why `AlphaGenerator` (§7.5.3) can treat all three uniformly.

### 7.5.3 The T1's sneaky true-lowercase rule

Neither the plain MC6847 nor the T1's *normal* mode has real lowercase —
codes `$00-$1F` always draw the corresponding uppercase letter, and setting
bit 6 just inverts its colours (§7.4.3). The T1 chip (and, sharing its
logic, the CoCo 3's GIME compat generator) has a second, genuinely
different mode: **true lowercase**, which redirects those same low codes to
a *different* set of glyphs entirely — actual lowercase letterforms with
descenders — instead of just inverting the uppercase ones. `AlphaGenerator`
([`crates/coco-core/src/video/text.rs:54-62`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L54-L62)) names the three chips this
codebase can emulate:

```rust
pub enum AlphaGenerator {
    /// CoCo 1/2 with the original MC6847.
    MC6847,
    /// CoCo 1/2 with the MC6847T1.
    MC6847T1,
    /// CoCo 3 CoCo-compatible text mode: the GIME's own generator, T1-style
    /// lowercase semantics, GIME_LOWRES_FONT glyphs.
    GIME,
}
```

and the gate that decides whether true lowercase kicks in, from
`resolve_alpha_cell` ([`text.rs:97-99`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L97-L99)):

```rust
let lowercase_capable = matches!(generator, AlphaGenerator::MC6847T1 | AlphaGenerator::GIME);
let true_lowercase =
    lowercase_capable && !inverse && ff22 & VDG_GM0_INTEXT != 0 && glyph_code < 0x20;
```

Four conditions, every one required:

1. **The chip must be capable at all** — a plain `MC6847` never takes this
   branch, full stop, because the `matches!` excludes it. Code `$01` on a
   CoCo 1 is *always* 'A', inverted or not.
2. **This cell's own inverse bit must be clear.** An inverse `$41` never
   goes lowercase, even on a T1 with the mode enabled globally.
3. **PIA1 `$FF22` bit 4 (`VDG_GM0_INTEXT`) must be set.** This is a
   *global*, whole-screen switch, not per-cell — the same physical pin the
   plain MC6847 uses for something else entirely (external character ROM
   select, unmodeled here; see the constant's doc comment for the
   dual-meaning history).
4. **The glyph code must be under `$20`.** Codes `$20-$3F` (space and
   punctuation) are untouched by lowercase mode on real hardware — there's
   nothing to make lowercase there anyway.

When all four hold, the cell draws from `font[0x40 + glyph_code]` instead
of `font[glyph_code % 64]` — literally a different 32-entry region of the
same array — and, notice the last line of the snippet in §7.4.3's excerpt,
the fg/bg colours are swapped exactly as if `inverse` were true, even
though condition 2 required it to be false. That's a real hardware quirk
this codebase reproduces faithfully rather than "fixing": true-lowercase
text is drawn colour-inverted relative to how you'd naively expect,
matching MAME's own `mc6847.cpp` bit for bit.

---

## 7.6 Semigraphics-4: the blocky graphics of every one-liner

Set bit 7 of a screen byte and the entire interpretation changes. This is
**semigraphics-4** (SG4): instead of a letter, the cell becomes a 2×2 grid
of coloured blocks, each independently on or off. Sixteen character cells
per row, sixteen rows, each holding a 2×2 sub-grid — do the arithmetic and
that's `32*2 = 64` by `16*2 = 32` independently-colored blocks: SG4 gives
the CoCo a crude 64×32 "graphics" mode entirely inside text-mode RAM, no
mode switch required. This is the mechanism behind every `CHR$(128+n)`
one-liner program you typed as a kid to draw a blocky mountain or invader —
no `PMODE`, no `SCREEN`, just `PRINT` with the right character codes.

The bit layout, from the same byte, reusing bits you haven't spent yet
([`crates/coco-core/src/video/text.rs:22-28`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L22-L28)):

```rust
const SG4_COLOR_SHIFT: u8 = 4;
const SG4_COLOR_MASK: u8 = 0x07;
const SG4_OFF_INDEX: usize = 8;
const SG4_UPPER_LEFT: u8 = 0x08;
const SG4_UPPER_RIGHT: u8 = 0x04;
const SG4_LOWER_LEFT: u8 = 0x02;
const SG4_LOWER_RIGHT: u8 = 0x01;
```

```
   bit:  7   6 5 4   3 2 1 0
        ┌───┬───────┬─────────┐
        │ 1 │ color │ pattern │
        └───┴───────┴─────────┘
              │        │ │ │ │
              │        │ │ │ └─ lower-right
              │        │ │ └─── lower-left
              │        │ └───── upper-right
              │        └─────── upper-left
              └──────────────── palette reg 0-7
```

Bits 6–4 pick one of GIME palette registers 0–7 as the "lit" colour for
*this whole cell* (all four blocks share one colour — SG4 is one colour per
cell, not per block); bits 3–0 are a four-block on/off pattern; an unlit
block always draws palette register 8 (which resolves to black in
CoCo-compatible mode). The paint loop
([`crates/coco-core/src/video/text.rs:167-182`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L167-L182), and the whole-field twin
`blit_semigraphics4` at [`:238`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L238)) is a direct transcription of that diagram:

```rust
if code & SEMIGRAPHICS_BIT != 0 {
    let on = palette[((code >> SG4_COLOR_SHIFT) & SG4_COLOR_MASK) as usize];
    let off = palette[SG4_OFF_INDEX];
    let bottom = glyph_row >= CELL_H / 2;
    for cx in 0..CELL_W {
        let right = cx >= CELL_W / 2;
        let block = match (bottom, right) {
            (false, false) => SG4_UPPER_LEFT,
            (false, true) => SG4_UPPER_RIGHT,
            (true, false) => SG4_LOWER_LEFT,
            (true, true) => SG4_LOWER_RIGHT,
        };
        let color = if code & block != 0 { on } else { off };
        paint_px(out, &mut x, xscale, color);
    }
}
```

`bottom`/`right` split the 8×12 cell into quadrants by comparing the
pixel's row/column against the cell's midpoint (`CELL_H/2 = 6`,
`CELL_W/2 = 4`) — no font lookup at all; SG4 cells never touch
`MC6847_FONT` or any of its siblings, because there's no glyph, just four
solid rectangles.

Now decode a real byte that puts both this section and §7.4 to a rigor
test: `$C1`. Binary: `1100 0001`. Bit 7 is **1** — before you even look at
bit 6, this cell is semigraphics, full stop; bit 6's meaning as "inverse"
simply does not apply here, because that meaning only exists in the `else`
branch. It's tempting to eyeball `$C1` as "`$41` (inverse 'A') plus
something" and expect a letter — that instinct is exactly the trap this
byte sets. Decode it correctly, as SG4:

- colour: `(0xC1 >> 4) & 0x07` = `0xC & 0x07` = `0b1100 & 0b0111` = `0b0100`
  = palette register **4**.
- pattern: low nibble `0x1` = `0001` = only `SG4_LOWER_RIGHT` set.

So `$C1` renders as: upper-left, upper-right, and lower-left blocks in the
"off" colour (palette register 8, black); lower-right block in whatever
palette register 4 currently resolves to. One lit square in the corner of
an otherwise-black cell — nothing at all like a letter.

---

## 7.7 Where the colours actually come from

Every colour used so far — text foreground/background, SG4's eight
selectable colours, the border — is a **palette register index**, not a
colour. Something still has to turn "palette register 4" into an actual
RGBA value, and on the CoCo 3 that something is the GIME, even while it's
imitating a VDG that (on real CoCo 1/2 hardware) never had programmable
palette registers at all. `TEXT_BG_INDEX`/`TEXT_FG_INDEX`
([`crates/coco-core/src/video.rs:47-48`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video.rs#L47-L48)) name which two of the GIME's
sixteen palette registers this mode reads:

```rust
pub const TEXT_BG_INDEX: usize = 12;
pub const TEXT_FG_INDEX: usize = 13;
```

— matching the MC6847's own `color_base_0`/`color_base_1` register
numbering, so a VDG-compatibility mode reading regs 12/13 is doing exactly
what a real MC6847-plus-analog-video-chip pairing would do, just with the
GIME standing in as the analog part. The important design point, already
flagged in the module doc: **these colours are data, programmed by the
ROM at boot, not hardcoded anywhere in the renderer.** `legacy_palette`
([`crates/coco-core/src/machine/video_mode.rs:62`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/video_mode.rs#L62)) proves it — on a CoCo 3
it snapshots all sixteen live palette registers through `GIME::color`
every field; it never special-cases index 12 or 13:

```rust
pub(super) fn legacy_palette(&self, css: bool) -> [[u8; 4]; video::PALETTE_LEN] {
    match self.config.variant {
        MachineVariant::Coco3 => {
            let mut resolved = [[0u8; 4]; video::PALETTE_LEN];
            for (i, entry) in resolved.iter_mut().enumerate() {
                *entry = self.bus.gime.color(self.bus.gime.palette[i]);
            }
            video::ColorSource::GIMEPalette(&resolved).resolve(css)
        }
        MachineVariant::Coco1 | MachineVariant::Coco2 => {
            video::ColorSource::VDGFixed.resolve(css)
        }
    }
}
```

Register-to-RGBA conversion itself is `GIME::rgb_color`
([`crates/coco-core/src/gime/palette.rs:56-62`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs#L56-L62)), and it's small enough to
read in one breath:

```rust
pub fn rgb_color(value: u8) -> [u8; 4] {
    let chan = |hi_bit: u8, lo_bit: u8| {
        let v = ((value >> hi_bit) & 1) << 1 | ((value >> lo_bit) & 1);
        v * 0x55
    };
    [chan(5, 2), chan(4, 1), chan(3, 0), 0xFF]
}
```

The GIME palette register is a 6-bit value, laid out `RRGGBB` two bits per
channel (documented convention: `RGBrgb`, i.e. the high bit of each pair is
more significant). `chan(hi, lo)` pulls one channel's two bits apart from
opposite ends of the byte, recombines them as a 0–3 two-bit number, and
scales `0..3` to `0..255` by multiplying by `0x55` (`85`) — the only four
values a 2-bit channel can produce are `0, 85, 170, 255`, evenly spaced
across the full byte range. Walk the exact value the boot screen's border
uses, `$12` = `0b010010`:

```
value = 0b01 0010
bit:     5432 10

R = chan(5, 2): bit5=0, bit2=0 → v = 0b00 = 0 → R = 0
G = chan(4, 1): bit4=1, bit1=1 → v = 0b11 = 3 → G = 3 * 0x55 = 0xFF
B = chan(3, 0): bit3=0, bit0=0 → v = 0b00 = 0 → B = 0
```

`rgb_color(0x12) = [0x00, 0xFF, 0x00, 0xFF]` — pure green, `#00FF00`. That's
`legacy_border_value`'s `BORDER6_GREEN` constant
([`crates/coco-core/src/video/text.rs:126`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L126)), used for the border of legacy
*graphics* modes; the plain-text border (this week's mode) instead resolves
`BORDER6_BLACK = 0x00`, which by the same arithmetic is trivially
`[0, 0, 0, 0xFF]` — confirming DESIGN.md's claim that the boot screen has a
**black border**. And `rgb_color(0x00)` for a freshly-reset, never-yet-programmed
palette register is likewise pure black — which is exactly why several of
this chapter's tests (§7.9) can leave a palette entry untouched and rely on
it reading as black rather than garbage.

`legacy_border_value` ([`text.rs:136-145`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L136-L145)) is worth reading once for what it
tells you about the CoCo's overall visual design language: text borders
black; graphics borders green (or white with CSS set) — real hardware
colour-codes the border by mode, a detail no photo of a "green screen"
alone would tell you, and only visible once you decode the actual register
math instead of eyeballing a screenshot.

---

## 7.8 The lab bench: PPM files, no GPU required

This codebase's `examples/` directory doubles as a lab bench precisely so
that "run it and look" doesn't require a display, a window manager, or a
GPU — which matters a great deal for a course meant to run in CI and on a
headless machine. The mechanism is almost insultingly simple: **PPM**
(Portable Pixmap), a plain-text-header, raw-binary-body image format simple
enough to write by hand:

```rust
let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
for px in m.framebuffer.chunks_exact(4) {
    ppm.extend_from_slice(&px[..3]);  // RGB, drop alpha
}
std::fs::write(path, ppm).unwrap();
```

`P6` (binary RGB), width, height, max channel value `255`, then three raw
bytes per pixel — no compression, no libraries, and (crucially) it drops
the framebuffer's alpha byte, since PPM has no alpha channel and every
pixel in this codebase is opaque anyway. Any image viewer, ImageMagick, or
`ffmpeg -i frame.ppm frame.png` reads it instantly.

Two examples in this crate use exactly this pattern:
[`crates/coco-core/examples/demo_frames.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/demo_frames.rs) (dumps periodic frames while
running an injected demo binary) and
[`crates/coco-core/examples/vdg_font_probe.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/vdg_font_probe.rs) (boots all three text
generators — CoCo 1, CoCo 2/T1, CoCo 3 — and dumps one PPM each, for
eyeballing "square O vs. rounded O vs. GIME O" side by side). **Be aware,
if you try them here:** both require real ROM images —
`demo_frames.rs` reads `roms/coco3.rom` unconditionally
(`.expect("roms/coco3.rom (git-ignored, local-only)")`), and
`vdg_font_probe.rs` needs `extbas11.rom`, `bas12.rom`, and `coco3.rom`. This
worktree has no `roms/` directory at all (it's git-ignored, present only on
machines where the copyrighted images were placed manually — check
`CLAUDE.md`'s "Local resources" note), so neither example runs here, and
you should not expect them to on a fresh checkout either. That's not a gap
in the course; it's the intended boundary between "code the emulator
core" (works everywhere, no ROM needed) and "trace-diff against a real
boot" (needs assets the repository deliberately doesn't ship).

The good news: everything this chapter actually needs to *verify* is
ROM-free, because `video::render_text` and `paint_legacy_text_line` take
already-decoded bytes and an already-resolved palette as plain arguments —
no CPU execution, no ROM, required at all. You can write your own PPM dump
of a hand-built screen in a dozen lines, reusing exactly the test fixtures
from §7.9:

```rust
use coco_core::video::{render_text, AlphaGenerator, BYTES_PER_PIXEL, FB_H, FB_W, PALETTE_LEN, SCREEN_LEN};

let mut palette = [[0u8; 4]; PALETTE_LEN];
palette[13] = [0x00, 0xFF, 0x00, 0xFF]; // TEXT_FG_INDEX: green
palette[12] = [0x00, 0x00, 0x00, 0xFF]; // TEXT_BG_INDEX: black
let border = [0x00, 0x00, 0x00, 0xFF];

let mut screen = [0x20u8; SCREEN_LEN]; // all spaces
screen[0] = 0x41; // 'A', inverse — §7.4.3

let mut fb = vec![0u8; FB_W * FB_H * BYTES_PER_PIXEL];
render_text(&screen, &palette, border, AlphaGenerator::GIME, 0, &mut fb);

let mut ppm = format!("P6\n{FB_W} {FB_H}\n255\n").into_bytes();
for px in fb.chunks_exact(4) {
    ppm.extend_from_slice(&px[..3]);
}
std::fs::write("/tmp/one_letter.ppm", ppm).unwrap();
```

No ROM, no `Machine`, no CPU cycle spent — just the pure decode function
you've been reading all chapter, fed synthetic RAM. This is the real "lab
bench" for this week, and it's the shape one of the exercises below asks
you to extend.

---

## 7.9 Three tests, walked

The test suites are, per the syllabus's own framing, "the textbook
exercises with answers." Three worth stepping through slowly.

### 7.9.1 Geometry: `border_and_active_area_use_their_colors`

([`crates/coco-core/tests/render.rs:50`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render.rs#L50))

```rust
#[test]
fn border_and_active_area_use_their_colors() {
    let mut fb = fb();
    render_text(&[SPACE; SCREEN_LEN], &palette(), BD, AlphaGenerator::MC6847, NO_GM0, &mut fb);

    assert_eq!(px(&fb, 0, 0), BD);
    assert_eq!(px(&fb, FB_W - 1, FB_H - 1), BD);
    for y in BORDER..BORDER + CELL_H {
        for x in BORDER..BORDER + CELL_W {
            assert_eq!(px(&fb, x, y), BG);
        }
    }
}
```

The whole screen is spaces (blank cells), so the active area should be
uniformly background-coloured, and the two extreme corners — `(0,0)` and
the bottom-right corner — should be border. This is the simplest possible
statement of §7.2.2's geometry: `BORDER` pixels of margin on every side of
a `COLS*CELL_W` by `ROWS*CELL_H` active rectangle, nothing more. If you
ever get the border/active math wrong in a refactor, this test is the
tripwire — it checks the *literal pixel* at the frame's four corners and
the *literal pixel* at the first cell's top-left, which is about as close
to "the formula from §7.2.2, executed" as a test can get.

### 7.9.2 The semigraphics-4 quadrant test

([`crates/coco-core/tests/render.rs:165`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render.rs#L165))

```rust
const SG4: u8 = 0x80 | (3 << 4) | 0b1001; // upper-left (0x08) + lower-right (0x01)
screen[0] = SG4;
render_text(&screen, &palette(), BD, AlphaGenerator::MC6847, NO_GM0, &mut fb);

assert_eq!(px(&fb, BORDER, BORDER), SG_COLOR);                                   // upper-left: lit
assert_eq!(px(&fb, BORDER + quad_x, BORDER), SG_OFF);                            // upper-right: unlit
assert_eq!(px(&fb, BORDER, BORDER + quad_y), SG_OFF);                            // lower-left: unlit
assert_eq!(px(&fb, BORDER + quad_x, BORDER + quad_y), SG_COLOR);                 // lower-right: lit
```

Decode the constant the same way §7.6 taught you: `0x80` sets SG4; `3 << 4`
puts colour index 3 in bits 6–4; `0b1001` sets `SG4_UPPER_LEFT` (`0x08`)
and `SG4_LOWER_RIGHT` (`0x01`). The test then samples exactly one pixel
from each of the four quadrants and checks it against the "on" or "off"
colour it predicted — a direct, minimal proof that the `(bottom, right)`
match in `paint_legacy_text_line`/`blit_semigraphics4` puts each bit where
you'd expect from the bit-layout diagram in §7.6, and nowhere else.

### 7.9.3 A CoCo 3 specifically: `coco3_compat_text_draws_gime_font_not_either_vdg_font`

([`crates/coco-core/tests/render_coco12/coco3_compat_text.rs:60`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_coco12/coco3_compat_text.rs#L60))

This one is the payoff test for §7.4's whole "no VDG chip exists on a CoCo
3" claim. It boots a *real* `Machine` (not a bare call to `render_text`),
forces legacy mode via `INIT0`'s `COCO` bit, writes one character, runs a
field, and checks which of three possible glyph shapes came out:

```rust
fn boot_parked_coco3() -> Machine {
    let rom = vec![0u8; 32 * 1024].into_boxed_slice();
    let mut m = Machine::new(MachineConfig::default(), rom);
    m.bus.gime.write_init0(init0::COCO);
    m.bus.write(0x0000, 0x20); // BRA
    m.bus.write(0x0001, 0xFE); // -2
    m.bus.gime.palette[TEXT_FG_INDEX] = GIME_WHITE6;
    // palette[TEXT_BG_INDEX] left at its zeroed default: black.
    m.bus.write(0xFFC7, 0); // SAM F0 set
    m.bus.write(0xFFC9, 0); // SAM F1 set -> display base $0600 (page 3)
    m
}

#[test]
fn coco3_compat_text_draws_gime_font_not_either_vdg_font() {
    let mut m = boot_parked_coco3();
    m.bus.pia1.b.output = 0; // text mode, CSS=0, GM0 clear
    m.bus.write(COCO3_SCREEN_BASE, CODE_O);
    m.run_field();

    let cell = sample_cell_canonical(&m.framebuffer, 0, 0, GIME_WHITE_RGBA, GIME_BLACK_RGBA);

    assert_eq!(cell, glyph_bits(&GIME_O_GLYPH), "...must draw the GIME's own 'O' glyph...");
    assert_ne!(cell, glyph_bits(&PLAIN_O_GLYPH), "...must not draw the plain MC6847's 'O'...");
    assert_ne!(cell, glyph_bits(&T1_O_GLYPH), "...must not draw the MC6847T1's 'O'...");
}
```

Two things worth noticing beyond the assertions themselves. First, the ROM
here is a synthetic **all-zero** 32K array, not `roms/coco3.rom` — the test
parks the CPU on an infinite `BRA *` at reset (bytes `$20, $FE`) so it never
executes anything else, then pokes the video state directly through the
bus. This is exactly the pattern from §7.8's PPM snippet, one level up: no
real ROM, full `Machine`, full field-timed render path, entirely
deterministic. Second — `sample_cell_canonical`, not the plain `sample_cell`
you saw in §7.9.2's suite — reads pixels off the CoCo 3's *canonical*
640×240 canvas (§7.3), accounting for the ×2 horizontal scale and the
`NON_WIDE_BORDER_X`/`vertical_window` offsets, rather than the smaller
fixed CoCo 1/2 buffer the plain helper assumes. Same underlying idea,
different geometry constants — which is precisely the split §7.3 flagged.

Letter `'O'` (code `$0F`) was chosen deliberately: it's one of the few
glyphs that visibly differs across all three fonts (square on the plain
MC6847, rounded on the T1, a third shape again on the GIME's own font), so
a single test can prove "the right font ROM, not merely *a* plausible
font" — the two `assert_ne!`s are doing as much work as the `assert_eq!`.

---

## 7.10 Reading assignment

In this order:

1. [`crates/coco-core/src/raster.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/raster.rs) (all 40 lines) — the canonical canvas,
   §7.3's entire subject.
2. [`crates/coco-core/src/video.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video.rs) — module-level doc, then `ColorSource`
   and its `resolve` method (§7.7's CoCo 1/2 vs. CoCo 3 split).
3. [`crates/coco-core/src/video/text.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs) — the whole file; it's 271 lines
   and every one of them was excerpted or explained somewhere in this
   chapter. Read `resolve_alpha_cell` twice.
4. [`crates/coco-core/src/font6847.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font6847.rs)'s module doc and the first ten
   entries of `MC6847_FONT`; then skim `font_gime.rs`'s doc comment (the
   `GIME_FONT` vs. `GIME_LOWRES_FONT` distinction: hi-res 40/80-column text
   uses the former, next week; this week's legacy mode uses the latter).
5. [`crates/coco-core/src/machine/video_mode.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/video_mode.rs) and
   [`crates/coco-core/src/machine/render.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs) — `video_mode()`,
   `paint_legacy_scanline`, and (briefly, for contrast) `render_coco_text`/
   `render_coco_graphics`, the CoCo 1/2 whole-field path.
6. [`crates/coco-core/src/gime/palette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs) — just `rgb_color`; ignore the
   composite tables entirely, they're week 9.
7. Tests: [`crates/coco-core/tests/render.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render.rs) in full (185 lines); skim
   `crates/coco-core/tests/render_coco12/` ([`mc6847_fonts.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_coco12/mc6847_fonts.rs),
   [`coco3_compat_text.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_coco12/coco3_compat_text.rs), [`common.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_coco12/common.rs)'s two `sample_cell*` helpers).

Run the whole suite while you read:

```
cargo test -p coco-core --test render --test render_coco12
```

Fifteen tests, all green, all with no ROM and no window.

---

## 7.11 Exercises

**7.1 — Hand-render a byte (build/by-hand).** Decode the byte `$9D` fully:
which bits are set, what mode does bit 7 select, and what does the cell
look like? (Hint: `$9D = 1001 1101`.) Then draw the resulting 8×12 pixel
grid on paper, `#`/`.` style like §7.5.1's worked example — for an
alphanumeric byte, look up the actual glyph rows in `MC6847_FONT`; for a
semigraphics byte, work out which of the four quadrants are lit from the
low nibble. Check your colour assignment (which quadrant/glyph pixels are
"on" vs. "off," and which palette index each maps to) against
`resolve_alpha_cell`/`paint_legacy_text_line`.

**7.2 — Sabotage and observe (sabotage, verified).** In
[`crates/coco-core/src/video/text.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs)'s `resolve_alpha_cell` (line 98–99),
change

```rust
let true_lowercase =
    lowercase_capable && !inverse && ff22 & VDG_GM0_INTEXT != 0 && glyph_code < 0x20;
```

to drop the GM0 gate entirely:

```rust
let true_lowercase = lowercase_capable && !inverse && glyph_code < 0x20;
```

Run `cargo test -p coco-core --test render_coco12` and read the two
failures closely — one is `mc6847_fonts::plain_vdg_draws_square_o_t1_draws_rounded_o`,
the other `coco3_compat_text::coco3_compat_text_draws_gime_font_not_either_vdg_font`.
Both boot with GM0 clear and expect an ordinary uppercase glyph; with the
gate gone, every code-under-`$20`, non-inverse cell now silently goes
lowercase regardless of what PIA1 `$FF22` says, so `'O'` (code `$0F`)
renders as whatever `font[0x40+0x0F]` holds instead of `font[0x0F]` — wrong
glyph, and the assertion dumps both 8×12 bit grids so you can see exactly
which pixels differ. (For comparison, try dropping only the `!inverse`
clause instead — no test in this suite exercises an inverted, GM0-set cell,
so that particular regression currently slips through green. Worth noting
as a real gap, not something to "fix" here.) Revert your change (`git
diff crates/coco-core/src/video/text.rs` should be empty, `git status`
clean) and confirm the suite is green again.

**7.3 — Extend the lab bench (build).** Using the ROM-free pattern from
§7.8, write a small program (a new example, or a scratch `main.rs` outside
this repo, your choice) that renders a full 32×16 screen reading "HELLO
FROM CHAPTER 7" centered on a green background with a black border, and
dumps it as a PPM. Open the result in an image viewer. (Reminder: screen
codes for letters are `code = letter_ascii - '@'` for uppercase, i.e.
`decode_alpha_char` run in reverse — and don't forget bit 6 if you want the
authentic black-on-green look instead of green-on-black.)

**7.4 — Modify a glyph (build).** Pick one glyph row in `MC6847_FONT` (say,
the `'S'` entry) and hand-edit one byte to visibly deform the letter —
break the top curve, say. Render it (via a test, or the pattern from
exercise 7.3) and confirm the deformation shows up exactly where you
expect in the pixel output. This is the fastest way to build real
intuition for the `bits & (0x80 >> cx)` addressing — you're now editing the
"ROM" the same way Motorola's mask engineers did in 1980, one bit at a
time.

**7.5 — Read and predict (read/predict).** Without running anything: a
CoCo 3 has `INIT0 COCO` clear and `$FF98 BP` set to 1. According to
`video_mode()` (§7.4.1), which `VideoMode` variant is active? Now suppose a
running BASIC program executes `POKE &HFF90, PEEK(&HFF90) OR 8` (setting
`INIT0` bit 3) mid-frame, one field before you next call
`text_screen_lines()`. What does `text_screen_lines()` report now, and from
which base address does it read? Justify both answers by naming the exact
function and branch you traced, then check yourself against
`machine/video_mode.rs`.

**7.6 — Recall the reveal (recall, three sentences max).** Without opening
any file, explain to someone who's never seen this codebase: (a) which
chip a real CoCo 3 uses to draw its boot prompt, despite that prompt
looking exactly like a CoCo 1/2 screen; (b) which single bit, set on every
character, is responsible for "black on green" rather than "green on
black"; (c) where the actual RGB values for that green and that black
ultimately come from. If you can answer all three without notes, this
chapter did its job.

---

## What's next

Week 8 turns off the `INIT0 COCO` bit this chapter spent so long explaining
and asks what happens on the other side of it: the GIME's *own* video
registers, `$FF98`–`$FF9F`, native text with real per-character colour
attributes and blink, and native bitmap graphics at up to 640 pixels wide.
You already know the canonical canvas, the palette-register-to-RGBA
pipeline, and the per-scanline call chain — week 8 is "same raster,
same painter, a completely different register file deciding what to paint."
Bring your `$FF90`–`$FF9F` memory map from week 1's table; you're about to
need every byte of it.
