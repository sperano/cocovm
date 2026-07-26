# Chapter 9 — Splits, Composite, and PMODEs

*Week 9. Goal: the effects that made demos possible, and the emulation-policy
questions they force. Weeks 7–8 gave you a raster, VDG text, and the GIME's
native text/graphics modes rendered from a register snapshot taken once per
field. This week removes that simplification in two directions at once: the
register snapshot turns out to be a lie (real software rewrites those
registers **while the beam is still scanning**, and the picture must show
both halves), and the "one renderer" story turns out to be a lie too — the
same 6-bit palette value paints a different colour depending on which cable
is plugged into the back of the machine. Both lies are things you have
personally seen and never had a name for: the two-tone game screen where the
top third and the bottom two-thirds clearly came from different POKEs, and
the muddy-brown mess your friend's composite TV made of colours that looked
crisp on your RGB monitor. By the end of this chapter you can point at the
exact struct and the exact test that explain each.*

---

## 9.1 Two pictures from one register file

Here is the claim this chapter defends: **composite vs. RGB is not a second
renderer.** There is exactly one code path that walks video RAM and produces
pixels — the `paint_scanline`/`paint_text_row`/`paint_graphics_row` functions
you read in week 8, untouched since. What changes between an RGB monitor and
a composite one is a single function call at the very last step, after every
geometric decision (which byte, which bit, which palette register) has
already been made: turning a 6-bit register *value* into an RGBA pixel. That
function is `GIME::color`, and this chapter is largely about the two
different things it can do with the same six bits.

That's also why the real GIME needed no mode switch for this at all. The
chip drives an RGB output pin *and* a composite output pin simultaneously,
all the time — it's the monitor cable, not a GIME register, that decides
which signal the phosphors respond to. `MonitorType` in this codebase is
purely an emulator/UI choice (`crates/coco-core/src/gime/palette.rs:8-17`)
threaded in through `MachineConfig`, not something CoCo software can read or
set:

```rust
/// Which monitor signal path resolves 6-bit palette values to RGB: the real
/// GIME drives both an RGB and a composite output simultaneously, and it's
/// the monitor cable — not a GIME register — that decides which one matters.
/// Emulator config/UI choice; default matches prior (RGB-only) behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MonitorType {
    #[default]
    RGB,
    Composite,
}
```

You pick a monitor once, when you configure the machine (`crates/coco-core/src/config.rs:167-171`,
`Option<MonitorType>`, `None` on machines with no monitor port at all — a
detail the CoCo 1/2 chapters will use). The GIME itself never finds out.

## 9.2 The RGB decode: a formula

Start with the easy half, because it sets up the contrast. Each GIME palette
register is 6 bits, laid out `RGBrgb` — a high bit and a low bit per
channel, giving four intensity levels (`0, 1, 2, 3`) per channel. RGB output
is a direct, arithmetic unpack — no lookup table, no hardware quirks, just
bit-picking and a fixed scale (`palette.rs:56-62`):

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

Read the bit assignment carefully — it's not "high nibble, low nibble." Red
takes bits 5 and 2, green takes bits 4 and 1, blue takes bits 3 and 0: the
register is `R1 G1 B1 R0 G0 B0`, interleaved. `chan` reassembles the two bits
for one channel into a 2-bit value (`0..3`) and multiplies by `0x55`
(`0x55 × 3 = 0xFF`, so the four levels land exactly on `0x00/0x55/0xAA/0xFF`
— evenly spaced, no rounding error). This is pure digital-to-analog
arithmetic: an RGB monitor has three separate electron guns, one per
channel, and the GIME just needs to hand each one a voltage. There's nothing
to measure and nothing to get wrong — which is exactly why it's a four-line
function instead of a 64-entry table.

## 9.3 The composite decode: a table, because there is no formula

Composite video has no separate channels. Luminance and colour are
multiplexed onto a single carrier wave — colour rides as the phase and
amplitude of a 3.58 MHz subcarrier added on top of the brightness signal —
and a real TV's tuner has to demodulate that carrier back into something a
phosphor can use. That demodulation is analog, lossy, and depends on exact
component tolerances in both the GIME's video DAC and the TV's decoder
circuit. You cannot derive it from the `RGBrgb` bit layout with arithmetic;
there is no clean function from "6-bit register value" to "NTSC composite
colour" the way `rgb_color` is a clean function from "6-bit register value"
to RGB voltage. So the codebase doesn't try. It ships the actual measured
result, ripped from MAME's own hand-calibrated table (`palette.rs:19-44`,
comment preserved verbatim because the provenance matters):

```rust
/// Composite-monitor palette (BPI=0), 64 entries indexed by the 6-bit GIME
/// palette value, `0xRRGGBB`. Hand-measured on real hardware — there is no
/// formula. Verbatim from MAME `src/mame/trs/gime.cpp` `get_composite_color`
/// (BSD-3-Clause, Nathan Woods; see NOTICE.md).
const COMPOSITE_PALETTE: [u32; 64] = [
    0x000000, 0x004c00, 0x004300, 0x0a3100, 0x2f1b00, 0x550100, 0x6c0000, 0x770006, 0x71004b,
    0x5c008b, 0x3b00b8, 0x1100ca, 0x001499, 0x002c62, 0x004011, 0x004b00, 0x2d2d2d, 0x069800,
    0x288f00, 0x537d00, 0x786700, 0xa04c00, 0xb63402, 0xc3224c, 0xbd1693, 0xa814d5, 0x881cfe,
    0x5e2cff, 0x105ee9, 0x0076b2, 0x008b60, 0x009618, 0x747474, 0x41d714, 0x62cf00, 0x8ebd00,
    0xb4a700, 0xdd8c01, 0xf5733a, 0xfe6085, 0xfd53ce, 0xe950ff, 0xc958ff, 0x9e67ff, 0x4e9aff,
    0x36b3f7, 0x26c9a3, 0x2bd558, 0xfdfdfe, 0x88e85a, 0xa1e03f, 0xbed238, 0xd8c342, 0xf1b161,
    0xfea08d, 0xfe95bf, 0xfd8ef1, 0xef8eff, 0xd895ff, 0xb9a1ff, 0x86c4ff, 0x78d4f2, 0x71e2b6,
    0xffffff,
];
```

`0xRRGGBB` packed 32-bit words, unpacked by shifting (`palette.rs:48-50`):

```rust
/// Unpack an `0xRRGGBB` composite-table entry into RGBA, matching
/// [`GIME::rgb_color`]'s return convention (opaque alpha).
fn unpack_rgb(v: u32) -> [u8; 4] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8, 0xFF]
}
```

