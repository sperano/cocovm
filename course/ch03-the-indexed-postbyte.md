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

## 3.1 Why 200 lines get a whole week

DESIGN.md ranks the CPU's hard parts "in order of pain," and indexed
addressing is first on the list, ahead of interrupts:

> 1. **Indexed addressing** — one postbyte encodes ~a dozen sub-modes:
>    constant offsets (5/8/16-bit), accumulator offsets (A/B/D), auto
>    inc/dec by 1 or 2, PC-relative (8/16), extended-indirect, and indirect
>    variants of most. Build one `fn ea_indexed(&mut self, bus) -> (u16 addr,
>    u32 extra_cycles)` and get it bulletproof — a large fraction of all
>    instructions route through it.

Look at how many opcodes cash that check in `crates/mc6809/src/exec.rs`
alone: `LEAX`/`LEAY`/`LEAS`/`LEAU`, indexed `LDA`/`STA`/`LDB`/`STB`/`LDD`/
`STD`, every indexed 8-bit ALU op (`ADD`/`ADC`/`SUB`/`SBC`/`CMP` for both
accumulators), indexed `AND`/`OR`/`EOR`/`BIT`, indexed `JMP`/`JSR`, and — via
the `$10`/`$11` prefix pages of §3.8 — indexed `CMPD`/`CMPY`/`LDY`/`STY`/
`LDS`/`STS`/`CMPU`/`CMPS`. Every one of those opcodes calls the same
function, `ea_indexed`, to turn a postbyte into an address. Get that one
function wrong and you don't break one instruction — you break a fraction of
the ISA at once, in ways that only show up as wrong pixels several
instructions later. That's why it earns a whole week of undivided attention.

---

## 3.2 The decode tree: one bit decides everything

Here are the field masks the whole chapter hangs off, from
`crates/mc6809/src/lib.rs:76-90`:

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

> **Rust corner: a private module as a bitflag namespace.** `postbyte` isn't
> a `struct` or an `enum` — it's a bare `mod` holding `pub const` bytes, not
> `pub` itself (visible only inside this crate). No derive, no trait, just
> named constants grouped under one path (`postbyte::INDIRECT`) — a name
> instead of a bare `0x10`, without designing a `bitflags`-style type for a
> mask consumed in exactly one file. `regsel` (§3.7) and `stack_mask` (§3.6)
> use the same trick. When bits need to compose (`OR`ed together, tested
> with `&`), plain `u8` constants serve better than an enum would — enums
> don't overlap bit patterns for free.

Every indexed postbyte starts with one branch, in `ea_indexed`
(`crates/mc6809/src/addressing.rs:58-67`):

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
held in reserve. `ea_indexed_offset5` decodes exactly that
(`addressing.rs:71-81`):

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

That's a hardware fact, not an emulator choice: **this form cannot be
indirect because there is no spare bit to hold the indirect flag.** The
`1 rr i mmmm` layout (bit 7 set) exists *because* it sacrifices two offset
bits — down to a 4-bit sub-mode selector — to make room for the `i` bit and
a whole family of other addressing tricks. The two forms aren't "5-bit
offset, indirect or not"; they're two different postbyte grammars sharing
one byte's bit budget, and only one affords an indirect flag at all.

The 5-bit form costs 1 extra cycle, always — cheapest of every indexed
variant, which is why hand-written assembly reaches for `,X` with a small
offset whenever it can: shortest encoding and fastest execution both.

---

## 3.3 The full form: register, indirect bit, then sixteen sub-modes

When bit 7 *is* set, `ea_indexed_full` takes over
(`addressing.rs:85-96`):

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
Indirection isn't its own sub-mode; it's a post-processing step layered
uniformly on top of any sub-mode's result. That uniformity glosses over one
datasheet nuance: real hardware documents only some sub-modes as
indirectable, and calls `,R+`/`,-R` (single-step auto inc/dec) undefined in
combination with indirect. The code doesn't special-case that restriction —
set the indirect bit on a `,R+` postbyte and you get a second fetch anyway.
Worth knowing if you ever chase a compatibility bug in this exact corner.

Now the sub-mode table itself, `ea_indexed_submode`
(`addressing.rs:100-171`), quoted whole because every case matters and you
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

The auto inc/dec arms (`0b0000`-`0b0011`) get a second, closer look in §3.5,
where the ordering inside each one — compute, write back, return — is the
entire subject.

