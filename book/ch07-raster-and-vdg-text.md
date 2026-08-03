# Chapter 7 — How a Raster Works, and the Legacy VDG Text Mode

*Week 7. Goal: render the CoCo's legacy text screen without a GPU. This chapter
builds a raster and framebuffer model, then decodes the GIME's MC6847-compatible
text mode into the green BASIC prompt.*

---

Video introduces vocabulary that systems programmers may not have used.
Terms such as *raster*,
*scanline*, *framebuffer*, and *blit* get used as though everyone had been
issued a copy of the definitions at some point. This chapter assumes nobody
was. Section 7.2 builds the entire model from an electron beam upward, and
it does so before a single line of Rust, because every function in the video
subsystem is trivial once the picture behind it is clear, and inscrutable
before that.

The core renderer uses no transforms, blending, or sampling. A pixel lives at a computable byte
offset in a flat array, and every renderer in `coco-core` — this week's
text mode, next week's GIME graphics, Chapter 9's PMODE bitmaps — is a loop
that decides which color to write at which offset. Once that formula is in
hand, the difficulty of a video mode is entirely a question of decoding
bytes, and decoding bytes is what the previous six weeks were about.

The green prompt shown at power-up is produced by the GIME's compatibility
implementation of the MC6847 Video Display
Generator, a chip that is not on the CoCo 3's board at all, in a
32-column-by-16-row text mode inherited whole from 1980. Everything in that
mode — the character cell, the font ROM, the inverse-video bit, the blocky
"semigraphics" cells that every BASIC one-liner drew with, the two palette
registers that decide the colors — is legacy compatibility, faithfully
reproduced by silicon that had no obligation to do so except that Tandy
could not afford to break the existing software library.

Nothing in this chapter requires a ROM image, a window, or a GPU. Every
claim it makes about pixels can be checked by calling a pure function on a
synthetic screen buffer and looking at the bytes that come back, which is
exactly what §7.8's lab bench and §7.9's tests do.

---

## 7.1 Why this is where video finally starts

Chapter 6 connects to the video renderer through one function call.

