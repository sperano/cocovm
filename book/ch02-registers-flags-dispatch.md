# Chapter 2 — CPU core I: registers, flags, dispatch, simple addressing

*Week 2. Goal: read `MC6809::step()` and know where every opcode goes. Last
week you saw the `MC6809` struct from the outside — a bag of registers behind
a `Bus` trait. This week you open it up. By the end you will be able to take
any 6809 mnemonic at all — `LDA $0400`, `CMPA #$0D`, `ASL ,X` — and point at
the exact line of Rust that runs when the real chip would run it, and predict
every flag it leaves behind without running the emulator to check.*

---

Chapter 1 made a claim that deserves to be cashed in immediately: that the CPU
is the *easy* part of an emulator, roughly three weeks of careful
table-copying from a datasheet, and that the GIME will take longer. This
chapter is where that claim gets tested. The MC6809 is generally regarded as
the most elegant 8-bit processor Motorola ever shipped — two accumulators that
pair into a 16-bit one, four 16-bit pointer registers, a relocatable direct
page, position-independent addressing modes, and an instruction set orthogonal
enough that assembly written for it reads almost like a high-level language.
All of that elegance has to end up in Rust this week, and the surprise is how
little Rust it takes.

The reason it takes so little is worth naming before you read any of it. An
instruction, on any processor, decomposes into three independent questions.
*Where does the operand come from?* That is the addressing mode. *What
arithmetic or logic is performed?* That is the operation. *What does the
result do to the condition codes?* That is the flag rule. On a badly organized
CPU emulator, all three answers get re-derived inside every one of the
two-hundred-odd opcode handlers, and the result is thousands of lines of
near-duplicate code in which exactly one arm has a typo that nobody finds for
a month. On a well-organized one, each answer is written down exactly once and
then combined. This chapter is a tour of the three places this codebase writes
them down: `addressing.rs` for where operands come from, `exec/exec_data.rs`
for what the operations do, and `alu.rs` for what happens to the flags.

When you finish, you will have read the entire opcode dispatch table — all of
it, not a representative excerpt — and hand-traced three instructions end to
end, counting cycles and flags on paper and then checking against real tests.
There is nothing to build this week. The reward for reading carefully is that
when Chapter 4's interrupt code goes wrong, and it will, you will be able to rule
the ALU out in a minute rather than an afternoon.

---

## 2.1 The register file, one more time, in Rust

Every emulator begins with the question of how to represent the machine's
state, and it is a question with more wrong answers than right ones. Represent
it too cleverly and every instruction pays a translation tax; represent it too
literally and you end up hand-maintaining relationships the hardware maintains
for free. The 6809's register file is a good place to watch that judgment
being exercised, because two of its nine registers are exactly the cases where
a designer is tempted to get clever.

Chapter 1 already showed you the whole CPU struct, because there's no way to
talk about the `Bus` trait without it. Read it again, now looking at the
fields you skimmed past — `a`, `b`, `x`, `y`, `u`, `s`, `pc`, `dp`, `cc` — from
[`crates/mc6809/src/lib.rs:139`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L139):

```rust
pub struct MC6809 {
    pub a: u8,
    pub b: u8,
    pub x: u16,
    pub y: u16,
    pub u: u16,
    pub s: u16,
    pub pc: u16,
    pub dp: u8,
    pub cc: u8,
    /// Total cycles executed since reset (for scheduling/debugging).
    pub cycles: u64,
    /// Running vs halted (SYNC/CWAI).
    pub state: State,
    pub nmi_armed: bool,
}
```

That's every register the 6809 datasheet documents, plain `u8`/`u16` fields,
public. No `union`, no bitfield macro, no `enum Register8 { A, B }`
indirection — when the code needs `self.a`, it writes `self.a`. Reaching for
the plainest Rust type that models the hardware fact, and letting the type
system do less work than you'd expect, is a recurring choice in this codebase.

It is worth being explicit about what that choice buys, because "just use
plain fields" sounds less like a design decision than like an absence of one.
The payoff shows up every time you debug. When a test fails and you print the
CPU, what comes out is nine numbers you can compare directly against an XRoar
or MAME register dump, in the same units, with no decoding step in between.
When you set a conditional breakpoint on `self.x == 0x0400`, the expression
means exactly what it says. An abstraction over the register file would have
to earn its keep against that, and on a chip with nine registers there is
simply not enough repetition for it to earn anything.

### D is not a register — it's a view

There is no `d: u16` field, and that's worth noticing. `D` on real 6809
silicon isn't a separate storage cell; it's `A` and `B` read and written as
one 16-bit unit, A the high byte. The struct doesn't pretend otherwise — it
stores `a` and `b` independently and computes `D` on demand ([`lib.rs:167`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L167)):

```rust
/// Accumulator `D` is the `A:B` pair (A high, B low).
pub fn d(&self) -> u16 {
    ((self.a as u16) << 8) | self.b as u16
}

pub fn set_d(&mut self, value: u16) {
    self.a = (value >> 8) as u8;
    self.b = value as u8;
}
```