`sel` is the raw `rr` field, still shifted but not yet masked; `index_reg`
masks it down to 2 bits itself (`sel & 0b11`), which is why both this
function and `ea_indexed_offset5` can hand it the same unmasked value from
different shift origins. Every arm is one of five shapes: mutate-then-return
(the auto inc/dec arms, §3.5), plain register, signed-offset addition
(accumulator or fetched), PC-relative addition, or the reserved fallback.
The full table, matching the real source arm for arm (extra cycles are
*before* the indirect bit's own +3, which stacks on top of any row):

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

Two entries deserve a second look. `n,PCR` measures the offset from `self.pc`
*after* the operand bytes have already been fetched — "the address of the
*next* instruction," exactly as the 6809 datasheet defines PC-relative. And
the reserved sub-modes (`0111`, `1010`, `1110`) aren't a dispatch bug: they're
the emulator's explicit policy for postbyte patterns the real chip leaves
undefined — fall back to a plain register read, charge no extra cycles, move
on. The disassembler makes the same call, rendered visibly, in §3.9.

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

---

## 3.4 Seven postbytes, end to end

You've hand-assembled indexed operands before; now trace the reverse
direction on real postbyte values, cross-checked against the executor tests
in `crates/mc6809/tests/indexed.rs`.

**`$05` — `LDA 5,X`.** Binary `0000_0101`. Bit 7 is clear, so this is the
5-bit form: `rr = 00` (X), `n = 00101 = 5`, sign bit (`0x10`) clear ⇒ offset
`+5`. Decoder path: `ea_indexed` → `ea_indexed_offset5`. EA = `X + 5`. Extra
cycles: 1. With `X = $2000`, EA = `$2005` — exactly
`indexed.rs::offset5_positive`, which loads `$42` from `$2005` and asserts
`cycles == 5` (base 4 for `LDA` indexed + 1).

**`$80` — `LDA ,X+`.** Binary `1000_0000`. Bit 7 set: `sel = pb >> 5 = 0b100`
→ masked to X; indirect clear; `mode = pb & 0x0F = 0b0000`. Decoder path:
`ea_indexed_full` → `ea_indexed_submode` case `0b0000`. With `X = $2000`,
this reads X's *current* value as the EA and only then writes `X+1` back —
the mechanics §3.5 examines in detail. Extra cycles: 2. Total: 4 + 2 = 6,
matching `indexed.rs::auto_increment_by_one`.

**`$98` — `LDA [16,X]`.** Binary `1001_1000`. `sel = pb >> 5 = 0b100` → X;
indirect bit `0x10` — is it set? `0x98 = 0b1001_1000`; bit 4 (value `0x10`)
is `1`. So indirect is *on*. `mode = pb & 0x0F = 0b1000`, the 8-bit-offset
sub-mode: fetch one more byte (`$10` = 16 decimal), EA-before-indirection =
`X + 16`, extra so far = 1. Then `ea_indexed_full` sees the indirect bit,
does a second bus read — `bus.read_u16(ea)` — to fetch the *real* pointer
from that address, and adds `INDIRECT_CYCLES = 3`. Total extra: 1 + 3 = 4.
Grand total: 4 (base) + 4 = 8, matching `indexed.rs::indirect_8bit_offset`
exactly (it asserts `cycles == 8` with the comment `// 4 + (1 + 3)`).

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
disassembler side *is* directly tested: `disasm_indexed.rs::indirect_16bit_offset`
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

**`$8C` — `LDA n,PCR` (8-bit).** The syllabus singles this one out, and it
deserves the full trace rather than a summary, because the "relative to the
*next* instruction" rule is the one detail every 6809 newcomer gets wrong
once. `indexed.rs::pc_relative_8bit` loads the program at `$1000`:

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
The 16-bit form (`$8D`, `indexed.rs::pc_relative_16bit`) is identical in
shape — `pc` lands at `$1004` after the two-byte offset fetch, `$1004 +
$0100 = $1104` — just with the wider fetch and a heavier bill: extra cycles
5, the single most expensive non-indirect sub-mode in the table.

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
shape `indexed.rs::accumulator_d_offset` asserts for `D,X`.

---

## 3.5 Auto inc/dec: whose turn is it, old value or new?

This is the exact question the syllabus flags, and all four auto inc/dec
arms of §3.3 answer it precisely, by-1 and by-2 forms side by side:

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

The by-1 and by-2 forms share their ordering exactly — `0b0000`/`0b0001`
are both "old value, then increment"; `0b0010`/`0b0011` are both "decrement,
then new value" — they differ only in the constant passed to
`wrapping_add`/`wrapping_sub` and in the extra-cycle count (2 vs 3, one more
cycle for the second byte of the step). That's not an accident of encoding:
`,R++`/`,--R` exist specifically for 16-bit registers (`X`/`Y`/`U`/`S`) and
16-bit data (`D`, via `LDD`/`STD` — see `$81` in §3.4), where you want the
pointer to land past a whole *word*, not into the middle of one. The by-1
forms are for byte-at-a-time buffers.

This is exactly the property `LDX ,--Y` (exercise 3.2) and `STD ,X++`
(exercise 3.7) ask you to pin down with a test: you can't assert the right
answer for either mode without knowing which value shows up as the EA *and*
which value ends up in the register afterward, and they don't always match
your intuition about "pointer arithmetic first vs. use first" if you're used
to C's `*p++`/`*--p` — the 6809 rule is symmetric with C, but you should
verify it here rather than assume it.

---

## 3.6 PSH/PUL: masks, order, and the "other" stack pointer

The register-mask postbyte for `PSHS`/`PULS`/`PSHU`/`PULU` gets its own
bitflag module, `crates/mc6809/src/lib.rs:95-105`:

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

Each set bit selects one register (pair) to transfer. `crates/mc6809/src/
stack.rs` implements the push and pull in one function each, both shared by
every explicit stack opcode *and* by the interrupt-frame code you'll read in
full next week:

```rust
pub(crate) fn psh(&mut self, bus: &mut impl Bus, mask: u8, to_s: bool) -> u32 {
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

Three things to notice, all load-bearing:

- **Push order is fixed, not mask order.** The `if` chain always tests `PC`,
  other-stack-pointer, `Y`, `X`, `DP`, `B`, `A`, `CC` — in that order,
  regardless of which bits are set. PC goes on first (deepest, highest
  address); CC goes on last (shallowest, lowest address). That's why `RTI`
  (next week) always finds CC one byte from the stack pointer, whatever
  registers a given `PSHS` saved.
- **16-bit values push high byte first.** Each pair calls `push8` for the low
  byte, then the high byte — but since every `push8` *pre-decrements* `sp`,
  the byte pushed second lands at the lower address. Net effect: high byte
  at the lower address, big-endian in memory, matching `Bus::write_u16` from
  Chapter 1. `stack.rs::pshs_16bit_is_big_endian` pins this down.
- **`OTHER_STACK_PTR` (`0x40`) is context-dependent.** The same bit means "U"
  for `PSHS`/`PULS` and "S" for `PSHU`/`PULU` — the entire reason for the
  `other` local, computed once from `to_s`. It's the one mask bit whose
  *meaning*, not just value, depends on which opcode you used.

`stack.rs::pshs_can_push_and_pull_u_via_bit6` exercises that third bullet
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

`pul` is the mirror image — same order, reversed, pulling low-address-first
so CC comes off first and PC last, exactly as the module doc states: "push
order is PC, U/S, Y, X, DP, B, A, CC (highest address first), so CC ends up
on top." Cost for both directions is `PUSH_PULL_BASE_CYCLES` (`5`,
`lib.rs:108`) plus one cycle per byte transferred — pushing all eight items
costs `5 + 12` (PC, the "other" pointer, X, and Y are 2 bytes each; DP, B, A,
CC are 1 byte each), exactly what `stack.rs::pshs_all_registers_cost_17`
asserts.

> **Rust corner: an `FnMut` closure capturing a generic `&mut` parameter.**
> `push8` captures `bus` (type `&mut impl Bus`) by reference, but takes `sp`
> and `bytes` as explicit parameters rather than also capturing those
> locals. `sp` is reassigned across eight `if` blocks; a closure holding
> `&mut sp` for its whole lifetime would block any other use of `sp` in
> between — the exact aliasing Rust's capture rules exist to reject. Passing
> `&mut sp` fresh each call sidesteps it: the closure borrows `sp` only for
> one call, then gives it back. Same partition-by-borrow instinct as Chapter
> 1's `Machine`/`SystemBus` split, at function scale.

`take_interrupt` (next week's reading) reuses `psh` directly —
`self.psh(bus, 0xFF, true)` for a full NMI/IRQ/SWI frame, `self.psh(bus,
PC_CC_MASK, true)` for FIRQ's PC+CC-only frame, where `PC_CC_MASK =
stack_mask::PC | stack_mask::CC` (`lib.rs:111`). Every interrupt frame is
the exact same `psh` you just read, called with a different mask.

---

## 3.7 TFR/EXG: nibble codes and the size-mismatch rules

`TFR`/`EXG` share one postbyte shape, a nibble pair, with its own selector
codes (`crates/mc6809/src/lib.rs:63-74`):

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

Codes `0x0`-`0x5` name the six 16-bit registers; `0x8`-`0xB` name the four
8-bit ones (`0x6`, `0x7`, `0xC`-`0xF` are reserved — the disassembler renders
those as `?6`, `?7`, etc., rather than guessing; see §3.9). `TFR` is
`opcode $1F`, postbyte `hi:lo` = source:dest; `EXG` is `$1E`, same postbyte
shape but reads both, then writes both swapped. Both cost 6 cycles
regardless of size, per `exec.rs`'s `exec_control_transfer`:

```rust
0x1F => { let pb = self.fetch_u8(bus); let v = self.tfr_value(pb >> 4, pb & 0x0F); self.reg_write(pb & 0x0F, v); 6 }
```

The interesting part is what happens when source and destination sizes
don't match, which the datasheet documents but which is easy to get subtly
wrong. `crates/mc6809/src/regs.rs`:

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

Three documented rules, one function:

| Transfer | Rule | Test |
|---|---|---|
| 16-bit → 8-bit | destination gets the low byte; `reg_write`'s `value as u8` truncates | `regs.rs::tfr_16_to_8_takes_lsb` (`TFR X,A` with `X=$1234` ⇒ `A=$34`) |
| `A`/`B` → 16-bit | high byte forced to `$FF` | `stack.rs::tfr_accumulator_to_16_sets_ff_high` (`TFR A,X` with `A=$7F` ⇒ `X=$FF7F`) |
| `CC`/`DP` → 16-bit | both bytes duplicate the source byte | `stack.rs::tfr_cc_to_16_duplicates_byte` (`TFR CC,X` with `CC=$42` ⇒ `X=$4242`) |

Notice the 16→8 truncation isn't handled inside `tfr_value` at all — its
`_ => sv` arm returns the full 16-bit source unchanged, and it's the generic
`reg_write` (week 2 territory, `regs.rs:32-46`) that does `self.a = value as
u8` for an 8-bit destination. The size-mismatch *rule* lives in `tfr_value`;
the *mechanism* lives in `reg_write`. `EXG` doesn't call `tfr_value` at all —
it swaps via two independent `reg_read`/`reg_write` calls, so an 8↔16 `EXG`
gets the same truncate/pad behavior for free, just by routing through the
same `reg_write`.

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
copy each way. That's the idiom: stash the condition codes computed by
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
someone else expects to find at a fixed distance from `S`. You'll meet the
stack-based version of this exact save/restore pattern again in week 4,
wrapped around every interrupt.

---

## 3.8 Page prefixes: the opcode isn't always one byte

`$10` and `$11` aren't "modifier" bytes layered on top of another opcode —
they're the *first byte* of a two-byte opcode, and the dispatcher treats
them that way structurally, not just semantically. From `exec.rs`'s
top-level `match`:

```rust
// $10/$11 prefix pages: long conditional branches, the 16-bit ops
// targeting Y/D/S/U, and SWI2/SWI3.
0x10 => self.exec_page10(bus),
0x11 => self.exec_page11(bus),
```

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

`$11` is the same shape, smaller: just `CMPU`/`CMPS` and `SWI3`. Two things
worth internalizing:

- **The base-page `match` never sees `op2`.** It fetches exactly one byte
  (`$10` or `$11`), recognizes it as a page selector, and hands the rest of
  the instruction to the sub-dispatcher. Page 10 and page 11 are two
  independent copies of "byte in, cycles out" — a prefixed instruction's
  total byte count and cycle cost are the prefix byte plus whatever the
  sub-dispatcher consumes.
- **Prefixed instructions cost one cycle more than their unprefixed
  equivalents** — fetching the extra opcode byte is itself a bus cycle.
  Base-page `LDX` immediate (`0x8E`) is 3 cycles; page-10 `LDY` immediate
  (`$10 $8E`) is 4. Base-page `CMPX` immediate (`0x8C`) is 4; page-10 `CMPD`
  immediate (`$10 $83`) is 5. One extra cycle every time — the prefix byte's
  own cost, paid once regardless of which second-byte opcode follows.

The disassembler mirrors this exact two-layer structure — `disasm.rs`'s
`decode_base` intercepts `$10`/`$11` before consulting the base-page table,
then hands off to a page-specific table function:

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
second, independent lookup keyed on the second byte.

---

## 3.9 The disassembler: table-driven, and never allowed to lie about length

`crates/mc6809/src/disasm.rs` is a pure function over a byte reader — no CPU
struct, no side effects, just `fn disassemble(read: &mut impl FnMut(u16) ->
u8, pc: u16) -> Insn`. Its module doc states its one governing rule plainly:

> `MC6809::step` is the *executing* dispatcher (it boots real BASIC) and is
> therefore the authority for which opcodes exist, which addressing mode
> each uses, and how many bytes each consumes — including which opcodes are
> illegal/undecoded. This module mirrors that opcode map byte-for-byte.

The split that makes this maintainable: *which mnemonic and mode opcode X
has* is data (`tables::base_entry`/`page10_entry`/`page11_entry`, plus small
per-nibble arrays for the RMW and branch-condition groups); *how to render
mode Y into an operand string* is one shared function per mode (`render`,
and `indexed::decode_indexed` for indexed). `decode_indexed` imports the
same `postbyte` masks from §3.2 and switches on the same `mmmm` values as
`ea_indexed_submode` — not a second, hand-written copy that could drift:

```rust
pub(super) fn decode_indexed<F: FnMut(u16) -> u8>(r: &mut Reader<F>) -> String {
    let pb = r.u8();
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

Every one of those helpers falls through to `ILLEGAL` on an unmatched
opcode — the same `_ => ILLEGAL` arm repeated at the bottom of each `match`,
which is what guarantees `???` for anything `step()` doesn't decode either,
no matter which of the five helper functions the opcode would have routed
through.

### The render path, mode by mode

Once `base_entry`/`page10_entry`/`page11_entry` hand back an `Entry {
mnemonic, mode }`, exactly one function turns `mode` into an operand
string — `render`, in `disasm.rs`:

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
`enum` — the match is exhaustive at compile time (leave one variant
unhandled and this file doesn't build; a Rust guarantee `step()`'s `match`
on a bare `u8` opcode can't get for free, since `u8` has 256 values and no
enum-style exhaustiveness check). Two arms are worth a second look because
they reuse machinery from earlier sections instead of inventing their own:

- **`Mode::Rel8`/`Mode::Rel16`** resolve a branch offset to an absolute
  target the same way `exec.rs`'s branch handling does — fetch the offset,
  sign-extend it (the `as i8 as i16 as u16` chain from §3.3's Rust corner,
  here rendering a jump target instead of an effective address), and add it
  to `r.cur` — the reader's cursor *after* the offset bytes are consumed,
  the disassembler's equivalent of `self.pc` after the operand fetch. It's
  the identical "relative to the next instruction" rule from `n,PCR`
  (§3.4), just applied to whole-instruction targets instead of an indexed
  EA — which is exactly why `LBNE $F7AE` in §3.9's ROM excerpt could be
  hand-verified with the same arithmetic exercise 3.8 asks for.
- **`Mode::StackS`/`Mode::StackU`** call `format_stack_mask`, which walks
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
  `stack_transfer.rs::pulu_partial_mask` confirms mask `$16` (`A|B|X =
  $02|$04|$10`) renders as `A,B,X` through this exact function — the
  same `A,B,X` string §3.9's ROM excerpt shows for `PSHS A,B,X` at `$8C37`
  (mask also `$16`), since `format_stack_mask` doesn't care which of the
  four stack mnemonics called it except for the `OTHER_STACK_PTR` bit,
  which this mask doesn't set.

Two rendering choices worth knowing before you read a disassembly listing:

- **Constant offsets render in signed decimal, not hex.** `format!
  ("{offset},{reg}")` with `offset: i16` — so postbyte `$88 $10` (8-bit
  offset `$10`) disassembles as `16,X`, not `$10,X`, and `$88 $FF`
  disassembles as `-1,X`. `disasm_indexed.rs::offset8_positive` and
  `::offset16_negative` pin this down. Only the extended-indirect address
  (`[$XXXX]`) stays hex, since it's an absolute address, not an offset.
- **Illegal opcodes still consume the right number of bytes.** The base-page
  table falls back to `ILLEGAL = e("???", Mode::Inherent)`, reading zero
  operand bytes, matching the executor's own fallback (`_ => 2` in `step`'s
  top-level `match`), which also stops after the opcode byte.
  `page10_illegal_second_byte_is_length_2` confirms `$10 $00` disassembles
  as `???` at `len == 2` — prefix plus unmatched second byte, exactly what
  `exec_page10`'s `_ => 2` arm consumes. Get this wrong on any illegal
  opcode and a scrolling disassembly view desyncs the moment it crosses
  one. The reserved indexed sub-modes get the same discipline: postbytes
  `$87`/`$8A`/`$8E` render as `,X???` (visibly flagged) while consuming the
  executor's same zero extra bytes
  (`reserved_submodes_marked_illegal_but_zero_extra_bytes`).

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
as much as a disassembler test). Two lines are worth tracing by hand with
what you now know:

