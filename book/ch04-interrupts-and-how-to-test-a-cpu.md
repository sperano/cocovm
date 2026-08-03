# Chapter 4 — CPU core III: interrupts, halt states, and how to test a CPU with no test suite

*Week 4. Goal: implement and verify the 6809's interrupt paths. This chapter
maps both stack frames byte by byte, explains interrupt masking and halt
states, and develops a validation strategy for a processor without a standard
per-instruction conformance suite.*

---

Chapters 2 and 3 followed instructions requested by the running program. A
machine also has asynchronous events. Video timing, disk transfers, and
keyboard scanning cannot wait for application code to poll at a convenient
moment. An *interrupt* suspends the current instruction stream and transfers
control to a handler.

The 6809 provides six exception vectors, two hardware interrupt lines with
different stack costs, three software-interrupt instructions, and two halt
instructions. Correct return from an interrupt depends on the exact stack
layout, so the chapter begins with the bytes pushed by each path.

The second half turns to validation. An incorrect frame can return to a
plausible but wrong address and fail thousands of instructions later. Without
a standard 6809 conformance suite, the repository combines focused tests, a
self-checking exerciser, and trace comparison against a reference emulator.

By the end of the week the CPU crate is finished. Chapter 5 opens the bus.

---

## 4.1 Two shapes of interrupt frame

When an interrupt fires, the interrupted code must later resume with the same
registers, flags, and next instruction. The CPU preserves that state on the
stack before entering the handler.

Every 6809 exception — `NMI`, `IRQ`, `FIRQ`, `SWI`, `SWI2`, `SWI3` — does the
same three things: save enough state to resume later, block re-entrant
interrupts as appropriate, and load `PC` from a fixed vector. What differs
is *how much* state "enough" means. Five of the six save everything: `A`,
`B`, `DP`, `X`, `Y`, `U`, `CC`, `PC` — twelve bytes, the **full frame**.
`FIRQ` alone saves only `CC` and `PC` — three bytes, the **partial frame**.

`FIRQ`, or fast interrupt request, uses the partial frame. A device with
latency-sensitive work, such as a UART about
to overrun, a disk controller with a byte sitting in its shift register — gets
in and out fast. Nine fewer bytes to push and nine fewer to pull add up to real
time saved at 0.895 MHz, where a single bus cycle is a little over a
microsecond. A handler entered through the
partial frame arrives with `A`, `B`, `X`, `Y`, `U`, and `DP` *not* saved. If it
touches any of them it must save and restore them itself. The 6809 does not
offer to do that work, and it does not stop the handler from being careless
about it either. The chip's contribution is to make the cheap path available;
the discipline is the programmer's.

Which frame is on the stack has to be recorded somewhere, because the `RTI`
that eventually unwinds it needs to know how many bytes to pull. There is no
second opcode for "return from fast interrupt" — one `RTI` serves both frames
— so the shape has to travel with the frame itself. That somewhere is bit 7 of
`CC`, the **E** (entire) flag:

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

Chapter 2 met this module as the home of `N`, `Z`, `V`, and `C` — the arithmetic
flags every instruction computes. The top three constants are the ones this
chapter cares about, and none of them is an arithmetic result. `IRQ_MASK` and
`FIRQ_MASK` are gates that decide whether a line is listened to at all.
`ENTIRE` is not a gate but a record: `E=1` means "the full frame is under me,"
`E=0` means "just `CC` and `PC`." It is written by the CPU on the way into an
exception and read by the CPU on the way out, and no arithmetic instruction
ever touches it.

The two halves of that contract sit two match arms apart in
`exec_interrupt_halt`. `take_interrupt` sets
`E` before pushing, which is why the stacked copy of `CC` always carries the
right answer, and `RTI` reads it back after pulling `CC` first — the one
register both frames agree is on top:

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

The `RTI` arm shows that the stacked `E` flag selects the frame size. The first
`pul` uses
mask `0x01`, which is `stack_mask::CC` alone: pull exactly one byte, and put
it in `CC`. Only *after* that byte has landed does the `if` run, and it tests
a bit of the value that was just read off the stack — not a bit of some
register the CPU was carrying, and not anything derived from the opcode. If
`E` came back set, mask `0xFE` pulls the remaining nine registers' worth of
bytes and the arm reports 15 cycles. If `E` came back clear, mask `0x80` pulls
`PC` and nothing else, for 6. Those two numbers are the data sheet's, written
as literals at the point each branch is taken, and §4.4 will have something to
say about why they are literals rather than a formula.

The other three arms introduce later sections. `SWI` at `$3F` is the software interrupt,
covered at the end of §4.2. `CWAI` at `$3C` and `SYNC` at `$13` are the two
halt instructions, covered in §4.4. `CWAI` calls `psh` with mask `0xFF` —
every register, the full frame — before
any interrupt has arrived at all. That is not a typo, and it is the most
interesting thing in this function.

> **Rust corner: `unreachable!` as a proof obligation.** The final arm of
> that `match` is not a fallback that returns a safe default; it is a panic:
> `_ => unreachable!("exec_interrupt_halt called for opcode {opcode:#04X}")`.
> Rust requires the `match` to be exhaustive, and `opcode` is a `u8`, so
> *something* has to cover the other 252 values. There were three choices.
> Return a plausible cycle count and carry on, which silently converts a
> dispatch bug into wrong timing. Return an `Option` or a `Result`, which
> pushes an error case up to every caller for a condition that cannot
> legitimately happen. Or assert that it cannot happen and say so loudly.
>
> The third is right here because the guarantee is real and checkable one
> screen away: `step`'s top-level match sends exactly `0x3F | 0x3B | 0x3C |
> 0x13` into this function ([`exec.rs:73`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L73)),
> and nothing else calls it. `unreachable!` documents that invariant in
> executable form: if a later edit adds a fifth opcode to the dispatch line
> and forgets the arm here, the test suite reports the exact opcode byte in
> the panic message rather than producing a subtly wrong emulator. Reach for
> `unreachable!` when a case is impossible *by construction* and the
> construction is local enough for a reader to verify. Reach for a real error
> type when the case is merely unlikely.

### Byte-by-byte, with real addresses

"Twelve bytes" and "three bytes" are the kind of claim that is easy to nod
along to and hard to use. What a person debugging a broken interrupt actually
needs is a picture: this address holds that register. So here are the actual
addresses, taken directly from
[`tests/interrupts.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/interrupts.rs)'s
own setup, which puts `S` at `0x2000` before the interrupt fires.

The full frame is what `IRQ`, `NMI`, `SWI`, `SWI2`, and `SWI3` all produce.
Starting from `S = $2000`:

| Address | Contents | Address | Contents |
|---------|----------|---------|----------|
| `$1FF4` | CC       | `$1FFA` | Y (hi)   |
| `$1FF5` | A        | `$1FFB` | Y (lo)   |
| `$1FF6` | B        | `$1FFC` | U (hi)   |
| `$1FF7` | DP       | `$1FFD` | U (lo)   |
| `$1FF8` | X (hi)   | `$1FFE` | PC (hi)  |
| `$1FF9` | X (lo)   | `$1FFF` | PC (lo)  |

`S` afterward is `$1FF4`, twelve below where it started. Two details in that
table repay a second look. The bytes run in register order from `CC` at the
lowest address up to `PC` at the highest, which is the reverse of the order in
which they were pushed — pushing walks downward, so the last thing pushed
lands lowest. And every sixteen-bit register appears high byte first at the
lower address, because the 6809 is big-endian everywhere, including on its own
stack.

That table is not a paraphrase of the code's intent. It is what
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

The last assertion is the one that ties the code to the table. `s.mem(0x1FF4)`
reads the byte at the address the table calls `CC`, and it comes back `0x80` —
the `E` bit alone. The test started `cc` at `0x00`, so every bit in that
stacked byte had to be put there by the exception sequence itself, and exactly
one was: `take_interrupt` sets `E` *before* pushing. The trailing comment
spells out the consequence that surprises people: the `I` and `F` masks are
set on the way in too, but *after* the push, so the copy on the stack has them
clear while the live `CC` has them set. That is not a quirk of this
implementation; it is what makes `RTI` correct, since the handler must return
to the caller's interrupt posture rather than its own.

### The round trip, proved

Stacking twelve bytes is only half a promise. The other half is that pulling
them back puts every register exactly where it was, and there is a test that
does nothing but check that, register by register, with a distinct value in
each so that a swap could not hide:

```rust
// crates/mc6809/tests/interrupts.rs:123-159
#[test]
fn swi_then_rti_restores_all_registers() {
    let mut s = Sys::code(0x1000, &[0x3F]); // SWI at $1000
    s.bus.load(0xFFFA, &[0x90, 0x00]);
    s.bus.load(0x9000, &[0x3B]); // RTI handler
    s.cpu.s = 0x2000;
    s.cpu.a = 0x11;
    s.cpu.b = 0x22;
    s.cpu.x = 0x3333;
    s.cpu.y = 0x4444;
    s.cpu.u = 0x5555;
    s.cpu.dp = 0x66;
    s.cpu.cc = 0x00;

    s.step(); // SWI -> handler
    assert_eq!(s.cpu.pc, 0x9000);

    // Clobber everything the handler might use.
    s.cpu.a = 0;
    s.cpu.b = 0;
    s.cpu.x = 0;
    s.cpu.y = 0;
    s.cpu.u = 0;
    s.cpu.dp = 0;

    let rti_cycles = s.step(); // RTI
    assert_eq!(s.cpu.a, 0x11);
    assert_eq!(s.cpu.b, 0x22);
    assert_eq!(s.cpu.x, 0x3333);
    assert_eq!(s.cpu.y, 0x4444);
    assert_eq!(s.cpu.u, 0x5555);
    assert_eq!(s.cpu.dp, 0x66);
    assert_eq!(s.cpu.pc, 0x1001); // return address (after the SWI opcode)
    assert_eq!(s.cpu.s, 0x2000); // stack unwound
    assert_eq!(rti_cycles, 15); // full-frame RTI
    assert_eq!(s.cpu.cc & cc::ENTIRE, cc::ENTIRE); // restored CC had E set
}
```

The structure of this test is worth stealing for any CPU core. Load a
distinguishable constant into every register that the frame is supposed to
carry — `0x11`, `0x22`, `0x3333`, `0x4444`, `0x5555`, `0x66`, all different,
none of them a value the CPU could plausibly produce by accident. Take the
exception. Then deliberately destroy all six from outside, simulating a
handler that used every register it could reach. Only then execute the `RTI`,
and demand that all six come back.