Chapter 6 built the clock. Its central loop runs the CPU in slices of one
scanline's worth of cycles, and after each slice it runs a per-line trailer
called `end_of_line()`
([`crates/coco-core/src/machine/run.rs:130`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs#L130)).
That trailer is where the machine's whole per-line rhythm lives: the
horizontal sync pulse that PIA0 latches, the two field-sync edges at their
designated scanlines, one audio sample flushed into the mix buffer, one
tick of the GIME interval timer, and — the line this chapter is about — one
call to `render_scanline()`:

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

The important structural fact is that `render_scanline()`
([`crates/coco-core/src/machine/render.rs:29`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L29))
was already being called on every line back in Chapter 6, after the CPU had
reached that line's instruction-granular cycle budget. It was simply not
doing anything a human could see.
Everything the video subsystem needed — a line counter, a field boundary, a
guarantee that the CPU had reached each scanline budget before
each call — was in place, and the function on the other end of the call was
geometry bookkeeping with no pixels behind it. This week fills it in. By the
end of the chapter, calling that function 262 times, of which 192 land
inside the active body of a 32×16 text screen, produces the CoCo 3 cold-boot
display through the emulator's scanline model.

That ordering — clock first, pixels second — is worth defending, because
the instinct almost everyone brings to a CoCo emulator is "start with the
GIME." The GIME is the interesting chip; it is the reason the CoCo 3 is
worth emulating at all; Chapter 1's tour gave it three separate jobs and a
paragraph of admiration. And yet starting there produces nothing you can
look at. The video hardware does not invent a screen. It reads bytes that a
CPU wrote, at addresses a memory decoder resolved, on a schedule a clock
imposed. Remove any one of those three and the video chip is a very
elaborate way to produce a blank rectangle. Chapters 2 through 6 built exactly
those three things, in the order that makes each one testable, and this
week collects on all of them at once.

It is equally worth being precise about what this chapter is *not*, because
"video" is a large word and the CoCo 3 has a lot of it. This week does not
cover the GIME's native 40- and 80-column text or its bitmapped graphics
modes, which live behind the register file at `$FF98`–`$FF9F`; those are
Chapter 8, which opens by turning off the very compatibility bit this chapter
spends its second half explaining. This week does not cover
composite artifact color, mid-frame register changes that split one screen
into two different modes, or the PMODE bitmap graphics that most CoCo games
actually used; those are Chapter 9. What is left after those exclusions is
deliberately narrow: one video mode, the one every CoCo 3 owner saw first
and saw most, decoded down to individual bits and individual pixels, with
nothing hand-waved.

---

## 7.2 Raster fundamentals, from zero

This book assumes no graphics programming background, so this section builds
the whole picture before any Rust. Skip nothing here — every later section
leans on this arithmetic, and those sections are short precisely because
this one is not.

### 7.2.1 What a CRT actually does

Start with the display, because on this machine the display is not a
peripheral that the video chip talks to. It is a clock that the video chip
obeys.

A cathode-ray-tube television does not have pixels the way a modern LCD
panel does. It has a single electron beam and a set of magnetic coils that
steer it across a phosphor-coated screen. Where the beam lands, the
phosphor glows; how brightly it glows depends on how hard the beam is
driven at that instant. The steering follows one fixed, boring path, over
and over, sixty times a second on the North American NTSC standard:

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

The beam sweeps left to right with its brightness modulated by the incoming
video signal, painting one horizontal strip of picture. That strip is a
*scanline*, and it is the atomic unit of everything that follows. At the
right-hand edge the beam is blanked — switched off, so it draws nothing —
and steered back to the left edge to start the next strip one line lower.
The signal tells the television when to do this by means of a *horizontal
sync* pulse, universally shortened to hsync: a distinctive dip in the
signal level that the set's timing circuitry recognizes as "start a new line
now." After the last scanline of the sweep, the beam is blanked again and
steered all the way back to the top-left corner, cued by a *vertical sync*
pulse, or vsync, meaning "start a new picture now."

One complete top-to-bottom sweep is a *field*. On NTSC a field is 262
scanlines, and fields arrive at roughly 59.94 per second — the two constants
derived in Chapter 6, which live in
[`crates/coco-core/src/config.rs:52`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/config.rs#L52)
and
[`:60`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/config.rs#L60).
Everything the emulator does per-line, per-field, or per-second is an
integer relationship between those two numbers and the CPU clock, which is
why Chapter 6 could drive video, audio, and the interval timer off one cycle
counter without any of them drifting apart.

Not every one of those 262 lines carries a picture, and the reasons are
worth understanding rather than memorizing. Part of the answer is
mechanical: the beam needs real time to travel from the bottom of the screen
back to the top, and the lines that elapse during that journey are *vertical
blanking* — counted by the timing hardware, invisible on the glass. The
other part of the answer is commercial. Consumer televisions of the era were
not precision instruments, and no two of them cropped the picture the same
way; a strip at the top, bottom, and sides always disappeared behind the
set's bezel or off the edge of the tube, a phenomenon the industry called
*overscan*. A video chip that painted meaningful content out to the very
edge of its sweep would have had that content eaten by an unknown fraction
of the sets in the field.

The universal answer, on every home computer and console of the period, was
to draw a smaller *active area* in the middle of the field and surround it
with a solid *border* color: border on the left and right of each visible
line, before and after the active pixels, and whole border-colored lines
above and below the active body. The border is not a decorative flourish
that a renderer may skip. It is a required part of a legal video signal and
a deliberate safety margin against the customer's television, and it is
exactly the colored strip that framed the CoCo's picture on every set it
was ever plugged into.

That gives the complete model, and it is small enough to hold in one
sentence: a field is 262 potential scanlines; not all of them are visible;
of the visible ones, only a rectangle in the middle carries picture, and
everything outside that rectangle is border or blanking. Every video chip
this course will meet — the MC6847, the SAM, and the GIME — is, at bottom, a
machine that answers two questions once per scanline. Is this line border or
picture? And if it is picture, what color is each pixel across it?

It is worth pausing on a thought experiment, because it explains a class of
bug that is otherwise baffling. Suppose an emulator decided the blanking
lines were a waste of effort and simply skipped them — 240 lines of work per
field instead of 262. The picture would look identical, and the machine
would be subtly, permanently wrong: the CPU would get 240 lines' worth of
cycles per field instead of 262, the interval timer counting horizontal
syncs would run about nine percent fast, and the field-sync interrupt that
BASIC's housekeeping depends on would arrive early. This is why Chapter 6's
loop counts all 262 lines and why `render_scanline` handles the invisible
ones by returning early rather than by never being called. Time is the
product; pixels are a side effect.

### 7.2.2 The framebuffer: a screen is just bytes

Real hardware has no framebuffer. The beam paints directly from a running
decode of RAM, live, as it sweeps: at the moment the beam is thirty
nanoseconds into scanline 40, the video chip is fetching the byte that
belongs at that spot and converting it to a voltage. There is no
intermediate copy of the screen anywhere, because there is nowhere to put
one and no reason to want one.

An emulator cannot work that way, because there is no beam. So it assembles
the output in an ordinary array in memory and hands that array to the host
to display — either once the whole field is complete, or, as §7.3 will show,
one scanline at a time as each is computed. That array is the
*framebuffer*, and in this codebase it is a plain `Vec<u8>` hanging off the
`Machine` struct. If that sounds familiar, it should: it is the field Chapter 1
used as its example of *derived* state, marked `#[serde(skip)]` and excluded
from save states on the grounds that the next rendered field repaints every
pixel from RAM and the registers anyway.

The byte layout is the convention called *RGBA*, which is close to
universal in graphics work: four bytes per pixel, in the order red, green,
blue, alpha. Alpha is opacity, and in this emulator it is always `0xFF`,
fully opaque, because nothing here ever needs to see through one pixel to
another. `BYTES_PER_PIXEL` is `4`
([`crates/coco-core/src/video.rs:40`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video.rs#L40)),
and it is a named constant rather than a literal `4` sprinkled through the
code for the usual reason: the number appears in every index calculation in
the subsystem, and a named constant makes each of those calculations
readable as "one pixel's worth" instead of as arithmetic. A single white
pixel is the four bytes `[0xFF, 0xFF, 0xFF, 0xFF]`; black is
`[0x00, 0x00, 0x00, 0xFF]`; and the pure green that dominates this chapter is
`[0x00, 0xFF, 0x00, 0xFF]`.

Pixels are stored *row-major*, meaning all of row 0's pixels left to right,
then all of row 1's, and so on to the bottom of the buffer. There is nothing
arbitrary about that choice. It is §7.2.1's scanline sweep written directly
into an array layout: the beam's path through the picture and the walk
through the framebuffer are the same walk. For a framebuffer `W` pixels
wide, the byte offset of the red channel of the pixel at column `x`, row `y`
is:

```
offset = (y * W + x) * BYTES_PER_PIXEL
```

Read that as two steps rather than one expression. The `y * W + x` part
converts a two-dimensional coordinate into a one-dimensional pixel index by
skipping `y` whole rows of `W` pixels and then stepping `x` pixels into the
row that follows. Multiplying by `BYTES_PER_PIXEL` converts a pixel index
into a byte offset. The quantity `W * BYTES_PER_PIXEL` — the number of bytes
from one pixel to the pixel directly below it — is called the *stride*, and
it is the only other piece of framebuffer vocabulary this book needs.

Walk the formula once by hand so it stops being abstract. Take a small 16×4
framebuffer, so `W = 16`, and find pixel `(3, 2)`, meaning column 3 of row 2:

```
offset = (2 * 16 + 3) * 4
       = (32 + 3) * 4
       = 35 * 4
       = 140
```

Byte 140 is that pixel's red channel; 141 is green, 142 is blue, and 143 is
alpha. Every pixel-writing function in this chapter — `paint_px`,
`blit_cell`, `blit_semigraphics4` — is that one multiplication, sometimes
with an extra offset added to skip past a border. You now know one hundred
percent of the arithmetic of two-dimensional graphics as this codebase
practices it. There is no further "graphics math" waiting in a later
chapter; everything else in the video subsystem is bookkeeping about *which*
color belongs at *which* offset.

Here is the arithmetic in the flesh, verbatim from the renderer
([`crates/coco-core/src/video/text.rs:260`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L260)).
The function's job is to draw one character cell: it receives the
framebuffer, the cell's row and column on the text screen, the twelve bytes
of font data for the glyph, and the two colors to draw with.

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

Read it from the inside out. The expression
`(y0 + cy) * FB_W + (x0 + cx)` is exactly `y * W + x` from the formula
above, with `x0` and `y0` computed first to shift the origin into the right
character cell and past the border. The two loops walk the cell's twelve
rows and eight columns. The test `bits & (0x80 >> cx)` picks out one bit of
the current font row, starting from the most significant bit and marching
right, which is how a font byte maps onto a row of pixels with the leftmost
pixel in the high bit. And `copy_from_slice` stamps four bytes — one RGBA
color — into the buffer at the computed offset.

The word *blit* in the function name is worth defining once, since it will
recur. It is old graphics jargon, short for "block transfer," and it means
copying a rectangular block of pixels into a buffer. In this codebase a blit
is never more sophisticated than the loop above.

The geometry constants those expressions lean on are all declared together,
and reading them as a group is the fastest way to internalize the shape of a
VDG text screen
([`crates/coco-core/src/video.rs:25-38`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video.rs#L25-L38)):

```rust
/// VDG character cell: 8 pixels wide × 12 raster lines (matches the font rows).
pub const CELL_W: usize = 8;
pub const CELL_H: usize = 12;

pub const COLS: usize = 32;
pub const ROWS: usize = 16;

/// Active display geometry.
pub const ACTIVE_W: usize = COLS * CELL_W; // 256
pub const ACTIVE_H: usize = ROWS * CELL_H; // 192
/// Border thickness around the active area.
pub const BORDER: usize = 16;
pub const FB_W: usize = ACTIVE_W + 2 * BORDER; // 288
pub const FB_H: usize = ACTIVE_H + 2 * BORDER; // 224
```

Every number in the rest of the chapter descends from those nine lines. A
character cell is eight pixels wide and twelve scanlines tall. The screen is
thirty-two cells across and sixteen down. Multiply those out and the active
picture is 256 by 192 pixels, which is the native resolution of every
CoCo-compatible video mode this book will meet, text and graphics alike. Add
a sixteen-pixel border on all four sides and the whole legacy framebuffer is
288 by 224. Notice that the constants are *derived* rather than restated:
`ACTIVE_W` is written as `COLS * CELL_W`, not as `256`. That is the same
discipline Chapter 1 praised in the CPU's register struct — say the thing once,
in the form that shows where it came from, and let the compiler do the
multiplication.

Once `blit_cell` is legible, every renderer in the video subsystem is
legible, because they are all this same loop with a different rule for
computing the color. This week's text mode looks the color up in a font
table. Next week's GIME graphics modes unpack it from packed pixel bits.
Chapter 9's PMODE renderers do the same with a different packing. The loop
never changes.

### 7.2.3 Connecting the two: one call per scanline

Chapter 6 built the clock and §7.2.2 built the target; this section is the wire
between them. The call chain, top to bottom, is short enough to write out in
full:

```
run_field()                        (machine/run.rs)
  └─ per line: end_of_line()        — hsync, then:
       └─ render_scanline()         (machine/render.rs) ← THIS CHAPTER
            └─ paint_legacy_scanline(row)   for legacy VDG text (this week)
                 └─ paint_legacy_text_line(...)  → blit one row of glyphs
                      └─ paint_px(...)      → the (y*W+x)*4 arithmetic
```

The top of that chain is worth reading in the source, because it is where
the per-field and per-line responsibilities separate. Here is the first half
of `render_scanline`, verbatim
([`crates/coco-core/src/machine/render.rs:29-51`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L29-L51)):

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
    let row = self.line as usize;
    let Some(scan) = self.field_scan.as_ref() else {
        return;
    };
    if row >= raster::CANVAS_H {
        return; // blanking lines 240..262
    }
    if scan.legacy {
        self.paint_legacy_scanline(row);
        return;
    }
```

Four decisions in twenty-three lines, each one a piece of the model from
§7.2.1. The first guard restricts this whole path to the CoCo 3; a CoCo 1 or
2 renders differently, and §7.3 explains why. The `self.line == 0` block runs
once per field. It *latches* the registers that a video chip samples at the
top of the picture rather than continuously — most importantly the
compatibility bit that decides whether this field is a legacy VDG field at
all — and it sizes the framebuffer to the canvas that §7.3 is about. The `row
>= raster::CANVAS_H` check is vertical blanking, exactly as promised: those
lines are counted, they cost the CPU its cycles, and they paint nothing. And
the final branch chooses this chapter's painter over next chapter's.

The word *latch* deserves its own note, since it will keep coming back. In
hardware, latching means sampling a signal at a defined instant and holding
that sample steady regardless of what the source does afterward. Real video
chips latch a handful of registers at the start of each field precisely so
that a program writing to those registers halfway down the screen cannot
tear the picture in two. Modeling that faithfully means the emulator must
copy those values once per field into somewhere stable, which is what
`FieldScan::latch` does. Everything *not* in that latched group is re-read
live on every scanline, and Chapter 9's mid-frame split effects are entirely
about which registers fall on which side of that line.

So `render_scanline` is called 262 times per field, whether or not the line
in question is visible. This week's job is one branch deep inside it,
`paint_legacy_scanline`, which decides for the current line whether it is
border or active and, if active, which row of which glyphs goes where.
That is the whole chapter in one sentence. Everything from here on is
filling in the "which glyphs" part correctly, and the first thing that needs
settling is what shape the canvas is.

---

## 7.3 The canonical 640×240 canvas

A modern display has no fixed relationship to a CoCo's video timing.
Nothing about a 1986 NTSC field tells a 2026 window manager how many pixels
wide to make anything, so *something* in the emulator has to decide the
dimensions of the buffer the core hands to the frontend. That decision is
more consequential than it looks, because it determines whether the frontend
stays simple forever or grows a special case for every video mode the
machine can enter.

The codebase's answer, for the CoCo 3, is a single fixed-size canvas that
every video mode renders into: this week's legacy text, next week's
GIME-native text and graphics, Chapter 9's advanced modes, all of them. The
whole module that defines it is forty lines, and it repays reading in full
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

Read this slowly. The design decision it embodies is worth considerably more
than forty lines suggest, and three separate ideas are packed into it.

### One fixed size, for every mode, forever

The CoCo 3 can be in dozens of legal video configurations. There is
32-column legacy text, 40- and 80-column GIME text, four bit depths of
bitmapped graphics at four widths apiece, and a handful of legacy PMODE
resolutions besides — each with its own native pixel count and its own
notion of how wide a pixel is.

The naive design renders each mode into a buffer of its own native size and
makes the frontend cope. That sounds harmless until you count what the
frontend then has to know: which modes are wide, which pixel aspect ratios
need correcting, how to rescale a texture when a running program switches
modes mid-session, and what to do about the borders each mode draws
differently. Every one of those is a place for mode-specific knowledge to
leak out of the emulator core and into the window code, and Chapter 15 would
spend its budget re-deriving the video subsystem from the outside.

`raster.rs` refuses that trade. It fixes one target — 640 by 240 — and makes
each *mode* responsible for scaling itself up to fill it, always by an
integer factor. A 256-pixel-wide legacy text row, this week's mode, is
doubled to 512 active pixels. An 80-column GIME text row next week may not
need scaling at all. The frontend, from Chapter 15 onward, then does exactly
one thing forever: take a 640×240 RGBA buffer, upload it as a texture, and
letterbox it to a 4:3 aspect ratio. No mode-specific frontend code, ever.

The specific numbers are not arbitrary either: 640×240 is MAME's own visible
window for the CoCo 3, which means a frame produced by this emulator and a
frame produced by MAME can be diffed pixel-for-pixel with no rescaling step
in between to muddy the comparison. That is the headless-testability payoff
the syllabus promised, and it is worth being concrete about what it buys: a
test can allocate a 640×240 `Vec<u8>`, call the renderer, and assert on
individual pixel colors, with no window, no GPU, and no scaling logic that
could be wrong in two places at once.

The insistence on *integer* scaling is a related discipline. A non-integer
scale requires deciding what to do with a pixel that lands half in one
output pixel and half in the next, and every answer to that question — round,
truncate, blend — is a small lie about what the hardware produced. Doubling
every native pixel into exactly two canvas pixels tells no lies at all, and
it makes the reverse mapping exact, which is precisely what the test helper
in §7.9.3 relies on when it samples one canvas pixel per native pixel.

### Wide, non-wide, and why this week never has to care

`NON_WIDE_ACTIVE_W` is 512, and the constant immediately below it computes
the side border from it: `(640 − 512) / 2 = 64` pixels on each side. Most
modes, including this week's legacy text, draw a 512-pixel active body
centered in the 640-pixel canvas with 64 pixels of border to the left and
right. A small number of GIME modes — next week's widest resolutions — use
the full 640 with no side border at all, and those are called *wide* modes.

Legacy VDG text is never wide. The module doc records where that rule comes
from: MAME's own scanline renderer computes `wide = !legacy && ($FF99 &
0x04)`, so the legacy flag alone rules wide mode out before any register bit
is consulted. That is why `paint_legacy_scanline` uses `NON_WIDE_BORDER_X`
and `NON_WIDE_ACTIVE_W` unconditionally and never checks a wide flag
anywhere. You will meet the register bit that selects wide mode next week;
this week it is simply not reachable.

The horizontal scale for this week's mode falls straight out of those
constants, and the codebase writes it as a division rather than as the
literal `2`
([`crates/coco-core/src/machine/render.rs:150-161`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L150-L161)):

```rust
} else {
    let generator = video::AlphaGenerator::GIME;
    let xscale = raster::NON_WIDE_ACTIVE_W / (video::COLS * video::CELL_W);
    video::paint_legacy_text_line(
        &buf[..row_bytes],
        &palette,
        generator,
        ff22,
        line_in_row,
        xscale,
        active,
    );
}
```

`NON_WIDE_ACTIVE_W / (COLS * CELL_W)` is `512 / (32 * 8)`, which is `512 /
256`, which is 2. Writing it that way rather than as a constant means the
scale factor cannot fall out of step with the geometry it is derived from:
change the active width or the cell width and the scale follows
automatically, or fails loudly by producing a nonsense value, rather than
silently disagreeing with a hardcoded literal three files away.

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
> pixel for `copy_from_slice` to stamp the same color into. `chunks_exact`
> (rather than plain `chunks`) guarantees every chunk is a full
> `BYTES_PER_PIXEL` long and silently drops any short trailing remainder —
> the right call here specifically *because* the slice length is
> `xscale * BYTES_PER_PIXEL` by construction, always an exact multiple, so a
> short last chunk could only mean a bug upstream, never a case worth
> handling gracefully. This one function is "draw a native pixel `xscale`
> times" for every legacy scanline in the codebase — nothing more clever is
> needed.
>
> Note also what the signature says about ownership of the cursor. `x` is an
> `&mut usize` rather than a return value, so the caller's horizontal
> position advances as a side effect of painting. That is a small piece of
> deliberate design: the caller loops over thirty-two cells and eight pixels
> per cell without ever computing an x coordinate itself, and the one place
> that knows how far a native pixel advances the cursor is the one place that
> draws it.

### The vertical window, and one glitch this emulator declines to model

`vertical_window(lpf)` answers the vertical half of §7.2.1's question:
given the two-bit lines-per-field selector in the GIME's `$FF99` register,
how many border rows sit above the active body, and how tall is that body? For
`lpf = 0` the answer is 25 border rows followed by a 192-row body. The
remaining 23 rows are the bottom border, which the function does not return
because whoever calls it can compute it: 25 above plus 192 of body plus 23
below is exactly the 240 visible rows of the canvas.

Two arithmetic facts are worth noticing in that first row. First, 192 is
exactly `ROWS * CELL_H` from §7.2.2 — sixteen character rows of twelve
scanlines each. The default vertical geometry of the GIME and the geometry
of a VDG text screen agree exactly, which is not a coincidence but the whole
point of a compatibility mode. Second, the value 25 is not a rounding: it
comes from MAME's own geometry computation, and this codebase's
`vertical_window` reproduces all four cases from that source rather than
inventing centered approximations of its own.

With one exception, which the doc comment is careful to flag. `LPF = 2` is
the glitched case: on real silicon that setting produces a line count the
documentation describes as zero or infinite, with the visible result
depending on exactly where in the raster the write landed. Modeling that
faithfully would mean modeling the write's raster position and the failure
mode it produces, for a setting no sane program uses. The codebase instead
picks a defined, centered approximation — 210 rows, sitting `(240 − 210) / 2`
from the top — and says so in the comment. That is a fidelity trade-off of
exactly the kind Chapter 1's budget metaphor described: the cost of being
stricter is high, the software that would notice is hypothetical, and the
decision is written down where the next person will find it rather than
discovered later as a mystery.

This is also a live register rather than a compile-time constant, and it is
read on every scanline. Even the legacy VDG path consults it, which is worth
stating explicitly because it violates the mental model most people bring:
real CoCo 3 hardware runs the GIME's vertical geometry unconditionally,
whether or not the chip is currently pretending to be a VDG. Here is that
read, at the top of `paint_legacy_scanline`
([`crates/coco-core/src/machine/render.rs:84-92`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L84-L92)):

```rust
let lpf =
    ((self.bus.gime.vres & gime::vres::LPF_MASK) >> gime::vres::LPF_SHIFT) as usize;
let (top, body) = raster::vertical_window(lpf);
if row < top || row >= top + body {
    for px in row_px.chunks_exact_mut(BYTES_PER_PIXEL) {
        px.copy_from_slice(&border);
    }
    return;
}
```

That is §7.2.1's first question, in Rust, once per line: is this line border
or picture? If the row falls above the active body or below it, the entire
640-pixel row is filled with the border color and the function returns
without touching a font or a screen byte. Only if the row falls inside the
body does anything else happen — and the first thing that happens is the
horizontal version of the same question, filling the 64-pixel margins on
each side before the 512-pixel active span is painted.

### A caveat: the canonical canvas is a CoCo 3 artifact

Since this codebase emulates real CoCo 1 and CoCo 2 hardware as well, one
boundary needs stating plainly before the tests in §7.9 confuse anybody.
The canonical canvas exists because the GIME defines one visible window
shared by every mode it can produce. A CoCo 1 or 2 has no GIME and no such
unification. Those machines render into their own fixed, smaller buffer —
`video::FB_W` by `video::FB_H`, the 288×224 geometry from §7.2.2 — and they
do it as a whole-field snapshot at the end of the field rather than line by
line.

The split is visible in two places. The default framebuffer geometry is the
VDG's, set at
[`crates/coco-core/src/machine.rs:17-18`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine.rs#L17-L18),
and the two entry points in `machine/render.rs` guard each other's
territory: `render_scanline` begins by returning unless the machine is a
CoCo 3, and `render_field`
([`crates/coco-core/src/machine/render.rs:179-188`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L179-L188))
begins by returning *if* it is. Every CoCo 3 field, legacy or GIME-native,
is painted line by line and is already complete when the field wraps; every
CoCo 1/2 field is painted in one pass at the wrap.

This chapter's worked examples are CoCo 3, since that is this course's
machine. But two of the three tests in §7.9 run against the CoCo 1/2 path,
because that is where the direct, ROM-free renderer entry point lives — so
keep the split in mind when a test suddenly uses a framebuffer width the
preceding section did not.

---

## 7.4 The reveal: the BASIC prompt was never GIME-native

Now the fact this chapter is built around, and it deserves a moment of
attention because it inverts what most people who used the machine believe.

The green `OK` prompt a CoCo 3 puts on screen at power-up was drawn by the
GIME pretending to be the old MC6847 VDG chip from the CoCo 1 and 2. It was
not drawn by any of the GIME's own native video modes. A real CoCo 3 has no
MC6847 soldered to its board at all; on power-up, the ROM configures the
GIME to imitate one, in a 32-column-by-16-row text mode that is
bit-for-bit compatible with what CoCo 1/2 software expects to find in RAM.
The GIME's native 40- and 80-column text appears only once a program asks
for it with `WIDTH 40` or `WIDTH 80` — which, outside a handful of
applications that wanted the extra columns, rarely happened.

This is not folklore. It is a correction recorded in the project's own
design document, `DESIGN.md` §6, written after the original plan had
confidently assumed otherwise and then met a real ROM:

> **Correction (2026-07, verified against the real ROM):** the power-on
> BASIC prompt is drawn in the **VDG-compatible 32×16 alphanumeric text
> mode** (COCO bit set), *not* a GIME native text mode — native 40/80-col
> text only appears with `WIDTH 40/80`. So the first visible-prompt
> milestone required the VDG text path ahead of "native first."

Two consequences follow, and both shape this course. Practically, the
codebase's own "first visible pixel" milestone required building this
legacy-compatibility path *before* any native GIME video register mattered
at all — the ambition to "start with the GIME" collided with the fact that
the GIME's first job is impersonation. Pedagogically, the same ordering is
the right one: the legacy mode is simpler, it is the screen the reader
already recognizes, and understanding it makes next week's native register
file read as a contrast rather than as a wall. Hence Chapter 7 before Chapter 8.

There is a broader pattern here, and Chapter 1 named it while looking at the
SAM-compatibility strobes in the `$FFC0–$FFDF` range. Backward compatibility
is not a footnote in this machine; it is a structural commitment that
reaches all the way into what the hardware does on the very first field
after power-on. The CoCo 3 boots into a 1980 video mode because thousands of
programs — including, decisively, the BASIC ROM itself — assumed that mode
existed.

### 7.4.1 How the machine decides which path it's on

Everything above turns on a single bit, and the function that reads it is
the map of the entire video subsystem for Chapters 7 through 9. It is short
enough to read in full
([`crates/coco-core/src/machine/video_mode.rs:19`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/video_mode.rs#L19)):

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

Take the two machine families in turn. A CoCo 1 or 2 has no GIME to consult,
so it always runs this week's path and picks text against graphics from a
single PIA1 bit: `VDG_AG`, bit 7 of `$FF22`
([`crates/coco-core/src/video/graphics.rs:16`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/graphics.rs#L16)).
That bit is the MC6847's alphanumerics-versus-graphics pin, brought out to a
PIA output line because on those machines the *CPU* had to drive the video
chip's mode pins directly — there was no video register file to write to.
This is a good early illustration of how much of a CoCo's video
configuration lives in a general-purpose parallel port rather than in
anything that looks like a display controller, a theme Chapter 10 will develop
at length.

A CoCo 3 checks one more gate before it gets there. `INIT0`'s `COCO` bit —
bit 7 of the register at `$FF90`, defined as `pub const COCO: u8 = 0x80` in
[`crates/coco-core/src/gime.rs:43-60`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L43-L60)
— is the compatibility switch. When it is set, the machine ignores every
native GIME video register and falls into the *exact same* `VDG_AG` branch
the CoCo 1/2 uses. Structurally, that shared branch is the whole reveal: the
GIME pretending to be a VDG is implemented, in this codebase, by literally
running the VDG's decision. Only when `COCO` is clear does the GIME's own
`$FF98` `BP` bit — bit 7 again, this time selecting bitmapped graphics
against hi-res text
([`crates/coco-core/src/gime.rs:72-83`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime.rs#L72-L83))
— get consulted at all.

Stock CoCo 3 BASIC sets `COCO` at cold start and never clears it unless a
program asks for a wider screen — precisely the fact this section opened
with, now traced to the single `if` that implements it.

### 7.4.2 The screen byte, bit by bit

With the mode settled, the question becomes what the renderer reads. The
legacy text screen is 512 bytes — `COLS * ROWS`, thirty-two by sixteen,
declared at
[`crates/coco-core/src/video.rs:29-43`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video.rs#L29-L43)
— living in ordinary RAM at a base address the SAM's page register points
at. On a CoCo 3 that register is the GIME's SAM-compatibility page overlay,
and the base address it produces is simply the page number times 512
([`crates/coco-core/src/gime/sam_compat.rs:73`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/sam_compat.rs#L73)).
Color BASIC programs it to page 2, giving `$0400` — the address every CoCo
BASIC programmer memorized, the one that makes `POKE 1024,65` put a
character in the top-left corner of the screen.

Each of those 512 bytes is a *character cell*, and a single byte packs three
independent pieces of information. The decoder names them with three bit
masks
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

Bit 7 is the mode selector for that one cell, and everything else in the
byte depends on it. When bit 7 is clear the cell is plain text: bits 5
through 0 select one of 64 glyphs, and bit 6 swaps the cell's foreground and
background colors. When bit 7 is set, none of that applies — the byte
switches to an entirely different interpretation called semigraphics-4, and
§7.6 takes it apart.

The glyph codes are worth one paragraph on their own, because they are not
ASCII and the mismatch trips people up. The mapping lives in
`decode_alpha_char`
([`crates/coco-core/src/video/text.rs:37`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L37)),
and it has two halves: codes `$00` through `$1F` map to the characters `'@'`
through `'_'`, computed as `'@' + code`, and codes `$20` through `$3F` map to
a second block starting at space, computed as `' ' + (code - 0x20)`. Sixty-four
glyphs is what fits in six bits, and what the VDG's designers chose to spend
them on was the uppercase alphabet, a handful of symbols, the digits, and
punctuation. There are no lowercase letters in that space at all — a
limitation the machine worked around in two different ways: the inverse bit,
and §7.5.3's genuinely strange true-lowercase mode.

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
> address bus is sixteen bits wide, full stop, and `Bus::read` (Chapter 1)
> takes a `u16` for exactly that reason — `wrapping_add` on `i as u16` is
> the same "must wrap at `$FFFF` → `$0000`" discipline you saw in Chapter 1's
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
>
> There is a second thing worth noticing in that loop, unrelated to types.
> The renderer fetches its screen bytes through `self.bus.read`, the same
> `Bus` trait method the CPU uses, rather than by indexing RAM directly.
> That is a fidelity decision with teeth: reading through the bus means the
> legacy video path sees exactly what the CPU would see at those logical
> addresses, MMU translation included. Next week's GIME-native path does the
> opposite, reading *physical* RAM with the MMU bypassed, because that is
> what the real GIME does in its native modes. Two data paths, because the
> hardware has two.

### 7.4.3 Decoding a real screen byte, end to end

Abstract bit layouts become concrete the moment you run one byte through
them by hand, so take the byte `$41`.

Split it into bits: `0100 0001`. Bit 7 is 0, so this is an alphanumeric
cell rather than semigraphics. Bit 6 is 1, so the cell is inverse video.
Bits 5 through 0 are `000001`, glyph code 1, which `decode_alpha_char` maps
to `'@' + 1`, the letter `'A'`. So `$41` is the letter 'A', drawn in inverse
video.

Notice something satisfying at this point: `$41` is *also* the ASCII code
for capital `'A'`. That is not a coincidence dressed up as one; it falls
straight out of the arithmetic. For any code `c` below `$20`, the
non-inverse decoded character is `'@' + c`, which is `$40 + c`. The inverse
*raw byte* for that same code is `c | $40`, and since `c`'s top two bits are
clear by construction, that expression equals `$40 + c` as well — the
identical value. For codes under `$20`, "the ASCII value of the decoded
character" and "the raw screen byte with the inverse bit forced on" are
literally the same number. It is a cute artifact of the VDG's code space
overlapping ASCII's uppercase block, not a rule anybody needs to memorize.
But it is why `$41` reads naturally as both "screen byte" and "the letter A"
at once, and why so much CoCo code gets away with treating the two as
interchangeable.

Now the part that matters for the black-on-green reveal, and it is where the
chapter's arithmetic turns into something visible. The stock Color BASIC ROM
fills the entire 512-byte text screen with the *inverse* form of every
character it prints. Bit 6 is set on every single screen byte, always — not
just for a blinking cursor, not just for a highlighted word, but as the
normal representation of ordinary text. `DESIGN.md` §6 records this as
verified against the real ROM and against a MAME screenshot.

Feed that fact into `resolve_alpha_cell`
([`crates/coco-core/src/video/text.rs:88-116`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L88-L116)),
the function every text pixel in this codebase routes through:

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

The whole mechanism is that penultimate line. When `inverse` is true, the
function returns the pair swapped, so the *background* color paints where
the glyph's set bits are and the *foreground* color paints everywhere
else. Every character on the stock boot screen therefore draws
its strokes in the background palette color and its surroundings in the
foreground palette color — backwards from what the words "foreground" and
"background" suggest in every other context they appear in.

Combine that with where those two colors actually come from, which §7.7
takes apart in detail: the foreground register resolves to green and the
background register to black. Swap them on every cell, all the time, and the
result is what a CoCo 3 shows at cold boot — black letters on a field of
solid green, with the green being what "foreground" resolves to and the
black letters coming from "background," inverted onto the page by one bit
that nobody thinks about.

This is the single strangest fact this chapter teaches, and it is exactly
correct. Boot the real ROM, or read
[`tests/coco1_boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/coco1_boot.rs)
from Chapter 6, and check for yourself that every non-blank screen byte has bit
6 set. There is also a design lesson buried in it, and it is one Chapter 1
raised in a different context: the renderer does not know any of this. It
has no notion of "the boot screen" or "how BASIC likes its text." It swaps
two colors when a bit is set, and the visual character of an entire
operating environment is an emergent property of the ROM's choice to set
that bit everywhere.

### 7.4.4 Which thirty-two bytes: the row cursor

One question remains before the glyphs themselves. `paint_legacy_scanline`
is handed a canvas row number between 0 and 239. How does it know which
thirty-two of the screen's 512 bytes belong to that row, and which of the
twelve rows *within* a character cell it is currently painting?

It does not compute them. It carries them, in the same `FieldScan` structure
that §7.2.3 introduced as the per-field latch. The two fields that matter
are `row_base`, the address of the current character row's first byte, and
`line_in_row`, the scanline index within that row. Both are seeded once per
field
([`crates/coco-core/src/gime_video.rs:146-158`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime_video.rs#L146-L158)):
a legacy field starts with `row_base` at the SAM-compatibility display base
and `line_in_row` at zero.

They are then advanced at the bottom of every active line
([`crates/coco-core/src/machine/render.rs:164-170`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L164-L170)):

```rust
// Advance the shared vertical counter (MAME `record_full_body_scanline`).
let scan = self.field_scan.as_mut().expect("legacy field latched");
scan.line_in_row += 1;
if scan.line_in_row >= lines_per_row {
    scan.line_in_row = 0;
    scan.row_base += row_bytes;
}
```

For legacy text, `lines_per_row` is `CELL_H` and `row_bytes` is `COLS`
([`crates/coco-core/src/machine/render.rs:112-118`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L112-L118)),
so the cursor advances by one glyph row for twelve consecutive scanlines and
then steps thirty-two bytes further into RAM. Twelve scanlines per character
row, sixteen character rows, 192 lines of active body — the numbers close
exactly, which is the arithmetic §7.3 flagged when it noticed that
`vertical_window(0)` returns a 192-row body.

There is a real design decision hiding in that counter, and it is the
opposite of what most people would write first. The obvious implementation
computes everything from the row number: character row is `(row - top) / 12`,
glyph row is `(row - top) % 12`, and the byte address is `base + char_row *
32`. That works, for this mode, today. It stops working the moment a
program changes the display base or the lines-per-row setting partway down
the screen, because a computed-from-scratch address answers the question
"where would row N be if the registers had always held their current values"
rather than "where is row N, given where row N−1 actually was." A running
cursor answers the second question, which is the one the hardware answers.
Chapter 9's mid-frame split effects live entirely in that difference.

With the mode chosen, the byte decoded, and the row located, exactly one
thing is left: turning a six-bit glyph code into twelve rows of pixels.

---

## 7.5 Fonts as data

Nothing about how these glyphs are drawn is special-cased in the renderer.
`blit_cell` from §7.2.2 does not know or care what letter it is drawing; it
receives twelve bytes and paints their bits. All of the "font" knowledge in
this codebase lives in plain `const` arrays, and this section is about
reading those arrays the way a human would — because sooner or later a glyph
will come out wrong and the fastest way to find out why is to eyeball the
data.

### 7.5.1 The MC6847's internal ROM, as a Rust array

`MC6847_FONT`
([`crates/coco-core/src/font6847.rs:37`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font6847.rs#L37))
has 64 entries, one per glyph code, and each entry is a `[u8; 12]`: twelve
bytes, one per raster row of the 8×12 character cell from §7.2.2. That is
the entire data structure. Here is the entry for `'A'`, glyph code `$01`,
the second row of the table:

```rust
[0x00, 0x00, 0x00, 0x08, 0x14, 0x22, 0x22, 0x3E, 0x22, 0x22, 0x00, 0x00,], // A
```

Each byte's eight bits correspond to the cell's eight columns,
most-significant bit first — the same `bits & (0x80 >> cx)` test that
`blit_cell` performs. Decoding by hand is mechanical: write out each
byte in binary, put a `#` where a bit is set and a `.` where it is clear,
and stack the twelve rows.

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

There is the letter: an apex at row 3, the two diagonals opening out through
rows 4 and 5, vertical strokes down both sides, the crossbar at row 7, and
legs continuing to row 9. Fifteen minutes with a pencil and this table will
teach more about how the machine's text looked than any amount of prose,
and it is genuinely the debugging technique of choice when a rendered glyph
comes out wrong — compare the bit grid you expect against the bit grid the
renderer produced, and the mismatch names the bug.

Two structural observations fall out of the decode. First, rows 0 through 2
and rows 10 and 11 are blank in *every* entry of `MC6847_FONT`, so all
sixty-four glyphs live in rows 3 through 9. The module documentation states
it slightly more loosely, from the source's perspective: the plain MC6847
"occupies rows 3-10 (top 2 and bottom 2 rows always blank — MAME
`vdg_fontdata8x12`)." Six or seven rows of actual strokes, centered in a
twelve-row cell with generous padding above and below, is what gives CoCo
text its line spacing — the reason a 32×16 screen never looks cramped
despite being only sixteen rows tall. Second, look at the horizontal extent
of the 'A': the widest row, `0x3E`, is `0011 1110`, which leaves the two
leftmost columns and the rightmost column clear. That is the inter-character
spacing, baked into the glyph data itself. There is no gap between cells on
this screen; adjacent cells touch exactly, and the space between letters is
blank columns inside each glyph.

> **Rust corner: a font is just a `const`, and that's the whole design.**
> `MC6847_FONT` is declared `pub const MC6847_FONT: [[u8; 12]; 64] = [...]`
> — a nested array literal, not a lazily-built value and not a loaded
> resource. Two more common-looking alternatives were available, and both
> are worse here. `include_bytes!("font.bin")` would embed the identical
> bytes but as an opaque `&'static [u8; 768]` with no structure — every
> glyph lookup would need hand-rolled index math (`bytes[code * 12 + row]`)
> instead of `font[code][row]`, reintroducing exactly the kind of
> off-by-one risk Chapter 3's indexed-postbyte decoder went to such lengths to
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
>
> The type is doing real work too. `[[u8; 12]; 64]` says "sixty-four
> glyphs of twelve rows" in the type system, so an out-of-range glyph index
> is a bounds check rather than a silent read into the neighboring glyph,
> and a function that takes a `&'static [u8; CELL_H]` — as
> `resolve_alpha_cell` returns — cannot be handed a slice of the wrong
> length. Compare that with the `&'static [u8]` a flat byte blob would give
> you, where every length invariant lives in a comment.

### 7.5.2 Two more fonts, same shape, different rows

The plain MC6847 is one of three character generators this codebase can
drive, and comparing them is the fastest way to see how much of a "font" is
convention rather than content.

`MC6847T1_FONT`
([`font6847.rs:112`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font6847.rs#L112))
has 96 entries. The first 64 are the same uppercase and symbol glyphs at the
same codes with the same meanings; the additional 32, at indices 64 through
95, exist only on the newer MC6847T1 chip and hold true lowercase letters,
reachable through the special path in §7.5.3. Compare the T1's 'A'
against the plain chip's:

```rust
// MC6847_FONT[1]  — rows 3-10
[0x00, 0x00, 0x00, 0x08, 0x14, 0x22, 0x22, 0x3E, 0x22, 0x22, 0x00, 0x00,]
// MC6847T1_FONT[1] — rows 1-8
[0x00, 0x08, 0x14, 0x22, 0x22, 0x3E, 0x22, 0x22, 0x00, 0x00, 0x00, 0x00,]
```

The same six strokes, shifted two rows higher in the cell, byte for byte
identical otherwise. That shift is not cosmetic and it is not a mistake in
the mask. The T1 needed the bottom rows free for lowercase *descenders* —
the tails on 'g', 'j', 'p', 'q', and 'y' that hang below the baseline — and
the only way to make room in a fixed twelve-row cell was to move every
uppercase glyph up. Adding lowercase to the chip therefore changed the
vertical position of every character already in it, which is exactly the
kind of consequence that makes hardware revisions interesting to emulate:
the visible difference between a CoCo 1 and a CoCo 2B is not a feature
anybody advertised; it is that all the letters sit two pixels higher.

`GIME_LOWRES_FONT`
([`crates/coco-core/src/font_gime.rs:162`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font_gime.rs#L162))
is the third font, and it is the one a real CoCo 3 actually uses for this
mode — because, as §7.4 established, there is no VDG chip on the board to
supply a font ROM. It follows the T1's layout conventions, with glyphs in
rows 1 through 8 and true lowercase in entries 64 through 95, but the
letterforms themselves are the GIME's own:

```rust
[0x00, 0x10, 0x28, 0x44, 0x44, 0x7C, 0x44, 0x44, 0x00, 0x00, 0x00, 0x00], // A  (GIME_LOWRES_FONT[1])
```

Decode that crossbar row against §7.5.1's grid and the difference is
immediate: `0x7C` is `0111 1100`, occupying columns 1 through 5, where the
plain chip's `0x3E` occupied columns 2 through 6. Same five-column stroke,
one column further left in the cell — enough to make a screenful of GIME
text sit visibly off from a screenful of MC6847 text even though the
letterforms are recognizably the same alphabet. The source is careful to
record that this table is genuinely a third font, not a transformation of
the second one:
the doc comment notes it is "NOT derivable from the T1 table by
bit-shifting (verified: e.g. entry 0 '@' row 6 is genuinely different, not a
shifted copy)."

Three real character-generator ROMs, three slightly different letterforms,
one shared array shape — twelve rows of eight bits, glyphs at 0–63,
lowercase at 64–95. That shared shape is exactly why the next section's
`AlphaGenerator` enum can treat all three uniformly, and why swapping one
for another costs the renderer nothing at all.

### 7.5.3 The T1's sneaky true-lowercase rule

Neither the plain MC6847 nor the T1 in its *normal* mode has real lowercase.
Codes `$00` through `$1F` always draw the corresponding uppercase letter,
and setting bit 6 merely inverts that letter's colors, as §7.4.3 showed.
The T1 chip has a second, genuinely different mode, called *true lowercase*,
which redirects those same low codes to an entirely different set of glyphs:
actual lowercase letterforms with descenders, rather than inverted uppercase
ones. The CoCo 3's GIME compat generator shares that logic.

The three generators are named by one enum
([`crates/coco-core/src/video/text.rs:54-62`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L54-L62)):

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

Notice what that enum is *not*. It is not "which CoCo is this" — that is
`MachineVariant`, and it is decided at construction. It is "which character
generator is currently driving the text screen," which is a different
question with a different answer on a CoCo 3, where the machine variant says
CoCo 3 and the generator says GIME because there is no VDG in the box. The
source is explicit about keeping the two ideas apart, and the distinction is
what makes §7.9.3's test possible to state at all.

Whether true lowercase engages is decided by one expression, from
`resolve_alpha_cell`
([`text.rs:97-99`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L97-L99)):

```rust
let lowercase_capable = matches!(generator, AlphaGenerator::MC6847T1 | AlphaGenerator::GIME);
let true_lowercase =
    lowercase_capable && !inverse && ff22 & VDG_GM0_INTEXT != 0 && glyph_code < 0x20;
```

Four conditions, every one of them required, and each corresponds to
something physical:

1. **The chip must be capable at all.** A plain `MC6847` never takes this
   branch, full stop, because the `matches!` excludes it. Code `$01` on a
   CoCo 1 is always 'A', inverted or not, and no PIA bit changes that.
2. **This cell's own inverse bit must be clear.** An inverse `$41` never
   goes lowercase, even on a T1 with the mode enabled globally. Inverse and
   lowercase are competing reinterpretations of the same bits, and inverse
   wins.
3. **PIA1 `$FF22` bit 4 must be set.** This is a *global* switch affecting
   the whole screen, not a per-cell attribute. The constant is named
   `VDG_GM0_INTEXT`
   ([`crates/coco-core/src/video/graphics.rs:26`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/graphics.rs#L26))
   because the physical pin has two different meanings on the two chips: on
   the plain MC6847 it selects an external character ROM, which this
   emulator does not model, and on the T1 the same pin is wired as GM0,
   which in alphanumeric mode enables lowercase. The name records both
   histories and the doc comment explains which one has an observable effect
   here.
4. **The glyph code must be under `$20`.** Codes `$20` through `$3F` are
   space, digits, and punctuation, which have no lowercase forms to switch
   to; real hardware leaves them untouched, and the emulator does too.

When all four hold, the glyph comes from a completely different region of
the same array
([`crates/coco-core/src/video/text.rs:100-107`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L100-L107)):

```rust
if true_lowercase {
    let glyph: &[u8; CELL_H] = match generator {
        AlphaGenerator::MC6847T1 => &MC6847T1_FONT[0x40 + glyph_code as usize],
        AlphaGenerator::GIME => &GIME_LOWRES_FONT[0x40 + glyph_code as usize],
        AlphaGenerator::MC6847 => unreachable!("Mc6847 is never lowercase_capable"),
    };
    (bg, fg, glyph)
} else {
```

Two details in eight lines deserve attention. The index is `0x40 +
glyph_code` rather than `glyph_code % 64`, which is the arithmetic that
reaches entries 64 through 95 — the lowercase section §7.5.2 described. And
the returned color pair is `(bg, fg)`, swapped, which is the same swap the
`inverse` branch performs even though condition 2 above required `inverse`
to be *false*. True-lowercase text is drawn color-inverted relative to what
anyone would naively expect.

That is not a bug and it is not a simplification. It is a real hardware
behavior, reproduced deliberately, matching MAME's own `mc6847.cpp` — and
the doc comment on `resolve_alpha_cell` explains the equivalence, noting
that MAME expresses the same result as drawing `raw_glyph ^ 0xFF`
non-inverted, which is arithmetically the same thing in the inverse-toggle
style this module already uses. Faithfully reproducing a quirk you would
never design on purpose, and writing down why the code looks wrong, is a
large fraction of what emulator work actually is.

The `unreachable!` arm is worth one closing note as Rust craft. The plain
MC6847 cannot get here, because `lowercase_capable` excluded it two lines
earlier — but the compiler cannot know that, since it has no way to connect
a boolean computed by `matches!` with the `match` arms below. Rather than
inventing a plausible-looking fallback that would silently paper over a
future refactor, the code asserts the invariant loudly and names it in the
panic message. If somebody later widens `lowercase_capable`, the test suite
finds out immediately.

---

## 7.6 Semigraphics-4: the blocky graphics of every one-liner

Set bit 7 of a screen byte and the entire interpretation changes. The cell
stops being a letter and becomes a 2×2 grid of colored blocks, each
independently on or off. This is *semigraphics-4*, universally shortened to
SG4, and it is the reason a machine whose text screen holds only 512 bytes
could draw pictures without leaving text mode at all.

Do the arithmetic on what that buys. The screen is thirty-two cells across
and sixteen down, and each cell holds a 2×2 sub-grid, so the effective
resolution is `32 * 2 = 64` blocks wide by `16 * 2 = 32` blocks tall. SG4
gives the CoCo a crude 64×32 graphics mode living entirely inside text-mode
RAM, requiring no mode switch, no dedicated graphics page, and no PMODE
setup. A BASIC program could draw with it using nothing but `PRINT
CHR$(128+n)`, and a great many did: every blocky mountain range, every
blocky invader, and every character-cell maze in the type-in listings of the
period was semigraphics-4. It is the visual signature of the platform's
first few years.

The bit layout reuses the bits that alphanumeric mode spent on the glyph
code, and the constants name each one
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

Bits 6 through 4 select one of palette registers 0 through 7 as the lit
color for *this whole cell*. That is worth emphasizing because it is the
mode's central limitation: SG4 is one color per cell, not one color per
block, so the four quadrants of a cell are either that color or unlit, with
no way to make the upper-left red and the lower-right blue. Bits 3 through 0
are the on/off pattern for the four quadrants, one bit each. An unlit block
always draws palette register 8, which resolves to black in
CoCo-compatible mode.

The paint loop is a direct transcription of that diagram
([`crates/coco-core/src/video/text.rs:167-182`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L167-L182)):

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

The two booleans do all the work. `bottom` compares the current scanline
within the cell against the cell's vertical midpoint, `CELL_H / 2`, which is
6. `right` compares the current column against the horizontal midpoint,
`CELL_W / 2`, which is 4. Together they name one of four quadrants, the
`match` turns that quadrant into the corresponding bit mask, and one `&`
against the screen byte decides lit or unlit. There is no font lookup
anywhere in this branch — an SG4 cell never touches `MC6847_FONT` or either
of its siblings, because there is no glyph involved, only four solid
rectangles.

That branch lives in the per-scanline painter, which draws one glyph row at
a time and therefore knows `glyph_row` but not `cy`. A near-identical loop
lives in the whole-field painter, `blit_semigraphics4`
([`text.rs:238`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L238)),
which draws all twelve rows of a cell in one call and therefore loops `cy`
itself. The duplication is real and it is worth being honest about: two
implementations of the same 2×2 decode exist because the CoCo 3 renders line
by line and the CoCo 1/2 renders field by field, as §7.3's caveat explained.
What the two paths *do* share is `resolve_alpha_cell`, the harder and more
error-prone half, which is exactly the right place to draw the line — the
part with four interacting conditions and a color-swap quirk is written
once, and the part that is two nested loops is written twice.

Now decode a byte that tests this section and §7.4 together: `$C1`.
In binary that is `1100 0001`. Bit 7 is **1**, so before looking at anything
else, this cell is semigraphics. Bit 6's meaning as "inverse" does
not apply, because that meaning exists only in the `else` branch of the code
above. It is tempting to read `$C1` as "`$41`, which was inverse 'A', plus
something extra" and expect a letter to appear — that instinct is exactly
the trap this byte sets, and it is the most common way to misread a CoCo
screen dump. Decode it correctly, as SG4:

- Color: `(0xC1 >> 4) & 0x07` is `0xC & 0x07`, which is `0b1100 & 0b0111` =
  `0b0100` = palette register **4**.
- Pattern: the low nibble is `0x1` = `0001`, so only `SG4_LOWER_RIGHT` is
  set.

So `$C1` renders as three unlit quadrants — upper-left, upper-right, and
lower-left, all in palette register 8's black — and one lit quadrant in the
lower right, in whatever palette register 4 currently resolves to. One
small lit square in the corner of an otherwise-black cell, and nothing at
all like a letter.

That raises the obvious question, and it is the subject of the next
section: what does "palette register 4" actually look like?

---

## 7.7 Where the colors actually come from

Every color named so far has been an index rather than a color. The text
foreground and background, SG4's eight selectable colors, the unlit block
color, the border — all of them are *palette register indices*, and
something still has to turn "palette register 4" into four bytes of RGBA
that a display can show.

On the CoCo 3, that something is the GIME, even while the GIME is busy
imitating a chip that never had programmable palette registers at all. This
is one of the places where the compatibility fiction is visibly a fiction:
a real MC6847 produced fixed analog colors determined by its own circuitry,
and a program could no more reprogram them than it could resolder the board.
The GIME's compat mode reproduces the *structure* of the VDG's color scheme
— which index means what — while sourcing the actual values from registers
the ROM programmed at boot.

### Which registers legacy text reads

Two constants name the registers this mode uses
([`crates/coco-core/src/video.rs:47-48`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video.rs#L47-L48)):

```rust
pub const TEXT_BG_INDEX: usize = 12;
pub const TEXT_FG_INDEX: usize = 13;
```

Registers 12 and 13, and not by accident: they match the MC6847's own
`color_base_0` and `color_base_1` numbering, so a VDG-compatibility mode
reading registers 12 and 13 is doing precisely what a real MC6847 paired
with its analog color circuitry did, with the GIME standing in for the
analog part. The sixteen-entry index layout is shared across the
whole legacy path: registers 0 through 7 are the eight SG4 colors, 8
through 11 the two-color graphics pairs, and 12 through 15 the two
alphanumeric color sets.

The design point the module documentation insists on is that these colors
are *data*, programmed by the ROM at boot, and not hardcoded anywhere in the
renderer. `legacy_palette`
([`crates/coco-core/src/machine/video_mode.rs:62`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/video_mode.rs#L62))
proves it: on a CoCo 3 it snapshots all sixteen live palette registers
through `GIME::color` and never special-cases index 12 or 13 anywhere.

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

The function is called once per scanline, which is what makes a palette
write take visible effect on the very next line rather than the next field —
the mechanism behind a whole family of raster tricks that Chapter 9 will chase
properly.

### The CoCo 1/2 side of the same interface

The second `match` arm above is the other half of the story, and it is a
small lesson in how to model two machines that differ in exactly one respect. A
CoCo 1 or 2 has no palette registers whatsoever; its colors are hardwired
analog levels in the MC6847. So instead of snapshotting registers, that path
resolves a fixed table
([`crates/coco-core/src/video.rs:66-83`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video.rs#L66-L83)),
whose last eight entries show the shared index layout clearly:

```rust
    [0x26, 0x30, 0x16, 0xFF], // 8  BLACK
    [0x30, 0xd2, 0x00, 0xFF], // 9  GREEN
    [0x26, 0x30, 0x16, 0xFF], // 10 BLACK
    [0xbf, 0xc8, 0xad, 0xFF], // 11 BUFF
    [0x00, 0x7c, 0x00, 0xFF], // 12 ALPHANUMERIC DARK GREEN
    [0x30, 0xd2, 0x00, 0xFF], // 13 ALPHANUMERIC BRIGHT GREEN
    [0x6b, 0x27, 0x00, 0xFF], // 14 ALPHANUMERIC DARK ORANGE
    [0xff, 0xb7, 0x00, 0xFF], // 15 ALPHANUMERIC BRIGHT ORANGE
```

Those are not idealized values. The module comment records that they
reproduce MAME's measured `mc6847.cpp` palette, which is why entry 8's
"black" is `[0x26, 0x30, 0x16]` — a dark, slightly green gray rather than
true black, because that is what the chip actually put on a screen. And
notice that entries 0 through 11 are exactly the values the CoCo 3 ROM
programs into its palette registers at cold start, which is the reason the
two machines can share every other piece of the legacy renderer.

The `ColorSource` enum that dispatches between them
([`crates/coco-core/src/video.rs:116-143`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video.rs#L116-L143))
carries one asymmetry worth understanding, because it explains a `css`
parameter that has been threading through every signature in this chapter.
The `$FF22` CSS bit selects the orange alphanumeric color set instead of
the green one. On a CoCo 1/2 that is a real hardware switch with no register
behind it, so `resolve` implements it by copying entries 14 and 15 over
entries 12 and 13 — meaning every caller can read `TEXT_BG_INDEX` and
`TEXT_FG_INDEX` unconditionally and never think about CSS again. On a CoCo 3
the GIME's registers already hold whatever the ROM programmed, so `css` is
ignored. One enum, two behaviors, and a single index convention that
survives both.

### From six bits to twenty-four

Register-to-RGBA conversion itself is `GIME::rgb_color`
([`crates/coco-core/src/gime/palette.rs:56-62`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs#L56-L62)),
and it is small enough to read in one breath:

```rust
pub fn rgb_color(value: u8) -> [u8; 4] {
    let chan = |hi_bit: u8, lo_bit: u8| {
        let v = ((value >> hi_bit) & 1) << 1 | ((value >> lo_bit) & 1);
        v * 0x55
    };
    [chan(5, 2), chan(4, 1), chan(3, 0), 0xFF]
}
```

A GIME palette register holds six meaningful bits, two per color channel,
and the layout is the one the documentation calls `RGBrgb`: the three high
bits are the more significant bit of red, green, and blue in that order, and
the three low bits are the less significant bit of each. So a channel's two
bits are not adjacent — they sit three positions apart, which is exactly why
`chan` takes two bit positions instead of a shift and a mask.

The closure does three things in one line. It pulls the high bit down to
position 1, pulls the low bit down to position 0, and combines them into a
two-bit number from 0 to 3. Then it multiplies by `0x55`, which is 85, to
scale that range onto a full byte: the only four values a two-bit channel
can produce are 0, 85, 170, and 255, evenly spaced across 0 to 255, with the
maximum landing exactly on `0xFF` rather than one short of it. Choosing 85
rather than, say, 64 is what makes full-intensity mean genuinely full.

Walk the exact value that produces this chapter's green, `$12`, which is
`0b010010`:

```
value = 0b01 0010
bit:     5432 10

R = chan(5, 2): bit5=0, bit2=0 → v = 0b00 = 0 → R = 0
G = chan(4, 1): bit4=1, bit1=1 → v = 0b11 = 3 → G = 3 * 0x55 = 0xFF
B = chan(3, 0): bit3=0, bit0=0 → v = 0b00 = 0 → B = 0
```

`rgb_color(0x12)` is `[0x00, 0xFF, 0x00, 0xFF]` — pure green, `#00FF00`.
`$12` is the value the codebase names `BORDER6_GREEN`
([`crates/coco-core/src/video/text.rs:126`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L126)),
and it is used for the border of legacy *graphics* modes. The plain-text
border, which is this week's mode, resolves `BORDER6_BLACK = 0x00` instead,
which by the same arithmetic is trivially `[0, 0, 0, 0xFF]` — confirming
`DESIGN.md`'s claim that the boot screen has a black border around its green
picture.

One convenient consequence of that arithmetic: a palette register that has
been reset and never programmed holds `0x00`, and `rgb_color(0x00)` is
likewise pure black. That is why several of this chapter's tests, §7.9.3's
in particular, can leave a palette entry untouched and rely on it reading as
black rather than as garbage. A zeroed register is a meaningful color, not
an uninitialized one.

### The border tells you what mode you are in

`legacy_border_value`
([`text.rs:136-145`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs#L136-L145))
is worth reading once for what it reveals about the machine's visual design
language. It examines three bits of `$FF22` and returns one of four
six-bit color values: graphics modes border green, or white when CSS is
set; one particular text variant, with GM2 set and GM1 clear, borders green
or orange; and every other text or semigraphics mode borders black.

Real hardware, in other words, color-codes the border by video mode. That
is a detail no photograph of "a green CoCo screen" would ever tell you, and
one that only becomes visible when you decode the register logic rather than
eyeballing a screenshot. It is also the sort of thing that makes an emulator
feel wrong in a way users struggle to articulate: get the border rule
backwards and every screenshot is subtly, unnameably off.

---

## 7.8 The lab bench: PPM files, no GPU required

Everything in this chapter so far has been read rather than run. This
section is about closing that loop cheaply, because "render it and look at
it" is the single most effective debugging technique available in graphics
work, and it should not require a window.

The `examples/` directory of `coco-core` doubles as a lab bench for exactly
that reason. That is more than a convenience: a course meant to be
followed on a headless machine, and a repository whose tests run in CI, both
need a way to produce and inspect an image without a display server, a
window manager, or a GPU driver. The mechanism is almost insultingly simple.
It is *PPM*, the Portable Pixmap format: a plain-text header followed by a
raw binary body, short enough to write by hand in five lines.

```rust
let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
for px in m.framebuffer.chunks_exact(4) {
    ppm.extend_from_slice(&px[..3]);  // RGB, drop alpha
}
std::fs::write(path, ppm).unwrap();
```

The header is four fields: `P6` declaring binary RGB, then width, height,
and the maximum channel value, 255. The body is three raw bytes per pixel
with no compression and no metadata. The one transformation worth noting is
the `&px[..3]`, which drops the framebuffer's alpha byte — PPM has no alpha
channel, and since every pixel in this emulator is opaque by construction,
nothing is lost. Any image viewer reads the result immediately, as do
ImageMagick and `ffmpeg -i frame.ppm frame.png` if a PNG is wanted.

Two examples in this crate use exactly this pattern.
[`crates/coco-core/examples/demo_frames.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/demo_frames.rs)
runs an injected demo binary and dumps periodic frames, and
[`crates/coco-core/examples/vdg_font_probe.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/vdg_font_probe.rs)
boots all three text generators — a CoCo 1 with the plain MC6847, a CoCo 2
with the T1, and a CoCo 3 with the GIME's own font — and writes one PPM
each, so that §7.5.2's three letterforms can be compared side by side
instead of taken on faith.

Both of them, however, need real ROM images to boot: `demo_frames.rs` reads
`coco3.rom` unconditionally, and `vdg_font_probe.rs` wants `extbas11.rom`,
`bas12.rom`, and `coco3.rom` besides. Those files are copyrighted, so the
repository does not ship them; the directory that holds them is git-ignored,
exactly as Chapter 1's practical notes described, and a fresh clone will not
have it. That is not a gap in the course. It is the intended boundary
between "code the emulator core," which works everywhere with nothing but
the repository, and "trace-diff against a real boot," which needs assets the
project deliberately does not distribute.

The good news is that everything this chapter needs to *verify* is
ROM-free. `video::render_text` and `paint_legacy_text_line` take
already-decoded screen bytes and an already-resolved palette as plain
arguments, so exercising them requires no CPU execution and no ROM at all.
A dozen lines will dump a hand-built screen to a PPM, reusing exactly the
fixtures §7.9 uses:

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

Read what that program does *not* do. It never constructs a `Machine`. It
never executes a 6809 instruction. It never touches a bus, a PIA, or an
interrupt. It allocates a screen of spaces, drops one byte into the corner,
resolves two colors by hand, and calls the pure decode function that this
whole chapter has been reading. The output is a 288×224 image, black
everywhere except one cell in the top-left corner: the inverse 'A' fills
that cell with the foreground green and draws the letter's strokes in the
background black, exactly as §7.4.3 predicted.

That is the real lab bench for this week, and it is the shape one of the
exercises below asks you to extend. It is also a good habit to carry
forward: when a renderer misbehaves, the first move is to call it directly
with synthetic input, because a wrong pixel with a `Machine` in the room has
a hundred possible causes and a wrong pixel without one has about three.

---

## 7.9 Three tests, walked

The test suites are, per the syllabus's own framing, "the textbook exercises
with answers." Three of them are worth stepping through slowly, because
between them they cover the three distinct things this chapter claimed:
that the geometry is right, that the bit decode is right, and that the CoCo
3 draws its own font rather than a borrowed one.

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

The setup is a screen filled entirely with spaces, so every cell is blank
and the active area should come out uniformly background-colored. The two
extreme corners of the framebuffer, `(0, 0)` and the bottom-right, should be
border. That is the simplest possible statement of §7.2.2's geometry:
`BORDER` pixels of margin on every side of a `COLS * CELL_W` by `ROWS *
CELL_H` active rectangle, nothing more.

The one detail worth pointing out is the palette this suite uses. It is not
green and black; it is red for foreground, green for background, and blue
for the border
([`crates/coco-core/tests/render.rs:20-24`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render.rs#L20-L24)),
chosen as *sentinels* rather than for realism. Three visually absurd
primaries mean an assertion failure names exactly which of the three roles
was painted in the wrong place, which a realistic palette of two similar
greens could never do. When you write your own renderer tests, steal this
technique: pick colors that could not possibly be confused for one another,
and let the failure message do the diagnosis.

If the border-versus-active arithmetic is ever broken by a refactor, this is
the tripwire that catches it. It checks the literal pixel at the frame's
corners and the literal pixels of the first cell, which is about as close to
"the formula from §7.2.2, executed" as a test can get.

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

Decode the constant the same way §7.6 taught. `0x80` sets the semigraphics
bit. `3 << 4` puts color index 3 into bits 6 through 4. And `0b1001` sets
`SG4_UPPER_LEFT` (`0x08`) and `SG4_LOWER_RIGHT` (`0x01`), leaving the other
two quadrants clear. Notice that the test writes the byte as an *expression*
built from the same shifts the decoder uses, rather than as the literal
`0xB9` it evaluates to. That is deliberate and it is worth copying: a test
that says `0x80 | (3 << 4) | 0b1001` documents its own intent, while a test
that says `0xB9` requires the reader to do §7.6's decode by hand before the
assertions mean anything.

The assertions then sample exactly one pixel from each of the four
quadrants, at the quadrant's own top-left corner, and check it against the
lit or unlit color predicted from the pattern bits. Four pixels is a
minimal but complete proof that the `(bottom, right)` match in the painter
puts each pattern bit in the quadrant the bit-layout diagram says it belongs
in, and nowhere else. A transposition bug — say, swapping upper-right and
lower-left — would leave the cell looking plausibly blocky and fail two of
these four assertions immediately.

### 7.9.3 A CoCo 3 specifically: `coco3_compat_text_draws_gime_font_not_either_vdg_font`

([`crates/coco-core/tests/render_coco12/coco3_compat_text.rs:60`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_coco12/coco3_compat_text.rs#L60))

This is the payoff test for §7.4's entire claim that no VDG chip exists on a
CoCo 3. Unlike the two above, it boots a real `Machine` rather than calling
`render_text` directly: it forces legacy mode through `INIT0`'s `COCO` bit,
writes one character to the screen, runs a full field through Chapter 6's
timing loop, and then checks which of three possible glyph shapes came out
the other end.

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

The fixture repays attention line by line, because it is a template for
every "full machine, no ROM" test in this book. The ROM is a synthetic
all-zero 32K array rather than a real image, which means the reset vector
resolves to `$0000`; the two bytes written there are `$20 $FE`, which is
`BRA *`, a branch to itself. The CPU therefore spends the entire field
looping on one instruction and never executes anything with side effects,
while the scanline clock runs normally and the renderer does its work.
"Park the CPU, poke the hardware, run a field" is a pattern worth
internalizing: it gives you the complete field-timed render path with none
of a real ROM's unpredictability, and the test is deterministic to the
pixel.

The palette setup is equally deliberate. Only the foreground register is
programmed, to white; the background register is left at its zeroed default,
which §7.7 established resolves to pure black. Two maximally distinct
colors mean the sampling helper can classify every pixel in the cell as
"on" or "off" with no ambiguity.

That helper is the second thing worth noticing. It is
`sample_cell_canonical`
([`crates/coco-core/tests/render_coco12/common.rs:113`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_coco12/common.rs#L113)),
not the plain `sample_cell` the CoCo 1/2 tests use. It reads pixels off the
CoCo 3's canonical 640×240 canvas from §7.3, which means it has to account
for the ×2 horizontal scale — it samples the left pixel of each doubled
pair — and for the 64-pixel side border and the 25-row top border that
`vertical_window(0)` puts above the body. Same underlying idea as its sibling,
different geometry constants, which is exactly the split §7.3's caveat
flagged. Its counterpart `glyph_bits`
([`common.rs:144`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_coco12/common.rs#L144))
decodes a raw font entry into the same boolean grid using the same
`0x80 >> col` convention as `blit_cell`, so the assertion is comparing a
sampled picture directly against source font data.

Finally, the choice of letter. `'O'`, screen code `$0F`, was picked because
it is one of the few glyphs that visibly differs across all three fonts: it
is square on the plain MC6847, rounded on the T1, and a third shape again in
the GIME's own table. That lets a single test prove "the right font ROM,"
not merely "a plausible font." The two `assert_ne!` lines are doing as much
work as the `assert_eq!` — without them, a renderer that accidentally used
the T1's table would pass, and the entire claim §7.4 is built on would go
unchecked.

---

## 7.10 Reading assignment

Read these in this order. Each one makes the next more legible, and the
whole assignment is comfortably an evening's work.

1. [`crates/coco-core/src/raster.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/raster.rs),
   all forty lines — §7.3's entire subject. Read the module doc comment as
   carefully as the code; it is where the reasoning behind the canonical
   canvas lives, including the MAME cross-references that make the geometry
   checkable rather than asserted.
2. [`crates/coco-core/src/video.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video.rs)
   — the module-level doc first, then the geometry constants, then
   `ColorSource` and its `resolve` method. That last pair is §7.7's CoCo 1/2
   versus CoCo 3 split, and it is a good small example of modeling a
   difference between two machines without duplicating everything around it.
3. [`crates/coco-core/src/video/text.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video/text.rs)
   — the whole file. It is 271 lines and every one of them was excerpted or
   explained somewhere in this chapter, which makes it a good test of
   whether the chapter landed. Read `resolve_alpha_cell` twice: once for
   what it does, and once for the doc comment above it, which states the
   hardware rules the function implements and cites where each came from.
4. [`crates/coco-core/src/font6847.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font6847.rs)'s
   module doc and the first ten entries of `MC6847_FONT`. Decode two or
   three of them by hand as in §7.5.1 — it goes faster than expected after
   the first one. Then skim
   [`crates/coco-core/src/font_gime.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/font_gime.rs)'s
   doc comment for the `GIME_FONT` versus `GIME_LOWRES_FONT` distinction:
   the hi-res 40- and 80-column text modes use the former, which is next
   week's material, and this week's legacy mode uses the latter.
5. [`crates/coco-core/src/machine/video_mode.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/video_mode.rs)
   and
   [`crates/coco-core/src/machine/render.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs)
   — `video_mode()` and `paint_legacy_scanline` in full, then, briefly and
   for contrast, `render_coco_text` and `render_coco_graphics`, which are
   the CoCo 1/2 whole-field path. Reading the per-scanline and whole-field
   painters back to back is the clearest way to feel why §7.3's caveat
   matters.
6. [`crates/coco-core/src/gime/palette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/palette.rs)
   — just `rgb_color`. Ignore the composite lookup tables entirely; they
   are Chapter 9's problem, and they are hand-measured data rather than
   anything you can reason about.
7. The tests:
   [`crates/coco-core/tests/render.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render.rs)
   in full, all 185 lines, then a skim of
   `crates/coco-core/tests/render_coco12/` —
   [`mc6847_fonts.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_coco12/mc6847_fonts.rs),
   [`coco3_compat_text.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_coco12/coco3_compat_text.rs),
   and [`common.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/render_coco12/common.rs)'s
   two `sample_cell*` helpers.

Run the whole suite while you read:

```
cargo test -p coco-core --test render --test render_coco12
```

Fifteen tests, all green, all with no ROM and no window.

---

## 7.11 Exercises

The mix this week runs from pencil-and-paper decoding to a deliberate
sabotage with a verified outcome. Exercise 7.1 is the one to do first even
if you skip the others; everything in §7.5 through §7.7 lands harder once
one byte has been decoded by hand.

**7.1 — Hand-render a byte (build/by-hand).** Decode the byte `$9D` fully:
which bits are set, what mode does bit 7 select, and what does the cell
look like? (Hint: `$9D = 1001 1101`.) Then draw the resulting 8×12 pixel
grid on paper, `#`/`.` style like §7.5.1's worked example — for an
alphanumeric byte, look up the actual glyph rows in `MC6847_FONT`; for a
semigraphics byte, work out which of the four quadrants are lit from the
low nibble. Check your color assignment (which quadrant/glyph pixels are
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
running BASIC program executes `POKE &HFF90, PEEK(&HFF90) OR 128` (setting
`INIT0` bit 7, the `COCO` bit) mid-frame, one field before you next call
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

Chapter 8 turns off the `INIT0 COCO` bit this chapter spent so long explaining
and asks what happens on the other side of it: the GIME's *own* video
registers at `$FF98`–`$FF9F`, native text with real per-character color
attributes and blink, and native bitmap graphics at up to 640 pixels wide.
The contrast is instructive. Where legacy text packs three meanings into one
byte and reads its geometry from a chip that no longer exists, GIME-native
modes have a register for everything, read physical RAM with the MMU
bypassed, and can put sixteen colors on screen at once.

The canonical 640×240 canvas remains unchanged. The
palette-register-to-RGBA pipeline is the same pipeline, `rgb_color` and all.
The per-scanline call chain, the `FieldScan` latch, and the row cursor are
all unchanged. Chapter 8 is the same raster and the same painter with a
completely different register file deciding what to paint.

Chapter 8 uses the `$FF90`–`$FF9F` register map introduced in Chapter 1.
