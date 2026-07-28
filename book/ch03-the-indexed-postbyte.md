# Chapter 3 — The Indexed Postbyte, Stacks, and the Disassembler

*Week 3. Goal: master the single hardest 200 lines in the CPU. Last week you
read `MC6809::step()` end to end and watched every opcode land somewhere in
one big `match`. This week you go back into that `match` and open the one
addressing mode that got skipped: indexed. You'll also finish the
subroutine/stack group (`PSH`/`PUL`/`TFR`/`EXG`) and meet the disassembler,
which mirrors everything you learn here byte-for-byte. Week 4 closes out the
CPU with interrupts — which, not coincidentally, push a stack frame using the
exact `psh` function you'll read today.*

---

Most of the 6809's addressing modes announce themselves in the opcode itself.
`LDA #$0A` is `$86`, immediate. `LDA <$40` is `$96`, direct. `LDA $0400` is
`$B6`, extended. Week 2 walked all three, and in each case the opcode byte
told the dispatcher everything it needed to know about where the operand
would come from; the bytes that followed were nothing but data. Decode was a
single `match` arm, and the arm knew its own answer.

Indexed addressing does not work that way, and the difference is the whole
subject of this chapter. There is exactly one indexed `LDA` opcode — `$A6` —
and it does not say which register the address is built from, whether an
offset is involved, how wide that offset is, whether a register gets modified
as a side effect, or whether the address computed is the address you want or
merely a pointer to it. All of that lives in the byte *after* the opcode, a
byte the 6809 literature calls the *postbyte*. One byte, eight bits, and
something on the order of a hundred distinct meanings packed into them.

The arithmetic explains the shape. Sixteen sub-modes, four index registers,
and an indirect flag multiply out to well over a hundred combinations. Giving
each combination its own opcode would consume half the base page for a single
instruction, and `LDA` is one of roughly two dozen instructions that need
indexed addressing. There is no room. So the encoding pushes the choice down
into a second byte, and every indexed instruction in the ISA shares the same
second-byte grammar — which is precisely why one decoder function can serve
all of them, and precisely why getting that one function wrong breaks
everything at once.

This week stays entirely inside the `mc6809` crate. No new device, no new
seam, nothing that touches the CoCo. What you get instead is the last
genuinely intricate piece of the CPU, plus two smaller encodings that work the
same way (`PSH`/`PUL`'s register mask and `TFR`/`EXG`'s nibble pair), plus the
disassembler — which is both this chapter's answer key and the first component
of the debugger that week 16 assembles.

---

## 3.1 Why 200 lines get a whole week

Before opening any code, it's worth establishing that the difficulty here is
not a matter of taste. DESIGN.md ranks the CPU's hard parts "in order of
pain," and indexed addressing is first on the list, ahead of interrupts:

> 1. **Indexed addressing** — one postbyte encodes ~a dozen sub-modes:
>    constant offsets (5/8/16-bit), accumulator offsets (A/B/D), auto
>    inc/dec by 1 or 2, PC-relative (8/16), extended-indirect, and indirect
>    variants of most. Build one `fn ea_indexed(&mut self, bus) -> (u16 addr,
>    u32 extra_cycles)` and get it bulletproof — a large fraction of all
>    instructions route through it.

