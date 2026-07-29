# Chapter 15 — The egui frontend: pixels, keys, and real time

*Week 15. Goal: everything host-side, for the graphics-shy. Weeks 1–14 built
a headless machine — `coco-core` renders into a `Vec<u8>`, records audio
into a `Vec<[f32; 2]>`, and has never once opened a window. This week you
finally look at the other side of the seam: `coco-egui`, the ~9,000-line
crate that turns that headless machine into something you can sit in front
of. The good news, and the whole point of this chapter: there is far less
"graphics programming" here than the phrase suggests. By the end you will
have read every line that touches a GPU in this entire codebase — there are
perhaps a dozen of them — and spent the rest of your time on arithmetic
(letterboxing, frame pacing) and plain application state (menus, a VM
manager, media attach/eject). Rust plus a working picture of what the CoCo 3
did covers everything this chapter needs except one new idea:
immediate-mode GUI, which §15.1 builds from nothing.*

---

For fourteen weeks the emulator has been a machine with no face. That was
deliberate, and Chapter 1 (§1.5) spelled out the reason: a core that never
opens a window is a core that can be tested by a robot, in continuous
integration, on a build agent with no display attached. The price of that
discipline is that the emulator has, until now, only ever been observable
through assertions and `.ppm` dumps. This week collects on that discipline
by building the face — and discovering that, because the core was designed
this way from the start, the face is almost embarrassingly thin.

Three separate anxieties tend to attach themselves to the phrase "now write
the frontend," and it is worth defusing all three before opening a file.
The first is that GUI programming means learning a large framework's object
model. It does not here: the framework this crate uses has no object model
to learn, and §15.1 explains why in about two pages. The second is that
displaying a picture means graphics programming — shaders, vertex buffers,
a render pipeline. It does not: §15.3 shows the two lines that put the
CoCo's picture on your screen, and then spends its remaining pages on
sixth-grade arithmetic about rectangles. The third is that connecting a
60 Hz emulated machine to a host that redraws at some unrelated rate is a
concurrency problem. It is not: §15.2 solves it with one `f64` field and
four lines of code, single-threaded, and the solution is one of the most
transferable ideas in this book.

What is genuinely new this week is a category of code the course has not
touched at all. Everything so far has been *emulation*: a data sheet says a
chip does X, so the code does X, and a test proves it. From §15.6 onward
this chapter is about *application design* — a manager window that lists
saved machines, a file format that survives being edited by a future
version of itself, a write-back discipline for media that has to be
crash-safe. None of it is emulation; all of it is the kind of code a serious
emulator eventually grows, and it exercises the same borrow-checker
strategy Chapter 1 established, in a setting that has nothing to do with a
6809.

There is one honest omission. This chapter deliberately walks past two
directories, `debugger/` and `save_state/`, without opening them. They are
Chapter 16's material, and you will see their call sites here — a `run_field`
that routes through a breakpoint check, a keyboard shortcut that
quick-saves a slot — without needing to know what is behind them yet.

---

## 15.1 Immediate mode, from zero

Every idea in this chapter rests on a single unfamiliar one, so it comes
first, and it assumes nothing. If the model of GUI programming
in your head is the one nearly everyone acquires first, the code in this
crate will look wrong until the model is replaced. Half an hour of careful
reading here saves a great deal of confusion later.

Anyone who has written a GUI before almost certainly wrote a
*retained-mode* one. Qt, the browser DOM, Swing, Cocoa, WPF, GTK — they all
work the same way at heart. You construct a tree of widget *objects* once:
a `QPushButton`, a `<div>`, a `JLabel`. The framework retains that tree for
the life of the window, holding onto every node, and from then on your
program's job is to *mutate* it. `button.setText("Pause")`.
`label.textContent = "42"`. `checkbox.setChecked(true)`. The framework
watches for changes and works out what region of the screen needs
repainting.

The consequence of that design is that a retained-mode program contains two
copies of the truth. There is the application's own state — a boolean field
somewhere that says whether the emulator is running — and there is the
framework's widget tree, which holds a button whose caption says "Pause" or
"Run". Nothing keeps them in agreement except code you write. "The button
and the model disagree" is such a common failure that entire architectural
patterns (MVC, MVVM, data binding, observables, reactive stores) exist
mostly to automate the reconciliation. Those patterns work. They are also a
lot of machinery to introduce in order to solve a problem the framework
created.

egui takes the other road. It is an *immediate-mode* GUI, and the crate
here is built on `eframe`, the thin windowing and backend layer
that wraps egui, opens a native window, and drives the event loop. In
immediate mode there is no persistent widget tree at all. Instead, your
`eframe::App` implements one method, called once per frame:

```rust
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.window_ui(ctx);
    }
```