Two of the assertions are about things the handler never touched, and they are
the interesting ones. `s.cpu.pc == 0x1001` is the return address: `$1000` held
the one-byte `SWI` opcode, so the stacked `PC` is the address *after* it,
which is the whole reason the interrupted program resumes rather than
re-executing the instruction that got interrupted. And `s.cpu.s == 0x2000`
says the stack pointer landed exactly where it began — twelve bytes down,
twelve bytes back up, no drift. A frame that pushed twelve and pulled eleven
would still pass most of the register checks and fail this one, which is
precisely the class of bug that is invisible until the third nested interrupt.

The partial frame is smaller and, because it is smaller, easier to get subtly
wrong. `FIRQ` is the only exception that produces it. Starting from the same
`S = $2000`:

| Address | Contents |
|---------|----------|
| `$1FFD` | CC       |
| `$1FFE` | PC (hi)  |
| `$1FFF` | PC (lo)  |

`S` afterward is `$1FFD`. This is confirmed the same way, in
`firq_uses_partial_frame`, which §4.6 walks through in full. Notice where the
partial frame's `CC` byte lands: `$1FFD`, the *same* address the full frame
uses for `U`'s low byte. That coincidence has alarmed more than one person
reading these two tables side by side, and it is nothing to be nervous about.
The stack is simply three bytes deep instead of twelve, and `stack.rs`'s
address arithmetic — `sp.wrapping_sub(1)`, once per byte — has no idea which
frame it is building. Addresses collide because both frames end at the same
place; they start at different ones.

### Push order, straight from `stack.rs`

Both frames are produced by the same helper that backs the `PSHS` instruction
you read in Chapter 3. The interrupt path does not have its own push routine at
all; it calls `psh` with a specific register mask and lets the general
machinery do the work. Here is that routine's contract, in its own words:

```rust
// crates/mc6809/src/stack.rs:22-25
/// PSHS/PSHU. `to_s` selects the hardware (S) stack; otherwise the user (U)
/// stack. Push order is PC, U/S, Y, X, DP, B, A, CC (highest address first),
/// so CC ends up on top. Bit 6 of the mask pushes the *other* stack pointer.
/// Returns the cycle count (base + 1 per byte).
```

"`PC` first" sounds backwards until the direction of travel is taken into
account. Pushing walks the stack pointer *downward*, so whatever is pushed
first ends up at the *highest* address, and whatever is pushed last ends up on
top, at the lowest address. `CC` is pushed last, which puts it exactly where
`RTI`'s first `pul` — the one that always reads `CC` before deciding anything
— expects to find it. The push order and the `RTI` decision procedure are two
ends of the same design; neither makes sense without the other.

The mask itself is a byte, one bit per register, and the bits have names:

```rust
// crates/mc6809/src/lib.rs:95-105
mod stack_mask {
    pub const CC: u8 = 0x01;
    pub const A: u8 = 0x02;
    pub const B: u8 = 0x04;
    pub const DP: u8 = 0x08;
    pub const X: u8 = 0x10;
    pub const Y: u8 = 0x20;
    /// The *other* stack pointer: U when pushing/pulling S, S when pushing/pulling U.
    pub const OTHER_STACK_PTR: u8 = 0x40;
    pub const PC: u8 = 0x80;
}
```

This is the same postbyte a programmer writes by hand as `PSHS A,B,X`, and the
interrupt path uses the same eight bits with two fixed values. The full frame
is mask `0xFF`: every bit set at once, including `OTHER_STACK_PTR` at `0x40`,
which when pushing to `S` means `U`. The partial frame is
`PC_CC_MASK = stack_mask::PC | stack_mask::CC`
([`lib.rs:111`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L111))
— two bits, `0x80 | 0x01 = 0x81` — which is why only three bytes move.

And here is the body those masks drive, the part that turns a bit pattern into
a byte count and a sequence of bus writes:

```rust
// crates/mc6809/src/stack.rs:36-46
        let mut bytes = 0u32;
        if mask & stack_mask::PC != 0 { push8(&mut sp, self.pc as u8, &mut bytes); push8(&mut sp, (self.pc >> 8) as u8, &mut bytes); }
        if mask & stack_mask::OTHER_STACK_PTR != 0 { push8(&mut sp, other as u8, &mut bytes); push8(&mut sp, (other >> 8) as u8, &mut bytes); }
        if mask & stack_mask::Y != 0 { push8(&mut sp, self.y as u8, &mut bytes); push8(&mut sp, (self.y >> 8) as u8, &mut bytes); }
        if mask & stack_mask::X != 0 { push8(&mut sp, self.x as u8, &mut bytes); push8(&mut sp, (self.x >> 8) as u8, &mut bytes); }
        if mask & stack_mask::DP != 0 { push8(&mut sp, self.dp, &mut bytes); }
        if mask & stack_mask::B != 0 { push8(&mut sp, self.b, &mut bytes); }
        if mask & stack_mask::A != 0 { push8(&mut sp, self.a, &mut bytes); }
        if mask & stack_mask::CC != 0 { push8(&mut sp, self.cc, &mut bytes); }
        if to_s { self.s = sp; } else { self.u = sp; }
        PUSH_PULL_BASE_CYCLES + bytes
```

Eight `if`s in a fixed order, and the order *is* the specification — there is
no sort, no table, no loop over set bits, because the 6809's transfer order is
not a function of the mask; it is a constant the mask merely selects from.
Run mask `0xFF` through it mentally and every branch is taken, sixteen `push8`
calls happen, and `bytes` ends at 12. Run `PC_CC_MASK` through it and exactly
two branches are taken, three `push8` calls happen, `bytes` ends at 3, and —
this is the part that makes §4.1's two tables agree — `CC` still lands last,
still on top, still where `RTI` will look first.

Each sixteen-bit register is pushed low byte first. That looks like the wrong
endianness until you remember the pointer is moving down: writing the low byte
first puts it at the higher address, and the high byte then lands below it, so
the pair reads back big-endian, exactly as `pull16_s` and the `Bus` trait's
`read_u16` expect.

The last two lines are the bookkeeping the interrupt path depends on and,
as §4.4 will show, does not entirely use. `sp` is written back to whichever
stack pointer the call selected, and the function returns
`PUSH_PULL_BASE_CYCLES + bytes` — five plus one per byte. Hold on to the fact
that this function computes a cycle count. Whether anybody *reads* it is a
different question.

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

Both frames are now fully accounted for: what goes on them, in what order, at
what address, and how the `E` bit lets one `RTI` unwind either. What has not
been said is *where the handler is* — the frame is only half of an exception,
and the other half is a fixed pointer at the top of memory.

---

## 4.2 The vector table, priority, and masking

An interrupt has to transfer control somewhere, and the CPU has no way to be
told where. There is no register a program can load with the address of its
`IRQ` handler, because the hardware has to work on a machine that has just
been powered on and has never executed an instruction. The only thing a cold
CPU and a running program can both agree on is an address baked into the
silicon. So the 6809 reserves the top of its address space for a table of
pointers, reads two bytes from a fixed slot in that table, and jumps there.

All seven vectors live in the top fourteen bytes of address space, one 16-bit
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

Laid out as a map, `RESET` sits at the very top and everything else descends
from it without a gap:

| Address       | Constant       | Exception                     |
|---------------|----------------|-------------------------------|
| `$FFFE–$FFFF` | `VECTOR_RESET` | RESET                         |
| `$FFFC–$FFFD` | `VECTOR_NMI`   | NMI                           |
| `$FFFA–$FFFB` | `VECTOR_SWI`   | SWI (`$3F`)                   |
| `$FFF8–$FFF9` | `VECTOR_IRQ`   | IRQ                           |
| `$FFF6–$FFF7` | `VECTOR_FIRQ`  | FIRQ                          |
| `$FFF4–$FFF5` | `VECTOR_SWI2`  | SWI2 (`$10 3F`)               |
| `$FFF2–$FFF3` | `VECTOR_SWI3`  | SWI3 (`$11 3F`)               |

These fourteen bytes create a constraint for the next
chapter has to satisfy: whatever else the memory map does — and the CoCo 3's
MMU can move almost anything anywhere — these fourteen bytes had better still
read out of ROM when the CPU asks for them, or a reset lands at an arbitrary
address. Chapter 5 opens with exactly that problem.

Reading a vector is the last thing an exception does, and it is a perfectly
ordinary bus read: `bus.read_u16(vector)`, big-endian, high byte at the lower
address, using the default method the `Bus` trait supplies. The CPU has no
special "vector fetch" pathway, which is a small but real fidelity win — the
real chip did not have one either.

### Masking: which lines get listened to

Each entry point independently decides whether it is allowed to fire at all.
`IRQ` respects `CC`'s `I` bit; `FIRQ` respects `F`; `NMI` respects neither and
has its own gate, `nmi_armed`, which is §4.3's subject. Here are the two
maskable ones, in full:

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

The two functions differ in three places:
which `CC` bit gates them, which vector they take, and what they pass for the
last three arguments of `take_interrupt`. The `bool` they return is the honest
answer to "was this serviced?", and the caller in Chapter 6 uses it for nothing at
all, which turns out to be correct — a masked line is still asserted, and the
device holding it down will still be holding it down the next time anyone
checks.

The nested `if State::Syncing` gives the masked path one side effect: it wakes
a CPU halted by `SYNC` even though the interrupt is not serviced. Section 4.4
returns to that behavior.

`take_interrupt`'s last two boolean parameters, `set_i` and `set_f`, decide
which masks get *set on entry* — what stops a second interrupt from preempting
the handler before it can save context. Reading straight off each call site:

| Exception | Sets `I` | Sets `F` | Frame    |
|-----------|----------|----------|----------|
| `NMI`     | yes      | yes      | full     |
| `IRQ`     | yes      | no       | full     |
| `FIRQ`    | yes      | yes      | partial  |
| `SWI`     | yes      | yes      | full     |
| `SWI2`    | no       | no       | full     |
| `SWI3`    | no       | no       | full     |