"A large fraction" is doing a lot of work in that sentence, so it's worth
making the number concrete. Search the executor for calls to `ea_indexed` and
you will find forty-six of them across two files. Look at how many opcodes
cash that check in [`crates/mc6809/src/exec.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs)
alone: `LEAX`/`LEAY`/`LEAS`/`LEAU`, indexed `LDA`/`STA`/`LDB`/`STB`/`LDD`/
`STD`, every indexed 8-bit ALU op (`ADD`/`ADC`/`SUB`/`SBC`/`CMP` for both
accumulators), indexed `AND`/`OR`/`EOR`/`BIT`, indexed `JMP`/`JSR`, and — via
the `$10`/`$11` prefix pages of §3.8 — indexed `CMPD`/`CMPY`/`LDY`/`STY`/
`LDS`/`STS`/`CMPU`/`CMPS`. Every one of those opcodes calls the same
function, `ea_indexed`, to turn a postbyte into an address. Get that one
function wrong and you don't break one instruction — you break a fraction of
the ISA at once, in ways that only show up as wrong pixels several
instructions later. That's why it earns a whole week of undivided attention.

Forty-six call sites actually undercounts the opcodes involved, because one of
those call sites serves an entire sixteen-opcode range at once. The
read-modify-write group — `NEG`, `COM`, `LSR`, `ROR`, `ASR`, `ASL`, `ROL`,
`DEC`, `INC`, `TST`, `CLR`, all of which you met in week 2 — handles its
indexed forms in a single arm, from
[`exec/exec_data.rs:252-259`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec/exec_data.rs#L252-L259):

```rust
            // Indexed
            0x60..=0x6F => {
                let op = opcode & 0x0F;
                let (ea, ic) = self.ea_indexed(bus);
                let m = bus.read(ea);
                let r = self.rmw_apply(op, m);
                if op != 0x0D { bus.write(ea, r); }
                6 + ic
            }
```

That arm is the shape every indexed opcode in the crate follows, so it is
worth reading slowly even though its subject is last week's material. The call
to `ea_indexed` returns two things at once, destructured into `ea` and `ic`:
the effective address, and the number of *extra* cycles the postbyte cost
beyond the instruction's own base price. The address goes to the bus, the ALU
work happens exactly as it would in any other addressing mode, and then the
last line adds the two costs together — `6 + ic`, where `6` is what indexed
read-modify-write costs before the postbyte gets a vote. Nothing in this arm
knows or cares which of the sixteen sub-modes ran. That ignorance is the
design: `ea_indexed` absorbs the entire complexity of the postbyte and hands
back a pair of plain integers.

There is a second reason this material rewards a full week, and it is the more
practical of the two. Addressing bugs do not announce themselves. A wrong flag
in the ALU tends to break a comparison, and a broken comparison tends to break
a branch, and a broken branch tends to derail visibly and soon. A wrong
effective address, by contrast, reads or writes a perfectly valid byte at a
perfectly plausible address that isn't the right one. The program keeps
running. Whatever was at the wrong address gets used as if it were correct.
The failure surfaces thousands of instructions later, in a subsystem that has
nothing to do with addressing, as a character in the wrong place on the screen
or a disk sector that checksums wrong. Week 4's trace-diffing exists largely to
catch exactly this class of bug, and the cheapest way to avoid needing it is to
get this week's 200 lines right the first time.

---

## 3.2 The decode tree: one bit decides everything

Everything in this chapter hangs off a handful of named bit masks, so those
come first. They live in their own little module, deliberately kept away from
the decoder that uses them, in
[`crates/mc6809/src/lib.rs:76-90`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L76-L90):

```rust
/// Field masks for the indexed-addressing postbyte (`1 rr i mmmm`).
mod postbyte {
    /// Indirect bit.
    pub const INDIRECT: u8 = 0x10;
    /// Shift to bring the 2-bit register selector (bits 5-6) to the low bits.
    pub const REG_SHIFT: u8 = 5;
    /// Sub-mode field (low nibble), valid only when bit 7 is set.
    pub const MODE_MASK: u8 = 0x0F;
    /// 5-bit constant-offset field, valid only when bit 7 is clear.
    pub const OFFSET5_MASK: u8 = 0x1F;
    /// Sign bit of the 5-bit offset.
    pub const OFFSET5_SIGN: u8 = 0x10;
    /// Extra cycles for an indirect fetch (the `[...]` forms).
    pub const INDIRECT_CYCLES: u32 = 3;
}
```

Read the doc comments as a specification rather than as annotation. Two of
those six constants are marked "valid only when bit 7 is set" and "valid only
when bit 7 is clear," and that pair of qualifications is the entire structure
of the postbyte stated in advance: `MODE_MASK` and `OFFSET5_MASK` describe two
different readings of the same byte, and which reading applies is decided by a
single bit. `INDIRECT_CYCLES` is the odd one out — a `u32` cycle count rather
than a `u8` mask — sitting in a module otherwise full of bit patterns, because
the indirect bit is the one field whose meaning includes a price.

> **Rust corner: a private module as a bitflag namespace.** `postbyte` isn't
> a `struct` or an `enum` — it's a bare `mod` holding `pub const` bytes, not
> `pub` itself (visible only inside this crate). No derive, no trait, just
> named constants grouped under one path (`postbyte::INDIRECT`) — a name
> instead of a bare `0x10`, without designing a `bitflags`-style type for a
> mask consumed in exactly one file. `regsel` (§3.7) and `stack_mask` (§3.6)
> use the same trick. When bits need to compose (`OR`ed together, tested
> with `&`), plain `u8` constants serve better than an enum would — enums
> don't overlap bit patterns for free.

With the vocabulary in place, here is the branch that splits the whole
addressing mode in two, in `ea_indexed`
([`crates/mc6809/src/addressing.rs:58-67`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L58-L67)):

```rust
    pub(crate) fn ea_indexed(&mut self, bus: &mut impl Bus) -> (u16, u32) {
        let pb = self.fetch_u8(bus);

        // 5-bit signed constant offset — the only non-indirect-capable form.
        if pb & 0x80 == 0 {
            return self.ea_indexed_offset5(pb);
        }

        self.ea_indexed_full(bus, pb)
    }
```

Bit 7 of the postbyte splits the entire addressing mode into two unrelated
layouts. If it's clear, the *whole remaining byte* — all 7 bits — is spent
on `0 rr nnnnn`: a 2-bit register select and a 5-bit signed offset, nothing
held in reserve. If it's set, the byte is read as `1 rr i mmmm` instead, and
the two forms have nothing in common past the `rr` field. This is not a fast
path and a slow path through one decoder; it is two decoders that happen to
share an entry point.

### The fetch that feeds it

Notice what the first line of `ea_indexed` does before any decoding happens at
all: `self.fetch_u8(bus)`. That call is not a peek. It is the same
instruction-stream fetch the dispatcher itself uses, and it advances the
program counter as a side effect
([`addressing.rs:7-17`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L7-L17)):

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

Five lines each, and between them they account for every byte any instruction
consumes. `fetch_u8` reads at `PC` and steps `PC` forward with `wrapping_add`,
the house style from Chapter 1 for anything that touches a 16-bit address.
`fetch_u16` is just two of those in a row, high byte first, because the 6809 is
big-endian — the same rule `Bus::read_u16` encodes for data reads.

Hold on to this. Several sub-modes call `fetch_u8` or `fetch_u16` again to pull
in an offset operand, and each of those calls advances `PC` further. By the
time a sub-mode finishes, `PC` points at whatever follows the entire
instruction. For most sub-modes that's bookkeeping nobody has to think about.
For the two PC-relative sub-modes it is the whole semantics of the mode, and
§3.4 traces the consequences byte by byte.

### Which register is `rr`?

Both postbyte layouts carry the same 2-bit register field in the same place,
and both hand it to the same tiny pair of helpers
([`addressing.rs:30-48`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L30-L48)):

```rust
    /// One of the four index registers selected by a 2-bit postbyte field
    /// (00=X, 01=Y, 10=U, 11=S).
    fn index_reg(&self, sel: u8) -> u16 {
        match sel & 0b11 {
            0b00 => self.x,
            0b01 => self.y,
            0b10 => self.u,
            _ => self.s,
        }
    }

    fn set_index_reg(&mut self, sel: u8, val: u16) {
        match sel & 0b11 {
            0b00 => self.x = val,
            0b01 => self.y = val,
            0b10 => self.u = val,
            _ => self.load_s(val),
        }
    }
```

Two things about this pair matter later. First, both mask internally: `sel &
0b11`. Callers are free to pass a value that still has junk in its high bits,
which is exactly what they do — the decoder shifts the postbyte right by five
and passes the result straight through without masking, so `sel` routinely
arrives as `0b100` or `0b101` rather than a clean two-bit number. Doing the
mask once, inside the accessor, means neither caller has to remember. The
disassembler's own `index_reg_name` follows the same convention for the same
reason ([`disasm/indexed.rs:97-104`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm/indexed.rs#L97-L104)).

Second, notice that `set_index_reg` exists at all. Only four of the sixteen
sub-modes ever write a register back — the auto increment and decrement forms
of §3.5 — but those four are the reason the whole decoder needs `&mut self`
rather than `&self`. An addressing mode that modifies the machine while
computing an address is unusual enough to be worth flagging now; §3.5 is
devoted to the consequences.

### The 5-bit form, decoded

`ea_indexed_offset5` handles the bit-7-clear layout in eleven lines
([`addressing.rs:71-81`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L71-L81)):

```rust
    fn ea_indexed_offset5(&mut self, pb: u8) -> (u16, u32) {
        use postbyte::*;
        let reg = self.index_reg(pb >> REG_SHIFT);
        let n = pb & OFFSET5_MASK;
        let offset = if n & OFFSET5_SIGN != 0 {
            n as i16 - (OFFSET5_SIGN as i16 * 2)
        } else {
            n as i16
        };
        (reg.wrapping_add(offset as u16), 1)
    }
```

The register lookup is one line. The rest is sign extension done by hand,
because a 5-bit field doesn't line up with any integer type Rust offers. The
test `n & OFFSET5_SIGN` asks whether bit 4 — the sign bit of a 5-bit two's
complement number — is set. If it is, the value is negative, and the code
recovers the true value by subtracting twice the sign bit's weight: `n - 32`.
A postbyte of `$1F` gives `n = 31`, which becomes `31 - 32 = -1`. The
`indexed.rs::offset5_negative_sign_extends` test loads exactly that postbyte
and confirms the effective address lands one byte *below* `X`, at `$1FFF`.

Now the observation that gives this section its title. The 5-bit form has no
indirect variant, and that is a hardware fact rather than an emulator choice:
**this form cannot be indirect because there is no spare bit to hold the
indirect flag.** Seven bits, and all seven are spoken for — two for the
register, five for the offset. The `1 rr i mmmm` layout exists *because* it
sacrifices two offset bits — down to a 4-bit sub-mode selector — to make room
for the `i` bit and a whole family of other addressing tricks. The two forms
aren't "5-bit offset, indirect or not"; they're two different postbyte
grammars sharing one byte's bit budget, and only one affords an indirect flag
at all. Exercise 3.3 asks you to state that argument in your own words, and
exercise 3.10 sets a trap that only springs if you haven't internalized it.

The 5-bit form costs 1 extra cycle. Only one sub-mode in the entire table is
cheaper — plain `,R` with no offset at all, which costs nothing — so among
forms that actually carry an offset, this is as cheap as indexed addressing
gets, and it is also the shortest, since the offset rides inside the postbyte
instead of following it. Shortest encoding and fastest execution both, which
is why hand-written 6809 assembly reaches for `,X` with a small offset
whenever the offset will fit. You will see the real Color BASIC ROM do exactly
that in §3.9, on a stack-relative access two bytes wide and five cycles
cheap.

---

## 3.3 The full form: register, indirect bit, then sixteen sub-modes

When bit 7 *is* set, control passes to `ea_indexed_full`, and this is the
function worth memorizing — not for its length, which is nine lines, but for
its shape, which explains why the sixteen sub-modes below it can afford to
ignore indirection entirely
([`addressing.rs:85-96`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L85-L96)):

```rust
    fn ea_indexed_full(&mut self, bus: &mut impl Bus, pb: u8) -> (u16, u32) {
        use postbyte::*;
        let sel = pb >> REG_SHIFT;
        let indirect = pb & INDIRECT != 0;
        let (mut ea, mut extra) = self.ea_indexed_submode(bus, sel, pb & MODE_MASK);

        if indirect {
            ea = bus.read_u16(ea);
            extra += INDIRECT_CYCLES;
        }
        (ea, extra)
    }
```

Read that shape carefully — it's the whole chapter in five lines. The
sub-mode decoder computes an address and an extra-cycle count exactly as if
indirection didn't exist. *Then*, regardless of which sub-mode ran, the
indirect bit — if set — treats whatever address the sub-mode produced as a
*pointer*: one more 16-bit bus read at that address to get the real
effective address, billed at 3 extra cycles (`postbyte::INDIRECT_CYCLES`).

Indirection, in other words, isn't its own sub-mode. It's a post-processing
step layered uniformly on top of any sub-mode's result. This is worth dwelling
on because it is the single largest simplification in the decoder. A design
that treated indirection as part of the sub-mode table would need roughly
twice as many arms, each duplicating its non-indirect twin's arithmetic and
then adding a dereference. Instead the table has one arm per addressing form,
and the dereference is written once, in the caller, three lines long.

That uniformity glosses over one datasheet nuance, and the code is honest
about it rather than silent. Real hardware documents only some sub-modes as
indirectable, and calls `,R+`/`,-R` — the single-step auto increment and
decrement forms — undefined in combination with indirect. The comments in the
sub-mode table below say so explicitly. But the code doesn't special-case the
restriction: set the indirect bit on a `,R+` postbyte and you get a second
fetch anyway. That is a deliberate choice, not an oversight, and the same
choice is mirrored on the disassembler side, where
[`disasm_indexed.rs::indirect_auto_increment_by_two`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/disasm_indexed.rs)
carries a comment making the parallel explicit. Worth knowing if you ever
chase a compatibility bug in this exact corner; a real assembler will never
emit such a postbyte, so the only way to reach it is hand-assembled bytes or
data being executed by mistake.

Now the sub-mode table itself, `ea_indexed_submode`
([`addressing.rs:100-171`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L100-L171)), quoted whole because every case matters and you
will refer back to this constantly:

```rust
    fn ea_indexed_submode(&mut self, bus: &mut impl Bus, sel: u8, mode: u8) -> (u16, u32) {
        match mode {
            0b0000 => {
                // ,R+  (auto-increment by 1; not indirectable on real silicon)
                let r = self.index_reg(sel);
                self.set_index_reg(sel, r.wrapping_add(1));
                (r, 2)
            }
            0b0001 => {
                // ,R++  (auto-increment by 2)
                let r = self.index_reg(sel);
                self.set_index_reg(sel, r.wrapping_add(2));
                (r, 3)
            }
            0b0010 => {
                // ,-R  (auto-decrement by 1; not indirectable on real silicon)
                let r = self.index_reg(sel).wrapping_sub(1);
                self.set_index_reg(sel, r);
                (r, 2)
            }
            0b0011 => {
                // ,--R  (auto-decrement by 2)
                let r = self.index_reg(sel).wrapping_sub(2);
                self.set_index_reg(sel, r);
                (r, 3)
            }
            0b0100 => (self.index_reg(sel), 0), // ,R  (no offset)
            0b0101 => {
                // B,R  (signed accumulator offset)
                let ofs = self.b as i8 as i16 as u16;
                (self.index_reg(sel).wrapping_add(ofs), 1)
            }
            0b0110 => {
                // A,R
                let ofs = self.a as i8 as i16 as u16;
                (self.index_reg(sel).wrapping_add(ofs), 1)
            }
            0b1000 => {
                // n,R  (8-bit signed offset)
                let ofs = self.fetch_u8(bus) as i8 as i16 as u16;
                (self.index_reg(sel).wrapping_add(ofs), 1)
            }
            0b1001 => {
                // n,R  (16-bit offset)
                let ofs = self.fetch_u16(bus);
                (self.index_reg(sel).wrapping_add(ofs), 4)
            }
            0b1011 => {
                // D,R  (16-bit accumulator offset)
                let ofs = self.d();
                (self.index_reg(sel).wrapping_add(ofs), 4)
            }
            0b1100 => {
                // n,PCR  (8-bit signed offset from the *next* instruction)
                let ofs = self.fetch_u8(bus) as i8 as i16 as u16;
                (self.pc.wrapping_add(ofs), 1)
            }
            0b1101 => {
                // n,PCR  (16-bit offset)
                let ofs = self.fetch_u16(bus);
                (self.pc.wrapping_add(ofs), 5)
            }
            0b1111 => {
                // [n]  extended indirect (register field ignored). The base cost
                // here plus the indirect fetch below sum to the datasheet's 5.
                (self.fetch_u16(bus), 2)
            }
            // Reserved/illegal postbytes (0b0111, 0b1010, 0b1110): behaviour is
            // undefined on hardware; fall back to a plain register read.
            _ => (self.index_reg(sel), 0),
        }
    }
```

That is seventy lines of straight-line code with no shared helpers between the
arms, and the lack of sharing is not laziness. The arms differ from each other
in ways that a helper would have to paper over with flags and conditionals, and
the resulting three-argument `apply_offset(sel, delta, before_or_after)` would
be considerably harder to check against a datasheet table than thirteen
explicit cases are. When the specification is a table, code that looks like the
table is easier to trust.

### Five shapes, sixteen arms

The thirteen live arms reduce to five recognizable shapes, and sorting them
that way makes the table memorable rather than merely long.

The first shape is *mutate and return*: arms `0b0000` through `0b0011`, the
auto increment and decrement forms. They are the only arms that write to a
register, and the ordering of the three lines inside each of them is the entire
semantic difference between them. §3.5 takes those four apart line by line.

The second shape is *the register itself*, arm `0b0100`, the `,R` form. One
expression, no offset, no fetch, no cost. This is the plain dereference —
"the byte X points at" — and it is the cheapest thing the postbyte can express.

The third shape is *register plus a signed offset*, and it accounts for five
arms. `B,R` and `A,R` (`0b0101`, `0b0110`) take the offset from an accumulator,
sign-extended from 8 bits. `n,R` in its 8-bit and 16-bit forms (`0b1000`,
`0b1001`) take it from the instruction stream. `D,R` (`0b1011`) takes it from
the 16-bit accumulator pair. Every one of them ends in the same
`self.index_reg(sel).wrapping_add(ofs)`, and the only interesting variation is
how `ofs` got its value.

There is a real asymmetry hiding in that group, and it catches people. `A,R`
and `B,R` sign-extend, so `A = $FF` means "one byte backward." `D,R` does not
sign-extend, because `D` is already 16 bits wide and there is nothing to
extend — `let ofs = self.d()` takes the register as-is. The practical
consequence is that `D = $FFFF` in `D,R` adds 65535 to the register rather
than subtracting one. Since address arithmetic wraps at 16 bits, those two
descriptions happen to name the same address, which is a coincidence worth
noticing but not worth relying on: the *code paths* are genuinely different,
and a 6309-style widening of the register file would break the coincidence.

The fourth shape is *PC plus a signed offset*, arms `0b1100` and `0b1101`.
Structurally identical to the third shape, except that the base register is
`self.pc` rather than an index register — and, crucially, `self.pc` as it
stands *after* the offset operand has been fetched. That timing is the whole
subject of the `n,PCR` trace in §3.4.

The fifth shape is *the fetched address itself*, arm `0b1111`, extended
indirect. The register field is ignored entirely; the two bytes that follow the
postbyte are the address. On its own this would be a very expensive way to
express extended addressing, which the 6809 already has a dedicated addressing
mode for. It only makes sense with the indirect bit set, which is exactly how a
real assembler emits it, and the comment in the arm says so: the base cost of 2
plus the indirect fetch's 3 sum to the datasheet's documented 5.

Which leaves the reserved patterns — `0b0111`, `0b1010`, and `0b1110` — falling
through to `_ => (self.index_reg(sel), 0)`. These are not a dispatch bug and
not an oversight. They are the emulator's explicit policy for postbyte patterns
the real chip leaves undefined: fall back to a plain register read, charge no
extra cycles, consume no operand bytes, move on. The disassembler makes the
same call and renders it visibly, as §3.9 shows.

`sel` is the raw `rr` field, still shifted but not yet masked; `index_reg`
masks it down to 2 bits itself (`sel & 0b11`), which is why both this
function and `ea_indexed_offset5` can hand it the same unmasked value from
different shift origins. The full table, matching the real source arm for arm
(extra cycles are *before* the indirect bit's own +3, which stacks on top of
any row):

| `mmmm` | Assembly | EA computation | Extra cycles |
|---|---|---|---|
| `0000` | `,R+` | old `R`, then `R += 1` | 2 |
| `0001` | `,R++` | old `R`, then `R += 2` | 3 |
| `0010` | `,-R` | `R -= 1`, then new `R` | 2 |
| `0011` | `,--R` | `R -= 2`, then new `R` | 3 |
| `0100` | `,R` | `R` | 0 |
| `0101` | `B,R` | `R + sign_extend(B)` | 1 |
| `0110` | `A,R` | `R + sign_extend(A)` | 1 |
| `1000` | `n,R` (8-bit) | `R + sign_extend(fetch_u8)` | 1 |
| `1001` | `n,R` (16-bit) | `R + fetch_u16` | 4 |
| `1011` | `D,R` | `R + D` | 4 |
| `1100` | `n,PCR` (8-bit) | `PC(next) + sign_extend(fetch_u8)` | 1 |
| `1101` | `n,PCR` (16-bit) | `PC(next) + fetch_u16` | 5 |
| `1111` | `[n]` | `fetch_u16` (register ignored) | 2 (+3 indirect ⇒ 5 total) |
| `0111`,`1010`,`1110` | reserved | plain `R`, no fetch | 0 |

### What the extra cycles are paying for

The cycle column is not arbitrary, and reading it as a pattern rather than as
fourteen memorized numbers makes the table much easier to hold in your head.

Anything that costs nothing does nothing beyond a register read: `,R` and the
reserved fallbacks. Anything that costs 1 does one small piece of work — a
single operand byte fetched, or a single 8-bit value sign-extended and added.
Anything that costs 4 or 5 is doing 16-bit work: two operand bytes to fetch, or
a wide addition, or in the case of `n,PCR` 16-bit, both. The auto increment and
decrement forms cost 2 and 3, sitting between the two groups, because they add
a register write-back to the register read, and the by-2 forms cost one more
than the by-1 forms.

The one row that looks odd is `D,R` at 4 extra cycles, since `D` is already in
the register file and no operand bytes get fetched. It costs the same as the
16-bit constant-offset form because the *addition* is the expensive part on
real silicon, not the fetch. Read the two rows together and the rule becomes
"16-bit offset arithmetic costs 4, wherever the offset came from," which is
easier to remember than either row alone.

Two entries deserve a second look for reasons that have nothing to do with
cycles. `n,PCR` measures the offset from `self.pc` *after* the operand bytes
have already been fetched — "the address of the *next* instruction," exactly as
the 6809 datasheet defines PC-relative, and §3.4 walks the arithmetic. And the
extended-indirect row's parenthetical, "register ignored," means what it says:
`$9F`, `$BF`, `$DF`, and `$FF` as postbytes all do the same thing, because the
`rr` field is never consulted in that arm. Four encodings, one behaviour.

> **Rust corner: `as i8 as i16 as u16`.** This triple cast, used for every
> signed offset above, is doing real work, not decoration. `self.b as i8`
> reinterprets the bit pattern of an unsigned byte as signed two's-complement
> (`0xFF` becomes `-1`). `as i16` *sign-extends* that into a wider signed
> type (`-1i8` becomes `-1i16`, i.e. `0xFFFF`, not `0x00FF`). The final `as
> u16` just relabels those bits as unsigned so `wrapping_add` — which only
> works between matching types — can add it to a register. Drop the middle
> step (`self.b as i8 as u16` directly) and you'd zero-extend instead:
> `0xFF` would add `255`, not `-1`. It's the same problem the 5-bit offset's
> manual `n as i16 - (OFFSET5_SIGN as i16 * 2)` in §3.2 solves by hand,
> because a 5-bit field doesn't align with any Rust integer width.

> **Rust corner: returning `(u16, u32)` instead of mutating a counter.**
> Every function in this chain returns a tuple of address and extra cycles,
> and every caller destructures it on the spot: `let (ea, ic) =
> self.ea_indexed(bus)`. The alternative — having `ea_indexed` add its own
> cycles directly to `self.cycles` and return only the address — would be
> shorter at each call site and considerably worse. `step()` is the single
> place that commits cycles to the CPU's clock, at the very end of the
> function, with `self.cycles += cycles as u64`; everything below it merely
> *reports* a cost. Keeping that discipline means a cycle count is a value
> you can inspect, test, and compare, which is exactly what the executor
> tests do when they assert `cycles == 8`. It also means the arithmetic that
> combines base cost and postbyte cost is visible at the call site — `4 + ic`,
> `5 + ic`, `6 + ic` — rather than hidden in two functions that have to agree
> with each other by convention.

---

## 3.4 Seven postbytes, end to end

Tables tell you what a decoder does; traces tell you whether you believe it.
This section runs seven real postbytes through the code above, one at a time,
computing the effective address and the cycle bill by hand and then checking
both against the tests. Hand-assembling an indexed operand runs the encoding
direction, which §3.10 covers; the traces below run the reverse, on real
postbyte values, cross-checked against the executor tests in
[`crates/mc6809/tests/indexed.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/indexed.rs).

