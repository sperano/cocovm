# Chapter 4 — CPU core III: interrupts, halt states, and how to test a CPU with no test suite

*Week 4, the last week inside the CPU crate before week 5 opens the bus. Goal:
know both interrupt stack frames cold — every byte, every address — and come
away with a validation strategy you can reuse on any CPU core you ever write,
because the 6809 will not hand you one for free.*

---

## 4.1 Two shapes of interrupt frame

Every 6809 exception — `NMI`, `IRQ`, `FIRQ`, `SWI`, `SWI2`, `SWI3` — does the
same three things: save enough state to resume later, block re-entrant
interrupts as appropriate, and load `PC` from a fixed vector. What differs
is *how much* state "enough" means. Five of the six save everything: `A`,
`B`, `DP`, `X`, `Y`, `U`, `CC`, `PC` — twelve bytes, the **full frame**.
`FIRQ` alone saves only `CC` and `PC` — three bytes, the **partial frame**.
That's the whole point of `FIRQ` ("fast" IRQ): a device with light,
latency-sensitive work — a UART about to overrun, a disk controller with a
byte sitting in its shift register — gets in and out fast. Nine fewer bytes
to push and pull is real time saved at 0.895 MHz.

Which frame is on the stack has to be recorded somewhere, because the `RTI`
that eventually unwinds it needs to know how many bytes to pull. That
somewhere is bit 7 of `CC`, the **E** (entire) flag:

```rust
// crates/mc6809/src/lib.rs:47-57
pub mod cc {
    pub const CARRY: u8 = 0x01;
    pub const OVERFLOW: u8 = 0x02;
    pub const ZERO: u8 = 0x04;
    pub const NEGATIVE: u8 = 0x08;
    pub const IRQ_MASK: u8 = 0x10;
    pub const HALF_CARRY: u8 = 0x20;
    pub const FIRQ_MASK: u8 = 0x40;
    pub const ENTIRE: u8 = 0x80;
}
```

`E=1` means "the full frame is under me"; `E=0` means "just `CC` and `PC`."
`take_interrupt` sets it before pushing, and `RTI` reads it back after
pulling `CC` first (the one register both frames agree is on top):

```rust
// crates/mc6809/src/exec.rs:263-289
fn exec_interrupt_halt(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
    match opcode {
        0x3F => { self.take_interrupt(bus, VECTOR_SWI, true, true, true); 19 }  // SWI
        0x3B => { // RTI — pull CC, then full frame if E set else PC only
            self.pul(bus, 0x01, true);
            if self.cc & cc::ENTIRE != 0 {
                self.pul(bus, 0xFE, true);
                15
            } else {
                self.pul(bus, 0x80, true);
                6
            }
        }
        0x3C => { // CWAI — clear CC bits, stack full frame, then halt
            let m = self.fetch_u8(bus);
            self.cc &= m;
            self.cc |= cc::ENTIRE;
            self.psh(bus, 0xFF, true);
            self.state = State::Waiting;
            22
        }
        0x13 => { self.state = State::Syncing; 2 } // SYNC — halt until interrupt

        _ => unreachable!("exec_interrupt_halt called for opcode {opcode:#04X}"),
    }
}
```

`RTI` proves the frame shape is data-driven, not opcode-driven: it pulls
`CC` first, looks at the bit it just pulled, and only *then* decides
whether nine more bytes are coming — 15 cycles for a full unwind, 6 for a
partial one, the datasheet's numbers hard-coded at the point each branch
is taken.

### Byte-by-byte, with real addresses