`IRQ` is the only hardware entry in the table that does not set both masks. An
`IRQ` handler runs with `I` set, so a second `IRQ` cannot preempt it and
re-enter the same code, but `F` is left exactly as the interrupted program had
it. If `F` was clear, a `FIRQ` can still get through — the slow interrupt does
not lock out the fast one. Everything else that can legitimately claim "I got
here first" locks out both lines: `NMI` because nothing should preempt it,
`FIRQ` because the entire point of the partial frame is to be quick and
uninterrupted, `SWI` because it is a deliberate trap into supervisor-style
code. And `SWI2`/`SWI3`, the software-call vectors used for operating-system
entry points, leave the masks completely alone, so a system call runs with
interrupts exactly as the caller had them.

That table is a summary of six one-line facts. The next subsection reads them
off the source.

### `take_interrupt`, one call at a time

One function implements all six combinations of frame size, masks, and vector:

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

The six call sites select different parameter combinations:

```rust
self.take_interrupt(bus, VECTOR_NMI,  true,  true,  true);  // nmi()  — lib.rs:197
self.take_interrupt(bus, VECTOR_IRQ,  true,  false, true);  // irq()  — lib.rs:209
self.take_interrupt(bus, VECTOR_FIRQ, true,  true,  false); // firq() — lib.rs:222
self.take_interrupt(bus, VECTOR_SWI,  true,  true,  true);  // SWI    — exec.rs:266
self.take_interrupt(bus, VECTOR_SWI2, false, false, true);  // SWI2   — exec.rs:169
self.take_interrupt(bus, VECTOR_SWI3, false, false, true);  // SWI3   — exec.rs:190
```

Read the parameters left to right against the body. `vector` picks which two
bytes at the top of memory load into `PC`, using the map from the start of
this section; the parameter's only use is the single `bus.read_u16(vector)`
near the bottom.

`set_i` and `set_f` drive the two `if` blocks in the middle, and notice where
those `if`s sit: *outside* the `if self.state != State::Waiting` guard,
unconditionally, on every call. That placement matters once `CWAI` enters the
picture in §4.4. An interrupt that wakes a `CWAI`'d CPU skips the stacking
half of this function entirely, and it still gets its masks set, because
mask-setting was never gated on stacking in the first place. Two `if`s that
could have been nested one level deeper, and were not, and the difference is a
behavior.

`entire` has an effect only *inside* the guard, and there it is a genuine
either/or. One branch sets `E` and calls `self.psh(bus, 0xFF, true)` — mask
`0xFF`, every `stack_mask` bit, the twelve-byte frame. The other clears `E`
and calls `self.psh(bus, PC_CC_MASK, true)` — two bits, three bytes. Never
both, never neither. Scan the six-line table above and `firq()` is the only
call passing `false`; every other exception passes `true`. One boolean, read
once, is the entire difference between "fast" and "everything else."

The final two lines run on every path. `PC` becomes the vector's contents, and
`state` becomes `Running` — which is how a `SYNC`'d or `CWAI`'d CPU gets
un-halted as a side effect of being interrupted, rather than through any
separate wake-up mechanism.

> **Rust corner: three positional booleans, and when that is acceptable.**
> `self.take_interrupt(bus, VECTOR_IRQ, true, false, true)` is exactly the
> call signature style that code reviewers are trained to flag. Rust has no
> named arguments, so nothing at the call site says which `true` is which,
> and swapping two of them compiles cleanly and produces a CPU that is wrong
> in a way no type error will catch. The textbook fix is an enum per
> parameter, or a small options struct, and Chapter 6 has a sidebar arguing for
> exactly that in a different context.
>
> Why is it tolerable here? Because of a property this function has and most
> functions do not: **the complete set of call sites fits on one screen and
> will never grow.** There are six exceptions on a 6809 and there will be six
> exceptions on a 6809 forever. The six calls sit in two files, they are
> reproduced above in their entirety, and each is followed by a comment
> naming the exception. A reader who wants to know what `false` means in
> position four reads the parameter list twenty lines up.
>
> The general rule is worth extracting, because it is about more than
> booleans: an abbreviation is safe in proportion to how easy it is to
> resolve. When the set of callers is closed, small, and local, positional
> arguments cost a glance. When callers are open-ended — a public API, a
> function called from forty places, a parameter list that will grow — the
> same style costs a bug. This one is closed by the hardware itself.

### Priority is somebody else's job

There is no `match`-based priority table anywhere in `mc6809`, and its absence
is a design decision rather than an omission. Each of `nmi()`, `firq()`, and
`irq()` is an independent public entry point that can be called at any time,
in any order. Nothing in the CPU crate expresses "`FIRQ` outranks `IRQ`."

That is because priority is not a property the CPU can observe. Masking is:
`I` and `F` are bits in a register the CPU owns, and the decision "am I
allowed to take this line right now" is answerable from inside. Priority is
different — it means "two devices are asserting two lines simultaneously,
which one wins" — and the CPU never sees two lines at once as a data
structure. Only the thing that knows which devices are pulling which lines can
order them, and that thing is the bus. Here is the whole of it, from Chapter 6's
run loop:

```rust
// crates/coco-core/src/machine/run.rs:174-183
/// Deliver pending FIRQ/IRQ to the CPU. The CPU itself honours the F/I masks
/// and leaves a masked, still-asserted line pending for the next check.
fn service_interrupts(&mut self) {
    if self.bus.firq_asserted() {
        self.cpu.firq(&mut self.bus);
    }
    if self.bus.irq_asserted() {
        self.cpu.irq(&mut self.bus);
    }
}
```

