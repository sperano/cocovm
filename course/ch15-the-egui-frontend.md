# Chapter 15 — The egui frontend: pixels, keys, and real time

*Week 15. Goal: everything host-side, for the graphics-shy. Weeks 1–14 built
a headless machine — `coco-core` renders into a `Vec<u8>`, records audio
into a `Vec<[f32; 2]>`, and has never once opened a window. This week you
finally look at the other side of the seam: `coco-egui`, the ~9,000-line
crate that turns that headless machine into something you can sit in front
of. The good news, and the whole point of this chapter: there is far less
"graphics programming" here than you fear. By the end you will have read
every line that touches a GPU in this entire codebase — there are perhaps a
dozen of them — and spent the rest of your time on arithmetic (letterboxing,
frame pacing) and plain application state (menus, a VM manager, media
attach/eject). If you know Rust and you know what the CoCo 3 did, you
already have everything you need for this chapter except one new idea:
immediate-mode GUI, which §15.1 builds from nothing.*

---

## 15.1 Immediate mode, from zero

If you've done any GUI programming before, it was almost certainly
**retained-mode**: Qt, the DOM, Swing, Cocoa. You construct a tree of widget
*objects* once — a `QPushButton`, a `<div>`, a `JLabel` — the framework
retains that tree for the life of the window, and from then on you *mutate*
it: `button.setText("Pause")`, `label.textContent = "42"`. The framework
watches for changes to the tree and figures out what to repaint. Your
program's UI state and the framework's widget tree are two separate things
that you're responsible for keeping in sync — and "my button and my model
disagree" is a whole category of bug retained-mode UI is famous for.

egui (and `eframe`, the thin windowing/backend layer around it that this
crate is actually built on) is **immediate-mode**. There is no persistent
widget tree at all. Instead, your `eframe::App` implements one method,
called once per frame:

```rust
impl eframe::App for CocoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.window_ui(ctx);
    }
}
```

(`crates/coco-egui/src/app.rs:323-325` — this is the *entire* trait
implementation; everything else in the file is plain `impl CocoApp`
methods.) Every widget you see on screen is a **function call that returns
a response**, made fresh, every single frame:

```rust
if ui.button("Reset").clicked() {
    self.machine.reset();
}
```

(`crates/coco-egui/src/chrome/toolbar.rs:13-15`.) There is no `Button`
object living anywhere between frames. `ui.button("Reset")` draws a button
at the current layout position, checks this frame's input for a click
inside its rect, and returns a `Response` whose `.clicked()` you inspect
immediately. Next frame, if this code path runs again, egui draws the
*same* button again from scratch. If you don't call `ui.button(...)` this
frame — because, say, a menu is closed — the button simply doesn't exist
this frame. There is nothing to hide, remove, or destroy.

This has a consequence you need to internalize before anything else in this
chapter makes sense: **all state lives in your struct, never in the
framework.** `self.running`, `self.aspect_correct`, `self.kb_mode` — every
one of `CocoApp`'s ~35 fields (`crates/coco-egui/src/app.rs:11-131`) is the
*entire* durable memory of the UI. A checkbox doesn't remember whether it's
checked; you do:

```rust
ui.checkbox(&mut self.aspect_correct, "4:3 aspect (F9)");
```

(`crates/coco-egui/src/chrome/menu_bar.rs:43`.) `ui.checkbox` takes a
`&mut bool`, draws the box in whichever state that bool currently holds,
and — if clicked this frame — flips it in place before returning. The
"widget" and the "model" were never two things to synchronize; there was
only ever one bool, and the checkbox is a temporary lens onto it that exists
for the duration of one function call.

### Why an emulator loves this

Now connect this to everything weeks 1–14 built. `Machine::run_field`
already redraws the *entire* CoCo screen every 1/60th of a second, from
scratch, whether or not anything changed — that's just what a raster
display is (week 6, week 7). An immediate-mode GUI does exactly the same
thing for the *window around* that screen: redraw everything, every frame,
from current state. The two halves of this program share a philosophy
before you write a line connecting them. There is no "damage tracking," no
"only redraw the status bar if the disk light changed" — you couldn't
introduce that bug if you tried, because there's no persistent tree to
selectively update. This is also precisely why `coco-core` staying headless
(week 1, §1.5) was free: the core was already built to be re-rendered
wholesale every field; handing that buffer to an immediate-mode frontend
that also re-renders everything every frame is not a mismatch to bridge,
it's the same idea twice.

The cost, to be honest about it, is CPU: real work happens every frame
whether or not the screen visibly changed (a paused, idle CoCo still walks
every menu-bar `ui.menu_button` call 60 times a second while the window has
focus). For an app this size, on modern hardware, that cost is
unmeasurable. It would matter in a 10,000-widget enterprise dashboard; it
does not matter here.

---

## 15.2 The per-frame loop: `step_emulation` and `field_debt`

Everything about advancing the emulator by wall-clock time lives in one
function, `CocoApp::step_emulation` (`crates/coco-egui/src/app/frame.rs`).
Read it in full — it's short enough to hold in your head, and it is the
single most important function in this chapter:

```rust
pub(crate) fn step_emulation(&mut self, ctx: &egui::Context) {
    self.handle_input(ctx);
    self.drive_joysticks(ctx);

    if self.running {
        for _ in 0..self.fields_due() {
            if self.type_ahead.is_active() {
                self.type_ahead.advance(&mut self.machine.bus.keyboard);
            }
            if !self.debugger.run_field(&mut self.machine) {
                self.running = false;
                break;
            }
        }
        let sample_rate = self.machine.audio_sample_rate();
        self.audio.push_samples(self.machine.take_audio(), sample_rate);
        ctx.request_repaint();
    } else {
        self.last_update = None;
        self.field_debt = 0.0;
    }

    let image = egui::ColorImage::from_rgba_unmultiplied(
        [self.machine.fb_width as usize, self.machine.fb_height as usize],
        &self.machine.framebuffer,
    );
    let texture = self.texture.get_or_insert_with(|| {
        ctx.load_texture("coco-fb", image.clone(), egui::TextureOptions::NEAREST)
    });
    texture.set(image, egui::TextureOptions::NEAREST);
}
```