> **Rust corner: packed hex literals as a compact data table.** `0x004c00`
> is a `u32` in the source, but nobody ever does arithmetic on it as a
> number — it's three bytes wearing a trenchcoat. `unpack_rgb` peels them
> back apart with shifts and an `as u8` truncation (`v >> 16` keeps only the
> byte that matters once cast down). This is a common trick for hand-written
> constant tables: one line per entry, red/green/blue visually grouped in
> pairs of hex digits, at the cost of needing a tiny unpack function instead
> of just indexing a `[[u8; 3]; 64]` array directly. You'll see the same
> `0xRRGGBB` packing anywhere a codebase ports a colour table from a C
> source (MAME's own `rgb_t` does exactly this) rather than reshaping it.

`GIME::color` is the dispatcher — the *only* place `MonitorType` is ever
consulted (`palette.rs:69-87`):

```rust
/// Resolve a 6-bit GIME palette value to RGBA through the currently
/// selected monitor path (`self.monitor`). RGB mode is [`Self::rgb_color`]
/// unconditionally; composite mode picks [`COMPOSITE_PALETTE`] or
/// [`COMPOSITE_PALETTE_180`] per $FF98 BPI, then averages channels to
/// grey when $FF98 MOCH is set (MAME `gime.cpp` `update_composite`).
pub fn color(&self, value: u8) -> [u8; 4] {
    match self.monitor {
        MonitorType::RGB => Self::rgb_color(value),
        MonitorType::Composite => {
            let table = if self.vmode & vmode::BPI != 0 {
                &COMPOSITE_PALETTE_180
            } else {
                &COMPOSITE_PALETTE
            };
            let [r, g, b, a] = unpack_rgb(table[value as usize & 0x3F]);
            if self.vmode & vmode::MOCH != 0 {
                let avg = ((r as u16 + g as u16 + b as u16) / 3) as u8;
                [avg, avg, avg, a]
            } else {
                [r, g, b, a]
            }
        }
    }
}
```

Every other renderer in this codebase — `paint_text_row`, `paint_graphics_row`,
the legacy VDG text and graphics painters you'll meet later in this chapter —
calls `g.color(register_value)` and never asks which branch it took. That's
the "not a different renderer" claim made concrete: swap `MonitorType` and
every pixel on screen can change without one line of the scanout code
running differently.

### Reading the table: a hue wheel, not a colour wheel

Look at the four indices `0x00`, `0x10`, `0x20`, `0x30` in `COMPOSITE_PALETTE`
— they land at array offsets 0, 16, 32, 48, i.e. every 16th entry starting
from zero:

```
0x00 -> 0x000000   (black)
0x10 -> 0x2d2d2d   (dark grey)
0x20 -> 0x747474   (mid grey)
0x30 -> 0xfdfdfe   (near-white)
```

All four are achromatic — equal (or near-equal) R, G, and B — and they form
a rising brightness ramp. That's not a coincidence baked into four
hand-picked entries; it's the structure of the whole table. Split the 6-bit
value into a low nibble (bits 0–3, 16 values) and a high 2 bits (bits 4–5, 4
values): the low nibble selects a **hue**, and the high bits select a
**luminance level**. Hue `0` is defined as "no colour," so all four
luminance steps at hue `0` are grey — which is exactly the `0x00/0x10/0x20/0x30`
family above. This matches the real GIME/CoCo palette-register convention
documented in SEB Unravelled II, and it's directly testable: `composite.rs`'s
`composite_decode_grey_anchors` test is precisely these four values.

Walk one non-zero hue across its four luminance steps and the same pattern
holds — hue `1` (green-family) at increasing brightness:

```
0x01 -> 0x004c00   (0,   76,  0)    dark green
0x11 -> 0x069800   (6,   152, 0)    brighter green
0x21 -> 0x41d714   (65,  215, 20)   bright green
0x31 -> 0x88e85a   (136, 232, 90)   pale green
```

Brightness rises cleanly with each step, and green stays dominant
throughout — but the ramp isn't a pure "same hue, more luminance" scale
either: the top step picks up a real amount of red (`0x88` out of `0xE8` of
green), pulling it slightly warmer and paler than a mechanical multiply-by-
luminance would produce. That's exactly what you'd expect from a *measured*
NTSC recording rather than a computed HSV ramp. A formula-generated table
wouldn't have that kind of drift; a hand-calibrated one does, because the
real hardware has it.

## 9.4 BPI: burst-phase invert, and a correction

`COMPOSITE_PALETTE_180` is a second, complete 64-entry table, selected when
`$FF98` bit 5 (`vmode::BPI`) is set:

```rust
/// Burst phase invert (alternate composite colour set).
pub const BPI: u8 = 0x20;
```

"Burst phase" is the reference signal a composite decoder locks onto to know
what phase angle corresponds to what hue; inverting it should, in principle,
rotate every hue by half the colour wheel — 180°, not some smaller angle.
That's worth stating precisely because an earlier pass at this material
claimed "roughly a 120° hue shift" for palette value `0x01` under BPI. The
actual test data doesn't support that number. `composite_decode_hue_and_bpi`
(`tests/composite.rs:28-38`):

```rust
#[test]
fn composite_decode_hue_and_bpi() {
    let mut g = GIME {
        monitor: MonitorType::Composite,
        ..GIME::new()
    };
    assert_eq!(g.color(0x01), [0x00, 0x4c, 0x00, 0xFF]);

    g.vmode = vmode::BPI;
    assert_eq!(g.color(0x01), [0x5a, 0x0e, 0x5a, 0xFF]);
}
```

`0x01` un-inverted is `(0, 76, 0)` — pure green, hue 120° on a standard
colour wheel (green sits exactly at 120° in HSV). Inverted it's `(90, 14, 90)`
— R and B tied and both far above G, which is the textbook signature of
magenta, hue very close to 300°. `300 - 120 = 180`. Check a second, darker
entry the same way: `0x03` un-inverted is `(10, 49, 0)` (yellow-green, ≈108°
by the HSV formula), inverted (`COMPOSITE_PALETTE_180[3]`) is `(54, 15, 64)`
(violet, ≈288°) — again a 180° delta. **The claim should read "~180°," not
"~120°"** — and that number isn't a coincidence, it's the register's own
name: BPI *inverts* the phase, which is a half-turn by construction, not a
third-turn. Check a more saturated entry, though (`0x0A`, un-inverted
`(59, 0, 184)` ≈ 259° — a blue-violet — vs. inverted `(17, 76, 0)` ≈ 107°, a
green: a 207° delta) and the rotation is no longer clean. That's the
hand-measured table telling the truth about hand-measured tables: a real
chip's phase response isn't perfectly linear across its whole brightness/
saturation range, so "invert the burst" is the right mental model, but
"exactly 180° for every entry" is not something you can rely on
pixel-by-pixel. Trust the table, not a formula, exactly as the source
comment insists.

## 9.5 MOCH: averaging to grey

The last bit `GIME::color` consults is `vmode::MOCH` (`$FF98` bit 4) —
monochrome-on-composite, for driving a green-screen or B&W composite
monitor. It doesn't touch the lookup at all; it post-processes whatever
colour the table produced, averaging the three channels:

```rust
if self.vmode & vmode::MOCH != 0 {
    let avg = ((r as u16 + g as u16 + b as u16) / 3) as u8;
    [avg, avg, avg, a]
} else {
    [r, g, b, a]
}
```

`composite_moch_averages_channels` (`tests/composite.rs:40-50`) nails down
the exact arithmetic, including the truncation, because "average" is
ambiguous until you specify the rounding:

```rust
#[test]
fn composite_moch_averages_channels() {
    // Normal-table index 0x01 = 0x004c00 -> r=0x00, g=0x4c (76), b=0x00.
    // (0 + 76 + 0) / 3 = 25 (integer division) = 0x19.
    let mut g = GIME {
        monitor: MonitorType::Composite,
        ..GIME::new()
    };
    g.vmode = vmode::MOCH;
    assert_eq!(g.color(0x01), [0x19, 0x19, 0x19, 0xFF]);
}
```

`(0 + 76 + 0) / 3` is `25.33...`; `u16` integer division truncates to `25`
(`0x19`), not rounds to `25` — the difference only matters by one count here,
but it's exactly the kind of off-by-a-hair detail that a bit-exact
trace-diff against MAME (week 4's testing philosophy) would catch and a
"looks about right" implementation would let through silently. `MonitorType::RGB`
never looks at `MOCH` or `BPI` at all — `rgb_monitor_ignores_bpi_and_moch`
(`tests/composite.rs:52-60`) sets both bits and asserts the RGB path is
unaffected, which is really a test of the `match` in `color()`: RGB's arm
returns immediately, full stop, before either flag is ever read.

## 9.6 War story: the NitrOS-9 EOU greyscale regression

Here's where the "no formula, hand-measured, hue-then-luminance" structure
stopped being a curiosity and started mattering. NitrOS-9's `gshell`
desktop, part of the **EOU** ("Ease of Use") package, draws a greyscale
interface — window chrome, shading, the works — using exactly the four
palette values you just met: `0x00`, `0x10`, `0x20`, `0x30`. It's real
software, written by people who owned real composite CoCo 3s, who chose
those four values specifically *because* they're the hardware's own grey
ramp — not because they picked four arbitrary-looking numbers and hoped.
Anyone running that software through a composite decode that got the hue-vs-
luminance split wrong would see EOU's "neutral grey desktop" rendered in
whatever stray colour the wrong decode produced instead — black, green, red,
yellow, something plausible-looking but *wrong*, and wrong in a way that's
easy to miss if you never happen to boot NitrOS-9 with a composite monitor
selected. The regression test that guards this is `eou_greyscale_regression`
(`tests/composite.rs:62-85`), and it's worth reading end to end because it
encodes exactly the property real software depended on — achromatic *and*
monotonically brighter — not just "matches these four hex triples":

```rust
#[test]
fn eou_greyscale_regression() {
    // NitrOS-9 EOU's gshell greyscale desktop programs palette regs 0/0x10/
    // 0x20/0x30. On a composite monitor these must decode to achromatic,
    // strictly increasing brightness (not black/green/red/yellow as an
    // RGB-only decode would render them).
    let g = GIME {
        monitor: MonitorType::Composite,
        ..GIME::new()
    };
    let regs = [0x00u8, 0x10, 0x20, 0x30];
    let colors: Vec<[u8; 4]> = regs.iter().map(|&r| g.color(r)).collect();
    // The hand-measured table isn't perfectly achromatic at every entry
    // (0x30 = 0xfdfdfe, one LSB off on blue), so allow a 1-count tolerance
    // rather than exact channel equality.
    const GREY_TOLERANCE: u8 = 1;
    for c in &colors {
        assert!(c[0].abs_diff(c[1]) <= GREY_TOLERANCE, "achromatic: r ~= g ({c:?})");
        assert!(c[1].abs_diff(c[2]) <= GREY_TOLERANCE, "achromatic: g ~= b ({c:?})");
    }
    assert!(colors[0][0] < colors[1][0]);
    assert!(colors[1][0] < colors[2][0]);
    assert!(colors[2][0] < colors[3][0]);
}
```

Two things about this test are worth slowing down on, because both are
lessons about testing hardware you didn't build. First, the comment's
parenthetical — `0x30 = 0xfdfdfe`, blue is one count short of the other two
channels — is the test author refusing to pretend the measured data is
cleaner than it is. A stricter test (`c[0] == c[1] && c[1] == c[2]`) would be
*more* elegant and *less* true; it would either need to be relaxed the first
time someone re-measured the table from a slightly different real GIME, or
it would quietly encourage "fixing" the table to be exactly grey — which
would be fixing it to be wrong, since the real chip's `0x30` really does
carry a one-LSB blue tint. The tolerance is the correct response to
"hardware doesn't round the way your intuition does." Second, notice what
the test does *not* assert: it never checks the four colours are internally
identical to `COMPOSITE_PALETTE[0]`/`[0x10]`/`[0x20]`/`[0x30]` by value —
that would just be re-testing `composite_decode_grey_anchors` under a
different name. Instead it asserts the *property* the real desktop software
actually needed (achromatic, strictly brighter) so that if the table's exact
hex values ever get re-measured against a different real GIME unit, this
test keeps passing as long as the property real software depends on still
holds. That's a regression test written from the *consumer's* requirement,
not from the implementation's current output — the right level to pin a
hand-measured constant at.

## 9.7 What monitor choice meant for software

Put the two halves of `GIME::color` side by side and a piece of CoCo 3
software history falls out for free: **the exact same POKE produced a
different-looking screen depending on what was plugged into the back of the
machine**, and nothing the program did could tell which monitor it was
talking to (the CoCo 3 has no monitor-sense pin; `MonitorType` doesn't exist
as a register). Extended Color BASIC's `SCREEN` statement and its graphics-
mode `PMODE`/`PCLS` calls choose a **colour set** — `CSS`, bit 3 of PIA1
`$FF22` — which selects *which* GIME palette registers a legacy mode reads
from (regs 8/9 vs. 10/11 for two-colour modes, 0–3 vs. 4–7 for four-colour;
you'll meet the exact tables in §9.14). That's a choice about *which*
register, made once, by the program. It is orthogonal to — and happens
entirely upstream of — the RGB-vs-composite choice made once, by whoever
plugged in the cable, about how *any* register's value gets decoded. A
program tuned its four PMODE colours by picking a CSS set that looked good
on the monitor its author owned; a different reader's composite TV would run
those exact same register values through `COMPOSITE_PALETTE` instead of
`rgb_color` and see something else entirely. This is the actual, mundane
reason 1980s CoCo software sometimes shipped a "for composite" and "for RGB"
colour scheme, or why a magazine type-in's screenshot never quite matched
what you saw on your own TV: two totally different physical processes — a
three-gun CRT reading three separate voltages, versus an NTSC decoder
demodulating a shared subcarrier — were being asked to render six identical
bits, and physics, not software, decided they wouldn't agree.

---

## 9.8 The lie week 8 told you (on purpose)

Week 8's `render_field` reads every GIME register exactly once and paints an
entire field from that one snapshot — fine for understanding geometry, but
it describes a machine where BASIC's `PALETTE` and `PMODE` statements only
ever run between fields, never while one is being drawn. Real 6809 code
makes no such promise. A game can poke the border colour from inside a
horizontal-sync interrupt handler that fires 60-some times a *second* at a
*specific scanline*, and a demo can rewrite the video-base register the
instant the beam crosses a row it knows by heart. The picture that results —
different colours in different bands of the *same* field — is a **raster
split**, and it was one of the CoCo demo scene's bread-and-butter tricks
(more of §9.12, once you've seen the mechanism).

The machine loop already had everything this needs, because week 6 built it
that way: `end_of_line()` (`crates/coco-core/src/machine/run.rs:130-172`)
calls `self.render_scanline()` — one canvas row — **every single scanline**,
not once per field. The per-scanline dispatcher (`machine/render.rs:29-64`)
is what you actually extend this week:

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
    let blink_on = self.bus.gime.blink_state;
    let scan = self.field_scan.as_mut().expect("checked Some above");
    gime_video::paint_scanline(
        &self.bus.gime,
        &self.bus.ram,
        scan,
        blink_on,
        row,
        &mut self.framebuffer,
    );
}
```

Two things happen only at `self.line == 0`: the framebuffer is resized, and
`FieldScan::latch` runs. Every other line, the function paints straight from
whatever `self.bus.gime`'s registers currently say — live. That's the whole
split mechanism, stated in one sentence: **most registers are read fresh
every line; a small, named group is read once, at line 0, and frozen for the
rest of the field.**

### `FieldScan`: the latch, made a value

`FieldScan` (`gime_video.rs:117-159`) is that freeze, reified as a struct
instead of scattered `if self.line == 0` checks:

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

impl FieldScan {
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
}
```

Three fields, three pieces of state that must survive across scanlines
without being re-derived from a register that might have changed underneath
them: which display path this field is on (`legacy`), where the current data
row starts (`row_base`, seeded from `$FF9D/$FF9E` or the SAM page — the
field-latched video base), and how far into the current character/pixel row
the scan is (`line_in_row`, seeded from the `$FF9C` scroll nibble — the
smooth-scroll seed). `row_base` and `line_in_row` aren't just latched once
and left alone, though — `advance_scan` (`gime_video.rs:247-258`) mutates
them every body row, stepping the row pointer forward by the *current*
line's live pitch once `line_in_row` wraps past the *current* live LPR. That
split — origin frozen at field start, cursor advancing live thereafter — is
exactly why a mid-field mode switch can still walk off the end of a row it
started in text mode and finish it in graphics mode (you'll see this exact
scenario in §9.10's fourth test).

Put together, that's the register taxonomy for the whole chapter:

| Group | Registers | When it takes effect |
|---|---|---|
| **Live** | `$FF9A` border, `$FF98` mode bits (BP/BPI/MOCH/LPR), `$FF99` VRES (LPF/HRES/CRES), `$FF9F` X-offset/HVEN, all 16 palette regs | The very next scanline painted |
| **Field-latched** | `$FF9D/$FF9E` video base, INIT0 `COCO`, `$FF9C` scroll seed | The *next field* — a write mid-frame is invisible until the wrap back to line 0 |

Notice what's *not* on the field-latched list: the palette registers
themselves. A `PALETTE` statement (or a raw `STA $FFBx`) is live, exactly
like the border — which is precisely why EOU's greyscale desktop and any
palette-cycling demo effect work at all on real hardware without waiting a
whole frame.

## 9.9 Four registers, four tests

`tests/scanline_split.rs` earns its "best-named test file in the repo"
reputation (course README, week 9) by testing this taxonomy one register
class at a time, against a machine running a **zero-filled ROM** — reset
vector points at `$0000`, which decodes to a harmless `NEG` loop, so
scanlines advance at a known, deterministic pace with nothing in ROM
touching a single video register. Every test pokes `m.bus.gime` fields
directly from the harness instead. The shared setup (`tests/scanline_split.rs:29-55`):

```rust
/// $FF9D/$FF9E value → physical $8000.
const VOFF: u16 = 0x1000;
const BASE: usize = (VOFF as usize) << 3;

/// The scanline the tests poke registers at: inside the 192-line body
/// (rows 25..217 for LPF=%00).
const SPLIT_LINE: u32 = 100;

/// A CoCo 3 machine in GIME-native 320×192×16 graphics with an identity
/// palette, running zero-ROM filler code.
fn gime_graphics_machine() -> Machine {
    let rom = vec![0u8; 32 * 1024].into_boxed_slice();
    let mut m = Machine::new(MachineConfig::default(), rom);
    let g = &mut m.bus.gime;
    g.init0 = 0; // COCO=0: GIME-native video
    g.vmode = vmode::BP;
    g.vres = 0x1E; // HRES=%111 (160 bytes), CRES=%10 → 320px, 16 colours
    g.vertical_offset = VOFF;
    for (i, reg) in g.palette.iter_mut().enumerate() {
        *reg = i as u8;
    }
    m
}

/// Run to the end of the current field (the wrap back to line 0).
fn finish_field(m: &mut Machine) {
    while !m.step_instruction().field_complete {}
}

/// Run until the machine is at the start of `line` within the current field.
fn run_to_line(m: &mut Machine, line: u32) {
    while m.scanline() != line {
        m.step_instruction();
    }
}
```

`palette[i] = i` is a deliberate identity mapping: reading pixel value `v`
back from a rendered row and comparing it to `GIME::rgb_color(v)` (RGB
monitor here, so no composite indirection to worry about) proves the render
picked up palette register `v` — no ambiguity about which register a given
on-screen colour came from.

### Live: the border splits mid-field

```rust
#[test]
fn border_write_mid_field_splits_the_border_at_the_line() {
    let mut m = gime_graphics_machine();
    m.bus.gime.border = 0x09;
    finish_field(&mut m); // first full field with the old border

    run_to_line(&mut m, SPLIT_LINE);
    m.bus.gime.border = 0x2A;
    finish_field(&mut m);

    let fb = &m.framebuffer;
    assert_eq!(
        px(fb, 0, 0),
        GIME::rgb_color(0x09),
        "top border painted before the write keeps the old colour"
    );
    assert_eq!(
        px(fb, 0, CANVAS_H - 1),
        GIME::rgb_color(0x2A),
        "bottom border painted after the write has the new colour"
    );
}
```

One field runs to completion first purely to get the machine's internal
line counter back to a known `0` (`run_to_line` free-runs the CPU until
`m.scanline() == SPLIT_LINE`, and there's no API to rewind mid-field — the
cleanest way to reach a specific line deterministically is to start a fresh
field). The interesting part is the second field: border is `0x09` when the
scan reaches row `SPLIT_LINE`, gets rewritten to `0x2A` mid-field, and the
top row (painted while `border` was still `0x09`) and bottom row (painted
after the write) genuinely differ within the *same* framebuffer. Nothing
special had to be built for this — `paint_side_borders` and the "not in the
active window" fill both call `resolve_colors(g)` fresh, every line
(`gime_video.rs:161-168`, §8's code, unmodified), and `resolve_colors` reads
`g.border` straight off the live `GIME`. Liveness here isn't a feature that
was added; it's what happens when nothing was added to *prevent* it.

### Live: a palette write recolours only what's below it

```rust
#[test]
fn palette_write_mid_field_recolors_only_lines_below_it() {
    let mut m = gime_graphics_machine();
    // Zeroed video RAM → every body pixel reads palette register 0.
    m.bus.gime.palette[0] = 0x01;
    finish_field(&mut m);

    run_to_line(&mut m, SPLIT_LINE);
    m.bus.gime.palette[0] = 0x02;
    finish_field(&mut m);

    let fb = &m.framebuffer;
    let split = SPLIT_LINE as usize;
    assert_eq!(px(fb, 0, split - 1), GIME::rgb_color(0x01), "body above the split keeps the old palette");
    assert_eq!(px(fb, 0, split), GIME::rgb_color(0x02), "the split line onward has the new palette");
    // Both orders: the same field shows both colours at once.
    assert_eq!(px(fb, 0, 30), GIME::rgb_color(0x01));
    assert_eq!(px(fb, 0, 200), GIME::rgb_color(0x02));
}
```

Same shape, different register — and it matters that it's a different
*kind* of register. The border is a colour directly; a palette register is
one level of indirection (video RAM holds an *index*, the palette register
holds the *colour* that index currently means). `resolve_colors` rebuilds
the whole 16-entry resolved-colour array from `g.palette` on every single
call to `paint_scanline` (`gime_video.rs:264-273`), so this test is really
checking that indirection is re-resolved, not cached, per line — a palette
that a demo cycles every scanline (a classic "more than 16 colours on
screen" trick, §9.12) needs exactly this property to work at all.

### Field-latched: the video base waits

```rust
#[test]
fn video_base_write_mid_field_waits_for_the_next_field() {
    let mut m = gime_graphics_machine();
    // Marker bytes for a body row BELOW the split (row 130 = body row 105),
    // distinct at the two candidate bases: only a row painted after the poke
    // can tell whether the base was re-latched mid-field.
    const MARKER_ROW: usize = 105; // body row → canvas row 25 + 105 = 130
    let other_voff = 0x1100u16;
    let other_base = (other_voff as usize) << 3;
    m.bus.ram[BASE + MARKER_ROW * 160] = 0x50; // palette 5 at the latched base
    m.bus.ram[other_base + MARKER_ROW * 160] = 0x70; // palette 7 at the new base
    finish_field(&mut m);

    run_to_line(&mut m, SPLIT_LINE);
    m.bus.gime.vertical_offset = other_voff;
    finish_field(&mut m);
    let marker_canvas_row = 25 + MARKER_ROW;
    assert_eq!(
        px(&m.framebuffer, 0, marker_canvas_row),
        GIME::rgb_color(5),
        "mid-field base write must NOT retarget this field (MAME new_frame)"
    );

    finish_field(&mut m);
    assert_eq!(
        px(&m.framebuffer, 0, marker_canvas_row),
        GIME::rgb_color(7),
        "the next field latches the new base"
    );
}
```

This is the field-latched half of the taxonomy, and the test is built to be
unforgiving about it. Notice the setup plants **two different marker bytes
at two different physical addresses** — `0x50` at the field's actual video
base, `0x70` at the *other* candidate base the mid-field write points at —
specifically so that a bug which re-latches `row_base` mid-field can't
accidentally produce the "right" colour for the wrong reason. If the test
only checked "does row 130 show palette 5," a broken implementation that
re-read `g.video_base()` live every row would need to coincidentally paint
garbage that happens to equal `GIME::rgb_color(5)` to pass — vanishingly
unlikely, but not the point. The point is: the marker at the *new* base
(`0x70`) exists so a *wrong* answer is forced to be visibly wrong (some
*other* concrete colour), not just "not obviously right." The write happens
mid-field (line `SPLIT_LINE`) but the marker row (canvas row 130, well below
the split) is asserted to *still* show the old base's byte (`palette 5`)
after that same field finishes — proving `row_base`, latched once at line 0
by `FieldScan::latch`, was never touched by the mid-field write to
`$FF9D/$FF9E`. Only after a *second* `finish_field` — a fresh call to
`FieldScan::latch` at the new field's line 0 — does the marker flip to
`palette 7`. `advance_scan` marches `scan.row_base` forward every body row
using the pitch computed from *live* registers (`$FF9F` HVEN and the
current row's byte count), but the row pointer's **origin** for the whole
field was fixed the instant the field began, and nothing mid-field can move
it. This is the split's other half, and it's the one real software had to
actually respect: a video-base change (switching which of two double-buffered
screens is being scanned out, say) is a *whole-field* operation, never a
mid-field one — which is a design constraint you now know comes straight
from the hardware, not from any laziness in this codebase.

### Mode switches split too

```rust
#[test]
fn mode_switch_mid_field_splits_text_and_graphics() {
    let mut m = gime_graphics_machine();
    finish_field(&mut m);

    run_to_line(&mut m, SPLIT_LINE);
    // Switch to 80-column text with attributes mid-field. The row pointer has
    // already advanced through the graphics rows, so fill a wide swath of
    // char/attr pairs (' ' on attr bg palette 2) wherever the fetch lands.
    m.bus.gime.vmode = 0x03; // BP=0, LPR=%011 (8-line rows)
    m.bus.gime.vres = 0x15;
    for i in (BASE..BASE + 0x8000).step_by(2) {
        m.bus.ram[i] = b' ';
        m.bus.ram[i + 1] = 0x02;
    }
    finish_field(&mut m);

    let fb = &m.framebuffer;
    assert_eq!(px(fb, 0, 30), GIME::rgb_color(0), "graphics decode above the split (zeroed RAM → palette 0)");
    assert_eq!(px(fb, 0, SPLIT_LINE as usize + 1), GIME::rgb_color(2), "text decode below the split");
}
```

`$FF98`'s BP bit (graphics/text select) is in the *live* group, same as the
border — `paint_body_row` re-checks `g.vmode & vmode::BP` on every call
(`gime_video.rs:220-241`), dispatching to `paint_graphics_row` or
`paint_text_row` fresh each line. So a program can flip from a graphics
canvas to a text status bar partway down the screen, and the emulator does
exactly what the register file says to do, one line at a time, with no
special-casing anywhere for "mode changed since last line." The test's own
comment about "the row pointer has already advanced through the graphics
rows" is worth sitting with: `scan.row_base`/`scan.line_in_row` don't know
or care that the byte layout underneath them just changed shape (2 bytes/char
text vs. 1 byte/pixel-group graphics) — they're purely a byte-address
cursor, advanced by whatever pitch the *current* line's decode reports. That
RAM is filled broadly (`step_by(2)` across the whole 32K graphics buffer)
specifically because the graphics-mode row pointer left the cursor somewhere
the test author didn't want to hand-compute exactly.

## 9.10 The centerpiece: a split written in 6809, not the harness

Every test above works by reaching into `m.bus.gime` from the test harness
and writing a register directly — useful for isolating one register class,
but it sidesteps the actual mechanism a real raster-split demo used: **an
interrupt handler**, running as ordinary 6809 code, timed by the GIME's own
interval timer. `timer_firq_from_rom_code_splits_the_border`
(`tests/scanline_split.rs:174-239`) is the test that proves the whole path —
timer hardware, interrupt controller, FIRQ delivery, and the live-register
raster split — works end to end, with **zero harness register pokes**. It
hand-assembles a tiny ROM and lets the emulated CPU do everything.

### The setup program

```rust
const OLD_BORDER: u8 = 0x09;
const NEW_BORDER: u8 = 0x2A;
/// FIRQ handler location in the ROM image ($8000 + offset).
const ISR: u16 = 0x8040;

let program: &[u8] = &[
    0x10, 0xCE, 0x1F, 0xF0, // LDS  #$1FF0      stack in low RAM
    0x86, OLD_BORDER,       // LDA  #OLD_BORDER
    0xB7, 0xFF, 0x9A,       // STA  $FF9A       border = old colour
    0x86, 0x20,             // LDA  #intr::TMR
    0xB7, 0xFF, 0x93,       // STA  $FF93       FIRQENR: timer source
    0x7F, 0xFF, 0x91,       // CLR  $FF91       INIT1: TINS=0 (hsync rate)
    0x7F, 0xFF, 0x94,       // CLR  $FF94       timer MSB = 0
    0x86, SPLIT_LINE as u8, // LDA  #SPLIT_LINE
    0xB7, 0xFF, 0x95,       // STA  $FF95       timer LSB (restarts count)
    0x86, 0x10,             // LDA  #init0::FEN
    0xB7, 0xFF, 0x90,       // STA  $FF90       INIT0: FIRQ out, COCO=0
    0x1C, 0xAF,             // ANDCC #$AF       unmask FIRQ/IRQ
    0x20, 0xFE,             // BRA  *           wait for the timer
];
```

Read it as a straight line — this is exactly the kind of ROM code you have
read for real, just shorter. `LDS #$1FF0` gives interrupts somewhere to
stack a return frame; nothing here touches the CPU's normal work because
there isn't any — this ROM's only job is to arm one interrupt and then spin.
`STA $FF9A` sets the border to `OLD_BORDER` (`0x09`) up front, so the field's
top half has a known starting colour. Then five writes configure the GIME's
timer/interrupt block one register at a time, and it's worth naming every
one since this is the first time this course has touched the GIME timer
directly (week 6 only asserted that `tick_timer` gets *called*; here you see
what arms it):

- `STA $FF93` with `intr::TMR` (`0x20`) in A writes **FIRQENR**, the
  per-source FIRQ enable register — "when the timer underflows, route it to
  FIRQ, not just IRQ." (`GIME::write_firq_enable`, `gime.rs:311-314`.)
- `CLR $FF91` writes **INIT1** to zero — among other things, clears `TINS`
  (`init1::TINS`, bit 5), selecting the *slow* timer clock: one tick per
  horizontal sync, not the fast ~70 ns clock. One tick per scanline is
  exactly the granularity a raster-split needs.
- `CLR $FF94` / `LDA #SPLIT_LINE` + `STA $FF95` load the 12-bit timer value
  — MSB first (zero, since `SPLIT_LINE` fits in one byte), LSB second. Both
  `write_timer_msb` and `write_timer_lsb` call `restart_timer()`
  (`gime.rs:346-370`), so the **second** write (the LSB) is what actually
  (re)starts the countdown, seeded from whatever the MSB write already
  latched into the top nibble.
- `STA $FF90` writes **INIT0** with `init0::FEN` (`0x10`) set — the master
  FIRQ-output-enable gate. `GIME::firq_asserted()` (`gime.rs:341-344`)
  requires *both* this bit and a nonzero `firq_pending`; without it, the
  timer could underflow all day and never reach the CPU.
- `ANDCC #$AF` clears bits `0x50` of CC — `cc::IRQ_MASK` (`0x10`) and
  `cc::FIRQ_MASK` (`0x40`), the two interrupt masks reset sets on every CoCo
  (week 4). This is the instruction that actually lets the CPU *notice* the
  interrupt once it arrives; everything before it was just arming hardware
  that stays silent while masked.
- `BRA *` is the whole rest of the "program": branch to self, forever. There
  is deliberately nothing else for the CPU to do — the only way this loop
  ever does anything again is an interrupt breaking it open.

### The handler

```rust
let isr: &[u8] = &[
    0xB6, 0xFF, 0x93, // LDA  $FF93   read status (clears the latch)
    0x86, NEW_BORDER, // LDA  #NEW_BORDER
    0xB7, 0xFF, 0x9A, // STA  $FF9A   border = new colour
    0x7F, 0xFF, 0x93, // CLR  $FF93   no further timer FIRQs
    0x3B,             // RTI
];
```

`LDA $FF93` reads **FIRQENR** as a *status* register this time, not the
enable register it was a moment ago in the setup code — same address, two
different jobs, and this is the exact "reads have side effects" story week 1
promised would come back (`Bus::read` takes `&mut self` specifically because
of registers like this one). `GIME::read_firq_status`
(`gime.rs:322-325`) is `std::mem::take(&mut self.firq_pending)`: reading it
returns the latched bits *and* zeroes them in the same motion. This ISR
doesn't even look at the value it read (the very next instruction
overwrites A) — the read exists purely for its side effect, acknowledging
the interrupt so the CPU's FIRQ line drops and `RTI` doesn't just re-enter
the handler instantly. `STA $FF9A` is the actual raster-split write: border
becomes `NEW_BORDER` (`0x2A`), live, effective starting the *next* scanline
exactly like every test in §9.9. `CLR $FF93` writes zero through
`write_firq_enable` (`gime.rs:311-314`), which does two things at once —
`self.firq_enable = 0` (no source can raise FIRQ anymore) and, per that
function's own doc comment, `self.firq_pending &= val` also re-clears
pending (a documented hardware anomaly: writing `0` to an enable bit clears
that source's latched status too, "SEB Unravelled II documents and MAME
models" per the source comment). Belt and suspenders against a second FIRQ
firing before `RTI` retires. `RTI` (`0x3B`) restores the **partial frame** —
just CC and PC, three bytes, because FIRQ only ever saves the partial frame
(week 4) — and the CPU drops back into `BRA *`, forever, border now
`NEW_BORDER`.

Wiring it together needs one more piece: the FIRQ hardware vector,
`$FFF6/$FFF7` (`VECTOR_FIRQ`, week 4), has to point at `ISR`, and the reset
vector at `$FFFE/$FFFF` has to point at the setup program:

```rust
rom[isr_off..isr_off + isr.len()].copy_from_slice(isr);
// Vectors (hardwired-internal $FFE0+ region): FIRQ → ISR, RESET → $8000.
rom[0x7FF6..0x7FF8].copy_from_slice(&ISR.to_be_bytes());
rom[0x7FFE..0x8000].copy_from_slice(&0x8000u16.to_be_bytes());
```

`0x7FF6` is `$FFF6 - $8000` — the ROM image is a flat 32K buffer representing
`$8000–$FFFF`, so every absolute address in this test is offset by `$8000`
to find its byte in `rom[]`. `to_be_bytes()` matters exactly as much here as
it did in week 1's `Bus::write_u16` default method: the 6809 fetches vectors
big-endian, and a little-endian byte order here would send the CPU to a
FIRQ handler at completely the wrong address on the very first interrupt.

### Reading the assertion

```rust
let mut m = Machine::new(MachineConfig::default(), rom.into_boxed_slice());
finish_field(&mut m);

let old = GIME::rgb_color(OLD_BORDER);
let new = GIME::rgb_color(NEW_BORDER);
let column: Vec<[u8; 4]> = (0..CANVAS_H).map(|y| px(&m.framebuffer, 0, y)).collect();
assert_eq!(column[0], old, "field starts on the old border");
assert_eq!(column[CANVAS_H - 1], new, "field ends on the new border");
let transitions: Vec<usize> = (1..CANVAS_H)
    .filter(|&y| column[y] != column[y - 1])
    .collect();
assert_eq!(transitions.len(), 1, "exactly one border split, got {transitions:?}");
let split_row = transitions[0];
let expected = SPLIT_LINE as usize;
assert!(
    (expected..=expected + 4).contains(&split_row),
    "split at row {split_row}, expected within {expected}..={}",
    expected + 4
);
```

This is the whole field's left border column, sampled row by row and
scanned for exactly one colour change — a clean, direct way to assert "the
raster split happened exactly once, at approximately the right line" without
predicting the exact instruction-cycle timing by hand. The tolerance window
(`expected..=expected+4`) is the honest acknowledgment of a chain of real
delays: the timer's countdown doesn't start the instant the CPU boots, it
starts when the `$FF95` write executes — a handful of instructions into the
setup program, itself already partway through line 0. From there,
`tick_timer` (`gime.rs:382-394`) counts down from `SPLIT_LINE + TIMER_RELOAD_OFFSET`
(the reload always adds the hardware's documented `+2` "reload offset" —
`gime.rs:162-165` — on top of the programmed value), one tick per `end_of_line`
call since `TINS` selects the horizontal-sync rate. That's `SPLIT_LINE + 2`
scanlines of countdown before the FIRQ source even latches — and then the
CPU still has to notice it (interrupts are recognized at instruction
boundaries only, week 6), take the FIRQ, and run three ISR instructions
before the border write actually lands. All of that stacks up to "a few
lines later than `SPLIT_LINE` exactly," which is precisely what the
`+4`-line tolerance is there to absorb, and precisely why hand-computing an
exact cycle count for this test would be more fragile than useful. The test
proves the *mechanism* — timer hardware, FIRQ delivery, a live-register
raster write, from genuine 6809 code — not a specific cycle count.

## 9.11 Why demos did this on real hardware

Once you've read that test, the motive for raster splits stops being
abstract. The CoCo 3's palette is 16 *simultaneous* registers — that number
never changes no matter what resolution or bit depth is selected — but
nothing stops a program from reprogramming those 16 registers partway down
the screen. A status bar at the bottom of a game screen, drawn in colours
that would clash with the play field above it, can get its own 16-colour
palette for free: set the game's colours, run the beam down to the status
bar's first line, rewrite all 16 palette registers (a `PALETTE`-statement's
worth of writes, or the raw `STA $FFBx ×16` a machine-language routine would
do), and the bottom band renders in an entirely different colour scheme —
still only 16 colours *at any given instant*, but more than 16 colours *on
screen*, because "on screen" spans more than one instant. A border split
(exactly what the FIRQ test demonstrates) is the same trick applied to the
cheapest possible canvas: no video RAM at all, just one register, timed off
one interrupt, to turn a plain rectangular border into a two- or three-band
frame around the action — purely decorative, purely free, and purely a
product of understanding that the raster is a *sequence*, not a snapshot.
Split-screen games (a status HUD with its own scroll position, distinct from
a playfield that scrolls underneath it) go one step further and rewrite the
*video base* — except, as §9.9's third test proved, that register is
field-latched, not live, so real split-screen effects on real hardware had
to be built from two independent *fields* interleaved by persistence of
vision, or from switching which fixed video-RAM window is being displayed at
a boundary the field-latch actually respects, never from a video-base write
mid-field. Knowing which registers are live and which are latched isn't
just an emulator-accuracy detail — it's the exact same knowledge a 1988
demo-scene programmer needed to know which effects were even possible.

This is also, from the emulator author's chair, the entire justification for
building `render_scanline` to paint one canvas row per call instead of
snapshotting the whole field at once (the way week 8 first showed it, and
the way the CoCo 1/2 legacy path in §9.16 still does it). A whole-field
snapshot renderer is simpler to write and faster to run, and it is *provably
wrong* the moment any real program does what this section just described —
it can only ever show the state of the registers at the one instant it
happened to sample them. `FieldScan` exists, as a named struct with a
documented latch/live split, specifically so the emulator's picture matches
what a CRT actually painted: continuously, one line at a time, at the mercy
of whatever the CPU had written by the time the beam got there.

---

## 9.12 Legacy VDG graphics: the two-chip tango

Step back from the GIME-native modes entirely. Long before `HSCREEN` and
`$FF98`, the CoCo 1/2's actual video chip — the Motorola MC6847 VDG — drove
a family of resolution-graphics modes that BASIC exposed as `PMODE 0`
through `PMODE 4`. The CoCo 3 has no MC6847 at all; the GIME's own
CoCo-compatible path (INIT0 `COCO`=1) *imitates* one closely enough that the
same BASIC programs, and the same POKEs, still work. This is legacy content
in the strict sense — a compatibility surface, not something week 8's
GIME-native pipeline touches — but it's the modes you actually POKEd at as a
kid, and it hides a genuinely strange piece of hardware history: **two
separate chips decided two separate axes of the same picture, and they never
had to agree.**

The horizontal geometry — how many bytes get fetched per row, how many bits
each pixel takes, which colour set applies — comes entirely from PIA1 `$FF22`,
the same register that also carries the VDG's alphanumeric/semigraphics
switch:

```rust
/// PIA1 $FF22 bit 7: 1 = VDG graphics, 0 = alphanumeric/semigraphics.
pub const VDG_AG: u8 = 0x80;
/// PIA1 $FF22 bit 3: colour-set select (picks which GIME palette registers apply).
pub const VDG_CSS: u8 = 0x08;
```

plus three bits, `GM2:GM1:GM0`, in `$FF22` bits 6–4. But the *vertical*
geometry — how many actual RAM rows get fetched before the picture repeats,
and how many times each fetched row gets redrawn to fill the fixed 192-line
active area — comes from an entirely different chip: the SAM's `V0–V2`
strobes, `$FFC0–$FFC5` (or, on a CoCo 3, the GIME's SAM-compatibility
overlay at the same addresses). BASIC always programs matching `GM`/`V`
pairs when you type `PMODE n`, so on stock software the two axes always
agree and this split is invisible. But the split is real, and the module
doc comment says so plainly (`video/graphics.rs:1-11`):

```rust
//! All VDG graphics modes scan out into the same 256×192 active area as text, so
//! lower-resolution modes are pixel-doubled to fill it. The horizontal decode (bytes
//! per row, bits per pixel, colour set) comes from PIA1 $FF22 (A/G, GM2–0, CSS); the
//! display base from the SAM page register; and the actual colours from the GIME
//! palette (SEB Fig 13). The *vertical* cadence (how many RAM rows are fetched, and
//! how many times each is repeated to fill the 192-line active area) instead comes
//! from the SAM V0–V2 bits — see [`LEGACY_GFX_LINES_PER_ROW`]. Real hardware doesn't
//! reconcile the two: if a program sets V and GM to a non-standard pairing, the
//! vertical cadence follows V and the horizontal decode follows GM independently.
```

"Real hardware doesn't reconcile the two" is not a defensive disclaimer —
it's a fact this codebase deliberately preserves rather than papers over. A
program that pokes `$FF22` and the SAM V strobes out of their documented
pairing (whether by a bug or on purpose, chasing an undocumented mode) gets
whatever the independent combination of the two axes actually produces on
real silicon, and `decode_vdg_graphics` reproduces that rather than
"helpfully" snapping to the nearest legal `PMODE`.

### The SAM V bits: vertical cadence

```rust
/// Lines-per-row for CoCo-compatible legacy graphics, indexed by the SAM V bits
/// packed `V2:V1:V0` (0–7). Hardware-verified (MAME
/// `gime_legacy_lines_per_row_graphic`): RAM rows fetched = `ACTIVE_H /
/// LEGACY_GFX_LINES_PER_ROW[v]` (64, 96, or 192), each repeated this many times
/// vertically to fill the 192-line active area.
pub const LEGACY_GFX_LINES_PER_ROW: [usize; 8] = [3, 3, 3, 2, 2, 1, 1, 1];
```

Read this table as "how many times each fetched RAM row gets redrawn before
the scan moves to the next one." `V=%000..010` (values 0–2) repeat every row
three times — `192 / 3 = 64` RAM rows fetched for the whole screen; `V=%011..100`
repeat twice — 96 rows fetched; `V=%101..111` repeat once — the full 192
rows, one RAM row per scan line, no doubling at all. This is a *coarseness*
knob, entirely independent of how wide a row is or how many colours it
holds — a mode can be simultaneously "128 pixels wide, 2 colours"
(a horizontal decision, from `$FF22`) and "every RAM row shown three times"
(a vertical decision, from the SAM), and that pairing is exactly `PMODE 0`
(`RG2`, `V=%000`).

### The GM bits: horizontal decode, and the PMODE table

```rust
pub fn decode_vdg_graphics(ff22: u8, sam_video: u8) -> VdgGraphicsMode {
    let gm = (ff22 & VDG_GM_MASK) >> VDG_GM_SHIFT;
    // (logical width, 4-colour?) for GM2..GM0 = 0..7.
    let (logical_w, four_colour) = match gm {
        0 => (64, true),   // CG1
        1 => (128, false), // RG1
        2 => (128, true),  // CG2
        3 => (128, false), // RG2  (PMODE 0)
        4 => (128, true),  // CG3  (PMODE 1)
        5 => (128, false), // RG3  (PMODE 2)
        6 => (128, true),  // CG6  (PMODE 3)
        _ => (256, false), // RG6  (PMODE 4)
    };
    let bpp = if four_colour { 2 } else { 1 };
    let lines_per_row = LEGACY_GFX_LINES_PER_ROW[(sam_video & SAM_VIDEO_MASK) as usize];
    VdgGraphicsMode {
        bytes_per_row: logical_w * bpp / 8,
        rows: ACTIVE_H / lines_per_row,
        bpp,
        logical_w,
    }
}
```

Lay the whole `GM` table out with the BASIC name — the number every CoCo
owner actually typed — and the horizontal geometry `decode_vdg_graphics`
derives from it:

| `GM` | MC6847 name | BASIC | `logical_w` | colours | `bpp` |
|---|---|---|---|---|---|
| `000` | CG1 | — | 64 | 4 | 2 |
| `001` | RG1 | — | 128 | 2 | 1 |
| `010` | CG2 | — | 128 | 4 | 2 |
| `011` | RG2 | `PMODE 0` | 128 | 2 | 1 |
| `100` | CG3 | `PMODE 1` | 128 | 4 | 2 |
| `101` | RG3 | `PMODE 2` | 128 | 2 | 1 |
| `110` | CG6 | `PMODE 3` | 128 | 4 | 2 |
| `111` | RG6 | `PMODE 4` | 256 | 2 | 1 |

Two things jump out. First, `PMODE 0`–`4` only ever reaches `GM` values 3–7
— `CG1`/`RG1`/`CG2` (`GM` 0–2) exist on the chip and in this decode table,
but stock Extended Color BASIC's `PMODE` statement never programs them; they
were reachable only by POKEing `$FF22` directly, which is exactly the kind
of "undocumented mode" corner the module doc comment above was talking
about. Second, `bytes_per_row = logical_w * bpp / 8` is the whole horizontal
byte-count story: `PMODE 4` (`RG6`, 256 pixels × 1 bit/pixel) is `256/8 = 32`
bytes per row; `PMODE 3` (`CG6`, 128 pixels × 2 bits/pixel) is
`128*2/8 = 32` bytes too — same memory footprint per row, traded for
resolution vs. colour depth, which is the entire point of a "chunky vs.
planar" resolution/depth tradeoff table.

Now the vertical half. This codebase's test suite explicitly documents the
*exact* SAM `V` value BASIC's `PMODE` setup pairs with three of these modes
— `render_graphics.rs`'s own comment calls out "SEB Unravelled II's GM/V
pairing table: the SAM V value BASIC's PMODE setup programs alongside each
PIA1 $FF22 GM value," backing three named constants: `V_RG3 = 0b101`
(`PMODE 2`), `V_CG6 = 0b110` (`PMODE 3`), `V_RG6 = 0b111` (`PMODE 4`). Run
those three through `LEGACY_GFX_LINES_PER_ROW` and you get each `PMODE`'s
real vertical cadence: `PMODE 2` and `PMODE 3` both fetch `192/1 = 192` RAM
rows (`LEGACY_GFX_LINES_PER_ROW[0b101] == LEGACY_GFX_LINES_PER_ROW[0b110] == 1`
— no vertical repetition at all), and `PMODE 4` fetches the same 192 rows
(`LEGACY_GFX_LINES_PER_ROW[0b111] == 1`) too. `PMODE 0` and `PMODE 1`'s
exact `V` pairing isn't named anywhere in this codebase's source or test
comments, so this chapter won't invent one — but their vertical cadence is
still fully pinned down by which *bucket* of `LEGACY_GFX_LINES_PER_ROW`
values (`3` or `2`) the real hardware's documented `PMODE`-to-`V` mapping
falls into, which is exactly what §9.17 exercise 9.5 asks you to reason
through for a mode this chapter didn't hand you the answer for.

`sam_video.rs` tests exactly the SAM-strobe half of this independently of
any graphics decode — the V bits are just three more write-only strobe
pairs, same shape as the page bits week 5 introduced:

```rust
#[test]
fn vdg_strobes_latch_sam_video_bits_independently() {
    let mut b = bus();
    assert_eq!(b.gime.sam_video, 0, "V bits reset to 0");
    b.write(V0_SET, 0); // written data is ignored — only the address matters
    assert_eq!(b.gime.sam_video, 0b001);
    b.write(V2_SET, 0xFF);
    assert_eq!(b.gime.sam_video, 0b101);
    b.write(V1_SET, 0);
    assert_eq!(b.gime.sam_video, 0b111);
    b.write(V0_CLEAR, 0);
    assert_eq!(b.gime.sam_video, 0b110);
    b.write(V2_CLEAR, 0);
    b.write(V1_CLEAR, 0);
    assert_eq!(b.gime.sam_video, 0);
}
```

`write(V0_SET, 0)` — the value written, `0`, is discarded entirely; only the
*address* (`$FFC1`, the odd half of the `V0` strobe pair) matters, exactly
like every other SAM-compat strobe. A companion test,
`vdg_strobes_do_not_disturb_the_adjacent_page_bits`, checks the V strobes at
`$FFC0–$FFC5` and the page-select strobes starting at `$FFC6` don't bleed
into each other — cheap insurance against an off-by-one in the address
decode that would otherwise be invisible (both fields are just bitfields in
`GIME`; a decode bug wiring `V2_SET` to `sam_page` instead of `sam_video`
would compile fine and only show up as a garbled screen).

### Worked example: unpacking pixels

`render_graphics.rs`'s tests exercise the actual bit-unpacking, MSB-first,
for both bit depths — this is the part of the pipeline that turns bytes into
colour indices:

```rust
#[test]
fn two_color_unpacks_msb_first_with_border() {
    let mode = decode_vdg_graphics(RG6, V_RG6); // 1:1, no scaling
    let mut data = vec![0u8; mode.bytes_per_row * mode.rows];
    data[0] = 0b1000_0000; // only the leftmost pixel is colour 1
    let mut fb = fb();
    render_graphics(&data, &mode, &[C0, C1], BD, &mut fb);

    assert_eq!(px(&fb, 0, 0), BD, "corner is border");
    assert_eq!(px(&fb, BORDER, BORDER), C1, "MSB pixel = colour 1");
    assert_eq!(px(&fb, BORDER + 1, BORDER), C0, "next pixel = colour 0");
}

#[test]
fn four_color_maps_two_bit_values_and_doubles_width() {
    let mode = decode_vdg_graphics(CG6, V_CG6); // 128 wide → hscale 2
    let mut data = vec![0u8; mode.bytes_per_row * mode.rows];
    data[0] = 0b00_01_10_11; // pixel values 0,1,2,3 left→right
    let mut fb = fb();
    render_graphics(&data, &mode, &[C0, C1, C2, C3], BD, &mut fb);

    // Each logical pixel is 2 host pixels wide.
    assert_eq!(px(&fb, BORDER, BORDER), C0);
    assert_eq!(px(&fb, BORDER + 1, BORDER), C0, "pixel 0 doubled");
    assert_eq!(px(&fb, BORDER + 2, BORDER), C1);
    assert_eq!(px(&fb, BORDER + 4, BORDER), C2);
    assert_eq!(px(&fb, BORDER + 6, BORDER), C3);
}
```

`0b1000_0000` for `RG6` (1 bit/pixel) lights exactly the leftmost of eight
pixels; `0b00_01_10_11` for `CG6` (2 bits/pixel) packs four 2-bit pixel
values, `00`, `01`, `10`, `11`, left to right within one byte — and both
follow the same "MSB first" rule the unpacking loop encodes directly
(`video/graphics.rs:119-132`):

```rust
pub fn paint_legacy_graphics_line(
    row_data: &[u8],
    mode: &VdgGraphicsMode,
    colors: &[[u8; 4]],
    xscale: usize,
    out: &mut [u8],
) {
    let pixels_per_byte = 8 / mode.bpp;
    let mask = (1u8 << mode.bpp) - 1;
    let mut x = 0;
    for bx in 0..mode.bytes_per_row {
        let byte = row_data.get(bx).copied().unwrap_or(0);
        for j in 0..pixels_per_byte {
            // Pixels are packed MSB-first within the byte.
            let shift = 8 - mode.bpp * (j + 1);
            let value = ((byte >> shift) & mask) as usize;
            let color = colors[value.min(colors.len() - 1)];
            paint_px(out, &mut x, xscale, color);
        }
    }
}
```

`shift = 8 - bpp*(j+1)`: for `j=0` (the first pixel in the byte) that's
`8 - bpp`, the *high* bits — confirming "MSB first" isn't a comment, it's
what the shift arithmetic actually does. The second test's `PMODE 3`
(`CG6`, `logical_w = 128`) doubles horizontally to fill the fixed 256-pixel
active area (`hscale = ACTIVE_W / mode.logical_w = 256/128 = 2`) — every
logical pixel becomes two adjacent framebuffer pixels, which is exactly why
`PMODE 3`'s 128×192 four-colour picture and `PMODE 4`'s 256×192 two-colour
picture both fill the identical physical screen area despite having wildly
different logical resolutions: the doubling (or tripling, for the 64-wide
modes) is baked into every mode's presentation, invisible to the program
that set it up.

A fourth test in the same file, `mismatched_v_and_gm_pairing_follows_v_for_vertical_cadence`,
is the module doc comment's disclaimer made concrete and checkable: it pairs
`GM=%111` (`RG6`'s 256-wide, 1-bpp horizontal decode) with `V=%011` — a
combination stock BASIC's `PMODE` setup never programs together (`RG6`'s
documented partner is `V=%111`) — and confirms the vertical cadence follows
`V` (`LEGACY_GFX_LINES_PER_ROW[3] == 2`, so 96 rows fetched, each shown
twice) while the horizontal decode keeps following `GM` (still 256 pixels
wide, 1 bit/pixel) regardless. The two axes really are that independent,
proven by a test that deliberately programs them out of their usual sync.

## 9.13 Where the colours come from

Pixel *values* out of `paint_legacy_graphics_line` are small integers — `0`
or `1` for two-colour modes, `0..3` for four-colour — and those integers are
indices, exactly like GIME-native graphics. What they index into depends on
bit depth and `CSS`, per SEB Unravelled II's Figure 13, reproduced directly
as two small compile-time tables (`video/graphics.rs:34-39`):

```rust
/// Palette-register indices for 2-colour modes, indexed by CSS (SEB Fig 13):
/// CSS=0 → regs 8,9; CSS=1 → regs 10,11.
const G2_PALETTE_INDICES: [[usize; 2]; 2] = [[8, 9], [10, 11]];
/// Palette-register indices for 4-colour modes, indexed by CSS (SEB Fig 13):
/// CSS=0 → regs 0–3; CSS=1 → regs 4–7.
const G4_PALETTE_INDICES: [[usize; 4]; 2] = [[0, 1, 2, 3], [4, 5, 6, 7]];
```

confirmed against `render_graphics.rs`'s own lookup test:

```rust
#[test]
fn palette_indices_follow_css_and_depth() {
    assert_eq!(vdg_palette_indices(1, 0), [8, 9].as_slice());
    assert_eq!(vdg_palette_indices(1, 1), [10, 11].as_slice());
    assert_eq!(vdg_palette_indices(2, 0), [0, 1, 2, 3].as_slice());
    assert_eq!(vdg_palette_indices(2, 1), [4, 5, 6, 7].as_slice());
}
```

On a real MC6847 (the actual CoCo 1/2 chip), those eight colours per depth
are wired to a fixed internal ROM — the chip itself decides what "colour 2
of `RG3`" looks like, and no program can change it, which is why
`video::VDG_FIXED_PALETTE` exists as a separate, hardcoded RGB table for
that variant. On a CoCo 3, though, there is no MC6847 — the GIME's
compatibility path routes those *same* eight palette-register indices
(`0–7` for four-colour modes, `8–11` for two-colour) through its own 16
programmable registers, exactly the registers you already know from GIME-
native modes and from `PALETTE`. `Machine::legacy_palette`
(`machine/video_mode.rs:62-75`) is the one place that branches on variant:

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

Two consequences worth naming. First, `self.bus.gime.color(...)` — not
`rgb_color` directly — is what resolves each of the CoCo 3's 16 palette
registers here, which means legacy `PMODE` graphics run through the exact
same RGB-vs-composite fork the rest of this chapter has been about: a
`PMODE 4` screen looks different on an RGB CoCo 3 than on a composite one,
for precisely the same reason a `HSCREEN` picture does. Second, `PMODE`
colours are *programmable* on a CoCo 3 in a way they categorically aren't on
a real CoCo 1/2 — a BASIC program can `PALETTE` its way to unusual `RG6`
colours that no MC6847-equipped machine could ever produce, because the
"chip that decides what colour index 1 means" changed from a fixed ROM to a
GIME register file, even though the pixel-index math upstream of it is
bit-for-bit identical hardware behaviour on both machines.

## 9.14 The live per-line path vs. the whole-field snapshot

One more asymmetry is worth pointing out before moving on, because it
connects straight back to §9.8–9.11's split mechanism. `Machine::paint_legacy_scanline`
(`machine/render.rs:75-171`) is the CoCo 3's **per-line** legacy renderer —
called from the very same `render_scanline` dispatcher as the GIME-native
path, once per canvas row, reading `$FF22` and `sam_video` fresh every
single line:

```rust
let ff22 = self.bus.pia1.b.output;
// ...
let ag = ff22 & video::VDG_AG != 0;
let css = ff22 & video::VDG_CSS != 0;
let sam_video = self.bus.gime.sam_video;
let (row_bytes, lines_per_row) = if ag {
    let mode = video::decode_vdg_graphics(ff22, sam_video);
    let lpr = video::LEGACY_GFX_LINES_PER_ROW[(sam_video & 0x07) as usize];
    (mode.bytes_per_row, lpr)
} else {
    (video::COLS, video::CELL_H)
};
```

Every register this function touches — `$FF22`, the SAM V bits, the border
(via `video::legacy_border_value(ff22)`) — is read live, exactly like the
GIME-native "live" group from §9.8's table. A raster split on a CoCo-3-in-
legacy-mode screen — `PMODE`'s colour set flipped, or the graphics/alpha bit
toggled, partway down the picture — works exactly as well as it does in
native mode, for the identical structural reason: nothing here is cached
across lines except the shared `row_base`/`line_in_row` cursor, carried in
the same `FieldScan` this whole chapter has been reading about
(`field_scan.legacy = true` just picks this function instead of
`gime_video::paint_scanline`, at the `FieldScan::latch` call in
`render_scanline`).

Contrast that with a real **CoCo 1/2** — no GIME, no `render_scanline`
dispatch, no per-line anything. `Machine::render_field` skips straight past
for a CoCo 3 (`if self.config.variant == MachineVariant::Coco3 { return; }`)
and only does real work for the older machines, and what it does is render
the *entire* field in one shot at field end, from a single snapshot read
through the bus (`machine/render.rs:229-279`):

```rust
fn render_coco_graphics(&mut self) {
    self.reset_legacy_fb();
    let ff22 = self.bus.pia1.b.output;
    let sam_video = match self.config.variant {
        MachineVariant::Coco3 => self.bus.gime.sam_video,
        MachineVariant::Coco1 | MachineVariant::Coco2 => self.bus.sam.v_bits(),
    };
    let mode = video::decode_vdg_graphics(ff22, sam_video);
    // ... one shot: fetch mode.bytes_per_row * mode.rows bytes, render once
```

This isn't an oversight — a real CoCo 1/2 has no GIME sitting between the
CPU and the VDG, so there is no canonical 640×240 raster to paint into a
line at a time in the first place; the VDG has its own, entirely separate
raster geometry (`docs/coco12-plan.md` Phase 3, cited directly in the
comments here), and this codebase's CoCo 1/2 support renders it as a
whole-field snapshot rather than reimplementing a second per-scanline raster
loop for a machine whose display chip this course hasn't built a scanout
model for. Practically: **mid-field raster splits are a CoCo 3 phenomenon in
this emulator**, on both the GIME-native and CoCo-3-legacy paths, because
both go through `render_scanline`. A CoCo 1/2 target — real VDG hardware,
whole-field renderer here — cannot show one in this codebase today, though
nothing about real VDG hardware rules it out (the actual chip scans a real
raster continuously, same as the GIME does); it's a scope line this codebase
draws deliberately, not a hardware fact.

## 9.15 Artifact colours: what the code actually does

One phrase belongs in this chapter precisely because CoCo veterans expect it
and the code needs to be honest about it either way: **NTSC composite
artifact colours** — the famous trick where `PMODE 4`'s black-and-white
checkerboard of alternating 1-bit pixels resolves, on a real composite
display, into *coloured* fringes (reds and blues/cyans along vertical edges)
that were never explicitly programmed as any palette value at all. This
happens because a real NTSC composite decoder derives chroma from how
*rapidly* the luminance signal changes between adjacent pixels — a fine
alternating black/white pattern looks, to the subcarrier-phase math, exactly
like a saturated colour, even though the video hardware only ever "meant" to
send two shades of grey. Games exploited it constantly on real CoCo 1/2 and
CoCo 3 hardware to fake four- or more colours out of a nominally two-colour
mode.

Searching this codebase for that mechanism comes up empty. The string
`"artifact"` doesn't appear anywhere in `coco-core`'s video code, and having
now read both colour paths end to end, it's clear why: **there is no
pixel-pattern-dependent colour synthesis anywhere in this renderer.**
`GIME::color` (§9.2–9.5) is a pure function of a single 6-bit register
value — it has no visibility into, and no dependency on, what colour the
*neighbouring* pixel resolved to. `COMPOSITE_PALETTE`/`COMPOSITE_PALETTE_180`
model what a composite monitor does with a **fully-formed 6-bit colour
value** the GIME's own video DAC already decided to output — they capture
the analog decode of an intentional colour choice, not the decode of a
*pattern* of luminance-only pixels that was never meant to carry colour
information at all. Every legacy graphics pixel this chapter has walked
through — `RG6`'s black/white bits included — resolves through exactly the
same `vdg_palette_indices` → `legacy_palette` → `GIME::color` path as every
other colour on screen, using whatever two colours `CSS` selected from
palette registers 8/9 or 10/11. If a program sets both of those registers to
plain black and white, this emulator will render plain black and white,
full stop — no matter what bit pattern is in video RAM, and regardless of
`MonitorType`. A real composite TV showing that exact same signal would not.

That's a real, honest gap, and it's worth being precise about *why* it's a
gap rather than a bug: implementing genuine artifact colour would mean
rewriting the innermost loop of `paint_legacy_graphics_line` (and its
GIME-native `paint_graphics_row` cousin, for `HSCREEN`'s composite path) to
stop being a per-pixel colour lookup and become a small sliding-window NTSC
decoder — tracking several consecutive pixels' luminance values, their
position relative to the color subcarrier's phase (which advances a fixed,
non-integer amount per pixel clock, so the *same* bit pattern artifacts
differently depending on which screen column it starts at), and synthesizing
chroma from the transition pattern rather than looking anything up in a
64-entry table at all. That's a materially different algorithm from
everything else in this chapter — every other colour decision in this
codebase is `O(1)` per pixel with no neighbour dependence — and it only
matters at all when `MonitorType::Composite` is selected, for exactly the
subset of legacy two-colour graphics modes where programs relied on the
trick. It's exactly the kind of deferred-scope decision the course's
Appendix C is for: real, named, and left for later because the 64-entry
table correctly covers every *intentional* colour choice a program makes,
and only misses the *unintentional* colour a real analog TV invents from a
bit pattern nobody asked it to colour at all.

One more loose thread worth naming while it's fresh, because it's adjacent
to the same "the table only sees the register, not the context" limitation:
`vmode::H50` (`$FF98` bit 3, "50 Hz field rate (else 60 Hz)") is parsed as a
named constant in `gime.rs` but never *read* anywhere else in the crate.
Field rate in this codebase comes from `MachineConfig`'s `VideoStandard`
(`NTSC`/`PAL`, chosen once, at machine-configuration time, same moment as
`MonitorType`) — not from this live register a real GIME lets software
toggle mid-operation. §9.17's essay exercise asks you to reason about
exactly what would have to change to close that gap too.

---

## 9.16 Reading assignment

In this order:

1. **`crates/coco-core/src/gime/palette.rs`, all of it** (88 lines) — both
   composite tables, `unpack_rgb`, `rgb_color`, `GIME::color`. Small enough
   to read in one sitting; everything in §9.1–9.7 traces back to this file.
2. **`crates/coco-core/tests/composite.rs`, all of it** — five focused unit
   tests plus the `eou_greyscale_regression` war story. Run it and watch
   every assertion you just read pass:
   ```
   cargo test -p coco-core --test composite
   ```
3. **`crates/coco-core/src/gime_video.rs`, `FieldScan` and `paint_scanline`**
   (`gime_video.rs:117–299`) — re-read `advance_scan` in particular; it's
   the one function that's *not* purely "live" or purely "latched," and
   understanding why (cursor advances live, origin frozen) is the key to
   the fourth `scanline_split.rs` test.
4. **`crates/coco-core/tests/scanline_split.rs`, all of it** — five tests,
   the whole chapter's central claim made executable:
   ```
   cargo test -p coco-core --test scanline_split
   ```
5. **`crates/coco-core/src/video/graphics.rs`, all of it** (178 lines) —
   `decode_vdg_graphics`, the palette-index tables, the MSB-first unpacker.
6. **`crates/coco-core/tests/sam_video.rs`** and **`tests/render_graphics.rs`**,
   all of both — the SAM V-strobe tests and the PMODE-decode worked
   examples.
   ```
   cargo test -p coco-core --test sam_video --test render_graphics
   ```
7. **`crates/coco-core/src/machine/render.rs`**, all of it — `render_scanline`,
   `paint_legacy_scanline`, and the contrast with the CoCo 1/2 whole-field
   `render_coco_graphics`/`render_coco_text`.

---

## 9.17 Exercises

**9.1 — Predict the split (recall, then verify).** Without re-reading
§9.9, answer from memory: a program writes the border colour ($FF9A) at
scanline 100. A different program writes the video base ($FF9D/$FF9E) at
scanline 100. Sketch what each field looks like — where (if anywhere) does
each field show a visible seam? Now name the exact two tests in
`scanline_split.rs` that prove your answer for each case, and run them:
```
cargo test -p coco-core --test scanline_split border_write_mid_field_splits_the_border_at_the_line
cargo test -p coco-core --test scanline_split video_base_write_mid_field_waits_for_the_next_field
```
If your prediction for the video-base case was "it also splits mid-field,"
find the exact line in `FieldScan::latch` that makes that impossible, and
explain in one sentence why the test plants *two* marker bytes instead of
one.

**9.2 — Composite table lookup drill (read, by hand).** Using only the
`COMPOSITE_PALETTE` array printed in §9.3 — no running code yet — decode
palette values `0x05`, `0x15`, `0x25`, and `0x35` (hue 5, all four
luminance steps) into their `(R, G, B)` triples. State whether the four
results are achromatic or chromatic, and whether brightness increases
monotonically the way the `0x00/0x10/0x20/0x30` grey ramp does. Then write a
two-line test (or a `cargo test -p coco-core --test composite -- --nocapture`
scratch assertion) confirming your hand-decoded values against
`GIME::color`. Finally: compute the *hue* (standard HSV formula) of `0x05`
and of `COMPOSITE_PALETTE_180[0x05]`, and check whether the delta is closer
to 120° or 180° — does this entry support or complicate the "BPI is a clean
180° invert" claim from §9.4?

**9.3 — Sabotage the field latch, then verify (sabotage — do this for
real).** In `gime_video.rs::paint_scanline`, the body-row fetch passes
`scan.row_base` (the field-latched origin) into `paint_body_row`. Change
that one call site to pass `g.video_base()` instead — bypassing the latch
entirely, re-reading the live vertical-offset registers on every line. Run:
```
cargo test -p coco-core --test scanline_split
```
Exactly one test should fail:
`video_base_write_mid_field_waits_for_the_next_field`. Before reading
further, predict what the failure message will show. The actual result is
more surprising than "shows palette 7 (the new base) instead of palette 5":
the sabotaged code reports `left: [0, 0, 0, 255]` — plain black, neither
candidate colour. Explain why, in terms of what else `scan.row_base`'s
per-row *advancement* (`advance_scan`) was doing that a bare
`g.video_base()` call does not. (Hint: `advance_scan` still runs and still
updates `scan.row_base` — but nothing reads that field anymore once the
fetch bypasses it. What address does every single row fetch from, now, no
matter how far down the field it is?) Revert your change and confirm
`git status` is clean and the full suite passes again before moving on.

**9.4 — Build a three-band border (build).** Extend
`border_write_mid_field_splits_the_border_at_the_line` (copy it to a new
test, don't just edit the original) to produce **three** visible border
bands in one field, not two: pick two split lines (e.g. 60 and 160, both
inside the 25..217 active body from `vertical_window`), write a different
border colour at each via `run_to_line`, and assert all three colours
appear at the right canvas rows with exactly two transitions in the left
border column (reuse the `transitions` scan from §9.10's FIRQ test as a
model, or just sample three specific rows directly). No ROM code required —
this is a pure register-poke test in the same style as the other four in
the file. Get it passing, then delete it (or keep it if you'd rather send a
PR) — the exercise is in building it and predicting *before running* which
of the two splits you expect to fall between rows.

**9.5 — PMODE geometry drill (recall, then verify).** From the table in
§9.12 and `LEGACY_GFX_LINES_PER_ROW`, work out — without looking anything up
— `bytes_per_row`, `rows`, `bpp`, and `logical_w` for: (a) `PMODE 2` (`RG3`)
with its documented `V=%101` pairing; (b) `GM=%110` (`CG6`) paired with a
*mismatched* `V=%000` instead of its documented `%110`. Then check both
against `decode_vdg_graphics` — for (a), against `render_graphics.rs`'s
`decodes_pmode_geometry` test directly; for (b), by calling
`decode_vdg_graphics(CG6, 0b000)` yourself (`CG6` is `AG | (6 << 4)` from
that test file) and inspecting the result. State in one sentence which axis
of your (b) answer changed from the "textbook" `CG6` geometry (still `V=%110`)
and which didn't — and why a program that meant to select `PMODE 3` but
fat-fingered only the SAM strobes, leaving `$FF22` untouched, would see a
picture that's still 128 pixels wide and 4 colours, just fetched from a
third as much RAM (64 rows instead of 192) with each fetched row repeated
three times vertically instead of drawn once.

**9.6 — Essay, five sentences max (no running code).** `vmode::H50` ($FF98
bit 3) is a real, named bit — a real GIME lets software toggle 50 Hz/60 Hz
field rate live, mid-operation — but nothing in this crate ever reads it;
field rate comes entirely from a static `VideoStandard` chosen once at
machine-configuration time. Sketch what would have to change to honour a
live `H50` write instead: which fixed assumption in `end_of_line`/`run.rs`
(week 6) would break first, and what would `render_scanline`'s canvas-row
math (§9.8, `CANVAS_H = 240` fixed) have to do differently mid-field if the
line count *itself* could change under it? You do not need to implement
this — the point is naming the load-bearing assumption a "just read the
register live, like the border" fix would violate that the border write
never does.

---

## What's next

Video is done. You now own the whole visible half of the machine: raster
timing (week 6), VDG text and the surprise CoCo-3-boots-in-VDG-mode fact
(week 7), GIME native text/graphics and the palette register file (week 8),
and — this week — the two ways that register file lies to a naive
whole-field renderer: it means something different depending on a cable
nobody can query, and it can change its mind mid-field in ways only some of
its registers are allowed to respect. Every remaining chapter through week
14 is a different device hanging off the same bus you mastered in week 5,
each with its own version of "state, a loop, and a seam" (week 1). Week 10
starts the input side: the PIAs, the keyboard matrix, and a joystick that
has no ADC chip at all — just a comparator, a DAC, and 1980-style software
doing successive approximation one bit at a time.