Priority, expressed as statement order. `FIRQ` is offered first; if it is
serviced, `take_interrupt` sets both `I` and `F` on the way in, so the `irq()`
call on the next line finds `I` set and declines. If `FIRQ` is masked or not
asserted, `IRQ` gets its turn. `NMI` is polled separately and earlier, ahead
of both, in the same function that calls this one
([`run.rs:110-113`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs#L110-L113)).
Six lines of ordinary control flow, and the ordering they express is the whole
of interrupt priority in this emulator.

One more constraint belongs to the same layer, and its comment states a fact
about the real chip that the CPU crate itself has no way to enforce:

```rust
// crates/coco-core/src/machine/run.rs:96-100
    /// The MC6809 recognizes interrupts only at the *end* of an instruction, so
    /// the first instruction after HALT* releases must execute before any
    /// pending NMI/IRQ/FIRQ is serviced. Skipping this lets the completion NMI
    /// of an FD-502 sector read preempt the DSKCON copy loop's `STB ,X+` that
    /// stores the sector's final byte — dropping one byte per sector on load.
```

Nothing prevents a caller from invoking `cpu.irq(&mut bus)` in the middle of
anything — the CPU crate's API is just a method. The rule that interrupts are
recognized only between instructions lives in the caller, and the comment
records what it cost to learn: one byte dropped per sector, on disk loads, in
Chapter 13's floppy controller. That is a useful thing to know about layered
designs in general. A narrow, permissive interface pushes obligations
outward, and the obligations do not stop being real just because the type
system stopped tracking them.

### SWI, SWI2, SWI3

The three software interrupts are the exceptions a program raises on purpose,
by executing an instruction, and all three push the full frame. There is no
"fast `SWI`" the way there is a fast `IRQ` — a deliberate trap has no latency
argument to make, since the program chose the moment. What differs between
them is prefix bytes and mask behavior.

`SWI` is a plain one-byte opcode, `$3F`, dispatched straight out of `step`'s
top-level match into `exec_interrupt_halt`, which §4.1 quoted in full.
`SWI2` and `SWI3` are page-prefixed — `$10 3F` and `$11 3F` — and are decoded
one level down, inside the prefix-page handlers built in Chapter 2:

```rust
0x3F => { self.take_interrupt(bus, VECTOR_SWI2, false, false, true); 20 } // SWI2 — exec.rs:169, exec_page10
0x3F => { self.take_interrupt(bus, VECTOR_SWI3, false, false, true); 20 } // SWI3 — exec.rs:190, exec_page11
```

The same opcode byte, `$3F`, appears three times in the ISA and means three
different things, because the prefix determines which `match` it is being
matched against before the mask arguments are ever read. `SWI` costs 19 cycles
and sets `I` and `F`; `SWI2` and `SWI3` cost 20 — one more, the price of
fetching the prefix byte — and touch no masks at all.

The mask difference is the interesting one, and the test named
`swi2_and_swi3_do_not_touch_masks` in the same file pins it down. Think about
what `SWI2` is for. On a 6809 running an operating system, `SWI2` is a
system-call vector: user code loads its arguments into registers, executes
`SWI2`, and the OS's handler runs. If that handler started by masking
interrupts, every system call — however trivial — would create a window in
which the machine could not respond to hardware. Leaving the masks alone means
a syscall inherits the caller's interrupt posture and gives it back unchanged.
`SWI`, by contrast, sets both masks, which is the posture of code that wants
the machine to stop moving underneath it while it runs — and a program that
wants otherwise has `SWI2` and `SWI3` available and no reason to reach for
`SWI`.

That accounts for five of the six exceptions. The sixth is the one whose name
says it cannot be ignored, and which this codebase ignores on purpose.

---

## 4.3 Why NMI waits for a stack pointer

`NMI` stands for non-maskable interrupt, and the name is a promise: no bit in
any register can turn it off. It is the line reserved for events that must be
noticed no matter what the running program has masked; on the CoCo it is
brought out to the cartridge port, where Chapter 13's disk controller pulls it to
announce that a sector transfer has finished (Chapter 1 met that wiring, and
Chapter 6's run loop polls the line ahead of both maskable ones). And yet the very
first thing `nmi()` does is check a flag that can make it *return without doing
anything at all*:

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

This behavior follows the processor's documented NMI arming rule.

`RESET` does not initialize `S`. Look back at the reset sequence and it sets
`DP`, sets both masks, loads `PC` — and says nothing about the stack pointer.
On real silicon `S` therefore comes up holding whatever garbage was in it at
power-on: an arbitrary sixteen-bit number that nothing put there on purpose.
Now suppose an `NMI` line asserts in that window, before the boot ROM has
executed its stack-pointer setup. This is not a hypothetical; it is a real,
physically possible event, since the devices that pull `NMI` are powering up at
the same moment the CPU is.

The CPU's first act would be to push twelve bytes through that garbage
pointer. If it happened to point into RAM, twelve bytes of whatever was there
would be destroyed — possibly the very ROM-copied code about to run. If it
pointed into ROM, the writes would be silently discarded and the frame would
simply not exist, so the `RTI` at the end of the handler would pull twelve
arbitrary ROM bytes and set `PC` to two of them. Either way the machine is
dead before it has started, in a way that would look like a random hang and
would be nearly impossible to diagnose from the outside.

Motorola's answer, baked into the silicon, is to make `NMI` recognition
conditional on evidence that software has taken control: the line is ignored
until a program has explicitly loaded a real value into `S` at least once. It
is not a mask — no instruction can turn it back off — it is an arming
condition, one-way, cleared only by reset.

The struct field records the rule and the write paths that must honor it:

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

Three of those fields are bookkeeping rather than architecture, and the
comment on the third is the longest in the struct for a reason: it is the only
place in the crate where a data-sheet rule, its rationale, and the complete
list of things that must honor it are written down together.

### Routing writes to `S` through `load_s`

Four instruction paths can name `S` as a destination. Missing one could
produce a CPU on which
`NMI` occasionally stays deaf for the rest of a boot, depending on how the ROM
happened to set up its stack.

The enforcement mechanism is a single private helper that every one of those
four points funnels through:

```rust
// crates/mc6809/src/lib.rs:185-189
/// Load the stack pointer from program action, arming NMI recognition.
fn load_s(&mut self, v: u16) {
    self.s = v;
    self.nmi_armed = true;
}
```

The first two call sites are the obvious ones. `LDS` in all four addressing
modes is one contiguous block in the `$10` prefix page:

```rust
// crates/mc6809/src/exec.rs:160-163
            0xCE => { let v = self.fetch_u16(bus);       self.load_s(v); self.set_nz16(v); 4 }
            0xDE => { let v = self.read_direct16(bus);   self.load_s(v); self.set_nz16(v); 6 }
            0xEE => { let (ea, ic) = self.ea_indexed(bus); let v = bus.read_u16(ea); self.load_s(v); self.set_nz16(v); 6 + ic }
            0xFE => { let v = self.read_extended16(bus); self.load_s(v); self.set_nz16(v); 7 }
```

and `LEAS` is a single line in the indexed group,
[`exec/exec_data.rs:105`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec/exec_data.rs#L105),
which computes an effective address and loads it into `S` rather than reading
memory at it.

The third is easy to forget, because it does not mention `S` anywhere in its
own code. `TFR` and `EXG` take a postbyte naming two registers by number, and
`S` is simply one of the numbers:

```rust
// crates/mc6809/src/regs.rs:32-46
    pub(crate) fn reg_write(&mut self, code: u8, value: u16) {
        match code {
            regsel::D => self.set_d(value),
            regsel::X => self.x = value,
            regsel::Y => self.y = value,
            regsel::U => self.u = value,
            regsel::S => self.load_s(value),
            regsel::PC => self.pc = value,
            regsel::A => self.a = value as u8,
            regsel::B => self.b = value as u8,
            regsel::CC => self.cc = value as u8,
            regsel::DP => self.dp = value as u8,
            _ => {} // invalid
        }
    }
```

Every arm in that `match` is a bare field assignment except one. `regsel::S`
routes through `load_s`, so `TFR X,S` and `EXG D,S` arm `NMI` exactly as `LDS`
does — which is correct, since from the hardware's point of view a stack
pointer loaded by `TFR` is every bit as valid as one loaded by `LDS`.

The fourth is the one that would be genuinely surprising if it were missing.
Chapter 3's indexed postbyte includes auto-increment and auto-decrement modes,
and `S` is one of the four registers those modes can name. Writing `LDA ,S++`
mutates `S` through the postbyte's writeback path, never touching `LDS` at
all:

```rust
// crates/mc6809/src/addressing.rs:41-48
    fn set_index_reg(&mut self, sel: u8, val: u16) {
        match sel & 0b11 {
            0b00 => self.x = val,
            0b01 => self.y = val,
            0b10 => self.u = val,
            _ => self.load_s(val),
        }
    }
```

Same shape as `reg_write`: three plain assignments and one call to `load_s`,
sitting in the fall-through arm because selector `0b11` is `S`.

Four call sites, one arming helper, one clearing site. Add a new instruction
that can write `S` and this is the checklist item: does it call `load_s`, or
does it quietly open a fifth way around the data sheet's rule? The invariant
is checkable in about ten seconds — grep the crate for `self.s = ` and every
hit is either `load_s`'s own assignment or `stack.rs`'s internal stack-pointer
bookkeeping in `push16_s`, `pull16_s`, `psh`, and `pul`. Not one of them is a
program-visible load that bypasses the helper.

The clearing site is `reset()` itself, three lines long, and worth reading
once since it is the starting state everything above assumes:

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

Five assignments, and two of them exist purely to keep the machine quiet.
Both interrupt masks go up, so nothing can fire before the ROM is ready, and
`nmi_armed` goes down, so `NMI` cannot fire either. Of the rest, `pc` takes
the reset vector, `dp` is zeroed, and `state` returns to `Running` so that a
reset out of a `CWAI` halt actually resumes execution rather than idling
forever. `S` is conspicuously not among them — the data sheet does not
initialize it, so neither does this.

### Non-maskable and yet ignorable, in two tests

Those two properties sound contradictory, and the test file states both of
them in adjacent tests, which is the clearest possible way to see that they
are not in conflict. The first proves the arming rule by doing what a real ROM
does:

```rust
// crates/mc6809/tests/interrupts.rs:244-258
#[test]
fn nmi_is_ignored_until_the_first_program_load_of_s() {
    // MC6809 datasheet: after reset, NMI is not recognized until S is loaded —
    // a frame push through a garbage pointer would corrupt memory.
    let mut s = Sys::code(0x1000, &[0x10, 0xCE, 0x20, 0x00, 0x12]); // LDS #$2000 ; NOP
    s.bus.load(0xFFFC, &[0x60, 0x00]); // NMI vector -> $6000
    s.bus.load(0xFFFE, &[0x10, 0x00]); // reset vector -> $1000
    s.cpu.reset(&mut s.bus);
    s.cpu.nmi(&mut s.bus);
    assert_eq!(s.cpu.pc, 0x1000, "unarmed NMI must be ignored");
    s.step(); // LDS #$2000 arms recognition
    s.cpu.nmi(&mut s.bus);
    assert_eq!(s.cpu.pc, 0x6000, "NMI must vector once S is loaded");
    assert_eq!(s.cpu.s, 0x2000 - 12);
}
```

The whole boot sequence is here in miniature. A real reset vector at `$FFFE`
pointing at `$1000`, a real `LDS #$2000` assembled by hand as `$10 $CE $20
$00` at that address, and `reset()` called for real rather than fields being
poked. Then `nmi()` fires *before* the `LDS` runs, and `PC` has not moved: the
assertion message says it plainly, "unarmed NMI must be ignored." One `step()`
later — the `LDS` — the identical `nmi()` call vectors to `$6000` and leaves
`S` twelve bytes below `$2000`. Same call, same CPU, different answer,
separated by one instruction.

The neighboring test takes the opposite side:

```rust
// crates/mc6809/tests/interrupts.rs:231-242
#[test]
fn nmi_is_non_maskable() {
    let mut s = Sys::new();
    s.bus.load(0xFFFC, &[0x60, 0x00]); // NMI vector -> $6000
    s.cpu.pc = 0x1234;
    s.cpu.s = 0x2000;
    s.cpu.nmi_armed = true; // recognition armed (S "loaded"); see arming test
    s.cpu.cc = cc::IRQ_MASK | cc::FIRQ_MASK; // fully masked
    s.cpu.nmi(&mut s.bus);
    assert_eq!(s.cpu.pc, 0x6000); // serviced anyway
    assert_eq!(s.cpu.s, 0x2000 - 12);
}
```

`cc` is set to both masks at once — the most masked a 6809 can be — and the
`NMI` is serviced regardless. Put the two tests together and the apparent
contradiction dissolves into a precise statement: `NMI` cannot be *masked*, by
any bit any program can set, but it can be *unarmed*, once, at reset, until
software proves it owns a stack. One property is under program control and the
other is not.

Every real ROM's cold-start code does an `LDS` within its first handful of
instructions for exactly this reason — not out of good style, but because
until it does, the hardware will not listen to `NMI` at all. §4.5's boot trace
shows that instruction in the real CoCo 3 ROM, seven instructions in, and
exercise 4.5 asks you to find it there yourself.

---

## 4.4 SYNC and CWAI: the CPU as a small state machine

Up to now, `step()` has meant "fetch one opcode, execute it, return a cycle
count." Every instruction in Chapters 2 and 3 fits that description. `SYNC` and
`CWAI` are where it stops being true, because both of them do something no
other 6809 instruction does: they stop the CPU. Not a jump, not a loop — the
processor genuinely ceases fetching and waits for the outside world.

Modeling that requires the CPU to have a state beyond its registers, and
three states are enough:

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

The state transitions fit in six rows:

| From      | Event                                                        | To        |
|-----------|--------------------------------------------------------------|-----------|
| `Running` | `SYNC` (`$13`) executes                                       | `Syncing` |
| `Running` | `CWAI` (`$3C`) executes, full frame stacked                   | `Waiting` |
| `Syncing` | `irq()`/`firq()` called with that line **masked**             | `Running` (not serviced) |
| `Syncing` | `irq()`/`firq()` called unmasked, or an armed `nmi()`         | `Running` (serviced, vectored) |
| `Waiting` | `irq()`/`firq()` called unmasked, or an armed `nmi()`         | `Running` (serviced, no re-stack) |
| any       | `reset()`                                                     | `Running` |

Every one of those transitions is a line of code already quoted in this
chapter. The two `Running →` rows are `exec_interrupt_halt`'s `$13` and `$3C`
arms. The masked-`Syncing` row is the nested `if` inside `irq()`/`firq()`. The
last two serviced rows are `take_interrupt`'s closing `self.state =
State::Running`. And the `reset()` row is `reset()`'s fourth line.

While halted, `step()` checks the state before fetching anything:

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

While halted, `step()` degenerates to "advance the clock by one cycle and do
nothing else." No fetch, so `PC` does not move; no decode, so no instruction
runs; the return value is 1 rather than 0 so that a caller counting cycles
does not spin forever waiting for time to pass. That last point is not
cosmetic. Chapter 6's run loop budgets a fixed number of cycles per scanline and
runs `step()` until the budget is spent, so a halted CPU that reported zero
cycles would hang the emulator rather than the emulated machine. Returning 1
keeps the wall clock, the scanline counter, and eventually the interrupt that
ends the halt all moving.

Note also who ends the halt: nobody inside the CPU. `step()` cannot un-halt
itself, because nothing it does changes `state`. The wake-up has to arrive
through `nmi()`, `irq()`, or `firq()`, called from outside — which in the real
emulator means the bus, once a device asserts a line. A CPU in `Syncing` or
`Waiting` with no machine attached idles forever, correctly.

### `SYNC`: wake on interrupt activity

`SYNC` is the lighter of the two halts, and its implementation is the shortest
arm in `exec_interrupt_halt`: set `state` to `Syncing`, return 2 cycles, done.
No frame, no vector, no mask changes.

Its contract is to wake on *any* interrupt activity,
serviced or not. That is why the mask checks inside `irq()` and `firq()`, back
in §4.2, explicitly test `self.state == State::Syncing` and flip it back to
`Running` even on the branch where the line is masked and `false` is about to
be returned. A masked `IRQ` arriving during a `SYNC` does not get serviced —
no frame is pushed, `PC` does not move to any vector — but it does end the
wait, and execution resumes with the instruction immediately after the `SYNC`.

Consider a routine that needs to know when a device
becomes ready but has nothing useful to do until then. Polling in a tight loop
burns cycles and, on a machine sharing its bus with video hardware, burns them
at exactly the wrong time. Masking the line and executing `SYNC` gets the same
answer for free: the CPU stops, the device eventually twitches its line, the
CPU resumes at the next instruction, and the program does its own dispatch in
straight-line code with no handler, no frame, and no `RTI`. Wake on the event,
decide for yourself what it meant.

### `CWAI`: stack before waiting

`CWAI` — "clear condition codes and wait for interrupt" — is heavier, and the
weight is precisely the point. Its arm in `exec_interrupt_halt` does four
things in order. It fetches an immediate byte and ANDs it into `CC`, which is
how the instruction makes itself interruptible: the operand is a mask with `I`
and/or `F` zeroed, so the same instruction that halts the CPU also opens the
gate that will let something wake it. It forces `E` set. It calls
`self.psh(bus, 0xFF, true)`, stacking the full twelve-byte frame **right then,
before any interrupt has arrived**. And it sets `state` to `Waiting`.

That third step is the one that makes `CWAI` more than a two-instruction
macro. A program could write `ANDCC #$EF` followed by `SYNC` and get something
superficially similar — clear the mask, then halt. What it would not get is
the pre-stacked frame. When the interrupt finally arrives on that program's
machine, the CPU has to push twelve bytes before the handler's first
instruction can run, and those twelve bus writes are twelve cycles of latency
that the device has been waiting through.

`CWAI` moves that work earlier, into a moment when the CPU had nothing better
to do anyway. `take_interrupt` then skips it, and the skip is one line:

```rust
if self.state != State::Waiting {
    // ... stack the frame (§4.2) ...
}
```

With that condition, a `CWAI`'d CPU goes straight
to setting masks and loading `PC`, because the frame-sized part of the job
already happened during `CWAI`'s own 22 cycles — cycles that were spent while
the machine was idle rather than while a device was waiting. This is why
hard-real-time 6809 code prefers `CWAI` over `ANDCC`-then-`SYNC` when it knows
in advance that an interrupt is coming: disk-controller service routines,
tight audio bit-bangers, anything where the handler's start time matters more
than the total work done.

There is a subtler consequence, and it is the kind of thing that looks like a
bug until it is thought through. Because the `entire` logic lives *inside* the
same skipped branch, a `CWAI`'d CPU's `E` flag is never touched by the
interrupt that eventually wakes it. It stays at the `1` that `CWAI` set,
regardless of whether an `IRQ`, an `NMI`, or a normally-partial-frame `FIRQ`
is what fires. A `FIRQ` that wakes a `CWAI`'d CPU therefore leaves `E=1`
behind, which is the opposite of what a `FIRQ` does to a running CPU.

That is correct, and the reason is a matter of who committed first. `CWAI`
stacked twelve bytes before it knew which line would fire. The `E` bit
describes what is *on the stack*, not which exception occurred, so it must
keep saying twelve. The eventual `RTI` reads `E=1`, pulls twelve, and lands
`S` exactly back where `CWAI` found it. Had `take_interrupt` "helpfully"
cleared `E` for the `FIRQ` case, the `RTI` would pull three bytes off a
twelve-byte frame and leave nine bytes of garbage on the stack forever.
Exercise 4.4 asks for this as an assertion rather than a paragraph.

### Interrupt latency: what it costs to get there

"`CWAI` is faster" has been an assertion so far. Putting a number on it is a
useful exercise, and an even more useful one is discovering exactly how far
the number can be pushed using only this codebase as a source. The answer is
"not as far as you would like," and the reason is a lesson about cycle counts
in general.

The one piece of hard currency the code offers is `PUSH_PULL_BASE_CYCLES = 5`
([`lib.rs:108`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L108)),
the constant `psh` and `pul` add to their per-byte cost. Run it against the
full frame: five plus twelve bytes is 17. Compare that to `SWI`'s hard-coded,
data-sheet-matching total of **19** cycles from §4.1. The two numbers are
close but not equal. `SWI` also has to fetch its own opcode and read two bytes
from the vector, and the real 6809's cycle-by-cycle bus timing does not
decompose into a clean "base plus bytes" sum the way `psh`'s *model* of
`PSHS`/`PULS` cost does. The two-cycle gap is real interrupt overhead, roughly
the vector fetch, but it is not one that this codebase's own formula predicts.

Run the comparison in the other direction and the gap does not even point the
same way. `pul`'s formula also predicts five plus twelve, 17, for unwinding a
full frame, but `RTI`'s hard-coded full-frame cost is **15** — two cycles
*under* the formula rather than over. Push and pull are not mirror images of
each other in the real chip's timing, and neither one is a pure function of
`PUSH_PULL_BASE_CYCLES`.

The lesson generalizes well past this chapter: **cycle counts are empirical
facts about specific silicon, not something you can always derive from a clean
formula.** The `psh`/`pul` formula is a model, accurate for the explicit
`PSHS`/`PULS`/`PSHU`/`PULU` opcodes it was built to cost in Chapter 3, and it was
never claimed to explain every stack-touching operation on the chip. The
hard-coded literals in `exec_interrupt_halt` are the tell: whoever wrote that
function knew the formula would not reproduce 19 and 15, so they did not try
to make it. When a data sheet and your model disagree, the data sheet is the
territory.

With that caveat on the table, the *shape* of the comparison still holds up,
and it is worth stating in three parts.

`IRQ` and `NMI` sit in the same neighborhood as `SWI`'s 19. They make the
same `psh(bus, 0xFF, true)` call and the same vector fetch, and they have no
opcode byte to fetch — a hardware line does not get decoded — so if anything
the real number is a shade under 19 rather than over.

`FIRQ` is the same operation with a quarter of the bytes moved. Three bytes
instead of twelve, on a chip where each byte is a bus cycle, plausibly lands
under half of `IRQ`'s cost. The whole reason the 6809 has two hardware
interrupt lines instead of one is this gap, and the gap is entirely a byte
count.

A `CWAI`'d CPU waking up pays neither of those costs again, and the *shape* of
what is left is more informative than its size. Look at what remains inside
`take_interrupt` once the `if self.state != State::Waiting` guard is skipped:
two `if`-gated `|=` operations on `CC` and a single `bus.read_u16(vector)`.
That is a small, fixed amount of work, not scaled by frame size at all,
because the frame-sized part already happened during `CWAI`'s own 22 cycles.
Flat, tiny, and independent of which line eventually fires — that is the
concrete version of the "twelve bus writes already spent" claim, and it is why
`CWAI` is the right tool for code that already knows an interrupt is imminent.

### Implementation limitation: external interrupt cycles

The current core does not add cycles for externally delivered interrupts.
Search `take_interrupt`,
`nmi`, `irq`, and `firq` for any write to `self.cycles` and there is not one.
The field is touched in exactly two places in the entire crate, both inside
`exec.rs`'s `step()`:

```rust
// crates/mc6809/src/exec.rs:31-36, 119
if self.state != State::Running {
    self.cycles += 1;
    return 1;
}
// ...
self.cycles += cycles as u64;
```

The first is the halt-state idle tick this section opened with. The second is
`step()`'s own bookkeeping after dispatching an opcode, and it is how
`SWI`, `SWI2`, `SWI3`, `RTI`, `CWAI`, and `SYNC` get their hard-coded costs
onto the clock — because those six are *opcodes*, decoded and executed through
`step()` like any other instruction.

`nmi()`, `irq()`, and `firq()` are not opcodes. They are public methods the
machine calls directly from outside `step()`
([`crates/coco-core/src/machine/run.rs:111,178,181`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs),
Chapter 6), and `take_interrupt`'s body — the function all three share — never
once mentions `self.cycles`. `psh`'s return value, the one thing in this whole
area that does compute a byte-accurate cost, is discarded every time
`take_interrupt` calls it: `self.psh(bus, 0xFF, true);` with no assignment, no
`+=`, nothing. The CPU's cycle counter therefore omits the entry cost of an
externally delivered interrupt.

> **Rust corner: a return value nobody reads.** `psh` is declared
> `fn psh(&mut self, bus: &mut impl Bus, mask: u8, to_s: bool) -> u32` and its
> last line is `PUSH_PULL_BASE_CYCLES + bytes`. Two of its call sites —
> `PSHS` and `PSHU` in `exec_control_transfer` — use that value as the
> instruction's cycle count, returning it straight up the stack. The two
> inside `take_interrupt` write `self.psh(bus, 0xFF, true);` and throw it
> away.
>
> Rust compiles that silently. Unused *variables* draw a warning, unused
> *return values* generally do not, because discarding a result is often
> exactly what a caller means. The opt-in is `#[must_use]`: annotate the
> function and every call site that ignores its result gets a warning until
> it says `let _ = ...` to make the discard explicit. `Result` carries the
> attribute for this reason, which is why ignoring an error draws a warning
> while ignoring an integer does not.
>
> Would `#[must_use]` be right here? It would have turned this section's
> finding into a compiler diagnostic on the day the code was written, which
> is the strongest argument available for any lint. It would also produce two
> `let _ =` lines whose meaning is "the interrupt path deliberately does not
> cost this," which is real information — the discard is a decision, and
> right now the only place that decision is recorded is in this book. The
> general habit is worth adopting: when a function returns something a caller
> could plausibly forget, make forgetting it a deliberate act.

This is a specific timing limitation, not evidence that the interrupt state
transitions are incorrect. A trace comparison against an implementation that
charges interrupt entry may therefore disagree on cycles while the register
state remains aligned. Whether to add that accounting is a fidelity decision
that should be verified against the processor timing documentation and the
machine scheduler.

### FIRQ inside an IRQ handler: nesting and the E flag

One more question `take_interrupt`'s design answers, if it is traced through
carefully: what happens when a second interrupt line asserts *while the CPU is
already running a handler for the first one*, before that handler's own `RTI`?

Look back at §4.2's mask table with that question in mind. `irq()` calls
`take_interrupt(bus, VECTOR_IRQ, true, false, true)`, and `set_f` is `false`.
That is not an oversight; it is the whole answer. An `IRQ` handler runs with
`I` set, so a second `IRQ` cannot preempt it, and with `F` untouched — if `F`
was clear before the `IRQ` fired, it is still clear once the handler starts.
The test that pins this down asserts it directly:

```rust
// crates/mc6809/tests/interrupts.rs:199-213
#[test]
fn irq_serviced_when_unmasked() {
    let mut s = Sys::new();
    s.bus.load(0xFFF8, &[0x80, 0x00]); // IRQ vector -> $8000
    s.cpu.pc = 0x1234;
    s.cpu.s = 0x2000;
    s.cpu.cc = 0x00;
    let serviced = s.cpu.irq(&mut s.bus);
    assert!(serviced);
    assert_eq!(s.cpu.pc, 0x8000);
    assert_eq!(s.cpu.s, 0x2000 - 12); // full frame
    assert_ne!(s.cpu.cc & cc::IRQ_MASK, 0); // I set
    assert_eq!(s.cpu.cc & cc::FIRQ_MASK, 0); // F left alone by IRQ
    assert_ne!(s.cpu.cc & cc::ENTIRE, 0);
}
```

`assert_eq!(s.cpu.cc & cc::FIRQ_MASK, 0)` is the load-bearing line, and it is
an assertion about something that did *not* happen. Tests that check for
absence are easy to leave out and are usually the ones that catch a
well-meaning "fix" years later.

With `F` still clear, a `FIRQ` line asserting during that handler sails
straight through `firq()`'s mask check exactly as if no interrupt were in
progress. `take_interrupt` does not inspect what is already on the stack — it
has no way to, and no need to — so it pushes `FIRQ`'s three-byte partial frame
*on top of* `IRQ`'s already-stacked twelve-byte frame, at whatever `S`
currently is, and vectors to the `FIRQ` handler.

Run that forward with real numbers. `S` starts at `$3000`, `CC` at `$00`, both
lines unmasked. `IRQ` fires, stacks the full frame, `S` becomes `$2FF4`, and
`CC` becomes `I=1, F=0, E=1`. While the `IRQ` handler runs with those masks,
`FIRQ` fires: it is unmasked, so it is serviced, and it stacks its partial
frame on top — `S` becomes `$2FF1`, `CC` becomes `I=1, F=1, E=0`. The stack
now holds two complete, independently shaped frames, one nested inside the
other, and nothing in `take_interrupt` had to know that.

When the `FIRQ` handler finishes and executes `RTI`, it pulls `CC` first, sees
`E=0`, and pulls just `PC`: three bytes total, `S` back to `$2FF4`, `PC` back
to wherever the `IRQ` handler was executing. And — this is the part worth
sitting with — `CC` is restored to exactly `I=1, F=0`, the state that was true
the instant `FIRQ` preempted. Control resumes *inside* the `IRQ` handler with
`FIRQ` unmasked again, ready to be preempted a second time if the device asks;
and that handler's own eventual `RTI` unwinds its full frame the normal way,
`E=1`, twelve bytes, back to whatever was interrupted in the first place.

Nesting works cleanly, to arbitrary depth, bounded only by masks and stack
space. It works for exactly two reasons, both of which are properties of code
already read in this chapter: `RTI` only ever looks at the byte on *top* of
the stack and never anything deeper, and each `take_interrupt` call is
self-contained about the frame it produces. Neither function has any notion of
nesting, which is why nesting is not a special case.

The reverse direction is blocked, and now the reason is visible rather than
asserted. A `FIRQ` handler runs with **both** `I` and `F` set, since `firq()`
passes `set_i: true, set_f: true`, so an `IRQ` line asserting mid-`FIRQ`-handler
hits `irq()`'s mask check, and that call simply returns `false` — unserviced,
exactly as if `I` had been set by any other means. Whether that line gets
tried again is a bus-level question for Chapter 6, not something `irq()` itself
tracks; from the CPU's side, `FIRQ` can always preempt `IRQ`, and `IRQ` can
never preempt `FIRQ` unless the `FIRQ` handler explicitly clears `I` itself
before its `RTI`. That asymmetry is the entire point of calling one of them
"fast."

`NMI` sits above both. `nmi()` checks no mask bit at all before calling
`take_interrupt` — only `nmi_armed` gates it, per §4.3 — so it can interrupt
`IRQ` handlers, `FIRQ` handlers, or anything else, non-maskable exactly as
advertised. Exercise 4.9 asks you to write the test that proves the
`IRQ`-with-nested-`FIRQ` sequence above, with the exact register values,
rather than trust this paragraph.

That is the interrupt subsystem, complete. What remains is the harder half of
the week: knowing whether any of it is right.

---

## 4.5 The 6809 testing problem

This repository does not have a standard per-instruction 6809 conformance
suite comparable to those available for several other processors.

Without that coverage, unusual flag combinations and addressing cases require
several independent validation methods.

[`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5 lays
out a three-legged reply to this, and this codebase takes each leg seriously.
None of the three is sufficient alone, which is exactly why there are three:
one gives breadth against a trusted reference, one gives independence from
your own assumptions, and one gives fast, readable failures on the corners you
already know are sharp.

### Leg 1 — trace-diff against a reference emulator

The first leg is the most powerful and the least glamorous. Boot the exact
same ROM from the exact same reset vector in this emulator and in a reference
implementation — XRoar or MAME, both of which have had 6809 cores hammered on
for decades by a larger community than one codebase can muster — and compare a
per-instruction trace, register by register.

With identical inputs and event schedules, the first trace divergence
localizes the first observable disagreement. The responsible cause lies after
the last matching state and no later than the first mismatching one. Reference
bugs and differences in memory, timing, or trace alignment must still be ruled
out.

This only works if both sides produce a trace in the same shape, so this
codebase carries the machinery as first-class infrastructure rather than as a
debugging script somebody keeps in a scratch directory. The trace ring's entry
type lives in the debugger core:

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

Nine fields, every register in the machine, and one word in the doc comment
that determines how every trace in this chapter reads: **BEFORE**. Each line
is the register file as it stood *before* the instruction at that `PC` ran, so
the effect of an instruction shows up on the *following* line. Getting that
backwards makes every trace look off by one and every diff blame the wrong
instruction, which is why the comment shouts it.

The entries live in a capped ring held by the `Debugger`, 1024 deep by
default:

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

The capacity is a deliberate trade. A ring keeps the *last* 1024 instructions
rather than the first, which is what a post-mortem wants — when something goes
wrong, the interesting history is the recent history. And `trace_enabled`
defaults to `false` because capturing nine registers per instruction is real
overhead on a "just run the machine" path that does not want it.

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

The live ring is for the interactive debugger of Chapter 16. For a bulk
trace-diff against MAME there is a standalone program,
[`examples/trace.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/trace.rs),
71 lines long, which walks a real boot and emits one formatted line per
retired instruction. Both paths share a single `format()` method, so there is
exactly one line format in the project to keep in sync with MAME's:

```rust
// crates/coco-core/src/debug.rs:168-176
pub fn format(&self) -> String {
    format!(
        "{:04X}:  A={:02X} B={:02X} X={:04X} Y={:04X} U={:04X} S={:04X} DP={:02X} CC={:02X}",
        self.pc, self.a, self.b, self.x, self.y, self.u, self.s, self.dp, self.cc
    )
}
```

Fixed-width hex everywhere, no thousands separators, no variable-length
fields. That is not typography; it is a diff requirement: `diff` compares
lines as text, so a register that prints as `A=F` on one line and `A=0F` on
another produces a spurious mismatch. Padding every field to its full width
means two lines differ if and only if the machines differ.

The program's no-cart mode is deliberately austere. It runs raw `m.step()`
with **no interrupts, no scanline timing, nothing but the CPU executing ROM**,
which isolates exactly what should be validated first: does the decoder
produce the right registers for the right bytes? Only once that is clean does
a second mode reintroduce interrupts, hsync, and the GIME timer via
`Machine::step_instruction()`, for a fuller diff against a specific MAME
`-cart1` run. Two modes, in difficulty order, so a failure in the first is
never confounded by the second's timing.

### A worked trace comparison

Here is real output, from actually running `cargo run -p coco-core
--example trace -- 13` against `roms/coco3.rom` — the first thirteen
instructions the CoCo 3's real Super Extended Color BASIC executes after
reset. Each line is the register file *before* that PC's instruction runs
(per `TraceEntry`'s own doc comment, §4.5 above). Each PC has been
disassembled by hand against the same ROM bytes (verified with
`mc6809::disasm::disassemble`, the Chapter 3 disassembler, pointed at the
file), and the trace annotated with the mnemonic that runs *at* that line:

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
  behavior — it's the ROM being defensive — but recognizing that as
  *expected* depends on knowing what `reset()` does.
- `LDS #$5EFF` at `$C002` is the instruction that arms `nmi_armed` for
  the rest of the boot — the answer to exercise 4.5 below, from §4.3:
  watch `S` jump from `$0000` to `$5EFF` between the `$C002` line and the
  `$C006` line, which is `load_s()` firing.
- `CC` flips from `$50` to `$58` at `$C00D`, right after `LDX #$FFB0`
  executes — `$FFB0` as a signed 16-bit value has its top bit set, so
  `LDX` sets `N` (Chapter 2's flag rules, still paying rent). Three lines
  later, once `STA ,X+` stores the *positive* byte `$12`, `N` clears
  again and `CC` drops back to `$50`. A flag that appears and disappears
  for one line is completely explained by the one instruction between —
  which is exactly the discipline trace-diffing rewards.

That second bullet deserves one more beat, because it is this chapter's two
halves meeting. §4.3 argued from a data sheet that no real ROM can afford to
run for long without an `LDS`, since until it does the machine is deaf to
`NMI`. Seven instructions into a real 1986 ROM, there it is: `LDS #$5EFF`,
before the palette is touched, before the MMU is configured, before anything
else of consequence. The rule and the artifact agree, and neither one was
consulted when writing the other.

Now put yourself in the trace-diff seat for real. Suppose a MAME reference
trace of the same boot matched this output line for line up through
`$C00A`, and then at `$C00D` MAME showed `CC=50` where this trace shows
`CC=58` — everything else on that line identical. You would not need to
re-read the whole CPU core. The only instruction that ran between the two
identical `$C00A` states and the `$C00D` line is `LDX #$FFB0` — nothing
else touched a register in between — so the bug is narrowed, with zero
ambiguity, to `LDX`'s flag computation for a negative 16-bit immediate.
That's the entire method: **find the first line where two otherwise-identical
traces disagree, and the previous instruction is guilty.** No guessing
which of a few thousand possible opcodes to suspect; the trace itself names
the defendant.

### Following the ROM deeper, against a real disassembly

The thirteen lines above aren't a guess at what the ROM does. They were
cross-checked, mnemonic for mnemonic, against
`docs/super-extended-basic-unravelled.pdf`'s Appendix B, a hand-annotated
disassembly of this exact ROM by Walter K. Zydhek. The `docs/` directory is
gitignored and copyrighted, so that file is not part of the repository, and it
isn't required reading either: every label and comment quoted below is
reproduced verbatim, so the PDF isn't needed to follow along. That reference
gives real labels for the code this chapter has been tracing blind:

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
Chapter 8 story (the GIME palette); for this chapter, the label is enough to
confirm the trace is reading the ROM correctly, which is the whole point of
cross-checking against an independent source.

The two sources are independent. The trace came from
executing bytes through this emulator's own decoder. The listing came from a
human reading the same bytes with a data sheet in 1993. If this emulator's
`LDS` decoder were wrong, the trace would show a different mnemonic or a
different `S` value, and the disagreement would be immediate and obvious.
Agreement across two paths that share no code is the closest thing to proof
available without a conformance suite — which is, in miniature, exactly leg
1's argument.

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
signed offset (`557,PCR`, Chapter 3's PC-relative addressing) rather than a
resolved target address — SEB Unravelled's listing resolves the same
instruction to `MMUIMAGE,PC` because a human annotator did the arithmetic
by hand. Both are correct; they're just different jobs. Do the arithmetic
yourself and they agree: `$C019` (the address right after this 4-byte
instruction) `+ 557 ($22D) = $C246`, exactly the `Y` value the trace shows
at the next line, exactly SEB's `MMUIMAGE` table address.

`LDA ,Y+` at `$C01B` pulls one byte from that table (`A` becomes `$38`, the
table's first entry) and `STA ,X+` writes it to `$FFA0`, the first of sixteen
MMU registers. What those registers actually *do* is Chapter 5's MMU story rather
than this chapter's, but the pattern is already visibly identical to the
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
this chapter isn't unpacking; that's Chapter 5's `INIT0` register story, not
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

The trace stops here because `ENDMOVE` and `BEGMOVE`
are both labels with fixed addresses in the same listing — `ENDMOVE =
$C36C`, `BEGMOVE = $C03F` — so the amount of code this loop relocates is
computable without stepping through it at all: `$C36C - $C03F = $32D =
813` bytes, moved four bytes per iteration. Since 813 isn't a multiple of 4,
the loop overshoots slightly on its last pass, landing on 204 trips around
a six-instruction loop rather than 203. Reaching the `JMP $4000` at `$C03C`
therefore takes well over a thousand trace lines, all of them the identical
`LDD`/`LDU`/`STD`/`STU`/`CMPX`/`BCS` shape already shown in one iteration
above. A real trace-diff session wouldn't read those thousand lines by eye
either — you'd script the diff, or set a breakpoint past the loop and only
compare state from there, the same instinct that makes `$C03C`, not `$C020`,
the honest edge of what an "extended worked example" should reproduce
line-by-line in a book.

That instinct generalizes into the practical shape of a real trace-diff
session. Machine-diff the whole file to find the first divergent line; read by
eye only the twenty lines around it. Straight-line setup code is where reading
by eye pays, because every line does something different. Loop bodies are
where scripting pays, because the thousandth iteration is not more informative
than the second — unless, of course, the divergence is *in* the thousandth
iteration, in which case the diff will say so and the eye can go there
directly.

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

This is also why `trace.rs`'s no-cart mode can get away with delivering no
interrupts at all and still produce a trace that matches MAME line for line.
For this stretch of the boot, a machine that services interrupts and a machine
that has none are executing the same instructions, because the ROM has masked
both lines and has not yet unmasked them. The austere mode is not an
approximation here; it is exact, right up to the moment BASIC changes its
mind.

### Leg 2 — a self-checking exerciser ROM

Leg 1 compares this emulator against another emulator, which is powerful and
has one structural weakness: it validates against somebody else's opinion of
the 6809. If MAME and this codebase happened to share a misreading of the data
sheet, a trace-diff would report perfect agreement.

The second leg closes that gap by running an existing, independent test
*program* — one that doesn't know or care what emulator it's running on — and
trusting its verdict. [`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md)
§5 names a concrete one:
[flexemu's `cputest.txt`](https://github.com/aladur/flexemu/blob/master/src/tools/cputest.txt)
by W. Schwotzer, tested on a real SGS-Thomson EF6809P processor. That last
detail is the leg's entire value: the expected answers in that file were
checked against silicon, not against a data sheet's prose. It is worth looking
at what's actually in the file rather than trusting the one-line description.
The file is fifty-eight `JSR`s deep, and its header reads:

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
submode Chapter 3 called "the single hardest 200 lines in the CPU," each one
independently checked against a real chip's answer rather than against one
codebase's opinion of what the data sheet means.

The second block, thirty routines long, is instruction-family coverage, in
the file's own order: `TNEG`, `TCOM`, `TDEC`, `TINC`, `TCLR`, `TADD`,
`TADDD`, `TADC`, `TMUL`, `TSEX`, `TSUB`,
`TSUBD`, `TSBC`, `TDAA`, `TCMP`, `TCMPD`, `TTST`, `TBIT`, `TLSR`,
`TLSL`, `TASR`, `TROL`, `TROR`, `TLD`, `TST`, `TLDD`, `TSTD`, `TLEA`,
`TTFR`, `TEXG` — `TFR`'s register-pair encodings (Chapter 3's other hard
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

Wiring this into a Rust test suite is where theory meets an obstacle, and the
obstacle is not the one people expect. `cputest.txt` is written for FLEX, not
for a bare `FlatBus`. Its header defines `WARMS EQU $CD03`, `PUTCHR EQU
$CD18`, `PSTRNG EQU $CD1E`, `PCRLF EQU $CD24`, and `OUTDEC EQU $CD39` — five
fixed addresses where FLEX's own ROM provides character output, string output,
decimal output, and a "return to the monitor" entry point. `OUTERR` above
calls straight into one of them, `PSTRNG`. A `FlatBus` is a bare 64K array
initialized to zero; nothing lives at `$CD18`, so running this file's raw
machine code against it today would `JSR` into uninitialized memory and
immediately go off into the weeds. The gap is not test coverage, and it is not
addressing modes. It is that this file assumes an operating system this crate
does not have.

It's a small gap to close, though, precisely because the CPU crate
already has everything needed: a `Bus` is just `read`/`write`, and a test
harness controls both sides. The sketch: assemble `cputest.txt` (any 6809
cross-assembler that accepts the FLEX-flavored syntax) into a flat binary,
load it into `FlatBus` at `$8100` (its own `ORG`), and instead of teaching
`FlatBus` to *be* FLEX, patch the five vector addresses in RAM with a
one-byte marker opcode `step()` doesn't otherwise produce. Then drive the
loop from Rust. Call `s.step()` repeatedly and, after each step, check whether
`pc` landed on one of the five patched addresses. If it did, either do
nothing and simulate an `RTS` — the four output routines need nothing more,
since a headless run doesn't have to render "All Tests succeded" to a
terminal — or, for `WARMS`, stop the loop, because the test program is
signaling it's done. The entire verdict is then one assertion:
`assert_eq!(sys.bus.mem[ERRFLG_ADDR], 0)`. No FLEX emulation, no output
rendering, just enough of a stub to keep the test program from running off the
rails when it tries to act polite about its own results.

That sketch is also a small demonstration of Chapter 1's argument about the
`Bus` trait paying rent. Because the seam is two methods and the test harness
owns the whole address space, "emulate just enough of an operating system to
keep a 2003 test program happy" is a page of Rust rather than a project.

This is still undone — there is no `cputest.txt` in this repository's tree
today, and no such harness. [`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5's recommendation stands
un-executed. That's a real, specific gap, and it's exactly the kind this
course wants you to be able to name precisely (a five-address I/O stub,
not a rewrite) rather than wave at vaguely. Exercise 4.10 is this project,
scoped down to a size you can actually finish in one sitting.

### Leg 3 — hand-written corner tests

The third leg is the one you can inspect directly right now, and the one you
have been reading all chapter. `crates/mc6809/tests/interrupts.rs`,
[`indexed.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/indexed.rs),
[`stack.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/stack.rs),
and friends are tests written by a human who read the data sheet (or the
reference PDFs in `docs/`) and encoded known-tricky corners as assertions:
indexed postbyte submodes from Chapter 3, `TFR`/`EXG` register encodings,
`FIRQ`-versus-`IRQ` stacking, `CWAI` and `SYNC`.

The file this chapter draws on says as much in its own header, which is a
compact statement of the leg-3 philosophy:

```rust
// crates/mc6809/tests/interrupts.rs:1-7
//! Test-driven coverage for the misc inherent ops (ORCC/ANDCC/SEX/ABX/MUL/DAA)
//! and the interrupt / halt subsystem (SWI/SWI2/SWI3, RTI, CWAI, SYNC, and the
//! external NMI/IRQ/FIRQ delivery API).
//!
//! Frame conventions verified against the reference: full frame pushes
//! PC,U,Y,X,DP,B,A,CC with E=1; FIRQ pushes only CC,PC with E=0. IRQ sets I;
//! FIRQ and NMI set I+F; SWI sets I+F; SWI2/SWI3 leave the masks alone.
```

Three lines of prose that are, word for word, §4.1 and §4.2 of this chapter.
That is what "the tests are the specification" means in practice: the
hardware fact, the doc comment, and the assertion are the same statement in
three registers.

These tests lack leg 1's breadth and leg 2's independence — they can only
check what their author already suspected — but they are fast, they run in CI
on every commit, and they fail with a specific, readable assertion rather than
"somewhere in a million-instruction trace, something differs." When a
refactor breaks the `CWAI` fast path, `cargo test -p mc6809 --test interrupts`
names the test and prints two stack-pointer values four seconds later.

Carry all three to whatever CPU you emulate next, even one with a JSON suite.
Diff against a trusted reference from an identical starting state, because the
first divergence localizes the bug to one instruction. Run independent,
self-checking test software when one exists, because it does not share your
misreadings. Hand-encode the corners you already know are sharp, because they
fail fast and legibly. A JSON conformance suite, where it exists, is leg 2 at
industrial scale — it does not replace the other two.

---

## 4.6 Reading three real tests

Everything in §4.1 and §4.4 is provable rather than a matter of faith, and
the proofs are short enough to read in full. Here are three tests from
[`crates/mc6809/tests/interrupts.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/interrupts.rs),
walked line by line: one for the partial frame from §4.1, one for `CWAI`'s
state machine from §4.4, and one for `SYNC`'s. Read them as leg 3 in action,
and also as models — each one is a template for a test you could write about
a different chip tomorrow.

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

Notice what this test does not do: it never calls `step()`. There is no
opcode anywhere in it, because `FIRQ` is not an instruction — it is a pin.
The test drives `firq()` directly, which is the payoff of a `Bus`-generic,
machine-agnostic CPU: the external-interrupt API can be exercised in complete
isolation, with no opcode fetch, no machine, and no timing, just the two
things the real `SystemBus` will eventually hand it from `run.rs` — itself and
a bus.

The setup is three assignments and one `load`. `s.bus.load(0xFFF6, &[0x70,
0x00])` plants two bytes at the `FIRQ` vector address, which is exactly what
`take_interrupt`'s `bus.read_u16(vector)` will read; `$70 $00` big-endian is
`$7000`. `pc = 0x1234` and `s = 0x2000` give the frame something recognizable
to save, and `cc = 0x00` means every bit that ends up set was set by the code
under test.

Then one call, and five separate facts checked after it. The vector was taken
(`pc == 0x7000`). Exactly three bytes of stack moved (`s == 0x2000 - 3`), not
twelve. Both masks came up, so no further interrupt of either kind can
preempt this handler. And the one byte that *did* get pushed — `CC`, at
`$1FFD`, per §4.1's partial-frame table — has `E` clear, which is what will
later tell `RTI` to pull three bytes rather than twelve. Those five assertions
together are a complete, self-contained specification of "partial frame."
Delete the implementation, hand someone this test, and they could rebuild it.

### `cwai_stacks_frame_then_interrupt_skips_restacking`

The `CWAI` optimization from §4.4 is an absence — a push that does *not*
happen — and absences are the hardest thing to test. This test does it by
capturing a value before and comparing after:

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

Read it in two halves, because it exercises `step()` *and* the external API in
sequence — which is unusual, and is exactly right for an instruction whose
effect only becomes visible when something outside the CPU responds to it.

The first half runs `CWAI #$EF` through `s.step()` like any ordinary
instruction. Starting from `cc = 0xFF`, every bit set including both masks,
makes the `ANDCC`-style clearing visible: `cc & IRQ_MASK == 0` afterward
proves the `#$EF` operand — `1110_1111`, with `I`'s bit zeroed — did its job.
`s_after_cwai` captures the stack pointer right there, twelve bytes below
where it started, which is the full frame stacked before any interrupt
exists. `cycles == 22` matches the hard-coded return value from §4.1's
`exec_interrupt_halt` block. And `state == State::Waiting` says the CPU is
now halted: the *next* `s.step()`, which this test deliberately never calls,
would just burn one cycle per §4.4, forever, until something outside calls
`irq()`, `firq()`, or `nmi()`.

The second half is the load-bearing part, and it rests on one equality.
Calling `irq()` directly — `I` is clear now, so it is serviced — and then
asserting `s.cpu.s == s_after_cwai`, *unchanged*. Not "twelve less than
`$2000`", which would also be true and would also pass if the guard were
broken in some other way, but equal to the value captured before the
interrupt. If `take_interrupt`'s `if self.state != State::Waiting` guard were
ever deleted, `S` would drop by another twelve as a second frame stacked on
top of the first, and this single equality would catch it immediately with a
concrete, readable number mismatch. Exercise 4.7 asks you to go delete that
guard and watch it happen.

### `sync_halts_and_idles_until_interrupt`

`CWAI`'s test proves a frame gets pre-stacked and not doubled. `SYNC`'s test
has a different job: proving a masked interrupt can wake the CPU *without*
servicing it. That is the behavior exercise 4.2 asks you to predict before
reading any code, and it is the one people most often get wrong on the first
guess, because "masked" sounds like "ignored."

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

Four `step()`/`irq()` calls, four separate facts, in order.

The first `s.step()` executes the `SYNC` opcode itself. No frame is stacked,
no vector is loaded; `state` simply becomes `Syncing`, matching §4.1's
one-line `exec_interrupt_halt` arm exactly. `pc_after_sync` is captured
*after* `SYNC` retires, so it is the address of the `NOP` that follows rather
than of `SYNC` itself — `step()` advanced `PC` past the one-byte opcode before
dispatching it, which is the fetch-execute rhythm from Chapter 1 showing up in an
assertion.

The second call is a `step()` while `Syncing`. This is the `if self.state !=
State::Running` branch from §4.4's `step()` excerpt, and both of its
observable effects get checked: it returns `1`, and it leaves `PC` exactly
where it was. No opcode fetch happened. The halt really does nothing but tick
the clock, and a test that only checked the return value would not have proved
that.

The third is the one this test exists to nail down. `cc = cc::IRQ_MASK` sets
`I`, explicitly masking `IRQ`, and then `irq()` is called. `assert!(!serviced)`
— the mask worked, no frame was stacked, no vector was taken, exactly as
`irq()`'s masked branch specifies in §4.2. And yet `assert_eq!(s.cpu.state,
State::Running)` on the very next line: the CPU woke up anyway. Both are true
simultaneously because they are checking two different effects of the same
four lines inside `irq()`:

```rust
if self.cc & cc::IRQ_MASK != 0 {
    if self.state == State::Syncing {
        self.state = State::Running;
    }
    return false;
}
```

The mask check and the `Syncing` wake-up check are two separate `if`s, not
one. Masking blocks `take_interrupt`, which is why `serviced` comes back
`false`, but the `state` flip happens unconditionally inside the masked
branch, before the early `return`. A single combined condition would have been
the natural thing to write and would have been wrong, and this test is the
reason nobody will ever "simplify" it that way undetected.

Fourth and last, a final `s.step()` runs the `NOP` that was sitting right
after `SYNC` the whole time. `PC` lands at `pc_after_sync + 1`, proving
execution really did resume at "the instruction after `SYNC`" rather than at
some interrupt vector.

Four assertions, each isolating a different clause of `irq()`'s eight-line
body. That is what "the test *is* the specification, in executable form" looks
like up close — and it is the standard the exercises below hold you to.

---

## 4.7 Reading assignment

Read these in roughly this order; the first four are the chapter's core and
the rest fill in the testing half.

1. **[`crates/mc6809/src/lib.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L110-L260), lines 110–260** — vector constants, `State`
   enum, `nmi_armed`'s doc comment, `reset()`, `load_s()`,
   `nmi()`/`irq()`/`firq()`, `take_interrupt()`: the entire interrupt
   subsystem in one contiguous read. It is 150 lines. The whole of §4.1
   through §4.4 is in there.
2. **[`crates/mc6809/src/exec.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs)** — `exec_interrupt_halt`
   (`SWI`/`RTI`/`CWAI`/`SYNC`) and the `SWI2`/`SWI3` arms inside
   `exec_page10`/`exec_page11`. Read the two prefix-page arms right after the
   base-page one and the "same opcode byte, three vectors" point makes itself.
3. **[`crates/mc6809/src/stack.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/stack.rs)**, all 76 lines — `psh`/`pul` back every
   frame here, plus `PSHS`/`PULS` from Chapter 3. Read `pul` beside `psh` and
   check that the orders really are exact inverses.
4. **[`crates/mc6809/tests/interrupts.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/interrupts.rs)**, all of it — every test in it is
   a claim this chapter makes, turned into an assertion. Three of them are
   walked line by line in §4.6; the rest reward the same treatment.
5. **[`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5**, the testing-strategy paragraphs — the source
   the three legs of §4.5 come from, in its original compressed form.
6. **[`crates/coco-core/src/debug.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/debug.rs)**, around line 186 (`Debugger` and its
   `trace` field) plus `TraceEntry` and `export_trace`. This is Chapter 16's
   material arriving early because leg 1 needs it.
7. **[`crates/coco-core/examples/trace.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/trace.rs)**, the whole file (71 lines) —
   short enough to read in one sitting, and the tool every trace in §4.5 came
   out of.
8. *Optional, where a local copy is available:* `docs/super-extended-basic-unravelled.pdf`
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

Several exercises require a written prediction before execution. Compare the
prediction with the observed result before revising the explanation.

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
`CWAI`'d CPU with `firq()` instead of `irq()`; ensure `F` is clear so the line
is serviced. Assert that (a) `S` does not move a second time, and (b) the
stacked `CC` byte still has `E=1`, even though `FIRQ` normally creates a
partial frame. This verifies that `CWAI`'s existing frame determines what
`RTI` will restore.

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
6809 test program that sets a scratch byte to `1`, `JSR`s to your fake
`PSTRNG` address, and then `JSR`s to your fake `WARMS` address. Drive it with
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

Chapter 5 leaves the CPU crate and implements the machine around its `Bus`
interface.

We open [`coco-core/src/bus.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs)
and answer a question assumed by this chapter's vector table:
when the CPU reads `$FFFE`, what actually intercepts that read before it
becomes a plain RAM access? Every exception in this chapter ended with
`bus.read_u16(vector)`, and every one of them took for granted that those
fourteen bytes at the top of memory would come back from ROM rather than from
whatever the MMU had most recently mapped there. Making that true is the first
thing Chapter 5 has to get right.

The decode order handles hardwired vectors first, followed by the I/O page,
ROM, and the GIME's MMU. It also separates the CoCo 1/2's SAM translation path
from the CoCo 3's GIME path.