Don't take "twelve bytes" and "three bytes" on faith — here are the actual
addresses, taken directly from `tests/interrupts.rs`'s own setup (`s.cpu.s =
0x2000` before the interrupt fires).

**Full frame** — `IRQ`, `NMI`, `SWI`, `SWI2`, `SWI3` (`S` before: `$2000`):

| Address | Contents | Address | Contents |
|---------|----------|---------|----------|
| `$1FF4` | CC       | `$1FFA` | Y (hi)   |
| `$1FF5` | A        | `$1FFB` | Y (lo)   |
| `$1FF6` | B        | `$1FFC` | U (hi)   |
| `$1FF7` | DP       | `$1FFD` | U (lo)   |
| `$1FF8` | X (hi)   | `$1FFE` | PC (hi)  |
| `$1FF9` | X (lo)   | `$1FFF` | PC (lo)  |

`S` after: `$1FF4`. That table is not a paraphrase — it's what
`swi_stacks_full_frame_and_vectors` actually asserts:

```rust
// crates/mc6809/tests/interrupts.rs:107-121
#[test]
fn swi_stacks_full_frame_and_vectors() {
    let mut s = Sys::code(0x1000, &[0x3F]); // SWI
    s.bus.load(0xFFFA, &[0x90, 0x00]); // SWI vector -> $9000
    s.cpu.s = 0x2000;
    s.cpu.cc = 0x00;
    let cycles = s.step();
    assert_eq!(s.cpu.pc, 0x9000);
    assert_eq!(s.cpu.s, 0x2000 - 12); // full 12-byte frame
    assert_eq!(cycles, 19);
    assert_ne!(s.cpu.cc & cc::IRQ_MASK, 0); // SWI sets I
    assert_ne!(s.cpu.cc & cc::FIRQ_MASK, 0); // and F
    assert_ne!(s.cpu.cc & cc::ENTIRE, 0); // E set
    assert_eq!(s.mem(0x1FF4), 0x80); // stacked CC = original | E (masks set after push)
}
```

`s.mem(0x1FF4) == 0x80` is exactly the table above: `$1FF4` holds `CC`, and
`0x80` is the `E` bit alone (`cc` started at `0x00`, so the only bit stacked
is the one `take_interrupt` sets *before* pushing).

**Partial frame** — `FIRQ` only (`S` before: `$2000`):

| Address | Contents |
|---------|----------|
| `$1FFD` | CC       |
| `$1FFE` | PC (hi)  |
| `$1FFF` | PC (lo)  |

`S` after: `$1FFD`. Confirmed the same way in `firq_uses_partial_frame`
(§4.6). Notice the partial frame's `CC` byte lands at `$1FFD` — the *same*
address the full frame uses for `U`'s low byte. Not a coincidence to be
nervous about — the stack is just three bytes deep instead of twelve, and
`stack.rs`'s address arithmetic (`sp.wrapping_sub(1)`) doesn't care which
frame it's building.

### Push order, straight from `stack.rs`

Both frames are produced by the same `psh` helper that backs `PSHS`; the
interrupt path just passes a specific register mask:

```rust
// crates/mc6809/src/stack.rs:22-25
/// PSHS/PSHU. `to_s` selects the hardware (S) stack; otherwise the user (U)
/// stack. Push order is PC, U/S, Y, X, DP, B, A, CC (highest address first),
/// so CC ends up on top. Bit 6 of the mask pushes the *other* stack pointer.
/// Returns the cycle count (base + 1 per byte).
```

"`PC` first" sounds backwards until you remember pushing walks the stack
pointer *downward*: whatever is pushed first ends up at the *highest*
address, and whatever is pushed last — `CC` — ends up on top, at the
lowest address, exactly where `RTI`'s first `pul` (which always reads `CC`
first) expects it. The full-frame mask is `0xFF` — every `stack_mask` bit
set at once, including `OTHER_STACK_PTR` (`0x40`, `U` when pushing to `S`).
The partial frame's mask is `PC_CC_MASK = stack_mask::PC | stack_mask::CC`
— just those two bits, why only three bytes move.

> **Rust corner: closures that borrow explicitly, not by capture.** `psh`'s
> inner helper, from `crates/mc6809/src/stack.rs`:
> ```rust
> let mut push8 = |sp: &mut u16, v: u8, n: &mut u32| {
>     *sp = sp.wrapping_sub(1);
>     bus.write(*sp, v);
>     *n += 1;
> };
> ```
> takes `sp` and the byte counter as `&mut` *parameters* rather than
> capturing them from the enclosing scope — only `bus` is captured
> (mutably, since `write` needs it). If `sp` were captured instead of
> passed, the closure would hold a mutable borrow of the local `sp` for its
> entire lifetime, and the surrounding code could never touch `sp` between
> calls — which `psh` needs to do, since it rebinds `self.s` from the final
> `sp` value once every register in the mask has been pushed. Passing state
> through parameters instead of capture is a general technique for keeping
> a closure's borrow narrow: it borrows only what it must, only for the
> duration of each individual call, leaving the caller free to read or
> mutate the same state in between calls.

---

## 4.2 The vector table, priority, and masking

All six vectors live in the top sixteen bytes of address space, one 16-bit
pointer apiece, descending from `RESET`:

```rust
// crates/mc6809/src/lib.rs:113-121
/// Hardware interrupt / exception vectors (top of the address space).
pub const VECTOR_SWI3: u16 = 0xFFF2;
pub const VECTOR_SWI2: u16 = 0xFFF4;
pub const VECTOR_FIRQ: u16 = 0xFFF6;
pub const VECTOR_IRQ: u16 = 0xFFF8;
pub const VECTOR_SWI: u16 = 0xFFFA;
pub const VECTOR_NMI: u16 = 0xFFFC;
/// RESET vector address (`$FFFE`/`$FFFF`).
pub const VECTOR_RESET: u16 = 0xFFFE;
```

Seven vectors, two bytes each, packed into `$FFF2`–`$FFFF` with no gaps —
`RESET` at the very top, then descending through `NMI`, `SWI`, `IRQ`,
`FIRQ`, `SWI2`, down to `SWI3` at the bottom. Each entry point independently
decides whether it's masked. `IRQ` respects
`CC`'s `I` bit; `FIRQ` respects `F`; `NMI` respects neither (its own gate,
`nmi_armed`, is §4.3):

```rust
// crates/mc6809/src/lib.rs:200-224
/// Deliver an IRQ. Ignored (returns `false`) while the I mask is set — but a
/// masked line still wakes a `SYNC`. Returns `true` if serviced.
pub fn irq(&mut self, bus: &mut impl Bus) -> bool {
    if self.cc & cc::IRQ_MASK != 0 {
        if self.state == State::Syncing {
            self.state = State::Running;
        }
        return false;
    }
    self.take_interrupt(bus, VECTOR_IRQ, true, false, true);
    true
}