Six of the seven traces use indexed `LDA`, opcode `$A6`, so that the postbyte
is the only thing that varies. Here is the arm that runs them, along with its
neighbours, from
[`exec/exec_data.rs:103-116`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec/exec_data.rs#L103-L116):

```rust
            0x30 => { let (ea, ic) = self.ea_indexed(bus); self.x = ea; self.set_z16(ea); 4 + ic }
            0x31 => { let (ea, ic) = self.ea_indexed(bus); self.y = ea; self.set_z16(ea); 4 + ic }
            0x32 => { let (ea, ic) = self.ea_indexed(bus); self.load_s(ea); 4 + ic }
            0x33 => { let (ea, ic) = self.ea_indexed(bus); self.u = ea; 4 + ic }

            // LDA / LDB / STA / STB indexed
            0xA6 => { let (ea, ic) = self.ea_indexed(bus); let v = bus.read(ea); self.a = v; self.set_nz8(v); 4 + ic }
            0xE6 => { let (ea, ic) = self.ea_indexed(bus); let v = bus.read(ea); self.b = v; self.set_nz8(v); 4 + ic }
            0xA7 => { let (ea, ic) = self.ea_indexed(bus); bus.write(ea, self.a); self.set_nz8(self.a); 4 + ic }
            0xE7 => { let (ea, ic) = self.ea_indexed(bus); bus.write(ea, self.b); self.set_nz8(self.b); 4 + ic }

            // LDD / STD indexed
            0xEC => { let (ea, ic) = self.ea_indexed(bus); let v = bus.read_u16(ea); self.set_d(v); self.set_nz16(v); 5 + ic }
            0xED => { let (ea, ic) = self.ea_indexed(bus); let v = self.d(); bus.write_u16(ea, v); self.set_nz16(v); 5 + ic }
```

Every base cost quoted below comes from that excerpt. Eight-bit loads and
stores are 4; sixteen-bit `LDD` and `STD` are 5; the four `LEA` opcodes are
also 4, and they are the family that makes this section's arithmetic directly
observable, since `LEAX` writes the computed effective address into `X` where a
test can read it directly. Everything else is `+ ic`, the postbyte's own bill.

The first four arms deserve a passing note even though they are not this
section's subject: `LEAX` and `LEAY` set the `Z` flag from the result while
`LEAS` and `LEAU` set no flags at all, which is a genuine 6809 asymmetry that
[`indexed.rs::leas_does_not_touch_flags`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/indexed.rs)
pins down by pre-setting `Z` and asserting it survives.

**`$05` — `LDA 5,X`.** Binary `0000_0101`. Bit 7 is clear, so this is the
5-bit form: `rr = 00` (X), `n = 00101 = 5`, sign bit (`0x10`) clear ⇒ offset
`+5`. Decoder path: `ea_indexed` → `ea_indexed_offset5`. EA = `X + 5`. Extra
cycles: 1. With `X = $2000`, EA = `$2005` — exactly
[`indexed.rs::offset5_positive`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/indexed.rs), which loads `$42` from `$2005` and asserts
`cycles == 5` (base 4 for `LDA` indexed + 1). The test also asserts
`s.cpu.x == 0x2000` afterwards, which looks redundant until you consider what
it is guarding against: a decoder that accidentally routed this postbyte into
an auto-increment arm would still produce the right address on the first
execution and only diverge on the second.

**`$80` — `LDA ,X+`.** Binary `1000_0000`. Bit 7 set: `sel = pb >> 5 = 0b100`
→ masked to X; indirect clear; `mode = pb & 0x0F = 0b0000`. Decoder path:
`ea_indexed_full` → `ea_indexed_submode` case `0b0000`. With `X = $2000`,
this reads X's *current* value as the EA and only then writes `X+1` back —
the mechanics §3.5 examines in detail. Extra cycles: 2. Total: 4 + 2 = 6,
matching [`indexed.rs::auto_increment_by_one`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/indexed.rs).

**`$98` — `LDA [16,X]`.** Binary `1001_1000`. `sel = pb >> 5 = 0b100` → X;
indirect bit `0x10` — is it set? `0x98 = 0b1001_1000`; bit 4 (value `0x10`)
is `1`. So indirect is *on*. `mode = pb & 0x0F = 0b1000`, the 8-bit-offset
sub-mode: fetch one more byte (`$10` = 16 decimal), EA-before-indirection =
`X + 16`, extra so far = 1. Then `ea_indexed_full` sees the indirect bit,
does a second bus read — `bus.read_u16(ea)` — to fetch the *real* pointer
from that address, and adds `INDIRECT_CYCLES = 3`. Total extra: 1 + 3 = 4.
Grand total: 4 (base) + 4 = 8, matching [`indexed.rs::indirect_8bit_offset`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/indexed.rs)
exactly (it asserts `cycles == 8` with the comment `// 4 + (1 + 3)`).

That test is also the clearest illustration in the suite of what indirection
actually costs in *memory* terms rather than cycles. It plants a two-byte
pointer at `$2010`/`$2011` holding `$4000`, and the payload byte at `$4000`.
Three separate addresses are involved in fetching one byte: the instruction
stream, the pointer, and the data. Real code pays that price when the pointer
is genuinely variable — a jump table, a dispatch vector, a linked structure —
and avoids it otherwise.

**`$99` — `LDA [256,X]`.** Binary `1001_1001` — one bit past `$98`. Same
register field and indirect bit; `mode = pb & 0x0F = 0b1001`, the *16-bit*
offset sub-mode. Two more operand bytes get fetched (`$01 $00` = 256
decimal), EA-before-indirection = `X + 256`, extra so far = 4 (not 1 — the
16-bit-offset row costs 4 extra cycles even without indirection, since it's
two bus reads instead of one; see the §3.3 table). Add the same
`INDIRECT_CYCLES = 3` for the pointer fetch: total extra = 4 + 3 = 7. Grand
total: 4 (base) + 7 = 11. There's no executor test pinning this exact
combination in `indexed.rs`, but it's not a new rule — it's the `1001` row
and the indirect-bit rule from §3.3 composed, the same composition the
`indirect_8bit_offset` test already exercises for the `1000` row. The
disassembler side *is* directly tested: [`disasm_indexed.rs::indirect_16bit_offset`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/disasm_indexed.rs)
confirms the rendering, `[256,X]`, at `len == 4` (opcode, postbyte, two
offset bytes).

**`$81` on `STD` — `STD ,X++`.** A different opcode this time: `STD` indexed
is `$ED`, base cost 5 (16-bit store, not 4). Postbyte `$81` is
`1000_0001` — `sel = 0b100` → X, indirect clear, `mode = 0b0001`: auto-increment
by 2, the pairing `LDD`/`STD` use constantly in real code to walk a buffer
two bytes at a time. With `X = $2000` and `D = $BEEF`, the *old* X (`$2000`)
is the EA — `$BE` lands at `$2000`, `$EF` at `$2001` — and X ends the
instruction at `$2002`. Extra cycles: 3. Total: 5 + 3 = 8. (Verified by hand
against the source; there's no `STD ,X++`-specific test in the suite today —
exercise 3.7 asks you to add one.)

This is the one trace in the section where the *store* direction matters.
`bus.write_u16(ea, v)` puts the high byte at `ea` and the low byte at `ea + 1`,
the big-endian rule from Chapter 1's `Bus` default methods, so a `,X++` walk
writes a 16-bit value and leaves the pointer exactly at the next 16-bit slot.
Pair it with `LDD ,Y++` in a loop and you have the canonical 6809 block-copy
inner loop, two bytes per iteration, with no explicit pointer arithmetic
anywhere in the code.

**`$8C` — `LDA n,PCR` (8-bit).** The syllabus singles this one out, and it
deserves the full trace rather than a summary, because the "relative to the
*next* instruction" rule is the one detail every 6809 newcomer gets wrong
once. [`indexed.rs::pc_relative_8bit`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/indexed.rs) loads the program at `$1000`:

```rust
    // LDA n,PCR at $1000. Offset is from the address of the *next* instruction.
    // opcode@1000, postbyte@1001, offset@1002 -> PC=0x1003 after decode.
    // EA = 0x1003 + 0x10 = 0x1013.
    let mut s = Sys::code(0x1000, &[LDA_INDEXED, 0x8C, 0x10]);
    s.set_mem(0x1013, 0xB2);
```

Walk `self.pc` byte by byte through the three functions this passes
through. It starts at `$1000`. `step()`'s own `fetch_u8` reads the opcode
`$A6` and advances `pc` to `$1001` — that call lives in the dispatcher, not
in any of the addressing code. `exec_indexed` routes `$A6` to
`self.ea_indexed(bus)`, whose *own* `fetch_u8` (the first line of the
function, §3.2) reads the postbyte `$8C` and advances `pc` to `$1002`; since
bit 7 is set, it calls `ea_indexed_full`, which calls `ea_indexed_submode`
with `mode = 0b1100`. Only *there*, inside the `0b1100` arm itself, does a
third `fetch_u8` read the offset byte `$10` — advancing `pc` to `$1003` —
and only *after* that fetch does the same line read `self.pc.wrapping_add
(ofs)`. By the time `self.pc` gets used, it's already `$1003`: the address
of whatever comes *after* this whole 3-byte instruction, not the address of
the offset byte (`$1002`) and certainly not the opcode (`$1000`). `$1003 +
$10 = $1013`, exactly where the test plants `$B2`. Total: base 4 + extra 1
= 5, matching the test's `cycles == 5`.

This is not a coincidence of implementation order — it's the only order
that *can* be correct, because `self.pc` is a single field with no memory
of "where it was two fetches ago." The rule falls out for free from fetching
the offset *before* reading `self.pc`, and it would silently break if
someone captured `self.pc` into a local variable before the `fetch_u8` call
instead of after (exercise 3.9 asks you to verify exactly that, empirically).
The 16-bit form (`$8D`, [`indexed.rs::pc_relative_16bit`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/indexed.rs)) is identical in
shape — `pc` lands at `$1004` after the two-byte offset fetch, `$1004 +
$0100 = $1104` — just with the wider fetch and a heavier bill: extra cycles
5, the single most expensive non-indirect sub-mode in the table.

It is worth asking what `n,PCR` is *for*, since it is the least intuitive of
the sub-modes. The answer is position-independent code. An instruction that
reaches a nearby constant by writing `LDA table,PCR` computes the constant's
address from wherever the instruction happens to be executing, so the same
bytes work correctly whether the routine is assembled at `$1000` or relocated
to `$3F00` at load time. On a machine whose memory map moves under software
control — which, once week 5 introduces the GIME's MMU, describes the CoCo 3
precisely — that property is worth the extra cycle. Note also that the
disassembler renders these operands as raw signed offsets rather than resolved
addresses: [`disasm_indexed.rs::pc_relative_8bit`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/disasm_indexed.rs)
asserts the operand string is `16,PCR`, not the absolute target. Branch targets
get resolved (§3.9); indexed PCR operands do not.

**`$AB` — `LDA D,Y`.** One more accumulator-offset form, on a different
register so it doesn't spoil exercise 3.1's `$8B`. Binary `1010_1011`:
`sel = pb >> 5 = 0b101` → masked to `0b01` = Y; indirect clear; `mode = pb &
0x0F = 0b1011`, `D,R`. With `Y = $3000` and `D = $0050`: `ofs = self.d()` —
note, *not* sign-extended, unlike `A,R`/`B,R`'s `as i8 as i16 as u16` chain.
`D,R` treats `D` as a plain unsigned 16-bit offset, so `D = $FFFF` would add
almost 64K forward, not step one byte backward the way `A = $FF` does in
`A,R`. EA = `$3000 + $0050 = $3050`. Extra cycles: 4, same bill as the
16-bit constant-offset form — both fetch (or, here, already hold) a full
16-bit value and add it in one step. Total: 4 (base) + 4 = 8, the same
shape [`indexed.rs::accumulator_d_offset`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/indexed.rs) asserts for `D,X`.

Seven postbytes, seven different paths through the same function, and every
cycle count derivable from one table plus one addition. If the traces feel
mechanical by the last one, that is the intended outcome: the postbyte is
intricate but not deep, and the way through it is to be relentlessly literal.

---

## 3.5 Auto inc/dec: whose turn is it, old value or new?

Of the sixteen sub-modes, four modify the machine as a side effect of computing
an address, and those four are where correctness gets slippery. The question is
simple to state and easy to get backwards: when a postbyte says `,X+`, is the
effective address the value X held *before* the increment, or the value it
holds after? The syllabus flags exactly this, and all four arms of §3.3 answer
it precisely, by-1 and by-2 forms side by side:

```rust
0b0000 => {
    // ,R+  (auto-increment by 1; not indirectable on real silicon)
    let r = self.index_reg(sel);
    self.set_index_reg(sel, r.wrapping_add(1));
    (r, 2)
}
0b0001 => {
    // ,R++  (auto-increment by 2)
    let r = self.index_reg(sel);
    self.set_index_reg(sel, r.wrapping_add(2));
    (r, 3)
}
```

```rust
0b0010 => {
    // ,-R  (auto-decrement by 1; not indirectable on real silicon)
    let r = self.index_reg(sel).wrapping_sub(1);
    self.set_index_reg(sel, r);
    (r, 2)
}
0b0011 => {
    // ,--R  (auto-decrement by 2)
    let r = self.index_reg(sel).wrapping_sub(2);
    self.set_index_reg(sel, r);
    (r, 3)
}
```

Post-increment reads the register *first* (`r = self.index_reg(sel)`, the
old value) and returns that unmodified `r` as the EA — the write-back
happens on a separate line, after `r` has already been captured. In
`,R+`/`,R++`, the data you touch is the byte (or byte pair) the register
*used* to point at; the register only points past it *afterward*.
Pre-decrement computes `r` as `old - 1`/`old - 2` in the same expression
that will become the new register value, and *that* already-decremented `r`
is what gets returned as the EA. In `,-R`/`,--R`, the decrement happens
first, and the data you touch is at the *new* address. There's no shared
helper between the four branches that could paper over this — each is three
lines of straight-line code, and the ordering of "compute," "write back,"
"return" is the entire semantic difference between them.

The tests make the distinction visible in a way the source alone does not.
[`indexed.rs::auto_increment_by_one`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/indexed.rs)
sets `X = $2000`, plants `$AA` at `$2000`, and asserts both that `A` ends up
holding `$AA` *and* that `X` ends up at `$2001`. Its neighbour
`auto_decrement_by_one` sets the same `X = $2000`, plants `$CC` at `$1FFF`,
and asserts `A == 0xCC` with `X == 0x1FFF`. Two tests, four assertions, and
between them they nail down both halves of both behaviours. Assert only the
loaded value and a decoder that decremented twice would still pass; assert
only the register and a decoder that returned the wrong EA would still pass.
This is the pattern exercise 3.2 asks you to reproduce for `LEAX ,--Y`.

The by-1 and by-2 forms share their ordering exactly — `0b0000`/`0b0001`
are both "old value, then increment"; `0b0010`/`0b0011` are both "decrement,
then new value" — they differ only in the constant passed to
`wrapping_add`/`wrapping_sub` and in the extra-cycle count (2 vs 3, one more
cycle for the second byte of the step). That's not an accident of encoding:
`,R++`/`,--R` exist specifically for 16-bit registers (`X`/`Y`/`U`/`S`) and
16-bit data (`D`, via `LDD`/`STD` — see `$81` in §3.4), where you want the
pointer to land past a whole *word*, not into the middle of one. The by-1
forms are for byte-at-a-time buffers.

### The stack discipline hiding in the four arms

There is a reason the four available forms are post-increment and
pre-decrement rather than all four combinations, and it becomes obvious the
moment you use `S` or `U` as the register.

Consider `STA ,-S` followed later by `LDA ,S+`. The store decrements `S` and
writes at the new, lower address; the load reads at the current address and
then steps `S` back up. That is a push followed by a pull, expressed entirely
in addressing modes, with no dedicated stack instruction involved. And it is
byte-for-byte the same discipline the `psh` and `pul` functions of §3.6
implement in software: pre-decrement to push, post-increment to pull, stack
growing downward. The auto modes and the stack opcodes are two expressions of
one convention.

That symmetry also explains the gaps. Pre-increment and post-decrement would
describe a stack that grows *upward*, which is not the convention this
processor uses anywhere, and the four sub-mode slots those forms would have
occupied are spent on other things instead. Exercise 3.10 sets a trap on
exactly this point: `,S--` looks like it ought to exist and does not.

### One side effect nobody expects: writing back to `S`

Look again at `set_index_reg` from §3.2 and notice that its four arms are not
symmetric. `X`, `Y`, and `U` are written with a plain assignment. `S` is not:

```rust
            _ => self.load_s(val),
```

That asymmetry is the first coupling this course meets between two subsystems
that look unrelated, and it is worth chasing down, because a plain assignment
there would compile, pass every test in `indexed.rs`, and introduce a bug that
only appears under interrupts.

The CPU struct carries a field called `nmi_armed`, and its doc comment in
[`lib.rs:153-158`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L153-L158)
explains what it is for:

```rust
    /// NMI is not recognized until the first program load of the stack
    /// pointer after reset (MC6809 datasheet) — before S is valid an NMI
    /// frame push would scribble through a garbage pointer. Set by any
    /// instruction that writes S (LDS, LEAS, TFR/EXG, indexed `,S++`-style
    /// writeback), cleared by reset.
    pub nmi_armed: bool,
```

Read that list of triggers carefully: `LDS`, `LEAS`, `TFR`/`EXG`, and
"indexed `,S++`-style writeback." The last item is precisely the auto
increment and decrement arms of this section, when `rr` selects `S`. An
addressing mode, in other words, is expected to participate in interrupt
arming. That is an unusual coupling — the sort of thing that looks like a bug
when you meet it cold in someone else's emulator — and the reason it exists is
stated right there in the comment: a non-maskable interrupt that arrives before
the program has established a stack pointer would push a twelve-byte register
frame through whatever garbage `S` happens to contain, corrupting memory the
program has not yet had a chance to claim.

Week 4 builds `take_interrupt` and the `nmi_armed` check that guards it, so the
consequences belong there rather than here. What belongs here is the
observation that this is what a hardware-derived invariant looks like when it
lands in code: not a comment saying "be careful with S," but a specific field
set by a specific enumerated list of instructions, with a datasheet citation
attached. The `load_s` helper at
[`lib.rs:185-189`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L185-L189)
is the single place that maintains the invariant, and every path that
deliberately arms NMI routes through it:

```rust
    /// Load the stack pointer from program action, arming NMI recognition.
    fn load_s(&mut self, v: u16) {
        self.s = v;
        self.nmi_armed = true;
    }
```

Three call sites in this chapter route through it, one for each item in the doc
comment's list. `exec_indexed`'s `LEAS` arm in §3.4 calls `self.load_s(ea)`
where its `LEAX` neighbour writes `self.x = ea` directly. `reg_write`'s `S` arm
in §3.7 calls `self.load_s(value)` where every other 16-bit register gets a
plain assignment. And `set_index_reg`'s fourth arm, above, does the same for the
auto increment and decrement modes. Three files, one invariant, maintained by
routing every write through a single two-line function rather than by
remembering to set a flag in three places.

That is a pattern worth naming, because it recurs constantly in emulator code.
When you meet a one-line helper whose only job is to set a second field
alongside the obvious one, it exists because some hardware rule couples two
things the code would otherwise keep apart, and the helper is where the coupling
is written down. The interesting question to ask about such a helper is always
the same: which callers use it, and are there any that should?

The ordering discipline is the property `LEAX ,--Y` (exercise 3.2) and `STD ,X++`
(exercise 3.7) ask you to pin down with a test: you can't assert the right
answer for either mode without knowing which value shows up as the EA *and*
which value ends up in the register afterward, and they don't always match
the "pointer arithmetic first vs. use first" intuition that anyone arriving
from C's `*p++`/`*--p` brings along — the 6809 rule is symmetric with C, but
it is worth verifying here rather than assuming.

---

## 3.6 PSH/PUL: masks, order, and the "other" stack pointer

The indexed postbyte is not the only place the 6809 packs a decision table into
a byte, and the next two sections cover the other two. `PSH` and `PUL` use a
postbyte that is not a table index at all but a *bitset*: eight bits, eight
registers, push whichever ones are set. It gets its own bitflag module,
[`crates/mc6809/src/lib.rs:95-105`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L95-L105):

```rust
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

Seven of the eight constants are unremarkable. The eighth has a doc comment
twice as long as any of its neighbours, and that comment is the section's
punchline; hold it for a moment.

There are four opcodes in this family — `PSHS`, `PULS`, `PSHU`, `PULU` — and
they compile down to just two functions, distinguished by a boolean.
[`crates/mc6809/src/stack.rs:26-47`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/stack.rs#L26-L47)
implements the push, and the same function serves every explicit stack opcode
*and* the interrupt-frame code you'll read in full next week:

```rust
pub(crate) fn psh(&mut self, bus: &mut impl Bus, mask: u8, to_s: bool) -> u32 {
    // Work on a local pointer; a 16-bit push stores low byte first (at the
    // higher address) then high byte, leaving the value big-endian in memory.
    let mut sp = if to_s { self.s } else { self.u };
    let other = if to_s { self.u } else { self.s };
    let mut push8 = |sp: &mut u16, v: u8, n: &mut u32| {
        *sp = sp.wrapping_sub(1);
        bus.write(*sp, v);
        *n += 1;
    };
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
}
```

Three things in that function are load-bearing, and each of them is a decision
that could have gone another way.

### Push order is fixed, not mask order

The `if` chain always tests `PC`, the other stack pointer, `Y`, `X`, `DP`, `B`,
`A`, and `CC`, in that order, regardless of which bits are set. A mask is a set,
not a sequence — there is no "first bit" in a bitset, and nothing in the
encoding tells the CPU which register the programmer thought of first. So the
order is a property of the instruction rather than of the operand, and the
function hard-codes it.

The consequence is that the stack layout is completely predictable. `PC` goes
on first, which means it lands deepest, at the highest address. `CC` goes on
last, which means it ends up shallowest, at the lowest address — directly under
the stack pointer. That is why `RTI` (next week) can always find `CC` one byte
from the stack pointer, whatever registers a given `PSHS` chose to save, and
why an interrupt handler can peek at the saved condition codes without knowing
anything about the frame's total size.

There is also a symmetry requirement lurking here. `pul` has to walk the same
order in reverse, and nothing in the type system enforces the correspondence —
it is two hand-written `if` chains that happen to mirror each other. Exercise
3.5 asks what happens if you break the symmetry in one direction only, and the
interesting part of the question is whether a round-trip test would even
notice a symmetric break.

### 16-bit values push high byte first

Each 16-bit register is pushed with two calls to `push8`: the low byte first,
then the high byte. That looks backwards until you notice that `push8`
*pre-decrements* the stack pointer before writing. The byte pushed second lands
at the lower address. Net effect: the high byte sits at the lower address, the
value reads big-endian in memory, and the layout matches `Bus::write_u16` from
Chapter 1 exactly.

[`stack.rs::pshs_16bit_is_big_endian`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/stack.rs)
pins this down with `PSHS X` and `X = $1234`: after the push, `$1FFE` holds
`$12` and `$1FFF` holds `$34`. Get this backwards and every 16-bit value that
ever crosses the stack — every return address, every saved index register,
every interrupt frame — comes back byte-swapped.

It's worth contrasting `psh`'s byte-at-a-time approach with the two dedicated
16-bit helpers that sit directly above it in the same file, at
[`stack.rs:9-20`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/stack.rs#L9-L20):

```rust
    /// Push PC (or any 16-bit value) onto the hardware (S) stack, big-endian.
    pub(crate) fn push16_s(&mut self, bus: &mut impl Bus, val: u16) {
        self.s = self.s.wrapping_sub(2);
        bus.write_u16(self.s, val);
    }

    /// Pull a 16-bit value from the hardware (S) stack.
    pub(crate) fn pull16_s(&mut self, bus: &mut impl Bus) -> u16 {
        let v = bus.read_u16(self.s);
        self.s = self.s.wrapping_add(2);
        v
    }
```

Those are what `JSR` and `RTS` use, and they say the same thing more directly:
subtract two, then write a big-endian word. Two implementations of one
convention, because `psh` has to interleave byte-sized and word-sized registers
under mask control while `push16_s` only ever moves a return address. When you
see two functions that agree on a rule but implement it differently, the
question to ask is whether they can drift — and here the answer is that
`stack.rs::jsr_extended_pushes_return_then_rts_returns` and
`pshs_16bit_is_big_endian` both assert the high byte at the lower address, so
a drift in either would be caught.

### `OTHER_STACK_PTR` is the one bit whose meaning moves

Now the doc comment held back earlier. Bit `0x40` selects "the other stack
pointer" — U when the instruction is `PSHS` or `PULS`, S when it is `PSHU` or
`PULU`. Every other bit in the mask names a fixed register. This one names a
role, and which register fills the role depends on the opcode.

That is the entire reason for the `other` local computed at the top of `psh`
from `to_s`. It is also why the corresponding arm in `pul` cannot write
back through the stack pointer's own register; it needs an explicit branch.
[`stack.rs::pshs_can_push_and_pull_u_via_bit6`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/stack.rs) exercises this
directly — walk it, because it's the only test in the suite that drives
`OTHER_STACK_PTR`. The program is `PSHS U` (`$34 $40`) followed by `PULS U`
(`$35 $40`), with `S = $2000` and `U = $1234` going in:

```rust
    s.step(); // PSHS U
    assert_eq!(s.cpu.s, 0x1FFE);
    assert_eq!(s.mem(0x1FFE), 0x12);
    assert_eq!(s.mem(0x1FFF), 0x34);
    s.cpu.u = 0; // clobber
    s.step(); // PULS U
    assert_eq!(s.cpu.u, 0x1234);
    assert_eq!(s.cpu.s, 0x2000);
```

Trace it against `psh`: mask `$40` matches only the `OTHER_STACK_PTR` arm,
so `other = self.u` (`$1234`, captured *before* any push touches `sp`) gets
written big-endian at `$1FFE`/`$1FFF`, and `S` — the stack this `PSHS`
actually uses — ends at `$2000 - 2 = $1FFE`. Nothing here touches `U` itself;
`PSHS` only *reads* `U` to save it, exactly like it would read `A` or `X`. On
the `PULS U` side, `pul`'s `OTHER_STACK_PTR` arm pulls two bytes and, because
`from_s` is `true`, assigns the result to `self.u` — not `self.s` — which is
the whole reason the arm needs the `if from_s { self.u = v; } else { self.s
= v; }` branch instead of just writing back through `sp`'s own register.
`PSHU $40`/`PULU $40` would run the identical mask through the identical
code with `to_s`/`from_s` flipped, saving and restoring `S` from the `U`
stack instead.

### `pul`, the mirror image

Here is the other half, from
[`stack.rs:51-75`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/stack.rs#L51-L75):

```rust
    pub(crate) fn pul(&mut self, bus: &mut impl Bus, mask: u8, from_s: bool) -> u32 {
        let mut sp = if from_s { self.s } else { self.u };
        let mut bytes = 0u32;
        let mut pull8 = |sp: &mut u16, n: &mut u32| {
            let v = bus.read(*sp);
            *sp = sp.wrapping_add(1);
            *n += 1;
            v
        };
        if mask & stack_mask::CC != 0 { self.cc = pull8(&mut sp, &mut bytes); }
        if mask & stack_mask::A != 0 { self.a = pull8(&mut sp, &mut bytes); }
        if mask & stack_mask::B != 0 { self.b = pull8(&mut sp, &mut bytes); }
        if mask & stack_mask::DP != 0 { self.dp = pull8(&mut sp, &mut bytes); }
        if mask & stack_mask::X != 0 { let hi = pull8(&mut sp, &mut bytes); let lo = pull8(&mut sp, &mut bytes); self.x = ((hi as u16) << 8) | lo as u16; }
        if mask & stack_mask::Y != 0 { let hi = pull8(&mut sp, &mut bytes); let lo = pull8(&mut sp, &mut bytes); self.y = ((hi as u16) << 8) | lo as u16; }
        if mask & stack_mask::OTHER_STACK_PTR != 0 {
            let hi = pull8(&mut sp, &mut bytes);
            let lo = pull8(&mut sp, &mut bytes);
            let v = ((hi as u16) << 8) | lo as u16;
            if from_s { self.u = v; } else { self.s = v; }
        }
        if mask & stack_mask::PC != 0 { let hi = pull8(&mut sp, &mut bytes); let lo = pull8(&mut sp, &mut bytes); self.pc = ((hi as u16) << 8) | lo as u16; }
        if from_s { self.s = sp; } else { self.u = sp; }
        PUSH_PULL_BASE_CYCLES + bytes
    }
```

Everything is reversed and nothing is surprising. `pull8` post-increments where
`push8` pre-decremented. The `if` chain runs `CC`, `A`, `B`, `DP`, `X`, `Y`,
other-pointer, `PC` — exactly `psh`'s order read backwards, which is what
pulling from a stack requires. Sixteen-bit registers are reassembled high byte
first, from the lower address, undoing the big-endian layout `psh` created. And
the `OTHER_STACK_PTR` arm is the one that needed room to breathe, spread over
five lines instead of one, because of the branch discussed above.

One structural note worth carrying forward: `pul`'s `self.s` write goes through
a plain assignment here, not through `load_s`, and the same is true of the
final write-back line. That is a place where the `nmi_armed` invariant from §3.5
and this function's mechanics touch, and it is left as an observation rather
than a claim — week 4 is where interrupt arming gets its full treatment and
where the question of which writes to `S` should count as "program action"
belongs.

### What it costs

Both directions cost `PUSH_PULL_BASE_CYCLES` — the value `5`, defined at
[`lib.rs:108`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L108) — plus exactly one cycle per byte transferred. The `bytes`
counter that `push8` and `pull8` increment is not bookkeeping for its own sake;
it is the second half of the instruction's price.

That gives a pleasantly simple mental model: a stack instruction costs five
cycles to exist, plus one per byte it moves. `PSHS A` costs 6. `PSHS X` costs
7. Pushing everything costs `5 + 12` — twelve bytes, because `PC`, the "other"
pointer, `X`, and `Y` are two bytes each while `DP`, `B`, `A`, and `CC` are one
— which is exactly the 17 that
[`stack.rs::pshs_all_registers_cost_17`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/stack.rs)
asserts. Remember that twelve; next week's full interrupt frame is the same
twelve bytes, pushed by this same function with a mask of `0xFF`.

> **Rust corner: an `FnMut` closure capturing a generic `&mut` parameter.**
> `push8` captures `bus` (type `&mut impl Bus`) by reference, but takes `sp`
> and `bytes` as explicit parameters rather than also capturing those
> locals. `sp` is reassigned across eight `if` blocks; a closure holding
> `&mut sp` for its whole lifetime would block any other use of `sp` in
> between — the exact aliasing Rust's capture rules exist to reject. Passing
> `&mut sp` fresh each call sidesteps it: the closure borrows `sp` only for
> one call, then gives it back. Same partition-by-borrow instinct as Chapter
> 1's `Machine`/`SystemBus` split, at function scale.

The four opcodes that reach these two functions are as thin as dispatch gets,
from [`exec.rs:229-232`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L229-L232):

```rust
            0x34 => { let mask = self.fetch_u8(bus); self.psh(bus, mask, true) }
            0x36 => { let mask = self.fetch_u8(bus); self.psh(bus, mask, false) }
            0x35 => { let mask = self.fetch_u8(bus); self.pul(bus, mask, true) }
            0x37 => { let mask = self.fetch_u8(bus); self.pul(bus, mask, false) }
```

Fetch the mask byte, call the function, return whatever cycle count it
computed. Note that these four arms don't add a base cost of their own the way
`4 + ic` did in §3.4 — `psh` and `pul` return the complete price including
`PUSH_PULL_BASE_CYCLES`, because the cost depends on the mask and the mask is
theirs to inspect.

`take_interrupt` (next week's reading) reuses `psh` directly —
`self.psh(bus, 0xFF, true)` for a full NMI/IRQ/SWI frame, `self.psh(bus,
PC_CC_MASK, true)` for FIRQ's PC+CC-only frame, where `PC_CC_MASK =
stack_mask::PC | stack_mask::CC` ([`lib.rs:111`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L111)). Every interrupt frame is
the exact same `psh` you just read, called with a different mask. That is the
payoff for having written the mask handling once: interrupts, which sound like
they need their own stacking machinery, turn out to need two constants.

---

## 3.7 TFR/EXG: nibble codes and the size-mismatch rules

The third and last postbyte encoding in this chapter is the simplest to decode
and the fussiest to get exactly right. `TFR` and `EXG` name two registers in
one byte, four bits each, using their own selector codes
([`crates/mc6809/src/lib.rs:63-74`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L63-L74)):

```rust
mod regsel {
    pub const D: u8 = 0x0;
    pub const X: u8 = 0x1;
    pub const Y: u8 = 0x2;
    pub const U: u8 = 0x3;
    pub const S: u8 = 0x4;
    pub const PC: u8 = 0x5;
    pub const A: u8 = 0x8;
    pub const B: u8 = 0x9;
    pub const CC: u8 = 0xA;
    pub const DP: u8 = 0xB;
}
```

The gap in the numbering is the interesting part. Codes `0x0`-`0x5` name the
six 16-bit registers; `0x8`-`0xB` name the four 8-bit ones. Bit 3 of the code,
in other words, *is* the size flag — set means 8-bit — which is why the size
test in `regs.rs` can be written as a single comparison rather than a lookup,
as you'll see in a moment. The unassigned codes (`0x6`, `0x7`, `0xC`-`0xF`) are
reserved; the disassembler renders those as `?6`, `?7`, and so on rather than
guessing, and [`stack_transfer.rs::tfr_invalid_register_code_marked_clearly`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/disasm/stack_transfer.rs)
confirms that `TFR D,?6` is what comes back for postbyte `$06`.

`TFR` is opcode `$1F`, with the postbyte read as `hi:lo` = source:destination.
`EXG` is `$1E`, the same postbyte shape, but it reads both registers and then
writes both back swapped. Both cost 6 cycles regardless of the sizes involved,
per `exec.rs`'s `exec_control_transfer`
([`exec.rs:217-226`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L217-L226)):

```rust
            0x1F => { let pb = self.fetch_u8(bus); let v = self.tfr_value(pb >> 4, pb & 0x0F); self.reg_write(pb & 0x0F, v); 6 }
            0x1E => {
                let pb = self.fetch_u8(bus);
                let (r0, r1) = (pb >> 4, pb & 0x0F);
                let v0 = self.reg_read(r0);
                let v1 = self.reg_read(r1);
                self.reg_write(r0, v1);
                self.reg_write(r1, v0);
                6
            }
```

Note what `EXG` does *not* do: it never calls `tfr_value`. It reads both
registers into locals, then writes each one back to the other's slot. Both
reads happen before either write, which is what makes a swap a swap rather than
a pair of copies where the second overwrites the first — the same reason the
idiomatic swap in any language needs a temporary. Whether `EXG` gets the
size-mismatch rules right anyway is a question the end of this section answers.

### The generic register file

Both instructions route through a pair of functions that turn a nibble code
into an actual register access
([`crates/mc6809/src/regs.rs:12-46`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/regs.rs#L12-L46)):

```rust
    fn reg_is16(code: u8) -> bool {
        code <= regsel::PC
    }

    pub(crate) fn reg_read(&self, code: u8) -> u16 {
        match code {
            regsel::D => self.d(),
            regsel::X => self.x,
            regsel::Y => self.y,
            regsel::U => self.u,
            regsel::S => self.s,
            regsel::PC => self.pc,
            regsel::A => self.a as u16,
            regsel::B => self.b as u16,
            regsel::CC => self.cc as u16,
            regsel::DP => self.dp as u16,
            _ => 0xFFFF, // invalid on 6809
        }
    }

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

Three details are worth pulling out. `reg_is16` is the one-line consequence of
the numbering gap: any code at or below `PC` (`0x5`) is a 16-bit register,
because the 8-bit codes all start at `0x8`. `reg_read` promotes every 8-bit
register to `u16` so that one function can return one type — the promotion is
zero-extension, which matters in a moment. And `reg_write` truncates on the way
back down with `value as u8`, which is where the 16-to-8 transfer rule actually
lives, as opposed to where you would expect to find it.

The `S` arm of `reg_write` calls `load_s` rather than assigning, which is the
`nmi_armed` coupling from §3.5 showing up again — this is the "TFR/EXG" item in
that field's list of triggers. The invalid arms are worth a glance too:
`reg_read` returns `0xFFFF` for a reserved code, `reg_write` silently does
nothing. Neither panics. An emulator that panicked on a reserved register code
would be an emulator that a corrupted byte stream could crash, which is not a
property you want in something that runs arbitrary 1980s software.

### The size-mismatch rules

The interesting part is what happens when source and destination sizes don't
match, which the datasheet documents but which is easy to get subtly
wrong. [`crates/mc6809/src/regs.rs:51-63`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/regs.rs#L51-L63):

```rust
    pub(crate) fn tfr_value(&self, src: u8, dst: u8) -> u16 {
        let sv = self.reg_read(src);
        match (Self::reg_is16(src), Self::reg_is16(dst)) {
            (false, true) => {
                let b = sv & 0x00FF;
                match src {
                    regsel::A | regsel::B => 0xFF00 | b, // A/B → 16: high byte = $FF
                    _ => (b << 8) | b,                   // CC/DP → 16: both bytes = source
                }
            }
            _ => sv, // same size, or 16 → 8 (reg_write truncates to the LSB)
        }
    }
```

The outer `match` is on a tuple of two booleans, and only one of the four
possible pairs gets special handling: 8-bit source into a 16-bit destination.
That is the case where a value has to be *widened*, and widening requires
inventing bits that the source didn't have. The 6809 documents two different
answers depending on which 8-bit register is the source, and the inner `match`
implements both.

Three documented rules, one function:

| Transfer | Rule | Test |
|---|---|---|
| 16-bit → 8-bit | destination gets the low byte; `reg_write`'s `value as u8` truncates | [`stack.rs::tfr_16_to_8_takes_lsb`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/stack.rs) (`TFR X,A` with `X=$1234` ⇒ `A=$34`) |
| `A`/`B` → 16-bit | high byte forced to `$FF` | [`stack.rs::tfr_accumulator_to_16_sets_ff_high`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/stack.rs) (`TFR A,X` with `A=$7F` ⇒ `X=$FF7F`) |
| `CC`/`DP` → 16-bit | both bytes duplicate the source byte | [`stack.rs::tfr_cc_to_16_duplicates_byte`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/stack.rs) (`TFR CC,X` with `CC=$42` ⇒ `X=$4242`) |

The second rule is the one that surprises people, and the test makes the
surprise explicit by choosing `A = $7F` — a *positive* value in signed terms.
Sign extension would give `$007F`; the actual result is `$FF7F`. The high byte
is `$FF` unconditionally, regardless of the source value, which means this is
not sign extension and not zero extension but a third thing that only makes
sense as a description of what the silicon does. An emulator author working
from intuition rather than from the datasheet would get this wrong in a way no
casual test would catch, since most values people test with happen to be
negative-looking.

Notice the 16→8 truncation isn't handled inside `tfr_value` at all — its
`_ => sv` arm returns the full 16-bit source unchanged, and it's the generic
`reg_write` that does `self.a = value as u8` for an 8-bit destination. The
size-mismatch *rule* lives in `tfr_value`; the *mechanism* lives in
`reg_write`. That split is worth naming because it answers a question about
`EXG` for free.

### Why `EXG` doesn't need `tfr_value`

`EXG` swaps via two independent `reg_read`/`reg_write` calls and never touches
`tfr_value`, which raises an obvious question: does an 8↔16 `EXG` get the
documented behaviour, or does it get whatever falls out?

Half of it falls out correctly. The truncation direction is handled by
`reg_write`, which both instructions share, so `EXG A,X` puts `X`'s low byte
into `A` exactly the way `TFR X,A` would. The widening direction is where the
two instructions diverge: `reg_read` on an 8-bit register zero-extends to
`u16`, so the value written into the 16-bit register is `$00nn` rather than
`$FFnn`. Whether real silicon agrees is a datasheet question this chapter does
not answer, and there is no test in the suite pinning down 8↔16 `EXG` today —
exercise 3.6 asks you to reason about exactly this, and it is a genuinely open
corner rather than a rhetorical one.

What the code *does* demonstrate cleanly is the payoff of putting the
mechanism in the shared function: whatever `reg_write` does about truncation,
both instructions inherit it, and neither can drift from the other.

### `TFR` in the wild

Every `TFR`/`EXG` example so far has been a hand-built test fixture. Here is
one straight from `roms/coco3.rom`, found by disassembling forward through
the ROM and confirmed sane by checking that ~20 instructions on either side
all decode as plausible, non-`???` 6809 code (a static sanity check — this
region isn't reachable from the reset-vector trace in §3.9, so unlike that
one, treat the *addresses* as approximate and the *idiom* as the point):

```text
$82F3: 1F A9        TFR  CC,B
$82F5: 81 98        CMPA #$98
$82F7: 27 1D        BEQ  $8316
$82F9: 81 97        CMPA #$97
$82FB: 27 14        BEQ  $8311
$82FD: 1F 9A        TFR  B,CC
$82FF: BD AD C6     JSR  $ADC6
```

Decode the two `TFR`s by hand: `$A9` is `1010_1001` → `hi = pb >> 4 =
0xA` (`CC`), `lo = pb & 0x0F = 0x9` (`B`) — `TFR CC,B`. `$9A` is `1001_1010`
→ `hi = 0x9` (`B`), `lo = 0xA` (`CC`) — the exact reverse, `TFR B,CC`. Both
registers are 8-bit, so neither transfer touches the size-mismatch rules
above at all — this is `tfr_value`'s `_ => sv` fallthrough, a plain byte
copy each way.

That's the idiom: stash the condition codes computed by
whatever ran just before `$82F3` into `B` (a scratch register `CMPA` won't
touch), run two comparisons that clobber the flags checking which token
this is, branch off on either match — and if neither hits, restore the
*original* flags from `B` before falling into `JSR $ADC6`, which evidently
depends on them. `TFR CC,B`/`TFR B,CC` saves and restores condition codes
across a stretch of code that has to compute new ones of its own — 6 cycles
each way, tying
`PSHS CC` (`5` base `+ 1` byte `= 6`) / `PULS CC` (same) exactly on cycle
count, but never touching the stack pointer or spending a byte of stack.

That's worth something any time nearby code is *also* using `S`-relative
addressing (this routine's neighbors are, a few bytes further down — `LDX
2,S` and `STX 2,S`, both 5-bit-offset forms from §3.2) and you'd rather not
have `PSHS`/`PULS` shift every one of those offsets out from under a value
someone else expects to find at a fixed distance from `S`. It is a small,
concrete illustration of something this course will keep running into: on a
machine this tight, the choice between two equally priced instructions is
usually made by a constraint that has nothing to do with either one's stated
purpose. You'll meet the stack-based version of this exact save/restore
pattern again in week 4, wrapped around every interrupt.

---

## 3.8 Page prefixes: the opcode isn't always one byte

Everything so far has assumed one opcode byte followed by operands. For most
of the ISA that holds, but the 6809 has 256 opcode slots and rather more than
256 instruction-and-mode combinations to name, so two of those slots are spent
buying 512 more. `$10` and `$11` aren't "modifier" bytes layered on top of
another opcode — they're the *first byte* of a two-byte opcode, and the
dispatcher treats them that way structurally, not just semantically. From
`exec.rs`'s top-level `match`
([`exec.rs:58-61`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L58-L61)):

```rust
            // $10/$11 prefix pages: long conditional branches, the 16-bit ops
            // targeting Y/D/S/U, and SWI2/SWI3.
            0x10 => self.exec_page10(bus),
            0x11 => self.exec_page11(bus),
```

Two arms, two function calls, no decoding of any kind at this level. The
base-page `match` recognizes `$10` and `$11` the same way it recognizes `NOP` —
as opcodes with a behaviour — and that behaviour happens to be "decode another
opcode."

`exec_page10` and `exec_page11` are complete second dispatch layers — each
fetches its *own* opcode byte and runs its own `match`:

```rust
fn exec_page10(&mut self, bus: &mut impl Bus) -> u32 {
    let op2 = self.fetch_u8(bus);
    match op2 {
        0x21..=0x2F => { /* long conditional branches: 16-bit offset */ }
        0x83 => { let m = self.fetch_u16(bus); self.sub16(self.d(), m); 5 } // CMPD immediate
        // ... CMPY, LDY, STY, LDS, STS ...
        0x3F => { self.take_interrupt(bus, VECTOR_SWI2, false, false, true); 20 } // SWI2
        _ => 2, // TODO: other $10-page opcodes
    }
}
```

The real function ([`exec.rs:125-173`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L125-L173))
fills in the elided middle with six instruction groups, and one of them is
worth quoting whole because it shows the prefix page doing exactly what week
2's base page did, one level down
([`exec.rs:150-157`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L150-L157)):

```rust
            // LDY
            0x8E => { let v = self.fetch_u16(bus);       self.y = v; self.set_nz16(v); 4 }
            0x9E => { let v = self.read_direct16(bus);   self.y = v; self.set_nz16(v); 6 }
            0xAE => { let (ea, ic) = self.ea_indexed(bus); let v = bus.read_u16(ea); self.y = v; self.set_nz16(v); 6 + ic }
            0xBE => { let v = self.read_extended16(bus); self.y = v; self.set_nz16(v); 7 }
            // STY
            0x9F => { let ea = self.ea_direct(bus);      bus.write_u16(ea, self.y); self.set_nz16(self.y); 6 }
            0xAF => { let (ea, ic) = self.ea_indexed(bus); bus.write_u16(ea, self.y); self.set_nz16(self.y); 6 + ic }
            0xBF => { let ea = self.ea_extended(bus);    bus.write_u16(ea, self.y); self.set_nz16(self.y); 7 }
```

Immediate, direct, indexed, extended — the same four-mode row, the same
`ea_indexed` call, the same `+ ic` arithmetic as everything in §3.4. The second
opcode byte is a full opcode in every sense. And note the second-byte values:
`$8E`, `$9E`, `$AE`, `$BE` are the *same* second-byte numbers that mean `LDX` on
the base page. Page 10 is, to a first approximation, "the base page but with Y
where X was, and S where U was." `$11` is the same shape, smaller: just
`CMPU`/`CMPS` and `SWI3`.

Two things are worth internalizing from this arrangement.

The first is that the base-page `match` never sees `op2`. It fetches exactly one
byte, recognizes it as a page selector, and hands the rest of the instruction to
the sub-dispatcher. Page 10 and page 11 are two independent copies of "byte in,
cycles out." A prefixed instruction's total byte count and cycle cost are the
prefix byte plus whatever the sub-dispatcher consumes, with no coordination
required between the two layers.

The second is that prefixed instructions cost one cycle more than their
unprefixed equivalents, because fetching the extra opcode byte is itself a bus
cycle. Base-page `LDX` immediate (`0x8E`) is 3 cycles; page-10 `LDY` immediate
(`$10 $8E`) is 4. Base-page `CMPX` immediate (`0x8C`) is 4; page-10 `CMPD`
immediate (`$10 $83`) is 5. One extra cycle every time — the prefix byte's own
cost, paid once regardless of which second-byte opcode follows. You can read
that surcharge directly off the two files: compare `exec_16bit`'s `0x8E` arm
returning `3` against `exec_page10`'s `0x8E` arm returning `4`.

There is one more consequence, and it is the one the disassembler cares about.
Both sub-dispatchers end in `_ => 2`, meaning an unrecognized second byte
consumes the prefix and the second byte and nothing else. Two bytes, two
cycles. Any tool that walks the instruction stream has to agree with that
exactly or it will lose sync, which brings us to the disassembler.

The disassembler mirrors this exact two-layer structure — `disasm.rs`'s
`decode_base` intercepts `$10`/`$11` before consulting the base-page table,
then hands off to a page-specific table function
([`disasm.rs:122-134`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm.rs#L122-L134)):

```rust
fn decode_base<F: FnMut(u16) -> u8>(r: &mut Reader<F>, opcode: u8) -> (&'static str, String) {
    match opcode {
        0x10 => {
            let op2 = r.u8();
            render(r, op2, tables::page10_entry(op2))
        }
        0x11 => {
            let op2 = r.u8();
            render(r, op2, tables::page11_entry(op2))
        }
        _ => render(r, opcode, tables::base_entry(opcode)),
    }
}
```

Same shape as the executor: recognize the prefix, consume it, delegate to a
second, independent lookup keyed on the second byte. Three lines of structure
in each of two files, kept in step by nothing but discipline and tests — which
is the subject of the next section.

---

## 3.9 The disassembler: table-driven, and never allowed to lie about length

A disassembler is an odd thing to build in week 3 of a CPU course. It executes
nothing, it is needed by nothing that runs, and the machine boots perfectly
well without it. It earns its place for two reasons. It is the answer key for
everything in this chapter — every postbyte you hand-decode can be checked
against it in one line of Rust — and it is the first piece of the debugger,
which week 16 assembles into a window with a disassembly pane.

[`crates/mc6809/src/disasm.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm.rs) is a pure function over a byte reader — no CPU
struct, no side effects, just `fn disassemble(read: &mut impl FnMut(u16) ->
u8, pc: u16) -> Insn`. Its module doc states its one governing rule plainly:

> `MC6809::step` is the *executing* dispatcher (it boots real BASIC) and is
> therefore the authority for which opcodes exist, which addressing mode
> each uses, and how many bytes each consumes — including which opcodes are
> illegal/undecoded. This module mirrors that opcode map byte-for-byte.

That sentence establishes a hierarchy, and the hierarchy is what makes the
whole arrangement maintainable. There is no shared source of truth that both
modules consult; there is an authority and a mirror. When they disagree, the
executor is right by definition and the disassembler has a bug. Any other
arrangement — a shared table, a code generator, a macro that emits both — would
have to be correct about the *executor's* behaviour including its illegal-opcode
fallbacks, which is a harder thing to be correct about than following it.

The split that makes this maintainable in practice: *which mnemonic and mode
opcode X has* is data (`tables::base_entry`/`page10_entry`/`page11_entry`, plus
small per-nibble arrays for the RMW and branch-condition groups); *how to
render mode Y into an operand string* is one shared function per mode
(`render`, and `indexed::decode_indexed` for indexed). `decode_indexed`
imports the same `postbyte` masks from §3.2 and switches on the same `mmmm`
values as `ea_indexed_submode` — not a second, hand-written copy that could
drift ([`disasm/indexed.rs:17-34`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm/indexed.rs#L17-L34)):

```rust
pub(super) fn decode_indexed<F: FnMut(u16) -> u8>(r: &mut Reader<F>) -> String {
    let pb = r.u8();

    // 5-bit signed constant offset — not indirectable (bit 7 clear).
    if pb & 0x80 == 0 {
        return decode_indexed_offset5(pb);
    }

    let sel = pb >> postbyte::REG_SHIFT;
    let indirect = pb & postbyte::INDIRECT != 0;
    let body = decode_indexed_body(r, sel, pb & postbyte::MODE_MASK);

    if indirect {
        format!("[{body}]")
    } else {
        body
    }
}
```

Set that beside `ea_indexed` from §3.2 and `ea_indexed_full` from §3.3 and the
correspondence is line for line. Bit 7 tested the same way, with the same
comment. The same `REG_SHIFT` and `INDIRECT` constants from the same module.
The same three-step structure of "decode the sub-mode, then apply indirection
uniformly on top." Even the indirection itself is analogous: where the executor
does one more bus read, the disassembler wraps the string in brackets. Two
functions, one grammar.

The sub-mode body function follows the same discipline, including for the
patterns nobody should ever encode
([`disasm/indexed.rs:86-91`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm/indexed.rs#L86-L91)):

```rust
        // Reserved/illegal postbytes (0b0111, 0b1010, 0b1110): the core
        // falls back to a plain register read with 0 extra bytes. Marked
        // with a `???` suffix so this doesn't read as a valid addressing
        // form.
        _ => format!(",{reg}???"),
```

Compare that against `ea_indexed_submode`'s `_ => (self.index_reg(sel), 0)`.
Both consume zero extra bytes; both fall back to the plain register. The
executor's version is silent about it because a running CPU has nobody to tell;
the disassembler's version is loud about it because a human is reading the
output. Same behaviour, different obligation.

### The reader, and why `len` is the product

Before the tables, one small type that carries more weight than its size
suggests. Every byte the disassembler consumes goes through a `Reader`, and
the `Reader` counts
([`disasm.rs:61-81`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm.rs#L61-L81)):

```rust
/// Tracks the read cursor and byte count for one instruction's decode.
struct Reader<'a, F: FnMut(u16) -> u8> {
    read: &'a mut F,
    cur: u16,
    len: u8,
}

impl<F: FnMut(u16) -> u8> Reader<'_, F> {
    fn u8(&mut self) -> u8 {
        let v = (self.read)(self.cur);
        self.cur = self.cur.wrapping_add(1);
        self.len += 1;
        v
    }

    fn u16(&mut self) -> u16 {
        let hi = self.u8() as u16;
        let lo = self.u8() as u16;
        (hi << 8) | lo
    }
}
```

That is `fetch_u8`/`fetch_u16` from §3.2 with a byte counter bolted on, and the
resemblance is not accidental — `cur` is the disassembler's `pc`, advanced by
exactly the same rule. Two consequences follow. First, `len` is never computed;
it is *accumulated*, which means it cannot disagree with the number of bytes
actually read. There is no table of instruction lengths to keep in sync,
because length is a side effect of decoding. Second, `cur` after an operand
fetch is the equivalent of `self.pc` after an operand fetch, which is what
makes the relative-branch rendering in `render` work without any special
arithmetic.

The result comes back as a small struct
([`disasm.rs:30-38`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm.rs#L30-L38)):

```rust
/// One disassembled instruction.
pub struct Insn {
    /// Total bytes consumed: opcode (+ `$10`/`$11` prefix byte, if any) +
    /// operand bytes. Matches exactly what `MC6809::step` would have read for
    /// the same byte sequence.
    pub len: u8,
    pub mnemonic: &'static str,
    pub operand: String,
}
```

Three fields, and the doc comment on the first one restates the module's
governing rule as a per-instruction contract.

### How `tables.rs` is organized

`tables::base_entry` doesn't decide anything itself; it's a router. Its
`match` splits `op` into ranges and hands each range to one small helper
function, mirroring `exec.rs`'s own family split from week 2 almost
exactly:

- `0x0E`/`0x6E`/`0x7E` (`JMP`'s three addressing forms) are checked *first*
  — spliced in ahead of the generic ranges for the same reason `step`'s own
  `match` orders them first (§3.1's opening list): `0x0E`, `0x6E`, and `0x7E`
  would otherwise fall inside the read-modify-write opcode ranges below and
  get swallowed by the wrong table.
- `0x00..=0x0F`, `0x40..=0x4F`, `0x50..=0x5F`, `0x60..=0x6F`, `0x70..=0x7F`
  route to `rmw_entry`, which indexes one of three 16-entry mnemonic arrays
  (`RMW_MEM`/`RMW_A`/`RMW_B`) by the opcode's low nibble — data, not code,
  for `NEG`/`COM`/`LSR`/.../`CLR` across memory, `A`, and `B`.
- A fixed list of miscellaneous opcodes (`NOP`, `SYNC`, the short-branch
  range, `LEAX`/`LEAY`/`LEAS`/`LEAU`, `PSHS`/`PULS`/`PSHU`/`PULU`, `RTS`,
  `RTI`, `SWI`, …) routes to `base_entry_misc`.
- The remaining four ranges — load/store (`base_entry_load_store`),
  accumulator-`A` ALU ops (`base_entry_accum_a`), accumulator-`B` ALU ops
  (`base_entry_accum_b`), and the wide/subroutine group covering `ADDD`/
  `SUBD`/`CMPX` plus `BSR`/`JSR` (`base_entry_wide_and_subr`) — each get
  their own flat `match` from opcode byte straight to `Entry { mnemonic,
  mode }`. `page10_entry`/`page11_entry` (§3.8) are the same shape again,
  one level down, keyed on the second byte instead of the first.

The "data, not code" claim in the second bullet is worth seeing, since it is the
clearest example in the file of a table that really is a table
([`disasm/tables.rs:14-29`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm/tables.rs#L14-L29)):

```rust
const RMW_MEM: [&str; 16] = [
    "NEG", "???", "???", "COM", "LSR", "???", "ROR", "ASR", "ASL", "ROL", "DEC", "???", "INC",
    "TST", "???", "CLR",
];
const RMW_A: [&str; 16] = [
    "NEGA", "???", "???", "COMA", "LSRA", "???", "RORA", "ASRA", "ASLA", "ROLA", "DECA", "???",
    "INCA", "TSTA", "???", "CLRA",
];
const RMW_B: [&str; 16] = [
    "NEGB", "???", "???", "COMB", "LSRB", "???", "RORB", "ASRB", "ASLB", "ROLB", "DECB", "???",
    "INCB", "TSTB", "???", "CLRB",
];

fn rmw_entry(nibble: u8, table: &[&'static str; 16], mode: Mode) -> Entry {
    e(table[(nibble & 0x0F) as usize], mode)
}
```

Forty-eight opcodes' worth of mnemonics in three array literals, indexed by a
nibble, with the illegal slots spelled `"???"` inline so the array stays
sixteen wide and the index stays the nibble. `rmw_entry` is two lines because
there is nothing left to do. Compare against week 2's `rmw_apply`, which
dispatches the *behaviour* on the same nibble: two files, one numbering, and
the numbering is the interface between them.

Every one of those helpers falls through to `ILLEGAL` on an unmatched
opcode — the same `_ => ILLEGAL` arm repeated at the bottom of each `match`,
which is what guarantees `???` for anything `step()` doesn't decode either,
no matter which of the five helper functions the opcode would have routed
through. `ILLEGAL` itself is a single shared constant
([`disasm.rs:108-120`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm.rs#L108-L120)):

```rust
#[derive(Clone, Copy)]
struct Entry {
    mnemonic: &'static str,
    mode: Mode,
}

const fn e(mnemonic: &'static str, mode: Mode) -> Entry {
    Entry { mnemonic, mode }
}

/// Placeholder for an opcode nothing decodes (see module doc: mirrors the
/// core's silent 2-cycle-default fallthrough).
const ILLEGAL: Entry = e("???", Mode::Inherent);
```

> **Rust corner: `&'static str` and a `const fn` constructor.** `Entry` holds
> a `&'static str` rather than a `String`, which means every mnemonic in the
> disassembler is a pointer into the binary's read-only data, not a heap
> allocation. Disassembling ten thousand instructions to fill a debugger pane
> allocates once per *operand* (which genuinely has to be formatted) and never
> for a mnemonic. The `const fn e(...)` helper is what lets `ILLEGAL` be a
> `const` rather than a function call: `const fn` promises the compiler the
> body can be evaluated at compile time, so `ILLEGAL` is baked into the binary
> as a finished `Entry` and the hundreds of `e("LDA", Indexed)` calls in
> `tables.rs` cost nothing at run time either. The two-character name is
> deliberate — a table with forty entries per screen is more readable when the
> constructor gets out of the way.

### The render path, mode by mode

Once `base_entry`/`page10_entry`/`page11_entry` hand back an `Entry {
mnemonic, mode }`, exactly one function turns `mode` into an operand
string — `render`, in `disasm.rs`
([`disasm.rs:139-173`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm.rs#L139-L173)):

```rust
fn render<F: FnMut(u16) -> u8>(
    r: &mut Reader<F>,
    raw_byte: u8,
    entry: Entry,
) -> (&'static str, String) {
    let operand = match entry.mode {
        Mode::Inherent => {
            if entry.mnemonic == "???" {
                format!("${raw_byte:02X}")
            } else {
                String::new()
            }
        }
        Mode::Imm8 => format!("#${:02X}", r.u8()),
        Mode::Imm16 => format!("#${:04X}", r.u16()),
        Mode::Direct => format!("${:02X}", r.u8()),
        Mode::Extended => format!("${:04X}", r.u16()),
        Mode::Indexed => indexed::decode_indexed(r),
        Mode::Rel8 => {
            let offset = r.u8() as i8 as i16 as u16;
            format!("${:04X}", r.cur.wrapping_add(offset))
        }
        Mode::Rel16 => {
            let offset = r.u16();
            format!("${:04X}", r.cur.wrapping_add(offset))
        }
        Mode::RegPair => {
            let pb = r.u8();
            format!("{},{}", reg_name(pb >> 4), reg_name(pb & 0x0F))
        }
        Mode::StackS => format_stack_mask(r.u8(), true),
        Mode::StackU => format_stack_mask(r.u8(), false),
    };
    (entry.mnemonic, operand)
}
```

Eleven `Mode` variants, eleven arms, no fallback needed because `Mode` is an
`enum` — the match is exhaustive at compile time. Leave one variant unhandled
and this file doesn't build. That is a guarantee `step()`'s `match` on a bare
`u8` opcode can't get for free, since `u8` has 256 values and no enum-style
exhaustiveness check, and it is a small argument for modelling the *rendering*
side as an enum even though the *executing* side has to stay a byte.

Notice too that the operand-byte consumption lives here, in the same expression
that formats the string. `Mode::Imm16` calls `r.u16()`, which advances the
cursor by two and adds two to `len`. There is no separate "how many bytes does
this mode take" table that could disagree; a mode's byte count is whatever its
render arm happens to read. That is the mechanism behind the `Insn::len`
contract, and it is why the illegal-opcode discipline below comes almost for
free.

Three arms are worth a second look because they reuse machinery from earlier
sections instead of inventing their own.

**`Mode::Rel8`/`Mode::Rel16`** resolve a branch offset to an absolute target
the same way `exec.rs`'s branch handling does — fetch the offset, sign-extend
it (the `as i8 as i16 as u16` chain from §3.3's Rust corner, here rendering a
jump target instead of an effective address), and add it to `r.cur`, the
reader's cursor *after* the offset bytes are consumed. That cursor is the
disassembler's equivalent of `self.pc` after the operand fetch. It's the
identical "relative to the next instruction" rule from `n,PCR` (§3.4), just
applied to whole-instruction targets instead of an indexed EA — which is
exactly why the `LBNE $F7AE` in the ROM excerpt below can be hand-verified with
the same arithmetic exercise 3.8 asks for.

**`Mode::RegPair`** is §3.7's nibble pair, rendered through `reg_name`, which
returns `?6`-style placeholders for the reserved codes rather than guessing.
One line of `format!`, two nibble extractions, and the same `pb >> 4` /
`pb & 0x0F` split the executor uses.

**`Mode::StackS`/`Mode::StackU`** call `format_stack_mask`, which walks
the exact same `stack_mask` bits from §3.6 — `CC`, `A`, `B`, `DP`, `X`,
`Y`, `OTHER_STACK_PTR`, `PC`, in that order — and joins whichever are set
into a comma list:

```rust
fn format_stack_mask(mask: u8, is_s_op: bool) -> String {
    let mut regs: Vec<&str> = Vec::with_capacity(8);
    if mask & stack_mask::CC != 0 { regs.push("CC"); }
    // ... A, B, DP, X, Y in the same shape ...
    if mask & stack_mask::OTHER_STACK_PTR != 0 {
        regs.push(if is_s_op { "U" } else { "S" });
    }
    if mask & stack_mask::PC != 0 { regs.push("PC"); }
    regs.join(",")
}
```

`is_s_op` is the disassembler's `to_s`/`from_s` — the exact same
`OTHER_STACK_PTR`-means-a-different-register trick from §3.6, decided once
by which mnemonic (`PSHS`/`PULS` vs `PSHU`/`PULU`) is being rendered
rather than by which stack the CPU would actually touch, since the
disassembler never touches any stack at all.

One deliberate difference from §3.6 is worth flagging, because it looks like a
bug and isn't. `psh` pushes in the order `PC`, other-pointer, `Y`, `X`, `DP`,
`B`, `A`, `CC`; `format_stack_mask` lists in the opposite order, low bit to
high. The function's own doc comment says why: the mask is a bitset, not an
order, so any listing order is a display convention rather than a hardware
fact, and low-to-high is the one that matches how the mask constants are
written down. [`stack_transfer.rs::pshs_all_registers_low_to_high_order`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/disasm/stack_transfer.rs)
pins the convention with mask `$FF` rendering as `CC,A,B,DP,X,Y,U,PC`, and
[`stack_transfer.rs::pulu_partial_mask`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/disasm/stack_transfer.rs) confirms mask `$16` (`A|B|X =
$02|$04|$10`) renders as `A,B,X` through this exact function — the
same `A,B,X` string the ROM excerpt below shows for `PSHS A,B,X` at `$8C37`
(mask also `$16`), since `format_stack_mask` doesn't care which of the
four stack mnemonics called it except for the `OTHER_STACK_PTR` bit,
which this mask doesn't set.

### Two rendering choices worth knowing

Before reading any disassembly listing produced by this module, two decisions
about *presentation* will save you a double-take.

**Constant offsets render in signed decimal, not hex.** The indexed renderer
formats offsets with `format!("{offset},{reg}")` where `offset` is an `i16`, so
postbyte `$88 $10` (8-bit offset `$10`) disassembles as `16,X`, not `$10,X`,
and `$88 $FF` disassembles as `-1,X`.
[`disasm_indexed.rs::offset8_positive`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/disasm_indexed.rs) and
`::offset16_negative` pin this down. Only the extended-indirect address
(`[$XXXX]`) stays hex, since it's an absolute address rather than an offset.
`decode_indexed`'s own doc comment is admirably candid about the status of that
decision: it labels the radix an "indexed offset radix" judgment call and says
it follows the task spec's `LDX 5,Y` example literally. Recording *why* a
formatting choice was made, and admitting when the reason is "the specification
happened to write it that way," is worth more than a rationalization would be —
the next person to touch the renderer knows exactly how much the convention is
load-bearing.

**Illegal opcodes still consume the right number of bytes.** The base-page
table falls back to `ILLEGAL = e("???", Mode::Inherent)`, reading zero operand
bytes, matching the executor's own fallback (`_ => 2` in `step`'s top-level
`match`), which also stops after the opcode byte.
`page10_illegal_second_byte_is_length_2` confirms `$10 $00` disassembles as
`???` at `len == 2` — prefix plus unmatched second byte, exactly what
`exec_page10`'s `_ => 2` arm consumes. Get this wrong on any illegal opcode and
a scrolling disassembly view desyncs the moment it crosses one, and every
instruction after that point on the screen is garbage rendered from misaligned
bytes. The reserved indexed sub-modes get the same discipline: postbytes
`$87`/`$8A`/`$8E` render as `,X???` (visibly flagged) while consuming the
executor's same zero extra bytes
(`reserved_submodes_marked_illegal_but_zero_extra_bytes`).

The suite guards the general property too, not just the special cases.
[`rom_and_scan.rs::forward_scan_never_lands_mid_instruction`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/disasm/rom_and_scan.rs)
walks a hand-assembled sequence mixing one-, two-, and three-byte instructions
and asserts that summing `len` lands exactly on each expected opcode boundary.
That is the invariant a disassembly pane depends on, stated as a test.

### A real rendered example

Everything above is easier to trust once you've seen it run against actual
ROM bytes instead of hand-picked test fixtures. `crates/mc6809/tests/disasm/
rom_and_scan.rs` reads `roms/coco3.rom`, follows the real reset vector, and
disassembles forward from the CoCo 3's actual entry point — the exact code
the real machine executes on power-up, before any BASIC prompt exists. Nine
bytes into that stream, the disassembler crosses a `$10`-prefixed long
branch and an indexed load in the same handful of instructions, both
concepts from this chapter:

```text
8C1B: 1A 50        ORCC #$50
8C1D: 86 0A        LDA  #$0A
8C1F: B7 FF 90     STA  $FF90
8C22: 7F FF DE     CLR  $FFDE
8C25: 7E C0 00     JMP  $C000
8C28: 7F FE ED     CLR  $FEED
8C2B: 7F FF 23     CLR  $FF23
8C2E: 86 CC        LDA  #$CC
8C30: B7 FF 90     STA  $FF90
8C33: 7F FF DE     CLR  $FFDE
8C36: 39           RTS
8C37: 34 16        PSHS A,B,X
8C39: 9E 88        LDX  $88
8C3B: D6 E7        LDB  $E7
8C3D: 10 26 6B 6D  LBNE $F7AE
8C41: E6 61        LDB  1,S
```

Every line here is a `disassemble()` call the test asserts on directly — the
mnemonics, operands, and byte lengths are pulled straight from
`rom_reset_entry_point_disassembles_as_hand_decoded`, which fails loudly if
the real ROM's opening bytes ever stop matching (a canary for a bad ROM file
as much as a disassembler test). The test even asserts the reset vector itself
reads `$8C1B` before decoding anything, so a wrong ROM image fails with a clear
message rather than a wall of mismatched mnemonics.

There is period texture in these sixteen lines if you know where to look. The
very first instruction, `ORCC #$50`, sets bits `$40` and `$10` of the condition
code register — the `F` and `I` masks from week 2's `cc` module — which is a
power-on routine's way of saying that nothing may interrupt the next few
instructions. Three
instructions later, `STA $FF90` writes into the `$FF90–$FF9F` range that
Chapter 1's I/O map labels "GIME control: INIT0/1, IRQs, timer, video" and
assigns to weeks 5 and 8. This is the CoCo 3 configuring itself in the first
microseconds after reset, and by week 8 you'll be able to read every line of
it.

Two lines are worth tracing by hand right now with what you know today.

**`8C3D: 10 26 6B 6D  LBNE $F7AE`.** `$10` is the page-10 prefix (§3.8); `$26`
is `BNE`'s low nibble (`6`) promoted to its long form, `LBNE`; the 16-bit
offset `$6B6D` is added to the PC *after* the whole 4-byte instruction —
`$8C41 + $6B6D = $F7AE` (wrapping `u16` arithmetic, same rule as every other
relative branch). `decode_base` reads the `$10`, hands `$26` to `page10_entry`,
which maps it through `LONG_BRANCH[6]`. That array index is worth noticing: the
branch mnemonic comes out of a sixteen-entry table indexed by the opcode's low
nibble, exactly like the RMW tables above, and exactly like week 2's
`branch_taken` dispatches the *condition* on the same nibble.

**`8C41: E6 61  LDB 1,S`.** `$E6` is `LDB` indexed; postbyte `$61` is
`0110_0001` — bit 7 *clear*, so this is the 5-bit offset form from §3.2, not
the full `1 rr i mmmm` layout: `rr = 11` (S), `n = 00001 = 1`. `LDB 1,S` reads
the byte just above whatever `PSHS A,B,X` (three instructions earlier, at
`$8C37`) left on the stack — a real, ROM-verified instance of the cheapest
indexed form doing exactly the job it exists for: reaching one byte past the
stack pointer without the overhead of an 8-bit-offset postbyte. Two bytes,
five cycles, and the offset rides inside the postbyte.

Work out which register it is reaching, using §3.6. `PSHS A,B,X` pushed with
mask `$16`, and `psh`'s fixed order pushes `X` first (two bytes, high then low
by address), then `B`, then `A` last at the lowest address. So `S` points at
the saved `A`, and `1,S` is the saved `B`. That is four bytes on the stack and
one instruction to reach past the first of them — the kind of tight,
offset-counting code that stack-relative addressing exists to make cheap, and
the kind that breaks the instant someone inserts an extra `PSHS` in between.
This is the concrete version of §3.7's argument for `TFR CC,B` over `PSHS CC`.

Frame this module correctly for where the course is going: `Insn { len,
mnemonic, operand }` — bytes consumed, name, rendered operand — is exactly
the payload a debugger's disassembly pane needs, and week 16 builds that
pane on top of this function. Every design decision here (never speculate
past what an instruction needs, match the executor's byte count even for
garbage, render offsets the way a human reads them) is a debugger
requirement in disguise.

---

## 3.10 The complete postbyte reference

Everything above walked specific bytes forward, from encoding to meaning.
This section runs the other direction — a reference for building a postbyte
from an assembly-language addressing form, the way you'd do it by hand
before an assembler existed. It is also, frankly, the section to photocopy and
keep next to the keyboard for the rest of the CPU work. Two formulas cover
every legal postbyte:

**5-bit form** (bit 7 clear): `byte = (rr << 5) | n5`, where `rr` is 2 bits
(`00`=X, `01`=Y, `10`=U, `11`=S) and `n5` is the 5-bit field holding a signed
offset in range **-16 to +15** (two's complement over 5 bits: `n5 = offset`
for `0 <= offset <= 15`, `n5 = offset + 32` for `-16 <= offset < 0`).

**Full form** (bit 7 set): `byte = $80 | (rr << 5) | (i << 4) | mmmm`, where
`i` is `1` for indirect and `mmmm` is the 4-bit sub-mode from the §3.3
table. The `rr` field means the same thing in both forms — this is exactly
`postbyte::REG_SHIFT` from §3.2 applied identically regardless of which
layout the rest of the byte uses.

**Worked recipe: `LEAY 3,S`.** No indirect (plain constant offset small
enough for the 5-bit form), register S (`rr = 11`), offset `+3`
(`n5 = 3`). `byte = (0b11 << 5) | 3 = 0b1100011 = $63`. Double-check against
the table below by decoding it back: `$63 = 0110_0011`, bit 7 clear, `rr =
pb >> 5 = 0b011` → masked to `11` = S, `n = pb & 0x1F = 0b00011 = 3`, sign
bit (`0x10`) clear ⇒ `+3`. Round-trips cleanly.

**Worked recipe: `LEAY [10,U]`.** This one *must* use the full form, because
it's indirect, and the 5-bit form (§3.2) has no indirect bit at all —
regardless of whether `10` would otherwise fit the 5-bit range. Full form:
`rr = 10` (U), `i = 1`, `mmmm = 0b1000` (`n,R`, 8-bit). `byte = $80 | (0b10
<< 5) | (1 << 4) | 0b1000 = $80 | $40 | $10 | $08 = $D8`. The offset itself,
`$0A` (10 decimal), is a separate operand byte that follows the postbyte —
the postbyte only ever encodes *which* sub-mode and *which* register, never
the offset's value once it's wider than 5 bits.

That last sentence is the encoding rule most worth internalizing, because it
explains why the table below has exactly sixteen rows regardless of how many
addressing forms exist. The postbyte is a *selector*. Everything that varies
continuously — offsets, addresses — lives in the bytes after it.

The full sixteen-row table, for register **X** (`rr = 00`). For Y, U, or S,
add `$20`, `$40`, or `$60` respectively to *both* columns — the register
field is the same three bits regardless of sub-mode, so the addend is
constant across every row (verified above: `,Y` is `$A4`, `,U+` is `$C0`,
`,S` is `$E4` — all §3.3's plain-`,R` byte `$84` or the auto-inc byte `$80`
plus exactly the row's own `rr` addend):

| `mmmm` | Assembly (`R` = X here) | Legal indirect on real silicon? | X direct | X indirect |
|---|---|---|---|---|
| `0000` | `,R+` | **No** | `$80` | `$90`† |
| `0001` | `,R++` | Yes | `$81` | `$91` |
| `0010` | `,-R` | **No** | `$82` | `$92`† |
| `0011` | `,--R` | Yes | `$83` | `$93` |
| `0100` | `,R` | Yes | `$84` | `$94` |
| `0101` | `B,R` | Yes | `$85` | `$95` |
| `0110` | `A,R` | Yes | `$86` | `$96` |
| `0111` | reserved | — | `$87` | `$97` |
| `1000` | `n,R` (8-bit) | Yes | `$88` | `$98` |
| `1001` | `n,R` (16-bit) | Yes | `$89` | `$99` |
| `1010` | reserved | — | `$8A` | `$9A` |
| `1011` | `D,R` | Yes | `$8B` | `$9B` |
| `1100` | `n,PCR` (8-bit) | Yes | `$8C` | `$9C` |
| `1101` | `n,PCR` (16-bit) | Yes | `$8D` | `$9D` |
| `1110` | reserved | — | `$8E` | `$9E` |
| `1111` | `[n]` extended | *is* indirect | `$8F`‡ | `$9F` |

† The datasheet calls `,R+`/`,-R` combined with the indirect bit undefined;
this codebase's decoder doesn't special-case it (§3.3), so `$90`/`$92`
still execute — just not as anything a real assembler would ever emit.
‡ `$8F` isn't a documented assembler form at all: mode `1111` *means*
"extended indirect," so the only legal encoding sets the indirect bit
(`$9F`). `$8F` is what the code does if you hand-construct it anyway —
`ea_indexed_submode`'s `0b1111` arm still runs (`fetch_u16`, no wrap), just
without the pointer dereference `[...]` implies.

Every hex byte in this table is either lifted directly from a test you've
already read (`$80`-`$8D`, `$94`, `$98`, `$9F`, `$A4`, `$C0`, `$E4`) or
computed from the same formula those bytes confirm (`$8E`-`$8F`, `$90`-
`$93`, `$95`-`$97`, `$99`-`$9E`) — nothing here is asserted without a
verified anchor point. The distinction matters more than it might seem: a
reference table in a book is exactly the kind of artifact that acquires errors
by being retyped, and the way to keep one honest is to be able to say, row by
row, where each number came from.

---

## 3.11 Reading assignment

The order below matters more this week than most, because the postbyte masks
are load-bearing for four different files and reading any of those files first
means reading bit patterns you don't yet have names for.

In this order: **[`lib.rs:76-105`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L76-L105)** (`postbyte`/`stack_mask`, load-bearing for
everything below); **[`addressing.rs:30-192`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/addressing.rs#L30-L192)** (`ea_indexed` through
`ea_indexed_submode` — read it twice, once for shape, once sub-mode by
sub-mode with the §3.3 table open); **[`stack.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/stack.rs)** (`psh`/`pul`, under 80
lines); **[`regs.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/regs.rs)** (`reg_read`/`reg_write`/`tfr_value`); **[`exec.rs:58-194`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L58-L194)**
(`exec_page10`/`exec_page11`); then **[`disasm.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm.rs)**, **[`disasm/tables.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm/tables.rs)**,
**[`disasm/indexed.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm/indexed.rs)** in that order — data first, rendering logic last;
finally **[`tests/disasm/rom_and_scan.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/disasm/rom_and_scan.rs)**, for the disassembler pointed at
real, ungenerated ROM bytes instead of hand-picked fixtures.

Then run, and read while they run:

```
cargo test -p mc6809 --test indexed
cargo test -p mc6809 --test stack
cargo test -p mc6809 --test disasm_indexed
cargo test -p mc6809 --test disasm rom_reset_entry_point
```

The last of those needs `roms/coco3.rom`, which is git-ignored and local-only;
the other three need nothing but the repository. If you are working from a
fresh clone or a git worktree, expect the ROM test to fail loudly rather than
skip, and read its error message — it is written to tell you exactly which file
it wanted.

---

## 3.12 Exercises

**3.1 — Hand-decode three postbytes (recall).** Without running anything,
decode indexed postbytes `$8B`, `$F4`, and `$9F` by hand: register field,
indirect bit, sub-mode, resulting assembly syntax, and total extra cycle
cost (sub-mode extra, plus 3 more if the indirect bit is set). Then check
every part of your answer against [`crates/mc6809/src/disasm/indexed.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/disasm/indexed.rs) —
run the byte through `disassemble` if you want the operand string, and trace
`ea_indexed_submode` by hand for the cycle math. Get the bit arithmetic
wrong at least once before you get it right; that's the point.

**3.2 — `LEAX ,--Y`, both halves (build).** Write a test in the style of
[`crates/mc6809/tests/indexed.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/indexed.rs) for `LEAX ,--Y` (opcode `$31`) that asserts
*two* things: the computed effective address (loaded into `X`) *and* the
post-instruction value of `Y`. Pick a starting `Y` where forgetting the
decrement, or decrementing by 1 instead of 2, gives a different, plausible
wrong answer — otherwise a broken implementation could pass by accident.

**3.3 — Why the 5-bit form can't be indirect (read).** From the bit layout
alone (`0 rr nnnnn` vs `1 rr i mmmm`), explain in two or three sentences why
this is a hardware fact about the byte's bit budget, not a design choice —
and why "just add an indirect flag to the 5-bit form anyway" isn't available
even if you wanted it.

**3.4 — Sabotage the indirect fetch (sabotage).** In `ea_indexed_full`,
change `extra += INDIRECT_CYCLES` to bill only 2 cycles instead of 3.
Predict, then run, which tests in `indexed.rs` fail and by how much
(`indirect_no_offset`, `indirect_8bit_offset`, `extended_indirect` each bake
the +3 into a total-cycle assertion). Revert, then change the `,R++`
sub-mode's extra-cycle literal from `3` to `2`: which single test catches
that, and why doesn't it disturb the `,R+` test right next to it?

**3.5 — Break the push order (sabotage).** In `psh`, swap the `A` and `B`
push lines. Predict which test in `stack.rs` fails first, and whether it
fails on the push half or the pull half of the round trip —
`pshs_pulls_restore_all_registers` checks final register values after a
round trip, so consider whether a swap applied symmetrically to *both*
`psh` and `pul` would even be detectable by that test, versus swapping
`psh` alone.

**3.6 — TFR size mismatch (recall).** Without looking at `tfr_value`, write
down what `TFR D,DP` does (16→8) and what `TFR DP,Y` does (8→16). Check both
against `tfr_value` and, for the truncation direction, `reg_write`. Which
rule lives in which function — and could `EXG` reuse both without
duplicating either?

**3.7 — `STD ,X++` (build).** Write the test §3.4 promises: `STD` indexed
(`$ED`) with postbyte `$81` (`,X++`). Assert three things — both stored
bytes (big-endian, at the *old* `X`) and `X`'s post-instruction value
(`old + 2`) — and the cycle count. Compute the cycle count by hand first (16-bit
store base + auto-increment-by-2 extra) and then check your test against it.

**3.8 — Trace the ROM's `LBNE` by hand (recall).** Using only the byte
sequence in §3.9's rendered example, compute the target address of `8C3D:
10 26 6B 6D LBNE $F7AE` from scratch: identify the page prefix, the branch
condition nibble, the 16-bit offset, and the address it's relative to
(which is *not* `$8C3D`). Show your arithmetic and confirm it lands on
`$F7AE`.

**3.9 — Sabotage `n,PCR` (sabotage, verified).** In `ea_indexed_submode`'s
`0b1100` arm (8-bit `n,PCR`), capture `self.pc` into a local *before*
calling `self.fetch_u8(bus)` instead of after, and add that captured value
to the offset instead of the post-fetch `self.pc`. Predict what breaks —
which specific byte address does the EA land on now, and why is it exactly
one less than correct? Then run `cargo test -p mc6809 --test indexed
pc_relative` and confirm: this change fails `pc_relative_8bit` while
leaving `pc_relative_16bit` completely untouched (the two arms don't share
code, so sabotaging one has zero blast radius on the other — the same
independence exercise 3.4 explores for the auto inc/dec arms). Revert
before moving on.

**3.10 — Hand-encode from §3.10's table (build/recall).** Using only the
two formulas in §3.10 (no peeking at `disasm/indexed.rs`), compute the
postbyte for `LDA [7,Y]` (7 fits the 5-bit signed range, but *indirect*
forms must use the 8-bit-offset full form, never the 5-bit form — why, in
one sentence, citing §3.2?) and for `LDA ,S--`. That second one is a trap:
re-read §3.3's sixteen-arm table before answering — does `,S--`
(*post*-decrement) exist on the 6809 at all, or have you conflated it with
`,--S` (*pre*-decrement, which does)? The four auto forms cover exactly
post-increment-by-1/2 and pre-decrement-by-1/2 — no pre-increment, no
post-decrement, on any register. Write the one valid postbyte in hex, then
verify by disassembling it; explain in a sentence why the other one isn't
encodable at all rather than just being illegal.

**3.11 — Extend the `TFR`/`EXG` ROM window (read).** §3.7's `TFR CC,B`/
`TFR B,CC` example didn't come from the reset-vector trace in §3.9, so its
surrounding addresses weren't independently confirmed by an existing test
the way `$8C1B`-`$8C41` is. Write a small program against
`mc6809::disasm::disassemble` (or extend an existing test) that reads
`roms/coco3.rom`, disassembles a 20-instruction window starting at `$82E8`,
and asserts none of them come out as `???` — the same static plausibility
check this chapter used by hand, made repeatable. (Local machine only:
needs `roms/coco3.rom`, per this repo's `CLAUDE.md`.)

---

## What's next

Week 4 finishes the CPU: `nmi`/`irq`/`firq` and the interrupt frames they
stack, using the exact `psh`/`pul` you read this week with fixed masks
(`0xFF` full frame, `PC_CC_MASK` for FIRQ). The twelve-byte frame that
`pshs_all_registers_cost_17` measured is the same twelve bytes an IRQ pushes,
and the `nmi_armed` field that kept surfacing in this chapter's margins finally
gets the section it deserves. You'll meet `CWAI`/`SYNC` as CPU *states* rather
than instructions — the `State` enum from Chapter 1, finally used — and, since
the 6809 has no per-instruction conformance suite like the 6502/Z80, the
three-legged validation strategy this codebase leans on instead: trace-diffing
against a reference emulator, a self-checking exerciser ROM, and the
hand-written corner tests you've read all month.

After that the CPU is done, and week 5 opens the machine.