Five things happen here, in order, every time `update()` runs: read host
input, drive the joysticks, run zero-or-more emulated fields, push whatever
audio those fields generated into the sound ring (week 11 owns that ring;
§15.5 below is the one-paragraph pointer), and upload the resulting
framebuffer as a texture. `window_ui` (the trait method's actual body) calls
this, then draws the menu/toolbar/status chrome, then draws the display —
in that order, every frame, no exceptions
(`crates/coco-egui/src/app/frame.rs:120-126`).

The one piece worth stopping on is `self.fields_due()` — the answer to "how
many times do I call `run_field` *this* call to `update()`?"

### Why you can't just run one field per repaint

The naive design is "one `update()` call, one emulated field." It is wrong,
and DESIGN.md flagged this back in week 6 (§4): "don't trust egui's repaint
cadence for emulation timing." Two failure modes, both real:

- **A 144 Hz gaming monitor.** eframe repaints roughly at your monitor's
  refresh rate. One field per repaint on a 144 Hz display runs the CoCo
  at 144 emulated fields per second — 2.4× real speed. Every game and
  every timing-sensitive BASIC program breaks.
- **A window drag, a slow debugger breakpoint, or the OS just being busy.**
  If `update()` isn't called for 400 ms and then is, "one field" makes the
  CoCo simply *lose* 400 ms of wall-clock time — audio glitches, the field
  sync IRQ (week 6) that stock BASIC idles on falls behind, and the tape
  motor (week 12), which times its own mechanics in real seconds, drifts.

The fix is the **`field_debt` accumulator**, and it's genuinely just one
small function:

```rust
pub(crate) fn fields_due(&mut self) -> usize {
    let now = std::time::Instant::now();
    let dt = match self.last_update.replace(now) {
        Some(prev) => (now - prev).as_secs_f64().min(MAX_FRAME_DT),
        None => 0.0,
    };
    self.field_debt += dt * self.machine.config.video.field_rate_hz();
    let due = (self.field_debt as usize).min(MAX_FIELDS_PER_UPDATE);
    self.field_debt = (self.field_debt - due as f64).min(1.0);
    due
}
```

(`crates/coco-egui/src/app/frame.rs:9-19`.) Walk it exactly once, slowly:

1. **Measure real elapsed time** since the previous call, `dt`, in seconds.
   The very first call after `(re)start` has no previous instant
   (`self.last_update` was `None`), so it credits zero elapsed time — the
   emulator doesn't try to "catch up" on the time before it existed.
2. **Convert `dt` to fields owed**, at the machine's own field rate
   (`VideoStandard::field_rate_hz()` — NTSC 59.94 Hz, PAL 50.0 Hz,
   `crates/coco-core/src/config.rs:58-61`, week 6), and *add* that to
   `field_debt` — a fractional running balance, not reset each call.
3. **Run the whole fields owed**, `due = floor(field_debt)`, capped at
   `MAX_FIELDS_PER_UPDATE`.
4. **Carry the fraction forward.** Subtract only the whole fields just paid
   out, so a debt of `2.7` fields becomes `0.7` — not zero. That `0.7`
   is still owed next call, and *will* trigger a field once enough more
   time accumulates on top of it. No time is silently thrown away by the
   act of rounding down, ever — except by the deliberate `.min(1.0)` at the
   very end, which we'll come back to.

This is the whole idea: **decouple emulation speed from repaint cadence by
never asking "how many repaints happened" and always asking "how much
wall-clock time elapsed."**

### Working the numbers: a 120 Hz monitor

Concretely, on a 120 Hz display calling `update()` roughly every 8.33 ms,
against NTSC's 59.94 Hz field rate:

| Call | `dt` (s) | `field_debt` before | `+= dt·59.94` | `due` | `field_debt` after |
|------|---------:|---------------------:|--------------:|------:|--------------------:|
| 1    | 0.00833  | 0.000                 | 0.4995        | 0     | 0.4995               |
| 2    | 0.00833  | 0.4995                | 0.9990        | 0     | 0.9990               |
| 3    | 0.00833  | 0.9990                | 1.4985        | 1     | 0.4985               |
| 4    | 0.00833  | 0.4985                | 0.9980        | 0     | 0.9980               |
| 5    | 0.00833  | 0.9980                | 1.4975        | 1     | 0.4975               |

A field runs roughly every *other* repaint — never every repaint — because
each individual 120 Hz tick only owes half a field. Averaged out, that's
one field roughly every 16.68 ms: 59.94 Hz, exactly the CoCo's real rate,
regardless of the display refreshing at 120 Hz. This is precisely the
`field_debt` doc comment's promise (`crates/coco-egui/src/app.rs:28-31`):
"120 Hz displays no longer run the CoCo at double speed." Try the same
table at 60 Hz (`dt ≈ 0.01667`) and you'll see `due` is 1 on almost every
call, with a small amount of jitter back and forth because 60 Hz repaints
and 59.94 Hz fields aren't *quite* the same rate either — the accumulator
absorbs that drift the same way, one fractional field at a time, instead of
ever needing a special case.

### The two guardrails, and the spiral of death

Two constants sit right next to `field_debt`'s owner in `main.rs`:

```rust
/// Cap on emulated fields run in one UI update: catches up after short host
/// stalls (~130 ms) but drops time beyond that instead of spiralling.
pub(crate) const MAX_FIELDS_PER_UPDATE: usize = 8;
/// Longest wall-clock gap credited to the emulation clock, in seconds. Gaps
/// beyond this (window drag, app hidden, debugger pause) are discarded.
pub(crate) const MAX_FRAME_DT: f64 = 0.25;
```

(`crates/coco-egui/src/main.rs:88-93`.) Both exist to prevent the same
disease: **the spiral of death.** Imagine there were no cap on fields per
update. Your host stalls for a second — a window manager hiccup, a
laptop waking from sleep, whatever. `field_debt` jumps to roughly 60. The
next `update()` call dutifully tries to run 60 fields *before* returning
control to the UI. But running 60 fields of CPU emulation, video scanout,
and audio rendering takes real wall-clock time too — say it takes 200 ms on
this machine. During those 200 ms, *more* real time has elapsed than the
frame accounted for, so `field_debt` is already non-zero again by the time
you check next. If the emulator can't emulate fields faster than real time
allows (true on a loaded system, or a debug build, or a slow machine), the
backlog never shrinks — it grows every single update, the UI stops
responding to input because it's permanently "catching up," and the
program is functionally hung while still burning 100% CPU. That's the
spiral: falling behind makes you fall further behind, forever.

`MAX_FIELDS_PER_UPDATE = 8` breaks the spiral by refusing to try to catch up
past a bounded amount of work per frame — the excess is simply not run this
call. `MAX_FRAME_DT = 0.25` attacks the same problem from the other end: a
genuinely enormous gap (the window was minimized for ten minutes) is
clamped to a quarter of a second's worth of *credited* time before it ever
reaches `field_debt`, so the accumulator never even sees the huge number in
the first place.

Now look again at the last line of `fields_due`:

```rust
self.field_debt = (self.field_debt - due as f64).min(1.0);
```

Notice this clamps the *carried-over remainder* to at most `1.0`, not just
the amount paid out. Work through the 250 ms-stall example: suppose
`field_debt` was `0.997` and a 200 ms gap arrives (under the 250 ms cap, so
uncredited-clamping doesn't even kick in). That adds `0.2 × 59.94 ≈ 11.99`
fields of debt, for a total of `12.985`. `due` is capped at `8`. After
subtracting, `12.985 − 8 = 4.985` fields would still be owed — nearly five
more fields' worth of "catch-up" pressure sitting in the accumulator,
which would otherwise make the *next several* frames also run at the
8-field cap, extending the visible slowdown well past the original stall.
The `.min(1.0)` throws that extra backlog away outright: after any update,
at most one field's worth of fractional debt survives, ever. A stall costs
you smoothness for exactly one clamped burst of up to 8 fields, and then
the clock is caught up to "now" — not caught up to "everything you
missed." This is a deliberate, opinionated policy: **prefer time perceived
as real over exact accounting of missed field count.** An emulator that
insisted on running every field it was ever "owed" would, after any real
stall, visibly fast-forward through the backlog — CoCo-native BASIC
programs sound and look wrong sped up, so the codebase chooses to drop the
excess instead.

Pausing gets its own paragraph in the `else` branch: `self.last_update` is
reset to `None` and `field_debt` to `0.0`. Without this, a debugger
breakpoint held for ten seconds would otherwise be interpreted, on
resuming, as ten seconds of owed field time — the emulator would burst
through hundreds of fields the instant you hit Continue. Resetting both
means resuming is a clean restart of the pacing clock, not a catch-up.

> **Rust corner — why `f64`, not `f32`, for `field_debt`.** `field_debt`
> accumulates a tiny fractional remainder *every single frame*, potentially
> for hours of continuous play. `f32` has roughly 7 decimal digits of
> precision; accumulated rounding error across millions of additions to a
> `f32` would eventually visibly drift the emulator's timing (the same
> failure mode as summing many small `f32` deltas in any long-running
> simulation). `f64`'s ~15-16 digits push that drift far below anything a
> human — or a tape-loader's timing tolerance — could ever notice. When you
> see an accumulator meant to run for a program's entire lifetime, reach for
> `f64` by default; reserve `f32` for values recomputed fresh each frame
> (like the framebuffer's pixel geometry in §15.3) where error can't build up.

---

## 15.3 Graphics programming, demystified

Here is the section that fulfils the syllabus's promise. The *entire*
GPU-facing surface of this program is: one texture upload per frame, and
one textured rectangle drawn over it. You already read the upload half at
the bottom of `step_emulation` above — reread it now that you know why it
runs unconditionally, every frame, whether or not the CoCo's screen
actually changed (immediate mode, again: there's no "did the pixels
change" check, because there's no retained copy to compare against):

```rust
let image = egui::ColorImage::from_rgba_unmultiplied(
    [self.machine.fb_width as usize, self.machine.fb_height as usize],
    &self.machine.framebuffer,
);
let texture = self.texture.get_or_insert_with(|| {
    ctx.load_texture("coco-fb", image.clone(), egui::TextureOptions::NEAREST)
});
texture.set(image, egui::TextureOptions::NEAREST);
```

`self.machine.framebuffer` is the plain `Vec<u8>` of RGBA bytes week 7
taught you to render into — the exact same buffer the headless PPM-writing
examples in `coco-core/examples/` dump to disk. `ColorImage::from_rgba_unmultiplied`
just wraps that byte slice with its width/height as a CPU-side image
description; no GPU interaction has happened yet. `get_or_insert_with`
allocates a GPU texture handle exactly *once*, the first frame — every
frame after that reuses the same handle. `texture.set(...)` is the one line
in this entire crate that actually crosses into GPU territory: it uploads
this frame's bytes into that already-allocated texture, replacing last
frame's contents. `egui::TextureOptions::NEAREST` tells the GPU "when this
texture is scaled up or down, sample the nearest source pixel, don't
blend neighbors" — this is what keeps the CoCo's blocky low-res pixels
crisp instead of blurry when stretched to fill a modern monitor; the
alternative, `LINEAR`, is what the manager's list-row *photo* thumbnails
use instead (`crates/coco-egui/src/manager.rs:310`), because a photograph
benefits from smoothing and a 288-pixel-wide CoCo screen does not.

That's the upload. The draw is `draw_display`
(`crates/coco-egui/src/app/frame.rs:83-108`), and it's arithmetic, not
graphics API calls:

```rust
pub(crate) fn draw_display(&mut self, ui: &mut egui::Ui) {
    let tex = self.texture.as_ref().unwrap();
    let tex_size = tex.size_vec2();
    let aspect = if self.aspect_correct {
        TARGET_ASPECT
    } else {
        tex_size.x / tex_size.y
    };
    let avail = ui.available_rect_before_wrap();
    let mut w = avail.width();
    let mut h = w / aspect;
    if h > avail.height() {
        h = avail.height();
        w = h * aspect;
    }
    let rect = egui::Rect::from_center_size(avail.center(), egui::vec2(w, h));
    let sized = egui::load::SizedTexture::new(tex.id(), rect.size());
    ui.put(rect, egui::Image::new(sized));
    self.display_rect = rect;
}
```

`ui.put(rect, egui::Image::new(sized))` is the second and last GPU-facing
call in the whole program — draw one textured quad, sized to `rect`. Every
line above it is deciding what `rect` should *be*. This is the entirety of
"3D graphics" in this codebase: one 2D rectangle, textured, no shaders you
write, no vertex buffers you manage, no camera, no lighting. egui and its
backend (glow/OpenGL, or optionally wgpu — see `Cargo.toml`'s `[features]`)
handle turning "draw this rect with this texture" into actual draw calls;
this file never touches that layer.

### Letterboxing, worked with real numbers

The rest is the "largest rectangle of a given aspect ratio that fits inside
a panel, centered" problem — the same problem that puts black bars around
a widescreen movie on an old 4:3 television, just computed in the other
direction. Trace the algorithm:

1. Decide the *target aspect ratio*, independent of the texture's actual
   pixel dimensions: `TARGET_ASPECT = 4.0 / 3.0` when aspect correction is
   on (real NTSC picture shape), or the texture's own raw `width/height`
   when it's off.
2. Assume the available panel's *full width* first: `h = w / aspect`.
3. If that guess is *taller* than the panel, the width assumption was
   wrong — clamp to the panel's full height instead and recompute `w` from
   that: `w = h * aspect`.
4. Center the resulting `w × h` rectangle in the panel. Whatever's left
   over on the unconstrained axis is the letterbox (or pillarbox) margin —
   `ui.put` never draws anything there; it stays whatever `CentralPanel`'s
   background fill is (`egui::Color32::BLACK`, set in `window_ui`).

Now plug in the numbers the syllabus asked for: a CoCo 3's canonical
640×240 raster canvas (`raster::CANVAS_W`/`CANVAS_H`, week 7), inside a
1920×1080 window. Subtract the fixed chrome heights `coco-egui` reserves —
`MENU_BAR_H` (22) + `TOOLBAR_H` (30) + `STATUS_BAR_H` (22) = 74 px — leaving
a `CentralPanel` of roughly **1920 × 1006**.

**Aspect-corrected** (`aspect = 4/3 ≈ 1.3333`):

```
w = 1920                  (try full width)
h = 1920 / 1.3333 = 1440  (taller than the 1006 available!)
→ clamp: h = 1006
  w = 1006 × 1.3333 = 1341.3
```

Final rect: **1341 × 1006**, centered — pillarboxed, with roughly
`(1920 − 1341.3) / 2 ≈ 289` px of black bar on the left and right. Height
was the binding constraint.

**Aspect-uncorrected** (`aspect = tex_size.x / tex_size.y = 640/240 ≈
2.6667` — the raw, non-square-pixel shape of the canvas itself):

```
w = 1920                 (try full width)
h = 1920 / 2.6667 = 720  (fits inside 1006 — no clamp needed)
```

Final rect: **1920 × 720**, centered — letterboxed, with roughly
`(1006 − 720) / 2 ≈ 143` px of black bar on top and bottom. Width was the
binding constraint this time, the opposite branch of the `if`.

Two different final rectangles, same algorithm, same source texture — the
only thing that changed was which `aspect` value was fed in. This is the
whole payoff of computing `aspect` *before* the fit logic runs, as a
mode-agnostic scalar, rather than hard-coding "stretch to 4:3" into the
layout math itself (the doc comment on `draw_display` calls this out
explicitly: "This keeps the frontend mode-agnostic — any renderer's buffer
size fits" — `crates/coco-egui/src/app/frame.rs:86-88`).

### Why the pixels aren't square in the first place

One more number worth internalizing, from `main.rs`'s own doc comment:

> Physical aspect the CoCo frame fills on an NTSC set (4:3). The
> framebuffer is 288×224 (≈1.29:1); when aspect correction is on, the image
> is stretched horizontally to this ratio so pixels are ~3% wider than
> tall, as on real hardware.
> (`crates/coco-egui/src/main.rs:84-87`)

That 288×224 figure is `coco_core::video::FB_W`/`FB_H` — the CoCo 1/2
legacy VDG canvas (`crates/coco-core/src/video.rs:34-38`: 256×192 active
area plus a 16-pixel border on every side). `4/3 ÷ (288/224) ≈ 1.037` — a
3.7% horizontal stretch, matching the "~3%" in the comment. This is not an
emulator quirk to apologize for: real NTSC CoCos drove non-square pixels
onto a 4:3 tube exactly this way, because the hardware's dot clock and the
television's physical aspect ratio were never designed to agree pixel-for-
pixel. `TARGET_ASPECT` is the frontend choosing to reproduce that
historical mismatch rather than "fix" it into square pixels no CoCo owner
ever actually saw.

One curiosity worth a passing note, precisely because it demonstrates
immediate mode's forgiving nature: `boot::native_options` (the function
that picks the *initial* OS window size before any frame has run) computes
that starting height from `coco_core::video::FB_H` — the fixed 224-pixel
legacy figure — for *every* machine variant, including a CoCo 3 whose real
canvas is 640×240. That's an approximation, not a bug: the number only
seeds the window's starting size. `draw_display` never consults it —
every frame it re-reads `ui.available_rect_before_wrap()` fresh and
recomputes the fit from scratch. Get the initial guess wrong and the worst
that happens is the user sees one frame's worth of a slightly mis-sized
window before the very next layout pass corrects it. There is no persisted
layout state to get *permanently* wrong — which is, again, exactly what
immediate mode buys you.

---

## 15.4 Input routing: two keyboards, one matrix

Host input enters through one function per frame, `CocoApp::handle_input`
(`crates/coco-egui/src/app/input.rs:25-43`), called at the very top of
`step_emulation` — before any field runs, so the matrix state a field sees
is this frame's, not last frame's:

```rust
pub(crate) fn handle_input(&mut self, ctx: &egui::Context) {
    self.consume_app_shortcuts(ctx);

    let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
    self.handle_hotkeys_and_paste(&events);
    if self.kb_mode == KbMode::Symbolic {
        self.queue_symbolic_taps(&events);
    }

    if self.type_ahead.is_active() {
        return;
    }
    if self.kb_mode == KbMode::Positional {
        self.drive_matrix_positionally(&events, mods);
    }
}
```

`ctx.input(|i| ...)` is egui's own read of this frame's raw events — every
key press/release, mouse move, paste, etc. since the last frame — handed to
you as a plain `Vec` you're free to iterate over multiple times. Four
things happen with it, in order: app-level shortcuts (⌘N, quick-save/load
slots — these never reach the CoCo at all), hotkeys and clipboard paste
(F9/F10/F11/F12, `Event::Paste`), symbolic-mode text queuing, and finally
— only in positional mode, and only when no paste/type-ahead burst is
still draining — direct matrix driving.

### Positional vs. symbolic: two philosophies, one matrix

`coco-egui` ships two entirely different answers to "which CoCo key does
this host keypress mean?", switchable live with F12
(`CocoApp::set_mode`), because the two answers serve incompatible goals.

**Positional** (the default) maps *physical key location* to *matrix
position*, MAME's convention: press the host key that sits where a real
CoCo key would sit, and whatever letter is actually printed on the CoCo key
underneath is what appears — SHIFT state included, exactly as the ROM's
own scan-and-shift logic (week 10) decides it. `key_to_pos`
(`crates/coco-egui/src/keymap.rs:5-43`) is a flat match from
`egui::Key` to the `(row, col)` `Pos` type week 10 defined:

```rust
K::A => (0, 1), K::B => (0, 2), K::C => (0, 3), K::D => (0, 4),
// ...
K::Minus => (5, 2),      // CoCo ':'
K::Semicolon => (5, 3),  // CoCo ';'
```

Positional mode is what a game wants: an arcade-style CoCo game reads
specific matrix rows every field (week 10's `sense()`), not ASCII
characters, and it expects "the key at this physical spot" to behave
identically to a real keyboard regardless of what glyph a modern OS thinks
that key produces. `drive_matrix_positionally`
(`crates/coco-egui/src/app/input.rs:115-135`) sets SHIFT/CTRL/ALT straight
from egui's `Modifiers` and then walks every keyboard event, setting each
mapped `Pos` true or false to match `pressed` — a direct, continuous
mirror of host key state onto CoCo matrix state, field after field.

**Symbolic** maps *the character you actually typed* to whichever CoCo key
(and shift state) *produces that character* — `kbd::char_key`, which
week 10 already walked in detail (§10.8: it corrects for the CoCo's
inverted shift convention, where unshifted keys show uppercase). This is
what you want for *typing*: paste a BASIC listing, or type at the prompt on
a non-US keyboard layout, and the letters that appear match the letters you
pressed, independent of physical key position. Because a host keystroke
and a CoCo matrix press aren't a 1:1 timing match — see the `TypeAhead`
discussion below — symbolic input doesn't drive the matrix directly at
all; it *queues* taps: `queue_symbolic_taps`
(`crates/coco-egui/src/app/input.rs:94-110`) turns `Event::Text` and
control keys into `(Pos, bool)` entries pushed onto
`self.type_ahead.queue`.

Both modes share one more wrinkle: **joystick keys are contested
territory.** When a joystick port is set to `JoySource::Keys` (arrows for
axes, Z/X for fire), those six keys must stop reaching the CoCo keyboard
matrix entirely, in *either* mode — `is_joystick_key`
(`crates/coco-egui/src/keymap.rs:66-76`) is consulted by both
`drive_matrix_positionally` and `queue_symbolic_taps` before they'll honor
an arrow key, so the two consumers (keyboard emulation, joystick emulation)
never fight over the same physical key.

### `TypeAhead`: why a queue, and why it's not new material here

Chapter 10 already walked `TypeAhead::advance` in full — the hold/gap state
machine that drains queued taps one emulated *field* at a time (not one
host frame at a time), because a real key press needs to survive across
multiple 60 Hz `KEYIN` scans of the ROM to register at all; a press and
release confined to a single field can land entirely between two scans and
simply vanish (§10.8's exact framing). `TYPE_HOLD_FIELDS = 2` and
`TYPE_GAP_FIELDS = 1` (`crates/coco-egui/src/main.rs:100-102`) are the
tuned constants: hold each synthesized keypress for two fields (safely
longer than one scan interval), then release for one field before the next
tap starts, so two identical consecutive characters — `"AA"` — read as two
separate keystrokes rather than one long hold that the ROM's own
debouncing (week 10) would collapse into a single `A`.

What's new to this chapter is *how the rest of the app treats the queue as
a lock*. `TypeAhead::is_active()` — non-empty queue, or a tap mid-hold/gap
— is checked in exactly one place, `handle_input`'s early return quoted
above: while a paste or symbolic-typed burst is still draining, positional
input is suppressed outright, in *both* keyboard modes. Without that gate,
a per-frame positional key-matrix write racing against a per-*field*
type-ahead tap would stomp on it unpredictably — the queue needs
uncontested ownership of the matrix for its whole draining run. Advancing
the queue itself only happens inside `step_emulation`'s field loop
(`if self.type_ahead.is_active() { self.type_ahead.advance(...) }`,
quoted in §15.2) — once per *emulated field*, never once per host frame —
which is exactly why paste timing stays correct on a 240 Hz gaming monitor
and a 30 Hz remote desktop session alike: it's paced by `field_debt`, the
same accumulator that paces everything else.

---

## 15.5 Audio: one paragraph

`step_emulation` ends its `running` branch with two lines:

```rust
let sample_rate = self.machine.audio_sample_rate();
self.audio.push_samples(self.machine.take_audio(), sample_rate);
```

`self.machine.take_audio()` drains the per-field-rendered sample grid week
11 built (`coco-core/src/audio.rs`, `machine/audio.rs` — cycle-timestamped
DAC events rendered once per scanline into a ~63 kHz oversampled grid).
`self.audio.push_samples` hands those samples to the *host* audio chain
week 11 also fully covered: the DC blocker, the Butterworth low-pass, the
linear-interpolation resampler down to the device's real output rate, the
cross-thread ring buffer a `cpal` callback drains, and the underrun fade
that replaces a click with silence when the ring runs dry. Nothing about
that chain changes in this chapter — `AudioOutput::push_samples` is simply
the one call site where the per-frame loop hands it fresh work, at exactly
the cadence `field_debt` decided fields should run. If you want the "why"
behind any of it, that's Chapter 11, not this one.

---

## 15.6 The VM manager: machines as data

Run the bare `coco` binary with no arguments and you don't get a single
booted machine — you get the **manager**: a VirtualBox/Parallels-style
window listing every machine you've defined, with Start/Pause/Stop
controls and a detail pane for editing hardware and media
(`crates/coco-egui/src/manager.rs`, module doc comment). This is a genuinely
different kind of code from everything else in this course: not emulation
at all, but *application state that happens to manage emulators* — worth
studying because it's the shape any serious frontend eventually needs
around a headless core, and because it exercises the borrow-checker
discipline from week 1 (§1.4) in a completely different setting.

### Machine definitions as data, not code

A machine is a small, human-editable TOML file,
`config_dir()/machines/<slug>.toml` — never a database, never a binary
format. `MachineDef` (`crates/coco-egui/src/machine_def.rs:49-76`) is a
**DTO** (Data Transfer Object — a struct whose only job is to mirror an
external file format field-for-field, kept deliberately separate from the
`coco_core::MachineConfig` the program actually runs on):

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MachineDef {
    pub schema: u32,
    pub name: String,
    #[serde(default)]
    pub created: Option<String>,
    pub hardware: HardwareDTO,
    #[serde(default)]
    pub media: MediaDTO,
    #[serde(default)]
    pub peripherals: PeripheralsDTO,
    #[serde(default)]
    pub ui: UIDTO,
    #[serde(skip)]
    pub unknown: toml::Table,
}
```

Why a separate DTO instead of `#[derive(Serialize)]` on `MachineConfig`
itself? Because an internal `coco-core` refactor (renaming a variant,
restructuring a field) would otherwise silently change what's written to
disk, breaking every user's saved machine out from under them with no
warning. The DTO is a deliberate translation seam: `to_machine_config`
converts DTO → real config (running `MachineConfig::validate` along the
way, so an invalid hardware combination — a CoCo 2 asked for PAL, say —
fails at load with one clear error instead of at boot with a confusing
one), and it reads friendly strings on disk ("512k", "mc6847t1") instead of
whatever `coco-core`'s enum discriminants happen to be this week.

`unknown` deserves a look, because it's a small, well-engineered piece of
forward compatibility you'll want to imitate: any TOML key `load_one`
doesn't recognize (top level, or one level into a known section) is logged
via `tracing::warn!` *and* stashed in this field
(`crates/coco-egui/src/machine_def/io.rs:69-90`, `extract_unknown`) rather
than silently dropped. When the definition is saved back out —
`merge_unknown` — those unrecognized keys are folded back into the freshly
serialized table before it hits disk. The consequence: a newer build that
adds a `[hardware].turbo_multiplier` key, opened and re-saved by an older
build that doesn't know about it yet, doesn't erase that key. Editing a
machine's name in the manager must never destroy a setting a future version
of the same program wrote.

### Atomic saves: tmp, then rename

`machine_def::save` (`crates/coco-egui/src/machine_def/io.rs:189-209`)
never writes the final file directly:

```rust
let tmp_path = dir.join(format!("{slug}.toml.tmp"));
let final_path = dir.join(format!("{slug}.toml"));
fs::write(&tmp_path, text).map_err(...)?;
fs::rename(&tmp_path, &final_path).map_err(...)?;
```

Write the complete new contents to a sibling `.tmp` file first, then
`rename` it over the real path. A crash, power loss, or `kill -9` between
those two calls leaves either the untouched old file or the fully-written
`.tmp` — never a half-written `<slug>.toml`, because `rename` on every
platform this program targets is atomic with respect to a concurrent
reader: a reader either sees the file before the rename or after, never
mid-write. This exact pattern reappears verbatim for thumbnail PNGs
(`write_thumbnail_png`, `crates/coco-egui/src/manager.rs:181-198`) — the
manager applies the same crash-safety discipline to *every* file it writes
on the user's behalf, not just the one that would be embarrassing to lose.

### The slug is the identity

A machine's filename stem — its **slug** — is its persistent identity, not
its display name (`plan-machine-persistence.md`'s "Identity = slug," quoted
in the module doc). `slugify` (`crates/coco-egui/src/machine_def.rs:137-156`)
lowercases, keeps `[a-z0-9]`, collapses everything else to single dashes;
`unique_slug` appends `-2`, `-3`, … until a candidate isn't taken. Renaming
a machine (`ManagerApp::migrate_slug`,
`crates/coco-egui/src/manager/lifecycle.rs:99-152`) is consequently exactly
two filesystem moves — `<old>.toml` → `<new>.toml`, and its artifact
directory alongside it — with a rollback if the second move fails partway
(the definition file is renamed back rather than left pointing at a
directory that no longer matches its own name). Relative `[media]` paths
inside a definition resolve *against the slug's own artifact directory*
(`resolve_media_path`, `crates/coco-egui/src/machine_def.rs:193-202`), which
is precisely why the rename has to move that directory too: a disk image
referenced as `"disk0.dsk"` means a different absolute file the instant the
slug changes, unless the artifact directory moves with it.

### Start/Stop, and where the running machine actually lives

`MachineEntry` (`crates/coco-egui/src/manager.rs:102-133`) is one row: a
slug, a parsed `MachineDef`, and — the only field that changes what's
*running* — `pub vm: Option<Box<CocoApp>>`. Stopped is `None`; Start
(`ManagerApp::start_vm`, `crates/coco-egui/src/manager/lifecycle.rs:69-76`)
calls `crate::launch_machine`, the manager's counterpart to the CLI's
`boot::boot_app` (both build a `CocoApp` from a config plus a set of
mounted media — `launch.rs`'s module doc names the CLI path as its
sibling), and stores `Some(Box::new(vm))` on success or records the failure
string in `entry.launch_error` on failure, leaving `vm` untouched at
`None`. Stop (`stop_vm`) writes a final thumbnail, then flushes dirty media
(the same `flush_media` — write back modified disks and tape — that
`CocoApp::on_exit` runs for the direct-boot window) and drops the `Box`.

> **Rust corner — `Option<Box<CocoApp>>`, not `Option<CocoApp>`.**
> `CocoApp` is a large struct: the whole `Machine` (CPU, RAM, GIME, both
> PIAs, every optional cartridge device) plus every UI dialog's own state
> (the debugger panel, the "New…" form, the paper window…). Every
> `MachineEntry` in the list pays the size of this field regardless of
> whether that machine is running — including entries for machines that
> are, and always will be, `Stopped`. Boxing puts the actual `CocoApp` on
> the heap and leaves only a pointer-sized `Option<Box<_>>` inline in
> `MachineEntry`, so a manager listing fifty stopped machines carries fifty
> small `None`s, not fifty machine-sized empty slots. This is the general
> rule for "occasionally-present, expensive-to-hold" fields in Rust: box
> the payload, keep the container thin.

### One native OS window per running VM

`draw_running_vms` (`crates/coco-egui/src/manager/vm_windows.rs:45-120`) is
called once per `ManagerApp::update`, after the manager's own panels, and
opens one **immediate viewport** — a real, separate native OS window — per
entry with a live VM:

```rust
let mut vm = self.entries[i].vm.take().expect("checked Some above");
let mut close_requested = false;
ctx.show_viewport_immediate(viewport_id, builder, |child_ctx, class| {
    if class == egui::ViewportClass::Embedded {
        vm.step_emulation(child_ctx);
        // ...a plain egui::Window fallback, display only, no chrome...
    } else {
        vm.window_ui(child_ctx);
        if child_ctx.input(|i| i.viewport().close_requested()) {
            close_requested = true;
        }
    }
});
self.entries[i].vm = Some(vm);
```

`egui::ViewportId::from_hash_of(("vm-window", &slug))` gives each VM's
window a stable identity across frames, so egui reuses the *same* OS
window rather than destroying and recreating it every update — the
manager's version of the same "identity persists, widgets don't"
discipline §15.1 opened with, just applied to whole windows instead of
buttons. On a backend with real multi-window support, `class` is the
default kind and the *entire* direct-boot experience — menu bar, toolbar,
status bar, the display — runs unmodified inside this child viewport via
the very same `window_ui` this chapter has been reading all along. On a
backend without it (`ViewportClass::Embedded` — kittest, for headless
testing, is exactly this case), the fallback deliberately shows *only*
`draw_display`'s bare screen inside a small anchored `egui::Window`,
because drawing two independent sets of menu bars/status bars into one
shared `ctx` would visually interleave them into one confusing mess.

> **Rust corner — `.take()` to split a borrow, one level up from week 1.**
> Look at that snippet again: `self.entries[i].vm.take()` moves the
> `Box<CocoApp>` out of the entry *before* the closure runs, into a local
> `vm` the closure captures by unique reference — with the entry's own slot
> left holding `None` for the closure's whole duration, then restored
> afterward (`self.entries[i].vm = Some(vm)`). Why not just borrow
> `&mut self.entries[i].vm` directly inside the closure? Because the
> closure also needs to push onto `to_stop: Vec<usize>`, a plain local, and
> — more importantly — the surrounding loop is iterating `self.entries` by
> index and will look at `self.entries[i]` again right after the viewport
> call returns; holding a live borrow of `self` for the whole closure body
> would collide with that. This is the exact move from Chapter 1, §1.4
> (`Machine { cpu, bus }` as disjoint fields so `cpu.step(&mut bus)`
> compiles) recurring in ordinary application code: when the borrow
> checker won't let you hand out two overlapping mutable views of the same
> owner, take the piece you need *out*, use it standalone, and put it back.

### Thumbnails: crash insurance, not a feature

Every stopped row shows a small preview of the machine's last screen. That
preview is a plain PNG, `<artifact-dir>/<slug>/thumbnail.png`, written by
`write_thumbnail_png` with the same tmp-then-rename atomicity as machine
definitions, on **three** occasions: an explicit Stop, the manager's own
`on_exit` (so quitting with VMs still running doesn't lose their preview),
and — the interesting one — a periodic refresh every `THUMBNAIL_REFRESH =
30` seconds while a VM is running
(`refresh_due_thumbnails`, `crates/coco-egui/src/manager/thumbnails.rs:36-55`).
That periodic write exists purely as **crash insurance**: if the process is
force-killed (a real OS crash, not a graceful Stop), the on-exit write
never runs — but the *previous* 30-second refresh already left something
useful on disk, so the next launch shows a recent screen instead of a
placeholder or nothing at all.

One small heuristic keeps that safety net from actively hurting you:

```rust
let all_black = rgba
    .chunks_exact(4)
    .all(|px| px[0] == 0 && px[1] == 0 && px[2] == 0);
if all_black && final_path.exists() {
    return Ok(());
}
```

(`crates/coco-egui/src/manager.rs:183-187`.) A blanked screen — a mode
switch mid-boot, `CLS 0`, the moment right after a machine's cold reset
before the ROM has painted anything — would otherwise clobber a genuinely
useful preview with a solid black square the *next* time the 30-second
timer fires, purely by bad luck of when the snapshot landed. Skipping the
write when the frame is uniformly black *and* a previous thumbnail already
exists keeps that one unlucky moment from erasing a better picture that's
already on disk; the `final_path.exists()` half of that condition matters
too — the *very first* thumbnail a brand-new machine ever writes must still
land even if that first frame happens to be black, since skipping then
would leave the row with no preview at all. §15.11's sabotage exercise asks
you to find both halves of that guarantee by breaking one of them.

---

## 15.7 Media UI pattern: `disk.rs` as the exemplar

Every attachable device — cartridges, floppies, VHDs, DriveWire disks,
cassette, the printer bit-banger — gets its own file under
`crates/coco-egui/src/media/`, all `impl CocoApp` methods, all following
the same shape. `media/disk.rs` is the clearest one to learn the pattern
from, because floppies are the only media type with real in-memory dirty
state to manage (VHD and DriveWire images write straight through to their
backing file on every command; there's nothing to flush).

The pattern, in order:

- **Ensure the controller exists.** `ensure_disk_controller` inserts a
  `DiskCart` (week 13) if the cartridge slot is empty or holds something
  else, loading `roms/disk11.rom` and — crucially — **power-cycling**, not
  warm-resetting, the machine:

  > Creating it cold-resets the machine: BASIC only probes for Disk BASIC
  > at cold start. Swapping a floppy in an already-present controller does
  > NOT reset, like on real hardware.
  > (`crates/coco-egui/src/media/disk.rs:9-11`)

  This is the frontend enforcing a real hardware constraint your ROM
  archaeology in week 13 already uncovered from the other side: the DK
  probe that links Disk BASIC into the language only runs on the cold-start
  path, so *inserting the controller* must be a power cycle even though
  *swapping a disk in it* must not be.
- **Insert acts, write-back protects.** `insert_disk` writes back whatever
  was already in the target drive *before* mounting the new image
  (`self.write_back_disk(drive)` — never silently discard unsaved changes
  to what's being ejected), then mounts the new one and records its source
  path in `self.disk_paths[drive]`.
- **Dirty tracking lives in the device, not the frontend.** `write_back_disk`
  asks the mounted `JvcDisk` itself, `disk.dirty()` (week 13), before
  touching the filesystem at all — the frontend never guesses whether a
  disk changed; it defers entirely to the device that actually knows.
  Failure lands in `self.cart_error` and the in-memory disk is left mounted
  and still dirty, so a later retry (or the next `flush_dirty_disks` on
  exit) can succeed without losing the edit.
- **Eject always writes back first**, exactly like ejecting a real floppy
  from a real drive after the OS has finished with it — `eject_disk` calls
  `write_back_disk` before `cart.eject_disk(drive)`, never after.

Every other media file in the directory — `cart.rs`, `tape.rs`,
`drivewire.rs`, `printer.rs` — repeats this shape with the specifics that
device demands (a cartridge has no "dirty" concept at all; a cassette's
write-back optionally also synthesizes a `.wav`, per week 12). If you
understand `disk.rs`, you can read any of them cold. And if you've been
following the thread since Chapter 14's closing paragraph: this is also
where that chapter's loose end gets tied off — the DMP-105's protocol and
fixed-point paper coordinates were week 14's; the actual scrolling
"Printer Paper" window a user watches fill up while `LLIST` runs
(`crates/coco-egui/src/paper_view.rs`) is `coco-egui` UI state built on top
of it, one more example of a device (`Dmp105Handle`) attached and detached
by the same request-then-mount discipline as a floppy.

---

## 15.8 Testing the UI headlessly: kittest

Everything you've read so far in this chapter — menus, dialogs, the
manager's list, the running-VM viewports — has a real automated test suite,
and none of it opens a visible window or needs a human at a monitor. That's
`egui_kittest` (`Cargo.toml`: `egui_kittest = { version = "0.33", features
= ["eframe"] }`), and it works by running egui's *real* layout and input
logic against a headless backend, then exposing the result as an
**AccessKit** accessibility tree — the same structured tree a screen reader
would consume — which tests query by label instead of by pixel coordinate.

### Booting a harness

`ui_tests/harness.rs` is the shared infrastructure every test file in
`ui_tests/` imports. Booting a direct-boot app harness looks like this:

```rust
pub(super) fn boot_harness() -> AppHarness {
    let roms_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms");
    let rom = load_default_rom(MachineVariant::Coco3, &roms_dir)
        .expect("roms/coco3.rom is required (git-ignored, local-only)");
    let rom_source = RomSource::File(roms_dir.join("coco3.rom"));
    let mut harness = egui_kittest::Harness::new_eframe(|_cc| {
        CocoApp::new(MachineConfig::default(), rom, rom_source, None,
                     [None, None], [None, None], std::array::from_fn(|_| None),
                     false, false, false)
    });
    harness.set_size(egui::vec2(1024.0, 768.0));
    harness.step();
    harness
}
```

`Harness::new_eframe` takes the same `CocoApp::new` constructor the real
`boot_app` calls, no test-only shortcuts. `harness.step()` is one simulated
frame — it runs `update()` exactly like a real event loop would, and it's
called manually, exactly once per unit of simulated time, so a test
controls pacing precisely instead of hoping timing works out. The comment
on `set_size` is worth remembering as a category of headless-testing gotcha
that has nothing to do with emulation: egui only puts *currently visible*
widgets into the AccessKit tree, so a viewport too small to show the whole
Machine menu would make its lower items simply un-queryable — not
disabled, not hidden by a flag, just never laid out this frame. The manager
harness (`manager_harness`) does the same dance with `ManagerApp::new`,
injecting `machines_dir`/`artifacts_root` as `None` or a temp directory —
**never** the real user config/data directories — precisely so tests can't
read or write a developer's actual saved machines.

### Interaction: hover, step, click, step, step

Every click helper in the harness follows the same four-beat rhythm:

```rust
pub(super) fn click<S: 'static>(harness: &mut egui_kittest::Harness<'static, S>, label: &str) {
    harness.get_by_label(label).hover();
    harness.step();
    harness.get_by_label(label).click();
    harness.step();
    harness.step();
}
```

Hover, step a frame, click, step twice more. This mirrors how egui itself
recognizes a click: it's not a single instantaneous event but a state
transition egui's own input handling notices *between* frames — hovering
first (so any hover-triggered layout, like a button's highlight, has
happened before the press lands), then a press-and-release pair that egui
fires `clicked()` for on release, and finally an extra settle frame so
whatever the click *caused* (a menu opening, a row becoming selected) is
fully reflected in the tree before the test's next assertion reads it.
Skipping any of these steps is the single most common way a kittest test
becomes flaky — not because the application logic is wrong, but because
the test asserted on a tree state that egui hadn't finished producing yet.

### Why label-based queries beat coordinates

`harness.get_by_label("Reset")` finds the one accessible node whose
AccessKit label is exactly `"Reset"` and panics if there's zero or more
than one match — it is, deliberately, as strict as `unwrap()`. Compare
that to a coordinate-based test (`harness.click_at(egui::pos2(340.0,
22.0))`): it would work today, and silently start clicking the *wrong
thing* the day anyone reorders a menu, resizes a toolbar, or changes a
font. A label-based test survives exactly the kind of refactor this
codebase does constantly (recall the module-splitting mentioned in this
repo's own commit history) because it asserts on *meaning* — "the control
labeled Reset" — not on *where that meaning happened to render this week*.
It's the UI-testing analogue of asserting on a register's value rather
than on a specific memory address holding it.

Two helpers handle the inevitable label collisions AccessKit users always
hit: `click_containing` matches by substring, for labels decorated with
extra text a submenu arrow or shortcut hint adds ("MultiPak Interface ⏵",
"New… ⌘N"); `lowest_by_label`/`click_in_menu` disambiguate a menu-popup
copy of a label the toolbar *also* shows ("Pause" appears in both places at
once) by picking whichever matching node is lowest on screen — the popup
always hangs below the toolbar row that opened it.

### Reading one real test

`ui_tests::manager_window::manager_row_context_menu_delete_confirms_and_removes`
(`crates/coco-egui/src/ui_tests/manager_window.rs:156-184`) is worth
reading start to finish as a script, because it reads like the exact
sequence of clicks a human tester would perform, in English:

```rust
click(&mut harness, "Beta CoCo 3");
assert_eq!(harness.state().selected, Some(1));

right_click(&mut harness, "Alpha CoCo 3");
click(&mut harness, "Delete…");
click(&mut harness, "Cancel");
assert_eq!(harness.state().entries.len(), 2, "Cancel must keep the machine");
assert!(dir.path().join("alpha.toml").exists(), "Cancel must keep the definition file");

right_click(&mut harness, "Alpha CoCo 3");
click(&mut harness, "Delete…");
click(&mut harness, "Delete");
assert_eq!(harness.state().entries.len(), 1);
assert!(!dir.path().join("alpha.toml").exists(), "the definition file must be removed");
assert!(dir.path().join("beta.toml").exists(), "only the confirmed machine is deleted");
assert_eq!(harness.state().detail_name(), Some("Beta CoCo 3"),
    "the selection must follow the surviving row as indices shift");
```

Select Beta. Right-click Alpha — this must *not* move the selection cue
(a real user decision the code comment cites: the context menu acts on the
row under the cursor, not on whatever's currently selected). Open Delete…,
click Cancel — nothing changes, the file survives. Right-click Alpha
again, Delete…, this time confirm — the row and its `.toml` are both gone,
and because Alpha sat at index 0 and Beta was selected at index 1, the
selection must shift down to stay pointed at Beta, not silently point at
whatever now occupies index 1. `harness.state()` — plain field access into
the real `ManagerApp` the harness owns — is how a test looks past the UI
entirely into the actual model state, the same way you'd inspect
`Machine` fields directly in a `coco-core` test rather than trying to OCR a
rendered screen.

> **Rust corner — one function, two apps.** `click<S: 'static>(harness:
> &mut egui_kittest::Harness<'static, S>, label: &str)` is generic over
> `S`, the app type the harness wraps — this exact function drives both
> `CocoApp` harnesses and `manager::ManagerApp` harnesses with no
> duplication, because `Harness<'static, S>`'s hover/step/click methods
> don't care what `S` is; they only need it to satisfy `Queryable`'s bound
> (`'static`, imported from `egui_kittest::kittest`). This is the same
> monomorphization story from Chapter 1's `Bus` trait (§1.3): the compiler
> emits one specialized copy of `click` for `S = CocoApp` and another for
> `S = manager::ManagerApp`, so there's no runtime cost to sharing this
> helper across two otherwise-unrelated app types — you get code reuse in
> *test* code the exact same way the CPU crate got it in production code.

---

## 15.9 Running the suite — an honest report

`cargo test -p coco-egui`, run in this worktree, which — like every
worktree that isn't the main checkout — has no `roms/` directory (git-
ignored, local-only, per the project's own convention). Here is exactly
what happened, not a sanitized summary:

```
test result: FAILED. 80 passed; 35 failed; 0 ignored; 0 measured; 0 filtered out
```

**All 35 failures are ROM-required, and only ROM-required.** Every one
panics at a `std::fs::read`/`load_default_rom` call reading
`roms/coco3.rom` or (for the FD-502 tests) `roms/disk11.rom`, with a
message stating exactly that: `"roms/coco3.rom is required (git-ignored,
local-only)"`. The failing set breaks down cleanly into three groups:

- `debugger::tests::*` (5) and `save_state::tests::*` (1) — unit tests that
  boot a real `Machine` directly with `Machine::new(config, load_rom())`.
- `ui_tests::direct_boot_menus::*` (17) and `ui_tests::new_vm_dialog::*`
  (7) — every kittest test that calls `boot_harness()`, which requires the
  real system ROM to construct a `CocoApp` at all.
- `ui_tests::manager_lifecycle::*` (4) — every test that actually calls
  `launch_machine` (Start a VM for real), as opposed to `manager_window.rs`
  and `manager_peripherals.rs`'s tests, which only exercise the manager's
  *list and edit* UI against injected `MachineEntry` fixtures
  (`sample_entry`, built from `MachineDef::from_config` — no ROM, no
  `Machine`, no boot) and consequently pass cleanly.

**The 80 passing tests are the whole non-ROM surface of the crate**: the
audio DSP unit tests (DC blocker, low-pass, resampler — pure math, no
`Machine`), every CLI parser test, every `machine_def` round-trip/atomicity/
slug test, the three thumbnail tests from §15.6 (`write_thumbnail_png`'s
round-trip and both halves of the all-black skip heuristic), the joystick
math tests, the paper-render/paper-export tests (pure rasterization, no
emulated printer attached), and — importantly for this chapter — every
`ui_tests::manager_window::*` and `ui_tests::manager_peripherals::*` test,
including the exact delete-confirmation test walked in §15.8 above. If you
have this worktree open and no `roms/` directory, `cargo test -p coco-egui`
will show you precisely this split; if you're working from the main
checkout with real ROMs present, all 115 tests should pass.

---

## 15.10 Reading assignment

In this order:

1. **`crates/coco-egui/src/main.rs:1-102`** — the crate's module list (a
   map of everything this chapter did and didn't cover) and the constants
   block: `SCALE`, `TARGET_ASPECT`, `MAX_FIELDS_PER_UPDATE`, `MAX_FRAME_DT`,
   `TYPE_HOLD_FIELDS`/`TYPE_GAP_FIELDS`.
2. **`crates/coco-egui/src/app.rs`** — the `CocoApp` struct in full; read
   every field's doc comment once, even the ones this chapter didn't
   discuss (week 16 owns several of them: `debugger`, and everything
   `save_state.rs`-adjacent).
3. **`crates/coco-egui/src/app/frame.rs`** — `fields_due`, `step_emulation`,
   `draw_display`, `window_ui`, in full. This is the file to reread when
   anything about timing or the display feels wrong later in the course.
4. **`crates/coco-egui/src/app/input.rs`** and **`keymap.rs`** — every
   function in both files is short; read them all, not just the excerpts
   above.
5. **`crates/coco-egui/src/manager.rs`**, **`manager/lifecycle.rs`**, and
   **`manager/vm_windows.rs`** — the module doc comment at the top of
   `manager.rs` first, then the three files in that order.
6. **`crates/coco-egui/src/media/disk.rs`** in full — then skim
   `media/tape.rs` and note everywhere it *differs* from the disk pattern.
7. **`crates/coco-egui/src/ui_tests/harness.rs`** in full, then
   **`crates/coco-egui/src/ui_tests/manager_window.rs`** — read every test
   in the file as if it were a QA script, not code.

Run the suite and read the failures, not just the pass count:

```
cargo test -p coco-egui
```

---

## 15.11 Exercises

**15.1 — Letterbox arithmetic (compute).** A `CentralPanel` measures
**1200 × 700** pixels (already net of menu/toolbar/status chrome). The
mounted texture is the CoCo 3's 640×240 canonical canvas. Compute the
final displayed rectangle, by hand, for (a) aspect correction **on**
(`aspect = 4/3`) and (b) aspect correction **off** (`aspect = 640/240`).
For each, state which axis is the binding constraint and how large the
margin bars are on the other axis. Then do it again for a panel of
**500 × 900** (a narrow, portrait-oriented window) with correction on —
notice which branch of `draw_display`'s `if h > avail.height()` fires this
time, and why it's the opposite branch from part (a).

**15.2 — `field_debt` simulation (compute).** A machine runs NTSC
(`field_rate_hz() = 59.94`). `update()` is called at these wall-clock
timestamps, in milliseconds since start: `0, 17, 33, 50, 250`. (The last
gap — 200 ms — models a stall, e.g. a window drag; it is under
`MAX_FRAME_DT`'s 250 ms cap, so it is *not* clamped before being credited.)
Starting from `field_debt = 0.0`, compute, for each call: `dt`, the debt
*before* adding this call's contribution, the debt after adding it, `due`
(applying the `MAX_FIELDS_PER_UPDATE = 8` cap), and the carried-over debt
(applying the final `.min(1.0)`). How many total fields ran across all five
calls? Which single line of `fields_due` is responsible for the emulator
*not* trying to run all ~12 fields nominally owed after the stall, and
which line is responsible for it not trying to make up the remainder
(≈5 fields) over the next several calls either?

**15.3 — Sabotage the thumbnail crash-insurance heuristic (sabotage,
verify by running the suite).** Open `crates/coco-egui/src/manager.rs` and
find `write_thumbnail_png`'s skip check:

```rust
if all_black && final_path.exists() {
    return Ok(());
}
```

Using `Edit`, remove the `&& final_path.exists()` clause, leaving just
`if all_black { return Ok(()); }`. Predict, before running anything: which
of the three thumbnail tests in §15.6/§15.9 (`write_thumbnail_png_round_trips_and_leaves_no_tmp`,
`uniformly_black_frame_keeps_the_previous_thumbnail`,
`black_frame_is_still_written_when_no_previous_thumbnail_exists`) now
fails, and why — trace exactly which assertion breaks. Then actually run
`cargo test -p coco-egui manager::tests` and check your prediction against
the real failure output. Finally, use `Edit` again to restore the original
`if all_black && final_path.exists() {` line exactly, rerun the same
command to confirm all three tests pass again, and run `git status` to
confirm the tree is clean.

**15.4 — Build: a 2× turbo toggle (build; describe your design, running
the real GUI to confirm is optional).** Add a "2× Turbo" checkbox to the
View menu (`chrome/menu_bar.rs`'s `view_menu_ui`) that, when checked, runs
the emulator at double real-time speed — a CoCo BASIC program that normally
takes 10 seconds should take about 5. Sketch the field where the toggle's
boolean state should live (which struct — `CocoApp`, and why not somewhere
in `Machine`, tying back to week 1's core/frontend split), and exactly
which line of `fields_due` you'd change to double the effective field rate
fed into `field_debt` (not the same thing as doubling `MAX_FIELDS_PER_UPDATE`
— explain in one sentence why doubling the *rate* is correct and doubling
the *per-update cap* alone would not reliably double perceived speed).
Note one interaction worth being honest about: at 2× speed, a host stall
that used to owe 8 fields now owes 16 — does your change increase how
often `MAX_FIELDS_PER_UPDATE`'s clamp actually triggers, and is that a
problem?

**15.5 — Read and predict a kittest test (read/predict, then verify by
running it).** Without running anything yet, read
`ui_tests::manager_window::manager_rename_migrates_definition_file_and_artifact_dir`
(`crates/coco-egui/src/ui_tests/manager_window.rs:262-303`) end to end and
write down, in order: (a) what `harness.state().entries[0].slug` equals
immediately after `name_field().focus()` and typing `" Two"` but *before*
`harness.key_press(egui::Key::Enter)`; (b) why the test calls
`harness.step()` **three** times after the Enter key press, when most of
this chapter's helpers only ever call it once or twice in a row — tie your
answer to `ManagerApp::apply_pending_renames`'s doc comment
(`manager/lifecycle.rs:154-161`) and the `rename_pending` field it
consumes. Then run `cargo test -p coco-egui ui_tests::manager_window` (this
one needs no ROM) and confirm your prediction against the passing test.

**15.6 — Read: the disk write-back chain, end to end (read).** Trace, by
reading source only (no running), the full path a modified floppy takes
from a BASIC `SAVE"PROG"` inside the emulator to bytes landing back on the
host filesystem: which `coco-core` type first notices the disk is dirty
(week 13), which `media/disk.rs` function is the *only* place that checks
`.dirty()` before touching the filesystem, and which three distinct
call sites in `CocoApp`/`ManagerApp` eventually reach that function
(eject, a controller swap, and — two of them — an application exit).
Then explain in one sentence why VHD images (`media/disk.rs`'s
`insert_vhd`/`eject_vhd`) need none of this machinery at all.

**15.7 — Essay, three sentences max (essay).** A colleague, new to
immediate-mode GUI, proposes: "let's cache the framebuffer texture upload
and only call `texture.set(...)` when `self.machine.framebuffer` actually
changed, to save GPU bandwidth — like retained-mode frameworks do with
dirty-rect tracking." Give the one concrete reason this specific
optimization is close to free to skip in *this* program (tie your answer
to how often the CoCo's own framebuffer genuinely is unchanged, frame to
frame, while running), and the one concrete reason implementing it anyway
would fight the grain of everything else in this chapter.

---

## What's next

Week 16 closes the course inside two directories this chapter deliberately
walked past without opening: `crates/coco-egui/src/debugger/` and
`crates/coco-egui/src/save_state/`. You already know their frontend
scaffolding without knowing it — `windows_ui`'s `self.debugger.windows_ui(...)`
call, `step_emulation`'s `self.debugger.run_field(&mut self.machine)`
routing every field through a breakpoint check, the quick-save/quick-load
shortcuts wired up in `consume_app_shortcuts`. Next week opens all three of
those up: the debugger core's breakpoint/watchpoint tables and the
side-effect-free `peek()` that keeps *looking* at memory from corrupting
the machine (the twin of `Bus::read`'s deliberate `&mut self` from Chapter
1, finally paying off from the other direction), and the save-state format
that Chapter 1's ownership discipline — no `Rc<RefCell<...>>` anywhere in
the state tree — bought for nearly free the moment `#[derive(Serialize,
Deserialize)]` landed on `Machine`. It is, deliberately, the last chapter:
by then you will have watched nearly every architectural decision this
course made in week 1 cash out, one at a time, for sixteen weeks.
