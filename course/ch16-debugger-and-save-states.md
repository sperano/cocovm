# Chapter 16 — The Debugger and Save States: the Payoff of Every Earlier Decision

*Week 16, the last week. Goal: watch fifteen weeks of architecture decisions
turn into two features you can actually click on — the debugger panel and
the save-state slots — and understand exactly which decision paid for which
feature, and what each one cost to make. Then a real retrospective: not
"what's next" (there is no next chapter), but an honest accounting of the
whole course's design.*

---

## 16.1 Nothing here is new machinery

Every other chapter in this course introduced a subsystem: a chip, a
protocol, a rendering path. This one introduces almost no new *mechanism*.
The debugger is a `for` loop around `Machine::step_instruction` — the exact
function week 6 built to make `run_field` resumable. Save states are one
`#[derive(Serialize, Deserialize)]` on a struct that was designed, in week 1,
specifically so that derive would work. If chapter 1's `Machine` had been
built the "obvious" way — `Rc<RefCell<...>>` everywhere, a raw C-style
back-pointer graph — this chapter would be the hardest one in the book,
requiring a bespoke serialization format and a hand-written debugger hook
threaded through every device. Instead it is closer to a victory lap: the
constraints already paid for it.

That is also why this chapter reads differently from the other fifteen. Most
chapters built something. This one mostly *reads receipts* — it opens files
you have already half-seen in passing footnotes ("we'll get to this in week
16") and shows you the bill has already been settled. Four promises made
early in the course come due here, and you should be able to name all four
before reading the sections that redeem them:

1. **Chapter 1** promised a `peek()` twin to `Bus::read` so the debugger
   couldn't corrupt the machine by looking at it, and promised that banning
   `Rc<RefCell<...>>` from the machine's state tree would make the whole
   thing `#[derive(Serialize)]`-able.
2. **Chapter 3** promised the disassembler — built to mirror the executor
   byte-for-byte, illegal opcodes rendered as `???` with correct length —
   would resurface as "the first piece of the debugger we'll meet again in
   week 16."
3. **Chapter 4** built the 1024-entry trace ring and the trace-diff workflow
   as *the* CPU-testing strategy for a chip with no reference test suite, and
   promised it would live inside the real debugger eventually.
4. **Chapter 6**, deriving `step_instruction`'s resumability, found — by
   actually running code, not just reading it — a real, documented gap: a
   breakpoint on an interrupt vector's target address can never fire,
   because interrupt entry and the handler's first instruction execute
   inside one atomic call. That finding was left as "week 16 will ask you to
   make a decision about this." This chapter honors it rather than quietly
   contradicting it: the shipped debugger does not fix the gap. It documents
   it, the same way the code already does.

Everything below traces exactly how each promise was kept, with the code
that keeps it.

---

## 16.2 The debugger core: `coco-core/src/debug.rs`

Open the module doc first, because it states the whole design in four
sentences:

```rust
//! Debug core: the [`Debugger`] the frontend owns and drives, plus the
//! side-effect-free primitives the machine exposes for it (`docs/plan-debugger.md`
//! §2). The [`Debugger`] holds PC breakpoints and memory watchpoints, runs the
//! machine one instruction at a time via [`Machine::step_instruction`] until a
//! stop condition trips ([`Debugger::run_until`]), and keeps an instruction
//! trace ring for "how did I get here" / MAME trace-diffing.
```

Notice what is *not* here: no second CPU loop, no "debug build" of the
machine, no shadow interpreter. `Debugger` is a plain struct the frontend
owns (`crates/coco-egui/src/debugger.rs` holds one), separate from
`Machine` entirely — debug state doesn't live in the thing you'd serialize
into a save state, which matters later in this chapter. Three pieces of
state live in it:

```rust
pub struct Debugger {
    breakpoints: HashMap<u16, Breakpoint>,
    watchpoints: HashMap<u16, Watchpoint>,
    trace: VecDeque<TraceEntry>,
    trace_cap: usize,
    pub trace_enabled: bool,
}
```

### Breakpoints

A `Breakpoint` is almost nothing — `enabled: bool` plus a `hits: u64`
counter, with room left for the deferred conditional-breakpoint feature
(the module doc says so explicitly: "conditional breakpoints hang off
`Breakpoint`"). The interesting part isn't the data, it's where the check
happens — inside the run loop, which we'll read in full in §16.4.

### Watchpoints, and the zero-cost-when-idle pattern

Watchpoints are more interesting, because their design teaches a pattern
worth internalizing beyond this codebase: **keep the debugger's book-keeping
entirely out of the hot path except during the instant it's actually
needed.**

The `Debugger` stores full `Watchpoint` records — `read`/`write` flags, an
`enabled` flag, a `hits` counter:

```rust
pub struct Watchpoint {
    pub read: bool,
    pub write: bool,
    pub enabled: bool,
    pub hits: u64,
}
```

But `SystemBus::read`/`write` — the hottest path in the entire emulator,
touched several times per instruction — never look at a `HashMap<u16,
Watchpoint>`. They look at this instead:

```rust
#[derive(Clone, Default)]
pub struct WatchTable {
    entries: HashMap<u16, WatchDirs>,
}

#[derive(Clone, Copy, Default)]
struct WatchDirs {
    read: bool,
    write: bool,
}
```

`WatchTable` is a *lean, enabled-only* snapshot — no `hits` counter (nothing
increments it on the bus's hot path), no disabled entries (they're filtered
out before the table is built). `Debugger::watch_table` builds it fresh from
the real `watchpoints` map every time a run starts:

```rust
fn watch_table(&self) -> WatchTable {
    let mut table = WatchTable::default();
    for (&addr, wp) in &self.watchpoints {
        if wp.enabled && (wp.read || wp.write) {
            table.watch(addr, wp.read, wp.write);
        }
    }
    table
}
```

And `Debugger::run_until` installs it into the bus only for the duration of
the run, uninstalling on every exit path:

```rust
pub fn run_until(&mut self, m: &mut Machine, max_instructions: u64) -> StopReason {
    m.bus.install_watches(self.watch_table());
    let reason = self.run_loop(m, max_instructions);
    m.bus.uninstall_watches();
    reason
}
```

Now look at what `install_watches` actually does, in `bus.rs`:

```rust
pub fn install_watches(&mut self, table: crate::debug::WatchTable) {
    self.watch = (!table.is_empty()).then_some(table);
    self.watch_hit = None;
}
```

An empty table — no watchpoints set, or a plain (non-debugger) run —
installs `None`, not `Some(empty_table)`. And `SystemBus::read`/`write`
check exactly that:

```rust
fn read(&mut self, addr: u16) -> u8 {
    // Debugger watch hook: a single null-check when no watchpoints are
    // installed (the common case), so the hot path is unchanged.
    if self.watch.is_some() {
        self.note_watch(addr, crate::debug::WatchKind::Read);
    }
    // ... the real decode
}
```

One `Option::is_some()` check, on a field that's `None` unless a debugger
run with actual watchpoints is in flight. `Machine::run_field` — the plain,
undebugged 262-times-a-second path every frame of normal emulation runs
through — never installs anything, never pays even that one branch's worth
of attention to watchpoints. The design gives you the full generality of
per-access watchpoints, compiled into the leanest table that can express
them, and it disappears completely — not "cheaply," completely — the moment
you close the debugger window.

> **Rust corner: `bool::then_some` for lazy `Option` construction.**
> `(!table.is_empty()).then_some(table)` reads almost like English once you
> know the method: "if this bool is true, `Some(value)`; otherwise `None`" —
> and unlike `if cond { Some(x) } else { None }`, it's an expression, so it
> slots straight into a field assignment with no intermediate `let`. `Option`
> has a whole family of these combinators (`.then(|| ...)` for a lazily
> computed value, seen a few lines later in `Debugger::run_loop`'s
> `self.trace_enabled.then(|| TraceEntry::capture(&m.cpu))` — the capture
> only happens when tracing is on, not "compute it, then discard it"). When
> you see `.then`/`.then_some` in this codebase, read it as "build this
> `Option` without a branch statement," and notice the eagerness difference:
> `then_some`'s argument is evaluated whether or not the condition holds
> (it's already a value here — `table` — so that's fine); `then`'s closure
> is only called when the condition holds, which is why the trace-capture
> call — genuinely work you want to skip — uses the closure form.

Breakpoints don't need this trick — a `HashMap<u16, Breakpoint>` lookup by
PC is already cheap and only happens once per instruction inside
`run_until`'s own loop, which is *already* debugger-only code, unlike
`read`/`write`, which run whether or not anyone opened the debugger. The
watchpoint design exists because the alternative — checking a `HashMap` on
every single bus access, debugger or not — would have taxed the 99% case
(no debugger open) to serve the 1% case (watchpoints set). That's the whole
lesson: **the cost of an optional feature should scale with whether it's in
use, not with whether it's theoretically available.**

### The trace ring, home at last

Chapter 4 built `TraceEntry` and a text format matching `examples/trace.rs`
specifically for trace-diffing against MAME/XRoar when the CPU had no bugs
to blame and no reference test suite to check against. It now lives here,
unchanged in spirit:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceEntry {
    pub pc: u16,
    pub a: u8,
    pub b: u8,
    pub x: u16,
    pub y: u16,
    pub u: u16,
    pub s: u16,
    pub dp: u8,
    pub cc: u8,
}
```

with a `format()` that matches `examples/trace.rs`'s `log_state` exactly —
verified by a test, `trace_entry_format_is_exact`, that hand-builds a CPU
state and checks the output string byte for byte:

```rust
let entry = TraceEntry::capture(&cpu);
assert_eq!(
    entry.format(),
    "8C1B:  A=12 B=34 X=5678 Y=9ABC U=DEF0 S=1357 DP=24 CC=68"
);
```

That exactness matters for a reason specific to trace-diffing: `diff -u`
against a MAME log is only useful if a *real* divergence produces the first
differing line, not a cosmetic formatting mismatch burying it in noise.

The ring itself is a fixed-capacity `VecDeque<TraceEntry>` (default
capacity `DEFAULT_TRACE_CAP = 1024`, overridable via
`Debugger::with_trace_capacity`):

```rust
fn push_trace(&mut self, entry: TraceEntry) {
    if self.trace.len() == self.trace_cap {
        self.trace.pop_front();
    }
    self.trace.push_back(entry);
}
```

Recording only happens when `trace_enabled` is set (default `false` — like
watchpoints, an opt-in cost: "a plain 'run' doesn't want" a snapshot taken
every single instruction), and `export_trace()` turns the whole ring into
text in the exact same format for pasting into a `diff` against a reference
trace — the same workflow chapter 4 built, now reachable from inside a
running session instead of only from a standalone `examples/trace.rs`
binary.

> **Rust corner: `VecDeque` as a bounded ring buffer.** A `Vec` can act as a
> stack (`push`/`pop` at the end) but shifting every element to drop from
> the *front* is O(n) — exactly what a "keep only the last N" ring buffer
> does constantly. `VecDeque` is a double-ended queue backed by a growable
> ring buffer internally, so `pop_front`/`push_back` are both O(1)
> amortized. `push_trace`'s pattern — check `len() == cap`, `pop_front` if
> so, then `push_back` unconditionally — is the idiomatic bounded-ring-buffer
> shape in Rust: no manual index arithmetic, no `unsafe`, and iteration
> (`self.trace.iter()`, what `Debugger::trace()` exposes) walks oldest-first
> for free because that's the queue's natural order.

---

## 16.3 `run_until`: the whole run loop, and the ch06 quirk it inherits

Here is the entire run loop, `debug.rs:371-407`, with nothing trimmed:

```rust
fn run_loop(&mut self, m: &mut Machine, max_instructions: u64) -> StopReason {
    for i in 0..max_instructions {
        let pc = m.cpu.pc;
        // Breakpoint check, skipped on the first iteration so a run can
        // resume off a breakpoint it is currently parked on rather than
        // re-triggering it immediately.
        let stop_at_bp = i > 0 && matches!(self.breakpoints.get(&pc), Some(bp) if bp.enabled);
        if stop_at_bp {
            let bp = self.breakpoints.get_mut(&pc).expect("just matched above");
            bp.hits += 1;
            return StopReason::Breakpoint(pc);
        }

        m.bus.clear_watch_hit();
        let snapshot = self.trace_enabled.then(|| TraceEntry::capture(&m.cpu));
        let event = m.step_instruction();
        if let (true, Some(entry)) =
            (matches!(event.kind, StepKind::Instruction { .. }), snapshot)
        {
            self.push_trace(entry);
        }

        if let Some(hit) = m.bus.take_watch_hit() {
            if let Some(wp) = self.watchpoints.get_mut(&hit.addr) {
                wp.hits += 1;
            }
            return StopReason::Watchpoint {
                addr: hit.addr,
                kind: hit.kind,
            };
        }
        if event.field_complete {
            return StopReason::FieldComplete;
        }
    }
    StopReason::Step
}
```

Four `StopReason` variants, checked in this exact order every iteration:

```rust
pub enum StopReason {
    Breakpoint(u16),
    Watchpoint { addr: u16, kind: WatchKind },
    FieldComplete,
    Step,
}
```

`FieldComplete` sitting next to `Breakpoint`/`Watchpoint` is the payoff of
chapter 6's `StepEvent { kind, field_complete }` shape: the debugger's run
loop can stop *exactly* where `run_field` would have stopped anyway, because
both are reading the same flag off the same `StepEvent`. There is no
separate "debugger scanline loop" — `run_until` is `for i in 0..max { ...
m.step_instruction() ... }` with three early-exit checks wrapped around the
one call chapter 6 spent an entire chapter making safe to call once, inspect,
and call again forever.

### The first-iteration subtlety

`i > 0 &&` in the breakpoint check is small and easy to skim past, and it
solves a real problem: without it, a debugger sitting parked exactly on a
breakpoint (which is where every debugger is, immediately after that
breakpoint fires) could never leave — the very next call to `run_until`
would see `pc == breakpoint_address` on iteration 0 and stop immediately,
without ever executing an instruction. The test that pins this down:

```rust
#[test]
fn run_until_resumes_off_parked_breakpoint() {
    let mut m = boot_machine();
    let mut dbg = Debugger::new();
    let parked = m.cpu.pc;
    dbg.add_breakpoint(parked);
    let reason = dbg.run_until(&mut m, 50);
    assert_ne!(reason, StopReason::Breakpoint(parked));
    assert_ne!(m.cpu.pc, parked);
}
```

The trade-off: a breakpoint set on the *current* PC when you resume is
guaranteed skipped for that one call. If the same address is reached again
later — a loop, a recursive call — it fires normally, because by then `i`
has had a chance to be `0` on a *different* pass through the address. This
is standard debugger behavior (GDB does the same thing for the same reason)
but it's worth knowing you're reading a deliberate, tested design decision
and not an oversight.

### The interrupt-vector breakpoint gap, honored

Chapter 6 found this by actually running a program, not by reading code —
worth restating here because this chapter is where the finding was always
headed. `step_cpu_unit` (`machine/run.rs`, quoted in full below) redirects
`cpu.pc` to an interrupt vector's target *and* executes that handler's first
instruction inside a single call:

```rust
fn step_cpu_unit(&mut self) -> (u32, bool) {
    let (cycles, was_instruction) = if self.bus.halt_asserted() {
        self.prev_halted = true;
        (1, false)
    } else {
        if !self.prev_halted {
            self.bus.poll_cart_interrupt();
            if self.bus.take_nmi() {
                self.cpu.nmi(&mut self.bus);
            }
            self.service_interrupts();
        }
        self.prev_halted = false;
        (self.cpu.step(&mut self.bus), true)
    };
    // ... peripheral ticks
    (cycles, was_instruction)
}
```

`self.service_interrupts()` (calling `cpu.firq`/`cpu.irq`, which push the
stack frame and set `cpu.pc` to the vector target) runs, and then — same
call, same `step_cpu_unit`, same `step_instruction` — `self.cpu.step(&mut
self.bus)` executes the handler's very first instruction. `run_loop` samples
`pc = m.cpu.pc` *before* calling `step_instruction`, and that's the only
moment it ever looks at the PC. The CPU's program counter genuinely does
equal the vector address for an instant, mid-call — but that instant is
never externally observable as "the value of `m.cpu.pc` right before a
`step_instruction` call," which is the only thing a PC breakpoint can check.
**A breakpoint on an interrupt vector's target address cannot fire**, full
stop, through this mechanism. Setting it one instruction later — the
handler's *second* instruction — works normally, because by then a full
`step_instruction` call has elapsed with the new PC sampled at its start.

The debug module's own doc comment on `run_until` states this as a fact
about the system, not a bug ticket:

```rust
/// Note on interrupt timing: [`Machine::step_instruction`] services a
/// pending NMI/FIRQ/IRQ at the *start* of the step (matching the hardware's
/// end-of-instruction recognition). A PC breakpoint therefore fires when
/// the PC *reaches* the address, before any interrupt that would preempt
/// that instruction is taken — the breakpoint wins.
```

This chapter's code does not fix it. That's a real, intentional choice, and
it's worth stating why rather than leaving it implicit: fixing it means
checking breakpoints against the *new* PC immediately after
`service_interrupts` redirects it — a second breakpoint check inside
`step_cpu_unit` itself, which currently knows nothing about breakpoints at
all (that's `Debugger`'s job, one layer up). Threading debugger state down
into the machine's innermost per-cycle loop is exactly the kind of coupling
chapter 1's whole "debug state lives in `Debugger`, not `Machine`"
separation was designed to avoid — `Machine` has no idea a debugger exists,
which is also why it's the thing you serialize (§16.6) and the debugger
isn't. The gap is a real limitation of a real design trade-off, not an
oversight, and now you know exactly which line to change if you ever need
to close it, and exactly what it would cost the rest of the design to do so.

The same mechanism produces a second, quieter surprise in the trace ring:
the `TraceEntry` recorded for the step that services an interrupt captures
`pc` *as sampled before that call* — `snapshot = self.trace_enabled.then(||
TraceEntry::capture(&m.cpu))` runs before `m.step_instruction()`, matching
`run_loop`'s own ordering exactly. One trace line therefore logs the
interrupted code's address, not the handler's — the trace ring is an honest
record of "what PC was about to execute when I asked," not "what
instruction actually retired this step," a distinction invisible for every
ordinary instruction (the two coincide) and visible for exactly the one
step per interrupt where they don't. If you ever trace-diff this emulator's
output against MAME across an interrupt boundary and see one line that
looks "off by one instruction," this is why — and now the explanation is
on the record in two chapters, not just one, so it stops looking like a bug
every time someone rediscovers it.

---

## 16.4 `peek()`: reading the machine without touching it

Chapter 1 flagged the problem the moment it introduced `Bus::read(&mut
self)`: reads have side effects on real hardware — a PIA data-register read
clears that port's interrupt flag; the GIME's `$FF92`/`$FF93` status
registers clear-on-read. A debugger that used `read()` to paint its memory
view would be *lying about the state it just showed you and also changing
that state while showing it to you* — the classic observer-effect bug,
except worse than the physics metaphor, because on a real oscilloscope
probing a PIA pin doesn't clear anything; on this bus, `read()` doing exactly
that is *correct emulation*, which is precisely why the debugger cannot be
allowed to call it.

### The corruption scenario, made concrete

Here's what "the debugger corrupts the machine" looks like as an actual
sequence of events, not an abstraction. Stock Color BASIC idles on the PIA
path (chapter 6): the 60 Hz vertical-sync interrupt sets PIA0's CB1 flag,
BASIC's IRQ handler reads PIA0's port B data register at `$FF02` specifically
*to acknowledge that interrupt* — the read's side effect (clearing
`cr::C1_FLAG`) is the acknowledgment; there is no separate "ack" register.
That's what breaks BASIC out of its `BRA *` idle loop 60 times a second, the
whole heartbeat this course has been building toward since week 6.

Now imagine the memory panel used `SystemBus::read` to paint its hex dump,
and the user happens to be hovering the mouse over (or the panel happens to
be scrolled to) the row containing `$FF00–$FF03` — PIA0's four registers —
at the exact moment a repaint fires, which on a debugger redrawing every
frame is *constantly*. Every repaint would call `read(0xFF02)`, and every
one of those calls would clear the just-latched CB1 flag before the ROM's
own interrupt handler ever got a chance to read it and acknowledge it
itself. From the emulated machine's point of view, the interrupt simply
never happened — the flag was gone before the handler that was supposed to
consume it ran. Depending on timing, either the ROM's `IRQ` handler runs
anyway (dispatched on the *shared* IRQ line, which the GIME also drives) and
finds nothing to acknowledge, or — worse, and this is the real failure mode
— a genuinely pending 60 Hz field-sync interrupt gets silently eaten by the
debugger's own repaint before BASIC's handler observes it at all. BASIC's
clock (`TIMER`, and every routine that polls it) stops advancing, or stutters
unpredictably, purely because a *read-only-looking* memory viewer was open.
That is not a hypothetical: it is the literal mechanism `Bus::read(&mut
self)`'s side effects create, applied to the literal address the ROM's own
clock interrupt depends on, and it is exactly the bug `peek()` exists to make
structurally impossible rather than merely "unlikely if the UI code is
careful."

### The twin contract

`bus/peek.rs` states the discipline in its module doc:

```rust
//! `peek` mirrors `Bus::read`'s address decode exactly but takes `&self` and
//! never mutates: no PIA Cx1/Cx2 flag clears, no GIME IRQ/FIRQ status ack, no
//! cartridge register side effects, and no watchpoint hook. Devices whose real
//! read mutates return a last-latched value (GIME status registers, via their
//! public `*_pending` fields) or open bus (most cartridge I/O).
```

Three things to notice in that sentence, in order of importance:

1. **`&self`, not `&mut self`.** This is enforced by the type system, not by
   convention — a `peek` implementation *cannot* mutate `self` even by
   accident, because the borrow checker won't let it write to any field.
   Compare that to `read(&mut self)`, which *could* mutate anything and the
   type system offers no protection against a future edit accidentally
   adding a stray side effect to one code path but not its `peek` twin — the
   discipline there is "mirror the decode exactly," enforced by review and
   by the `peek_matches_read_for_ram_and_rom` test, not by the compiler.
2. **"No watchpoint hook."** `peek` doesn't call `note_watch` — which is
   correct: the debugger's own memory-view repaints are not CPU accesses,
   and a watchpoint exists to catch what the *emulated program* does to an
   address, not what the debugger UI happens to look at.
3. **"Devices whose real read mutates return a last-latched value... or open
   bus."** `peek` isn't a no-op stand-in for hardware that has no honest
   answer to "what's here without side effects" — real chips don't have
   that concept either. It's a *best available* read: the GIME's IRQ status
   register peek returns `self.gime.irq_pending` directly (the field a real
   read would both return *and* clear) instead of performing the clear; most
   cartridge I/O, which genuinely has no side-effect-free read on real
   silicon, answers open bus, same as an unmapped read would.

The full `io_peek` match arm for the GIME status registers makes point 3
concrete:

```rust
INIT0_REG => self.gime.init0,
INIT1_REG => self.gime.init1,
// Read would clear these (status ack); peek reports them intact.
IRQENR_REG => self.gime.irq_pending,
FIRQENR_REG => self.gime.firq_pending,
TIMER_MSB_REG..=GIME_LAST => 0,
```

and the PIA case delegates to `MC6821::peek`, which the comment explains
precisely:

```rust
IO_BASE..=PIA0_LAST => {
    // A real read refreshes only port A's input pins; port B keeps
    // its latched `input` (see `io_read`), so peek does the same.
    self.pia0
        .peek((addr & 0x03) as u8, self.pia0_pa_pins(), self.pia0.b.input)
}
```

`peek` doesn't invent a different answer from `read` for the *data* — it
reproduces `read`'s exact data path (refresh port A's live pins the same
way) while skipping only the *flag-clearing* half of what a real register
read does. The debugger sees the same byte the CPU would see; it just
doesn't get credited with having asked.

Two tests pin the whole contract down. First, that `peek` genuinely never
mutates a PIA flag while `read` genuinely does:

```rust
#[test]
fn peek_does_not_clear_pia_flags() {
    let mut m = boot_machine();
    m.bus.pia0.write(1, cr::DDR_ACCESS);
    m.bus.pia0.a.set_c1(false); // high->low sets C1_FLAG
    assert_ne!(m.bus.pia0.a.control & cr::C1_FLAG, 0, "flag should be set");

    let _ = m.bus.peek(0xFF00);
    let _ = m.bus.peek(0xFF00);
    assert_ne!(
        m.bus.pia0.a.control & cr::C1_FLAG,
        0,
        "peek must not clear the flag"
    );

    let _ = m.bus.read(0xFF00);
    assert_eq!(
        m.bus.pia0.a.control & cr::C1_FLAG,
        0,
        "read must clear the flag"
    );
}
```

Second, that for the *pure* regions — RAM and internal ROM, where there is
genuinely no side effect to preserve — `peek` and `read` agree byte for
byte:

```rust
#[test]
fn peek_matches_read_for_ram_and_rom() {
    let mut m = boot_machine();
    for addr in [0x8000u16, 0x8C1B, 0xA000, 0xFFFE] {
        assert_eq!(m.bus.peek(addr), m.bus.read(addr), "ROM peek/read at {addr:04X}");
    }
    m.bus.write(0x1234, 0xAB);
    assert_eq!(m.bus.peek(0x1234), 0xAB);
    assert_eq!(m.bus.peek(0x1234), m.bus.read(0x1234));
}
```

### The discipline: every debugger view goes through `peek`

`coco-egui/src/debugger.rs`'s module doc states this as a hard rule, not a
suggestion:

```rust
//! Every read view (disassembly, memory, stack) goes through
//! [`coco_core::SystemBus::peek`] — never `read` — so simply having the
//! debugger open can never perturb PIA/GIME/cart state
```

Check the code and the rule holds everywhere reads happen: the disassembly
panel feeds `disassemble(&mut |a| machine.bus.peek(a), addr)` — the closure
handed to the disassembler (chapter 3's table-driven decoder, byte-for-byte
faithful to the executor) reads through `peek`, never `read`. The stack
panel's return-address annotation does the same: `machine.bus.peek(addr)`
for both the stack slot itself and the speculative disassembly of whatever
16-bit value sits there. The memory panel's logical view calls
`machine.bus.peek(addr)` for every cell it paints.

**Writes are different, and deliberately so.** The memory panel's editor
doesn't call some hypothetical side-effect-free "poke" — it calls
`machine.poke(addr, val)`, and `Machine::poke` is exactly one line:

```rust
pub fn poke(&mut self, addr: u16, val: u8) {
    self.bus.write(addr, val);
}
```

Real `write`, real side effects. The doc comment above it explains why this
asymmetry is correct, not an oversight: "Real hardware has no side-effect-free
write... a debugger editing memory is expected to trip the same PIA/GIME
register semantics a running program's own store would." There is no analog
to a "peek" for writes because a write's entire *point* is to change state —
poking `$FF22` to flip a VDG mode bit should behave exactly like a program
doing the same `STA $FF22`, because from the hardware's perspective it's
indistinguishable. `peek` exists to answer "what does this address hold
right now" without changing the answer to that question by asking it;
writes don't have — and shouldn't have — an equivalent notion.

---

## 16.5 The debugger UI, panel by panel

`crates/coco-egui/src/debugger.rs` and its six submodules
(`controls.rs`, `registers.rs`, `disasm.rs`, `memory.rs`, `stack.rs`,
`hardware.rs`) are the part of this codebase you'd actually click on. The
module doc frames the whole thing as one discipline applied consistently:

```rust
//! [`DebuggerPanel`] owns the `coco_core::debug::Debugger` (breakpoints,
//! watchpoints, trace ring) and is the single entry point `CocoApp::update`
//! drives the per-field run loop through ([`DebuggerPanel::run_field`]) so a
//! tripped breakpoint or watchpoint pauses the emulator the same way the
//! Run/Pause button does, rather than needing a second "why did we stop"
//! flag.
```

"The same way the Run/Pause button does, rather than needing a second 'why
did we stop' flag" is worth sitting with: `DebuggerPanel::run_field` returns
a plain `bool` — `true` means "field completed normally, keep the app's
existing `running` state as-is," `false` means "stop, as if Pause were
clicked":

```rust
pub fn run_field(&mut self, machine: &mut Machine) -> bool {
    for _ in 0..MAX_CHAINED_RUNS {
        match self.core.run_until(machine, RUN_BUDGET) {
            StopReason::FieldComplete => return true,
            StopReason::Step => continue,
            StopReason::Breakpoint(_) | StopReason::Watchpoint { .. } => return false,
        }
    }
    true
}
```

`RUN_BUDGET` (200,000 instructions) is comfortably above one field's real
instruction count, so an ordinary run with nothing set always exits via
`FieldComplete` well inside that budget; `StopReason::Step` — the budget
exhausted with no field boundary and no trip — is the rare fallback that
keeps looping within the same field rather than a case the caller has to
special-case. The comment on this exact function documents a bug this
design caught during development: conflating `FieldComplete` with "keep
looping" (the way `run_until_stop`, its Step-Over/Run-to-Cursor sibling,
correctly *does* want to keep going past field boundaries) silently ran
extra fields per call — caught by a test named exactly for what it
guarantees, `run_field_with_no_breakpoints_matches_plain_run_field`.

### Controls: Run, the four step commands, breakpoints

`controls.rs` is the smallest panel and the one that drives everything else.
Four step primitives, each deliberately different from `run_until`'s
breakpoint/watchpoint machinery for its own documented reason:

- **Step In** — exactly one retired instruction, skipping (not stopping on)
  burned HALT* cycles, because a click should always visibly advance CPU
  state. It bypasses `Debugger::run_until` entirely and calls
  `machine.step_instruction()` directly in a loop, because a single
  deterministic step under full user control has no need for breakpoint
  bookkeeping.
- **Step Over** — disassembles the current instruction (through `peek`, of
  course) and, for `JSR`/`BSR`/`LBSR`, temp-breakpoints the return address
  and chains `run_until` calls until it's hit; every other opcode falls
  through to Step In.
- **Step Out** — runs raw `step_instruction` calls until `cpu.s` rises past
  its value at the start of the call, i.e. until the enclosing subroutine's
  `RTS` has popped the return address. The doc comment is honest about the
  limitation this creates: because it doesn't chain through
  `Debugger::run_until`, it "does not stop early for a breakpoint/watchpoint
  hit inside the callee" — the S-rise condition isn't something the
  Debugger core's stop-condition vocabulary can express, so this one path
  quietly trades completeness for a check the run loop has no way to encode.
- **Step Scanline** — runs until `machine.current_scanline()` changes or the
  field wraps, giving you the "one raster line at a time" granularity that's
  the natural unit for GIME split-screen debugging (chapter 9's mid-frame
  register changes).

**Run to Cursor** is the disassembly panel's payoff wired into Controls: click
a row in the disassembly view to set `cursor_addr`, then Run to Cursor
installs a temporary breakpoint there (unless a real one already exists, in
which case it's left alone afterward) and runs until it — or any other stop
condition — trips:

```rust
fn run_to(&mut self, machine: &mut Machine, target: u16) {
    let had_existing = self.core.breakpoint(target).is_some();
    if !had_existing {
        self.core.add_breakpoint(target);
    }
    self.run_until_stop(machine);
    if !had_existing {
        self.core.remove_breakpoint(target);
    }
}
```

Every one of the raw-stepping primitives is bounded — `MAX_RAW_STEPS =
2_000_000` for direct `step_instruction` loops, `MAX_CHAINED_RUNS = 1_000`
for chained `run_until` calls — so a callee that never returns, or a
Run-to-Cursor target that's never reached, gives the UI thread back control
rather than hanging it forever. The comment on `MAX_CHAINED_RUNS` puts a
number on what that cap actually buys: "~1000 fields is minutes of
emulated time, generous for any real call/run."

### Registers: editable, live, and paused-only

`registers_ui` draws A/B/D/X/Y/U/S/PC/DP plus the eight CC flags as
individually toggleable checkboxes (`E F H I N Z V C`, top to bottom
matching the bit layout chapter 2 introduced), directly against
`machine.cpu.*` fields — no `peek` needed here, because the CPU's own
registers have no side-effect-on-read concept at all; the debugger *is* the
CPU state, not a read of it through a bus. Every editable widget is wrapped
in `ui.add_enabled(editable, ...)` where `editable = !running` — you can drag
a register's hex value or flip a CC bit only while the machine is paused. The
discipline is uniform across every panel (memory cells, register fields):
*"editable only while paused; while running the panels still redraw every
frame from live `peek`s, same as the main screen"* — you can always watch,
you can only touch when nothing is racing your edit.

### Disassembly: the payoff of chapter 3, verbatim

The disassembly panel calls the exact same `disassemble` function from
`mc6809::disasm` that chapter 3 built to mirror the executor byte-for-byte
and render illegal opcodes as `???` with the correct byte length — the
property that makes a scrolling disassembly view never desynchronize from
the instruction stream even when it walks through data or an undocumented
opcode:

```rust
let insn = disassemble(&mut |a| machine.bus.peek(a), addr);
```

Each row shows a breakpoint-toggle checkbox in the gutter, a `>` marker and
yellow highlight on the current PC row, and click-to-select for
Run-to-Cursor. Advancing to the next row uses `insn.len.max(1)` — the `.max(1)`
guards against a zero-length decode ever stalling the scroll on a byte the
disassembler can't classify, so even a corrupted or deliberately-`???`
stream keeps scrolling forward one byte at a time rather than looping in
place.

### Memory: logical and physical, chapter 5's distinction made literal

This is where chapter 5's MMU work becomes something you toggle with a
button instead of computing by hand. `MemoryView` is a two-variant enum:

```rust
enum MemoryView {
    /// Through the live MMU, exactly as the running CPU sees it (`peek`).
    Logical,
    /// Raw installed RAM, bypassing the MMU — a bank/block picker selects
    /// where in the (up to 2048K) physical space the view starts.
    Physical,
}
```

The Logical path is just `machine.bus.peek(addr)` for sixteen columns times
sixteen rows starting from a navigable top-of-view address — the same
64K-address-space view a running 6809 program has, MMU translation and all,
so a breakpoint stopped mid-BASIC and a memory dump of `$0400` show you the
exact byte the interrupted program would have read next.

The Physical path bypasses `peek`/the MMU entirely and indexes
`machine.bus.ram` directly by byte offset, with a `Block:` `DragValue`
stepping through 8K blocks (`BLOCK_SIZE`, the exact granularity chapter 5's
GIME MMU translates in) up to `machine.bus.ram.len() / BLOCK_SIZE`. This is
the view that answers a question the Logical view *cannot* answer: "which
physical 8K block is MMU task 0 slot 2 actually pointing at, and what's
really stored there regardless of what the CPU currently sees through it?"
— exactly the question a 512K CoCo 3 program bank-switching through eight
8K windows into 64 physical blocks needs answered, and exactly the
distinction chapter 5 spent a whole chapter establishing (`phys = block<<13
| addr&0x1FFF`) precisely so a debugger could someday expose both sides of
it as separate, honest views rather than collapsing them into one address
space that lies about which one you're looking at.

Watched addresses in the Logical view get a highlighted background (an
amber `egui::Frame` fill) so a watchpoint you set is visually distinguishable
from an ordinary cell before it ever trips — a passive reminder of what's
being watched, not just a stop-condition list buried in a separate panel.
Editing a cell in either view calls through to a real write —
`machine.poke(addr, val)` for Logical, a direct `machine.bus.ram[idx] = val`
for Physical (since Physical view addresses have already left CPU-address
space entirely; there's no `peek`/`poke` contract to honor at a raw RAM
offset) — matching the "writes always have real side effects" rule from
§16.4.

### Stack: unwinding with a disassembly hint

`stack_ui` reads sixteen 16-bit slots starting at `S`, each through `peek`
(both bytes of every word), and next to each one runs the *disassembler*
against whatever value sits there — not because every stack slot holds a
return address (locals and saved registers don't), but because a genuine
return address, when you disassemble the code sitting at that address,
reads as recognizable, plausible instructions, while a local variable's
bit pattern almost never does. The doc comment calls this exactly what it
is: "a best-effort hint, not a claim." It costs nothing extra to compute
(the disassembler is already loaded and already side-effect-free through
`peek`) and it turns a raw hex dump of the stack into something you can
skim for "this looks like a call chain" without doing the arithmetic
yourself.

### Hardware: the panel you wished you had at twelve

`hardware_ui` is the one panel with no editable fields at all — the doc
comment is explicit that nothing here has "obvious edit semantics": this is
latched and derived hardware state, not CPU-visible memory. It decodes
`INIT0`/`INIT1` bit by bit (`COCO`, `MMUEN`, `IEN`, `FEN`, `MC3`–`MC0` for
INIT0; `TINS`, `TR` for INIT1), shows the IRQ/FIRQ enable and pending masks
alongside whether `IEN`/`FEN` are actually gating them, dumps the GIME timer
reload/count/fast-clock state, and prints all eight MMU task-slot blocks for
both tasks side by side — every register chapter 5 and chapter 8 spent pages
decoding from first principles, now read off the live machine in one glance.
Below that, both PIAs' A/B ports (output, DDR, control, input, and the raw
C1-flag bit) render in a shared table, and a final line shows the
cartridge-port line states — `HALT*`, whether the CART line ties to `Q`,
whether an NMI is pending — the exact three signals chapter 13's FD-502
handshake and chapter 6's `step_cpu_unit` HALT-before-interrupts ordering
depend on.

If you POKEd `$FF22` as a kid to flip into a graphics mode and squinted at
the screen to guess whether it worked, this panel is the tool you wished
existed: every bit of every register you were guessing at, decoded and
labeled, updating live, for free, the instant you pause.

---

## 16.6 Save states: the `.ccstate` container, byte by byte

`crates/coco-core/src/snapshot.rs` states the container format in one line
of ASCII art:

```text
magic "CCSTATE" (7 bytes) | container_version: u8 | schema: u32 LE | gzip(CBOR payload)
```

Twelve header bytes (`HEADER_LEN = CONTAINER_MAGIC.len() + 1 + 4 = 12`),
then a gzip stream. Reading `codec.rs::save` top to bottom builds exactly
that:

```rust
pub fn save(machine: &Machine, media: &MediaRefs) -> Result<Vec<u8>, SnapshotError> {
    if machine.bus.cart.contains_custom() {
        return Err(SnapshotError::CustomCartNotSnapshotable);
    }

    let payload = SnapshotPayloadRef { media, machine };
    let mut cbor = Vec::new();
    ciborium::into_writer(&payload, &mut cbor)...;

    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    std::io::Write::write_all(&mut gz, &cbor)...;
    let compressed = gz.finish()...;

    let mut out = Vec::with_capacity(HEADER_LEN + compressed.len());
    out.extend_from_slice(CONTAINER_MAGIC);
    out.push(CONTAINER_VERSION);
    out.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}
```

Two version numbers, deliberately independent, and the module doc is
explicit about why there are two:

- **`CONTAINER_VERSION`** — this module's own framing: magic, header layout,
  where the schema field sits, where the gzip body starts. Bumping it means
  "the *shape of the file itself* changed" — extremely rare, and every
  `.ccstate` ever written would need this exact byte checked to know how to
  even find the schema field.
- **`SCHEMA_VERSION`** — the *machine-tree* schema: what `SnapshotPayload`
  (and everything nested inside `Machine`) looks like. This one is meant to
  almost never move either — see §16.7 — because the whole point of the
  evolution rules is to absorb ordinary struct growth without a version
  bump at all. "Everything the four rules... can absorb should NOT bump it."

Both are checked in `load` *before* touching the (attacker-controlled) gzip
body, in this order:

```rust
pub fn load(bytes: &[u8]) -> Result<SnapshotPayload, SnapshotError> {
    let header = parse_header(bytes)?;
    if header.schema > SCHEMA_VERSION {
        return Err(SnapshotError::SchemaTooNew { found: header.schema, current: SCHEMA_VERSION });
    }
    let cbor = gunzip(header.body)?;
    if header.schema == SCHEMA_VERSION {
        return decode_payload(&cbor);
    }
    migrate(header.schema, &cbor).unwrap_or_else(|| {
        Err(SnapshotError::NoMigration { found: header.schema, current: SCHEMA_VERSION })
    })
}
```

Notice the ordering: a schema *newer* than this build understands is
rejected before a single byte of the gzip body is even inflated — "so a
crafted file claiming one never pays for (or risks) inflating its gzip body
at all," in the code's own words. `migrate` is a hook, currently empty
(`fn migrate(_old_schema: u32, _cbor: &[u8]) -> Option<...> { None }`) — the
table a future breaking change would populate, keyed by old schema number,
to decode the old shape and upgrade it. Since `SCHEMA_VERSION` has never
moved past `1`, this table has never needed an entry; it exists as the
documented place a real migration would go, not as evidence one has ever
been written.

### Why CBOR, not bincode/postcard

The module doc gives the real reason, and it's worth sitting with because it
determines whether the evolution rules in §16.7 are even possible to state:

> CBOR carries field names with the data, so serde's evolution tools
> (`#[serde(default)]`/`alias`) work across versions instead of every struct
> needing hand-rolled versioning.

A positional binary format (bincode, postcard) encodes a struct as "field 1's
bytes, then field 2's bytes, then field 3's bytes" with no names anywhere in
the wire format — decoding *requires* the reader's struct definition to have
the exact same fields in the exact same order as the writer's, or the bytes
land in the wrong fields silently. CBOR (when serde derives it the way this
codebase does — as a map) instead writes `{"a": ..., "b": ..., "c": ...}`
with real field-name keys. That single difference is what makes "add a field
with `#[serde(default)]`" a *decodable* operation at all: a decoder reading
an old file just never sees the new key in the map, so `#[serde(default)]`
supplies the value serde's derive macro would otherwise have nowhere to get
it from. Positional formats can't offer this without hand-written
versioning schemes bolted on top; CBOR gets it from serde's ordinary derive
machinery for free, which is the whole reason this format was chosen over
faster, smaller binary alternatives — the compatibility contract (§16.7) is
the entire point, and it's cheaper to buy with the right wire format than to
build by hand on top of the wrong one.

### The compression bomb guard

`gunzip` is worth reading in full, because it's a small, complete lesson in
defending against an adversarial length field:

```rust
const MAX_PAYLOAD_BYTES: u64 = 64 * 1024 * 1024;

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, SnapshotError> {
    let mut out = Vec::new();
    let mut limited = GzDecoder::new(bytes).take(MAX_PAYLOAD_BYTES);
    limited.read_to_end(&mut out)...;
    if out.len() as u64 == MAX_PAYLOAD_BYTES {
        let mut probe = [0u8; 1];
        let more = limited.into_inner().read(&mut probe)...;
        if more > 0 {
            return Err(SnapshotError::InvalidPayload(
                "payload exceeds MAX_PAYLOAD_BYTES".to_string(),
            ));
        }
    }
    Ok(out)
}
```

A gzip stream's own header carries an uncompressed-size hint, and that hint
is exactly as trustworthy as the rest of the file — which is to say, not at
all, because a hostile `.ccstate` is free to lie about it. A few kilobytes of
all-zero bytes compress to almost nothing and decompress to gigabytes — the
classic "decompression bomb." `Read::take(MAX_PAYLOAD_BYTES)` wraps the
decoder in an adapter that simply refuses to hand back more than the cap,
*regardless of what the stream claims about its own length* — the
allocation this code ever performs is bounded by `MAX_PAYLOAD_BYTES`
(64 MiB — chosen with headroom over "2 MB max RAM plus every other device's
state," per the constant's own comment) no matter what a hostile input's
gzip header says. The `if out.len() == MAX_PAYLOAD_BYTES` branch then
distinguishes two cases that look identical after the `take` — "a payload
that happened to be exactly the cap size" versus "a payload that's actually
bigger and got silently truncated" — by probing for one more byte past the
cap on the underlying stream: if there's more, this was genuinely oversized
and the error says so explicitly. `hostile_payload.rs`'s
`oversized_gzip_payload_is_rejected_without_allocating_it` test builds
exactly this attack — 70 MB of zeros, comfortably past the 64 MiB cap — and
verifies the error names `MAX_PAYLOAD_BYTES` in its message rather than
either succeeding or panicking with an out-of-memory abort.

> **Rust corner: `Read::take` as a resource-bounding adapter, not just a
> convenience.** `std::io::Read::take(self, limit: u64) -> Take<Self>`
> wraps any reader so it reports EOF after `limit` bytes, however many the
> underlying source actually has. It's usually introduced as a convenience
> for "read the first N bytes of a stream," but here it's doing real
> security work: the *only* thing standing between a crafted gzip header and
> an unbounded allocation is this adapter refusing to ask the decompressor
> for more than the cap, ever, regardless of what the compressed stream
> claims to contain. The pattern generalizes past gzip: any time you're
> decoding a length-prefixed or self-describing format from untrusted bytes,
> the length the format itself reports is exactly as trustworthy as the rest
> of the input — wrap the reader in a hard cap *before* trusting anything it
> says about its own size, the same way this function does.

---

## 16.7 Media: references, never bytes

Every media reference — a `MediaRef` — is a path plus a hash, nothing else:

```rust
pub struct MediaRef {
    pub path: PathBuf,
    pub sha256: String,
}
```

`MediaRefs` collects one of these (or a `Vec` of them, for multi-drive
devices) per media *kind* a snapshot might reference: the system ROM,
ROM-bearing cartridges (keyed by Multi-Pak slot), floppies, VHDs, DriveWire
images, tape. None of their actual bytes ever enter `SnapshotPayload`. Two
separate, equally load-bearing reasons, both stated in the module doc:

1. **Copyright.** Super Extended Color BASIC, Disk BASIC, any commercial
   cartridge ROM — these are copyrighted binaries this project doesn't have
   the right to redistribute, ever, in any form. A save state that embedded
   ROM bytes would turn every shared `.ccstate` file into an unlicensed copy
   of Tandy's firmware. Recording *where the frontend found it* and *what it
   hashed to* lets a snapshot be portable and shareable without ever
   containing the thing it can't legally contain.
2. **Size.** A `.dsk` floppy image, and especially a VHD hard-disk image,
   can dwarf the rest of the machine's state by orders of magnitude — "a
   `.dsk` is bigger than the machine state," as the syllabus puts it flatly.
   Embedding media bytes would turn a save state that should be a few
   kilobytes of gzipped CBOR into a multi-hundred-megabyte file for what's
   conceptually "I pressed save mid-game."

The hash is what makes this safe rather than merely convenient. `sha256_file`
streams the file in 8 KiB chunks (never reading a hundreds-of-MB VHD whole
into memory just to hash it) and `MediaRef::verify` checks a reference
against the filesystem right now:

```rust
pub fn verify(&self) -> MediaCheck {
    match sha256_file(&self.path) {
        Ok(actual) if actual == self.sha256 => MediaCheck::Ok,
        Ok(actual) => MediaCheck::Mismatch { actual },
        Err(_) => MediaCheck::Missing,
    }
}
```

If you save a state, then swap the floppy in drive 0 for a different `.dsk`
at the same path, then load that snapshot back — `verify` catches exactly
this. The frontend's `read_if_present` surfaces a `Mismatch` as a warning
("`floppy (...) doesn't match the hash recorded in this save state; loaded
anyway`") rather than a hard failure, because a swapped-but-present file is
recoverable information the user might have intended (a deliberately
different disk in the same drive slot), while a genuinely absent file — no
ROM, no floppy, nothing to load at all — becomes
`SnapshotError::MissingMedia`, collected across *every* missing reference at
once (not stopped at the first one found) so the user can be prompted for
every missing file in one pass rather than one frustrating retry at a time.

---

## 16.8 The four evolution rules, and what breaks without each one

The module doc states four rules governing every type inside
`SnapshotPayload`. Each one earns its place by describing exactly what fails
if you skip it — read them that way, not as arbitrary style guidance:

**Rule 1 — never remove or rename a field without `#[serde(alias = "old_name")]`
or a migration.** CBOR decodes by matching key names in the map against the
struct's field names. Delete a field outright, or rename it without an
alias, and every snapshot written before that change now has a map key the
current struct doesn't recognize — for a *removed* field this is silently
harmless (the extra key is just ignored), but for a *renamed* field it means
the old data for that logical field is invisible to the new struct, and — if
the field had no `#[serde(default)]` — decoding fails outright with a
missing-field error. `#[serde(alias = "old_name")]` tells the derive macro
"also accept this key as populating this field," which is exactly a rename
with backward compatibility built in.

**Rule 2 — every added field carries `#[serde(default = "...")]` reproducing
the *old* behavior.** Without this, loading a snapshot written before the
field existed hits a required key that's missing from the CBOR map, and
`ciborium::from_reader` returns a hard decode error — the *entire* snapshot
fails to load over one new `bool`. `#[serde(default)]` (or a named default
function for a non-`Default`-able type, or a non-zero sentinel) tells serde
"if this key is absent, use this value instead of failing" — and the rule's
insistence on "reproduces the old behaviour" matters precisely because the
wrong default is worse than a load failure: it loads *successfully* into
the wrong state, silently, with no error to catch it.

**Rule 3 — never change the meaning or units of an existing field; add a new
field and migrate instead.** This one has no compiler or serde mechanism
backing it at all — a `u32` field storing microseconds yesterday and
milliseconds today deserializes without complaint either way; the bytes on
disk are the same shape, just interpreted differently. An old snapshot
would load *successfully* and then behave subtly, catastrophically wrong —
a timer firing 1000× too fast or too slow, discovered only by whoever
happens to load an old file after the change ships. This rule exists purely
in review discipline (the module doc says so: "enforced in review, not by
the compiler"), which is exactly why it's stated explicitly rather than left
implicit — the other three rules are things serde can be *made* to enforce;
this one can only be caught by someone reading the diff and asking "does
this field mean the same thing it meant yesterday?"

**Rule 4 — enum variants may be added, never repurposed.** CBOR encodes an
enum by variant name (same self-describing principle as struct field names),
so adding a new variant to, say, `RestoreNote` is exactly as safe as adding a
struct field — old snapshots simply never produced that variant, and nothing
about their decoding changes. But if a future change reused an *existing*
variant name for a different meaning, every snapshot that recorded the old
meaning would silently decode as the new one — the same "loads successfully,
means something else" failure as rule 3, specific to enums.

### What actually *guarantees* compliance

The four rules are review discipline — nothing forces a developer to follow
them except a code reviewer catching a violation. What makes them
*verifiable* rather than merely aspirational is the golden-fixture gate,
`snapshot_fixtures.rs`: a real `.ccstate` file, committed to the repository
alongside the synthetic ROM it was booted from and a `.trace` file recording
2,000 instructions of continuation from the snapshot point. Every future
build must still load that exact file, restore it, and continue producing
*that exact trace*:

```rust
let payload = snapshot::load(&ccstate).unwrap_or_else(|e| {
    panic!(
        "fixture {stem} failed to load -- the snapshot compatibility contract \
         (`coco_core::snapshot`'s module doc: a snapshot written today must load in every \
         future version) was broken: {e}"
    )
});
```

This is what turns "we promise never to break old snapshots" from a
sentence in a doc comment into a test that fails, loudly, with a precise
diagnostic, the moment any future change to `Machine`'s tree violates one of
the four rules against a file that has already shipped. The fixture uses a
small hand-assembled synthetic ROM rather than the real Color BASIC image
specifically so it's safe to commit — booting the real ROM copies its
32K image into RAM during cold start (the same fact `snapshot_roundtrip.rs`'s
doc comment documents), which would embed copyrighted bytes into a test
fixture forever.

---

## 16.9 Restore: re-injecting what serde could never carry

`snapshot::restore` (`crates/coco-core/src/snapshot/restore.rs`) turns a
decoded `SnapshotPayload` plus caller-supplied `MediaSources` bytes into a
live `Machine`, in nine explicitly ordered, explicitly documented steps —
worth reading the doc comment on `restore` itself as a checklist, because
the ordering is load-bearing (step 5, the WD1773 transfer bound-check,
literally cannot run before step 4 reattaches disk data — "this can't run
any earlier," the comment says). Two things in this flow are worth this
chapter's attention beyond the mechanics: what comes back as `None` no
matter how careful the serialization was, and what happens to time.

### `#[serde(skip)]`: host resources that were never data

Chapter 1 flagged `Machine::framebuffer` as `#[serde(skip)]` — derived
scratch, cheap to rebuild, not worth bloating every snapshot to persist.
That was the easy case: a `Vec<u8>` that regrows on demand costs nothing to
skip. The harder cases, scattered through chapters 11 and 14, are fields
that are skipped not because they're *cheap to rebuild* but because they
were **never data in the first place** — they're live handles to something
outside the machine entirely, and no amount of clever serialization could
have captured them, because a serialized byte stream fundamentally cannot
contain a live OS resource.

`Machine::after_restore` (called as restore step 8, after every media
reference has been resolved) rebuilds the cheap-to-derive skipped fields —
framebuffer geometry, audio scratch buffers, the SN76489/AY-3-8913 PSGs'
lookup tables — but it cannot, by construction, reach into the frontend and
hand back a live resource the *core* never owned. That's the frontend's job,
and `coco-egui/src/save_state/restore.rs`'s
`reinject_host_only_resources` does exactly it, right after
`self.machine = restored.machine`:

```rust
fn reinject_host_only_resources(&mut self) {
    if let Some(rtc) = self.machine.bus.cart.as_disto_rtc() {
        rtc.set_time_source(host_time_source());
    }
    if let Some(dw) = self.machine.bus.drivewire.as_mut() {
        dw.set_clock(host_dw_clock());
    }
    if let Some(pak) = self.machine.bus.cart.as_deluxe_rs232() {
        pak.set_endpoint(Box::new(coco_core::serial::Loopback::new()));
    }
}
```

Three concrete examples of "this was never data":

- **The Disto RTC's time source.** A real-time clock cartridge needs to
  answer "what time is it," which means it needs a live handle to the host
  OS's clock — not a value, a *source* of values. Serializing "the current
  time" would freeze the clock at save time; what needs to survive a restore
  is the *capability* to ask again, which is exactly the kind of thing
  `#[serde(skip)]` marks and `reinject_host_only_resources` supplies fresh.
  Until re-injected, the RTC comes back reporting a placeholder — 1970-01-01
  — which is honest (`RestoreNote::RtcPlaceholderTime` says so explicitly)
  rather than silently wrong.
- **The DriveWire clock**, same shape: a live source of "now," not a value.
- **The Deluxe RS-232 endpoint** — chapter 14's `unsafe`-with-`SAFETY`-comments
  PTY code, `posix_openpt`/`grantpt`/`unlockpt` calls that hand back a raw
  file descriptor connected to the host's pseudo-terminal subsystem. There is
  no serializable representation of "an open file descriptor to a live PTY
  on a different machine, possibly one that no longer exists" — the endpoint
  restores as `Loopback` (echo whatever you send back to yourself) rather
  than any attempt to resurrect the old connection, and
  `RestoreNote::Rs232EndpointLoopback` tells the frontend to say so.

Every one of these is a case chapter 1's closing parenthetical anticipated:
"a couple of host-facing edges bend [the no-shared-ownership] rule... both
excluded from save states." Save states are where that exclusion becomes
directly visible as a *behavior*, not just a type-system fact — the note
system (`RestoreNote`) exists specifically so "this came back in a
documented placeholder form" is a typed, matchable value the frontend can
react to individually (the egui frontend filters out
`RtcPlaceholderTime` from the user-facing toast, since by the time the toast
would show, the RTC has *already* been re-synced — see the very next line of
`apply_restored_machine`) rather than a string a caller would have to
pattern-match against wording that might change.

### Dropping time debt: why "catching up" would be wrong

The last piece of `apply_restored_machine` is three assignments that look
almost too small to matter:

```rust
self.last_update = None;
self.field_debt = 0.0;
self.type_ahead.clear();
```

`field_debt` is week 15's fractional wall-clock accumulator
(`app/frame.rs`): every real-time frame adds `dt * field_rate_hz()` to it,
and the app runs as many whole fields as that debt covers, keeping the
fractional remainder for next time — the mechanism that decouples "how often
the host repaints" from "how many emulated fields have actually run," so a
120 Hz monitor doesn't run the CoCo at 120 fields a second. If a save state
had been loaded three days after it was saved, and `field_debt` (or the
`Instant` `last_update` was measured against) survived the restore
unchanged, the very next frame after loading would see an enormous wall-clock
gap since `last_update` and interpret it as "you owe me three days' worth of
emulated fields" — the app would either hang running millions of fields
trying to "catch up," or (if the existing `MAX_FIELDS_PER_UPDATE` guard
caught it) silently skip straight to "now" while the emulated machine's
internal clock lurches forward by three days' worth of fields in one
repaint, which is not what "load a save state" means to anyone. Resetting
`field_debt` to zero and clearing `last_update` is the same discipline
week 15 already applies to an ordinary pause/resume — "drop any time owed to
the wall clock so resuming doesn't 'catch up' across the load, like a
pause," in the code's own comment — save-state restore is simply another
event that must not let real elapsed time leak into emulated time.
`type_ahead.clear()` is the same idea applied to a different queue: a
scripted key-type-ahead buffer left over from before the load has no
business being replayed into a machine whose BASIC prompt, cursor position,
and keyboard-scan state just changed underneath it.

---

## 16.10 `hostile_payload.rs`: a save file is untrusted input

A `.ccstate` file is not a value this program produced and immediately
consumed — it's a file that sits on a filesystem, gets emailed around,
lives in a bug report attachment, and gets loaded back in by a build that
might be months newer than the one that wrote it. Every one of those paths
is an opportunity for the bytes to have been altered — by corruption, by a
well-meaning hex-editor experiment, or by someone deliberately probing for a
crash. `snapshot_engine/hostile_payload.rs` is the test file that takes that
seriously: instead of only testing "does a save I just made load back
correctly," it constructs specific malformed inputs and asserts the failure
mode is always a typed `SnapshotError`, never a panic.

The file's own module doc explains its methodology precisely: most of the
fields it wants to corrupt have no `pub` setter that could ever reach an
out-of-range value through the normal API — the normal protocol dispatch
that populates them "never produces one." So these tests build a *valid*
snapshot through the real `snapshot::save` call, then hand-mutate the raw
CBOR bytes directly — "the same way a hex editor on a real `.ccstate` file
would" — using a small helper that decodes to a generic `ciborium::Value`,
walks a chain of map keys, and overwrites the leaf:

```rust
fn mutate_cbor(cbor: &[u8], path: &[&str], new_value: Value) -> Vec<u8> {
    let mut root: Value = ciborium::from_reader(cbor).expect("decode cbor");
    let mut cursor = &mut root;
    for key in path {
        let map = cursor.as_map_mut()...;
        cursor = &mut map.iter_mut().find(|(k, _)| k.as_text() == Some(*key))...1;
    }
    *cursor = new_value;
    // re-encode
}
```

Four concrete attacks this file mounts, each against a real bug class:

1. **`ssc_load_cap_past_ram_size_is_invalid_payload_not_a_panic`** — sets an
   SSC (Sound/Speech Cartridge) load buffer's `cap` field to `999_999`. The
   normal code path (`Ssc::feed_load`) indexes `self.ram[cursor]` for every
   `cursor` up to `cap`, with no bounds check of its own against the real RAM
   size (`512` bytes) — because on the *legitimate* path, `cap` can never
   legitimately exceed that. A tampered payload removes that guarantee, and
   without a restore-time check this would be a straightforward out-of-bounds
   panic (or, in a hypothetical unsafe implementation, worse) the moment the
   machine resumed and the SSC's load state machine advanced.
2. **`wd1773_transfer_index_past_buf_len_is_invalid_payload_not_a_panic`** —
   same shape, on the WD1773 floppy controller's in-flight sector transfer:
   an `index` field set to `999_999` against a 256-byte sector buffer.
3. **`cassette_bit_out_of_range_is_invalid_payload_not_a_panic`** — sets the
   cassette decoder's `bit` field (which indexes into a byte via a right
   shift, `byte >> bit`) to `9`. In Rust, shifting a `u8` by 9 either panics
   in a debug build or is unspecified behavior in release — a shift amount
   past the type's bit width is exactly the kind of thing that reads as
   "surely never happens" until a hostile input makes it happen.
4. **`nested_multipak_is_invalid_payload_not_a_panic`** — this one isn't even
   a hand-edited byte; it's a shape the *public API itself* doesn't prevent.
   `Cart: From<MultiPak>` makes a `MultiPak` itself `impl Into<Cart>`, so
   nothing in the type system stops one Multi-Pak slot holding another
   Multi-Pak — real hardware physically can't build this (an MPI slot is a
   passive backplane connector, not another MPI chassis), but a crafted
   payload can construct it purely through ordinary deserialization, no
   byte-editing required.

Every one of these is caught by `validate_payload_shape` (restore step 1) or
a device-specific `validate_restored_transfer`/similar check, and every one
resolves to a typed error the caller can display, never a crash:

```rust
let err = expect_err(snapshot::restore(payload, MediaSources::default()));
assert!(matches!(err, SnapshotError::InvalidPayload(_)), "{err:?}");
```

This is the same discipline as `gunzip`'s decompression-bomb guard (§16.6)
applied one layer up the stack: **every field a hostile payload might have
tampered with gets validated against the invariants the normal code path
would have upheld automatically, before the machine is ever allowed to run
on it.** A save file crossing this boundary is not trusted just because it
decodes — decoding successfully only proves the *shape* is well-formed CBOR;
whether the *values* are ones the running machine could ever legitimately
have produced is a separate question, and this restore path answers it
explicitly rather than assuming.

---

## 16.11 Lockstep: determinism as a testable property

`crates/coco-core/tests/snapshot_engine/lockstep.rs` (and its phase-1
sibling, `snapshot_roundtrip.rs`) test something more fundamental than "does
loading crash" — they test that a save/restore cycle is *invisible* to the
machine's own execution. The concept: boot the real ROM, run it for a while,
save, restore into a second `Machine`, then run **both** machines side by
side — the original, never-saved one and the restored one — one instruction
at a time, comparing full CPU state after *every single step*, for a million
steps:

```rust
const LOCKSTEP_STEPS: u32 = 1_000_000;

for i in 0..LOCKSTEP_STEPS {
    let orig_event = original.step_instruction();
    let rest_event = restored.step_instruction();
    assert_eq!(orig_event, rest_event, "step event diverged at lockstep instruction {i}");
    assert_eq!(
        CpuSnapshot::of(&original.cpu),
        CpuSnapshot::of(&restored.cpu),
        "CPU state diverged at lockstep instruction {i}"
    );
    assert_eq!(
        original.current_scanline(), restored.current_scanline(),
        "scanline position diverged at lockstep instruction {i}"
    );
}
```

The reasoning behind "one instruction, compare, repeat" rather than "run a
million and compare once at the end" is precision: any single field left
out of the serde tree, or restored into subtly the wrong state, shows up as
a divergence at the *specific* instruction where its effect first differs —
possibly step 3, possibly step 900,000 if the missing state only matters
once an interrupt fires or a timer wraps — rather than as a vague "the
machines ended up different somehow" a million steps later with no way to
localize the cause.

Why `step_instruction` (chapter 6's resumable primitive) and not the bare
CPU-only `step`? The comment states it precisely: `step_instruction` drives
"the full per-scanline pipeline (GIME timer ticks, PIA field-sync IRQs,
audio-event flushing, cartridge ticking, and the resumable
`line`/`line_cycles_spent` state)," and the snapshot is deliberately taken
*mid-field, at an arbitrary instruction boundary* — "exactly what the
frontend's save-while-running does." A bare CPU-only comparison would miss
any divergence in that per-scanline machinery entirely; testing at the level
the real frontend actually saves at is what makes this test mean what it
claims to mean.

This is, in the fullest sense, a **determinism test**: it asserts that
"emulated CoCo, saved and restored" and "the same emulated CoCo, left
running the whole time" are the *same machine* from that point forward, byte
for byte, for a million instructions — not merely "close enough" or
"eventually converges," but bit-identical at every single step, including
the exact number of CPU cycles burned by the exact same interrupts landing
on the exact same scanlines. That property doesn't hold by accident; it
holds because every piece of state the running machine's execution actually
depends on — registers, RAM, GIME registers, PIA latches, the mid-field
`line`/`line_cycles_spent`/`field_scan` bookkeeping chapter 6 hoisted onto
`Machine` specifically to make resumability possible — made it into the
serde tree, and everything that didn't (the `#[serde(skip)]` fields) is
either genuinely derived (safe to omit) or genuinely host-only (re-injected
explicitly, §16.9) rather than silently forgotten. Lockstep is the test that
would catch it if any of those judgment calls were wrong.

---

## 16.12 Running the suites — what actually passes here

This worktree, like every clone or worktree of this repository, has no
`roms/` directory — real ROM images are git-ignored and present only on the
maintainer's primary checkout, exactly as chapter 1 warned. That makes this
a useful, honest demonstration of which tests are ROM-dependent and which
aren't, rather than a hypothetical. Running the suites this chapter is built
on, in this exact worktree:

```
$ cargo test -p coco-core --test debug
running 10 tests
test result: FAILED. 1 passed; 9 failed; 0 ignored; ...

$ cargo test -p coco-core --test snapshot_roundtrip
running 2 tests
test result: FAILED. 0 passed; 2 failed; 0 ignored; ...

$ cargo test -p coco-core --test snapshot_engine
running 17 tests
test result: FAILED. 12 passed; 5 failed; 0 ignored; ...

$ cargo test -p coco-core --test snapshot_fixtures
running 2 tests
test result: ok. 1 passed; 0 failed; 1 ignored; ...
```

Every single failure traces back to one cause: `load_rom()`/`boot_machine()`
helpers that `panic!` on `std::fs::read("../../roms/coco3.rom")` when the
file isn't there. That's nine of `debug.rs`'s ten tests (only
`trace_entry_format_is_exact`, which hand-builds a `MC6809` struct directly
with no ROM at all, survives); both of `snapshot_roundtrip.rs`'s tests; and
five of `snapshot_engine.rs`'s seventeen — specifically `lockstep`'s one
test (needs a full ROM boot to have anything interesting to lock-step
against) and four of `media.rs`'s tests that hash or reattach the real
system ROM.

The other twelve `snapshot_engine` tests — every `header.rs` test (magic,
container version, schema-too-new), all four `hostile_payload.rs` attacks
(SSC, WD1773, cassette, nested Multi-Pak) plus the oversized-gzip guard, and
three of `media.rs`'s tests (`MediaRef::verify`, the RAM-length-mismatch
guard, `sha256_file`/`sha256_hex` agreement) — need no ROM at all, because
they either build a `Machine` with an empty ROM image (`Box::new([])`,
perfectly legal for tests that never execute a CPU instruction) or work at
the byte level entirely. Every one of them passes. And `snapshot_fixtures.rs`
passes its one real (non-`#[ignore]`d) test completely: the golden-fixture
gate that proves the committed `v1-synthetic.ccstate` still loads, restores,
and continues trace-identically, using a hand-assembled synthetic ROM that
was written specifically so this test *never* needs a real, copyrighted ROM
image to prove the compatibility contract holds.

The pattern is exactly what chapter 1's "ROMs are local-only" warning
predicted, and it doubles as a small confirmation of this chapter's own
argument: the tests that matter most for proving the *engine's* correctness
under adversarial input — hostile_payload's four attacks, the header
tests, the golden-fixture gate — were deliberately written to need no
copyrighted material at all, which is exactly why they're the ones still
green in a bare worktree with no `roms/` directory.

---

## 16.13 Retrospective: the week-1 decisions, scored

Fifteen weeks ago this course opened with two abstractions: the `Bus`
trait, and a ban on `Rc<RefCell<...>>` in the machine's state tree. Here is
the honest accounting — what each decision (and a few later ones) bought,
where in this course it paid off, and what it genuinely cost. A retrospective
that only lists payoffs isn't honest; every one of these had a price, and
naming the price is the point.

**The `Bus` trait, `read(&mut self)`.**
*Bought:* honest PIA/GIME side effects with zero interior-mutability
ceremony (week 1); a CPU testable against a bare `FlatBus` with no machine
attached, run in ~200 tests with zero CoCo code compiled in (week 1, week
2); and, this week, the entire `peek()`/`read()` twin-contract that makes a
debugger safe to open. *Cost:* every device with a read side effect now
needs two implementations kept in sync by hand — `read`'s real path and
`peek`'s side-effect-free mirror — with only a test
(`peek_matches_read_for_ram_and_rom`, and device-specific ones like
`peek_does_not_clear_pia_flags`) rather than the type system enforcing that
they agree on the *data*, only that `peek` can't mutate. A future device
added without its own `peek` arm silently falls through to
`SystemBus::io_peek`'s `_ => OPEN_BUS` catch-all — wrong, not a compile
error — unless someone remembers to add it and a test catches the gap.

**No `Rc<RefCell<...>>` in the machine's state tree.**
*Bought:* `#[derive(Serialize, Deserialize)]` on `Machine` working at all —
this chapter's entire save-state system exists because this rule was set on
day one, not retrofitted (DESIGN.md §9 says exactly this: "nearly free if
you avoid `Rc`/`RefCell`/raw pointers... and miserable to retrofit"). *Cost:*
every place a subsystem needs several fields of `self` at once — GIME video
scanout needing the GIME's registers, RAM, and the framebuffer
simultaneously (chapter 1, `render.rs:56`) — pays for it in destructuring
boilerplate (`let SystemBus { gime, ram, .. } = &mut self.bus;`) or free
functions taking exactly the disjoint borrows they need instead of `&mut
self` methods, spread across every rendering chapter (7 through 9). And the
rule has two genuine, documented exceptions where the host forces shared
ownership anyway — the printer capture sink (`Rc<RefCell<Vec<u8>>>`,
chapter 14) and the Deluxe RS-232 PTY bridge's raw `libc` calls (chapter
14) — both explicitly excluded from the serde tree and explicitly
re-injected on restore (§16.9), which is extra restore-path code that
exists purely because the ownership rule *couldn't* be made absolute at the
edge where the emulator meets the host OS.

**The headless core (`coco-core` renders into `Vec<u8>`, never opens a
window).**
*Bought:* the PPM lab bench (chapters 7–9, no GPU needed for any rendering
test); the real-ROM boot tests running in CI with no display (chapter 6);
and, indirectly, this chapter's golden-fixture gate and lockstep test, which
would be essentially impossible to write reliably against a windowed,
frame-timed application. *Cost:* every piece of state a human actually wants
to *see* while debugging — the framebuffer, most obviously — has to be
either persisted (bloating every snapshot for no functional reason) or
cheaply rebuildable and explicitly rebuilt (`#[serde(skip)]` plus
`after_restore`, exactly the framebuffer's fate). The core buys testability
by knowing nothing about presentation, which means presentation-adjacent
bookkeeping (pacing, `field_debt`, the toast message system) all lives one
layer up in `coco-egui`, duplicated in spirit — though not in code — for
every frontend this core might ever grow.

**Instruction-granular cycle accounting, not cycle-exact.**
*Bought:* a CPU simple enough to write, read, and trust in three weeks
(chapters 2–4) instead of the multi-month effort a cycle-exact 6809 core
would demand; good enough to boot real BASIC, run real games, and pass every
functional test this course has built. *Cost, precisely stated, not
hand-waved:* chapter 4 found — by grepping for `self.cycles` and finding it
touched in exactly two places in the entire `mc6809` crate, neither one
inside `take_interrupt` — that **interrupt entry costs zero cycles** in this
emulator's own accounting. `nmi()`/`irq()`/`firq()` push a full or partial
stack frame (up to twelve bus writes) and never once increment `self.cycles`
doing it; `psh`'s own byte-accurate return value is computed and then
discarded every time `take_interrupt` calls it. As chapter 4 put it: "an
externally-delivered interrupt is, as far as this CPU crate's clock is
concerned, free." That's not a bug hiding — it's a direct, named consequence
of the instruction-granular choice DESIGN.md §5 states as policy from the
start, and it's exactly where a trace-diff against real MAME (which *does*
cost this) would start disagreeing on cycle counts while still agreeing on
every register value. Nothing this course's own software needs has ever
required that precision — CoCo software mostly cares which *scanline* a
handler lands on, the granularity chapter 6 already provides, not whether
the handler's first instruction lands 10 or 19 cycles after the line was
asserted — but "nothing needed it yet" is a fact about this course's test
suite, not a proof the gap can never matter. The live-register rendering
chapters 8–9 built (registers re-read every scanline rather than latched
once per field) claws back *some* of the precision a coarser design would
have lost outright — mid-frame palette and border changes land on the right
line even though mid-*instruction* timing is still out of reach — but that's
a partial recovery at the video layer, not a fix to the CPU's own cycle
accounting.

**The scanline-driven, resumable `step_instruction` loop.**
*Bought:* everything in §16.2–16.5 — `run_until`'s ability to stop after any
single instruction, mid-field, and hand a perfectly valid, resumable machine
back to a caller; every step command in the debugger UI; and, less
obviously, the lockstep test's ability to compare two machines one
instruction at a time at all. None of that works without `line`,
`line_cycles_spent`, `line_budget`, and `field_scan` having been hoisted out
of `run_field`'s local loop variables and onto `Machine` itself in chapter
6 — the exact refactor whose only visible justification, at the time, was
"a debugger will need this eventually." *Cost:* the same fusion that makes
`step_instruction` atomic and resumable — interrupt entry plus the handler's
first instruction, one call, no seam in between — is exactly what makes a
breakpoint on an interrupt vector's target address structurally unreachable
(§16.3). Resumability and "every intermediate PC value is independently
breakpointable" turned out to be in tension, and this codebase chose
resumability; the gap is the receipt.

**Fidelity as an explicit, subsystem-by-subsystem budget (DESIGN.md's
table, chapter 1).**
*Bought:* effort spent exactly where real software would notice — cycle-
timestamped audio events because the DAC's actual output waveform depends on
*when within a scanline* the CPU wrote, cycle-granular cassette FSK edges
because the ROM's own decoder counts them, but only a functional,
byte-paced WD1773 state machine because BASIC's disk driver never counts
controller cycles the way its tape loader counts tape cycles. *Cost:* every
one of those choices is a bet about what software this emulator will ever
be asked to run, made without the ability to test against all CoCo software
that has ever existed. The bet has held for this course's test suite and
every real-ROM boot test in it; it is not, and cannot be, a proof it will
hold for the next program someone points this emulator at. "Tighten later
only if a game needs it" (DESIGN.md §5) is a genuinely good policy for
managing effort — and it is also, honestly, a promise that some future bug
report will read "this fidelity line was in the wrong place for this one
piece of software," and someone will have to move it.

The through-line across all six: **every one of these decisions was cheap
to make in week 1 and would have been expensive or impossible to retrofit
in week 16.** That asymmetry — decide the shape early, pay a small
continuous tax for it every week after, versus discover the need for it
late and rewrite the foundation — is the single idea this whole course has
been trying to teach through sixteen weeks of specific chips instead of
stating abstractly on day one. You now have both the abstract statement and
fifteen weeks of concrete evidence for it.

---

## 16.14 Where you go next

There is no week 17. Three honest directions from here, in ascending order
of commitment:

1. **Write your own core against the same `Bus` trait.** The two-method
   contract in `crates/mc6809/src/lib.rs` is the entire interface a machine
   needs to implement to reuse this project's CPU. Nothing in `mc6809`
   knows a CoCo exists — exercise 1.8 already made you verify this
   mechanically by reading `Cargo.toml`. Build a `FlatBus`-shaped toy for a
   different memory map and you have a second machine.
2. **Port to another 6809 machine.** The Vectrex and the Dragon (a
   British CoCo-compatible-but-not-quite competitor) both ran the MC6809.
   Neither has a GIME, a SAM, or this project's exact I/O page — which is
   the point: porting forces you to discover exactly how much of
   `coco-core` is genuinely CoCo-specific (the bus decode, every device from
   `pia.rs` on down) versus genuinely 6809-generic (everything in `mc6809`,
   and the debugger/save-state *machinery* in this chapter, which has
   nothing CoCo-specific in its design even though this codebase's instance
   of it does).
3. **Contribute to the deferred-scope list**, Appendix C's honest accounting
   of what this codebase chose not to build yet: the 6309 (the 6809's
   binary-compatible, faster, more-registers successor, popular in real
   CoCo 3 upgrades), artifact-color composite emulation beyond what chapter
   9 already covers, and true cycle-exact mid-instruction timing — the exact
   gap this chapter's retrospective just spent a paragraph pricing out.
   Every one of those is a real, bounded, well-scoped piece of work, sitting
   on top of an architecture built, from week 1, specifically so that adding
   to it wouldn't require rebuilding anything underneath.

Whichever you pick, you now know where every seam in this codebase is, and
— more usefully — you know *why* each seam is where it is, which is the
thing a table of contents can never teach on its own.

---

## 16.15 Reading assignment

In this order:

1. `crates/coco-core/src/debug.rs`, all of it — small enough to read in one
   sitting, and every line of it has now been explained in this chapter.
2. `crates/coco-core/src/bus/peek.rs`, all of it, side by side with
   `crates/coco-core/src/bus.rs`'s `io_read`/`sam_io_read` — for every I/O
   region, ask "what would `read` have mutated here, and how does `peek`
   avoid it?"
3. `crates/coco-egui/src/debugger.rs` and its six submodules — skim first
   for the panel layout, then reread `controls.rs`'s four step primitives
   slowly; they're the part of this chapter most worth typing out by hand.
4. `crates/coco-core/src/snapshot.rs`'s module doc, twice — once before
   reading any of `snapshot/`'s five files, once after, to notice how much
   of the second reading was already explained by the first.
5. `crates/coco-core/tests/snapshot_engine/hostile_payload.rs`, all of it —
   this is the file in this whole course most worth reading as a security
   document, not just a test file.

```
cargo test -p coco-core --test debug --test snapshot_roundtrip \
    --test snapshot_engine --test snapshot_fixtures
```

Run it in whatever checkout you have. If you have `roms/coco3.rom`, expect
everything green. If you don't, expect exactly the pattern §16.12 measured
— and take the shape of that pattern as itself informative: it tells you,
mechanically, which of this project's guarantees don't need a single byte
of copyrighted software to prove.

---

## 16.16 Exercises

**16.1 — `peek` vs. `read`, by address (read + recall).** Using
`bus/peek.rs`'s `io_peek` match arms, list every `$FF00–$FFBF` sub-range
where `peek` and `read` return *different logic paths* (not necessarily
different byte values on a given call, but a genuinely different code path
— e.g. `IRQENR_REG`/`FIRQENR_REG` reading a latched field instead of
performing the read-acknowledges-and-clears dance). Then list the
sub-ranges where they're identical. For each *different* case, name the
specific side effect `read` performs that `peek` must avoid, in one
sentence. (You should find at least: both PIAs' Cx1 flags, the GIME
IRQ/FIRQ pending registers, and — check `sam_peek`'s `SamTarget::Io` arm for
the CoCo 1/2 path too — the same PIA cases there.)

**16.2 — Hand-decode a header (build + verify).** A `.ccstate` file begins
with these sixteen bytes, in hex:

```
43 43 53 54 41 54 45 01 01 00 00 00 1f 8b 08 00
```

Using `CONTAINER_MAGIC = b"CCSTATE"`, `HEADER_LEN = 12`, and `SCHEMA_VERSION`'s
`u32` little-endian encoding, decode: the magic string, the container
version, the schema version, and identify what the next two bytes (`1f 8b`)
are a signature for (hint: it's not part of this module's own framing).
Then write a five-line Rust snippet using `snapshot::CONTAINER_MAGIC` and
`snapshot::CONTAINER_VERSION` that asserts this exact byte sequence is what
`snapshot::save` on a freshly-`Machine::new`'d machine with an empty ROM
would produce for its first twelve bytes — run it and confirm.

**16.3 — The breakpoint that can't be hit (recall + predict).** Without
re-reading §16.3, explain from memory: (a) why a breakpoint set on an
interrupt vector's target address never fires through `Debugger::run_until`;
(b) what happens instead if you set the breakpoint one instruction later;
(c) what the trace ring records for the PC of the step that services the
interrupt, and why. Then check yourself against §16.3 and `debug.rs`'s own
doc comment on `run_until`.

**16.4 — Sabotage `hostile_payload.rs`'s defense, verified (sabotage +
run).** In `crates/coco-core/src/snapshot/restore.rs`, find
`validate_payload_shape` and comment out (don't delete — you'll revert by
uncommenting) the line `machine.bus.validate_restored().map_err(...)?;`.
Run `cargo test -p coco-core --test snapshot_engine` and record exactly
which test(s) newly fail and what panic message appears (you're looking
for one of the `hostile_payload.rs` "not a panic" tests to now genuinely
panic, or fail its `matches!(err, SnapshotError::InvalidPayload(_))`
assertion because no error was ever produced). Then revert your edit with
the exact inverse `Edit`, rerun the suite, and confirm `git status` is
clean and every test that failed is green again. Write two sentences on
what specific guarantee that one line was providing, based on what broke
without it.

**16.5 — Add a field, prove old snapshots still load (build).** Pick any
struct that lives inside `Machine`'s serde tree with a `u8`- or `bool`-shaped
piece of state you understand (a decent candidate: `crate::pia::MC6821`, or
any small device struct in a file you've already read from an earlier
chapter). Add a new field — something genuinely new, e.g. a `debug_counter:
u32` that increments once per `tick`/`step` call — following evolution
rules 1 and 2 exactly: `#[serde(default)]`, and a default that reproduces
old behavior (zero, if the counter is purely informational). Then run
`cargo test -p coco-core --test snapshot_fixtures` and confirm
`all_committed_fixtures_still_load` still passes — the committed
`v1-synthetic.ccstate` fixture, written *before* your field existed, must
still load, restore, and produce its exact committed `.trace` continuation.
If it fails, you violated one of the four rules; say which one and fix it.
Revert your change once you've proven the point, unless you were explicitly
asked to keep it.

**16.6 — Lockstep, read and predict (read + predict).** Before running
anything, predict: if you take `snapshot_engine/lockstep.rs`'s
`full_round_trip_continues_trace_identically` test and change
`WARMUP_STEPS` from `200_000` to `0` (save immediately after boot, before
BASIC's cold-start has done anything), does the test still pass? Write down
your prediction and the reasoning, specifically addressing whether anything
about *when* in the boot sequence the snapshot is taken should matter to a
test that's asserting "saved-and-restored behaves identically to
never-saved" — then make the change and run it to check. (If your prediction
was wrong, the interesting question is *why* you expected timing to matter
when the test's whole point is that it shouldn't — or vice versa.)

**16.7 — Capstone (essay + project, no length cap).** Pick a real CoCo
program you have access to — a game, a demo, a piece of OS-9, anything that
runs on real hardware — and get it running in this emulator. Then use the
debugger and trace ring from this chapter to explain, with cited addresses
and register values from an actual session (not from documentation), *one
specific thing that program does with the hardware* that you did not
already know before this course. Candidates that tend to produce a good
answer: a mid-frame palette or border split (chapter 9) caught live in the
Hardware panel as the border color changes; a keyboard-scan routine watched
byte-by-byte through a PIA-address watchpoint (chapter 10); a disk boot
sequence traced through the WD1773's command dispatch (chapter 13) using
Step Over to skip past the sector-copy loop and Step In to walk through the
handshake itself. Write it up as you would a debugging session for a
colleague: what you set (breakpoints, watchpoints), what you saw, and what
it proved about how the software uses the hardware you have now spent
sixteen weeks learning to emulate.
