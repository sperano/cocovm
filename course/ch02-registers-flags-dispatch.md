# Chapter 2 — CPU core I: registers, flags, dispatch, simple addressing

*Week 2. Goal: read `MC6809::step()` and know where every opcode goes. Last
week you saw the `MC6809` struct from the outside — a bag of registers behind
a `Bus` trait. This week you open it up. By the end you will be able to take
any 6809 mnemonic you've ever hand-assembled — `LDA $0400`, `CMPA #$0D`,
`ASL ,X` — and point at the exact line of Rust that runs when the real chip
would run it, and predict every flag it leaves behind without running the
emulator to check.*

---

## 2.1 The register file, one more time, in Rust

Chapter 1 already showed you the whole CPU struct, because there's no way to
talk about the `Bus` trait without it. Read it again, now looking at the
fields you skimmed past — `a`, `b`, `x`, `y`, `u`, `s`, `pc`, `dp`, `cc` — from
`crates/mc6809/src/lib.rs:139`:

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
indirection — when the code needs `self.a`, it writes `self.a`. Reach for
the plainest Rust type that models the hardware fact, and let the type
system do less work than you'd expect: a recurring choice in this codebase.

### D is not a register — it's a view

There is no `d: u16` field, and that's worth noticing. `D` on real 6809
silicon isn't a separate storage cell; it's `A` and `B` read and written as
one 16-bit unit, A the high byte. The struct doesn't pretend otherwise — it
stores `a` and `b` independently and computes `D` on demand (`lib.rs:167`):

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
leaving `a = 0x12`, `b = 0x34` — verified by a real test in §2.8. Why
`EXG A,B` swaps the two halves and `TFR D,X` moves the whole 16-bit value:
A and B are always two registers wearing one costume.

### The CC register: eight bits, eight names

CC is the other register not stored as eight separate booleans, for the same
reason: real 6809 code reads and writes it as a byte too (`TFR CC,A`,
`PSHS CC`, `ORCC #$50`). The bit layout, from `lib.rs:47`:

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
until week 4 — this week lives entirely in `N Z V C`, plus `H` for `DAA`.

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

---

## 2.2 Dispatch: there is no opcode table — the `match` *is* the table