That is the *entire* trait implementation — three lines at
[`crates/coco-egui/src/app.rs:326-328`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app.rs#L326-L328), forwarding to a plain inherent
method. Everything else in that file is ordinary `impl CocoApp`. There is
no widget registration, no event handler installation, no constructor that
builds a layout. The window is whatever `update` draws this time around,
and `update` runs again from scratch on the next frame.

Every widget you see on screen, then, is not an object but a *function call
that returns a response*, made fresh, every single frame:

```rust
                if ui.button("Reset").clicked() {
                    self.machine.reset();
                }
```

That is the Reset button of the real toolbar, at
[`crates/coco-egui/src/chrome/toolbar.rs:9-11`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/chrome/toolbar.rs#L9-L11). Read it as three
things happening inside one expression. `ui.button("Reset")` draws a button
at whatever the current layout position happens to be, then checks this
frame's input for a click that landed inside the rectangle it just drew,
then returns a `Response` describing what it found. `.clicked()` asks that
response one question. The `if` acts on the answer, immediately, in the
same statement — which is why the style is called immediate mode.

There is no `Button` object living anywhere between frames. Next frame, if
this code path runs again, egui draws the same button again from scratch,
having retained nothing about it. And if this code path does *not* run —
because a menu is closed, or a piece of hardware is not installed — the
button simply does not exist this frame. There is nothing to hide, nothing
to remove, nothing to destroy, and nothing to leak.

### All state lives in your struct

The consequence that has to be internalized before anything else in this
chapter makes sense is this: *the framework remembers nothing about your
application, so your application must remember everything.*

Look at the checkbox that toggles aspect correction, in the View menu:

```rust
        ui.checkbox(&mut self.aspect_correct, "4:3 aspect (F9)");
```

([`crates/coco-egui/src/chrome/menu_bar.rs:43`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/chrome/menu_bar.rs#L43).) The signature is the
whole lesson. `ui.checkbox` takes a `&mut bool` — a mutable borrow of a
field that belongs to `CocoApp`. It draws the box in whichever state that
bool currently holds, and, if the click landed on it this frame, it flips
the bool in place before returning. The widget and the model were never two
things needing synchronization; there was only ever one bool, and the
checkbox is a temporary lens onto it that exists for the duration of one
function call and then evaporates.

Scale that up and you have `CocoApp` itself: roughly thirty-five fields
([`crates/coco-egui/src/app.rs:11-131`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app.rs#L11-L131)) that between them constitute the
*entire* durable memory of the user interface. `self.running`,
`self.aspect_correct`, `self.kb_mode`, `self.show_kbd_help`,
`self.cart_path` — read that struct and you have read every fact the UI
knows about itself. Nothing is hiding in a framework's internals. There is
no equivalent of "ask the widget what it currently says," because there is
no widget to ask between frames.

### Widgets that only exist some frames

Immediate mode's most useful property is one that has no retained-mode
equivalent at all: a widget that is not drawn does not exist, and *not
drawing it* is an ordinary `if` in ordinary Rust. The status bar makes
this concrete. Here is the whole thing:

```rust
    pub(crate) fn status_bar_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("Keyboard: {} (F12)", self.kb_mode.label()));
                self.cart_status(ui);
                self.rs232_status(ui);
                self.mpi_status(ui);
                self.disk_status(ui);
                self.vhd_status(ui);
                self.drivewire_status(ui);
                self.tape_status(ui);
                if let Some(toast) = self.toast_message() {
                    ui.separator();
                    ui.label(toast);
                }
            });
        });
    }
```

([`crates/coco-egui/src/chrome/status_bar.rs:5-22`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/chrome/status_bar.rs#L5-L22).) The toast at the
end is a conditional widget, not a label that gets shown and hidden:
whether the bar ends with a toast is decided fresh, sixty times a second,
by asking `toast_message()`. The seven `*_status` calls are where it gets
interesting. Each one is written like this:

```rust
    fn cart_status(&self, ui: &mut egui::Ui) {
        let Some(path) = &self.cart_path else { return };
        ui.separator();
        ui.label(format!("Cart: {}", file_name(path)));
    }
```

([`crates/coco-egui/src/chrome/status_bar.rs:24-28`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/chrome/status_bar.rs#L24-L28).) With no
cartridge inserted, the function returns before drawing anything, and the
status bar this frame simply has no cartridge section — no separator, no
label, no reserved space that has to be collapsed. Eject the cartridge and
the section is gone on the very next frame, with no teardown code, no
`removeChild`, no visibility flag, and no possibility of an orphaned widget
lingering because someone forgot to destroy it. In a retained-mode
framework this same behavior is a lifecycle problem; here it is an early
`return`.

The same trick governs entire windows. Everything optional the app can
show — keyboard help, the About box, the Orchestra-90 level meters, the
debugger, the printer paper window, two error banners — is drawn by one
function whose body is a list of conditions:

```rust
    pub(crate) fn windows_ui(&mut self, ctx: &egui::Context) {
        if self.show_kbd_help {
            let symbolic = self.kb_mode == KbMode::Symbolic;
            kbd_help::window(ctx, &mut self.show_kbd_help, symbolic);
        }
        if self.show_about {
            about::window(ctx, &mut self.show_about);
        }
        if self.show_orch90
            && let Some(orch90) = self.machine.bus.cart.as_orch90()
        {
            orch90_meters::window(ctx, &mut self.show_orch90, orch90.left(), orch90.right());
        }
        self.debugger.windows_ui(ctx, &mut self.machine, &mut self.running);
        if let Some(err) = self.paper_window.ui(ctx) {
            self.cart_error = Some(err);
        }
        self.disk_controller_prompt_ui(ctx);
        self.cart_error_ui(ctx);
    }
```

([`crates/coco-egui/src/chrome/windows.rs:5-24`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/chrome/windows.rs#L5-L24).) The Orchestra-90
branch is the one worth dwelling on, because it demonstrates a subtlety
that catches people. It requires two conditions: the user has asked for the
meters *and* an Orchestra-90 cartridge is actually present in the machine
right now. Those two facts live in completely different places — one is a
UI preference on `CocoApp`, the other is a question asked of the emulated
cartridge slot — and immediate mode lets them be combined in a plain
boolean expression at the moment of drawing, with no subscription, no
listener, and no invalidation.

That in turn explains a field doc comment that would otherwise read as an
oversight:

> "View > Orchestra-90 Levels" window toggle ([`orch90_meters::window`]).
> Stays whatever the user last set even if the cartridge is later
> ejected — the window simply doesn't draw without a live `Orch90`
> (see the call site in `update`).
> ([`crates/coco-egui/src/app.rs:19-23`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app.rs#L19-L23))

Ejecting the cartridge does not have to reach over and clear
`show_orch90`. There is no window object to close. The preference keeps its
value, the window stops being drawn because its guard is false, and if the
cartridge is reinserted the window reappears exactly as the user left it.
A retained-mode version of this would need explicit close-on-eject logic,
plus explicit restore-on-insert logic if you wanted the same behavior, and
those two pieces of code would be in different files from each other.

The View menu shows the third variation on the theme — a control that
exists but is disabled:

```rust
        let orch90_present = self.machine.bus.cart.as_orch90().is_some();
        ui.add_enabled(
            orch90_present,
            egui::Checkbox::new(&mut self.show_orch90, "Orchestra-90 Levels"),
        );
```

([`crates/coco-egui/src/chrome/menu_bar.rs:53-57`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/chrome/menu_bar.rs#L53-L57).) "Enabled" is not
a property set on a persistent object and later unset; it is an argument
passed to a function that is called again next frame with a freshly
computed value. Insert an Orchestra-90 cartridge and the menu item becomes
clickable on the next frame, because the next frame recomputes
`orch90_present`. Nobody has to remember to re-enable it.

> **Rust corner — closures, and why the UI code nests.** Every panel in the
> excerpts above is built by passing a closure to a `show` method:
> `TopBottomPanel::bottom("status_bar").show(ctx, |ui| { … })`. The closure
> receives a `&mut egui::Ui`, egui's cursor into the layout, and everything
> the closure draws lands inside that panel. This shape is what lets an
> immediate-mode API express containment without a tree of parent and child
> objects: "inside this panel" literally means "inside these braces."
>
> The Rust consequence is worth noticing, because it explains the shape of
> essentially every function in this crate. Those closures capture `self`
> by unique (mutable) reference — they have to, since they call
> `self.cart_status(ui)` and mutate `self.aspect_correct`. That means no
> *other* live borrow of `self` may exist for the closure's duration.
> Nearly all of the frontend's methods therefore take `&mut self`, do their
> work through field accesses, and hand out no long-lived references. When
> a borrow conflict does arise the answer is always the same one Chapter 1
> taught in §1.4: split the state, or move the piece you need out of the
> struct and put it back afterwards. §15.6 shows exactly that maneuver
> applied to a whole running virtual machine.

### Why an emulator loves this

Now connect all of that to what Chapters 1–14 built, because the fit is
better than coincidence.

`Machine::run_field` already redraws the *entire* CoCo screen every 1/60th
of a second, from scratch, whether or not anything on it changed. That is
not an implementation choice this codebase made; it is simply what a raster
display is (Chapter 6, Chapter 7). A CRT does not know which pixels changed. It
sweeps the whole frame, every frame, forever, and the emulator models that
faithfully by rendering every scanline of every field.

An immediate-mode GUI does exactly the same thing for the *window around*
that screen: redraw everything, every frame, from current state. The two
halves of this program share a philosophy before you write a single line
connecting them. There is no damage tracking, no dirty-rectangle
bookkeeping, no "only repaint the status bar if the disk light changed."
You could not introduce that class of bug if you tried, because there is no
persistent tree to selectively update and therefore nothing to update
incorrectly.

This is also precisely why keeping `coco-core` headless (Chapter 1, §1.5) cost
nothing at integration time. A core built to be re-rendered wholesale every
field, handed to a frontend that also re-renders everything every frame, is
not a mismatch to be bridged with an adapter layer. It is the same idea
twice, and the seam between them turns out to be a byte buffer and a
function call.

The cost, to be honest about it, is CPU time spent on work that produced no
visible change. A running but idle CoCo — one sitting at the BASIC prompt
with nothing to do — still walks every `ui.menu_button` call in the menu
bar sixty times a second, still formats the status bar's strings, still
asks the cartridge slot whether an Orchestra-90 is present. For an
application of this size on modern hardware that cost is unmeasurable —
the emulated machine's own field rendering dwarfs it. It would matter in a
ten-thousand-widget enterprise dashboard.
It does not matter here, and §15.2 shows that the frontend has an explicit
lever for the one case where it might: while paused, the app stops asking
for repaints at all.

That lever is the per-frame loop, which is where this chapter goes next.

---

## 15.2 The per-frame loop: `step_emulation` and `field_debt`

Immediate mode answers "what gets drawn." It says nothing at all about
"how much emulated time should pass between one drawing and the next," and
that second question is where naive emulator frontends go wrong — usually
in a way that is invisible on the developer's own machine and catastrophic
on somebody else's. This section is the answer, and it is short enough to
memorize.

Everything about advancing the emulator by wall-clock time lives in one
function, `CocoApp::step_emulation`. Its doc comment states the contract:

> Advance emulation for one host frame — input, joysticks, the
> wall-clock-paced field loop, audio, and the framebuffer texture
> upload. Runs regardless of which chrome (if any) is drawn around the
> display this frame: [`Self::window_ui`] (full native window) and the
> manager's `ViewportClass::Embedded` fallback both call this before
> drawing anything, so a VM keeps emulating even in the degraded
> single-window case.
> ([`crates/coco-egui/src/app/frame.rs:21-28`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/frame.rs#L21-L28))

Hold onto the second half of that — "runs regardless of which chrome is
drawn around the display" — because §15.6 collects on it. For now, read the
function in full. It is the single most important one in this chapter, and
short enough to hold in your head at once:

```rust
    pub(crate) fn step_emulation(&mut self, ctx: &egui::Context) {
        self.handle_input(ctx);
        self.drive_joysticks(ctx);

        if self.running {
            // Run however many fields the wall clock owes us (real-time pacing),
            // stepping type-ahead per field so paste timing is refresh-agnostic.
            // Routed through the debugger so an enabled breakpoint/watchpoint
            // pauses the emulator cleanly instead of running straight through
            // it — a no-op when no breakpoints/watchpoints are set (the
            // common case), since `DebuggerPanel::run_field` then always
            // completes the field, same as `Machine::run_field` directly.
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
            // Drop any fields owed to the wall clock while paused (debugger
            // pause included), so resuming doesn't instantly "catch up" on
            // the paused interval — a clean pause, not just a frozen screen.
            self.field_debt = 0.0;
        }

        self.upload_framebuffer_texture(ctx);
    }
```

([`crates/coco-egui/src/app/frame.rs:29-62`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/frame.rs#L29-L62).) That last call is the
texture upload, split into its own method
(`upload_framebuffer_texture`, [`crates/coco-egui/src/app/frame.rs:71-83`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/frame.rs#L71-L83))
because a *suspended* VM's window runs only that step — §15.6 explains
why. Five things happen, in
this order, every time the host asks for a frame. Host input is read and
routed into the emulated keyboard matrix. The joysticks are polled and
their pot values written. Zero or more *emulated fields* are run. Whatever
audio those fields generated is pushed into the sound ring. And the
resulting framebuffer is uploaded as a texture.

The ordering is not incidental. Input is read *before* any field runs, so
the matrix state a field observes is this frame's, not last frame's — a key
pressed and released quickly still reaches the ROM's keyboard scan in the
right field. The texture upload happens *after* the loop and, crucially,
*outside* the `if self.running` block: a paused emulator still uploads its
last framebuffer every frame, which is what keeps the picture on screen
while nothing advances.

Two of the five deserve pointers rather than explanation, because earlier
chapters own them. `self.debugger.run_field(&mut self.machine)` is week
16's material; for now, take the comment at its word that with no
breakpoints set it behaves exactly like calling `Machine::run_field`
directly, and that a `false` return means "a breakpoint tripped, pause the
machine" — which is why the loop assigns `self.running = false` and breaks
rather than continuing. `self.audio.push_samples(...)` is Chapter 11's, and
§15.5 below is its one-paragraph pointer.

`ctx.request_repaint()` is worth a sentence of its own, since it sits
inside the `running` branch and nowhere else. It tells egui not to go idle,
because there will be something new to draw next frame — which is exactly
true of a running emulator and exactly false of a paused one. A paused CoCo
therefore stops driving repaints, and the frontend's per-frame cost — the
one §15.1 was honest about — drops to whatever the windowing system asks
for anyway.

The one piece left is `self.fields_due()`, the answer to the question this
section exists for: how many times does `run_field` run during *this* call?

### Why you can't just run one field per repaint

The obvious design is "one `update()` call, one emulated field." It is
wrong, and this is not a subtle wrongness discovered late — DESIGN.md
flagged it back in §4 with the instruction "don't trust egui's repaint
cadence for emulation timing," prescribing a real-time accumulator that
runs whole emulated fields while the accumulated delta exceeds a field
period ([DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md)). Two failure modes force that, and both are
routine rather than exotic.

The first is a fast display. eframe repaints roughly at the host display's
refresh rate, and that rate is not 60 Hz on a great deal of modern
hardware. On a 144 Hz gaming monitor, one field per repaint runs the CoCo
at 144 emulated fields per second — 2.4 times real speed. Every game
becomes unplayable, every piece of music plays sharp and fast, and every
BASIC program that measures time by counting interrupts measures it wrong.
The emulator is not slightly off; it is running a different machine.

The second is any interruption at all. Drag the window, hit a debugger
breakpoint, let the operating system schedule something else for a moment,
and `update()` might not be called for 400 ms. Under "one field per
repaint," the emulated machine simply *loses* those 400 ms of wall-clock
time. Audio, which is being consumed by a sound device at a fixed rate on
another thread, glitches. The 60 Hz field-sync interrupt (Chapter 6) that
stock BASIC idles on falls behind. And the cassette motor (Chapter 12), which
models its own mechanics in real seconds, drifts out of step with the tape
data it is supposed to be pulling past the head.

The fix is a *field-debt accumulator*, and it is genuinely one small
function:

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

([`crates/coco-egui/src/app/frame.rs:9-19`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/frame.rs#L9-L19).) Walk it exactly once,
slowly, because every line is load-bearing.

1. **Measure real elapsed time.** `dt` is the wall-clock seconds since the
   previous call. The very first call after a start or a resume has no
   previous instant — `self.last_update` was `None` — so it credits zero
   elapsed time. The emulator does not try to catch up on time that passed
   before it existed.
2. **Convert `dt` into fields owed.** The conversion factor is the
   machine's own field rate, `VideoStandard::field_rate_hz()`, which is
   59.94 for NTSC and 50.0 for PAL ([`crates/coco-core/src/config.rs:58-63`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/config.rs#L58-L63),
   Chapter 6). That product is *added* to `field_debt`, which is a running
   fractional balance and is never reset at the top of the call.
3. **Pay out the whole fields owed.** `due` is `floor(field_debt)`, capped
   at `MAX_FIELDS_PER_UPDATE`.
4. **Carry the fraction forward.** Subtract only the whole fields actually
   paid out, so a debt of 2.7 fields becomes 0.7 rather than zero. That 0.7
   is still owed, and *will* trigger a field once enough further time piles
   on top of it. No time is silently discarded by the act of rounding down
   — except by the deliberate `.min(1.0)` at the very end, which we will
   come back to and which is a policy decision rather than an accident.

The idea in one sentence: *decouple emulation speed from repaint cadence by
never asking how many repaints happened and always asking how much
wall-clock time elapsed.* That sentence generalizes far beyond emulators.
Any simulation that must advance at a fixed rate while being driven by an
event loop at some other rate wants this function, and games have been
writing versions of it for decades.

> **Rust corner — `Option::replace`, a swap in one expression.**
> `self.last_update.replace(now)` does two things at once: it stores `now`
> into the `Option` field, and it returns whatever was there before, as an
> `Option<Instant>`. The `match` immediately below consumes that return
> value. The whole "remember the current time, and also tell me the
> previous one" transaction is therefore a single expression with no
> temporary variable and no window in which the field holds a stale value.
>
> Written the long way it would be three statements — read the old value,
> write the new one, then branch on the old — and it would be entirely
> possible to get the order wrong and read back the value just written.
> `replace` is one of a small family of `Option` methods (`take`, `replace`,
> `get_or_insert_with`, `is_none_or`) that this crate leans on heavily;
> `get_or_insert_with` shows up in the texture upload above, `take` runs
> the manager's viewport loop in §15.6, and `is_none_or` decides when the
> detail pane's edit state must be reseeded for a newly selected row.
> Learning the family pays off quickly when reading Rust that manipulates
> optional state.

### Working the numbers: a 120 Hz monitor

Abstract accumulator arguments are unconvincing; arithmetic is not. Take a
120 Hz display, which calls `update()` roughly every 8.33 ms, against
NTSC's 59.94 Hz field rate:

| Call | `dt` (s) | `field_debt` before | `+= dt·59.94` | `due` | `field_debt` after |
|------|---------:|---------------------:|--------------:|------:|--------------------:|
| 1    | 0.00833  | 0.000                 | 0.4995        | 0     | 0.4995               |
| 2    | 0.00833  | 0.4995                | 0.9990        | 0     | 0.9990               |
| 3    | 0.00833  | 0.9990                | 1.4985        | 1     | 0.4985               |
| 4    | 0.00833  | 0.4985                | 0.9980        | 0     | 0.9980               |
| 5    | 0.00833  | 0.9980                | 1.4975        | 1     | 0.4975               |

A field runs roughly every *other* repaint, and never every repaint,
because each individual 120 Hz tick only owes half a field. Averaged out
that is one field every 16.68 ms: 59.94 Hz, exactly the CoCo's real rate,
on a display refreshing at twice that. Three of the five calls above did no
emulation at all and merely re-uploaded the same texture, which is the
correct behavior — the CoCo genuinely had nothing new to show yet.

This is precisely what the `field_debt` doc comment promises:

> Fractional emulated fields owed to the wall clock (`DESIGN.md` §4):
> fields run when it reaches 1, the remainder carries over. This decouples
> emulation speed from the host refresh rate (120 Hz displays no longer
> run the CoCo at double speed).
> ([`crates/coco-egui/src/app.rs:28-31`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app.rs#L28-L31))

Run the same table at 60 Hz, where `dt ≈ 0.01667`, and something subtler
shows up. Each call adds about 0.999 fields of debt, so `due` is 1 on
almost every call — but not quite every call, because 60 and 59.94 are not
the same number. Roughly every thousand frames the accumulator falls a
whisker short of 1.0 and that frame runs no field at all, which is exactly
right: a 60 Hz host really does tick slightly faster than an NTSC CoCo's
field rate, and the machine really should drop one field per thousand to
stay honest. The accumulator absorbs that drift the same way it absorbs
everything else, one fractional field at a time, with no special case
anywhere in the code for "host rate close to but not equal to field rate."

That is the mark of a good design: the awkward case and the easy case go
down the same code path.

### The two guardrails, and the spiral of death

An accumulator that faithfully remembers every field it is owed has a
failure mode of its own, and it is a bad one. Two constants sit next to
`field_debt`'s definition to prevent it:

```rust
/// Cap on emulated fields run in one UI update: catches up after short host
/// stalls (~130 ms) but drops time beyond that instead of spiralling.
pub(crate) const MAX_FIELDS_PER_UPDATE: usize = 8;
/// Longest wall-clock gap credited to the emulation clock, in seconds. Gaps
/// beyond this (window drag, app hidden, debugger pause) are discarded.
pub(crate) const MAX_FRAME_DT: f64 = 0.25;
```

([`crates/coco-egui/src/main.rs:88-93`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/main.rs#L88-L93).) Both exist to prevent the same
disease, which has a name: *the spiral of death*.

Imagine there were no cap on fields per update. The host stalls for a
second — a window manager hiccup, a laptop waking from sleep, an antivirus
scanner, whatever. `field_debt` jumps to roughly 60. The next `update()`
call dutifully tries to run 60 fields *before* returning control to the UI.
But running 60 fields of CPU emulation, video scanout, and audio rendering
takes real wall-clock time too. Say it takes 200 ms on this machine. During
those 200 ms, more real time elapses than the frame accounted for, so
`field_debt` is already non-zero again by the time the next call measures
it.

That is survivable if the emulator is comfortably faster than real time,
which it usually is. It is not survivable if it is not — on a loaded
system, in a debug build, on a slow machine, or with a demanding cartridge
attached. Then the backlog never shrinks. It grows every single update. The
UI stops responding to input because it is permanently "catching up," the
window will not even close, and the program is functionally hung while
burning an entire core. That is the spiral: falling behind makes you fall
further behind, forever, and the only exit is a signal from outside.

`MAX_FIELDS_PER_UPDATE = 8` breaks the spiral by refusing to attempt more
than a bounded amount of catch-up work per frame. Whatever cannot be run
this call is simply not run this call. `MAX_FRAME_DT = 0.25` attacks the
same problem from the opposite end: a genuinely enormous gap — the window
was minimized for ten minutes, the laptop lid was shut — is clamped to a
quarter of a second's worth of *credited* time before it ever reaches
`field_debt`. The accumulator never even sees the huge number.

### Throwing away time on purpose

Now look again at the last line of `fields_due`, which does something the
walkthrough above deliberately deferred:

```rust
        self.field_debt = (self.field_debt - due as f64).min(1.0);
```

Notice what is clamped. It is not the amount paid out; it is the
*carried-over remainder*. At most one field's worth of fractional debt
survives any call to this function, ever.

Work through a concrete stall to see why that matters. Suppose
`field_debt` was 0.997 and a 200 ms gap arrives — under the 250 ms cap, so
`MAX_FRAME_DT` does not clamp it. That adds 0.2 × 59.94 ≈ 11.99 fields of
debt, for a total of 12.985. `due` is computed as 12, then capped to 8, so
eight fields run. Subtracting gives 4.985 fields still nominally owed:
nearly five more fields of catch-up pressure sitting in the accumulator,
which would make the *next several* frames also run at the eight-field cap,
extending the visible slowdown well past the original stall.

The `.min(1.0)` throws that backlog away outright. A stall costs you
smoothness for exactly one clamped burst of up to eight fields, and then
the clock is caught up to "now" — not to "everything you missed."

This is an opinionated policy, and it is worth naming rather than
absorbing silently: *prefer time perceived as real over exact accounting of
missed field count.* An emulator that insisted on running every field it
was ever owed would, after any real stall, visibly fast-forward through the
backlog. Sound plays at the wrong pitch, sprites teleport, a BASIC
program's screen output scrolls past faster than it was ever meant to. The
codebase chooses to drop the excess instead, on the grounds that a
half-second of missing history is less objectionable than a half-second of
wrong-speed playback. Different emulators make this call differently; what
matters is making it deliberately, in one line, where a reader can find it.

Pausing gets its own treatment in the `else` branch, and for the same
reason. `self.last_update` is reset to `None` and `field_debt` to `0.0`.
Without this, a debugger breakpoint held for ten seconds would be
interpreted on resume as ten seconds of owed field time — hundreds of
fields, if the two guardrails above did not exist. With them, `MAX_FRAME_DT`
credits only a quarter second of that gap, which is still about fifteen
fields owed and a full eight-field burst the instant you hit Continue —
undoing the entire point of having stopped. Resetting both makes resuming a
clean restart of the pacing clock rather than a catch-up. The comment in
the source puts it exactly right: this is "a clean pause, not just a frozen
screen."

> **Rust corner — why `f64`, not `f32`, for `field_debt`.** `field_debt`
> accumulates a tiny fractional remainder *every single frame*, potentially
> for hours of continuous play. `f32` carries roughly seven decimal digits
> of precision; accumulated rounding error across millions of additions to
> an `f32` would eventually drift the emulator's timing visibly, which is
> the same failure mode as summing many small `f32` deltas in any
> long-running simulation. `f64`'s fifteen-to-sixteen digits push that
> drift far below anything a human — or a tape loader's timing tolerance —
> could ever notice.
>
> The general rule to take away: when you see an accumulator meant to run
> for a program's entire lifetime, reach for `f64` by default. Reserve
> `f32` for values recomputed fresh each frame, like the framebuffer's
> pixel geometry in §15.3, where error has no opportunity to build up
> because nothing is carried forward.

With timing settled, the picture itself is next — and it is smaller than
the timing was.

---

## 15.3 Graphics programming, demystified

This is the section that fulfils the chapter opener's promise. The *entire*
GPU-facing surface of the emulated display consists of one texture upload
per frame and one textured rectangle drawn with it — and the handful of
other GPU-facing lines in the crate (the manager's photo pane and its row
thumbnails, the printer paper window) are those same two calls again, with
different pixels. Everything else that looks like graphics is arithmetic,
and by the end of this section you will have read all of it.

You already read the upload half at the bottom of `step_emulation`. Reread
it now, knowing why it runs unconditionally, every frame, whether or not
the CoCo's screen actually changed — immediate mode again: there is no
"did the pixels change" check, because there is no retained copy to compare
against.

```rust
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [
                self.machine.fb_width as usize,
                self.machine.fb_height as usize,
            ],
            &self.machine.framebuffer,
        );
        let texture = self.texture.get_or_insert_with(|| {
            ctx.load_texture("coco-fb", image.clone(), egui::TextureOptions::NEAREST)
        });
        texture.set(image, egui::TextureOptions::NEAREST);
```

Four steps, and only the last two reach a GPU. `self.machine.framebuffer`
is the plain `Vec<u8>` of RGBA bytes that Chapter 7 taught you to render into
— the exact same buffer the headless PPM-writing examples in
`coco-core/examples/` dump to disk, with no frontend involved at all.
`ColorImage::from_rgba_unmultiplied` wraps that byte slice together with
its width and height into a CPU-side image description; nothing has crossed
into graphics-driver territory yet. `get_or_insert_with` allocates a GPU
texture handle exactly *once*, on the first frame the app ever draws, and
every frame after that reuses the same handle. And `texture.set(...)` is
the line that crosses into GPU territory on *every* frame: it uploads this
frame's bytes into the already-allocated texture, replacing last frame's
contents.

`egui::TextureOptions::NEAREST` is the option that makes the picture look
right, and it deserves a paragraph because it is the only piece of graphics
vocabulary this chapter needs. When a texture is drawn at a size other than
its native pixel dimensions — and it always is, since a 640-pixel-wide CoCo
canvas is being stretched across a 1341-pixel-wide rectangle — the hardware
has to decide what color to put at each destination pixel. *Nearest*
sampling picks the single closest source pixel and uses it unchanged.
*Linear* sampling blends the neighboring source pixels together. For a
CoCo screen, nearest is the only defensible choice: it keeps the machine's
chunky low-resolution pixels crisp and square-edged when magnified, exactly
as a real set's phosphor blocks appeared, instead of smearing them into a
soft blur that no CoCo owner ever saw.

The frontend does use linear sampling — twice, and both times for
photographs rather than emulated screens. The manager's decorative photo
pane uploads with `TextureOptions::LINEAR`
([`crates/coco-egui/src/manager.rs:343-346`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager.rs#L343-L346)), and so does a suspended
machine's saved screen thumbnail when it is loaded back from its PNG
([`crates/coco-egui/src/manager/thumbnails.rs:57-61`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/thumbnails.rs#L57-L61)). Both are being
scaled *down* into a small area rather than up, and a photograph shrunk
with nearest sampling looks harsh and aliased. Same API, opposite choice,
for a reason you can state in one sentence — which is what makes it worth
knowing rather than memorizing.

### One rectangle, and the arithmetic that places it

That is the upload. The draw is `draw_display`, and it is arithmetic rather
than graphics API calls:

```rust
    pub(crate) fn draw_display(&mut self, ui: &mut egui::Ui) {
        let tex = self.texture.as_ref().unwrap();
        let tex_size = tex.size_vec2();
        // Aspect the displayed frame should have, independent of the buffer's
        // pixel dimensions: 4:3 when corrected, else the raw square-pixel aspect.
        // This keeps the frontend mode-agnostic — any renderer's buffer size fits.
        let aspect = if self.aspect_correct {
            TARGET_ASPECT
        } else {
            tex_size.x / tex_size.y
        };
        // Largest rect of that aspect that fits the panel, centered (letterboxed).
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
        // Remembered for `drive_joysticks` next frame, to map pointer
        // position to joystick axes (see the `display_rect` field doc).
        self.display_rect = rect;
    }
```

([`crates/coco-egui/src/app/frame.rs:83-108`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/frame.rs#L83-L108).) Find
`ui.put(rect, egui::Image::new(sized))` in the middle of that. It is the
display path's second and last GPU-facing call: draw one textured quad,
sized to `rect`. Every line above it exists to decide what `rect`
should *be*, and the line below it merely records the answer.

This is the entirety of "3D graphics" in this codebase — one 2D rectangle,
textured. No shaders that you write. No vertex buffers that you manage. No
camera, no projection matrix, no lighting model, no render pass. egui and
its backend turn "draw this rect with this texture" into actual draw calls,
and this file never touches that layer. Which backend does that work is,
incidentally, a build-time choice: the crate ships on eframe's glow
(OpenGL) backend by default, with an optional `wgpu` feature that compiles
eframe's wgpu backend in instead ([`crates/coco-egui/Cargo.toml:59-63`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/Cargo.toml#L59-L63)). The startup
banner prints which one is live ([`crates/coco-egui/src/startup.rs:102-125`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/startup.rs#L102-L125)),
and nothing in this chapter changes between them.

The final line, `self.display_rect = rect`, is the one piece of state
`draw_display` leaves behind, and it is there for a non-obvious consumer:

> Letterboxed display rect from the last frame's `CentralPanel`, used to map
> pointer position to joystick axes. One frame stale (see `drive_joysticks`).
> ([`crates/coco-egui/src/app.rs:37-39`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app.rs#L37-L39))

Mouse-as-joystick needs to convert "the pointer is at this window position"
into "the stick is this far right and this far down," and that conversion
needs to know where inside the window the CoCo picture actually is. Since
`drive_joysticks` runs at the *top* of `step_emulation` and `draw_display`
runs at the *bottom* of the frame, the rect the joystick code reads is
always one frame old. The doc comment says so plainly rather than
pretending otherwise. At sixty frames a second, a mouse-driven joystick
reading a 16 ms-old rectangle is indistinguishable from one reading a fresh
rectangle, and the alternative — restructuring the frame so layout happens
before input — would be a large change to buy nothing.

### Letterboxing, worked with real numbers

The remaining arithmetic solves a problem with a name everyone already
knows from television: fit the largest rectangle of a given aspect ratio
inside a panel, centered, and leave the rest blank. It is the same problem
that puts black bars above and below a widescreen film on a 4:3 set, and
the same one that puts bars at the sides of a 4:3 broadcast on a widescreen
set. Trace the algorithm as four steps:

1. Decide the *target aspect ratio*, independent of the texture's actual
   pixel dimensions. That is `TARGET_ASPECT = 4.0 / 3.0` when aspect
   correction is on — the real shape of an NTSC picture — or the texture's
   own raw width-over-height when correction is off.
2. Assume the panel's *full width* first, and derive the height that aspect
   demands: `h = w / aspect`.
3. If that guess is *taller* than the panel, the width assumption was
   wrong. Clamp to the panel's full height instead and recompute the width
   from it: `w = h * aspect`.
4. Center the resulting `w × h` rectangle in the panel. Whatever is left
   over on the unconstrained axis is the letterbox (or pillarbox) margin.
   `ui.put` never draws anything there, so it stays whatever the
   `CentralPanel`'s background fill is — `egui::Color32::BLACK`, set where
   the panel is created in `window_ui`.

Now plug in real numbers. Take a CoCo 3 running in a GIME-native mode, so
its canvas is the canonical 640×240 raster (`raster::CANVAS_W` and
`CANVAS_H`, [`crates/coco-core/src/raster.rs:15-18`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/raster.rs#L15-L18), Chapter 7), inside a
1920×1080 window. Subtract the fixed chrome heights `coco-egui` reserves —
`MENU_BAR_H` at 22, `TOOLBAR_H` at 30, and `STATUS_BAR_H` at 22, totalling
74 pixels — and the `CentralPanel` is roughly **1920 × 1006**.

**Aspect-corrected**, so `aspect = 4/3 ≈ 1.3333`:

```
w = 1920                  (try full width)
h = 1920 / 1.3333 = 1440  (taller than the 1006 available!)
→ clamp: h = 1006
  w = 1006 × 1.3333 = 1341.3
```

The final rectangle is **1341 × 1006**, centered. Height was the binding
constraint, so the leftover space is horizontal: roughly
`(1920 − 1341.3) / 2 ≈ 289` pixels of black bar down each side. That is
pillarboxing.

**Aspect-uncorrected**, so `aspect = tex_size.x / tex_size.y = 640/240 ≈
2.6667` — the raw, non-square-pixel shape of the canvas itself:

```
w = 1920                 (try full width)
h = 1920 / 2.6667 = 720  (fits inside 1006 — no clamp needed)
```

The final rectangle is **1920 × 720**, centered. Width was the binding
constraint this time, the `if` did not fire, and the leftover space is
vertical: roughly `(1006 − 720) / 2 ≈ 143` pixels of black bar top and
bottom. That is letterboxing.

Two different final rectangles, same algorithm, same source texture. The
only thing that changed between them was which `aspect` value was fed in.
That is the whole payoff of computing `aspect` *before* the fit logic runs,
as a mode-agnostic scalar, rather than hard-coding "stretch to 4:3" into
the layout math. The doc comment says so directly: "This keeps the frontend
mode-agnostic — any renderer's buffer size fits"
([`crates/coco-egui/src/app/frame.rs:86-88`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/frame.rs#L86-L88)). A CoCo 1 in a legacy VDG
mode hands this function a 288×224 buffer instead of a 640×240 one and
needs no code change whatsoever, because the function never assumed a size.

### Why the pixels aren't square in the first place

One more number is worth internalizing, and it comes from `main.rs`'s own
doc comment on `TARGET_ASPECT`:

> Physical aspect the CoCo frame fills on an NTSC set (4:3). The
> framebuffer is 288×224 (≈1.29:1); when aspect correction is on, the image
> is stretched horizontally to this ratio so pixels are ~3% wider than
> tall, as on real hardware.
> ([`crates/coco-egui/src/main.rs:84-87`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/main.rs#L84-L87))

That 288×224 figure is `coco_core::video::FB_W` and `FB_H` — the CoCo 1
and 2 legacy VDG canvas, which is a 256×192 active area plus a 16-pixel
border on every side ([`crates/coco-core/src/video.rs:33-38`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/video.rs#L33-L38)). Do the
division: `4/3 ÷ (288/224) ≈ 1.037`, a 3.7% horizontal stretch, which
matches the "~3%" the comment claims.

This is not an emulator quirk to apologize for. Real NTSC CoCos drove
non-square pixels onto a 4:3 tube in exactly this way, because the
hardware's dot clock and the television's physical aspect ratio were never
designed to agree pixel for pixel — the dot clock came from the color
subcarrier (Chapter 1, §1.2), and the tube's shape came from a broadcast
standard set decades earlier. `TARGET_ASPECT` is the frontend choosing to
reproduce that historical mismatch rather than "fix" it into square pixels
that no CoCo owner ever actually saw. Turning aspect correction off with F9
is the other choice, and it is the right one when comparing a screenshot
against a reference emulator pixel for pixel.

### The initial window size, and why getting it wrong is harmless

There is a small curiosity here that is worth a look precisely because it
demonstrates how forgiving immediate mode is about mistakes. Here is the
function that picks the *initial* operating-system window size, before any
frame has ever run:

```rust
pub(crate) fn native_options(variant: MachineVariant) -> eframe::NativeOptions {
    // Size for the aspect-corrected (wider) image so it always fits; the
    // uncorrected image is narrower and simply leaves margin.
    let img_h = coco_core::video::FB_H as f32 * SCALE;
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/coco3-console-8bit.png"))
        .expect("embedded icon PNG is valid");
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([img_h * TARGET_ASPECT, img_h + MENU_BAR_H + TOOLBAR_H + STATUS_BAR_H])
            .with_icon(icon)
            .with_title(format!("cocovm — {}", machine_label(variant))),
        ..Default::default()
    }
}
```

([`crates/coco-egui/src/boot.rs:58-71`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/boot.rs#L58-L71).) Notice the input:
`coco_core::video::FB_H`, the fixed 224-pixel legacy figure, multiplied by
`SCALE` — for *every* machine variant, including a CoCo 3 whose native
canvas is 240 rows tall rather than 224. The manager's own per-VM window
does the identical thing
([`crates/coco-egui/src/manager/vm_windows.rs:23-28`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/vm_windows.rs#L23-L28)), deliberately, with a
comment saying it uses "the same formula `main()` uses for the direct-boot
window."

That is an approximation, and it is not a bug, because of what the number
is *for*. It seeds the window's starting size and nothing else.
`draw_display` never consults it. Every single frame, that function re-reads
`ui.available_rect_before_wrap()` and recomputes the fit from scratch. Get
the initial guess wrong and the worst outcome is that the very first frame
shows a picture with slightly more letterbox margin than a perfectly
computed window would have had — and the user, who is free to resize the
window anyway, will never know.

Sit with the counterfactual for a moment, because it is the real lesson. In
a retained-mode framework, an initial layout is frequently something that
must be *corrected* later: you compute a size, build a widget tree around
it, and if the size was wrong you now need invalidation, relayout, and
possibly a resize handler that undoes assumptions the constructor made.
Here there is no persisted layout state to get permanently wrong, because
layout is not persisted at all. It is recomputed from the current window
size sixty times a second, forever. An initial guess is a *guess*, in the
plainest sense, and the next frame overwrites it.

---

## 15.4 Input routing: two keyboards, one matrix

Getting pixels out of the emulator is half the seam. Getting keystrokes in
is the other half, and it is the more interesting half, because a modern
host keyboard and a 1980 matrix keyboard disagree about what a keystroke
even *is*. This section is about that disagreement and the two
incompatible ways this crate resolves it.

Host input enters through one function per frame, called at the very top of
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

        // While a paste / type-ahead burst is draining it owns the matrix, in either
        // mode, so replayed taps aren't clobbered by the per-frame positional writes.
        // (The taps themselves advance once per *emulated field*, in `update`.)
        if self.type_ahead.is_active() {
            return;
        }
        if self.kb_mode == KbMode::Positional {
            self.drive_matrix_positionally(&events, mods);
        }
    }
```

([`crates/coco-egui/src/app/input.rs:25-43`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/input.rs#L25-L43).) `ctx.input(|i| ...)` is
egui's own read of this frame's raw events — every key press and release,
every mouse move, every paste since the last frame — handed to you inside a
closure. This function copies out the two things it needs and leaves
immediately, which is deliberate: with the events cloned into a local
`Vec`, the three consumers below can each iterate the whole list
independently, in order, without holding anything borrowed from the context
while they mutate `self`.

Four things then happen, in a specific order, and the order encodes a
priority. App-level shortcuts go first and are *consumed*, so
they never reach the CoCo at all. Hotkeys and clipboard paste go second.
Symbolic-mode text queuing goes third. And direct matrix driving goes last
— only in positional mode, and only when no paste or type-ahead burst is
still draining.

### Shortcuts the CoCo never sees

The first stage is the one that decides which keystrokes belong to the
application rather than to the emulated machine:

```rust
    pub(crate) fn consume_app_shortcuts(&mut self, ctx: &egui::Context) {
        // ⌘N is the MANAGER's new-machine shortcut and means nothing in a
        // VM window — but it's still consumed here, as a deliberate no-op,
        // so a user hitting it out of habit doesn't type an `N` into the
        // running machine via the positional matrix (which forwards keys
        // regardless of the COMMAND modifier).
        let _ = ctx.input_mut(|i| i.consume_shortcut(&new_vm::NEW_MACHINE_SHORTCUT));
        // COMMAND+<n> quick-loads state slot n; COMMAND+SHIFT+<n> quick-saves
        // it (`save_state.rs`).
        for slot in 0..save_state::QUICK_SLOTS {
            if ctx.input_mut(|i| i.consume_shortcut(&save_state::save_slot_shortcut(slot))) {
                self.quick_save(slot);
            }
            if ctx.input_mut(|i| i.consume_shortcut(&save_state::load_slot_shortcut(slot))) {
                self.quick_load(slot, ctx);
            }
        }
    }
```

([`crates/coco-egui/src/app/input.rs:49-66`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/input.rs#L49-L66).) The word doing the work
is `consume_shortcut`, and its doc comment on the enclosing function
explains why it must run first: these shortcuts are "consumed before the
event snapshot `handle_input` takes, so the keypress never reaches the CoCo
matrix or the symbolic type-ahead." The ⌘N line is the instructive one:
creating machines belongs to the *manager* (its toolbar's "New…" and its
own ⌘N handler), so a VM window consumes the chord and deliberately does
nothing with it — swallowing it is still better than letting the positional
matrix type an `N` into BASIC. The quick-save and quick-load slots are
Chapter 16's feature, wired up here.

The second stage handles keys the application claims without consuming, plus
the clipboard:

```rust
    pub(crate) fn handle_hotkeys_and_paste(&mut self, events: &[egui::Event]) {
        for ev in events {
            match ev {
                egui::Event::Key { key, pressed: true, repeat: false, .. } => match key {
                    egui::Key::F12 => {
                        let next = match self.kb_mode {
                            KbMode::Positional => KbMode::Symbolic,
                            KbMode::Symbolic => KbMode::Positional,
                        };
                        self.set_mode(next);
                    }
                    egui::Key::F10 => self.show_kbd_help = !self.show_kbd_help,
                    egui::Key::F9 => self.aspect_correct = !self.aspect_correct,
                    egui::Key::F11 => self.debugger.open = !self.debugger.open,
                    _ => {}
                },
                egui::Event::Paste(text) => self.enqueue_text(text),
                _ => {}
            }
        }
    }
```

([`crates/coco-egui/src/app/input.rs:70-90`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/input.rs#L70-L90).) Two details repay a second
look. The pattern `pressed: true, repeat: false` means these toggles fire
once per physical press and ignore the operating system's auto-repeat —
holding F9 down does not strobe aspect correction on and off forty times a
second. And `Event::Paste` is a single event regardless of platform,
because, as the function's own doc comment notes, egui and eframe normalize
the platform paste shortcut — ⌘V on macOS, Ctrl+V elsewhere — into one
event. The frontend never has to know which operating system it is on.

### Positional versus symbolic: two philosophies, one matrix

`coco-egui` ships two entirely different answers to the question "which
CoCo key does this host keypress mean?", switchable live with F12, because
the two answers serve goals that cannot both be satisfied at once.

*Positional* mode, the default, maps physical key *location* to matrix
*position*, which is MAME's convention. Press the host key that sits where
a real CoCo key would sit, and whatever letter is printed on the CoCo key
underneath is what appears — including shift behavior, exactly as the
ROM's own scan-and-shift logic (Chapter 10) decides it. The map is a flat
`match` from `egui::Key` to the `(row, col)` `Pos` type Chapter 10 defined:

```rust
        K::A => (0, 1), K::B => (0, 2), K::C => (0, 3), K::D => (0, 4),
```

...through to the punctuation block, where the mapping stops being obvious:

```rust
        K::Minus => (5, 2),      // CoCo ':'
        K::Semicolon => (5, 3),  // CoCo ';'
        K::Comma => (5, 4),      // CoCo ','
        K::Equals => (5, 5),     // CoCo '-'
        K::Period => (5, 6),     // CoCo '.'
        K::Slash => (5, 7),      // CoCo '/'
```

([`crates/coco-egui/src/keymap.rs:5-43`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/keymap.rs#L5-L43).) Read those comments
carefully: pressing the host's minus key produces a colon on the CoCo,
because the CoCo's colon key sits where a US keyboard's minus key sits.
That looks like a bug and is the whole point. Positional mode promises
physical correspondence, not glyph correspondence.

Positional is what a game wants. An arcade-style CoCo game reads specific
matrix rows every field (Chapter 10's `sense()`), not ASCII characters, and it
expects "the key at this physical spot" to behave identically to a real
keyboard regardless of what glyph a modern operating system thinks that key
produces. The implementation is a direct, continuous mirror:

```rust
    pub(crate) fn drive_matrix_positionally(&mut self, events: &[egui::Event], mods: egui::Modifiers) {
        let joystick_keys = self.joysticks.keys_active();
        let kb = &mut self.machine.bus.keyboard;
        kb.set(kbd::SHIFT, mods.shift);
        kb.set(kbd::CTRL, mods.ctrl);
        kb.set(kbd::ALT, mods.alt);
        for ev in events {
            if let egui::Event::Key { key, physical_key, pressed, .. } = ev {
                let k = physical_key.unwrap_or(*key);
                if k == egui::Key::F12 {
                    continue;
                }
                if joystick_keys && is_joystick_key(k) {
                    continue;
                }
                if let Some(pos) = key_to_pos(k) {
                    kb.set(pos, *pressed);
                }
            }
        }
    }
```

([`crates/coco-egui/src/app/input.rs:115-135`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/input.rs#L115-L135).) The three modifier
lines are set from egui's `Modifiers` snapshot rather than from events,
because a modifier is a *level*, not an edge — what matters is whether
Shift is down right now, not whether it was pressed this frame. Then every
key event sets its mapped position true or false to match `pressed`. Note
`physical_key.unwrap_or(*key)`: the physical key is preferred where egui
can supply it, which is what makes positional mode behave the same on a
French AZERTY keyboard as on a US QWERTY one. And F12 is skipped
explicitly, since it belongs to the mode toggle rather than to the CoCo.

*Symbolic* mode maps the character you actually typed to whichever CoCo key
and shift state *produces* that character. That is `kbd::char_key`, which
Chapter 10 already walked in detail (§10.8), including the way it corrects for
the CoCo's inverted shift convention where unshifted keys show uppercase.
Symbolic is what you want for *typing*: paste a BASIC listing, or type at
the prompt on a non-US layout, and the letters that appear match the
letters you pressed, independent of physical key position.

Symbolic mode does not drive the matrix directly at all. Instead it
*queues*:

```rust
    pub(crate) fn queue_symbolic_taps(&mut self, events: &[egui::Event]) {
        let joystick_keys = self.joysticks.keys_active();
        for ev in events {
            match ev {
                egui::Event::Text(text) => self.enqueue_text(text),
                egui::Event::Key { key, pressed: true, .. } => {
                    if joystick_keys && is_joystick_key(*key) {
                        continue;
                    }
                    if let Some(pos) = control_key_pos(*key) {
                        self.type_ahead.queue.push_back((pos, false));
                    }
                }
                _ => {}
            }
        }
    }
```

([`crates/coco-egui/src/app/input.rs:94-110`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/input.rs#L94-L110).) Text events go through
`enqueue_text`, which is the shared path for both typed characters and
clipboard pastes:

```rust
    pub(crate) fn enqueue_text(&mut self, text: &str) {
        for c in text.chars() {
            if let Some(entry) = kbd::char_key(c) {
                self.type_ahead.queue.push_back(entry);
            }
        }
    }
```

([`crates/coco-egui/src/app/input.rs:17-23`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/input.rs#L17-L23).) Characters with no CoCo
key at all are silently skipped rather than substituted — pasting text
containing an em-dash or an accented vowel drops those characters instead
of injecting something wrong. Keys that produce no text, like Enter and the
arrows, do not arrive as `Event::Text` and so are handled separately by
`control_key_pos` ([`crates/coco-egui/src/keymap.rs:46-61`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/keymap.rs#L46-L61)), a second,
much smaller map for exactly that set.

Switching between the two modes is not free, and the code that does it is
one of those five-line functions that prevents a bug you would otherwise
spend an evening on:

```rust
    pub(crate) fn set_mode(&mut self, mode: KbMode) {
        if mode != self.kb_mode {
            self.kb_mode = mode;
            self.machine.bus.keyboard.release_all();
            self.type_ahead.clear();
        }
    }
```

([`crates/coco-egui/src/app/input.rs:7-13`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/input.rs#L7-L13).) Positional mode holds keys
down for as long as the host key is held. If the mode changes while a key
is down, the release event would be interpreted by the *new* mode — which,
in symbolic mode, ignores releases entirely — and that matrix position
would stay stuck down forever, with BASIC repeating a character until the
end of time. `release_all` makes the mode switch a clean slate, and
`type_ahead.clear()` does the same for any queue mid-drain.

### Contested keys: the joystick problem

Both modes share one more wrinkle, and it is the kind of detail that only
shows up once two features exist at the same time. When a joystick port is
set to `JoySource::Keys` — arrows for the axes, Z and X for the fire
buttons — those six keys must stop reaching the CoCo keyboard matrix
entirely, in *either* mode. Otherwise pressing right to steer also types a
character, and the game's own keyboard handler sees input the player never
meant to give it.

The arbitration is one predicate, consulted by both consumers:

```rust
/// Keys claimed by `joy::JoySource::Keys` (arrows for the axes, Z/X for the fire
/// buttons) once a joystick port uses that source — these stop reaching the CoCo
/// keyboard matrix so the two consumers don't fight over the same physical keys.
pub(crate) fn is_joystick_key(key: egui::Key) -> bool {
    matches!(
        key,
        egui::Key::ArrowUp
            | egui::Key::ArrowDown
            | egui::Key::ArrowLeft
            | egui::Key::ArrowRight
            | egui::Key::Z
            | egui::Key::X
    )
}
```

([`crates/coco-egui/src/keymap.rs:63-76`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/keymap.rs#L63-L76).) You saw it guarded by
`if joystick_keys && is_joystick_key(...) { continue; }` in both
`drive_matrix_positionally` and `queue_symbolic_taps` above. One predicate,
two call sites, one rule: whoever owns a physical key owns it exclusively.
That is a pattern worth stealing whenever two subsystems can both plausibly
claim the same input.

### `TypeAhead` as a lock

Chapter 10 already walked `TypeAhead::advance` in full — the hold-and-gap
state machine that drains queued taps one emulated *field* at a time rather
than one host frame at a time. The reason, from §10.8, is that a real key
press has to survive across multiple 60 Hz `KEYIN` scans of the ROM to
register at all; a press and release confined to a single field can land
entirely between two scans and simply vanish. The tuned constants are
`TYPE_HOLD_FIELDS = 2` and `TYPE_GAP_FIELDS = 1`
([`crates/coco-egui/src/main.rs:100-102`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/main.rs#L100-L102)): hold each synthesized keypress
for two fields, safely longer than one scan interval, then release for one
field before the next tap begins, so that two identical consecutive
characters — the `"AA"` in a pasted `DATA` statement — read as two separate
keystrokes instead of one long hold that the ROM's own debouncing (Chapter 10)
would collapse into a single `A`.

What is new to *this* chapter is how the rest of the application treats
that queue: as a lock on the keyboard matrix. The predicate is trivial:

```rust
    /// True while taps are still queued or a tap is mid hold/gap — i.e. a paste or
    /// type-ahead burst is still draining and owns the keyboard matrix.
    pub(crate) fn is_active(&self) -> bool {
        !self.queue.is_empty() || !matches!(self.phase, TypePhase::Idle)
    }
```

([`crates/coco-egui/src/typeahead.rs:41-45`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/typeahead.rs#L41-L45).) It is checked in exactly
one place for this purpose: `handle_input`'s early return, quoted at the
top of this section. While a paste or a symbolic-typed burst is still
draining, positional input is suppressed outright, in *both* keyboard
modes.

Think about what happens without that gate. The type-ahead queue writes a
matrix position true, expecting it to stay true for two whole fields. But
`drive_matrix_positionally` runs once per *host frame*, and on a 144 Hz
display that is more than twice per field. It would set every mapped
position to match the host's current key state — which, for the key
type-ahead is currently synthesizing, is *not pressed*, because the user's
finger is nowhere near it. The synthesized keypress would be cancelled
milliseconds after it started, and pastes would drop characters
unpredictably, more often on faster displays. Handing the queue
uncontested ownership of the matrix for its whole draining run is the fix,
and it costs two lines.

Advancing the queue happens in only one place too, and it is inside
`step_emulation`'s field loop rather than anywhere in `handle_input`:
`if self.type_ahead.is_active() { self.type_ahead.advance(...) }`, quoted
back in §15.2. Once per *emulated field*, never once per host frame. That
single placement decision is why paste timing stays correct on a 240 Hz
gaming monitor and a 30 Hz remote desktop session alike — it is paced by
`field_debt`, the same accumulator that paces everything else in the
machine.

---

## 15.5 Audio: one paragraph

Audio genuinely does deserve one paragraph here, and the brevity is a
feature: Chapter 11 built the entire chain, and repeating it would be padding.
`step_emulation` ends its `running` branch with two lines:

```rust
            let sample_rate = self.machine.audio_sample_rate();
            self.audio.push_samples(self.machine.take_audio(), sample_rate);
```

`self.machine.take_audio()` drains the per-field-rendered sample grid week
11 built — cycle-timestamped DAC events rendered once per scanline into an
oversampled grid ([`coco-core/src/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/audio.rs),
[`machine/audio.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/audio.rs)). `self.audio.push_samples` hands those samples to
the host audio chain Chapter 11 also covered in full: the DC blocker, the
Butterworth low-pass that prevents the decimation from folding
above-Nyquist content back into the audible band, the resampler down to the
device's real output rate, the cross-thread ring buffer a `cpal` callback
drains, and the underrun fade that replaces a click with silence when the
ring runs dry. Nothing about any of that changes in this chapter.

What *is* this chapter's business is the seam itself, and the audio
module's own doc comment describes it in one sentence worth quoting:

> The device runs on its own high-priority thread and pulls frames out of a
> `Mutex<VecDeque<[f32; 2]>>` that `push_samples` (called once per `update()`
> on the UI thread) fills. There is no synchronisation beyond that mutex —
> audio and video are independently paced, exactly like a real CoCo's TV and
> speaker.
> ([`crates/coco-egui/src/audio.rs:6-10`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/audio.rs#L6-L10))

That last clause is the design in miniature. The frontend does not try to
keep audio and video in lockstep, because the hardware never did: a real
CoCo's speaker and its television were driven by the same machine but
synchronized by nothing except both being fed from the same clock.
Here, `field_debt` decides how many fields run, those fields
generate however many samples they generate, and `push_samples` hands them
over at exactly that cadence. The ring buffer absorbs the jitter. Nothing
blocks, nothing waits, and there is no second thread in the emulator to
reason about — only in `cpal`, on the consuming side.

The "why" behind any of that filtering is Chapter 11's business, not this
chapter's.

---

## 15.6 The VM manager: machines as data

Everything up to here has been the emulator's own window. This section is
about a second window entirely — and about a category of code the course
has not touched in fourteen weeks.

Run the `coco` binary with no arguments and you do not get a booted
machine at all. You get the *manager*: a window in the style of VirtualBox
or Parallels, listing every machine you have defined, with a deck-style
transport — power on, suspend to disk, power off — and a detail pane for
editing hardware and attached media.
The dispatch is three lines in `main()` — "bare `coco` (no CLI arguments)
opens the CocoVM manager window; any argument keeps the direct-boot
emulator path" ([`crates/coco-egui/src/main.rs:110-114`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/main.rs#L110-L114)) — and everything
downstream of it is in [`crates/coco-egui/src/manager.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager.rs) and its
submodules.

This is worth studying for two reasons that have nothing to do with the
6809. First, it is the shape any serious frontend eventually needs around a
headless core, and the problems it solves — persistence, identity,
crash-safety, forward compatibility — are the same problems in any
application that saves the user's work. Second, it exercises the
borrow-checker discipline from Chapter 1 (§1.4) in a setting where no
hardware is involved at all, which is the best possible evidence that the
discipline was a general design principle rather than an emulator trick.

### Machine definitions as data, not code

A machine is a small, human-editable TOML file at
`config_dir()/machines/<slug>.toml`. Not a database. Not a binary format.
Not a serialized object graph. A file you can open in a text editor,
understand, and fix.

The type mirroring that file is `MachineDef`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MachineDef {
    /// Must equal [`CURRENT_SCHEMA`] to load; see that constant's doc.
    pub schema: u32,
    /// Display name — the manager list row's title.
    pub name: String,
    /// Informational only (e.g. an ISO date); never interpreted.
    #[serde(default)]
    pub created: Option<String>,
    pub hardware: HardwareDTO,
    #[serde(default)]
    pub media: MediaDTO,
    #[serde(default)]
    pub peripherals: PeripheralsDTO,
    #[serde(default)]
    pub ui: UIDTO,
```

([`crates/coco-egui/src/machine_def.rs:49-64`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/machine_def.rs#L49-L64).) It is deliberately a
*DTO* — a Data Transfer Object, meaning a struct whose only job is to
mirror an external data format field for field and be converted to and from
the types the program actually runs on. It is kept separate from
`coco_core::MachineConfig`, the type the emulator is built from.

Why not put `#[derive(Serialize, Deserialize)]` on `MachineConfig` and
save that? The module doc gives the reason, and it is a good one: an
internal `coco-core` refactor — renaming an enum variant, restructuring a
field, splitting one config into two — would otherwise silently change what
is written to disk, breaking every user's saved machines out from under
them, with no compiler error and no warning. The DTO is a deliberate
translation seam. It reads friendly strings on disk (`"512k"`,
`"mc6847t1"`) rather than whatever `coco-core`'s enum discriminants happen
to serialize as this week, and its `to_machine_config` runs
`MachineConfig::validate` along the way — so an invalid hardware
combination, a CoCo 2 asked for PAL, say, fails at load with one clear
error rather than at boot with a confusing one.

Every `#[serde(default)]` on that struct is a small forward-compatibility
promise too: a definition file with no `[media]` section at all is not an
error; it is a machine with no media.

### Surviving a newer version of yourself

The last field of `MachineDef` is the one worth imitating, and it solves a
problem most configuration formats simply lose to:

```rust
    #[serde(skip)]
    pub unknown: toml::Table,
```

([`crates/coco-egui/src/machine_def.rs:74-75`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/machine_def.rs#L74-L75).) Any TOML key the
loader does not recognize — at the top level, or one level into a known
section — is both logged via `tracing::warn!` and stashed in this field
rather than silently dropped. The collection is a plain double loop:

```rust
fn extract_unknown(table: &toml::Table) -> toml::Table {
    let mut unknown = toml::Table::new();
    for (key, value) in table {
        if !TOP_LEVEL_KEYS.contains(&key.as_str()) {
            unknown.insert(key.clone(), value.clone());
        }
    }
```

([`crates/coco-egui/src/machine_def/io.rs:69-75`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/machine_def/io.rs#L69-L75).) When the definition
is written back out, `merge_unknown` folds those keys into the freshly
serialized table before it hits disk
([`crates/coco-egui/src/machine_def/io.rs:99-118`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/machine_def/io.rs#L99-L118)).

The consequence is the thing to remember. Suppose a future build adds a
`[hardware].turbo_multiplier` key. A user sets it, then opens the same
machine in an older build that has never heard of it, changes the machine's
name, and saves. Without `unknown`, the older build would write out only
the keys it knows and the turbo setting would be gone — destroyed by an
unrelated edit, with no error and no way to notice until the newer build
was run again. With it, the key survives untouched.

Distinguish that from the *schema* number, which is handled the opposite
way. An unrecognized key is forward-compatible and merely warned about; an
unrecognized schema number is fatal, because it means the shape of the file
itself may have changed and no key-level reasoning is safe
([`crates/coco-egui/src/machine_def.rs:36-41`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/machine_def.rs#L36-L41)). Additive changes are
tolerated; structural ones are refused. That is exactly the right split.

### Atomic saves: tmp, then rename

`machine_def::save` never writes the final file directly:

```rust
    let tmp_path = dir.join(format!("{slug}.toml.tmp"));
    let final_path = dir.join(format!("{slug}.toml"));
    fs::write(&tmp_path, text).map_err(|e| format!("{}: {e}", tmp_path.display()))?;
    fs::rename(&tmp_path, &final_path).map_err(|e| format!("{}: {e}", final_path.display()))?;
```

([`crates/coco-egui/src/machine_def/io.rs:204-207`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/machine_def/io.rs#L204-L207).) The complete new
contents go to a sibling `.tmp` file first, and only then is that file
renamed over the real path.

The property this buys is worth stating precisely, because "atomic" gets
used loosely. A crash, a power loss, or a `kill -9` at *any* instant during
this sequence leaves the filesystem holding either the untouched old file
or the fully-written temporary one. It can never leave a half-written
`<slug>.toml`, because a rename over an existing path is atomic with
respect to a concurrent reader on every platform this program targets: a
reader either observes the file before the rename or after it, never
during. Writing in place has no such guarantee — a process killed halfway
through `fs::write` leaves a truncated file that will not parse, and the
user's machine definition is gone.

The same pattern appears verbatim for thumbnail PNGs a few paragraphs
below. The manager applies it to *every* file it writes on the
user's behalf, not merely to the one that would be most embarrassing to
lose, which is the right instinct: the discipline is cheap enough that
deciding case by case costs more thought than it saves.

### The slug is the identity

A machine's filename stem — its *slug* — is its persistent identity, not
its display name. Renaming a machine from "Alpha" to "Alpha Two" changes
what the list row says; it also has to change what the file is called, and
those are two different operations with different failure modes.

`slugify` lowercases, keeps `[a-z0-9]`, collapses every run of other
characters to a single dash, trims leading and trailing dashes, and falls
back to `"machine"` if nothing survives
([`crates/coco-egui/src/machine_def.rs:137-156`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/machine_def.rs#L137-L156)). Collisions are then
resolved by a function that is pleasingly boring:

```rust
pub fn unique_slug(base: &str, taken: &dyn Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let mut suffix = 2u32;
    loop {
        let candidate = format!("{base}-{suffix}");
        if !taken(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}
```

([`crates/coco-egui/src/machine_def.rs:161-173`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/machine_def.rs#L161-L173).) The `taken` predicate is
passed in rather than hard-coded, and the call sites are where the care
shows. Creating a machine checks *both* the in-memory list and the
directory on disk, because the in-memory list would miss a `<slug>.toml`
written by a second running instance or placed there by hand since startup
([`crates/coco-egui/src/manager/lifecycle.rs:44-48`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/lifecycle.rs#L44-L48)). Without the
on-disk half of that check, `save`'s unconditional rename would silently
overwrite somebody else's file.

Renaming is therefore exactly two filesystem moves: `<old>.toml` to
`<new>.toml`, and the machine's artifact directory alongside it. If the
second move fails partway, the first is rolled back — the definition file
is renamed back rather than left pointing at a directory that no longer
matches its own name
([`crates/coco-egui/src/manager/lifecycle.rs:213-266`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/lifecycle.rs#L213-L266)). The comment
on that rollback states the priority plainly: "a stale slug beats relative
[media] entries resolving into a directory that no longer matches the
definition's file name."

Why the artifact directory has to follow at all comes down to one function.
Relative `[media]` paths inside a definition resolve *against the slug's
own artifact directory* ([`crates/coco-egui/src/machine_def.rs:193-202`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/machine_def.rs#L193-L202)),
so a disk image referenced as `"disk0.dsk"` means a different absolute file
the instant the slug changes. Move the definition without moving the
directory and every relative media path in it silently points at nothing.

There is one more subtlety, and it is the sort of thing that only appears
once a feature meets a real user. A machine cannot be renamed on disk while
its VM is running — the running VM writes `thumbnail.png` into the
artifact directory by path, and renaming out from under it races — nor
while it is *suspended*, for a subtler reason: the frozen
`suspended.ccstate` records its media by absolute, pre-rename path (week
16's `MediaRefs`), so moving the artifact directory under it would make
the frozen state unrestorable. So a rename requested in either state
merely sets a flag, and a separate pass picks it up once the machine is
powered off:

```rust
    pub(super) fn apply_pending_renames(&mut self) {
        while let Some(index) = self
            .entries
            .iter()
            .position(|e| e.rename_pending && e.vm.is_none() && !e.suspended)
        {
            self.migrate_slug(index);
        }
    }
```

([`crates/coco-egui/src/manager/lifecycle.rs:279-287`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/lifecycle.rs#L279-L287).) It runs once per
`update()`, before any panel draws, so row indices stay stable for the
whole frame — and it re-searches from scratch each iteration rather than
iterating indices, because each `migrate_slug` re-sorts the list
alphabetically underneath it. The loop terminates because `migrate_slug`
clears `rename_pending` unconditionally, on success or failure. Exercise
15.5 asks you to connect this deferral to a kittest test that has to step
three frames instead of two.

### Start, Suspend, Stop: where the machine lives

A machine is always in exactly one of three states — **Powered Off**,
**Running**, or **Suspended** (frozen to disk, resumable later, even after
quitting the manager) — and one row of the list is one `MachineEntry`: a
slug, a parsed `MachineDef`, and two fields that between them encode that
state. The first changes what is actually *executing*:

```rust
    pub vm: Option<Box<CocoApp>>,
```

That field is [`crates/coco-egui/src/manager.rs:116`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager.rs#L116), inside the struct at
[`crates/coco-egui/src/manager.rs:108-146`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager.rs#L108-L146). Powered Off is `None`. The
second, `suspended: bool`, mirrors something that lives on disk: a
suspended machine's whole frozen state is a `suspended.ccstate` file in
its artifact directory (written by week 16's save-state engine), and *the
file's existence is the state* — the flag is only a per-frame cache so
drawing never has to stat the filesystem, seeded from the file when the
manager starts. The status label is derived from the pair rather than
stored:

```rust
fn vm_status_label(entry: &MachineEntry) -> &'static str {
    if entry.suspended {
        STATUS_SUSPENDED
    } else if entry.vm.is_some() {
        STATUS_RUNNING
    } else {
        STATUS_POWERED_OFF
    }
}
```

([`crates/coco-egui/src/manager.rs:173-181`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager.rs#L173-L181).) Three states, computed
fresh at draw time; the only one with any persistence is Suspended, and
its persistence is the state file itself, not a status field in the
definition. This is §15.1's lesson applied to application state rather
than widgets: do not store what you can compute, because stored copies go
stale and computed ones cannot — and when something genuinely must
persist, make the artifact itself the truth rather than a second record of
it.

Starting is short:

```rust
    pub(super) fn start_vm(&mut self, index: usize) {
        let entry = &mut self.entries[index];
        entry.launch_error = None;
        match crate::launch_machine(&entry.def, &entry.slug) {
            Ok(vm) => entry.vm = Some(Box::new(vm)),
            Err(e) => entry.launch_error = Some(e),
        }
    }
```

([`crates/coco-egui/src/manager/lifecycle.rs:72-79`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/lifecycle.rs#L72-L79).) `launch_machine`
is the manager's counterpart to the CLI's `boot::boot_app`; both build a
`CocoApp` from a config plus a set of mounted media, and the two modules'
doc comments name each other as siblings
([`crates/coco-egui/src/launch.rs:29-42`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/launch.rs#L29-L42)). The important difference is
error handling: the CLI path prints and exits, while this one must return
an `Err` for the detail pane to display, since crashing the manager because
one machine's disk image is missing would be absurd. On failure `vm` is
left untouched at `None`, so a failed Start leaves a Powered Off row
rather than a half-constructed one.

Suspend is the transport's ⏸, enabled only while Running, and it composes
three things this chapter and the next already own: capture the screen as
the row's preview (`write_entry_thumbnail`), freeze the whole machine to
`suspended.ccstate` with week 16's `save_state_to` (which flushes dirty
media as part of its own contract), and stop the clock in place
(`set_running(false)`) — in that order, so the screenshot is exactly the
frozen frame ([`crates/coco-egui/src/manager/lifecycle.rs:91-116`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/lifecycle.rs#L91-L116)).
The window stays open as a display-only viewing port (§ below), and
closing it merely drops the `Box` — the state is already safe on disk.
Play on a suspended machine (`resume_vm`,
[`crates/coco-egui/src/manager/lifecycle.rs:130-159`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/lifecycle.rs#L130-L159)) either
un-pauses the still-alive VM or, if the window was closed, launches fresh
and restores the frozen state over it — and, either way, *deletes* the
state file, because the running machine immediately diverges from the
frozen copy and a stale file would misreport Suspended after the next
power-off. A failed restore keeps the file and the Suspended state: the
frozen copy is still the truth.

Stopping is the power switch:

```rust
    pub(super) fn stop_vm(&mut self, index: usize) {
        if let Some(mut vm) = self.entries[index].vm.take() {
            vm.flush_media();
        }
        let entry = &mut self.entries[index];
        entry.suspended = false;
        entry.thumbnail = None;
        entry.thumbnail_load_attempted = false;
        if let Some(root) = &self.artifacts_root {
            let dir = root.join(&entry.slug);
            for file in [SUSPEND_STATE_FILE, THUMBNAIL_FILE] {
                if let Err(e) = fs::remove_file(dir.join(file))
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!("could not remove {file} for '{}': {e}", entry.slug);
                }
            }
        }
    }
```

([`crates/coco-egui/src/manager/lifecycle.rs:173-191`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/lifecycle.rs#L173-L191).) Flush dirty
media — the same `flush_media` that `CocoApp::on_exit` runs for the
direct-boot window — drop the `Box`, and discard both halves of any frozen
state: the `.ccstate` and the screenshot, since a powered-off row shows a
black preview, never a stale picture. Dropping the `Box` is what shuts the
machine down; there is no `shutdown()` method, because Rust's ownership
already provides one.

> **Rust corner — `Option<Box<CocoApp>>`, not `Option<CocoApp>`.**
> `CocoApp` is a large struct. It contains the whole `Machine` (CPU, RAM,
> the GIME, both PIAs, every optional cartridge device) plus every UI
> dialog's own state: the debugger panel, the paper window, and thirty-odd
> more fields. With `Option<CocoApp>`, every
> `MachineEntry` in the list would pay the full size of that struct
> regardless of whether the machine is running — including entries for
> machines that are, and always will be, stopped, since `Option<T>` is at
> least as large as `T`.
>
> Boxing puts the actual `CocoApp` on the heap and leaves only a
> pointer-sized `Option<Box<_>>` inline in the entry, so a manager listing
> fifty stopped machines carries fifty small `None`s rather than fifty
> machine-sized empty slots. This is the general rule for
> occasionally-present, expensive-to-hold fields in Rust: box the payload
> and keep the container thin. The `vm` field's own doc comment states the
> rationale in one sentence, which is the right place for it.

### One native OS window per running VM

`draw_running_vms` is called once per `ManagerApp::update`, after the
manager's own panels, and opens one *immediate viewport* — a real,
separate, native operating-system window — for every entry with a live VM.
The loop's core is three lines:

```rust
            let mut vm = self.entries[i].vm.take().expect("checked Some above");
            let suspended = self.entries[i].suspended;
            let mut close_requested = false;
            ctx.show_viewport_immediate(viewport_id, builder, |child_ctx, class| {
```

([`crates/coco-egui/src/manager/vm_windows.rs:64-67`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/vm_windows.rs#L64-L67).) The
`viewport_id` above it is
`egui::ViewportId::from_hash_of(("vm-window", &slug))`, which gives each
VM's window a stable identity across frames. That stability is what makes
egui reuse the *same* operating-system window rather than destroying and
recreating one every update — the manager's version of the same "identity
persists, widgets don't" discipline §15.1 opened with, applied to whole
windows instead of buttons. The slug, once again, is the identity.

The closure receives a `class` telling it what kind of viewport it actually
got, and branches. On a backend with real multi-window support, a Running
machine takes the straightforward path:

```rust
                } else {
                    vm.window_ui(child_ctx);
                    if child_ctx.input(|i| i.viewport().close_requested()) {
                        close_requested = true;
                    }
                }
            });

            self.entries[i].vm = Some(vm);
            if close_requested {
                to_stop.push(i);
            }
```

([`crates/coco-egui/src/manager/vm_windows.rs:135-146`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/vm_windows.rs#L135-L146).) That single
`vm.window_ui(child_ctx)` call is the payoff for everything §15.2 and
§15.3 established. The *entire* direct-boot experience — menu bar, toolbar,
status bar, the letterboxed display, every dialog — runs unmodified inside
this child viewport, through the very same `window_ui` this chapter has
been reading all along. There is no second implementation of the emulator
window for the manager to maintain, and no risk of the two drifting apart,
because there is only one.

A *Suspended* machine whose window is still open takes a middle branch
([`crates/coco-egui/src/manager/vm_windows.rs:112-134`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/vm_windows.rs#L112-L134)):
just the framebuffer-texture upload plus the display — no chrome, and no
`step_emulation` either, since its `handle_input` would keep the
quick-load/quick-save shortcuts and keyboard/joystick writes live. The
window becomes a viewing port onto the frozen frame, never a control
surface — a full chrome would leave Reset, Load State, disk mounts, and
the debugger's own Run/Step pointed at a machine whose on-disk frozen copy
they would silently diverge from, and one stray click later, Play would
"resume" (and discard the state file of) a machine that no longer matches
what the user froze.

The last branch is the degraded case, and it exists because not every
backend can open real child windows — kittest, the headless test backend
§15.8 covers, is exactly this case. There, `class` is
`ViewportClass::Embedded` and the fallback deliberately shows *only* the
bare display:

```rust
                    vm.step_emulation(child_ctx);
```

(for a Running machine — a suspended one gets the same
texture-upload-only gating here as in the native branch), followed by an
anchored `egui::Window` whose body is just `vm.draw_display(ui)`
([`crates/coco-egui/src/manager/vm_windows.rs:82-108`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/vm_windows.rs#L82-L108)). Two decisions
in that fallback are worth reading the comments for. It skips
`draw_chrome` entirely because drawing two independent sets of menu bars
and status bars into one shared context would interleave them into a single
confusing window. And it caps the window at
`EMBEDDED_FALLBACK_SIZE = 320×240` rather than the native window's full
size, because — as the comment records, "found the hard way, via a kittest
regression" — a window that large, even anchored to a corner, spans most of
a modest canvas and silently eats clicks meant for the manager's own panels
underneath ([`crates/coco-egui/src/manager/vm_windows.rs:8-17`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/vm_windows.rs#L8-L17)).

Notice, finally, that the fallback still calls `step_emulation`. This is
the promise `step_emulation`'s doc comment made back in §15.2, now
collected: the VM keeps emulating even in the degraded single-window case,
because emulation was deliberately separated from the drawing of chrome.

> **Rust corner — `.take()` to split a borrow, one level up from Chapter 1.**
> Look at that loop again. `self.entries[i].vm.take()` moves the
> `Box<CocoApp>` out of the entry *before* the closure runs, into a local
> `vm` the closure captures by unique reference, leaving the entry's slot
> holding `None` for the closure's whole duration — and then restores it
> afterwards with `self.entries[i].vm = Some(vm)`.
>
> Why not simply borrow `&mut self.entries[i].vm` inside the closure? The
> function's own doc comment answers it: close requests are collected into
> a plain local `Vec<usize>` and applied *after* the loop with
> `close_vm_window` — the power switch for a Running machine, a mere
> VM-object drop for a Suspended one — because that needs
> `&mut self.entries[i]`, which would conflict with the `vm` the loop is
> already holding out of that same slot. Holding a live borrow of `self`
> across the closure body would collide with the surrounding loop's own
> indexing of `self.entries`.
>
> This is the exact move from Chapter 1, §1.4 — `Machine { cpu, bus }` as
> disjoint fields so that `cpu.step(&mut bus)` compiles — recurring in
> ordinary application code with no hardware anywhere in sight. When the
> borrow checker will not let you hand out two overlapping mutable views of
> the same owner, take the piece you need *out*, use it standalone, and put
> it back. The `expect("checked Some above")` is honest about the one
> invariant that makes it safe: the loop already skipped entries whose `vm`
> is `None`.

### Thumbnails: the suspend-time screenshot

Each machine's row preview is a direct statement of its state. A Running
machine shows its live framebuffer — the very texture §15.3 uploaded,
drawn a second time in a smaller rectangle, one extra quad and no extra
upload. A Powered Off machine shows plain black, like the screen of a
machine with no power. And a Suspended one shows the *frozen frame*: at
suspend time, `write_entry_thumbnail`
([`crates/coco-egui/src/manager/thumbnails.rs:18-34`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/thumbnails.rs#L18-L34)) captures the
framebuffer as a plain PNG at `<artifact-dir>/<slug>/thumbnail.png` —
written with the same temporary-file-then-rename atomicity as machine
definitions — so the row keeps showing that exact frame after the VM
window closes, and even across manager restarts. Powering off deletes it
along with the state file; the screenshot has no meaning without the
frozen machine it depicts.

While the suspended VM object is still alive its (unchanging) live texture
serves as the preview for free; the PNG is loaded back lazily, only once
the object is gone
([`crates/coco-egui/src/manager/thumbnails.rs:42-62`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/thumbnails.rs#L42-L62)).

One small heuristic in the PNG writer deserves attention:

```rust
    let all_black = rgba
        .chunks_exact(4)
        .all(|px| px[0] == 0 && px[1] == 0 && px[2] == 0);
    if all_black && final_path.exists() {
        return Ok(());
    }
```

([`crates/coco-egui/src/manager.rs:209-214`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager.rs#L209-L214).) Consider what a
screen capture is exposed to. The CoCo's screen is genuinely, uniformly
black at plenty of legitimate moments: during a mode switch, right after a
`CLS 0`, in the instant following a reset before the ROM has painted
anything. Suspend during one of those — on a machine suspended and resumed
before, so a useful screenshot already exists — and that preview would be
clobbered by a solid black square through sheer bad luck of timing.

Skipping the write when the frame is uniformly black *and* a previous
thumbnail already exists prevents that. The second half of the condition
matters just as much as the first: the *very first* thumbnail a machine
ever writes must still land even if that frame happens to be black, since
skipping it then would leave the row with no preview at all. Two clauses,
two distinct guarantees, and §15.11's sabotage exercise asks you to find
both of them by breaking one.

---

## 15.7 Media UI pattern: `disk.rs` as the exemplar

The `media/` directory is where the frontend meets every device Chapters 12
through 14 built. Cartridges, floppies, virtual hard disks, DriveWire
disks, the cassette deck, the printer bit-banger — each gets its own file,
each file is nothing but `impl CocoApp` methods, and all of them follow the
same shape. Learn the shape once from the clearest example and you can read
any of the others cold.

[`media/disk.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/media/disk.rs) is that example, because floppies are the only media
type with real in-memory dirty state to manage. VHD and DriveWire images
write straight through to their backing file on every command, so there is
nothing to flush and nothing to lose; a floppy image lives entirely in
memory until somebody decides to write it back, which means every question
about *when* to do so has to be answered explicitly.

**Ensure the controller exists.** Before a floppy can be mounted, an FD-502
has to be in the cartridge slot. `ensure_disk_controller` inserts one (week
13), loading `roms/disk11.rom` — and then does something that looks
excessive until you know the ROM:

```rust
        self.machine.insert_cartridge(DiskCart::new(rom.into_boxed_slice()));
        // Power cycle, not warm reset: the DK probe that links Disk BASIC
        // only runs on the ROM's cold-start path (a warm reset leaves the
        // DOS ROM unlinked and the drives dead).
        self.machine.power_cycle();
```

([`crates/coco-egui/src/media/disk.rs:51-55`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/media/disk.rs#L51-L55).) The function's own doc
comment states the rule from the user's side: "Creating it cold-resets the
machine: BASIC only probes for Disk BASIC at cold start. Swapping a floppy
in an already-present controller does NOT reset, like on real hardware"
([`crates/coco-egui/src/media/disk.rs:9-11`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/media/disk.rs#L9-L11)).

This is the frontend enforcing a real hardware constraint that Chapter 13's
ROM archaeology already uncovered from the other side. The DK probe that
links Disk BASIC into the language runs only on the cold-start path, so
*inserting the controller* must be a power cycle — while *swapping a disk
in it* must not be, because on real hardware nobody power-cycled a CoCo
to change floppies. Two superficially similar operations, opposite
behavior, and the difference is dictated by the ROM rather than by
convenience. Because a power cycle destroys unsaved state, the menu path
does not do it silently: `request_insert_disk` parks the action behind a
confirmation dialog when the controller is not present yet, and acts
immediately when it is ([`crates/coco-egui/src/media/disk.rs:64-73`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/media/disk.rs#L64-L73)).

**Insert acts, write-back protects.** `insert_disk` mounts the new image —
but not before dealing with whatever was already there. The sequence is
`ensure_disk_controller`, read the file, parse it as a `JvcDisk`, then
`self.write_back_disk(drive)` with the comment "whatever was in the drive
first", and only then `cart.insert_disk(drive, disk)`
([`crates/coco-egui/src/media/disk.rs:86-102`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/media/disk.rs#L86-L102)). Unsaved changes to the
outgoing floppy are never silently discarded. Note also the ordering with
respect to failure: the file is read and parsed *before* anything is
disturbed, so a corrupt image leaves the previously mounted disk exactly
where it was.

**Dirty tracking lives in the device, not the frontend.** The single
function that decides whether a floppy needs saving is this one:

```rust
    pub(crate) fn write_back_disk(&mut self, drive: usize) {
        let Some(path) = self.disk_paths[drive].clone() else {
            return;
        };
        let Some(cart) = self.machine.bus.cart.as_disk_cart() else {
            return;
        };
        let Some(disk) = cart.disk(drive) else {
            return;
        };
        if !disk.dirty() {
            return;
        }
        if let Err(e) = std::fs::write(&path, disk.bytes()) {
            self.cart_error = Some(format!("could not save {}: {e}", path.display()));
        }
    }
```

([`crates/coco-egui/src/media/disk.rs:146-162`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/media/disk.rs#L146-L162).) Four guard clauses and
one write. The frontend never guesses whether a disk changed; it asks the
mounted `JvcDisk` itself via `disk.dirty()` (Chapter 13), deferring entirely
to the device that actually knows. That is the correct division: the
frontend knows *where* the image came from, the device knows *whether* it
changed, and neither pretends to know the other's business.

Failure is handled by not pretending it succeeded. The error lands in
`self.cart_error` — which §15.1 showed becoming a dismissible banner purely
by virtue of being `Some` — and the in-memory disk is left mounted and
still dirty. A later retry, or the next `flush_dirty_disks` on exit, can
therefore still succeed without the user having lost the edit. Compare that
to clearing the dirty flag optimistically: a full disk or a read-only
filesystem would then silently eat somebody's afternoon of BASIC.

**Eject always writes back first.** `eject_disk` calls `write_back_disk`
before `cart.eject_disk(drive)`, never after
([`crates/coco-egui/src/media/disk.rs:135-141`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/media/disk.rs#L135-L141)) — exactly like ejecting
a real floppy from a real drive after the operating system has finished
with it, and for exactly the same reason.

The contrast that proves the pattern is `insert_vhd` and `eject_vhd` in the
very same file. Their doc comments explain that a VHD is a bus-level device
independent of the cartridge slot: "no controller to ensure, no machine
reset, and no write-back on eject/replace (VHD command execution writes
straight through to the backing file)"
([`crates/coco-egui/src/media/disk.rs:171-175`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/media/disk.rs#L171-L175)). Every step of the
floppy dance exists because of a specific property of floppies, and a
device without those properties skips every step. Exercise 15.6 asks you to
state that in one sentence after tracing the chain yourself.

Every other file in the directory repeats this shape with whatever
specifics its device demands. A cartridge has no dirty concept at all. A
cassette's write-back optionally also synthesizes a `.wav` alongside the
canonical `.cas`, per Chapter 12. This directory is also where the loose end
from Chapter 14's closing paragraph finally gets tied off: the DMP-105's
protocol and fixed-point paper coordinates were Chapter 14's material, while
the scrolling "Printer Paper" window a user watches fill up during an
`LLIST` ([`crates/coco-egui/src/paper_view.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/paper_view.rs)) is `coco-egui` UI state
built on top of it — one more device attached and detached by the same
request-then-mount discipline as a floppy.

---

## 15.8 Testing the UI headlessly: kittest

Everything in this chapter so far — menus, dialogs, the manager's list, the
running-VM viewports — has a real automated test suite, and none of it
opens a visible window or needs a human at a monitor. That claim deserves
skepticism, because "GUI testing" has an earned reputation for flakiness,
so this section explains exactly how it works and what it costs.

The tool is `egui_kittest`, a dev-dependency
([`crates/coco-egui/Cargo.toml:65-66`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/Cargo.toml#L65-L66)). It runs egui's *real* layout and
input logic against a headless backend — the same code that would run in
front of a user, not a mock — and then exposes the result as an *AccessKit*
accessibility tree. AccessKit is the structured representation a screen
reader consumes: a tree of nodes, each with a role (button, checkbox, text
input) and a label. Tests query that tree by label rather than by pixel
coordinate, which turns out to change everything about how durable they
are.

The test module's own doc comment opens with a list headed "Interaction
conventions discovered the hard way," and it is worth reading before the
code, because each line is a bug somebody already paid for:

> - Clicks hover on one frame and press/release on the next: egui routes a
>   press using the previous frame's hit-test data, so a press with no
>   prior hover misses windows that were (re)anchored this frame.
> - Menus close on *any* item click (egui's default menu close behavior),
>   so every menu interaction reopens the menu from the bar.
> - Submenu buttons expose their label with a trailing "⏵" arrow — match
>   them with `_contains`, not exactly.
> ([`crates/coco-egui/src/ui_tests.rs:7-14`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/ui_tests.rs#L7-L14))

### Booting a harness

[`ui_tests/harness.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/ui_tests/harness.rs) is the shared infrastructure every test file in
`ui_tests/` imports. Booting a direct-boot app harness looks like this:

```rust
pub(super) fn boot_harness() -> AppHarness {
    let roms_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms");
    let rom = load_default_rom(MachineVariant::Coco3, &roms_dir)
        .expect("roms/coco3.rom is required (git-ignored, local-only)");
    let rom_source = RomSource::File(roms_dir.join("coco3.rom"));
    let mut harness = egui_kittest::Harness::new_eframe(|_cc| {
        CocoApp::new(
            MachineConfig::default(),
            rom,
            rom_source,
            None,
            [None, None],
            [None, None],
            std::array::from_fn(|_| None),
            false,
            false,
            false,
        )
    });
    // Room for the full Machine menu: egui only puts on-screen widgets in
    // the AccessKit tree, so a too-small viewport hides the lower items.
    harness.set_size(egui::vec2(1024.0, 768.0));
    harness.step();
    harness
}
```

([`crates/coco-egui/src/ui_tests/harness.rs:20-44`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/ui_tests/harness.rs#L20-L44).) Three things in
there are worth calling out.

`Harness::new_eframe` takes the same `CocoApp::new` constructor the real
`boot_app` calls, with the same ten arguments, and no test-only shortcuts.
There is no `CocoApp::new_for_testing`. Whatever the harness exercises is
what the application does.

`harness.step()` is one simulated frame. It runs `update()` exactly as a
real event loop would, and it is called *manually*, exactly once per unit
of simulated time. That is the property that makes these tests
deterministic where GUI tests usually are not: nothing happens between
steps, so a test controls pacing precisely instead of sleeping and hoping.

And the comment on `set_size` names a category of headless-testing gotcha
that has nothing to do with emulation: egui only puts *currently visible*
widgets into the AccessKit tree. A viewport too small to show the whole
Machine menu makes its lower items simply un-queryable — not disabled, not
hidden behind a flag, just never laid out this frame, and therefore absent
from the tree the test searches. The failure looks like "the menu item does
not exist," which sends you hunting in the wrong file.

The manager harness does the same dance with `ManagerApp::new`, and adds
one discipline the direct-boot harness does not need:

```rust
pub(super) fn manager_harness_with_artifacts(
    machines_dir: Option<PathBuf>,
    artifacts_root: Option<PathBuf>,
    entries: Vec<manager::MachineEntry>,
) -> ManagerHarness {
    let mut harness = egui_kittest::Harness::new_eframe(move |_cc| {
        manager::ManagerApp::new(None, machines_dir, artifacts_root, entries)
    });
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();
    harness
}
```

([`crates/coco-egui/src/ui_tests/harness.rs:191-202`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/ui_tests/harness.rs#L191-L202).) Both directory
paths are *injected* — as `None`, or as a temporary directory — and never
the real user configuration or data directories. That injection is why
`ManagerApp::new` takes them as parameters at all rather than calling
`machine_def::machines_dir()` itself; the production call site passes the
real ones in `manager::run`. A test suite that could read or delete a
developer's actual saved machines would be a test suite nobody runs twice.

### Interaction: hover, step, click, step, step

Every click helper in the harness follows the same five-beat rhythm:

```rust
pub(super) fn click<S: 'static>(harness: &mut egui_kittest::Harness<'static, S>, label: &str) {
    harness.get_by_label(label).hover();
    harness.step();
    harness.get_by_label(label).click();
    harness.step();
    harness.step();
}
```

([`crates/coco-egui/src/ui_tests/harness.rs:50-56`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/ui_tests/harness.rs#L50-L56).) Hover, step a
frame, click, step twice more. Each beat exists for a reason the module doc
listed above.

The hover comes first because, as that doc says, egui routes a press using
the *previous* frame's hit-test data — so a press with no prior hover
misses windows that were anchored or re-anchored this frame. The step
between hover and click lets that hit-test data become current. The click
itself is a press and a release, and egui fires `clicked()` on the release.
And the two trailing steps let whatever the click *caused* — a menu
opening, a row becoming selected, a modal appearing — be fully reflected in
the accessibility tree before the test's next assertion reads it.

Skipping any of these beats is the single most common way a kittest test
becomes flaky, and the failure is misleading: the application logic is
fine, and the test merely asserted on a tree state egui had not finished
producing.

### Why label-based queries beat coordinates

`harness.get_by_label("Reset")` finds the one accessible node whose
AccessKit label is exactly `"Reset"`, and panics if there is either no
match or more than one. It is deliberately as strict as `unwrap()`.

Compare that to a coordinate-based test — something like clicking at
position (340, 22). It would pass today. It would silently start clicking
the *wrong thing* the day anyone reorders a menu, resizes the toolbar,
changes a font, or adds an item above the one being targeted. And nothing
about the failure would point at the cause: the test would exercise some
other control and assert on state that control never touched.

A label-based test survives exactly the kind of refactor this codebase does
constantly — the module-splitting visible throughout its own commit history
— because it asserts on *meaning*: "the control labeled Reset." Not on
where that meaning happened to render this week. It is the UI-testing
analogue of asserting on a register's value rather than on a specific
memory address that happens to hold it.

Strictness has a cost, of course, which is that label collisions must be
handled explicitly rather than papered over. Three helpers do that.
`click_containing` matches by substring, for labels carrying decoration the
visible caption does not show — a submenu's trailing "⏵" ("MultiPak
Interface ⏵", "Slot 1 ⏵"). `lowest_by_label` and `click_in_menu`
disambiguate a menu-popup copy of a label that the toolbar *also* shows —
"Reset" appears in both places at once — by picking whichever matching node
sits lowest on screen, since a popup always hangs below the toolbar row
that opened it. And for the genuinely intentional duplicates there is:

```rust
pub(super) fn label_exists<S: 'static>(harness: &egui_kittest::Harness<'static, S>, label: &str) -> bool {
    harness.get_all_by_label(label).next().is_some()
}
```

([`crates/coco-egui/src/ui_tests/harness.rs:209-211`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/ui_tests/harness.rs#L209-L211).) Its doc comment
names the case exactly: a status word like "Running" is deliberately shown
twice at once, once weakly in the list row and once strongly in the detail
pane header, both driven by the same `vm_status_label` from §15.6. Asserting
that it exists is right; asserting that it exists *once* would be asserting
on a layout decision.

There is one more addressing wrinkle worth knowing, because it is the kind
of thing that costs an hour if nobody wrote it down. A combo box does not
expose its current selection as a label at all — egui sets it as the
accessibility *value* instead — so `select_combo_at` addresses the combo
button with `get_by_value` and the popup items, which are plain
selectables, with `get_by_label`
([`crates/coco-egui/src/ui_tests/harness.rs:79-115`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/ui_tests/harness.rs#L79-L115)). Role, label, and
value are three different axes of the accessibility tree, and knowing which
one a widget uses is most of the skill in writing these tests.

### Reading one real test

`manager_row_context_menu_delete_confirms_and_removes`
([`crates/coco-egui/src/ui_tests/manager_window.rs:226-254`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/ui_tests/manager_window.rs#L226-L254)) is worth
reading start to finish, because it follows the exact sequence of clicks a
human tester would perform, written in something very close to English:

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
    assert_eq!(
        harness.state().detail_name(),
        Some("Beta CoCo 3"),
        "the selection must follow the surviving row as indices shift"
    );
```

Select Beta. Right-click Alpha — and note that this must *not* move the
selection, a real user-facing decision the code comment records: the context
menu acts on the row under the cursor, not on whatever happens to be
selected. Open Delete…, click Cancel, and confirm that nothing changed and
the file survives. Then right-click Alpha again, Delete…, and this time
confirm: the row and its `.toml` are both gone, Beta's file is untouched,
and — the assertion that would be easy to forget to write — the selection
has followed Beta down from index 1 to index 0 rather than silently
pointing at whatever now occupies index 1.

`harness.state()` is the other half of what makes these tests readable.
It is plain field access into the real `ManagerApp` the harness owns, which
lets a test look past the user interface entirely and inspect the actual
model — the same way you would inspect `Machine` fields directly in a
`coco-core` test rather than trying to read pixels off a rendered screen.
The user interface is driven like a user drives it; the assertions are made
where the truth lives.

> **Rust corner — one function, two apps.** Look at the signature again:
> `click<S: 'static>(harness: &mut egui_kittest::Harness<'static, S>, label: &str)`.
> It is generic over `S`, the app type the harness wraps, and this exact
> function drives both `CocoApp` harnesses and `manager::ManagerApp`
> harnesses with no duplication — because `Harness<'static, S>`'s
> hover/step/click methods do not care what `S` is. They only need it to
> satisfy the `Queryable` trait's `'static` bound.
>
> This is the same monomorphization story from Chapter 1's `Bus` trait
> (§1.3), arriving from a completely different direction. The compiler
> emits one specialized copy of `click` for `S = CocoApp` and another for
> `S = manager::ManagerApp`, so sharing this helper across two otherwise
> unrelated app types costs nothing at run time. Generic code paid off in
> the CPU crate's hot loop; here it pays off in test code, where the
> benefit is not speed but the absence of a second, subtly divergent copy
> of the click sequence.

---

## 15.9 Running the suite — an honest report

A chapter that claims a test suite exists owes you the actual output,
including the parts that do not pass. Here is `cargo test -p coco-egui`,
run in a worktree that — like every worktree that is not the main checkout
— has no `roms/` directory, since ROM images are git-ignored and local-only
by the project's own convention. This is what happened, not a sanitized
summary:

```
test result: FAILED. 83 passed; 30 failed; 0 ignored; 0 measured; 0 filtered out
```

**All 30 failures are ROM-required, and only ROM-required.** Every one
panics at a `std::fs::read`/`load_default_rom` call reading
`roms/coco3.rom` or (for the FD-502 tests) `roms/disk11.rom`, with a
message stating exactly that: `"roms/coco3.rom is required (git-ignored,
local-only)"`. The failing set breaks down cleanly into three groups:

- `debugger::tests::*` (5) and `save_state::tests::*` (1) — unit tests that
  boot a real `Machine` directly with `Machine::new(config, load_rom())`.
- `ui_tests::direct_boot_menus::*` (16) — every kittest test that calls
  `boot_harness()`, which requires the real system ROM to construct a
  `CocoApp` at all.
- `ui_tests::manager_lifecycle::*` (8) — every test that actually calls
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

The split is itself worth a moment's reflection, because it is the same
line Chapter 1 drew, showing up in the test results. The tests that need a
copyrighted ROM are exactly the tests that need a *machine*; the tests that
need only the application — its file format, its list management, its
arithmetic — need nothing but the repository. Anyone can clone this project
and immediately run eighty meaningful tests. That is not an accident of
packaging; it is what keeping the frontend's own logic separable from the
emulated hardware buys.

---

## 15.10 Reading assignment

In this order:

1. **[`crates/coco-egui/src/main.rs:1-102`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/main.rs#L1-L102)** — the crate's module list (a
   map of everything this chapter did and didn't cover) and the constants
   block: `SCALE`, `TARGET_ASPECT`, `MAX_FIELDS_PER_UPDATE`, `MAX_FRAME_DT`,
   `TYPE_HOLD_FIELDS`/`TYPE_GAP_FIELDS`.
2. **[`crates/coco-egui/src/app.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app.rs)** — the `CocoApp` struct in full; read
   every field's doc comment once, even the ones this chapter didn't
   discuss (Chapter 16 owns several of them: `debugger`, and everything
   [`save_state.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/save_state.rs)-adjacent).
3. **[`crates/coco-egui/src/app/frame.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/frame.rs)** — `fields_due`, `step_emulation`,
   `draw_display`, `window_ui`, in full. This is the file to reread when
   anything about timing or the display feels wrong later in the course.
4. **[`crates/coco-egui/src/app/input.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/app/input.rs)** and **[`keymap.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/keymap.rs)** — every
   function in both files is short; read them all, not just the excerpts
   above.
5. **[`crates/coco-egui/src/manager.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager.rs)**, **[`manager/lifecycle.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/lifecycle.rs)**, and
   **[`manager/vm_windows.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/vm_windows.rs)** — the module doc comment at the top of
   [`manager.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager.rs) first, then the three files in that order.
6. **[`crates/coco-egui/src/media/disk.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/media/disk.rs)** in full — then skim
   [`media/tape.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/media/tape.rs) and note everywhere it *differs* from the disk pattern.
7. **[`crates/coco-egui/src/ui_tests.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/ui_tests.rs)**'s module doc, then
   **[`crates/coco-egui/src/ui_tests/harness.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/ui_tests/harness.rs)** in full, then
   **[`crates/coco-egui/src/ui_tests/manager_window.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/ui_tests/manager_window.rs)** — read every test
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

**15.3 — Sabotage the thumbnail keep-previous heuristic (sabotage,
verify by running the suite).** Open [`crates/coco-egui/src/manager.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager.rs) and
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
View menu ([`chrome/menu_bar.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/chrome/menu_bar.rs)'s `view_menu_ui`) that, when checked, runs
the emulator at double real-time speed — a CoCo BASIC program that normally
takes 10 seconds should take about 5. Sketch the field where the toggle's
boolean state should live (which struct — `CocoApp`, and why not somewhere
in `Machine`, tying back to Chapter 1's core/frontend split), and exactly
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
([`crates/coco-egui/src/ui_tests/manager_window.rs:332-373`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/ui_tests/manager_window.rs#L332-L373)) end to end and
write down, in order: (a) what `harness.state().entries[0].slug` equals
immediately after `name_field().focus()` and typing `" Two"` but *before*
`harness.key_press(egui::Key::Enter)`; (b) why the test calls
`harness.step()` **three** times after the Enter key press, when most of
this chapter's helpers only ever call it once or twice in a row — tie your
answer to `ManagerApp::apply_pending_renames`'s doc comment
([`manager/lifecycle.rs:268-278`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/manager/lifecycle.rs#L268-L278)) and the `rename_pending` field it
consumes. Then run `cargo test -p coco-egui ui_tests::manager_window` (this
one needs no ROM) and confirm your prediction against the passing test.

**15.6 — Read: the disk write-back chain, end to end (read).** Trace, by
reading source only (no running), the full path a modified floppy takes
from a BASIC `SAVE"PROG"` inside the emulator to bytes landing back on the
host filesystem: which `coco-core` type first notices the disk is dirty
(Chapter 13), which [`media/disk.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/media/disk.rs) function is the *only* place that checks
`.dirty()` before touching the filesystem, and which call sites in
`CocoApp`/`ManagerApp` eventually reach that function — among them an
eject, a controller swap, and two distinct application exits. Then
explain in one sentence why VHD images ([`media/disk.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-egui/src/media/disk.rs)'s
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

Chapter 16 closes the course inside two directories this chapter deliberately
walked past without opening: `crates/coco-egui/src/debugger/` and
`crates/coco-egui/src/save_state/`. You already know their frontend
scaffolding without knowing it — `windows_ui`'s `self.debugger.windows_ui(...)`
call, `step_emulation`'s `self.debugger.run_field(&mut self.machine)`
routing every field through a breakpoint check, the quick-save/quick-load
shortcuts wired up in `consume_app_shortcuts`. Next week opens all three of
those up: the debugger core's breakpoint/watchpoint tables and the
side-effect-free `peek()` that keeps *looking* at memory from corrupting
the machine (the twin of `Bus::read`'s deliberate `&mut self` from Chapter 1,
finally paying off from the other direction), and the save-state format
that Chapter 1's ownership discipline — no `Rc<RefCell<...>>` anywhere in
the state tree — bought for nearly free the moment `#[derive(Serialize,
Deserialize)]` landed on `Machine`. It is, deliberately, the last chapter:
by then you will have watched nearly every architectural decision this
course made in Chapter 1 cash out, one at a time, for sixteen weeks.