- **`8C3D: 10 26 6B 6D  LBNE $F7AE`.** `$10` is the page-10 prefix (§3.8);
  `$26` is `BNE`'s low nibble (`6`) promoted to its long form, `LBNE`; the
  16-bit offset `$6B6D` is added to the PC *after* the whole 4-byte
  instruction — `$8C41 + $6B6D = $F7AE` (wrapping `u16` arithmetic, same
  rule as every other relative branch). `decode_base` reads the `$10`, hands
  `$26` to `page10_entry`, which maps it through `LONG_BRANCH[6]`.
- **`8C41: E6 61  LDB 1,S`.** `$E6` is `LDB` indexed; postbyte `$61` is
  `0110_0001` — bit 7 *clear*, so this is the 5-bit offset form from §3.2,
  not the full `1 rr i mmmm` layout: `rr = 11` (S), `n = 00001 = 1`. `LDB
  1,S` reads the byte just above whatever `PSHS A,B,X` (three instructions
  earlier, at `$8C37`) left on the stack — a real, ROM-verified instance of
  the cheapest indexed form doing exactly the job it exists for: reaching
  one byte past the stack pointer without the overhead of an 8-bit-offset
  postbyte.

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
before an assembler existed. Two formulas cover every legal postbyte:

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
verified anchor point.