If you've read another 8-bit emulator's source before, you may be expecting
an array: `const OPCODES: [fn(&mut Cpu); 256] = [...]`, indexed by the fetched
byte. That's the classic table-driven design, and it's a fine choice for the
6502 (256 opcodes, one addressing mode each, done). The 6809 doesn't get that
table here. Instead, `MC6809::step` is one large `match` on the opcode byte,
and the match arms *are* the dispatch table (`crates/mc6809/src/exec.rs:30`):

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
        0x20..=0x2F => {
            let offset = self.fetch_u8(bus) as i8 as i16 as u16;
            if self.branch_taken(opcode) {
                self.pc = self.pc.wrapping_add(offset);
            }
            3
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

> **Rust corner: chained `as` casts for sign extension.** Look back at the
> branch arm: `let offset = self.fetch_u8(bus) as i8 as i16 as u16;`. Three
> casts in a row looks like noise until you track what each one does.
> `fetch_u8` returns a `u8` — the raw byte, no notion of sign. `as i8`
> *reinterprets* those same 8 bits as signed (`0x80` stops meaning "128" and
> starts meaning "−128") without changing a single bit — this is the cast
> that actually encodes "the 6809's branch offset is signed." `as i16` then
> *sign-extends*: because the source type is signed, Rust fills the newly
> exposed high byte with copies of the sign bit, so `-1i8` (`0xFF`) becomes
> `-1i16` (`0xFFFF`), not `0x00FF`. The final `as i16 as u16` reinterprets
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

A word about the doc comment on `step` versus the one at the top of `lib.rs`.
`lib.rs` still opens with a stale banner — *"STATUS: skeleton... only a few
opcodes are decoded"* — left over from an early milestone and never updated.
The comment that actually describes what you just read is on `step` itself
(`exec.rs:12`): *"This is the complete 6809 user-mode ISA; only a handful of
illegal opcodes remain undecoded and are treated as 2-cycle NOPs during
bring-up."* When a file-level comment and a function-level comment disagree,
trust the one attached to the code you're actually looking at. This crate has
roughly two hundred passing tests; it is not a skeleton.

### Reading the match

A few things to notice about how this is organized. Some arms decode
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

One comment is load-bearing: right above the control-transfer arm,
`// JMP arms MUST precede the RMW range arms below (0x0E/0x6E/0x7E would
otherwise be swallowed by 0x00-0x0F / 0x60-0x6F / 0x70-0x7F)`. `match` tries
arms top to bottom and stops at the first match, so `0x0E` (`JMP` direct) has
to be claimed by the control-transfer arm before the RMW range `0x00..=0x0F`
gets a chance. Overlapping ranges are legal Rust, and the first one wins
silently — miss this ordering and `JMP` compiles fine and silently becomes
`NEG`.

### Family functions: one `step`, eleven helpers

Each `exec_*` function owns a contiguous slice of the opcode map and nothing
else. `exec_load_store` (`crates/mc6809/src/exec/exec_data.rs:11`) only ever
sees `LDA`/`LDB`/`STA`/`STB`/`LDD`/`STD`:

```rust
pub(super) fn exec_load_store(&mut self, bus: &mut impl Bus, opcode: u8) -> u32 {
    match opcode {
        // LDA — immediate / direct / extended
        0x86 => { let ea = self.fetch_u8(bus); self.a = ea; self.set_nz8(ea); 2 }
        0x96 => { let v = self.read_direct8(bus); self.a = v; self.set_nz8(v); 4 }
        0xB6 => { let v = self.read_extended8(bus); self.a = v; self.set_nz8(v); 5 }
        // LDB, STA, STB, LDD, STD ...
        _ => unreachable!("exec_load_store called for opcode {opcode:#04X}"),
    }
}
```

That `unreachable!()` catch-all asserts that `step`'s dispatch and
`exec_load_store`'s opcode set stay in lockstep: if someone extends the
`step` match's load-store chain but forgets the matching arm here, the
program panics loudly on first use — instead of silently falling through and
doing nothing. Compare `step`'s own catch-all, `_ => 2`, deliberately *not*
a panic (undecoded opcodes are 2-cycle no-ops "during bring-up") — two
catch-alls, two different meanings.

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
as `DESIGN.md` frames it — the right trade for a CPU you're going to read,
trace, and debug for fifteen more weeks.

---

## 2.3 Addressing modes: immediate, direct, extended

Three of the 6809's addressing modes are simple enough to cover this week —
the postbyte-driven indexed mode is next week's hardest 200 lines. All three
live in `crates/mc6809/src/addressing.rs`, and all three route through two
tiny primitives at the top of the file (`addressing.rs:7`):

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
`fetch_u16` is two of those, high byte first (big-endian, as it must be — you
met this on `Bus::read_u16` last week, hand-written again here because these
bytes come from the *instruction stream* at `PC`, not an arbitrary address).

**Immediate** addressing has no helper function at all — it's just
`fetch_u8`/`fetch_u16` called directly at the use site, because "immediate"
*is* "the operand lives in the instruction stream." You saw it already in
`exec_load_store`: `0x86 => { let ea = self.fetch_u8(bus); ... }` for
`LDA #$42`.

**Direct** addressing (`addressing.rs:20`):

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
frequently-touched I/O pages get put where `DP` reaches. The emulator reads
`self.dp`, shifts it into the high byte, and ORs in the fetched low byte —
nothing clever. Whatever `DP` holds (BASIC leaves it at `$00`; reset sets it
there per `lib.rs:178`, `self.dp = 0`) is exactly what direct mode uses,
bug-for-bug identical to hardware.

**Extended** addressing (`addressing.rs:26`) is the least surprising mode
in the instruction set:

```rust
/// Extended-mode effective address: a 16-bit operand.
pub(crate) fn ea_extended(&mut self, bus: &mut impl Bus) -> u16 {
    self.fetch_u16(bus)
}
```

A full 16-bit address, fetched straight from the instruction stream. `ea_direct`
and `ea_extended` both return an address — they do not read memory *at* that
address themselves. Reading is a separate step, bundled by four small helpers
right below them (`addressing.rs:173`):

```rust
pub(crate) fn read_direct8(&mut self, bus: &mut impl Bus) -> u8 {
    let ea = self.ea_direct(bus);
    bus.read(ea)
}

pub(crate) fn read_extended8(&mut self, bus: &mut impl Bus) -> u8 {
    let ea = self.ea_extended(bus);
    bus.read(ea)
}
```

(plus `read_direct16`/`read_extended16`, same shape, over `bus.read_u16`).
The split — compute-the-address vs. read-the-address — exists because not
every instruction wants both: `LDA` wants "compute EA, then read" (exactly
what `read_direct8`/`read_extended8` give in one call); `STA` wants "compute
EA, then write," so it calls `ea_direct`/`ea_extended` directly and does its
own `bus.write`; `JMP` wants *only* the address, `ea_direct(bus)` alone,
straight into `self.pc`. Three instructions, three combinations of the same
two primitives, no wasted work, no wrapper trying to cover every case.

> **Rust corner: `pub(crate)` visibility.** Every function in this section is
> `pub(crate)`, not `pub`: visible anywhere inside the `mc6809` crate (so
> `exec.rs` and `exec/exec_data.rs` call them freely) but invisible outside
> it — a user of this crate can call `MC6809::step`, but can't reach in and
> call `ea_direct` directly. It's Rust's version of "internal linkage,"
> marking these as decoder plumbing, not public contract. If you ever hit a
> "function is private" error while poking at this codebase from a test
> file, that's usually a sign you're testing at the wrong level — test
> through `step()`, as every file in `tests/` does.

---

## 2.4 A worked example: `LDA $0400`, start to finish

You've written this instruction a hundred times to check what BASIC left in
screen memory. Here is *everything* that happens, in order, when the emulator
executes the three bytes `$B6 $04 $00`.

**Setup.** `PC` points at `$B6`. `self.cc` is whatever it was left at.

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
   bit 6 is the inverse bit; you'll decode screen bytes properly in week 7).
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
   between them; `DP:offset` is a literal byte concatenation, not `DP * 256
   + offset`'s arithmetic cousin doing anything different, just phrased the
   way the datasheet phrases it.
3. **Read.** `bus.read(0x0540)` returns `$01`.
4. **Apply `add8`.** `self.a = self.add8(0x0F, 0x01, 0)`. By hand:
   `sum = 0x10`, `r = 0x10`. Half-carry: `(0x0F & 0x0F) + (0x01 & 0x0F) =
   0x10`, which is `> 0x0F` — `H` sets, the classic low-nibble-rolls-over
   case from a `$0F`-ending byte. `C`: `sum = 0x10`, not `> 0xFF` — clear.
   `V`: `(0x0F ^ 0x10) & (0x01 ^ 0x10) & 0x80 = 0x1F & 0x11 & 0x80 = 0` —
   clear. `N`: `r & 0x80 = 0` — clear. `Z`: `r != 0` — clear.
5. **Cycles.** `4`, the direct-mode row's literal — one less than extended's
   `5`, because there's one fewer address byte to fetch, and paid for by the
   `DP` concatenation instead.

Total: `A` goes from `$0F` to `$10`, flags land at
`(H, N, Z, V, C) = (true, false, false, false, false)`, 4 cycles charged,
`PC` advanced by exactly two bytes (opcode plus the one operand byte). This
trace was checked the same way as the second: `Sys::code(0x0000, &[0x9B,
0x40])` with `s.cpu.dp = 0x05`, `s.cpu.a = 0x0F`, `$0540` pre-loaded to
`$01` produces exactly `a == 0x10`, `cycles == 4`,
`flags == (true, false, false, false, false)`. Change `DP` to `$06` with
nothing else touched and the exact same instruction reads `$0640` instead —
the byte at the old address is simply not visited. That's the whole point
of a *direct page* register: the one-byte operand is cheap precisely because
it's relative to something the program controls, not because the hardware
is doing anything clever with it.

---

## 2.5 Flag computation as shared primitives

Here's a fact about 6809 flags that's easy to miss from the datasheet's
per-instruction tables but obvious once you read the code: `ADD`, `ADC`,
`SUB`, `SBC`, and `CMP` don't each have their own flag logic. They share two
functions, `add8` and `sub8` (`crates/mc6809/src/alu.rs:202`):

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

`ADDA` calls `self.add8(self.a, m, 0)`. `ADCA` calls `self.add8(self.a, m, c)`
where `c` is the *current* carry flag, fetched right before the call:
`let c = self.cc & cc::CARRY;`. That single `carry_in` parameter is the
entire difference between `ADD` and `ADC` — one function, one flag formula,
two callers (`exec/exec_data.rs:49-64`). The trick generalizes: `carry_in`
doubles as borrow-in for `sub8`, so `SUBA` is `self.sub8(self.a, m, 0)` and
`SBCA` is `self.sub8(self.a, m, c)`. `CMPA` computes the same subtraction as
`SUBA` and throws the result away: `self.sub8(self.a, m, 0);`, return value
unused. If you've ever wondered whether `CMPA #$0D` secretly modifies `A` —
no; the arm never assigns `sub8`'s return value anywhere:

```rust
// CMPA (result discarded, flags only)
0x81 => { let m = self.fetch_u8(bus);        self.sub8(self.a, m, 0); 2 }
```

`A` is untouched; only the flags `sub8` sets as a side effect survive.

### Why CMP feels different from SUB, but isn't

You've written this a thousand times, scanning a string for its terminator:

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
conditional branch you've ever chained after a `CMP` — `BEQ`, `BNE`, `BLO`,
`BHI`, `BLT`, `BGT`, `BLE`, `BGE` — is reading flags that a plain `SUB` would
have produced identically; "compare-and-branch" on the 6809 is compositionally
just "subtract-and-branch-on-the-leftover-flags," and now you can see why
in the source instead of taking the datasheet's word for it.

One more thing falls out of this once you notice `sub8`'s `borrow_in`
parameter: `CMPA` always calls it with `0`, never with the carry flag —
there is no "compare with borrow" instruction on the 6809, the way `SBCA`
exists alongside `SUBA`. That's not a gap; it's because you never need one.
For a quantity that's already 16 bits, `CMPD`/`CMPX`/`CMPY`/`CMPU`/`CMPS`
compare the whole thing in one shot (`sub16`, no separate borrow-in either —
see below). For a wider multi-byte comparison you'd chain by hand, subtract
low bytes with `SUBB` and high bytes with `SBCA`, then read the flags off
the *last* subtraction — but at that point you're computing a real
difference you intend to keep, so you reach for `SBC`, not `CMP`, precisely
because you already know `CMP` is only ever the zero-borrow-in,
throw-away-the-result case.

### `exec_alu8`, in full

Section 2.2 showed you individual arms from `exec_alu8` scattered across
`step`'s dispatch comment. Here's the whole function
(`crates/mc6809/src/exec/exec_data.rs:46`) — six operations (`ADD`, `ADC`,
`SUB`, `SBC`, `CMP`, and their `A`/`B` variants) across three addressing
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

Read it as a grid, not a list: six row-groups (`ADD`/`ADC`/`SUB`/`SBC`/`CMP`,
doubled for `A` and `B`), three columns each (immediate/direct/extended —
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

### BIT is to AND as CMP is to SUB

The same discard trick shows up one function over, for a different family.
`exec_logic8` (`crates/mc6809/src/exec/exec_data.rs:140`) handles
`AND`/`OR`/`EOR`/`BIT` across immediate, direct, indexed, and extended — the
first place in this chapter you'll see the indexed rows' shape, even before
next week's `ea_indexed` is explained: each one returns an `(ea, ic)` pair,
an address and its extra cycle cost, exactly parallel to extended mode's
single `ea`:

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
idiom this powers is just as familiar as the `CMP`/`BEQ` pair: `BITA #$80` /
`BMI negative_bit_set` to test one bit of `A` without disturbing it — you
already knew `BIT` "doesn't change A," and now you know exactly why: the
`&` happens, the flags get set from it, and the computed byte has nowhere to
go. `AND`/`OR`/`EOR` all call `set_nz8`, so — per §2.5's convention below —
every one of these arms clears `V` and leaves `C`/`H` untouched, whether or
not the result gets written anywhere.

### Half-carry and why it exists at all

`H` (bit 3 carry, from `(a & 0x0F) + (m & 0x0F) + carry_in > 0x0F`) has
exactly one consumer in this ISA: `DAA`. It's carried along by every
`ADD`/`ADC` regardless, in case the next instruction decimal-corrects the
result. `sub8` deliberately does *not* compute `H` — its doc comment says so
plainly ("H is left undefined (unaffected here)") — because subtraction
never needs decimal-adjusting on the 6809, so the code doesn't pretend to
compute a flag nothing reads.

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

Check it against something you already know: `$7F + $01` must set `V` (a
positive byte overflowing into negative territory). `a = 0x7F`, `m = 0x01`,
`r = 0x80`. `a ^ r = 0xFF`, `m ^ r = 0x81`. AND: `0x81` — bit 7 set, `V`
fires. Exactly the case in `tests/alu.rs::adda_signed_overflow`, walked
through in §2.8.

### The "load clears V" convention

Loads, stores, and logic operations don't compute a signed-overflow condition
at all — there's no subtraction or addition to overflow. But the 6809
datasheet still specifies `V = 0` after every `LD`/`ST`/`AND`/`OR`/`EOR`.
That convention lives in one small function, `set_nz8`
(`crates/mc6809/src/alu.rs:36`):

```rust
/// Set N and Z from an 8-bit result and clear V (the LD/ST/logic convention;
/// C and H are left unaffected).
pub(crate) fn set_nz8(&mut self, value: u8) {
    self.set_overflow(false);
    self.set_nz8_only(value);
}
```

Every `LDA`/`LDB`/`STA`/`STB` arm from §2.4 calls `set_nz8`, never
`set_nz8_only` directly — that's how `V` ends up cleared after a load without
every arm saying so. Compare `set_nz16` (`alu.rs:63`, same shape, 16-bit) used
by `LDD`/`LDX`/`LDY`, and `set_z16` (`alu.rs:43`) used only by `LEAX`/`LEAY`,
which — unusually — touch `Z` and *nothing else*, not even `N`. Three
closely related helpers, each named for exactly the flags it touches;
picking the right one is picking the right datasheet row.

### The 16-bit echo: `ADDD`/`SUBD`/`CMPX`

Everything in this section has a 16-bit twin, and it's worth seeing once so
you believe it's the same idea and not a coincidence. `exec_16bit`
(`crates/mc6809/src/exec/exec_data.rs:194-208`) opens with `ADDD`/`SUBD`/
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
position you'd predict having just read `CMPA`. `add16`/`sub16`
(`alu.rs:254`, `alu.rs:264`) are `add8`/`sub8` widened to `u16`: same carry
rule (`sum > 0xFFFF` instead of `> 0xFF`), same signed-overflow XOR-and-mask
rule against bit 15 instead of bit 7, same N/Z convention — with one thing
quietly missing. Neither has an `H` parameter, because no 16-bit instruction
on the 6809 needs a nibble-carry flag; `DAA` only ever operates on `A`,
never on `D` as a whole, so there is nothing 16-bit for half-carry to serve.
The rest of `exec_16bit` — `LDX`/`STX`/`LDU`/`STU`, elided here — is the
same load/store shape you already read in full for `LDD`/`STD` back in
§2.2's `exec_load_store`; nothing new happens there either.

### `DAA`: the payoff for `H`

`daa()` (`alu.rs:178`) is the one place `H` gets read back:

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

Walk it against the datasheet rules with the two cases the codebase itself
tests, `crates/mc6809/tests/interrupts.rs:83` and `:94` (yes, that file —
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

---

## 2.6 Read-modify-write: one dispatcher, keyed on a nibble

Section 2.2 flagged that the RMW opcode range (`NEG`/`COM`/`LSR`/`ROR`/
`ASR`/`ASL`/`ROL`/`DEC`/`INC`/`TST`/`CLR`) encodes addressing mode in the high
nibble and operation in the low nibble. `exec_rmw`
(`crates/mc6809/src/exec/exec_data.rs:236`) reads the high nibble (via the
opcode range it's matched under) and hands the low nibble to a second-level
dispatcher, `rmw_apply` (`alu.rs:156`):

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

`NEG` is implemented as literally nothing new: `0 - m`, routed through the
same `sub8` you already read in §2.5 — negating a byte and subtracting it
from zero are the same operation, and the flags fall out identically (a
borrow occurs, and thus `C` sets, for every input except `0`). The four
callers of `rmw_apply` — inherent-A, inherent-B, direct, indexed, extended —
all funnel through this one nibble switch; the only thing that differs
between them is *where `m` came from* and *whether the result gets written
back*.

### The `TST` exception

That last point is the sharpest edge in this section. `TST` computes flags
like every other RMW op but must never write its "result" anywhere — a
read-only flags probe; on real hardware, writing back would be an extra,
unwanted bus write (harmless to RAM, a real bug against a write-sensitive I/O
register). `rmw_apply` handles this by returning `m` completely unchanged for
the `TST` nibble (`0xD`) — the transform is the identity function — but the
caller still has to know not to write it back, and does, explicitly, in all
three memory-operand arms of `exec_rmw` (`exec/exec_data.rs:243`):

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

### `ASL`/`ROL`'s overflow bit, decoded

`asl8` and `rol8` (`alu.rs:133`, `alu.rs:142`) both compute `V` the same
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

---

## 2.7 Cycle counting: one number per instruction

You've now read enough arms to notice the pattern: every single one ends in
an integer literal — `2`, `4`, `5`, `6 + ic` — or delegates to a function that
returns one. That number is the *entire* timing model. `DESIGN.md` §5 states
the policy directly:

> Don't try to be cycle-*exact* mid-instruction at first; instruction-granular
> cycle counts are enough to get the ROM booting and sync interrupts roughly
> right. Tighten later only if a game needs it.

Practically: `step()` never tracks *when* during an instruction a bus access
happens, only that the whole instruction cost some fixed number of cycles
matching the datasheet's per-opcode table. `LDA` extended costs 5 cycles no
matter which of those 5 cycles the fetch, EA computation, and memory read
would "really" occupy on silicon. That's enough for everything week 6
builds — a scanline loop that runs instructions until roughly 57 cycles have
elapsed, then does video/audio/timer work — because nothing downstream needs
to know that cycle 3 specifically is when the address bus becomes valid. The
moment something *would* need that (a demo whose raster trick depends on
exact sub-instruction bus timing) is called out in `DESIGN.md` as a
deliberately deferred scope, not an oversight. `self.cycles: u64` — "the
machine's clock" from chapter 1 — is simply the running total of these
per-instruction numbers.

---

## 2.8 How the tests teach

The `mc6809` test suite isn't incidental — it's written so that reading one
test file after reading the source teaches you the same flag rules a second
way: concretely, with real numbers. Take `adda_signed_overflow` from
`crates/mc6809/tests/alu.rs:57`:

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

`Sys` (`crates/mc6809/tests/common/mod.rs:7`) formalizes the week-1 exercise's
`FlatBus`-backed toy: a real `MC6809` wired to a real `FlatBus`, `Sys::code`
loading a byte program and pointing `PC` at it, `s.step()` calling
`self.cpu.step(&mut self.bus)` directly. `0x8B` is `ADDA` immediate (§2.5's
`0x8B => { let m = self.fetch_u8(bus); self.a = self.add8(self.a, m, 0); 2 }`)
— so this test asks exactly what you can now answer by hand: what happens
when `A = 0x7F` and you `ADDA #$01`?

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
`crates/mc6809/tests/logic_rmw.rs:67`:

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
testing that `exec_logic8`'s `0x85` arm never contains an `self.a = ...`
anywhere, the same "look for the absent assignment" reading you did by eye
on `CMPA`'s arm in §2.5, now automated into something CI runs on every
commit.

One filename oddity worth flagging while you're in the test directory: the
two `DAA` tests walked in §2.5 live in `tests/interrupts.rs`, not a
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
executor too — `DAA` sits in a family function named for having nothing
else in common with its neighbors beyond "not big enough to deserve its own
arm."

---

## 2.9 Reading assignment

In this order:

1. **`crates/mc6809/src/exec.rs`, all of it** — the `step` match, then each
   `exec_*` family function, until you can say for any opcode byte on your
   instruction card which family it lands in and why.
2. **`crates/mc6809/src/exec/exec_data.rs`, all of it** — the bulk of the
   ISA's byte count. Read `exec_load_store`/`exec_alu8` closely; skim
   `exec_indexed`/`exec_16bit` (indexed addressing is next week; the 16-bit
   ops are §2.5's primitives applied to `u16`).
3. **`crates/mc6809/src/alu.rs`, all of it** — the shortest, densest file in
   the crate: every flag rule in the ISA lives here exactly once.
4. **`crates/mc6809/src/addressing.rs`, lines 1–57 plus 173–192** — the
   `fetch_*`/`ea_*`/`read_*` helpers; skip the indexed postbyte decoder in
   the middle, that's next week.
5. **The tests** — `tests/loads.rs`, `tests/alu.rs`, `tests/logic_rmw.rs`,
   `tests/common/mod.rs` — a second explanation of §2.5–2.6, in numbers
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
family-function split against `DESIGN.md` §5's framing of the indexed
postbyte as "the hardest part." In three to five sentences: why does routing
a large fraction of the ISA through one `ea_indexed` function (next week)
make the `match`-as-table design *more* attractive, not less, than a
256-entry function-pointer table? (Hint: what does a function-pointer table
force every entry's signature to look like, versus what the indexed forms
need to return?)

**2.6 — Recall: the CC layout (recall).** From memory, write the eight CC
bits in order (`E F H I N Z V C`) with their hex masks, and name the one
ALU-visible flag whose only consumer this week is a single non-arithmetic
instruction. Check against `cc` in `lib.rs:47` and §2.5's `DAA` discussion.

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
