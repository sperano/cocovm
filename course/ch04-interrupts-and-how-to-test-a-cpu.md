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
addresses, taken directly from [`tests/interrupts.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/interrupts.rs)'s own setup (`s.cpu.s =
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
> inner helper, from [`crates/mc6809/src/stack.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/stack.rs):
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
decision ([`crates/coco-core/src/machine/run.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs)'s `service_interrupts`
checks `FIRQ` before `IRQ`, `NMI` is polled separately ahead of both — week
6). The CPU crate expresses masking, a per-line property encoded in `CC`;
priority, an ordering property, is the bus's job, because only the bus
knows which devices are asserting which lines at all.

### SWI vs SWI2 vs SWI3

All three software interrupts push the full frame — there's no "fast `SWI`"
the way there's a fast `IRQ`. What differs is prefix bytes and mask
behavior. `SWI` is a plain one-byte opcode (`$3F`), dispatched straight out
of `step`'s top-level match ([`exec.rs:266`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L266), inside `exec_interrupt_halt`,
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

`LDS` in all four addressing-mode forms ([`exec.rs:160-163`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L160-L163)) calls `load_s`;
so does `LEAS` ([`exec/exec_data.rs:105`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec/exec_data.rs#L105)); so does `TFR`/`EXG` targeting `S`
([`regs.rs:38`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/regs.rs#L38), inside `reg_write`); and so does the indexed-addressing auto
inc/dec form that names `S` as the pointer being written back
([`addressing.rs:41-47`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L41-L47), `set_index_reg`) — `,S++` and friends genuinely
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

### Interrupt latency: what it costs to get there

"CWAI is faster" has been an assertion so far. Here is what it would take
to put a real number on it — and an honest limit on how far that number can
be pushed with only this codebase as a source.

The one piece of hard currency the code gives you is `PUSH_PULL_BASE_CYCLES
= 5` ([`lib.rs:108`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L108)), the constant `psh`/`pul` add to their per-byte cost.
Run it against the full frame: `5 + 12 bytes = 17`. Compare that to `SWI`'s
hard-coded, datasheet-matching total of **19** cycles (§4.1). The two
numbers are close but not equal — `SWI` also has to fetch its own opcode
and read two bytes from the vector, and the real 6809's cycle-by-cycle bus
timing doesn't decompose into a clean "base + bytes" sum the way `psh`'s
*model* of PSHS/PULS cost does. That two-cycle gap is real interrupt
overhead (roughly: the vector fetch), it's just not one this codebase's own
formula predicts exactly.

Run the same comparison the other direction and the gap doesn't even point
the same way: `pul`'s formula also predicts `5 + 12 = 17` for unwinding a
full frame, but `RTI`'s hard-coded full-frame cost is **15** — two cycles
*under* the formula, not over. Push and pull aren't mirror images of each
other in the real chip's timing, and neither one is a pure function of
`PUSH_PULL_BASE_CYCLES`. The lesson generalizes past this chapter: **cycle
counts are empirical facts about specific silicon, not something you can
always derive from a clean formula.** `psh`/`pul`'s formula is a *model*,
accurate for the explicit `PSHS`/`PULS`/`PSHU`/`PULU` opcodes it was built
to cost (week 3) — it was never claimed to explain every stack-touching
operation on the chip, and `SWI`/`RTI`'s own hard-coded literals are the
tell: whoever wrote `exec_interrupt_halt` knew the formula wouldn't
reproduce these numbers, so they didn't try.

With that caveat on the table, the *shape* of the comparison still holds up
and is worth stating plainly:

- **`IRQ`/`NMI`** (full frame, same `psh(bus, 0xFF, true)` call `SWI` makes)
  sit in the same neighborhood as `SWI`'s 19 — same push, same vector fetch,
  no opcode byte to fetch (there's no opcode; a hardware line doesn't get
  decoded), so if anything the real number is a shade *under* 19, not over.
- **`FIRQ`** (partial frame, three bytes instead of twelve) is
  structurally the same operation with a ninth of the bytes moved — cheap
  by comparison, plausibly under half of `IRQ`'s cost. The whole reason the
  6809 has two hardware interrupt lines instead of one is exactly this gap.
- **A `CWAI`'d CPU waking up** pays neither of those costs *again*. Look at
  what's left inside `take_interrupt` once the `if self.state !=
  State::Waiting` guard is skipped: two `if`-gated `|=` operations on `CC`
  and a single `bus.read_u16(vector)`. That's a small, fixed amount of work
  — not scaled by frame size at all, because the frame-sized part already
  happened during `CWAI`'s own (already-counted) 22 cycles. This is the
  concrete version of §4.4's "twelve bus writes already spent" claim: the
  *shape* of the cost — flat, tiny, independent of which line eventually
  fires — not just its rough size, is what makes `CWAI` the right tool for
  code that already knows an interrupt is imminent.

Here is the honest headline, though, and it's a bigger deal than any of the
above estimates: **none of this is actually costed.** Search `take_interrupt`,
`nmi`, `irq`, and `firq` for any write to `self.cycles` and you will not
find one — `self.cycles` is touched in exactly two places in the entire
crate, both inside `exec.rs`'s `step()`:

```rust
// crates/mc6809/src/exec.rs:31-36, 119
if self.state != State::Running {
    self.cycles += 1;
    return 1;
}
// ...
self.cycles += cycles as u64;
```

The first line is the halt-state idle tick (§4.4's `step()` excerpt); the
second is `step()`'s own bookkeeping after dispatching an opcode — which is
exactly how `SWI`/`SWI2`/`SWI3`/`RTI`/`CWAI`/`SYNC` get their hard-coded
costs onto the clock, because those six are *opcodes*, decoded and executed
through `step()` like any other instruction. `nmi()`, `irq()`, and `firq()`
are not opcodes — they're public methods the machine calls directly from
outside `step()` ([`crates/coco-core/src/machine/run.rs:111,178,181`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs), week
6), and `take_interrupt`'s body — the function every one of them shares —
never once mentions `self.cycles`. `psh`'s own return value, the one thing
in this whole file that *does* compute a byte-accurate cost, is discarded
every time `take_interrupt` calls it (`self.psh(bus, 0xFF, true);` — no
assignment, no `+=`, nothing). An externally-delivered interrupt is, as far
as this CPU crate's clock is concerned, free.

Is that a bug? Not obviously — it's the same policy [`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5 states
outright for the rest of the core ("don't try to be cycle-*exact*
mid-instruction at first; instruction-granular cycle counts are enough").
Nothing in this codebase currently needs sub-instruction interrupt latency:
the CoCo's timing-sensitive software mostly cares about *which scanline* a
handler runs on (week 6's coarser granularity), not whether the handler's
first instruction lands 10 or 19 cycles after the line asserted. But it is
a real, specific gap, worth knowing exactly where it lives rather than
assuming a `cycles` field on a struct called `MC6809` accounts for
everything that happens to that CPU. If you ever need to trace-diff against
a reference emulator that *does* cost this (real MAME does), this is
precisely where the two traces would start disagreeing on cycle counts —
even while agreeing on every register value.

### FIRQ inside an IRQ handler: nesting and the E flag

One more question `take_interrupt`'s design answers, if you trace it
through carefully: what happens when a second interrupt line asserts
*while the CPU is already running a handler for the first one* — before
that handler's own `RTI`?

Look back at §4.2's mask table with this question in mind. `irq()` calls
`take_interrupt(bus, VECTOR_IRQ, true, false, true)` — `set_f` is `false`.
That's not an oversight; it's the whole answer. An `IRQ` handler runs with
`I` set (so a second `IRQ` can't preempt it) but `F` **untouched** — if `F`
was clear before the `IRQ` fired, it is still clear once the handler starts
running. A `FIRQ` line asserting at that moment sails straight through
`firq()`'s mask check (`self.cc & cc::FIRQ_MASK != 0` is false) exactly as
if no interrupt were already in progress. `take_interrupt` doesn't inspect
what's already on the stack — it has no way to, and no need to — so it
simply pushes FIRQ's three-byte partial frame *on top of* IRQ's already-
stacked twelve-byte frame, at whatever `S` currently is, and vectors to the
`FIRQ` handler.

Run this forward with real numbers (`S` starting at `$3000`, `CC` starting
at `$00`, both lines unmasked): `IRQ` fires, stacks the full frame, `S`
becomes `$2FF4`, `CC` becomes `I=1,F=0,E=1`. While the `IRQ` handler is
running with those masks, `FIRQ` fires: it is serviced (unmasked), stacks
its partial frame *on top* — `S` becomes `$2FF1`, `CC` becomes
`I=1,F=1,E=0`. The stack now holds two complete, independently-shaped
frames, one nested inside the other, and nothing about `take_interrupt`
had to know that. When the `FIRQ` handler finishes and executes `RTI`, it
pulls `CC` first — sees `E=0` — and pulls just `PC`: three bytes total,
`S` back to `$2FF4`, `PC` back to wherever the `IRQ` handler was executing,
and — this is the part worth sitting with — `CC` restored to exactly
`I=1,F=0`, the state that was true the instant `FIRQ` preempted. Control
resumes *inside* the `IRQ` handler, `FIRQ` is unmasked again, and that
handler's own eventual `RTI` unwinds its full frame the normal way,
`E=1`, twelve bytes, back to whatever was interrupted in the first place.
Nesting works cleanly, to arbitrary depth (bounded only by masks and stack
space), for exactly the reason `RTI` only ever looks at the byte on *top*
of the stack — never anything deeper — and each `take_interrupt` call is
self-contained about the frame it produces.

The reverse direction is blocked, and now you can see precisely why: a
`FIRQ` handler runs with **both** `I` and `F` set (`firq()` passes
`set_i: true, set_f: true`), so an `IRQ` line asserting mid-`FIRQ`-handler
hits `irq()`'s own mask check and that call simply returns `false` —
unserviced, exactly as if `I` had been set by any other means. Whether
that line gets tried again is a bus-level question (week 6), not
something `irq()` itself tracks; from the CPU's side, `FIRQ` can always
preempt `IRQ`, and `IRQ` can never preempt `FIRQ` unless the `FIRQ`
handler explicitly clears `I` itself before its `RTI`. That
asymmetry is the entire point of calling one of them "fast." `NMI` sits
above both: `nmi()` checks no mask bit at all before calling
`take_interrupt` — only `nmi_armed` gates it (§4.3) — so it can interrupt
`IRQ` handlers, `FIRQ` handlers, or anything else, non-maskable exactly as
advertised. Exercise 4.9 asks you to write the test that proves the `IRQ`
nested-`FIRQ` sequence above, with the exact register values, rather than
trust this paragraph.

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

[`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5 lays out the three-legged reply to this, and this codebase
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

[`examples/trace.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/trace.rs) (71 lines) walks a real boot and emits one formatted
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

### Following the ROM deeper, against a real disassembly

The thirteen lines above aren't a guess at what the ROM does — they were
cross-checked, mnemonic for mnemonic, against `docs/super-extended-basic-
unravelled.pdf`'s Appendix B, a hand-annotated disassembly of this exact
ROM by Walter K. Zydhek (`docs/` is local to this machine, gitignored,
copyrighted — not required reading, but every label and comment quoted
below is reproduced verbatim so you don't need the PDF to follow along).
That reference gives real labels for the code this chapter has been
tracing blind:

```
       C000 1A 50           LC000    ORCC #$50           DISABLE IRQ, FIRQ INTERRUPTS
       C002 10 CE 5E FF              LDS   #$5EFF        INITIALIZE STACK POINTER
       C006 86 12                    LDA   #$12          PALETTE COLOR: COMPOSITE-GREEN, RGB-INDIGO
                            * INITIALIZE ALL PALETTE REGISTERS TO GREEN (COMPOSITE)
       C008 C6 10                    LDB   #16           16 PALETTE REGISTERS
       C00A 8E FF B0                 LDX   #PALETREG     POINT X TO THE PALETTE REGISTERS
       C00D A7 80           LC00D    STA   ,X+           SAVE THE COLOR IN THE PALETTE REGISTER
       C00F 5A                       DECB                BUMP COUNTER
       C010 26 FB                    BNE   LC00D         LOOP UNTIL ALL PALETTE REGISTERS DONE
```

Every byte, every mnemonic, every branch target matches what §4.5's trace
already showed — and now you know *why* `B` starts at `$10` (SEB Unravelled
just writes it in decimal, "16 PALETTE REGISTERS") and what `$FFB0` is
called (`PALETREG`). What those sixteen registers actually configure is a
week-8 story (the GIME palette); for this chapter, the label is enough to
confirm the trace is reading the ROM correctly, which is the whole point of
cross-checking against an independent source.

The loop this chapter already traced runs all sixteen times before
falling through to `$C012` — the ROM's own comment even numbers this as
step 1 of its initialization ("Clear the Screen... by storing `$12`s in
all of the palette registers"). Extend the trace past `$C010` (exercise 4.5
asks you to do exactly this) and the *same shape* of loop repeats
immediately, over a different sixteen-entry register block:

```
       C012 8E FF A0                 LDX   #MMUREG       POINT X TO THE MMU REGISTERS
       C015 31 8D 02 2D              LEAY  MMUIMAGE,PC   POINT Y TO THE MMU REGISTER IMAGES
       C019 C6 10                    LDB   #16           16 MMU REGISTERS
       C01B A6 A0           LC01B    LDA   ,Y+           GET A BYTE FROM THE IMAGE
       C01D A7 80                    STA   ,X+           SAVE IT IN THE MMU REGISTER
       C01F 5A                       DECB                BUMP COUNTER
       C020 26 F9                    BNE   LC01B         LOOP UNTIL DONE
```

Real trace output for the setup and first pass through that second loop —
same technique as before (`cargo run -p coco-core --example trace`,
register file per line, mnemonic column hand-annotated against
`mc6809::disasm::disassemble`, not part of `trace.rs`'s own output), this
time started past the palette loop:

```
C012:  A=12 B=00 X=FFC0 Y=0000 U=0000 S=5EFF DP=00 CC=54    LDX  #$FFA0
C015:  A=12 B=00 X=FFA0 Y=0000 U=0000 S=5EFF DP=00 CC=58    LEAY 557,PCR
C019:  A=12 B=00 X=FFA0 Y=C246 U=0000 S=5EFF DP=00 CC=58    LDB  #$10
C01B:  A=12 B=10 X=FFA0 Y=C246 U=0000 S=5EFF DP=00 CC=50    LDA  ,Y+
C01D:  A=38 B=10 X=FFA0 Y=C247 U=0000 S=5EFF DP=00 CC=50    STA  ,X+
C01F:  A=38 B=10 X=FFA1 Y=C247 U=0000 S=5EFF DP=00 CC=50    DECB
C020:  A=38 B=0F X=FFA1 Y=C247 U=0000 S=5EFF DP=00 CC=50    BNE  $C01B
```

Read the register motion the same way as before, with one wrinkle: this
codebase's own disassembler renders `LEAY`'s indexed operand as a raw
signed offset (`557,PCR`, week 3's PC-relative addressing) rather than a
resolved target address — SEB Unravelled's listing resolves the same
instruction to `MMUIMAGE,PC` because a human annotator did the arithmetic
by hand. Both are correct; they're just different jobs. Do the arithmetic
yourself and they agree: `$C019` (the address right after this 4-byte
instruction) `+ 557 ($22D) = $C246`, exactly the `Y` value the trace shows
at the next line, exactly SEB's `MMUIMAGE` table address. `LDA ,Y+` at
`$C01B` pulls one byte from that table (`A` becomes `$38`, the table's
first entry) and `STA ,X+` writes it to `$FFA0`, the first of sixteen MMU
registers — what those registers actually *do* is week 5's MMU story, not
this chapter's, but you can already see the pattern is identical to the
palette loop: read a value, store it, decrement, branch. Trace-diffing
doesn't care that the semantics differ; the mechanical shape of "loop over
sixteen registers" is something you'd recognize on sight in a diff, MMU or
palette.

The ROM doesn't stop there. Right after the MMU loop, one more instruction
re-enables CoCo-compatible addressing and the MMU itself, and then the
code sets up for something bigger:

```
       C022 86 CE                    LDA   #COCO+MMUEN+MC3+MC2+MC1        ENABLE COCO COMPATIBLE MODE; ENABLE MMU
       C024 B7 FF 90                 STA   INIT0                          AND TURN ON THE NORMAL SPARE CHIP SELECT
                            * MOVE THE INITIALIZATION CODE FROM ROM TO RAM($4000); THIS IS DONE IN
                            * PREPARATION FOR MOVING BASIC FROM ROM TO RAM.
       C027 30 8D 00 14              LEAX  BEGMOVE,PC                     POINT TO START OF ROM CODE
       C02B 10 8E 40 00              LDY   #$4000                         RAM LOAD ADDRESS
       C02F EC 81           LC02F    LDD   ,X++                           GRAB TWO BYTES
       C031 EE 81                    LDU   ,X++                           GRAB TWO MORE BYTES
       C033 ED A1                    STD   ,Y++                           MOVE FIRST SET OF BYTES
       C035 EF A1                    STU   ,Y++                           AND THEN THE SECOND
       C037 8C C3 6C                 CMPX  #ENDMOVE                       ARE ALL BYTES MOVED?
       C03A 25 F3                    BCS   LC02F                          KEEP GOING UNTIL DONE
       C03C 7E 40 00                 JMP   L4000                          JUMP INTO THE MOVED CODE
```

(`COCO+MMUEN+MC3+MC2+MC1` is SEB Unravelled's own symbolic sum for the
`$CE` byte the ROM actually loads — five named GIME configuration bits
this chapter isn't unpacking; that's week 5's `INIT0` register story, not
this one's.)

The real trace confirms the setup lands exactly where the listing says it
should — `X = $C03F` (`BEGMOVE`), `Y = $4000` — right before the copy loop
begins:

```
C027:  A=CE B=00 X=FFB0 Y=C256 U=0000 S=5EFF DP=00 CC=58    LEAX 20,PCR
C02B:  A=CE B=00 X=C03F Y=C256 U=0000 S=5EFF DP=00 CC=58    LDY  #$4000
C02F:  A=CE B=00 X=C03F Y=4000 U=0000 S=5EFF DP=00 CC=50    LDD  ,X++
```

Same wrinkle as before: `LEAX 20,PCR`'s disassembled operand is the raw
offset; `$C02B + 20 ($14) = $C03F`, which is exactly the `X` the very next
trace line shows, and exactly `BEGMOVE`.

This is a natural place to stop tracing, and it's worth being explicit
about *why* rather than just running out of room: `ENDMOVE` and `BEGMOVE`
are both labels with fixed addresses in the same listing — `ENDMOVE =
$C36C`, `BEGMOVE = $C03F` — so the amount of code this loop relocates is
computable without stepping through it at all: `$C36C - $C03F = $32D =
813` bytes, moved four bytes per iteration — 813 isn't a multiple of 4, so
the loop overshoots slightly on its last pass, landing on 204 trips around
a six-instruction loop, not 203 — well over a thousand trace lines to reach
the `JMP $4000` at `$C03C`, all of it the identical `LDD`/`LDU`/`STD`/
`STU`/`CMPX`/`BCS` shape already shown in one iteration above. A real
trace-diff session wouldn't read those thousand lines by eye either — you'd
script the diff, or set a breakpoint past the loop and only compare state
from there, the same instinct that makes `$C03C`, not `$C020`, the honest
edge of what an "extended worked example" should reproduce line-by-line in
a book.

One interrupt-relevant fact survives the whole trip, worth naming before
moving on: every `CC` value shown from `$8C1B` through `$C02F` — `$50`,
`$54`, `$58`, `$59`, whatever transient `N`/`Z`/`C` bits ride along —
keeps bits `$50` (`cc::IRQ_MASK | cc::FIRQ_MASK`) set throughout. Nothing
in this trace — not the palette loop, not the MMU loop, not the code
relocation — is interruptible. `ORCC #$50` at `$8C1B` and again at `$C000`
(the worked example above) is still in force at every single line quoted
in this section, and it stays that way until BASIC's own initialization
explicitly decides otherwise, well past where this chapter's trace stops.
That's not a coincidence you need to verify against a PDF — it follows
directly from §4.2's `irq()`/`firq()`, which never clear a mask on their
own, and the fact that nothing in this stretch of code executes `ANDCC`.

### Leg 2 — a self-checking exerciser ROM

The second leg is running an existing, independent test *program* — one
that doesn't know or care what emulator it's running on — and trusting its
verdict. [`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5 names a concrete one:
[flexemu's `cputest.txt`](https://github.com/aladur/flexemu/blob/master/src/tools/cputest.txt)
by W. Schwotzer, tested on a real SGS-Thomson EF6809P processor. It's
worth looking at what's actually in that file rather than trusting the
one-line description — fifty-eight `JSR`s deep, its header reads:

```
*
*  MC6809 CPU Emulation Validation
*
* Tested on an SGS Thomson EF6809P Processor
*
* W. Schwotzer                     20.07.2003
*
```

The first block of subroutine calls exercises addressing modes
specifically — twenty-one of them, one `JSR` per mode, and the labels
carrying the exact syntax under test tell you precisely what's covered
without reading a single line of implementation: `LDA n8,X`, `LDA ,X+`,
`LDD ,X++`, `LDA ,-X`, `LDD ,--X`, `LDA A,X`, `LDA B,Y`, `LDD D,X`,
`LDD n16,X`, `LDA n8,PC`, `LDD n16,PC`, `LEAX [n8,X]`, `LEAX [,X++]`,
`LEAX [,--X]`, `LEAX [A,X]`, `LEAX [B,Y]`, `LEAX [D,X]`, `LEAX [n16,X]`,
`LEAX [n8,PCR]`, `LEAX [n16,PCR]`, `LEAX [addr]` — every indexed-postbyte
submode week 3 called "the single hardest 200 lines in the CPU," each one
independently checked against a real chip's answer, not just this one's
opinion of what the datasheet means. The second block, thirty routines
long, is instruction-family coverage, in the file's own order: `TNEG`,
`TCOM`, `TDEC`, `TINC`, `TCLR`, `TADD`, `TADDD`, `TADC`, `TMUL`, `TSEX`,
`TSUB`, `TSUBD`, `TSBC`, `TDAA`, `TCMP`, `TCMPD`, `TTST`, `TBIT`, `TLSR`,
`TLSL`, `TASR`, `TROL`, `TROR`, `TLD`, `TST`, `TLDD`, `TSTD`, `TLEA`,
`TTFR`, `TEXG` — `TFR`'s register-pair encodings (week 3's other hard
corner) get their own dedicated routine, `TTFR`, not folded into the
addressing-mode sweep.

Every failing check funnels through one routine, and its comment is worth
quoting because it's the entire self-checking design in four lines:

```
**************************************************
* Print error message for a failed Test
* Parameters:
* U: Pointer to Mnemonic
**************************************************
OUTERR LDA   #1
       STA   ERRFLG
       LDX   #ERRM1
       JSR   PSTRNG
       TFR   U,X
       JSR   PSTRNG
       RTS
```

`ERRFLG` — one byte, initialized to `0` — is the entire verdict. Every
individual test routine that finds a wrong answer sets it to `1` and calls
`OUTERR` with `U` pointing at that test's own mnemonic string, so a human
running it under FLEX (the 6809 development OS this file targets — more
on that below) sees exactly which instruction failed, not just that
*something* did. At the very end, `OUTSUC` checks the same byte: if it's
still `0`, "All Tests succeded" prints (a real typo in a twenty-year-old
file, reproduced here verbatim on purpose — this is what an actual
artifact looks like, not a tidied-up textbook example).

**How you would actually wire this in — and the one real obstacle.**
`cputest.txt` is written for FLEX, not for a bare `FlatBus`: its header
defines `WARMS EQU $CD03`, `PUTCHR EQU $CD18`, `PSTRNG EQU $CD1E`,
`PCRLF EQU $CD24`, `OUTDEC EQU $CD39` — five fixed addresses where FLEX's
own ROM provides character/string/decimal output and a "return to the
monitor" entry point. `OUTERR` above calls straight into one of them
(`PSTRNG`). `FlatBus` is a bare 64K array; nothing lives at `$CD18`, so
running this file's raw machine code against it today would `JSR` into
uninitialized memory and immediately go off into the weeds. That gap —
not test coverage, not addressing modes, just "this file assumes an
operating system this crate doesn't have" — is the one real obstacle
between [`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md)'s recommendation and an actual `cargo test`.

It's a small gap to close, though, and precisely because the CPU crate
already has everything needed: a `Bus` is just `read`/`write`, and a test
harness controls both sides. The sketch: assemble `cputest.txt` (any 6809
cross-assembler that accepts the FLEX-flavored syntax) into a flat binary,
load it into `FlatBus` at `$8100` (its own `ORG`), and instead of teaching
`FlatBus` to *be* FLEX, patch the five vector addresses in RAM with a
one-byte marker opcode `step()` doesn't otherwise produce — then drive the
loop from Rust: call `s.step()` in a loop; after each one, check whether
`pc` landed on one of the five patched addresses; if it did, either do
nothing and simulate an `RTS` (for the four output routines — a headless
run doesn't need to render "All Tests succeded" to a terminal) or, for
`WARMS`, stop the loop — the test program is signaling it's done. Then
the entire verdict is one assertion: `assert_eq!(sys.bus.mem[ERRFLG_ADDR],
0)`. No FLEX emulation, no output rendering, just enough of a stub to keep
the test program from running off the rails when it tries to act polite
about its own results.

This is still undone — there is no `cputest.txt` in this repository's tree
today, and no such harness. [`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5's recommendation stands
un-executed. That's a real, specific gap, and it's exactly the kind this
course wants you to be able to name precisely (a five-address I/O stub,
not a rewrite) rather than wave at vaguely. Exercise 4.10 is this project,
scoped down to a size you can actually finish in one sitting.

### Leg 3 — hand-written corner tests

The third leg you can inspect directly right now: `crates/mc6809/tests/
interrupts.rs`, [`indexed.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/indexed.rs), [`stack.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/stack.rs), and friends — tests written by a
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

## 4.6 Reading three real tests

Everything in §4.1 and §4.4 is provable, not assertable-on-faith — here are
three tests from [`crates/mc6809/tests/interrupts.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/interrupts.rs), walked line by line:
one for the partial frame (§4.1), one for `CWAI`'s state machine (§4.4),
one for `SYNC`'s (§4.4 again).

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

### `sync_halts_and_idles_until_interrupt`

`CWAI`'s test proves a frame gets pre-stacked and not doubled. `SYNC`'s
test has a different job: proving a masked interrupt can wake the CPU
*without* servicing it — the exact behavior exercise 4.2 asks you to
predict before reading any code.

```rust
// crates/mc6809/tests/interrupts.rs:264-284
#[test]
fn sync_halts_and_idles_until_interrupt() {
    let mut s = Sys::code(0x1000, &[0x13, 0x12]); // SYNC ; NOP
    s.step(); // SYNC
    assert_eq!(s.cpu.state, State::Syncing);
    let pc_after_sync = s.cpu.pc;

    // While syncing, step() just idles.
    let idle = s.step();
    assert_eq!(idle, 1);
    assert_eq!(s.cpu.pc, pc_after_sync); // no fetch

    // A masked IRQ still wakes SYNC (without servicing).
    s.cpu.cc = cc::IRQ_MASK;
    let serviced = s.cpu.irq(&mut s.bus);
    assert!(!serviced);
    assert_eq!(s.cpu.state, State::Running);
    // Next step runs the instruction after SYNC.
    s.step();
    assert_eq!(s.cpu.pc, pc_after_sync + 1);
}
```

Four `step()`/`irq()` calls, four separate facts, in order. First
`s.step()` executes the `SYNC` opcode itself — no frame stacked, no vector
loaded, `state` simply becomes `Syncing`, matching §4.1's one-line
`exec_interrupt_halt` arm exactly. `pc_after_sync` is captured *after*
`SYNC` retires — it's the address of the `NOP` that follows, not `SYNC`
itself, because `step()` already advanced `PC` past the one-byte opcode
before dispatching it. Second, a `step()` call while `Syncing` — this is
the `if self.state != State::Running` branch from §4.4's `step()` excerpt
— returns `1` and leaves `PC` exactly where it was: no opcode fetch
happened, `pc_after_sync` is unchanged, proving the halt really does
nothing but tick the clock.

Third is the one this test exists to nail down: `cc = cc::IRQ_MASK` (`I`
set, `IRQ` explicitly masked), then `irq()` is called. `assert!(!serviced)`
— the mask worked, no frame was stacked, no vector was taken, exactly as
`irq()`'s masked branch says (§4.2). And yet `assert_eq!(s.cpu.state,
State::Running)` on the very next line — the CPU woke up anyway. Both
things are true simultaneously because they're checking two different
effects of the same four lines inside `irq()`:

```rust
if self.cc & cc::IRQ_MASK != 0 {
    if self.state == State::Syncing {
        self.state = State::Running;
    }
    return false;
}
```

The mask check and the `Syncing`-wakeup check are two separate `if`s, not
one — masking blocks `take_interrupt` (hence `serviced == false`), but the
`state` flip happens unconditionally inside the masked branch, before the
early `return`. Fourth and last, a final `s.step()` runs the `NOP` that
was sitting right after `SYNC` the whole time — `PC` lands at
`pc_after_sync + 1`, proving execution really did resume at "the
instruction after `SYNC`," not at some interrupt vector. Four assertions,
each one isolating a different clause of `irq()`'s eight-line body — this
is what "the test *is* the specification, in executable form" looks like
up close.

---

## 4.7 Reading assignment

1. **[`crates/mc6809/src/lib.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L110-L260), lines 110–260** — vector constants, `State`
   enum, `nmi_armed`'s doc comment, `reset()`, `load_s()`,
   `nmi()`/`irq()`/`firq()`, `take_interrupt()`: the entire interrupt
   subsystem in one contiguous read.
2. **[`crates/mc6809/src/exec.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs)** — `exec_interrupt_halt`
   (`SWI`/`RTI`/`CWAI`/`SYNC`) and the `SWI2`/`SWI3` arms inside
   `exec_page10`/`exec_page11`.
3. **[`crates/mc6809/src/stack.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/stack.rs)**, all 76 lines — `psh`/`pul` back every
   frame here, plus `PSHS`/`PULS` from week 3.
4. **[`crates/mc6809/tests/interrupts.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/interrupts.rs)**, all of it — every test in it is
   a claim this chapter makes, turned into an assertion.
5. **[`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5**, the testing-strategy paragraphs.
6. **[`crates/coco-core/src/debug.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/debug.rs)**, around line 186 (`Debugger` and its
   `trace` field) plus `TraceEntry` and `export_trace`.
7. **[`crates/coco-core/examples/trace.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/trace.rs)**, the whole file (71 lines).
8. *Optional, this machine only:* `docs/super-extended-basic-unravelled.pdf`
   Appendix B, starting at `$C000` — the disassembly §4.5's worked example
   is cross-checked against. Every line quoted from it in this chapter is
   reproduced verbatim, so skip this if you don't have local access to
   `docs/`; you won't be missing any fact, just the pleasure of finding it
   yourself in a thirty-year-old scanned reference.

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
([`lib.rs:200-211`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L200-L211)) to check it, then run
`sync_halts_and_idles_until_interrupt` to see it asserted live.

**4.3 — Sabotage the E flag (sabotage, then verify for real).** In
[`crates/mc6809/src/lib.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs), change `firq()`'s call — `self.take_interrupt(bus,
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

**4.5 — Reproduce the trace, then predict past where this chapter stopped
(read + verify).** From the repo root (with `roms/coco3.rom` present), run
`cargo run -p coco-core --example trace -- 60`. First, reproduce §4.5's
claims yourself rather than taking the chapter's word for them: confirm
`S` really does jump from `$0000` to `$5EFF` at `$C002`/`$C006` (the `LDS`
that arms `nmi_armed`, §4.3), and confirm the palette and MMU loops run
the full sixteen iterations each before falling through. Then go past
where "Following the ROM deeper" stopped: count the instructions from
`$8C1B` up through the last setup line before the copy loop's first
`LDD ,X++` at `$C02F` (the palette loop and MMU loop each run sixteen
times, three instructions and four instructions per pass respectively —
work out the rest of the count from the trace itself, not by guessing).
Add that to the copy loop's own cost, using "Following the ROM deeper"'s
arithmetic (`ENDMOVE - BEGMOVE = 813` bytes, four bytes and six
instructions per iteration). Predict the approximate total instruction
count at which the trace should show `PC = $C03C` executing `JMP L4000`,
then run the trace with that count (pad it generously) and confirm. How
close was the prediction, and which part of the estimate — the setup
count, the iteration count, or the exact point `BCS` stops branching —
would you refine first if you needed to land on the exact number?

**4.6 — SWI2 vs SWI (recall, three sentences max).** Why does it make
sense for `SWI` to set both `I` and `F` on entry while `SWI2`/`SWI3` set
neither? What class of code (what `SWI2` is typically used *for* on a real
CoCo — an operating system's syscall vector) benefits from an exception
that leaves the caller's interrupt posture completely alone?

**4.7 — Sabotage the CWAI fast path (sabotage, then verify for real).** In
`take_interrupt` ([`crates/mc6809/src/lib.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs)), delete the
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

**4.9 — Verify FIRQ nesting inside IRQ (build, then verify for real).**
Write the test the end of §4.4's nesting discussion asked for. Start with
`s.cpu.s = 0x3000`, `s.cpu.cc = 0x00`, plant vectors for both `IRQ` and
`FIRQ` (any two distinct target addresses), and drive `s.cpu.irq(&mut
s.bus)` followed by `s.cpu.firq(&mut s.bus)` — no `step()` needed for
either call. Assert, in order: (a) after `IRQ`, `cc & FIRQ_MASK == 0` —
`F` is still clear; (b) `FIRQ` is serviced (`firq()` returns `true`); (c)
`S` drops by exactly `12 + 3` from `$3000` total; (d) `cc` after `FIRQ`
has both `IRQ_MASK` and `FIRQ_MASK` set. Then place an `RTI` opcode at the
`FIRQ` vector's target, `step()` through it, and assert `S` returns to
its post-`IRQ` value (not all the way back to `$3000`) and `cc &
FIRQ_MASK == 0` again — `FIRQ` re-enabled, still "inside" the `IRQ`
handler. If any assertion surprises you, that's the point: predict the
exact hex values before running it, the same discipline as every other
exercise in this chapter.

**4.10 — Sketch the `cputest.txt` harness (build, scoped down).** You
don't need to actually assemble `cputest.txt` for this one (though you're
welcome to). Using `FlatBus` and a hand-assembled snippet of just two
routines — one that mimics `PSTRNG`'s job (do nothing, just `RTS`) and one
that mimics `WARMS` (a sentinel your Rust loop can detect) — write a small
test 6809 program that: sets a scratch byte to `1`, `JSR`s to your fake
`PSTRNG` address, then `JSR`s to your fake `WARMS` address. Drive it with
a loop in Rust (not `Sys::step()`'s test helper — write the loop by hand)
that calls `cpu.step(&mut bus)` repeatedly and, after each step, checks
whether `cpu.pc` equals your `PSTRNG` or `WARMS` stub address; for
`PSTRNG`, manually pop the return address off the stack and set `pc` to
it (simulating the `RTS` your stub never gets to execute, since you never
put a real opcode there); for `WARMS`, break out of the loop. Confirm the
scratch byte still reads `1` afterward. This is leg 2's entire technique,
in miniature, without needing a real assembler or the actual test file.

---

## What's next

Week 5 leaves the CPU crate behind — `mc6809` mostly just sits there,
correct, generic over whatever `Bus` you hand it. We open
[`coco-core/src/bus.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs) and ask the question this chapter's vector table
quietly assumed an answer to: when the CPU reads `$FFFE`, what actually
intercepts that read before it becomes a plain RAM access? That's the
decode order — hardwired vectors first, then the I/O page, then ROM, then
the GIME's MMU — and the first time the CoCo 1/2's SAM and the CoCo 3's
GIME visibly diverge into two different address-translation paths.