The hardware fact and nothing more. `LDD #$1234` calls `set_d(0x1234)`,
leaving `a = 0x12`, `b = 0x34` — verified byte-for-byte by `ldd_immediate`
([`crates/mc6809/tests/loads.rs:116`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/loads.rs#L116)). It is also
why `EXG A,B` swaps the two halves while `TFR D,X` moves the whole 16-bit
value: A and B are always two registers wearing one costume.

Consider the alternative for a moment, because it is the design a lot of
first-time emulator authors reach for. Store `d: u16` as the real field, and
derive `A` and `B` from it with accessors. That works too — arithmetic on `D`
gets marginally simpler — but now every one of the dozens of instructions that
touch `A` or `B` individually pays a shift-and-mask, and every time you write
`B` you must be careful not to clobber the high half. The version in this
codebase inverts the cost: the two-byte registers are free and the rare 16-bit
view costs a shift and an OR. Since `A` and `B` are touched far more often
than `D` is, that is the cheaper way round — but the reason to prefer it isn't
the cycle count on a modern host, where neither version is measurable. It's
that `a` and `b` are the storage the datasheet describes, so a struct that
stores them is a struct you can check against the datasheet field by field.

A second, quieter benefit falls out of the same choice. Because `d()` takes
`&self` and returns a plain `u16`, it composes with everything: `self.add16(self.d(), m)`
reads the pair, does the arithmetic, and writes the result back through
`set_d` in one line, with no borrow gymnastics and no intermediate state that
could get out of sync. You'll see that exact line four times in §2.5.

### The CC register: eight bits, eight names

CC is the other register not stored as eight separate booleans, for the same
reason: real 6809 code reads and writes it as a byte too (`TFR CC,A`,
`PSHS CC`, `ORCC #$50`). The bit layout, from [`lib.rs:47`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L47):

```rust
/// Condition Code register bit masks. CC = `E F H I N Z V C`.
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

Read top-to-bottom against the doc comment's `E F H I N Z V C`: the
datasheet's bit numbering (bit 0 = C, up to bit 7 = E), turned into named
constants. `E` (entire-frame) and `I`/`F` (interrupt masks) don't concern you
until Chapter 4 — this week lives entirely in `N Z V C`, plus `H` for `DAA`.

Notice that the module and the doc comment are arranged in opposite
directions, and that this is not sloppiness. The constants ascend numerically,
`0x01` through `0x80`, which is the order you want when checking a mask
against a hex dump. The doc comment's `E F H I N Z V C` descends from bit 7 to
bit 0, which is the order every 6809 reference card and assembler manual
prints it in, because that is how the byte reads left to right when
you write it out in binary. Both orders are correct and both are useful; the
file gives you a way to get from either one to the other without arithmetic.

The four flags this chapter cares about deserve one sentence each, since every
formula in §2.5 and §2.6 resolves to setting or clearing one of them. `N` is
bit 7 of the result, treated as a sign bit — a pure copy, not a judgment. `Z`
is set when the result is zero, which is the single most-branched-on condition
in any instruction set. `V` is *signed* overflow: the result would be wrong if
you were interpreting the bytes as signed two's-complement numbers. `C` is
*unsigned* carry or borrow: the result would be wrong if you were interpreting
them as unsigned. `V` and `C` are the pair that trips people up, because they
answer the same question about two different interpretations of the same
bits, and the hardware computes both on every arithmetic operation without
knowing or caring which one the programmer meant.

> **Rust corner: a module of constants instead of `bitflags!`.** You might
> expect a `bitflags!`-generated type here, or an `enum` with `#[repr(u8)]`.
> The codebase uses neither — `cc` is a plain module of `pub const u8`
> values, and the CC register itself is a bare `u8`. Every flag test reads
> `self.cc & cc::ZERO != 0`; every flag set is `self.cc |= cc::ZERO` /
> `self.cc &= !cc::ZERO`. A real bitflags type buys type-safe combination and
> a `Debug` impl that prints flag names, at the cost of a dependency and one
> more layer between you and the byte the hardware actually has. For a
> register real 6809 programs manipulate as a raw byte (`ORCC #$50` is not
> "set bits symbolically," it's "OR this literal into CC"), keeping the
> emulator's representation exactly that literal is the more honest model —
> and it's what you'll read in §2.5 for every ALU flag computation.
>
> The pattern recurs, which is how you know it's a house style rather than a
> one-off. Three more modules of bare constants sit further down the same
> file: `regsel` for the register-selector nibbles that `TFR`/`EXG` use
> ([`lib.rs:59`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L59)), `postbyte` for the indexed-addressing bit fields
> ([`lib.rs:76`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L76)), and `stack_mask` for the `PSH`/`PUL` register mask
> ([`lib.rs:92`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L92)). All three describe fields inside a byte that the
> *program* constructs — an assembler emits `$1F 0x89` for `TFR A,B` and the
> CPU has to take that nibble apart — so all three get the same treatment:
> names for the bits, the byte left alone. Chapter 3 lives inside two of those
> three modules.

The register file is the state. What turns state into a machine is the loop
that advances it, and on a CPU that loop's entire personality is in one
`match`.

---

## 2.2 Dispatch: there is no opcode table — the `match` *is* the table

Anyone who has read another 8-bit emulator's source will be expecting an
array: `const OPCODES: [fn(&mut Cpu); 256] = [...]`, indexed by the fetched
byte. That's the classic table-driven design, and it's a fine choice for the
6502 (256 opcodes, one addressing mode each, done). The 6809 doesn't get that
table here. Instead, `MC6809::step` is one large `match` on the opcode byte,
and the match arms *are* the dispatch table ([`crates/mc6809/src/exec.rs:30`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L30)):

```rust
pub fn step(&mut self, bus: &mut impl Bus) -> u32 {
    if self.state != State::Running {
        // Halted by SYNC/CWAI: burn an idle cycle until the machine delivers
        // an interrupt (via nmi/irq/firq) that resumes execution.
        self.cycles += 1;
        return 1;
    }
    let opcode = self.fetch_u8(bus);
    let cycles = match opcode {
        0x12 => 2, // NOP

        // ---- Branches -----------------------------------------------------
        // Offsets are relative to the PC *after* the operand is consumed.
        // Short Bcc: 8-bit signed offset, 3 cycles (taken or not). Long LBcc:
        // 16-bit offset, 5 cycles / 6 if taken. LBRA is always, 5 cycles.
        0x20..=0x2F => {
            let offset = self.fetch_u8(bus) as i8 as i16 as u16;
            if self.branch_taken(opcode) {
                self.pc = self.pc.wrapping_add(offset);
            }
            3
        }
        0x16 => {
            // LBRA — unconditional, 16-bit offset
            let offset = self.fetch_u16(bus);
            self.pc = self.pc.wrapping_add(offset);
            5
        }
        // $10/$11 prefix pages: long conditional branches, the 16-bit ops
        // targeting Y/D/S/U, and SWI2/SWI3.
        0x10 => self.exec_page10(bus),
        0x11 => self.exec_page11(bus),

        // ---- Subroutines, jumps, register transfer, stack -----------------
        // JMP arms MUST precede the RMW range arms below (0x0E/0x6E/0x7E would
        // otherwise be swallowed by 0x00-0x0F / 0x60-0x6F / 0x70-0x7F).
        0x0E | 0x6E | 0x7E | 0x9D | 0xAD | 0xBD | 0x8D | 0x17 | 0x39 | 0x1F | 0x1E | 0x34
        | 0x36 | 0x35 | 0x37 => self.exec_control_transfer(bus, opcode),

        // ---- CC manipulation, misc inherent -------------------------------
        0x1A | 0x1C | 0x1D | 0x3A | 0x3D | 0x19 => self.exec_misc_inherent(bus, opcode),

        // ---- Interrupt / halt ---------------------------------------------
        0x3F | 0x3B | 0x3C | 0x13 => self.exec_interrupt_halt(bus, opcode),

        // LDA/LDB/STA/STB/LDD/STD — immediate / direct / extended
        0x86 | 0x96 | 0xB6 | 0xC6 | 0xD6 | 0xF6 | 0x97 | 0xB7 | 0xD7 | 0xF7 | 0xCC | 0xDC
        | 0xFC | 0xDD | 0xFD => self.exec_load_store(bus, opcode),

        // ---- 8-bit ALU (immediate / direct / extended) --------------------
        // Cycles: immediate 2, direct 4, extended 5. Carry-in for ADC/SBC is
        // the current C flag (cc::CARRY == 0x01, so masking yields 0 or 1).
        0x8B | 0x9B | 0xBB | 0xCB | 0xDB | 0xFB | 0x89 | 0x99 | 0xB9 | 0xC9 | 0xD9 | 0xF9
        | 0x80 | 0x90 | 0xB0 | 0xC0 | 0xD0 | 0xF0 | 0x82 | 0x92 | 0xB2 | 0xC2 | 0xD2 | 0xF2
        | 0x81 | 0x91 | 0xB1 | 0xC1 | 0xD1 | 0xF1 => self.exec_alu8(bus, opcode),

        // ---- Indexed addressing (base cost + postbyte extra cycles) --------
        // 8-bit load/store base 4; 16-bit LDD/STD base 5; LEA base 4.
        0x30 | 0x31 | 0x32 | 0x33 | 0xA6 | 0xE6 | 0xA7 | 0xE7 | 0xEC | 0xED | 0xAB | 0xEB
        | 0xA9 | 0xE9 | 0xA0 | 0xE0 | 0xA2 | 0xE2 | 0xA1 | 0xE1 => {
            self.exec_indexed(bus, opcode)
        }

        // ---- 8-bit logic (AND/OR/EOR/BIT) --------------------------------
        // N,Z from result; V cleared; C and H unaffected. BIT sets flags only.
        // Cycles: immediate 2, direct 4, indexed 4+, extended 5.
        0x84 | 0x94 | 0xA4 | 0xB4 | 0xC4 | 0xD4 | 0xE4 | 0xF4 | 0x8A | 0x9A | 0xAA | 0xBA
        | 0xCA | 0xDA | 0xEA | 0xFA | 0x88 | 0x98 | 0xA8 | 0xB8 | 0xC8 | 0xD8 | 0xE8 | 0xF8
        | 0x85 | 0x95 | 0xA5 | 0xB5 | 0xC5 | 0xD5 | 0xE5 | 0xF5 => {
            self.exec_logic8(bus, opcode)
        }

        // ---- 16-bit ALU / load / store (D, X, U) --------------------------
        // ADDD/SUBD affect N,Z,V,C. CMPX is sub16 discarded. LDX/LDU/STX/STU
        // set N,Z and clear V. Cycles: ADD/SUB/CMP imm 4/dir 6/idx 6+/ext 7;
        // LD imm 3/dir 5/idx 5+/ext 6; ST dir 5/idx 5+/ext 6.
        0xC3 | 0xD3 | 0xE3 | 0xF3 | 0x83 | 0x93 | 0xA3 | 0xB3 | 0x8C | 0x9C | 0xAC | 0xBC
        | 0x8E | 0x9E | 0xAE | 0xBE | 0x9F | 0xAF | 0xBF | 0xCE | 0xDE | 0xEE | 0xFE | 0xDF
        | 0xEF | 0xFF => self.exec_16bit(bus, opcode),

        // ---- 8-bit read-modify-write (NEG/COM/LSR/ROR/ASR/ASL/ROL/DEC/INC/TST/CLR)
        // Low nibble selects the op (see rmw_apply). Cycles: inherent 2,
        // direct 6, indexed 6+, extended 7. TST reads but never writes back.
        0x40..=0x4F | 0x50..=0x5F | 0x00..=0x0F | 0x60..=0x6F | 0x70..=0x7F => {
            self.exec_rmw(bus, opcode)
        }

        _ => 2, // TODO: unimplemented opcode
    };
    self.cycles += cycles as u64;
    cycles
}
```

This is the *whole* table — nothing was cut from the excerpt above. Every one
of the 6809's base-page opcodes (plus the two prefix bytes) is accounted for
in one screenful, and that fact is itself the argument for this design: you
can hold the entire dispatch surface in your head, or at least on one scroll
of one editor tab.

Before dissecting the arms, look at the frame around them, because it is the
fetch-execute loop Chapter 1 described, with nothing added. Three lines do the
whole job. `let opcode = self.fetch_u8(bus);` fetches and advances `PC`. The
`match` produces a cycle count as its value. `self.cycles += cycles as u64;`
banks the time. Everything else in the function is the table. That the loop
body is three statements and a lookup is not a simplification for the book —
it is what a CPU is, once the decode table is somewhere else.

The early return at the top is the one piece of the frame that isn't decode.
If `state` isn't `Running`, the CPU has halted itself with `SYNC` or `CWAI`
and no instruction executes at all; `step` burns a single cycle and returns.
That single cycle matters more than it looks. Time still has to pass while the
CPU waits, because the thing it is waiting *for* — an interrupt from the
GIME's timer, or from a PIA — is generated by the scheduler counting cycles.
A halted CPU that returned zero would freeze the clock and wait forever for an
event that can never arrive. Chapter 4 builds the other half of that handshake.

The branch arms are the only instructions with operands that `step` decodes
completely by itself, and they earn the privilege by being shaped unlike
anything else. `0x20..=0x2F` is the whole short-branch family in one
range — sixteen opcodes, `BRA`, `BRN`, and fourteen conditional branches —
because they all have identical structure: fetch one signed byte, ask
`branch_taken` whether the condition holds, and conditionally add. The
condition test lives in `branch_taken`, over in
[`crates/mc6809/src/branch.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/branch.rs), which is one `match` on the opcode's low
nibble mapping `0x0` through `0xF` onto the sixteen conditions; the *shape*
lives here. `0x16`, `LBRA`, is the 16-bit-offset unconditional version, and
it sits alone because its offset is a full word rather than a sign-extended
byte.

Read the comment above those arms carefully, because it states a rule that
causes real bugs when violated: *"Offsets are relative to the PC after the
operand is consumed."* By the time `self.pc.wrapping_add(offset)` runs,
`fetch_u8` has already advanced `PC` past the offset byte, so a branch offset
of zero means "continue with the next instruction," not "branch to yourself."
Every 6809 assembler computes its offsets on that assumption. Get it backwards
and short forward branches land two bytes early — which does not crash, and
does not fail obviously; it silently executes an operand byte as an opcode.

> **Rust corner: chained `as` casts for sign extension.** Look back at the
> branch arm: `let offset = self.fetch_u8(bus) as i8 as i16 as u16;`. Three
> casts in a row look like noise until you track what each one does.
> `fetch_u8` returns a `u8` — the raw byte, no notion of sign. `as i8`
> *reinterprets* those same 8 bits as signed (`0x80` stops meaning "128" and
> starts meaning "−128") without changing a single bit — this is the cast
> that actually encodes "the 6809's branch offset is signed." `as i16` then
> *sign-extends*: because the source type is signed, Rust fills the newly
> exposed high byte with copies of the sign bit, so `-1i8` (`0xFF`) becomes
> `-1i16` (`0xFFFF`), not `0x00FF`. The final `as u16` reinterprets
> those bits as unsigned again, because `self.pc` is a `u16` and
> `wrapping_add` needs matching types. Drop any cast in this chain and you
> get a different, wrong number: skip the `as i8` and a negative offset
> sign-extends as if it were positive (zero-fills instead of one-fills);
> skip the final `as u16` and it won't compile, since `u16::wrapping_add`
> doesn't accept an `i16`. This exact three-cast idiom — narrow signed,
> widen signed, reinterpret unsigned — is how this codebase sign-extends
> anywhere a byte from the instruction stream needs to become a signed
> 16-bit displacement; you'll see it again on the 5-bit indexed offset next
> week.
>
> One consequence is worth stating because it looks like a bug the first
> time you see it: a *backward* branch is implemented as an *addition* of a
> very large unsigned number. `BRA` with offset `$FE` becomes
> `pc.wrapping_add(0xFFFE)`, and because 16-bit arithmetic wraps, adding
> `0xFFFE` is subtracting 2. There is no separate backward case anywhere in
> the branch code, and there doesn't need to be — two's complement and
> wrapping addition give you subtraction for free, which is precisely why
> processors have used the representation since the 1960s.

A word about the doc comment on `step` versus the one at the top of `lib.rs`.
`lib.rs` still opens with a stale banner — *"STATUS: skeleton... only a few
opcodes are decoded"* — left over from an early milestone and never updated.
The comment that actually describes what you just read is on `step` itself
([`exec.rs:12`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L12)): *"This is the complete 6809 user-mode ISA; only a handful of
illegal opcodes remain undecoded and are treated as 2-cycle NOPs during
bring-up."* When a file-level comment and a function-level comment disagree,
trust the one attached to the code you're actually looking at. This crate has
roughly two hundred passing tests; it is not a skeleton.

Stale comments are worth a moment of attention rather than an eye-roll,
because they are a permanent feature of real codebases and this one is
instructive about *why*. The banner was true on the day it was written and
became false gradually, one merged milestone at a time, with no single commit
where anyone was obviously wrong to leave it alone. The function-level comment
survived because it sits directly above the code it describes, where a reader
changing that code cannot avoid seeing it. Comments live longer the closer
they are to what they document. That is an argument for putting the important
ones on functions rather than on files.

### Reading the match

Notice a few things about how this is organized. Some arms decode
completely inline (`0x12 => 2` for `NOP`; the branch range `0x20..=0x2F`) —
one operand shape, one job. Most arms are `|`-chains of opcode bytes routing
to one family function: the `exec_load_store` chain
(`0x86 | 0x96 | 0xB6 | 0xC6 | ...`) is `LDA` immediate/direct/extended, `LDB`
immediate/direct/extended, `STA` direct/extended, and so on. 6809 opcode
assignment is historical, not algorithmic, so most of these chains are literal
enumerations rather than computed ranges — except the RMW arm
(`0x40..=0x4F | 0x50..=0x5F | 0x00..=0x0F | 0x60..=0x6F | 0x70..=0x7F`), where
the high nibble genuinely does pick the addressing mode and the low nibble
the operation (`NEG`, `COM`, `LSR`, …) — a real structural regularity in the
6809's own opcode map that the Rust code mirrors rather than flattens. You'll
use that low nibble again in §2.6.

The word "historical" in that paragraph is doing real work, and it needs
unpacking, because it explains why a large part of this table has to be
enumerated by hand. The opcode map has patches of real regularity, and you can
see one of them in §2.5's `exec_alu8`: `ADDA` immediate is `$8B` and `ADDB`
immediate is `$CB`, `SUBA` is `$80` and `SUBB` is `$C0` — the `$80–$BF` block
is the A-accumulator operations and `$C0–$FF` repeats the same layout for B.
Those patches sit next to bytes that landed wherever there was room. A
generated table could exploit the regular parts, but it would still need a
hand-written exception list for the rest, and an exception list you must
consult in order to read the regular part is worse than no pattern at all. The
`|`-chains are that exception list, written once, in the open.

One comment is load-bearing: right above the control-transfer arm,
`// JMP arms MUST precede the RMW range arms below (0x0E/0x6E/0x7E would
otherwise be swallowed by 0x00-0x0F / 0x60-0x6F / 0x70-0x7F)`. `match` tries
arms top to bottom and stops at the first match, so `0x0E` (`JMP` direct) has
to be claimed by the control-transfer arm before the RMW range `0x00..=0x0F`
gets a chance. Overlapping ranges are legal Rust, and the first one wins
silently — miss this ordering and `JMP` compiles without complaint and
becomes `NEG`.

Sit with that failure mode, because it is a perfect specimen of the kind of
bug this course is training you to anticipate. Nothing warns you. The compiler
is happy: overlapping patterns are not an error, and Rust's unreachable-pattern
lint fires on a *fully* shadowed arm, not on three bytes shadowed out of a
sixteen-byte range. The tests are happy too, unless one of them happens to
exercise `JMP` in direct mode specifically. What you get instead is a machine
that boots, runs thousands of instructions correctly, and then — the first
time the ROM does a `JMP` through the direct page — negates a byte of memory
and falls through to the next instruction. The symptom appears somewhere else
entirely, minutes later, as a corrupted variable. This is exactly the "a wrong
flag in an obscure addressing mode shows up four thousand instructions later"
problem Chapter 1 warned about, and the defense, here, was a comment in
capital letters written by someone who had thought about it once so that
nobody has to think about it again.

### The two prefix pages

Two arms in the table delegate to something other than a family function:
`0x10` and `0x11`. These are not instructions at all. They are *prefix bytes*,
Motorola's answer to running out of room in a 256-entry opcode map, and they
work by declaring "the real opcode is the next byte, interpreted from a
different table." The 6809 needs two of them because the base page has no
space for the 16-bit operations targeting `Y`, `S`, and `U`, nor for the long
conditional branches.

The `$10` page's handler opens like this ([`exec.rs:125`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L125)):

```rust
/// `$10`-prefixed page: long conditional branches, CMPD/CMPY/LDY/STY/LDS/STS,
/// SWI2. Prefixed ops cost one more cycle than their base-page equivalents.
fn exec_page10(&mut self, bus: &mut impl Bus) -> u32 {
    let op2 = self.fetch_u8(bus);
    match op2 {
        0x21..=0x2F => {
            let offset = self.fetch_u16(bus);
            if self.branch_taken(op2) {
                self.pc = self.pc.wrapping_add(offset);
                6
            } else {
                5
            }
        }
```

The structure is the same as `step`'s: fetch a byte, match on it, return
cycles. The one new fact is in the doc comment — *"Prefixed ops cost one more
cycle than their base-page equivalents"* — which is the price of the extra
fetch, and it is why `CMPD` immediate costs 5 where the base-page `CMPX`
immediate costs 4. The other new fact is in the arm itself: a long conditional
branch costs 6 cycles when taken and 5 when not. Contrast that with the short
branches back in `step`, which return a flat `3` either way. This asymmetry is
real, it is on the instruction card, and it is the only place this week where
a cycle count depends on a runtime condition rather than on the opcode alone.

`exec_page11` is the same shape and much shorter: `CMPU`, `CMPS`, and `SWI3`.
Two prefix pages, one code shape, no new machinery — which is exactly what you
want from a feature that exists purely because a byte only holds 256 values.

### Family functions: one `step`, eleven helpers

Below `step` sit eleven `exec_*` functions, five in `exec.rs` and six in
`exec/exec_data.rs`, and the split between them is worth understanding as a
piece of navigation advice rather than as trivia. Each function owns a
contiguous slice of the opcode map and nothing else, so "where does opcode
`$B6` live?" always has exactly one answer, and finding it is a two-step
lookup: which arm of `step` claims the byte, then which arm of that family
function.

`exec_load_store` ([`crates/mc6809/src/exec/exec_data.rs:11`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec/exec_data.rs#L11)) is the smallest
of the six and the best one to read first, because it contains every
load and store the base page has and nothing else:

```rust
/// LDA / LDB / STA / STB / LDD / STD — immediate / direct / extended.
pub(super) fn exec_load_store(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
    match opcode {
        // LDA — immediate / direct / extended
        0x86 => { let ea = self.fetch_u8(bus); self.a = ea; self.set_nz8(ea); 2 }
        0x96 => { let v = self.read_direct8(bus); self.a = v; self.set_nz8(v); 4 }
        0xB6 => { let v = self.read_extended8(bus); self.a = v; self.set_nz8(v); 5 }

        // LDB — immediate / direct / extended
        0xC6 => { let v = self.fetch_u8(bus); self.b = v; self.set_nz8(v); 2 }
        0xD6 => { let v = self.read_direct8(bus); self.b = v; self.set_nz8(v); 4 }
        0xF6 => { let v = self.read_extended8(bus); self.b = v; self.set_nz8(v); 5 }

        // STA — direct / extended
        0x97 => { let ea = self.ea_direct(bus); bus.write(ea, self.a); self.set_nz8(self.a); 4 }
        0xB7 => { let ea = self.ea_extended(bus); bus.write(ea, self.a); self.set_nz8(self.a); 5 }

        // STB — direct / extended
        0xD7 => { let ea = self.ea_direct(bus); bus.write(ea, self.b); self.set_nz8(self.b); 4 }
        0xF7 => { let ea = self.ea_extended(bus); bus.write(ea, self.b); self.set_nz8(self.b); 5 }

        // LDD — immediate / direct / extended
        0xCC => { let v = self.fetch_u16(bus); self.set_d(v); self.set_nz16(v); 3 }
        0xDC => { let ea = self.ea_direct(bus); let v = bus.read_u16(ea); self.set_d(v); self.set_nz16(v); 5 }
        0xFC => { let ea = self.ea_extended(bus); let v = bus.read_u16(ea); self.set_d(v); self.set_nz16(v); 6 }

        // STD — direct / extended
        0xDD => { let ea = self.ea_direct(bus); let v = self.d(); bus.write_u16(ea, v); self.set_nz16(v); 5 }
        0xFD => { let ea = self.ea_extended(bus); let v = self.d(); bus.write_u16(ea, v); self.set_nz16(v); 6 }

        _ => unreachable!("exec_load_store called for opcode {opcode:#04X}"),
    }
}
```

Fifteen instructions, fifteen one-line arms, and the whole thing fits on a
screen. Read it as a grid: six row-groups (`LDA`, `LDB`, `STA`, `STB`, `LDD`,
`STD`) crossed with the addressing modes each supports. The loads have three
modes; the stores have two, because "store to an immediate operand" is
meaningless — there is no address to write to when the operand *is* the
instruction stream. That absence is not an omission in this code; it is an
absence in the 6809's opcode map, faithfully reproduced.

Three details in those arms repay a second look. The first is that every
single arm ends in a bare integer, and that integer is the instruction's cycle
count — the arm's value, returned up through `step`. The second is the flag
call at the end of each arm: `set_nz8` for the byte operations, `set_nz16` for
the `D` ones. Neither arm computes a flag itself; §2.5 explains what those two
functions do and why loads clear `V`. The third is that the stores set flags
from the value they *wrote*, not from anything they read — `self.set_nz8(self.a)`
after `bus.write(ea, self.a)` — which looks redundant until you remember that
`STA` is specified to leave `N` and `Z` describing `A`. It is a real datasheet
row, not a copy-paste artifact.

One small inconsistency in the excerpt deserves a mention, so that you don't
spend time looking for meaning in it. In the `0x86` arm the fetched byte
is bound to `ea`, while the two arms below it bind to `v`. `ea` normally means
"effective address" everywhere else in this codebase; here it holds an
immediate *value*. Nothing behaves differently — it is a name that drifted —
but noticing it is good practice, because the same habit applied to
`exec_rmw`'s `op != 0x0D` in §2.6 will tell you something that does matter.

That `unreachable!()` catch-all asserts that `step`'s dispatch and
`exec_load_store`'s opcode set stay in lockstep: if someone extends the
`step` match's load-store chain but forgets the matching arm here, the
program panics loudly on first use — instead of silently falling through and
doing nothing. Compare `step`'s own catch-all, `_ => 2`, which is deliberately
*not* a panic: undecoded opcodes are 2-cycle no-ops "during bring-up." Two
catch-alls, two different meanings.

The difference between those two catch-alls is a design position worth
adopting. `step` sits at the boundary between the emulator and arbitrary
6809 code, and arbitrary code contains garbage: a program that jumps into
data will fetch bytes that are not instructions, and on real silicon that
produces *something* rather than a halt. Crashing the emulator there would be
modeling the hardware badly. `exec_load_store` sits at a boundary between two
pieces of this codebase, both under the author's control, and a mismatch
across it is not a hardware condition — it is a bug that a human introduced
minutes ago and should hear about immediately. Panic where the contract is
yours to keep; degrade gracefully where the input is someone else's to
provide.

> **Rust corner: a block is an expression, and that's the cycle count.**
> Every arm in `exec_load_store` has the form
> `0x96 => { statement; statement; statement; 4 }`. That trailing `4` has no
> semicolon, which in Rust makes it the *value* of the block, and therefore
> the value of the match arm, and therefore — since the `match` is the last
> expression in the function — the value returned by `exec_load_store`. No
> `return` keyword appears anywhere in the file.
>
> This is not merely terse. It means the cycle count cannot be forgotten:
> a block whose last statement ends in a semicolon evaluates to `()`, and a
> function declared `-> u32` that produces `()` does not compile. Leave the
> `4` off any arm in this file and `cargo build` fails, pointing at the arm.
> The timing model from §2.7 is enforced by the type checker, which is a
> considerably better guarantee than a code-review convention.

### The trade-off you're being shown

A function-pointer table (`[fn(&mut MC6809, &mut dyn Bus); 256]`, 6502-style)
buys O(1) dispatch and a compact, data-driven table you could in principle
generate. What it loses: every entry is a bare function pointer carrying no
information about *why* opcodes are grouped, and adding an instruction means
writing a function plus wiring one array slot far from the code that explains
it. The `match`-as-table design pays a tiny, LLVM-optimized dispatch cost to
buy something this course cares about more: open one file, search for an
opcode byte, and land in code whose surrounding context *is* its
documentation — family grouping, addressing-mode comments, the cycle-count
table in the comment above each arm. Transparency over compactness, exactly
as [DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) frames it — the right trade for a CPU you're going to read,
trace, and debug for fifteen more weeks.

The phrase "a tiny dispatch cost" invites suspicion, so it deserves a fair
accounting. A `match` on a `u8` with this many arms does not compile into a
chain of two hundred comparisons; LLVM turns dense integer
matches into jump tables and sparse ones into binary searches, and this one is
dense enough to get the good treatment for most of its range. Even if it
didn't, Chapter 1 already did the arithmetic: the host retires roughly ten
thousand instructions in the time the emulated CoCo retires one. The dispatch
cost is not the reason to choose either design. Legibility is, and it is the
reason this codebase chose the one it did.

The table has one more consumer worth knowing about, mentioned in the module
comment at the top of `exec.rs`: *"The top-level match in `step` is the
authoritative opcode map (mirrored byte-for-byte by [`crate::disasm`])."*
There is a disassembler in this crate, and its structure deliberately
parallels the executor's, opcode byte for opcode byte, so the two can be read
side by side and diffed by eye. Chapter 3 puts that mirror to work. For now, note
only that "the dispatch table is readable" was not an aesthetic preference —
another component was built to match its shape.

With the map in hand, the next question is the one every arm in it already
assumes an answer to: given an opcode, where does its operand come from?

---

## 2.3 Addressing modes: immediate, direct, extended

The 6809's reputation rests largely on its addressing modes. A 6502 programmer
gets zero page, absolute, and a handful of indexed forms; a 6809 programmer
gets a relocatable direct page, program-counter-relative addressing that makes
position-independent code natural rather than heroic, and an indexed mode with
about a dozen sub-modes hiding behind a single postbyte. All of that richness
has to be decoded before any instruction can do its work, which makes
addressing the layer everything else stands on.

Three of those modes are simple enough to cover this week —
the postbyte-driven indexed mode is next week's hardest 200 lines. All three
live in [`crates/mc6809/src/addressing.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs), and all three route through two
tiny primitives at the top of the file ([`addressing.rs:7`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L7)):

```rust
pub(crate) fn fetch_u8(&mut self, bus: &mut impl Bus) -> u8 {
    let v = bus.read(self.pc);
    self.pc = self.pc.wrapping_add(1);
    v
}

pub(crate) fn fetch_u16(&mut self, bus: &mut impl Bus) -> u16 {
    let hi = self.fetch_u8(bus) as u16;
    let lo = self.fetch_u8(bus) as u16;
    (hi << 8) | lo
}
```

`fetch_u8` is "read the byte at PC, then advance PC" — the one operation
every instruction byte and operand byte in the machine goes through.
`fetch_u16` is two of those, high byte first, big-endian as it must be. You
met that rule on `Bus::read_u16` last week; it is hand-written again here
because these bytes come from the *instruction stream* at `PC` rather than
from an arbitrary address.

That duplication deserves a defense, since a reader who has just internalized
"write the endianness rule down once" from Chapter 1 will bristle at seeing it
written down twice. The two functions do genuinely different things.
`Bus::read_u16(addr)` reads a word from an address you already computed and
leaves the CPU alone. `MC6809::fetch_u16` reads a word from wherever `PC`
happens to point *and advances `PC` by two as a side effect*. Expressing the
second in terms of the first would mean reading `self.pc`, calling
`bus.read_u16`, then adding two — three operations to avoid writing one shift
and one OR, and a version whose relationship to `PC` is less obvious, not
more. The endianness rule appearing twice is a real (small) risk; the
compensating control is that `tests/loads.rs` pins the byte order of both
paths independently, which §2.8 comes back to.

**Immediate** addressing has no helper function at all:
`fetch_u8`/`fetch_u16` are called directly at the use site, because
"immediate" *is* "the operand lives in the instruction stream." You saw it in
`exec_load_store`: `0x86 => { let ea = self.fetch_u8(bus); ... }` for
`LDA #$42`. There is no address to compute, no bus transaction beyond the
instruction fetch itself, and correspondingly no cycles beyond the minimum:
immediate is the 2-cycle column in every row of §2.5's grid.

**Direct** addressing ([`addressing.rs:20`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L20)):

```rust
/// Direct-mode effective address: `DP:operand_byte`.
pub(crate) fn ea_direct(&mut self, bus: &mut impl Bus) -> u16 {
    let lo = self.fetch_u8(bus) as u16;
    ((self.dp as u16) << 8) | lo
}
```

One operand byte, combined with the **Direct Page register** to form a
16-bit address. This is the 6809's answer to the 6502's zero page, made
relocatable: instead of always meaning `$00xx`, direct mode means
`DP:xx` — wherever `DP` currently points. A one-byte operand is cheaper to
fetch and one byte shorter than extended, so hot variables and
frequently touched I/O pages are placed where `DP` reaches. The emulator reads
`self.dp`, shifts it into the high byte, and ORs in the fetched low byte —
nothing clever. Whatever `DP` holds (BASIC leaves it at `$00`; reset sets it
there per [`lib.rs:178`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L178), `self.dp = 0`) is exactly what direct mode uses,
bug-for-bug identical to hardware.

The word *relocatable* is the whole story, and it earns a paragraph, because
`DP` is the register most often misunderstood by people arriving from the
6502. On a 6502, zero page is a fixed, scarce, contested
resource: 256 bytes at `$0000`, shared by the ROM, the operating system, and
your program, with a permanent low-grade negotiation over who owns which
bytes. The 6809 keeps the cheap one-byte operand but lets the program choose
which page it points at. A subroutine that needs fast access to a table can
point `DP` at that table's page, run, and put `DP` back. A device driver can
point `DP` at `$FF` and reach the entire I/O page in two-byte instructions.
The cost is that `DP` becomes part of your calling convention: every routine
either knows what `DP` holds or has to set it, and forgetting is how direct
mode turns into a wild pointer. That is exactly the same cost the emulator
pays — `ea_direct` reads whatever `self.dp` currently holds and asks no
questions.

**Extended** addressing ([`addressing.rs:26`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L26)) is the least surprising mode
in the instruction set:

```rust
/// Extended-mode effective address: a 16-bit operand.
pub(crate) fn ea_extended(&mut self, bus: &mut impl Bus) -> u16 {
    self.fetch_u16(bus)
}
```

A full 16-bit address, fetched straight from the instruction stream. One line,
delegating entirely to `fetch_u16`, and the function exists at all only so
that call sites can say what they mean: `ea_extended` at a use site reads as
"extended addressing," where a bare `fetch_u16` would read as "operand."
Naming the mode is the whole contribution.

`ea_direct` and `ea_extended` both return an address — they do not read memory
*at* that address themselves. Reading is a separate step, bundled by four
small helpers right below them ([`addressing.rs:173`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L173)):

```rust
pub(crate) fn read_direct8(&mut self, bus: &mut impl Bus) -> u8 {
    let ea = self.ea_direct(bus);
    bus.read(ea)
}

pub(crate) fn read_extended8(&mut self, bus: &mut impl Bus) -> u8 {
    let ea = self.ea_extended(bus);
    bus.read(ea)
}

pub(crate) fn read_direct16(&mut self, bus: &mut impl Bus) -> u16 {
    let ea = self.ea_direct(bus);
    bus.read_u16(ea)
}

pub(crate) fn read_extended16(&mut self, bus: &mut impl Bus) -> u16 {
    let ea = self.ea_extended(bus);
    bus.read_u16(ea)
}
```

Four functions, two lines each, and between them they cover every "compute an
address and immediately read it" case in the base page. The split —
compute-the-address versus read-the-address — exists because not
every instruction wants both: `LDA` wants "compute EA, then read" (exactly
what `read_direct8`/`read_extended8` give in one call); `STA` wants "compute
EA, then write," so it calls `ea_direct`/`ea_extended` directly and does its
own `bus.write`; `JMP` wants *only* the address, `ea_direct(bus)` alone,
straight into `self.pc`. Three instructions, three combinations of the same
two primitives, no wasted work, no wrapper trying to cover every case.

Go back to `exec_load_store` and check that claim against the code. `0x96`
(`LDA` direct) calls `read_direct8`. `0x97` (`STA` direct) calls `ea_direct`
and then `bus.write`. The `JMP` arms live in `exec_control_transfer` and call
`ea_direct`/`ea_extended` with nothing after them but an assignment to
`self.pc`. Three shapes, visible in the source, no shape unused. This is what
"decoupled" buys: an addressing mode is a function that produces an address,
and what happens to that address is entirely the caller's business.

> **Rust corner: `pub(crate)` visibility.** Every function in this section is
> `pub(crate)`, not `pub`: visible anywhere inside the `mc6809` crate (so
> `exec.rs` and `exec/exec_data.rs` call them freely) but invisible outside
> it — a user of this crate can call `MC6809::step`, but can't reach in and
> call `ea_direct` directly. It's Rust's version of "internal linkage,"
> marking these as decoder plumbing, not public contract. If you ever hit a
> "function is private" error while poking at this codebase from a test
> file, that's usually a sign you're testing at the wrong level — test
> through `step()`, as every file in `tests/` does.
>
> Rust has a whole ladder of these, and this crate uses three rungs of it
> deliberately. `pub` is the crate's public API: `MC6809`, `Bus`, `step`,
> `cc`. `pub(crate)` is everything the CPU's own modules share:
> `fetch_u8`, `add8`, `rmw_apply`. `pub(super)` — which is what the
> `exec_*` family functions in `exec/exec_data.rs` are declared with — is
> narrower still, meaning "visible to the parent module only," so those
> functions can be called by `exec.rs` and by nothing else in the crate.
> Each rung is a sentence about who is allowed to depend on what, checked by
> the compiler rather than by convention.

You now have the three pieces every instruction needs: a dispatch table that
finds the code, addressing helpers that find the operand, and (coming in §2.5)
flag primitives that describe the result. The fastest way to see them fit
together is to walk one instruction all the way through.

---

## 2.4 A worked example: `LDA $0400`, start to finish

Reading code top-down tells you how a program is organized. Tracing one input
through it tells you how it *works*, and the two are different kinds of
knowledge. This section spends three instructions on the second kind: one
load, one read-modify-write, one direct-mode arithmetic operation. Trace them
with a pencil rather than a debugger. The goal is to reach the point where
predicting the emulator's output is faster than running it, because from week
4 onward that skill is the difference between finding a bug in ten minutes and
finding it in a day.

`LDA $0400` pulls a byte out of the text screen to see what BASIC left
there — one of the most-typed instructions on the platform. `$0400` is the
top-left character cell of the CoCo's text screen, the address Chapter 1 said
would become as familiar as `$FF90`. Here is *everything* that happens, in
order, when the emulator executes the three bytes `$B6 $04 $00`.

**Setup.** `PC` points at `$B6`. `self.cc` holds whatever the previous
instruction left there.

1. **`step()` fetches the opcode.** `let opcode = self.fetch_u8(bus);` reads
   the byte at `PC` (`$B6`) and advances `PC` by one; `PC` now points at the
   first address byte (`$04`).
2. **Dispatch.** `0xB6` is in the `exec_load_store` `|`-chain, so `step`
   calls `self.exec_load_store(bus, 0xB6)`, which matches its own `0xB6` arm:
   `let v = self.read_extended8(bus); self.a = v; self.set_nz8(v); 5`.
3. **`read_extended8` computes the address.** It calls `ea_extended`, which
   calls `fetch_u16`: two more `fetch_u8` calls read `$04` then `$00`,
   advancing `PC` past all three opcode bytes, combining big-endian:
   `(0x04 << 8) | 0x00 = 0x0400`.
4. **`read_extended8` reads the byte there.** `bus.read(0x0400)` — on the
   real machine a screen-RAM byte; in a unit test, whatever `FlatBus` holds
   there. Say it's `$7E` (on the text screen that's an inverse-video `>` —
   bit 6 is the inverse bit; you'll decode screen bytes properly in Chapter 7).
5. **The value lands in `A`**, and **flags are set from it**: `self.a = v;
   self.set_nz8(v);` — `N` from bit 7 of `0x7E` (clear, positive byte), `Z`
   from whether it's zero (it isn't), `V` unconditionally cleared, `C`/`H`
   left exactly as they were.
6. **Cycles.** The arm's tail value, `5`, flows back up as `exec_load_store`'s
   return, becomes `step`'s `cycles`, is added to `self.cycles`, and returned.

Total: three bytes consumed, `PC` advanced by three, `A` loaded, `N`/`Z`/`V`
set from the loaded byte, `C`/`H` untouched, 5 cycles charged — matching the
6809 instruction card exactly (`LDA` extended: 5 cycles, 3 bytes). Notice
what *didn't* need special-casing: `read_extended8` doesn't know it's being
called for a *load* rather than a compare or anything else — the same
function is reused verbatim by `CMPA`, `ADDA`, `ANDA` extended, and every
other extended-mode 8-bit read in the ISA. Addressing mode and operation are
fully decoupled, so `addressing.rs` never needs to know what an ALU
operation is.

There is one thing in that trace worth flagging as a habit rather than a fact:
`PC` is advanced by the *fetches themselves*, three separate times, never by
an "instruction length" table. No code anywhere in this crate knows that `LDA`
extended is three bytes long. The length is an emergent consequence of how
many times the arm calls a `fetch_*` function, which means it cannot disagree
with the decode — a whole category of table-maintenance bug that simply
doesn't exist in this design. Chapter 3's disassembler, which genuinely does need
instruction lengths, has to derive them separately, and that difference is
what makes the two implementations independent enough to be worth
cross-checking.

### A second trace: `ASL $0400`, read-modify-write end to end

`LDA` only ever *reads*. To see the other half of the addressing-mode story —
a write coming back out — trace `ASL $0400`, opcode `$78`, with `$0400`
holding `$41` (ASCII `'A'`, since screen memory holds all kinds of bytes, not
just glyphs already tagged with the inverse bit). This exercises §2.3's
extended addressing, §2.6's nibble-keyed RMW dispatch, and the `V`
computation from §2.6 all in one instruction, with an actual write-back at
the end where `LDA` had none.

1. **Fetch and dispatch.** `step()` fetches `$78`; that byte falls in the RMW
   range `0x70..=0x7F`, so `step` calls `self.exec_rmw(bus, 0x78)`, which
   matches its own extended arm.
2. **Split the opcode.** `let op = opcode & 0x0F;` gives `op = 0x8` — the
   `ASL`/`LSL` row in `rmw_apply`'s doc comment. The addressing mode (extended)
   was already fixed by which `step` arm got here, not by this byte.
3. **Compute the address and read.** `ea_extended(bus)` fetches `$04`, `$00`
   → `ea = 0x0400`. `let m = bus.read(ea);` reads `$41`.
4. **Apply the transform.** `let r = self.rmw_apply(0x8, 0x41);` dispatches to
   `asl8(0x41)`. Work the flags by hand from §2.6's rules: `m = 0100_0001`.
   `r = m << 1 = 1000_0010 = 0x82`. Carry is the old bit 7: `m & 0x80 = 0`, so
   `C` clears. Overflow is `(m ^ (m << 1)) & 0x80`: `0x41 ^ 0x82 = 0xC3`, bit 7
   set, so `V` sets — bit 7 and bit 6 of the *original* byte disagreed
   (`0`, `1`), so the shift flipped the sign, exactly the overflow case from
   §2.6. `set_nz8_only(0x82)`: bit 7 of the result is set, so `N` sets; the
   result isn't zero, so `Z` clears. `H` is untouched — `asl8`, like every
   RMW op, never touches it; only `add8` does.
5. **Write back.** `op` is `0x8`, not the `TST` nibble `0x0D`, so
   `bus.write(ea, r)` fires: `$0400` now holds `$82`.
6. **Cycles.** The extended arm of `exec_rmw` returns the literal `7`
   regardless of which RMW operation ran — the fixed cost from the comment
   above the match arm in §2.2 (`extended 7`).

Total: `$0400` goes from `$41` to `$82`, flags land at
`(H, N, Z, V, C) = (unchanged, true, false, true, false)`, 7 cycles charged.
This was checked against the real emulator while writing this chapter —
`Sys::code(0x0000, &[0x78, 0x04, 0x00])` with `$0400` pre-loaded to `$41`
produces exactly `mem(0x0400) == 0x82`, `cycles == 7`,
`flags == (false, true, false, true, false)` (with `H` starting clear, as it
does on a fresh `MC6809`). Notice the shape: steps 1–3 are addressing-mode
plumbing you've now seen twice; step 4 is a pure function of the byte that
was there; step 5 is the one line that makes RMW a *write* instead of a
*read*. Everything else in the RMW family — `NEG`, `COM`, `LSR`, `ROR`,
`ASR`, `ROL`, `DEC`, `INC`, `TST`, `CLR` — is this same five-step shape with
a different `op_nibble` and, for `TST` alone, step 5 skipped.

Consider what this instruction would do on the real machine, since `$0400` is
screen memory. `ASL $0400` reads the character cell, shifts it, and writes it
back; the next time the video hardware scans that address, the top-left
character has changed. A loop of `ASL` over screen memory is a visible effect,
not an abstract one. That immediacy is why the platform's culture ran on
`POKE` — the distance between a byte and a character cell was a single
store — and Chapter 7 is where the emulator has to honor it.

### A third trace: `ADDA <$40`, watching `DP` do its job

Both traces so far used extended addressing, where the address is just the
two bytes after the opcode — `DP` never enters the picture. To see §2.3's
"why `DP` exists" claim actually happen, trace `ADDA <$40` (opcode `$9B`,
the direct-mode row of `exec_alu8`) with `DP = $05` already loaded (say, by
an earlier `LDA #$05` / `TFR A,DP` sequence — TFR/EXG are next week, but
imagine it done), `A = $0F` going in, and `$0540` holding `$01`.

1. **Fetch and dispatch.** `step()` fetches `$9B`; it's in `exec_alu8`'s
   `|`-chain, so `step` calls `self.exec_alu8(bus, 0x9B)`, matching:
   `let m = self.read_direct8(bus); self.a = self.add8(self.a, m, 0); 4`.
2. **`read_direct8` computes the address — this is the step extended mode
   never has.** It calls `ea_direct(bus)`: `let lo = self.fetch_u8(bus) as
   u16;` fetches the single operand byte `$40` and advances `PC` past it
   (two bytes total consumed, not three — direct mode is one byte shorter
   than extended, exactly §2.3's cost argument). Then
   `((self.dp as u16) << 8) | lo` does the concatenation:
   `self.dp = 0x05` becomes `0x0500` after the shift, ORed with `lo = 0x40`
   gives `ea = 0x0540`. Notice this is *not* addition — `DP` occupies the
   high byte and the operand occupies the low byte, with no carry possible
   between them. `DP:offset` is a literal byte concatenation. It comes out
   to the same value `DP * 256 + offset` would, but concatenation is the way
   the datasheet phrases it, and the way the hardware works.
3. **Read.** `bus.read(0x0540)` returns `$01`.
4. **Apply `add8`.** `self.a = self.add8(0x0F, 0x01, 0)`. By hand:
   `sum = 0x10`, `r = 0x10`. Half-carry: `(0x0F & 0x0F) + (0x01 & 0x0F) =
   0x10`, which is `> 0x0F` — `H` sets, the classic low-nibble-rolls-over
   case from a `$0F`-ending byte. `C`: `sum = 0x10`, not `> 0xFF` — clear.
   `V`: `(0x0F ^ 0x10) & (0x01 ^ 0x10) & 0x80 = 0x1F & 0x11 & 0x80 = 0` —
   clear. `N`: `r & 0x80 = 0` — clear. `Z`: `r != 0` — clear.
5. **Cycles.** `4`, the direct-mode row's literal — one less than extended's
   `5`, because there is one fewer address byte to fetch, with the `DP`
   concatenation doing that work instead.

Total: `A` goes from `$0F` to `$10`, flags land at
`(H, N, Z, V, C) = (true, false, false, false, false)`, 4 cycles charged,
`PC` advanced by exactly two bytes (opcode plus the one operand byte). This
trace was checked the same way as the second: `Sys::code(0x0000, &[0x9B,
0x40])` with `s.cpu.dp = 0x05`, `s.cpu.a = 0x0F`, `$0540` pre-loaded to
`$01` produces exactly `a == 0x10`, `cycles == 4`,
`flags == (true, false, false, false, false)`. Change `DP` to `$06` with
nothing else touched and the exact same instruction reads `$0640` instead —
the byte at the old address is never touched. That's the whole point
of a *direct page* register: the one-byte operand is cheap precisely because
it's relative to something the program controls, not because the hardware
is doing anything clever with it.

Three traces, three shapes: read-only, read-modify-write, and
read-plus-arithmetic. Between them they touch every kind of step an
instruction in this chapter can take. What they leave unexplained is the part
that did the most work in the third one — `add8`, and the flag rules hiding
inside it.

---

## 2.5 Flag computation as shared primitives

Flags are where CPU emulators go wrong, and they go wrong quietly. A wrong
result in `A` shows up on the next instruction; a wrong `V` bit shows up only
when some branch, possibly thousands of instructions later, takes the path it
shouldn't have. The datasheet does not help as much as you'd hope, because it
presents flag behavior as a table with one row per instruction — five columns
of `↕`, `0`, and `•` for every mnemonic in the set — which invites you to
implement it the same way, one flag rule per opcode handler, two hundred
times.

Here's a fact about 6809 flags that's easy to miss from those per-instruction
tables but obvious once you read the code: `ADD`, `ADC`,
`SUB`, `SBC`, and `CMP` don't each have their own flag logic. They share two
functions, `add8` and `sub8` ([`crates/mc6809/src/alu.rs:202`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs#L202)):

```rust
/// 8-bit add with carry-in: `a + m + carry_in`. Sets H, N, Z, V, C per the
/// 6809 datasheet. Used by ADD (carry_in=0) and ADC (carry_in=C).
///
/// - C: carry out of bit 7.
/// - H: carry out of bit 3 (used by DAA).
/// - V: signed overflow — operands share a sign that differs from the result.
pub(crate) fn add8(&mut self, a: u8, m: u8, carry_in: u8) -> u8 {
    let sum = a as u16 + m as u16 + carry_in as u16;
    let r = sum as u8;
    let half = (a & 0x0F) + (m & 0x0F) + carry_in;
    self.cc &= !(cc::HALF_CARRY | cc::NEGATIVE | cc::ZERO | cc::OVERFLOW | cc::CARRY);
    if half > 0x0F {
        self.cc |= cc::HALF_CARRY;
    }
    if sum > 0xFF {
        self.cc |= cc::CARRY;
    }
    if (a ^ r) & (m ^ r) & 0x80 != 0 {
        self.cc |= cc::OVERFLOW;
    }
    if r & 0x80 != 0 {
        self.cc |= cc::NEGATIVE;
    }
    if r == 0 {
        self.cc |= cc::ZERO;
    }
    r
}
```

Read the first three lines as a unit, because they are the whole trick. The
addition is performed in `u16`, not `u8`, so the ninth bit survives: `sum >
0xFF` is a direct test for carry-out that needs no bit-twiddling. `let r = sum
as u8;` then narrows to the byte the register will actually hold, discarding
that ninth bit *after* it has been consulted. The third line does the same
thing one nibble down: adding only the low nibbles of both operands, in a type
wide enough to hold five bits of result, makes `half > 0x0F` a direct test for
carry out of bit 3. Both flags come from computing in a wider type than the
answer needs and then looking at what spilled over. That idiom recurs
throughout this file, and it is the single most useful thing to take away from
`alu.rs`.

The fourth line — `self.cc &= !(...)` — is the one that is easy to omit and
brutal to debug. It clears all five flags this function is responsible for
*before* any of them is set, so that each `if` below is a pure "set if true"
rather than a "set if true, and hope the previous instruction left it clear."
Without it, `add8` could only ever turn flags on, and a `C` set by some
instruction three steps back would survive an addition that didn't carry. Note
also what the mask does *not* include: `IRQ_MASK`, `FIRQ_MASK`, and `ENTIRE`
are untouched, because arithmetic has no business changing the interrupt masks
that Chapter 4 depends on.

`ADDA` calls `self.add8(self.a, m, 0)`. `ADCA` calls `self.add8(self.a, m, c)`
where `c` is the *current* carry flag, fetched right before the call:
`let c = self.cc & cc::CARRY;`. That single `carry_in` parameter is the
entire difference between `ADD` and `ADC` — one function, one flag formula,
two callers ([`exec/exec_data.rs:49-64`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec/exec_data.rs#L49-L64)). The trick generalizes: `carry_in`
doubles as borrow-in for `sub8`, so `SUBA` is `self.sub8(self.a, m, 0)` and
`SBCA` is `self.sub8(self.a, m, c)`. `CMPA` computes the same subtraction as
`SUBA` and throws the result away: `self.sub8(self.a, m, 0);`, return value
unused. That raises the obvious question of whether `CMPA #$0D` secretly
modifies `A` — it doesn't; the arm never assigns `sub8`'s return value
anywhere:

```rust
// CMPA (result discarded, flags only)
0x81 => { let m = self.fetch_u8(bus);        self.sub8(self.a, m, 0); 2 }
```

`A` is untouched; only the flags `sub8` sets as a side effect survive.

There is one detail in `let c = self.cc & cc::CARRY;` that is easy to skim
past and that the dispatch comment in §2.2 calls out explicitly: *"cc::CARRY
== 0x01, so masking yields 0 or 1."* The carry flag is deliberately the lowest
bit of `CC`, which means masking it out produces exactly the integer 0 or the
integer 1 — already the right numeric value to add. No shift, no `if`, no
`as u8` from a `bool`. Had the datasheet put `C` anywhere else in the byte,
every `ADC` and `SBC` arm would need a shift, and someone would eventually
forget one. Bit assignments in a status register are not arbitrary, and this
is one of the places you can see the hardware designers thinking about the
same problem.

Since `sub8` is invoked by five instruction families and by `NEG` in §2.6, it
is worth seeing rather than inferring ([`alu.rs:232`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs#L232)):

```rust
/// 8-bit subtract with borrow-in: `a - m - borrow_in`. Sets N, Z, V, C per the
/// 6809 datasheet; H is left undefined (unaffected here). Used by SUB
/// (borrow_in=0), SBC (borrow_in=C), and CMP (result discarded).
///
/// - C: set on borrow (`a < m + borrow_in`).
/// - V: signed overflow — minuend and subtrahend differ in sign and the result
///   sign differs from the minuend.
pub(crate) fn sub8(&mut self, a: u8, m: u8, borrow_in: u8) -> u8 {
    let diff = (a as u16)
        .wrapping_sub(m as u16)
        .wrapping_sub(borrow_in as u16);
    let r = diff as u8;
    self.cc &= !(cc::NEGATIVE | cc::ZERO | cc::OVERFLOW | cc::CARRY);
    if diff & 0x100 != 0 {
        self.cc |= cc::CARRY;
    }
    if (a ^ m) & (a ^ r) & 0x80 != 0 {
        self.cc |= cc::OVERFLOW;
    }
    if r & 0x80 != 0 {
        self.cc |= cc::NEGATIVE;
    }
    if r == 0 {
        self.cc |= cc::ZERO;
    }
    r
}
```

Structurally it is `add8`'s twin, and the differences are all instructive.
The subtraction is done in `u16` with `wrapping_sub`, so when the result would
go negative it wraps to something with bit 8 set — and `diff & 0x100 != 0`
reads that borrow directly, the mirror image of `add8`'s `sum > 0xFF`. The
clearing mask is one flag shorter, omitting `HALF_CARRY`, which is the code
being honest about the doc comment's *"H is left undefined (unaffected
here)"*: `sub8` neither sets `H` nor clears it, so whatever the last `ADD`
left there survives a subtraction untouched. And the overflow expression is
subtly different from `add8`'s — `(a ^ m) & (a ^ r)` rather than
`(a ^ r) & (m ^ r)` — for a reason that the two doc comments spell out and
that this section returns to below.

### Why CMP feels different from SUB, but isn't

This pair shows up in every 6809 program that scans a string for its
terminator:

```asm
        CMPA  #$0D
        BEQ   found_cr
```

It doesn't *feel* like arithmetic. It feels like a primitive comparison
operator — the assembly equivalent of `if (a == 0x0D)`. But you now know
exactly what `CMPA` is: `self.sub8(self.a, m, 0)` with the return value
thrown away. There is no separate "compare" circuit on the 6809, and there
is no separate `cmp8` function in this emulator either — `CMPA #$0D` and
`SUBA #$0D` run *the exact same Rust function* on *the exact same inputs*
and leave *the exact same flags*. The only difference between the two
instructions, anywhere in this codebase, is one line: whether the arm writes
`sub8`'s return value back into `self.a` or lets it fall on the floor. Every
conditional branch that gets chained after a `CMP` — `BEQ`, `BNE`, `BLO`,
`BHI`, `BLT`, `BGT`, `BLE`, `BGE` — is reading flags that a plain `SUB` would
have produced identically. "Compare-and-branch" on the 6809 is really
"subtract-and-branch-on-the-leftover-flags," and now you can see why in the
source instead of taking the datasheet's word for it.

One more thing falls out of this once you notice `sub8`'s `borrow_in`
parameter: `CMPA` always calls it with `0`, never with the carry flag —
there is no "compare with borrow" instruction on the 6809, the way `SBCA`
exists alongside `SUBA`. That's not a gap: you never need one.
For a quantity that's already 16 bits, `CMPD`/`CMPX`/`CMPY`/`CMPU`/`CMPS`
compare the whole thing in one shot (`sub16`, no separate borrow-in either —
see below). For a wider multi-byte comparison you'd chain by hand, subtract
low bytes with `SUBB` and high bytes with `SBCA`, then read the flags off
the *last* subtraction. At that point, though, you're computing a real
difference you intend to keep, so you reach for `SBC` rather than `CMP`,
precisely because you already know `CMP` is only ever the zero-borrow-in,
throw-away-the-result case.

### `exec_alu8`, in full

Section 2.2 showed you individual arms from `exec_alu8` scattered across
`step`'s dispatch comment. Here's the whole function
([`crates/mc6809/src/exec/exec_data.rs:46`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec/exec_data.rs#L46)) — five operations (`ADD`, `ADC`,
`SUB`, `SBC`, `CMP`, each in `A` and `B` variants) across three addressing
modes, thirty opcode bytes, and not one of them does anything but shuffle
arguments into `add8`/`sub8`:

```rust
pub(super) fn exec_alu8(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
    match opcode {
        // ADDA
        0x8B => { let m = self.fetch_u8(bus);        self.a = self.add8(self.a, m, 0); 2 }
        0x9B => { let m = self.read_direct8(bus);    self.a = self.add8(self.a, m, 0); 4 }
        0xBB => { let m = self.read_extended8(bus);  self.a = self.add8(self.a, m, 0); 5 }
        // ADDB
        0xCB => { let m = self.fetch_u8(bus);        self.b = self.add8(self.b, m, 0); 2 }
        0xDB => { let m = self.read_direct8(bus);    self.b = self.add8(self.b, m, 0); 4 }
        0xFB => { let m = self.read_extended8(bus);  self.b = self.add8(self.b, m, 0); 5 }

        // ADCA
        0x89 => { let c = self.cc & cc::CARRY; let m = self.fetch_u8(bus);       self.a = self.add8(self.a, m, c); 2 }
        0x99 => { let c = self.cc & cc::CARRY; let m = self.read_direct8(bus);   self.a = self.add8(self.a, m, c); 4 }
        0xB9 => { let c = self.cc & cc::CARRY; let m = self.read_extended8(bus); self.a = self.add8(self.a, m, c); 5 }
        // ADCB
        0xC9 => { let c = self.cc & cc::CARRY; let m = self.fetch_u8(bus);       self.b = self.add8(self.b, m, c); 2 }
        0xD9 => { let c = self.cc & cc::CARRY; let m = self.read_direct8(bus);   self.b = self.add8(self.b, m, c); 4 }
        0xF9 => { let c = self.cc & cc::CARRY; let m = self.read_extended8(bus); self.b = self.add8(self.b, m, c); 5 }

        // SUBA
        0x80 => { let m = self.fetch_u8(bus);        self.a = self.sub8(self.a, m, 0); 2 }
        0x90 => { let m = self.read_direct8(bus);    self.a = self.sub8(self.a, m, 0); 4 }
        0xB0 => { let m = self.read_extended8(bus);  self.a = self.sub8(self.a, m, 0); 5 }
        // SUBB
        0xC0 => { let m = self.fetch_u8(bus);        self.b = self.sub8(self.b, m, 0); 2 }
        0xD0 => { let m = self.read_direct8(bus);    self.b = self.sub8(self.b, m, 0); 4 }
        0xF0 => { let m = self.read_extended8(bus);  self.b = self.sub8(self.b, m, 0); 5 }

        // SBCA
        0x82 => { let c = self.cc & cc::CARRY; let m = self.fetch_u8(bus);       self.a = self.sub8(self.a, m, c); 2 }
        0x92 => { let c = self.cc & cc::CARRY; let m = self.read_direct8(bus);   self.a = self.sub8(self.a, m, c); 4 }
        0xB2 => { let c = self.cc & cc::CARRY; let m = self.read_extended8(bus); self.a = self.sub8(self.a, m, c); 5 }
        // SBCB
        0xC2 => { let c = self.cc & cc::CARRY; let m = self.fetch_u8(bus);       self.b = self.sub8(self.b, m, c); 2 }
        0xD2 => { let c = self.cc & cc::CARRY; let m = self.read_direct8(bus);   self.b = self.sub8(self.b, m, c); 4 }
        0xF2 => { let c = self.cc & cc::CARRY; let m = self.read_extended8(bus); self.b = self.sub8(self.b, m, c); 5 }

        // CMPA (result discarded, flags only)
        0x81 => { let m = self.fetch_u8(bus);        self.sub8(self.a, m, 0); 2 }
        0x91 => { let m = self.read_direct8(bus);    self.sub8(self.a, m, 0); 4 }
        0xB1 => { let m = self.read_extended8(bus);  self.sub8(self.a, m, 0); 5 }
        // CMPB
        0xC1 => { let m = self.fetch_u8(bus);        self.sub8(self.b, m, 0); 2 }
        0xD1 => { let m = self.read_direct8(bus);    self.sub8(self.b, m, 0); 4 }
        0xF1 => { let m = self.read_extended8(bus);  self.sub8(self.b, m, 0); 5 }

        _ => unreachable!("exec_alu8 called for opcode {opcode:#04X}"),
    }
}
```

Read it as a grid, not a list: ten row-groups (`ADD`/`ADC`/`SUB`/`SBC`/`CMP`,
each doubled for `A` and `B`), three columns each (immediate/direct/extended —
indexed is missing on purpose; it's routed through `exec_indexed` instead,
next week's territory). Every cell differs from its neighbors in exactly one
axis at a time: move down a row and the addressing-mode fetch changes
(`fetch_u8` → `read_direct8` → `read_extended8`); move to the `ADC`/`SBC`
rows and a `let c = self.cc & cc::CARRY;` appears before the fetch; move to
the `CMP` rows and the assignment back to `self.a`/`self.b` disappears. No
cell contains logic that isn't one of those three axis changes — everything
that could vary *does* vary along a named axis, and nothing else does. This
is what "the flags live in `add8`/`sub8`, not in the opcode handlers" looks
like at full scale, not just in the one CMPA line quoted above.

That grid property is also how you review code like this, and it is a
technique worth stealing. Nobody can verify thirty opcode arms by reading them
as thirty independent statements; attention fails somewhere around the tenth.
But you can verify a grid, by reading *down* each column and confirming that
only the addressing call changes, then reading *across* each row and
confirming that only the accumulator changes. Anomalies become visually
obvious — a `read_direct8` in the extended column stands out the way a
misaligned character does. The rigid one-line-per-arm formatting, which looks
like a style choice, is what makes that scan possible; the same code broken
across three lines per arm would hide the same bug completely.

> **Rust corner: why `self.a = self.add8(self.a, m, 0)` compiles.**
> That line reads `self.a`, passes it to a method taking `&mut self`, and
> assigns the result back to `self.a` — three uses of `self` in one
> statement, one of them mutable. New Rust users expect the borrow checker
> to object.
>
> It doesn't, for two separate reasons. First, `u8` is `Copy`: `self.a` as
> an argument is a *copy* of the byte, evaluated before the call, not a
> reference into the struct. Second, Rust uses *two-phase borrows* for
> method calls — the `&mut self` receiver starts as a shared reservation
> while the arguments are evaluated and only becomes a real exclusive borrow
> when the call actually begins. By the time `add8` runs, the arguments are
> plain values sitting on the stack and nothing else refers into `self`.
>
> The practical upshot is that the natural way to write ALU code is also the
> way that compiles, and you can stop bracing for a fight. Where the fight
> *does* happen is with non-`Copy` types — and Chapter 1's field-partition
> strategy is the general answer, applied one level up.

### BIT is to AND as CMP is to SUB

The same discard trick shows up one function over, for a different family.
`exec_logic8` ([`crates/mc6809/src/exec/exec_data.rs:140`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec/exec_data.rs#L140)) handles
`AND`/`OR`/`EOR`/`BIT` across immediate, direct, indexed, and extended. This
is the first place in the chapter you'll see the indexed rows' shape, even
before next week's `ea_indexed` is explained: each one returns an `(ea, ic)`
pair — an address and its extra cycle cost — exactly parallel to extended
mode's single `ea`:

```rust
pub(super) fn exec_logic8(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
    match opcode {
        // ANDA
        0x84 => { let m = self.fetch_u8(bus);        self.a &= m; self.set_nz8(self.a); 2 }
        0x94 => { let m = self.read_direct8(bus);    self.a &= m; self.set_nz8(self.a); 4 }
        0xA4 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.a &= m; self.set_nz8(self.a); 4 + ic }
        0xB4 => { let m = self.read_extended8(bus);  self.a &= m; self.set_nz8(self.a); 5 }
        // ANDB
        0xC4 => { let m = self.fetch_u8(bus);        self.b &= m; self.set_nz8(self.b); 2 }
        0xD4 => { let m = self.read_direct8(bus);    self.b &= m; self.set_nz8(self.b); 4 }
        0xE4 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.b &= m; self.set_nz8(self.b); 4 + ic }
        0xF4 => { let m = self.read_extended8(bus);  self.b &= m; self.set_nz8(self.b); 5 }

        // ORA
        0x8A => { let m = self.fetch_u8(bus);        self.a |= m; self.set_nz8(self.a); 2 }
        0x9A => { let m = self.read_direct8(bus);    self.a |= m; self.set_nz8(self.a); 4 }
        0xAA => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.a |= m; self.set_nz8(self.a); 4 + ic }
        0xBA => { let m = self.read_extended8(bus);  self.a |= m; self.set_nz8(self.a); 5 }
        // ORB
        0xCA => { let m = self.fetch_u8(bus);        self.b |= m; self.set_nz8(self.b); 2 }
        0xDA => { let m = self.read_direct8(bus);    self.b |= m; self.set_nz8(self.b); 4 }
        0xEA => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.b |= m; self.set_nz8(self.b); 4 + ic }
        0xFA => { let m = self.read_extended8(bus);  self.b |= m; self.set_nz8(self.b); 5 }

        // EORA
        0x88 => { let m = self.fetch_u8(bus);        self.a ^= m; self.set_nz8(self.a); 2 }
        0x98 => { let m = self.read_direct8(bus);    self.a ^= m; self.set_nz8(self.a); 4 }
        0xA8 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.a ^= m; self.set_nz8(self.a); 4 + ic }
        0xB8 => { let m = self.read_extended8(bus);  self.a ^= m; self.set_nz8(self.a); 5 }
        // EORB
        0xC8 => { let m = self.fetch_u8(bus);        self.b ^= m; self.set_nz8(self.b); 2 }
        0xD8 => { let m = self.read_direct8(bus);    self.b ^= m; self.set_nz8(self.b); 4 }
        0xE8 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.b ^= m; self.set_nz8(self.b); 4 + ic }
        0xF8 => { let m = self.read_extended8(bus);  self.b ^= m; self.set_nz8(self.b); 5 }

        // BITA (A AND m, discard result)
        0x85 => { let m = self.fetch_u8(bus);        self.set_nz8(self.a & m); 2 }
        0x95 => { let m = self.read_direct8(bus);    self.set_nz8(self.a & m); 4 }
        0xA5 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.set_nz8(self.a & m); 4 + ic }
        0xB5 => { let m = self.read_extended8(bus);  self.set_nz8(self.a & m); 5 }
        // BITB
        0xC5 => { let m = self.fetch_u8(bus);        self.set_nz8(self.b & m); 2 }
        0xD5 => { let m = self.read_direct8(bus);    self.set_nz8(self.b & m); 4 }
        0xE5 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read(ea); self.set_nz8(self.b & m); 4 + ic }
        0xF5 => { let m = self.read_extended8(bus);  self.set_nz8(self.b & m); 5 }

        _ => unreachable!("exec_logic8 called for opcode {opcode:#04X}"),
    }
}
```

Look at the last two row-groups. `BITA`/`BITB` are not a fourth logic
operation sitting next to `AND`/`OR`/`EOR` — they're `AND` with the same
discard-the-result move you just read on `CMP`. `0x85 => { let m =
self.fetch_u8(bus); self.set_nz8(self.a & m); 2 }` computes `self.a & m` and
feeds it straight to `set_nz8` without ever assigning it anywhere, exactly
the way `CMPA`'s arm fed `sub8`'s return value nowhere. The 6809 assembly
idiom this powers is as common as the `CMP`/`BEQ` pair: `BITA #$80` /
`BMI negative_bit_set` tests one bit of `A` without disturbing it. Every
reference describes `BIT` as leaving `A` alone, and here is exactly why: the
`&` happens, the flags get set from it, and the computed byte has nowhere to
go. `AND`/`OR`/`EOR` all call `set_nz8`, so — per the "load clears V"
convention described below — every one of these arms clears `V` and leaves
`C`/`H` untouched, whether or not the result gets written anywhere.

There is a second lesson buried in this function, and it is about where the
indexed rows are. Notice that `exec_logic8` handles four addressing modes
including indexed, while `exec_alu8` handles only three and hands indexed off
to `exec_indexed`. Two family functions, two different answers to the same
organizational question, sitting side by side in the same file. Neither is
wrong, and the inconsistency is worth seeing rather than smoothing over,
because it is what real code looks like when it grows: the logic family was
small enough to keep its indexed rows local, and the arithmetic family — with
five operations across two accumulators — was not. When you go looking for
`ADDA` indexed and it isn't where `ANDA` indexed was, that's why.

### Half-carry and why it exists at all

`H` (bit 3 carry, from `(a & 0x0F) + (m & 0x0F) + carry_in > 0x0F`) has
exactly one consumer in this ISA: `DAA`. It's carried along by every
`ADD`/`ADC` regardless, in case the next instruction decimal-corrects the
result. `sub8` deliberately does *not* compute `H` — its doc comment says so
plainly ("H is left undefined (unaffected here)") — because subtraction
never needs decimal-adjusting on the 6809, so the code doesn't pretend to
compute a flag nothing reads.

That asymmetry looks like an oversight until you know what `H` is for.
Binary-coded decimal was a routine way for eight-bit software to handle
decimal quantities exactly: packed BCD stores two decimal digits per byte,
arithmetic is performed in plain binary, and a correction step afterwards puts
the digits back into range. The correction has to know whether the low digit
overflowed, and plain binary addition records that nowhere — hence a dedicated
half-carry flag, computed on every single add, for the benefit of one
instruction that most programs never execute. `DAA` is the payoff, and the
last subsection of §2.5 walks it.

### The signed overflow test, decoded

The `V` line in `add8` — `(a ^ r) & (m ^ r) & 0x80 != 0` — is the textbook
signed-overflow rule in bitwise clothes. From the doc comment: *"operands
share a sign that differs from the result."* `a ^ r` has bit 7 set exactly
when `a` and the result disagree in sign; `m ^ r`, likewise for `m`. AND them
and bit 7 survives only when *both* operands disagreed with the result's
sign — which can only happen if `a` and `m` shared a sign to begin with
(positive + positive = negative, or the reverse: the only two ways signed
8-bit addition overflows). `sub8`'s rule mirrors this for subtraction, per
its own doc comment — *"minuend and subtrahend differ in sign and the result
sign differs from the minuend"* — as `(a ^ m) & (a ^ r) & 0x80 != 0`.

Why the two expressions differ is worth reasoning through rather than
memorizing, because it is the same insight from two angles. Adding two numbers
of *opposite* sign can never overflow — the result is between them in
magnitude — so the only dangerous case for addition is same-sign operands,
which is what `add8` tests for indirectly. Subtracting is adding the negation,
so the dangerous case inverts: subtracting a *positive* from a *negative* (or
vice versa) is what can push you off the end of the range, while `a - m` with
`a` and `m` of the same sign always lands somewhere between them. Hence
`(a ^ m)`, which is set exactly when the operands' signs differ. Both
expressions then AND in `(a ^ r)`: did the result end up disagreeing with the
minuend? Same question, opposite precondition.

Check it against a case whose answer is known in advance: `$7F + $01` must
set `V` (a positive byte overflowing into negative territory). `a = 0x7F`,
`m = 0x01`, `r = 0x80`. `a ^ r = 0xFF`, `m ^ r = 0x81`. AND: `0x81` — bit 7
set, `V` fires. Exactly the case in [`tests/alu.rs::adda_signed_overflow`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/alu.rs), walked
through in §2.8.

Now check a case that must *not* set it, since a rule that only ever fires is
no rule at all: `$FF + $01`, which is `-1 + 1` in signed terms and wraps to
zero. `a = 0xFF`, `m = 0x01`, `r = 0x00`. `a ^ r = 0xFF`, `m ^ r = 0x01`. AND
them: `0x01` — bit 7 clear, so `V` stays clear, correctly, because `-1 + 1 =
0` is a perfectly representable answer. Note what *does* fire in that case:
the unsigned carry, since `0xFF + 0x01 = 0x100 > 0xFF`. Same two bytes, `V`
clear and `C` set, and both are right — a demonstration that the two flags
answer questions about two different interpretations and that the hardware
computes both without needing to know which one you meant.

### The "load clears V" convention

Loads, stores, and logic operations don't compute a signed-overflow condition
at all — there's no subtraction or addition to overflow. But the 6809
datasheet still specifies `V = 0` after every `LD`/`ST`/`AND`/`OR`/`EOR`.
That convention lives in one small function, `set_nz8`
([`crates/mc6809/src/alu.rs:36`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs#L36)):

```rust
/// Set N and Z from an 8-bit result and clear V (the LD/ST/logic convention;
/// C and H are left unaffected).
pub(crate) fn set_nz8(&mut self, value: u8) {
    self.set_overflow(false);
    self.set_nz8_only(value);
}
```

Two lines, and the composition is the point: `set_nz8` is `set_nz8_only` plus
one extra promise. Splitting it that way means an operation that needs `N` and
`Z` without touching `V` — and §2.6 has several — calls the inner function
directly, while everything obeying the load convention calls the outer one.
The choice between them is a single identifier at each call site, and getting
it wrong is a flag bug that no amount of staring at the arithmetic will
reveal.

Every `LDA`/`LDB`/`STA`/`STB` arm from §2.4 calls `set_nz8`, never
`set_nz8_only` directly — that's how `V` ends up cleared after a load without
every arm saying so. Compare `set_nz16` ([`alu.rs:63`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs#L63), same shape, 16-bit) used
by `LDD`/`LDX`/`LDY`, and `set_z16` ([`alu.rs:43`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs#L43)) used only by `LEAX`/`LEAY`,
which — unusually — touch `Z` and *nothing else*, not even `N`. Three
closely related helpers, each named for exactly the flags it touches;
picking the right one is picking the right datasheet row.

`set_z16`'s existence is the strongest evidence that this family of helpers
was derived from the datasheet rather than from intuition. Nothing about
computing an address makes `Z` interesting and `N` uninteresting; it is simply
what the CC table specifies, and the doc comment records it as a fact rather
than explaining it away — *"Used by LEAX/LEAY, which touch no other condition
codes."* A programmer writing flag code from first principles would tidy that
asymmetry into consistency without noticing. A programmer transcribing the
table would not, and a function named for exactly one flag is what keeps the
next reader from tidying it later.

### The 16-bit echo: `ADDD`/`SUBD`/`CMPX`

Everything in this section has a 16-bit twin, and seeing one is enough to
convince you that the resemblance is design rather than coincidence.
`exec_16bit` ([`crates/mc6809/src/exec/exec_data.rs:194-208`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec/exec_data.rs#L194-L208)) opens with `ADDD`/`SUBD`/
`CMPX`:

```rust
// ADDD
0xC3 => { let m = self.fetch_u16(bus);       let r = self.add16(self.d(), m); self.set_d(r); 4 }
0xD3 => { let m = self.read_direct16(bus);   let r = self.add16(self.d(), m); self.set_d(r); 6 }
0xE3 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read_u16(ea); let r = self.add16(self.d(), m); self.set_d(r); 6 + ic }
0xF3 => { let m = self.read_extended16(bus); let r = self.add16(self.d(), m); self.set_d(r); 7 }
// SUBD
0x83 => { let m = self.fetch_u16(bus);       let r = self.sub16(self.d(), m); self.set_d(r); 4 }
0x93 => { let m = self.read_direct16(bus);   let r = self.sub16(self.d(), m); self.set_d(r); 6 }
0xA3 => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read_u16(ea); let r = self.sub16(self.d(), m); self.set_d(r); 6 + ic }
0xB3 => { let m = self.read_extended16(bus); let r = self.sub16(self.d(), m); self.set_d(r); 7 }
// CMPX (result discarded)
0x8C => { let m = self.fetch_u16(bus);       self.sub16(self.x, m); 4 }
0x9C => { let m = self.read_direct16(bus);   self.sub16(self.x, m); 6 }
0xAC => { let (ea, ic) = self.ea_indexed(bus); let m = bus.read_u16(ea); self.sub16(self.x, m); 6 + ic }
0xBC => { let m = self.read_extended16(bus); self.sub16(self.x, m); 7 }
```

`CMPX` is `sub16` with its result unused, at exactly the byte and line
position you'd predict after reading `CMPA`. `add16`/`sub16`
([`alu.rs:254`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs#L254), [`alu.rs:264`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs#L264)) are `add8`/`sub8` widened to `u16`: same carry
rule (`sum > 0xFFFF` instead of `> 0xFF`), same signed-overflow XOR-and-mask
rule against bit 15 instead of bit 7, same N/Z convention — with one thing
quietly missing. Neither has an `H` parameter, because no 16-bit instruction
on the 6809 needs a nibble-carry flag; `DAA` only ever operates on `A`,
never on `D` as a whole, so there is nothing 16-bit for half-carry to serve.
The rest of `exec_16bit` — `LDX`/`STX`/`LDU`/`STU`, elided here — is the
same load/store shape you already read in full for `LDD`/`STD` back in
§2.2's `exec_load_store`; nothing new happens there either.

Also missing from the 16-bit twins: a carry-in parameter. `add16` and `sub16`
take two arguments where their 8-bit counterparts take three, because the
6809 has no `ADCD` or `SBCD` — the 16-bit operations exist to be the top of
a multi-precision chain, not a link in the middle of one. When a program needs
32-bit arithmetic it builds it out of the 8-bit `ADC`, one byte at a time, and
`add16` is never in the loop. Absent parameters tell you as much about an
instruction set as present ones do.

### `DAA`: the payoff for `H`

`daa()` ([`alu.rs:178`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs#L178)) is the one place `H` gets read back:

```rust
pub(crate) fn daa(&mut self) -> u32 {
    let a = self.a;
    let lsn = a & 0x0F;
    let msn = a >> 4;
    let mut corr = 0u8;
    if self.cc & cc::HALF_CARRY != 0 || lsn > 9 {
        corr |= 0x06;
    }
    if self.cc & cc::CARRY != 0 || msn > 9 || (msn > 8 && lsn > 9) {
        corr |= 0x60;
    }
    let result = a.wrapping_add(corr);
    self.a = result;
    self.set_carry(corr & 0x60 != 0);
    self.set_nz8_only(result);
    2
}
```

The shape matches any hand-written BCD-adjust routine: correct the low
nibble by `+6` if it's out of BCD range (`>9`) *or* the last `ADD` reported a
nibble carry (`H`); correct the high nibble by `+6` (shifted, `0x60`) under
the equivalent condition, plus the case where the low correction is about to
ripple into the high nibble (`msn > 8 && lsn > 9`). The doc comment is honest
about one gap — *"V is left undefined (untouched here)"* — matching real
6809 silicon, where `DAA`'s effect on `V` is undocumented and the code
doesn't invent a value for it.

Why `+6` specifically? The answer is the arithmetic behind the whole
instruction, and it takes one line to see: a nibble holds sixteen values but a
decimal digit uses ten, so whenever a digit's addition spills past 9, it has
landed six short of where the next digit should begin. Adding 6 pushes it over
that gap and produces the carry into the next nibble that decimal arithmetic
wanted all along. The `0x60` for the high nibble is the same 6, shifted into
position.

Walk it against the datasheet rules with the two cases the codebase itself
tests, [`crates/mc6809/tests/interrupts.rs:83`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/interrupts.rs#L83) and [`:94`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/interrupts.rs#L94) (yes, that file —
more on the filename in §2.8). First, a plain BCD add with no carry chain:
`$64 + $27` in packed BCD is "64 + 27 = 91," and binary addition gets you
partway there. The test drives it by loading the *already-added* binary sum
straight into `A` and running `DAA` alone:

```rust
#[test]
fn daa_adjusts_bcd_sum() {
    // $64 + $27 = binary $8B; DAA -> BCD $91.
    let mut s = Sys::code(0x0000, &[0x19]);
    s.cpu.a = 0x8B;
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x91);
    assert_eq!(cycles, 2);
    assert_eq!(s.cpu.cc & cc::CARRY, 0);
}
```

Trace `daa()` by hand against `a = 0x8B` with a fresh CC (`H = 0`, `C = 0`,
the state `Sys::code` starts from). `lsn = 0xB` (11), `msn = 0x8`. Low-nibble
test: `H` is clear, but `lsn = 11 > 9` — the digit itself is out of BCD
range — so `corr |= 0x06`. High-nibble test: `C` is clear, `msn = 8` is not
`> 9`, and the ripple case needs `msn > 8` (`8 > 8` is false) — so the high
nibble gets **no** correction. `corr` stays `0x06`. `result = 0x8B + 0x06 =
0x91` — exactly BCD "91," exactly what the test asserts, and `corr & 0x60 ==
0` so `C` stays clear, matching the third assertion. This is the ordinary
case: one BCD digit spilled past 9 during the binary add, `DAA` nudges only
that digit back into range, no carry out of the byte.

Now the case where the whole byte overflows BCD range — decimal 100 doesn't
fit in two BCD digits, so the correction has to produce a carry:

```rust
#[test]
fn daa_produces_carry() {
    let mut s = Sys::code(0x0000, &[0x19]);
    s.cpu.a = 0x9A;
    s.step();
    assert_eq!(s.cpu.a, 0x00); // 0x9A + 0x66 = 0x100
    assert_ne!(s.cpu.cc & cc::CARRY, 0);
    assert_ne!(s.cpu.cc & cc::ZERO, 0);
}
```

`a = 0x9A`: `lsn = 0xA` (10), `msn = 0x9`. Low-nibble test: `lsn = 10 > 9` —
`corr |= 0x06`. High-nibble test: this time it's the *third* clause that
fires — `msn > 8` (`9 > 8`, true) **and** `lsn > 9` (`10 > 9`, true) — the
low-nibble correction is about to carry into the high nibble, so the high
correction has to apply too, even though `msn` itself isn't `> 9` yet:
`corr |= 0x60`. `corr = 0x66`. `result = 0x9A + 0x66 = 0x100`, truncated to a
`u8` by `wrapping_add`: `0x00` — exactly the test's comment. `corr & 0x60 !=
0`, so `set_carry(true)` fires: two BCD digits' worth of value overflowed the
byte, and the carry out is the only place that "hundreds" digit can go —
precisely how a multi-byte BCD add chains `DAA` after `DAA` across bytes on
real hardware, carry flag feeding the next byte's `ADCA`. `result == 0`
also trips `Z`, matching the test's last assertion for a reason that has
nothing to do with the carry chain — it's just what `0x9A + 0x66` happens to
wrap to this time.

Those two tests between them cover the three clauses of the high-nibble
condition, and that coverage is no accident. Two carefully
chosen inputs pin down a condition with three alternatives, because each input
was picked to make a different clause the deciding one. That is what a good
flag test looks like, and it is the standard exercise 2.1 asks you to meet.

---

## 2.6 Read-modify-write: one dispatcher, keyed on a nibble

The instructions covered so far read an operand and write a register. The
read-modify-write family works the other way round: it reads a location,
transforms the byte in place, and writes it back to the same address. Eleven
operations qualify — `NEG`, `COM`, `LSR`, `ROR`, `ASR`, `ASL`, `ROL`, `DEC`,
`INC`, `TST`, and `CLR` — and each is available on `A`, on `B`, and on memory
in three addressing modes, which would be somewhere north of fifty opcodes if
each got its own arm. It doesn't come to fifty arms, because the 6809's opcode
map hands the emulator a gift.

Section 2.2 flagged that the RMW opcode range encodes addressing mode in the
high nibble and operation in the low nibble. `exec_rmw`
([`crates/mc6809/src/exec/exec_data.rs:236`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec/exec_data.rs#L236)) reads the high nibble (via the
opcode range it's matched under) and hands the low nibble to a second-level
dispatcher, `rmw_apply` ([`alu.rs:156`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs#L156)):

```rust
/// Dispatch an 8-bit read-modify-write op by the opcode's low nibble and
/// return the new value (flags set as a side effect). NEG(0), COM(3), LSR(4),
/// ROR(6), ASR(7), ASL/LSL(8), ROL(9), DEC(A), INC(C), TST(D), CLR(F). TST
/// returns its input unchanged (flags only — caller must not write it back);
/// illegal nibbles (1,2,5,B,E) are no-ops.
pub(crate) fn rmw_apply(&mut self, op_nibble: u8, m: u8) -> u8 {
    match op_nibble {
        0x0 => self.sub8(0, m, 0), // NEG is 0 - m
        0x3 => self.com8(m),
        0x4 => self.lsr8(m),
        0x6 => self.ror8(m),
        0x7 => self.asr8(m),
        0x8 => self.asl8(m),
        0x9 => self.rol8(m),
        0xA => self.dec8(m),
        0xC => self.inc8(m),
        0xD => {
            self.set_nz8(m); // TST: flags only
            m
        }
        0xF => self.clr8(),
        _ => m, // illegal nibble
    }
}
```

This is a two-dimensional decode collapsed into two one-dimensional ones. The
high nibble picked the addressing mode back in `step`; the low nibble picks
the operation here; and because the two are independent in the opcode map,
they can be independent in the code. Five call sites — inherent-A, inherent-B,
direct, indexed, extended — share one operation table of eleven entries, in
place of fifty-odd arms each repeating both decisions. When you meet a
processor whose opcode map has this kind of structure, exploiting it is
usually right; the trap is exploiting *apparent* structure that the map only
mostly has, which is why the rest of `step` enumerates its opcodes by hand.

`NEG` is implemented as literally nothing new: `0 - m`, routed through the
same `sub8` you already read in §2.5 — negating a byte and subtracting it
from zero are the same operation, and the flags fall out identically (a
borrow occurs, and thus `C` sets, for every input except `0`). The five
callers of `rmw_apply` — inherent-A, inherent-B, direct, indexed, extended —
all funnel through this one nibble switch; the only thing that differs
between them is *where `m` came from* and *whether the result gets written
back*.

The gaps in the nibble list are as informative as the entries. Nibbles 1, 2,
5, `B`, and `E` have no operation assigned — the 6809 simply doesn't define
those opcodes — and `rmw_apply` returns the input unchanged for them rather
than panicking. That is `step`'s "undecoded opcodes are no-ops" policy from
§2.2, applied one level down: a program that jumps into data can produce these
bytes, so they must not crash the emulator, and the doc comment records
exactly which nibbles they are so the next reader doesn't have to work it out.

### The per-operation flag quirks

`rmw_apply` routes to ten small functions plus one inline arm for `TST`, and
while they all share the same short shape, their flag rules are the least
uniform corner of the instruction set. The comment block that introduces them
([`alu.rs:68`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs#L68)) is the densest documentation in the crate:

```rust
// ---- 8-bit read-modify-write primitives -------------------------------
// Each returns the transformed value and sets condition codes per the
// MC6809 datasheet. Flag subtleties verified against the Motorola CC tables:
// COM forces C=1; LSR forces N=0 and leaves V untouched; LSR/ASR/ROR leave V
// unaffected; INC/DEC leave C unaffected.

/// COM: one's complement. N,Z from result; V cleared; C set to 1.
fn com8(&mut self, m: u8) -> u8 {
    let r = !m;
    self.set_overflow(false);
    self.set_nz8_only(r);
    self.cc |= cc::CARRY;
    r
}
```

Every clause in that comment is a place where the obvious implementation would
be wrong. `COM` forces `C` to 1 unconditionally — not "sets it from
something," just sets it — which no amount of reasoning about one's complement
will predict; it is a datasheet row, and the code says so with a bare
`self.cc |= cc::CARRY;`. `LSR` forces `N` to 0, because a logical right
shift puts a zero into bit 7 and the sign bit therefore cannot be set; the
code gets this for free by calling `set_nz8_only` on the shifted result, which
is the honest way to arrive at a documented constant. `INC` and `DEC` leave
`C` alone, which is why loops that use `DEC` as a counter can carry a flag
across the decrement — a fact assembly programmers rely on constantly and that
an emulator author "simplifying" the flag code would break.

Look at which helper each of them calls, because that is where the subtleties
actually live. `com8` calls `set_overflow(false)` and `set_nz8_only` — it
clears `V` explicitly and never touches `C` through the helper, then forces
`C` itself. `lsr8`, `asr8`, and `ror8` call `set_nz8_only` and never mention
`V` at all, which is exactly what "leaves V unaffected" means expressed in
code. The choice between `set_nz8` and `set_nz8_only` in each of these
functions is not stylistic; it *is* the datasheet's `V` column, transcribed.

### The `TST` exception

That last point is the sharpest edge in this section. `TST` computes flags
like every other RMW op but must never write its "result" anywhere: it is a
read-only flags probe. On real hardware the write-back would be an extra,
unwanted bus write — harmless against RAM, a real bug against a
write-sensitive I/O register. `rmw_apply` handles this by returning `m`
completely unchanged for the `TST` nibble (`0xD`) — the transform is the
identity function — but the caller still has to know not to write it back,
and does, explicitly, in all three memory-operand arms of `exec_rmw` ([`exec/exec_data.rs:243`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec/exec_data.rs#L243)):

```rust
// Direct
0x00..=0x0F => {
    let op = opcode & 0x0F;
    let ea = self.ea_direct(bus);
    let m = bus.read(ea);
    let r = self.rmw_apply(op, m);
    if op != 0x0D { bus.write(ea, r); } // TST: no write-back
    6
}
```

`0x0D` here is the literal nibble from the `rmw_apply` doc comment, matched
by eye against the `TST` row — not a named constant. The inherent-mode arms
(`0x40..=0x4F` for A, `0x50..=0x5F` for B) need no such guard: assigning the
identity transform back into `self.a` is a no-op by construction, so `TSTA`
works without one — "write the unchanged value back" and "don't write" are
indistinguishable when there's no bus in between.

That last sentence contains the whole reason this case is dangerous, so it is
worth restating in the negative. The guard's absence is *invisible* in every
context where the write target is a register, and *invisible* in every context
where the write target is plain RAM, because writing a byte's own value back
into RAM changes nothing observable. It becomes visible only when the target
is a hardware register whose write side does something — and the CoCo's I/O
page is full of those. A missing `if op != 0x0D` would therefore pass the
entire CPU test suite, pass a boot test, and then corrupt something the first
time a program did `TST $FF02`. Exercise 2.4 walks straight into this and asks
you to work out what a test would have to assert to catch it.

The literal `0x0D`, repeated in three arms with no name attached, is the other
half of the risk. Chapter 1's house style calls for named constants over
magic numbers, and this is a place the codebase does not follow it; the nibble
is documented in `rmw_apply`'s comment and matched by eye at each call site.
Whether that is a defect or an acceptable local convention is a fair question
to hold while reading — and a fair thing to change, once you have run the
suite and know what would catch you if you got it wrong.

### `ASL`/`ROL`'s overflow bit, decoded

`asl8` and `rol8` ([`alu.rs:133`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs#L133), [`alu.rs:142`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs#L142)) both compute `V` the same
non-obvious way:

```rust
fn asl8(&mut self, m: u8) -> u8 {
    let r = m << 1;
    self.set_carry(m & 0x80 != 0);
    self.set_overflow((m ^ (m << 1)) & 0x80 != 0);
    self.set_nz8_only(r);
    r
}
```

`(m ^ (m << 1)) & 0x80` tracks two bits. `m << 1` shifts every bit of `m` up
one position (truncating as a `u8` — bit 7 falls off the top, which is what
feeds `C` separately via `m & 0x80`), so bit 7 of `m << 1` is bit 6 of the
*original* `m`. XOR that against `m`'s own bit 7, mask to bit 7: the result
is `m`'s bit 7 XOR `m`'s bit 6 — precisely the signed-overflow condition for
a left shift. In two's complement, shifting left by one preserves sign only
if the top two bits already agreed (`00...` stays positive, `11...` stays
negative); the moment bit 7 and bit 6 disagree, the shift flips the sign,
which is overflow for an operation meant to be "multiply by 2." `ROL` uses
the identical expression: the carry-in it rotates into bit 0 doesn't touch
the sign bit, so the overflow test doesn't need to know about carry-in.

Two test cases in `tests/logic_rmw.rs` pin down both halves of that
condition, and they are worth having in mind before exercise 2.3 asks you to
break this line. `asla_sets_overflow_and_carry` runs `ASLA` with `A = 0x80`:
bit 7 is 1, bit 6 is 0, they disagree, `V` sets — and the result is `0x00`, so
`Z` sets and `C` catches the departing bit 7.
`asla_no_overflow_when_signs_stable` runs the same instruction with
`A = 0xFF`: bits 7 and 6 are both 1, they agree, `V` stays clear, and the
result `0xFE` is still negative, which is the point. Same opcode, same code
path, opposite answers, both derived from the same XOR.

---

## 2.7 Cycle counting: one number per instruction

Chapter 1 argued that cycles are the currency of the whole machine: video
timing, audio rates, cassette bit periods, and floppy pacing are all
conversions from a cycle count, and Chapter 6's main loop will be denominated in
them. That makes this the section where the CPU starts paying into the rest of
the emulator, and it is remarkable how little machinery the payment requires.

You've now read enough arms to notice the pattern: every single one ends in
an integer literal — `2`, `4`, `5`, `6 + ic` — or delegates to a function that
returns one. That number is the *entire* timing model. [DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5 states
the policy directly:

> Don't try to be cycle-*exact* mid-instruction at first; instruction-granular
> cycle counts are enough to get the ROM booting and sync interrupts roughly
> right. Tighten later only if a game needs it.

In practice, `step()` never tracks *when* during an instruction a bus access
happens, only that the whole instruction cost some fixed number of cycles
matching the datasheet's per-opcode table. `LDA` extended costs 5 cycles no
matter which of those 5 cycles the fetch, EA computation, and memory read
would "really" occupy on silicon. That's enough for everything Chapter 6
builds — a scanline loop that runs instructions until roughly 57 cycles have
elapsed, then does video/audio/timer work — because nothing downstream needs
to know that cycle 3 specifically is when the address bus becomes valid. The
moment something *would* need that (a demo whose raster trick depends on
exact sub-instruction bus timing) is called out in [DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) as a
deliberately deferred scope, not an oversight. `self.cycles: u64` — "the
machine's clock" from Chapter 1 — is simply the running total of these
per-instruction numbers.

That is the rung-2 position on Chapter 1's fidelity ladder — "instruction- or
byte-granular," results right and time accounted for at whole-instruction
resolution — chosen deliberately and recorded in the table in §1.6. The
question that decides whether rung 2 is enough is always the same one: who
notices? Software notices sub-instruction timing only if it can observe the
bus between one instruction's start and its end, and on this machine that
requires either a cycle-precise raster trick or a peripheral whose timing is
tighter than an instruction. Neither is in scope before Chapter 12.

Not every instruction charges a constant, though, and the exceptions are all
principled. There are exactly three shapes:

- **A literal.** Most instructions. `LDA` extended is `5`, always.
- **A base plus the postbyte's extra cost**, written `4 + ic` or `6 + ic`.
  This is the indexed forms, where `ea_indexed` returns both an address and
  the extra cycles that particular sub-mode costs. Chapter 3 fills in where `ic`
  comes from; this week, read `+ ic` as "indexed addressing charges for its
  own complexity."
- **A count computed at run time.** `PSH`/`PUL` are the clearest case: the
  cost depends on how many registers the mask names, so
  [`lib.rs:107`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L107) defines a base and the transfer adds one cycle per byte
  moved: *"Base cycle count for PSH/PUL, before adding one cycle per byte
  transferred."* Pushing `CC` alone and pushing every register are genuinely
  different amounts of work, and the model says so.

The long conditional branches from §2.2 are a fourth, milder case — 6 taken,
5 not — and together these four shapes cover the whole ISA. The thing
they have in common is that every one of them is *derivable from the
datasheet's table*, which is what keeps the model checkable. When Chapter 4's
interrupt code arrives with its own costs (19 cycles for `SWI`, 20 for `SWI2`,
22 for `CWAI`), you will be able to look each one up rather than trusting it.

The place where the cycle count is added holds a subtlety of its own. Look at
the bottom of `step`: `self.cycles += cycles as u64;` runs *after* the
instruction's effects are complete. The instruction's whole cost is banked at
its end, not spread across it, which is the same statement as
"instruction-granular" from the scheduler's point of view. When Chapter 6 asks
"has 57 cycles elapsed?", the answer moves in jumps of 2 to 22 rather than
smoothly — the scanline boundary lands wherever an instruction happens to
finish, up to about twenty cycles late. That imprecision is the budget being
spent, and knowing exactly where it is spent is what will let you tighten it
later if some program turns out to notice.

---

## 2.8 How the tests teach

Chapter 1's study method put "read the tests before the implementation" first,
on the argument that a test states a hardware fact in five lines where the
implementation spreads it across a decode chain and three helper functions.
This section makes good on that claim for the material of this chapter, and it
also introduces the harness every CPU test in the book uses.

The `mc6809` test suite isn't incidental — it's written so that reading one
test file after reading the source teaches you the same flag rules a second
way: concretely, with real numbers. Take `adda_signed_overflow` from
[`crates/mc6809/tests/alu.rs:57`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/alu.rs#L57):

```rust
#[test]
fn adda_signed_overflow() {
    let mut s = Sys::code(0x0000, &[0x8B, 0x01]); // ADDA #$01
    s.cpu.a = 0x7F;
    s.step();
    assert_eq!(s.cpu.a, 0x80);
    // 0x7F + 1 = 0x80: positive+positive -> negative -> V and N set; H from low nibble.
    assert_eq!(flags(&s), (true, true, false, true, false));
}
```

`Sys` ([`crates/mc6809/tests/common/mod.rs:7`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/common/mod.rs#L7)) formalizes the Chapter 1 exercise's
`FlatBus`-backed toy: a real `MC6809` wired to a real `FlatBus`, `Sys::code`
loading a byte program and pointing `PC` at it, `s.step()` calling
`self.cpu.step(&mut self.bus)` directly. `0x8B` is `ADDA` immediate (§2.5's
`0x8B => { let m = self.fetch_u8(bus); self.a = self.add8(self.a, m, 0); 2 }`)
— so this test asks exactly what you can now answer by hand: what happens
when `A = 0x7F` and you `ADDA #$01`?

The `flags(&s)` helper is worth knowing by name, because every arithmetic test
in the suite ends with it. It is a small function that packs the five flags
into a tuple in the fixed order `(H, N, Z, V, C)`, defined at the top of both
`tests/alu.rs` and `tests/logic_rmw.rs` — separately, since Rust compiles each
integration test file as its own crate. That ordering is a convention you
have to internalize once and then never think about again; misreading position
2 as `V` instead of `Z` is the most common way to misjudge one of these
assertions at a glance.

Work it with the §2.5 rule before reading the assertion. `a = 0x7F`,
`m = 0x01`, `carry_in = 0`. Low nibble: `0xF + 0x1 = 0x10 > 0x0F` — `H` sets.
Full sum `0x7F + 0x01 = 0x80`, not `> 0xFF` — `C` clear. Signed overflow:
`a ^ r = 0xFF`, `m ^ r = 0x81`, AND `0x81`, bit 7 set — `V` sets (positive
plus positive landing on a negative bit pattern). `r & 0x80 != 0` — `N`
sets. `r == 0`? No — `Z` clear. In the test's `(H, N, Z, V, C)` order:
`(true, true, false, true, false)` — exactly the tuple asserted, and exactly
what the comment above it says in English. When source, comment, and test
all agree three different ways, you've found a fact worth trusting
completely — matching that three-way agreement is the bar for tests you
write yourself this week.

A second one, this time confirming the BIT-is-AND-and-discard claim from
§2.5 rather than a flag formula — `bita_sets_flags_without_changing_a` from
[`crates/mc6809/tests/logic_rmw.rs:67`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/logic_rmw.rs#L67):

```rust
#[test]
fn bita_sets_flags_without_changing_a() {
    let mut s = Sys::code(0x0000, &[0x85, 0x80]); // BITA #$80
    s.cpu.a = 0xC0;
    s.step();
    assert_eq!(s.cpu.a, 0xC0); // unchanged
    assert_eq!(flags(&s), (false, true, false, false, false)); // 0xC0 & 0x80 = 0x80
}
```

The test's own name states the claim it exists to pin down: *unchanged*.
Trace it against `exec_logic8`'s `0x85` arm from §2.5:
`self.set_nz8(self.a & m)`. `self.a & m = 0xC0 & 0x80 = 0x80` — bit 7 set,
so `N` sets; the value isn't zero, so `Z` clears; `set_nz8` unconditionally
clears `V`; `C`/`H` are untouched by `set_nz8` and start clear on a fresh
`Sys`, so they read clear. `(false, true, false, false, false)` — matches.
And the assertion that actually matters for this test's *point*,
`assert_eq!(s.cpu.a, 0xC0)`, isn't testing a flag formula at all — it's
testing that `exec_logic8`'s `0x85` arm never contains a `self.a = ...`
anywhere, the same "look for the absent assignment" reading you did by eye
on `CMPA`'s arm in §2.5, now automated into something CI runs on every
commit.

A third, from `tests/loads.rs`, shows the same technique applied to addressing
rather than to flags — and it is the direct-mode claim of §2.3 reduced to five
lines:

```rust
#[test]
fn lda_direct_uses_dp() {
    // DP=$12, operand $34 -> effective address $1234.
    let mut s = Sys::code(0x0000, &[0x96, 0x34]);
    s.cpu.dp = 0x12;
    s.set_mem(0x1234, 0x56);
    let cycles = s.step();
    assert_eq!(s.cpu.a, 0x56);
    assert_eq!(cycles, 4);
}
```

The choice of `$12` and `$34` is the tell. Any pair of bytes would exercise
the code path, but this pair makes the concatenation legible: `DP` is `$12`,
the operand is `$34`, and the address the test pokes is `$1234`. Read the
setup and you can see the effective address without doing arithmetic, which
means a reader can check the *test* as easily as the test checks the code.
Choosing values that make the invariant visible costs nothing and is the
difference between a test that documents behavior and one that merely
verifies it.

One filename oddity worth flagging while you're in the test directory: the
two `DAA` tests walked in §2.5 live in [`tests/interrupts.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/interrupts.rs), not a
`tests/daa.rs` or `tests/misc_inherent.rs` you might expect from the opcode
map. The file's own header comment says why — it bundles "the misc inherent
ops (ORCC/ANDCC/SEX/ABX/MUL/DAA) and the interrupt / halt subsystem"
together, two unrelated corners of the ISA that happen to share one thing:
neither fits cleanly into the load/store, ALU, logic, indexed, 16-bit, or
RMW families this chapter organizes around. Test file boundaries in this
codebase generally track the `exec_*` family split you learned in §2.2, but
not perfectly — when a test's contents don't match its filename's obvious
guess, that's a signal about the *code's* organization, not a bug in the
tests. `exec_misc_inherent` (§2.2's dispatch table, the `0x1A | 0x1C | 0x1D
| 0x3A | 0x3D | 0x19` arm) is exactly this leftover-bin shape in the
executor too — `DAA` sits in a family function whose members have nothing
in common beyond "not big enough to deserve its own arm."

There is one more thing to take from the test files before you go read them,
and it is in their headers rather than their bodies. `tests/logic_rmw.rs`
opens with a nine-line summary of flag conventions — *"COM: N,Z; V cleared; C
forced to 1"*, *"LSR: N forced to 0; V unaffected; C = old bit 0"*, and so on
— introduced by the phrase *"verified against the Motorola MC6809 CC
tables."* That header is a specification, written down where the tests that
enforce it can be read against it. When you extend this suite, extending the
header is part of the job; a test that pins down a rule nobody wrote down is
a rule that will get "simplified" away in six months.

---

## 2.9 Reading assignment

This week's reading is the largest single body of source in the course, and
the order below is chosen so that each file answers a question the previous
one raised. Budget an evening. The goal is not to memorize opcode bytes — the
instruction card exists for that — but to be able to predict, for any opcode,
which file and which function you'd land in.

1. **[`crates/mc6809/src/exec.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs), all of it** — the `step` match, then each
   `exec_*` family function, until you can say for any opcode byte on your
   instruction card which family it lands in and why.
2. **[`crates/mc6809/src/exec/exec_data.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec/exec_data.rs), all of it** — the bulk of the
   ISA's byte count. Read `exec_load_store`/`exec_alu8` closely; skim
   `exec_indexed`/`exec_16bit` (indexed addressing is next week; the 16-bit
   ops are §2.5's primitives applied to `u16`).
3. **[`crates/mc6809/src/alu.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/alu.rs), all of it** — the shortest, densest file in
   the crate: every flag rule in the ISA lives here exactly once.
4. **[`crates/mc6809/src/addressing.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs), lines [1–57](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L1-L57) plus [173–192](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L173-L192)** — the
   `fetch_*`/`ea_*`/`read_*` helpers; skip the indexed postbyte decoder in
   the middle, that's next week.
5. **The tests** — [`tests/loads.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/loads.rs), [`tests/alu.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/alu.rs), [`tests/logic_rmw.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/logic_rmw.rs),
   [`tests/common/mod.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/common/mod.rs) — a second explanation of §2.5–2.6, in numbers
   instead of formulas.

Run the suite and watch these specific files' worth of tests pass:

```
cargo test -p mc6809 --test loads --test alu --test logic_rmw
```

---

## 2.10 Exercises

**2.1 — Hand-trace `ADCA #$7F` (build).** With `C = 1` and `A = $80` going in,
predict all five flags (`H N Z V C`) using the `add8` rule from §2.5 before
touching a keyboard. Then write a test in the style of `tests/alu.rs` —
`Sys::code(0x0000, &[0x89, 0x7F])`, `s.cpu.a = 0x80`,
`s.cpu.cc |= cc::CARRY` — and check your prediction against `s.cpu.a` and the
flags. If hand-trace and test disagree, find which one is wrong.

**2.2 — Trace `STD $0500` end to end (build).** In the shape of §2.4's
`LDA $0400` walkthrough, write out every fetch, effective-address
computation, and flag update for `STD $0500` (opcode `$FD`) with `D = $BEEF`
going in. Predict the cycle count before checking `exec_load_store`'s `0xFD`
arm, and predict what `set_nz16` does to `V`/`N`/`Z`. Verify with
`s.mem(0x0500)`/`s.mem(0x0501)` and `s.cpu.cc`.

**2.3 — Sabotage: delete `ASL`'s overflow computation (sabotage, verified).**
Remove the `self.set_overflow(...)` line from `asl8` in `alu.rs` (leave
`rol8` untouched). Predict which test in `tests/logic_rmw.rs` fails, and
why — which existing assertion actually depends on `ASL` setting `V`, versus
merely not contradicting a default of "clear"? Then run
`cargo test -p mc6809 --test logic_rmw` and check. (Verified for this
chapter: exactly one test fails, `asla_sets_overflow_and_carry` — `V = true`
expected on `ASLA` with `A = 0x80`, reported `false` by the broken code. Its
neighbor, `asla_no_overflow_when_signs_stable`, keeps passing — explain why
it can't tell the difference.) Revert before continuing.

**2.4 — The `TST` write-back trap (sabotage, trickier — verify, don't
assume).** In `exec_rmw`'s extended-mode arm, remove the
`if op != 0x0D { ... }` guard so every RMW op writes back unconditionally,
including `TST`. Predict whether `tst_extended_does_not_write`
(`tests/logic_rmw.rs`) now fails, then run it. If the result surprised you,
explain — from `rmw_apply`'s `TST` arm and from what `FlatBus::write` does —
why the existing suite can't catch this bug, and what a test would need to
assert to catch it. Then explain why the missing guard is still a real bug
against `SystemBus` on real hardware even though no `mc6809` test notices.
Revert before continuing.

**2.5 — Read: why a `match`, not a table (read).** Re-read §2.2's
family-function split against [DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5's framing of the indexed
postbyte as "the hardest part." In three to five sentences: why does routing
a large fraction of the ISA through one `ea_indexed` function (next week)
make the `match`-as-table design *more* attractive, not less, than a
256-entry function-pointer table? (Hint: what does a function-pointer table
force every entry's signature to look like, versus what the indexed forms
need to return?)

**2.6 — Recall: the CC layout (recall).** From memory, write the eight CC
bits in order (`E F H I N Z V C`) with their hex masks, and name the one
ALU-visible flag whose only consumer this week is a single non-arithmetic
instruction. Check against `cc` in [`lib.rs:47`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L47) and §2.5's `DAA` discussion.

**2.7 — Sabotage: delete `add8`'s half-carry line (sabotage, verified —
broader than you'd guess).** In `alu.rs`, remove just the
`if half > 0x0F { self.cc |= cc::HALF_CARRY; }` lines from `add8` (leave the
`half` computation itself and everything else untouched, so `H` now always
reads clear coming out of `ADD`/`ADC`). §2.5 said `H` has "exactly one
consumer in this ISA: `DAA`" — before running anything, guess: does that mean
only one test in `tests/alu.rs` should care whether `H` is right? Write down
your guess, then run `cargo test -p mc6809 --test alu` and read the actual
failure list. (Verified for this chapter: **four** tests fail —
`adda_half_carry_from_low_nibble`, `adda_carry_and_zero_wraps`,
`adda_signed_overflow`, and `adca_carry_in_completes_wrap` — every test
whose scenario happens to carry out of the low nibble, regardless of what
else it's testing. `adda_immediate_plain` and `adca_adds_carry_in` keep
passing.) In a sentence or two: reconcile the "only one consumer" fact from
§2.5 with a four-test blast radius — what does that tell you about the
difference between "how many things in the ISA *use* a flag's value" and
"how many tests exist to pin down a flag's *correctness*"? Revert before
continuing.

**2.8 — Sabotage: weaken `DAA`'s low-nibble test (sabotage, verified).** In
`daa()`, change `if self.cc & cc::HALF_CARRY != 0 || lsn > 9` to just
`if self.cc & cc::HALF_CARRY != 0` (drop the `|| lsn > 9` clause; leave the
high-nibble `if` alone). Using §2.5's two worked traces
(`daa_adjusts_bcd_sum`, `$8B → $91`, and `daa_produces_carry`, `$9A → $00`
with carry), predict by hand — with `H` clear on a freshly-loaded `Sys`, the
way both tests set it up — what `corr` each one now computes, and whether
each test's `assert_eq!` on `s.cpu.a` still holds. Then run
`cargo test -p mc6809 --test interrupts` and check both predictions.
(Verified for this chapter: **both** `daa_adjusts_bcd_sum` and
`daa_produces_carry` fail — for `$8B`, `corr` drops to `0x00` and `A` stays
`$8B` instead of correcting to `$91`; for `$9A`, only the high-nibble
`0x60` still fires, giving `corr = 0x60` and `A = 0xFA` instead of wrapping
to `$00` with carry.) Revert before continuing.

**2.9 — Trace `SUBB <$20`, direct mode, a third way (build).** Following
§2.4's third trace, write out every step for `SUBB <$20` (opcode `$D0`)
with `DP = $10`, `B = $05`, and `$1020` holding `$08`. Predict the effective
address by hand (DP:offset concatenation, not addition), predict all five
flags from `sub8`'s rule in §2.5, and predict the cycle count before
checking `exec_alu8`'s `0xD0` arm. Then write a `Sys`-based test to confirm.
(Hint: this is a borrow case — `B` ends up smaller than it started, and one
flag in particular should surprise you if you expected subtraction to
behave like addition's mirror image on `H`.)

---

## What's next

Next week you stay inside the CPU and take on the single hardest 200 lines in
it: the indexed-addressing postbyte, `1 rr i mmmm`, which a large fraction of
all instructions route through — 5-bit offsets, accumulator offsets, auto
inc/dec, PC-relative addressing, extended-indirect, and the indirect bit that
triggers a second memory fetch on top of the first. You'll also meet
`PSH`/`PUL`'s register-mask encoding, `TFR`/`EXG`'s nibble codes, and the
disassembler that mirrors the executor byte-for-byte. Keep your instruction
card handy — you'll be hand-decoding postbytes like `$8B`, `$F4`, and `$9F`
before the chapter is out.

Two things from this week become load-bearing there. The `(ea, ic)` pair you
met in `exec_logic8`'s indexed rows is the shape `ea_indexed` returns, and
`ic` is the reason §2.7 has a "base plus extra" cycle case at all — next week
explains where every one of those extra cycles comes from. And the three-cast
sign-extension idiom from §2.2's branch arm reappears immediately, applied to
a 5-bit field rather than an 8-bit one, which turns out to be the one case
where `as i8` isn't enough on its own.