---

## 3.11 Reading assignment

In this order: **`lib.rs:76-105`** (`postbyte`/`stack_mask`, load-bearing for
everything below); **`addressing.rs:30-192`** (`ea_indexed` through
`ea_indexed_submode` — read it twice, once for shape, once sub-mode by
sub-mode with the §3.3 table open); **`stack.rs`** (`psh`/`pul`, under 80
lines); **`regs.rs`** (`reg_read`/`reg_write`/`tfr_value`); **`exec.rs:58-194`**
(`exec_page10`/`exec_page11`); then **`disasm.rs`**, **`disasm/tables.rs`**,
**`disasm/indexed.rs`** in that order — data first, rendering logic last;
finally **`tests/disasm/rom_and_scan.rs`**, for the disassembler pointed at
real, ungenerated ROM bytes instead of hand-picked fixtures.

Then run, and read while they run:

```
cargo test -p mc6809 --test indexed
cargo test -p mc6809 --test stack
cargo test -p mc6809 --test disasm_indexed
cargo test -p mc6809 --test disasm rom_reset_entry_point
```

---

## 3.12 Exercises

**3.1 — Hand-decode three postbytes (recall).** Without running anything,
decode indexed postbytes `$8B`, `$F4`, and `$9F` by hand: register field,
indirect bit, sub-mode, resulting assembly syntax, and total extra cycle
cost (sub-mode extra, plus 3 more if the indirect bit is set). Then check
every part of your answer against `crates/mc6809/src/disasm/indexed.rs` —
run the byte through `disassemble` if you want the operand string, and trace
`ea_indexed_submode` by hand for the cycle math. Get the bit arithmetic
wrong at least once before you get it right; that's the point.

**3.2 — `LEAX ,--Y`, both halves (build).** Write a test in the style of
`crates/mc6809/tests/indexed.rs` for `LEAX ,--Y` (opcode `$31`) that asserts
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
(`0xFF` full frame, `PC_CC_MASK` for FIRQ). You'll meet `CWAI`/`SYNC` as CPU
*states*, and — since the 6809 has no per-instruction conformance suite like
the 6502/Z80 — the three-legged validation strategy this codebase leans on
instead: trace-diffing against a reference emulator, a self-checking
exerciser ROM, and the hand-written corner tests you've read all month.