/// Deliver a FIRQ. Ignored while the F mask is set (but wakes a `SYNC`).
/// Uses the fast partial frame (CC+PC only) and sets both I and F.
pub fn firq(&mut self, bus: &mut impl Bus) -> bool {
    if self.cc & cc::FIRQ_MASK != 0 {
        if self.state == State::Syncing {
            self.state = State::Running;
        }
        return false;
    }
    self.take_interrupt(bus, VECTOR_FIRQ, true, true, false);
    true
}
```

`take_interrupt`'s last two boolean parameters, `set_i` and `set_f`, decide
which masks get *set on entry* — what stops a second interrupt from
preempting the handler before it can save context. Reading straight off
each call site:

| Exception | Sets `I` | Sets `F` | Frame    |
|-----------|----------|----------|----------|
| `NMI`     | yes      | yes      | full     |
| `IRQ`     | yes      | no       | full     |
| `FIRQ`    | yes      | yes      | partial  |
| `SWI`     | yes      | yes      | full     |
| `SWI2`    | no       | no       | full     |
| `SWI3`    | no       | no       | full     |

`IRQ` is the odd one out in the "sets both" column, deliberately: an `IRQ`
handler that wants a higher-priority `FIRQ` to interrupt it can just... not
touch `F`, because `IRQ` never set it. Everything else that can
legitimately claim "I got here first" — `NMI`, `FIRQ`, `SWI` — locks out
both lines on entry. `SWI2`/`SWI3`, the software-call vectors (OS-9 system
calls and the like), leave the masks alone so a syscall handler runs with
interrupts exactly as they were in the caller.

### `take_interrupt`, one call at a time

The table above is a summary; the six call sites that produce it all funnel
through a single function, and it's worth reading in full — nothing else
anywhere in the crate stacks a frame or vectors `PC`:

```rust
// crates/mc6809/src/lib.rs:226-254
/// Common interrupt sequence: stack the frame (unless `CWAI` already did),
/// set the requested masks, and vector. `entire` selects the full frame (E=1)
/// vs the FIRQ partial frame (E=0).
fn take_interrupt(
    &mut self,
    bus: &mut impl Bus,
    vector: u16,
    set_i: bool,
    set_f: bool,
    entire: bool,
) {
    if self.state != State::Waiting {
        if entire {
            self.cc |= cc::ENTIRE;
            self.psh(bus, 0xFF, true);
        } else {
            self.cc &= !cc::ENTIRE;
            self.psh(bus, PC_CC_MASK, true);
        }
    }
    if set_i {
        self.cc |= cc::IRQ_MASK;
    }
    if set_f {
        self.cc |= cc::FIRQ_MASK;
    }
    self.pc = bus.read_u16(vector);
    self.state = State::Running;
}
```

Five parameters, and the six call sites each pick a different combination —
every one of them, verbatim, with the exact line it lives on:

```rust
self.take_interrupt(bus, VECTOR_NMI,  true,  true,  true);  // nmi()  — lib.rs:197
self.take_interrupt(bus, VECTOR_IRQ,  true,  false, true);  // irq()  — lib.rs:209
self.take_interrupt(bus, VECTOR_FIRQ, true,  true,  false); // firq() — lib.rs:222
self.take_interrupt(bus, VECTOR_SWI,  true,  true,  true);  // SWI    — exec.rs:266
self.take_interrupt(bus, VECTOR_SWI2, false, false, true);  // SWI2   — exec.rs:169
self.take_interrupt(bus, VECTOR_SWI3, false, false, true);  // SWI3   — exec.rs:190
```

Read the parameters left to right against the body. `vector` picks which
two bytes at the top of memory load into `PC` — the table from the top of
this section. `set_i`/`set_f` drive the two `if` blocks at the *bottom* of
the function, and notice where those `if`s sit: *outside* the
`if self.state != State::Waiting` guard, unconditionally, every time. That
matters once CWAI enters the picture (§4.4) — even an interrupt that wakes
a `CWAI`'d CPU and therefore skips re-stacking still gets its masks set by
whichever entry point woke it, because mask-setting isn't gated on stacking
at all. `entire` only has an effect *inside* that guard, and it's a genuine
either/or: `self.psh(bus, 0xFF, true)` (mask `0xFF`, every `stack_mask` bit,
the full 12-byte frame) or `self.psh(bus, PC_CC_MASK, true)` (mask
`PC_CC_MASK`, two bits, the 3-byte frame) — never both, never neither.
Scan the six-line table above and `firq()` is the only call passing `false`;
everyone else passes `true`. One boolean, read once, is the entire
difference between "fast" and "everything else."

There's no `match`-based priority table anywhere in `mc6809` itself — each
of `nmi()`/`firq()`/`irq()` is an independent entry point, and *priority is
enforced by whoever calls them, in the order they're called*: a bus-level
decision (`crates/coco-core/src/machine/run.rs`'s `service_interrupts`
checks `FIRQ` before `IRQ`, `NMI` is polled separately ahead of both — week
6). The CPU crate expresses masking, a per-line property encoded in `CC`;
priority, an ordering property, is the bus's job, because only the bus
knows which devices are asserting which lines at all.

### SWI vs SWI2 vs SWI3

All three software interrupts push the full frame — there's no "fast `SWI`"
the way there's a fast `IRQ`. What differs is prefix bytes and mask
behavior. `SWI` is a plain one-byte opcode (`$3F`), dispatched straight out
of `step`'s top-level match (`exec.rs:266`, inside `exec_interrupt_halt`,
quoted in §4.1). `SWI2`/`SWI3` are page-prefixed — `$10 3F` and `$11 3F` —
decoded one level down, inside the prefix-page handlers:

```rust
0x3F => { self.take_interrupt(bus, VECTOR_SWI2, false, false, true); 20 } // SWI2 — exec.rs:169, exec_page10
0x3F => { self.take_interrupt(bus, VECTOR_SWI3, false, false, true); 20 } // SWI3 — exec.rs:190, exec_page11
```

Same opcode byte (`$3F`), three different vectors, because the prefix
determines which `take_interrupt` call it routes to before the mask
arguments are even read. `SWI` costs 19 cycles and sets `I`+`F`; `SWI2`/
`SWI3` cost 20 (one more, the price of the prefix byte) and touch no masks
at all — verified in `swi2_and_swi3_do_not_touch_masks`, same test file.

---

## 4.3 Why NMI waits for a stack pointer

`NMI` (non-maskable interrupt) is supposed to mean "cannot be ignored, full
stop." And yet the very first thing `nmi()` does is check a flag that can
make it *return without doing anything*:

```rust
// crates/mc6809/src/lib.rs:191-198
/// Deliver a non-maskable interrupt: full frame, sets I+F. Ignored until
/// the first program load of S arms recognition (see `nmi_armed`).
pub fn nmi(&mut self, bus: &mut impl Bus) {
    if !self.nmi_armed {
        return;
    }
    self.take_interrupt(bus, VECTOR_NMI, true, true, true);
}
```

This isn't a design choice made in this codebase — it's a documented
property of the real chip. `RESET` doesn't initialize `S`; on real silicon
it comes up holding whatever garbage was in it at power-on. If an `NMI`
line happened to assert in that window — a real, physically possible
event — the CPU's first act would be pushing twelve bytes through a stack
pointer aimed at random RAM (or ROM, silently discarding the write and
losing the frame). Either way, the machine is dead before it starts.
Motorola's answer, baked into the silicon: `NMI` recognition doesn't arm
until software has explicitly loaded a real value into `S` at least once.

The struct field and its doc comment say all of this in one place:

```rust
// crates/mc6809/src/lib.rs:139-159
pub struct MC6809 {
    // ...
    pub cc: u8,
    /// Total cycles executed since reset (for scheduling/debugging).
    pub cycles: u64,
    /// Running vs halted (SYNC/CWAI).
    pub state: State,
    /// NMI is not recognized until the first program load of the stack
    /// pointer after reset (MC6809 datasheet) — before S is valid an NMI
    /// frame push would scribble through a garbage pointer. Set by any
    /// instruction that writes S (LDS, LEAS, TFR/EXG, indexed `,S++`-style
    /// writeback), cleared by reset.
    pub nmi_armed: bool,
}
```

"Any instruction that writes S" is a promise the codebase has to keep at
*every* place code can address `S` as a destination — four of them:

```rust
// crates/mc6809/src/lib.rs:185-189
/// Load the stack pointer from program action, arming NMI recognition.
fn load_s(&mut self, v: u16) {
    self.s = v;
    self.nmi_armed = true;
}
```

`LDS` in all four addressing-mode forms (`exec.rs:160-163`) calls `load_s`;
so does `LEAS` (`exec/exec_data.rs:105`); so does `TFR`/`EXG` targeting `S`
(`regs.rs:38`, inside `reg_write`); and so does the indexed-addressing auto
inc/dec form that names `S` as the pointer being written back
(`addressing.rs:41-47`, `set_index_reg`) — `,S++` and friends genuinely
mutate `S` through the postbyte, not just through `LDS`. Four call sites,
one arming flag, one clearing site (`reset`, which sets `nmi_armed = false`
alongside masking `I` and `F`). Add a new instruction that can write `S`
and this is the checklist: does it call `load_s`, or does it silently open
a fifth way to bypass the datasheet rule?

`reset()` itself is three lines and worth reading once, since it's the
starting state everything above assumes:

```rust
// crates/mc6809/src/lib.rs:176-183
/// RESET: DP=0, IRQ+FIRQ masked, PC loaded from the reset vector.
pub fn reset(&mut self, bus: &mut impl Bus) {
    self.dp = 0;
    self.cc |= cc::IRQ_MASK | cc::FIRQ_MASK;
    self.pc = bus.read_u16(VECTOR_RESET);
    self.state = State::Running;
    self.nmi_armed = false;
}
```

Every real ROM's cold-start code does an `LDS` within its first handful of
instructions for exactly this reason — not out of good style, but because
until it does, the hardware won't listen to `NMI` at all. §4.7's trace
exercise asks you to find that instruction in the real CoCo 3 ROM.

---

## 4.4 SYNC and CWAI: the CPU as a small state machine

Up to now, `step()` has been "fetch one opcode, execute it, return a cycle
count." `SYNC` and `CWAI` are where that stops being true: they turn the
CPU into a state machine with three states, only one of which is
"executing instructions."

```rust
// crates/mc6809/src/lib.rs:123-135
/// Execution state. The 6809 can halt itself waiting for an interrupt.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    #[default]
    Running,
    /// `SYNC`: halted until any interrupt line asserts; resumes with the next
    /// instruction (or services the interrupt if it is unmasked).
    Syncing,
    /// `CWAI`: the full register frame is already stacked; halted until an
    /// unmasked interrupt, which is then serviced without re-stacking.
    Waiting,
}
```

`step()`'s very first check is on this field, before it fetches anything:

```rust
// crates/mc6809/src/exec.rs:30-36
pub fn step(&mut self, bus: &mut impl Bus) -> u32 {
    if self.state != State::Running {
        // Halted by SYNC/CWAI: burn an idle cycle until the machine delivers
        // an interrupt (via nmi/irq/firq) that resumes execution.
        self.cycles += 1;
        return 1;
    }
    let opcode = self.fetch_u8(bus);
    // ...
```

While halted, `step()` degenerates to "advance the clock by one cycle and
do nothing else" — the machine keeps calling it every scanline (week 6),
but nothing moves until something outside calls `nmi()`, `irq()`, or
`firq()`.

**`SYNC`** (opcode `$13`, the one-liner in §4.1's `exec_interrupt_halt`
match) is the lighter of the two: it just flips the state to `Syncing` and
returns, 2 cycles. Its contract is "wake me on *any* interrupt activity,
serviced or not" — which is why the mask checks inside `irq()`/`firq()`
(quoted in full in §4.2) explicitly test `self.state == State::Syncing` and
flip it back to `Running` even on the branch where the line is masked and
`false` gets returned. A masked `IRQ` arriving during `SYNC` does *not* get
serviced — no frame is pushed, `PC` doesn't move to the vector — but it does
end the wait, and execution resumes with the instruction right after
`SYNC`. That's the classic "wait for any device to twitch the line, then
decide for yourself what to do" idiom real 6809 code uses when it wants to
poll cheaply without missing the moment something happens.

**`CWAI`** (opcode `$3C`, "clear, wait for interrupt", also in §4.1's match)
is heavier, and the weight is the point. It ANDs `CC` with an immediate mask
(typically clearing `I` and/or `F` to make itself interruptible), forces
`E=1`, and — critically — **stacks the full twelve-byte frame right then,
before any interrupt has even arrived.** That's the whole reason `CWAI`
exists instead of just `ANDCC` followed by `SYNC`: when the interrupt does
show up, `take_interrupt` (quoted in full in §4.2) can skip the push
entirely and go straight to vectoring, because the work is already done.
The load-bearing line is the guard around the whole stacking half of the
function:

```rust
if self.state != State::Waiting {
    // ... stack the frame (§4.2) ...
}
```

That single condition is the entire optimization: a `CWAI`'d CPU skips
straight to setting masks and loading `PC` — twelve bus writes' worth of
latency already spent by the time the interrupt line asserts, which is why
hard-real-time 6809 code (disk-controller ISRs, tight audio-DAC
bit-bangers) prefers `CWAI` over `ANDCC`-then-`SYNC` when it knows in
advance an interrupt is coming.

A subtler consequence: because the `entire`/mask-setting logic lives
*inside* that same branch, a `CWAI`'d CPU's `E` flag is never touched by
the eventual interrupt — it stays at the `1` `CWAI` set, regardless of
whether an `IRQ`, `NMI`, or a normally-partial-frame `FIRQ` is what wakes
it. That's correct: `CWAI` commits to the full frame *before* it knows
which line will fire, so `RTI` will later restore everything — even though
a bare `FIRQ` arriving mid-execution would have saved only three bytes.

---

## 4.5 The 6809 testing problem

Here is the uncomfortable fact this whole codebase's CPU-testing strategy
is built around: **there is no per-instruction conformance suite for the
6809.** [TomHarte/SingleStepTests](https://github.com/SingleStepTests) —
one JSON case per opcode per addressing mode, generated from real silicon,
the gold standard for validating a CPU core — covers the 6502 family, the
Z80, the 68000, the 8088. Not the 6800/6809 family. Write a 6502 emulator
and you can download ten thousand machine-generated cases and know,
mechanically, whether your core is byte-for-byte correct. Write a 6809
emulator and that safety net does not exist, and hand-verifying flag
behavior by eye, trusting your own arithmetic, is a trap: a subtle
rare-condition bug (`SBC` with carry-in in the one edge case you didn't
check) can sit dormant for months and corrupt a save state at the worst
moment.

`DESIGN.md` §5 lays out the three-legged reply to this, and this codebase
takes each leg seriously:

### Leg 1 — trace-diff against a reference emulator

Boot the exact same ROM from the exact same reset vector in this emulator
and in a reference implementation (XRoar or MAME — both have had 6809
cores hammered on for decades by a larger community than one codebase can
muster), and compare a per-instruction trace, register by register. **The
first line where the two traces disagree is the bug** — not "roughly
where," the *exact* instruction, since everything before was by definition
identical.

This only works if both sides produce a trace in the same shape, so this
codebase carries the machinery as first-class infrastructure. The trace
ring's entry type lives in the debugger core:

```rust
// crates/coco-core/src/debug.rs:136-150
/// One entry in the instruction trace ring: the CPU register file captured
/// immediately BEFORE an instruction retired. Formatted identically to
/// `examples/trace.rs`'s `log_state` for MAME trace-diffing.
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

held by the `Debugger` in a capped ring, `DEFAULT_TRACE_CAP = 1024` entries
by default:

```rust
// crates/coco-core/src/debug.rs:183-192, 333-338
pub struct Debugger {
    breakpoints: HashMap<u16, Breakpoint>,
    watchpoints: HashMap<u16, Watchpoint>,
    trace: VecDeque<TraceEntry>,
    trace_cap: usize,
    /// When true, record every retired instruction into the trace ring during
    /// [`Debugger::run_until`]. Off by default — the ring costs a snapshot per
    /// instruction, which a plain "run" doesn't want.
    pub trace_enabled: bool,
}

fn push_trace(&mut self, entry: TraceEntry) {
    if self.trace.len() == self.trace_cap {
        self.trace.pop_front();
    }
    self.trace.push_back(entry);
}
```

> **Rust corner: `VecDeque` as a ring buffer.** `push_trace` is the entire
> ring-buffer implementation: check the length against the cap, drop the
> oldest entry with `pop_front` if full, push the new one with `push_back`.
> `VecDeque<T>` is a growable double-ended queue backed by a circular
> buffer internally, so both operations are `O(1)` — there's no shifting
> of the other 1023 entries every time one drops off the front the way
> there would be with `Vec::remove(0)`. This is the standard-library type
> to reach for whenever you need "keep the last N things" anywhere in an
> emulator: audio sample history, recent-writes logs, anything with a
> natural expiry-by-age. `Vec` would work functionally but degrade to
> `O(n)` per eviction; `VecDeque` doesn't.

`examples/trace.rs` (71 lines) walks a real boot and emits one formatted
line per retired instruction in that same shape, via a `format()` method
shared with the live trace ring so there's exactly one format to keep in
sync:

```rust
// crates/coco-core/src/debug.rs:168-176
pub fn format(&self) -> String {
    format!(
        "{:04X}:  A={:02X} B={:02X} X={:04X} Y={:04X} U={:04X} S={:04X} DP={:02X} CC={:02X}",
        self.pc, self.a, self.b, self.x, self.y, self.u, self.s, self.dp, self.cc
    )
}
```

Its no-cart mode deliberately runs raw `m.step()` with **no interrupts, no
scanline timing, nothing but the CPU executing ROM** — isolating exactly
what you want to validate first (does the decoder produce the right
registers for the right bytes) before a second mode reintroduces
interrupts, hsync, and the GIME timer via `Machine::step_instruction()` for
a fuller diff against a specific MAME `-cart1` run.

### A worked example: reading a trace like a detective

Here is real output, from actually running `cargo run -p coco-core
--example trace -- 13` against `roms/coco3.rom` on this machine — the
first thirteen instructions the CoCo 3's real Super Extended Color BASIC
executes after reset. Each line is the register file *before* that PC's
instruction runs (per `TraceEntry`'s own doc comment, §4.5 above). I've
disassembled each PC by hand against the same ROM bytes (verified with
`mc6809::disasm::disassemble`, the week-3 disassembler, pointed at the
file) and annotated the trace with the mnemonic that runs *at* that line:

```
8C1B:  A=00 B=00 X=0000 Y=0000 U=0000 S=0000 DP=00 CC=50    ORCC #$50
8C1D:  A=00 B=00 X=0000 Y=0000 U=0000 S=0000 DP=00 CC=50    LDA  #$0A
8C1F:  A=0A B=00 X=0000 Y=0000 U=0000 S=0000 DP=00 CC=50    STA  $FF90
8C22:  A=0A B=00 X=0000 Y=0000 U=0000 S=0000 DP=00 CC=50    CLR  $FFDE
8C25:  A=0A B=00 X=0000 Y=0000 U=0000 S=0000 DP=00 CC=54    JMP  $C000
C000:  A=0A B=00 X=0000 Y=0000 U=0000 S=0000 DP=00 CC=54    ORCC #$50
C002:  A=0A B=00 X=0000 Y=0000 U=0000 S=0000 DP=00 CC=54    LDS  #$5EFF
C006:  A=0A B=00 X=0000 Y=0000 U=0000 S=5EFF DP=00 CC=50    LDA  #$12
C008:  A=12 B=00 X=0000 Y=0000 U=0000 S=5EFF DP=00 CC=50    LDB  #$10
C00A:  A=12 B=10 X=0000 Y=0000 U=0000 S=5EFF DP=00 CC=50    LDX  #$FFB0
C00D:  A=12 B=10 X=FFB0 Y=0000 U=0000 S=5EFF DP=00 CC=58    STA  ,X+
C00F:  A=12 B=10 X=FFB1 Y=0000 U=0000 S=5EFF DP=00 CC=50    DECB
C010:  A=12 B=0F X=FFB1 Y=0000 U=0000 S=5EFF DP=00 CC=50    BNE  $C00D
```

Read this the way you'd read any trace-diff: PC first, then register by
register, and cross-check every change against the instruction that just
ran. Three things pop straight out once you know what to look for:

- `ORCC #$50` at `$8C1B` sets exactly `cc::IRQ_MASK | cc::FIRQ_MASK`
  (`$10 | $40 = $50`) — the same two bits `reset()` sets (§4.3). BASIC's
  own cold-start code re-masks both interrupts on purpose, in software,
  right after the hardware reset already did it; the second `ORCC #$50`
  at `$C000` does it again after a `JMP`. Nothing here is CPU-crate
  behavior — it's the ROM being defensive — but you can only recognize
  that as *expected* because you already know what `reset()` does.
- `LDS #$5EFF` at `$C002` is the instruction that arms `nmi_armed` for
  the rest of the boot — the answer to exercise 4.5 below, from §4.3:
  watch `S` jump from `$0000` to `$5EFF` between the `$C002` line and the
  `$C006` line, which is `load_s()` firing.
- `CC` flips from `$50` to `$58` at `$C00D`, right after `LDX #$FFB0`
  executes — `$FFB0` as a signed 16-bit value has its top bit set, so
  `LDX` sets `N` (week 2's flag rules, still paying rent). Three lines
  later, once `STA ,X+` stores the *positive* byte `$12`, `N` clears
  again and `CC` drops back to `$50`. A flag that appears and disappears
  for one line is completely explained by the one instruction between —
  which is exactly the discipline trace-diffing rewards.

Now put yourself in the trace-diff seat for real. Suppose a MAME reference
trace of the same boot matched this output line for line up through
`$C00A`, and then at `$C00D` MAME showed `CC=50` where this trace shows
`CC=58` — everything else on that line identical. You would not need to
re-read the whole CPU core. The only instruction that ran between the two
identical `$C00A` states and the `$C00D` line is `LDX #$FFB0` — nothing
else touched a register in between — so the bug is narrowed, with zero
ambiguity, to LDX's flag computation for a negative 16-bit immediate.
That's the entire method: **find the first line where two otherwise-
identical traces disagree, and the previous instruction is guilty.** No
guessing which of a few thousand possible opcodes to suspect; the trace
itself names the defendant.

### Leg 2 — a self-checking exerciser ROM

The second leg is running an existing, independent test *program* — one
that doesn't know or care what emulator it's running on — and trusting its
verdict. `DESIGN.md` §5 names a concrete one:
[flexemu's `cputest.txt`](https://github.com/aladur/flexemu/blob/master/src/tools/cputest.txt)
by W. Schwotzer, a 6809 assembly program exercising the arithmetic/logic
instructions, `TFR`/`EXG`, and the full range of addressing modes, checking
its own results, verified by its author against a real SGS-Thomson EF6809P
chip. A clean pass on real hardware and a clean pass in your emulator is
strong, independent evidence your core matches silicon in the cases covered.

Worth being honest here: this is a *documented, recommended* strategy in
`DESIGN.md`, not yet an integration test wired into `cargo test` — there is
no `cputest.txt` in this repository's tree today. That's a real gap, and
exactly the kind this course wants you to notice rather than take on
faith. Assembling it, running it headless against `FlatBus`, and wiring
its pass/fail signal into a test would close leg 2 for real.

### Leg 3 — hand-written corner tests

The third leg you can inspect directly right now: `crates/mc6809/tests/
interrupts.rs`, `indexed.rs`, `stack.rs`, and friends — tests written by a
human who read the datasheet (or the reference PDFs in `docs/`) and
encoded known-tricky corners as assertions: indexed postbyte submodes
(week 3), `TFR`/`EXG` register encodings, `FIRQ`-vs-`IRQ` stacking,
`CWAI`/`SYNC`. These lack leg 1's breadth and leg 2's independence, but
they're fast, run in CI on every commit, and fail with a specific,
readable assertion instead of "somewhere in a million-instruction trace,
something differs." None of the three legs alone is enough — that's why
there are three.

Carry this to whatever CPU you emulate next, even one with a JSON suite:
(1) diff against a trusted reference from an identical starting state,
first divergence localizes the bug; (2) run independent, self-checking
test software when one exists; (3) hand-encode the corners you already
know are sharp. A JSON conformance suite, where it exists, is leg 2 at
industrial scale — it doesn't replace the other two.

---

## 4.6 Reading two real tests

Everything in §4.1 and §4.4 is provable, not assertable-on-faith — here are
two tests from `crates/mc6809/tests/interrupts.rs`, walked line by line:
one for the partial frame (§4.1), one for the state machine (§4.4).

### `firq_uses_partial_frame`

```rust
// crates/mc6809/tests/interrupts.rs:215-229
#[test]
fn firq_uses_partial_frame() {
    let mut s = Sys::new();
    s.bus.load(0xFFF6, &[0x70, 0x00]); // FIRQ vector -> $7000
    s.cpu.pc = 0x1234;
    s.cpu.s = 0x2000;
    s.cpu.cc = 0x00;
    let serviced = s.cpu.firq(&mut s.bus);
    assert!(serviced);
    assert_eq!(s.cpu.pc, 0x7000);
    assert_eq!(s.cpu.s, 0x2000 - 3); // CC + PC only
    assert_ne!(s.cpu.cc & cc::IRQ_MASK, 0); // FIRQ sets both masks
    assert_ne!(s.cpu.cc & cc::FIRQ_MASK, 0);
    assert_eq!(s.mem(0x1FFD), 0x00); // stacked CC has E clear (partial frame)
}
```

Nothing here calls `step()` — this test drives `firq()` directly, the point
of a `Bus`-generic, machine-agnostic CPU: you can call the
external-interrupt API in complete isolation, no opcode fetch, no machine,
just the two entry points (`firq`/`bus`) the real `SystemBus` eventually
calls from `run.rs`. `s.bus.load(0xFFF6, ...)` plants the `FIRQ` vector's
target directly at the vector address `take_interrupt`'s
`bus.read_u16(vector)` will read. Five separate facts checked after one
call — vector taken, exactly 3 bytes of stack moved, both masks up, and the
one byte that *did* get pushed (`CC`, at `$1FFD` per §4.1's table) has `E`
clear — is a complete, self-contained specification of "partial frame."

### `cwai_stacks_frame_then_interrupt_skips_restacking`

This one tests the §4.4 interaction from the other direction — that
`CWAI`'s pre-stacked frame is *not* re-stacked when the interrupt that
wakes it finally arrives:

```rust
// crates/mc6809/tests/interrupts.rs:286-306
#[test]
fn cwai_stacks_frame_then_interrupt_skips_restacking() {
    let mut s = Sys::code(0x1000, &[0x3C, 0xEF]); // CWAI #$EF (clears I mask)
    s.bus.load(0xFFF8, &[0x80, 0x00]); // IRQ vector
    s.cpu.s = 0x2000;
    s.cpu.cc = 0xFF; // everything set
    let cycles = s.step();
    assert_eq!(cycles, 22);
    assert_eq!(s.cpu.state, State::Waiting);
    assert_eq!(s.cpu.cc & cc::IRQ_MASK, 0); // ANDed with $EF cleared I
    let s_after_cwai = s.cpu.s;
    assert_eq!(s_after_cwai, 0x2000 - 12); // full frame stacked once

    // IRQ now allowed (I clear). Because CWAI already stacked, the interrupt
    // must NOT push a second frame.
    let serviced = s.cpu.irq(&mut s.bus);
    assert!(serviced);
    assert_eq!(s.cpu.s, s_after_cwai); // stack pointer unchanged — no re-stack
    assert_eq!(s.cpu.pc, 0x8000);
    assert_eq!(s.cpu.state, State::Running);
}
```

Read it in two halves, because it exercises `step()` *and* the external
API in sequence. The first half runs `CWAI #$EF` through `s.step()` like
any ordinary instruction — starting `cc = 0xFF` (every bit set, including
both masks) makes the ANDCC-style clearing visible: `cc & IRQ_MASK == 0`
afterward proves the `#$EF` operand (`1110_1111`, `I`'s bit zeroed) did its
job. `s_after_cwai` captures the stack pointer right there, twelve bytes
below where it started — the full frame, stacked before any interrupt
exists. `cycles == 22` matches the code's hard-coded return value from
§4.1's `exec_interrupt_halt` block. The CPU is now sitting in
`State::Waiting`, and the *next* `s.step()` (which this test never calls)
would just burn a cycle per §4.4 — nothing happens until something calls
`irq()`/`firq()`/`nmi()` from outside.

The second half is the load-bearing part: calling `irq()` directly (`I`
is clear now, so it's serviced) and asserting `s.cpu.s == s_after_cwai` —
*unchanged*. If `take_interrupt`'s `if self.state != State::Waiting` guard
were ever deleted, `S` would drop by another 12 (a second frame stacked on
top of the first) and this single equality would catch it immediately,
with a concrete, readable number mismatch — exactly what exercise 4.7
below asks you to go verify by actually deleting it.

---

## 4.7 Reading assignment

1. **`crates/mc6809/src/lib.rs`, lines 110–260** — vector constants, `State`
   enum, `nmi_armed`'s doc comment, `reset()`, `load_s()`,
   `nmi()`/`irq()`/`firq()`, `take_interrupt()`: the entire interrupt
   subsystem in one contiguous read.
2. **`crates/mc6809/src/exec.rs`** — `exec_interrupt_halt`
   (`SWI`/`RTI`/`CWAI`/`SYNC`) and the `SWI2`/`SWI3` arms inside
   `exec_page10`/`exec_page11`.
3. **`crates/mc6809/src/stack.rs`**, all 76 lines — `psh`/`pul` back every
   frame here, plus `PSHS`/`PULS` from week 3.
4. **`crates/mc6809/tests/interrupts.rs`**, all of it — every test in it is
   a claim this chapter makes, turned into an assertion.
5. **`DESIGN.md` §5**, the testing-strategy paragraphs.
6. **`crates/coco-core/src/debug.rs`**, around line 186 (`Debugger` and its
   `trace` field) plus `TraceEntry` and `export_trace`.
7. **`crates/coco-core/examples/trace.rs`**, the whole file (71 lines).

Run the interrupt tests and watch every claim in this chapter come back
green:

```
cargo test -p mc6809 --test interrupts
```

---

## 4.8 Exercises

**4.1 — Draw both frames (recall + draw).** Suppose `S = $3000` the instant
before (a) an `IRQ` fires and (b) an `FIRQ` fires (`CC = $00` in both cases).
Draw a table like §4.1's — every address from the new `S` up to
`$2FFF`/`$3000` and what's stacked there — and state the new `S` in both
cases. Check against what `swi_stacks_full_frame_and_vectors` and
`firq_uses_partial_frame` assert for the `$2000` case; your `$3000` table
should be the same pattern shifted by `$1000`.

**4.2 — SYNC with a masked interrupt (read + verify).** Predict, before
looking anything up: a 6809 executes `SYNC` with `CC`'s `I` bit set, then an
`IRQ` line asserts. Does the CPU (a) stay halted forever, (b) service the
interrupt anyway, or (c) wake up and resume at the instruction after
`SYNC` without servicing it? Write down your answer, read `irq()`
(`lib.rs:200-211`) to check it, then run
`sync_halts_and_idles_until_interrupt` to see it asserted live.

**4.3 — Sabotage the E flag (sabotage, then verify for real).** In
`crates/mc6809/src/lib.rs`, change `firq()`'s call — `self.take_interrupt(bus,
VECTOR_FIRQ, true, true, false)` — so the last argument is `true` (make
`FIRQ` claim the full frame). Predict which test(s) in `interrupts.rs` fail
and why, *before* running `cargo test -p mc6809 --test interrupts`. Then run
it and compare. (One test fails, on a specific stack-pointer arithmetic
mismatch, not a vague "frame is wrong" — connect it back to §4.1's byte
tables. Revert the change afterward.)

**4.4 — A frame that doesn't double-stack (build).** Write a new test,
modeled on `cwai_stacks_frame_then_interrupt_skips_restacking`, but wake the
`CWAI`'d CPU with `nmi()` instead of `irq()` (remember `nmi_armed` must be
`true` first, or use the reset-then-`LDS` pattern from
`nmi_is_ignored_until_the_first_program_load_of_s`). Assert that (a) `S`
doesn't move a second time, and (b) the stacked `CC` byte still has `E=1`
even though this wake-up path would normally mean a partial frame — the
subtlety at the end of §4.4, proved with an assertion instead of prose.

**4.5 — Extend the worked trace (read + annotate).** From the repo root
(with `roms/coco3.rom` present), run `cargo run -p coco-core --example
trace -- 60` and pick up where §4.5's worked example leaves off, at
`$C010`. First, reproduce §4.5's claim yourself: confirm `S` really does
jump from `$0000` to `$5EFF` at `$C002`/`$C006` and that this is the `LDS`
that arms `nmi_armed` (§4.3) — don't take the chapter's word for it. Then
go further: the `STA ,X+` / `DECB` / `BNE $C00D` loop repeats until `B`
reaches zero. Find the PC where control finally falls out of that loop,
and identify — by reading the byte(s) at that PC in `roms/coco3.rom`, or
disassembling by hand — what instruction runs next. Does anything else
touch `S` before the trace ends?

**4.6 — SWI2 vs SWI (recall, three sentences max).** Why does it make
sense for `SWI` to set both `I` and `F` on entry while `SWI2`/`SWI3` set
neither? What class of code (what `SWI2` is typically used *for* on a real
CoCo — an operating system's syscall vector) benefits from an exception
that leaves the caller's interrupt posture completely alone?

**4.7 — Sabotage the CWAI fast path (sabotage, then verify for real).** In
`take_interrupt` (`crates/mc6809/src/lib.rs`), delete the
`if self.state != State::Waiting { ... }` guard around the frame-stacking
half of the function, but keep its *body* — i.e. always stack the frame,
unconditionally, on every call. Predict which test in `interrupts.rs` fails
and by how much `S` is off, *before* running `cargo test -p mc6809 --test
interrupts`. Then verify. (This is a different sabotage from exercise 4.3:
that one broke *which* frame shape gets stacked; this one breaks *whether a
second frame gets stacked at all* on top of a `CWAI`'d one. Revert
afterward and confirm `cargo test -p mc6809` is clean again.)

**4.8 — Fill in the call-site table (build + recall).** §4.2 lists all six
`take_interrupt` call sites with their `set_i`/`set_f`/`entire` arguments.
Starting from `cc = $00`, hand-compute the resulting `cc` (just the `E`,
`I`, `F` bits) for each of the six exceptions firing in isolation, then
write a tiny test for the two rows §4.2 doesn't already show a full test
for (pick `NMI` and one of `SWI2`/`SWI3`) asserting your predicted `cc`
value directly, the way `swi_stacks_full_frame_and_vectors` does for
`SWI`.

---

## What's next

Week 5 leaves the CPU crate behind — `mc6809` mostly just sits there,
correct, generic over whatever `Bus` you hand it. We open
`coco-core/src/bus.rs` and ask the question this chapter's vector table
quietly assumed an answer to: when the CPU reads `$FFFE`, what actually
intercepts that read before it becomes a plain RAM access? That's the
decode order — hardwired vectors first, then the I/O page, then ROM, then
the GIME's MMU — and the first time the CoCo 1/2's SAM and the CoCo 3's
GIME visibly diverge into two different address-translation paths.
