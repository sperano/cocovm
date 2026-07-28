# Chapter 1 — The Shape of the Machine

*Week 1. Goal: understand the whole emulator's shape before touching any
chip. By the end of this chapter you will know what an emulator actually
is (it's smaller than you think), how the CoCo 3's chips divide up the
work, and the two abstractions in this codebase that everything else
hangs off — one of which exists because of the 6809, and one of which
exists because of Rust.*

---

There is a particular kind of intimidation that comes with the phrase
"write an emulator." It sounds like the sort of project that requires a
signal-processing background, or at least a deep familiarity with the
electrical behaviour of 1980s silicon. It doesn't. What it requires is
patience, a data sheet, and a willingness to be relentlessly literal
about what each chip does. The reason emulators look mysterious from the
outside is that finished ones are large — this repository is around fifty
thousand lines — and size reads as complexity. But the size is almost
entirely *breadth*: one more device, one more register, one more mode
bit. The depth is shallow, and the shape at the bottom is always the
same.

That shape is what this chapter is about. This week does not write a
single opcode. Instead it establishes the vocabulary and the mental model
that the next fifteen weeks are built on: what the three moving parts of
any emulated device are, the five chips that divided the CoCo 3's work
among themselves, and the two design decisions in this codebase that,
had they gone the other way, would have made the rest of the project
miserable. One of those decisions comes from the 6809's own hardware
interface. The other comes from Rust's borrow checker, and it is the one
that most first-time emulator authors in this language get wrong.

There is no code to run until §1.8, and even that is one command.

---

## 1.1 What an emulator actually is

Let's strip away the mystique before it has a chance to settle. An
emulator — any emulator, for any machine — is three things: some state,
a loop that advances the state, and a seam through which the state
touches the outside world. That's the whole idea. Everything else is
detail work. It's worth taking each of the three seriously for a moment,
because once you can see them in one device you can see them in all of
them, and the rest of this book becomes an exercise in pattern
recognition rather than an exercise in memorization.

### State

*State* is a handful of plain variables that mirror the registers and
memories of the real chips. Not a model of them, not an abstraction over
them — a mirror. When the data sheet says the 6809 has an 8-bit
accumulator called A, the emulator has a `u8` called `a`, and that is the
entire relationship.

Here is the proof, and it is worth stating that this is real code from
[`crates/mc6809/src/lib.rs:139`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L139),
not a teaching simplification trimmed for the book. This is the entire
CPU of the CoCo 3:

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

Every one of those fields is familiar to anyone who has written 6809
assembly. `a` and `b` are the accumulators that pair up into `D`. `x` and
`y` are the index registers. `u` and `s` are the two stack pointers, user
and system. `pc` is the program counter, `dp` the direct page register
that most programs set once and then ignore, and `cc` the condition code
byte whose bits are read as `E F H I N Z V C`. That's it. Fourteen
registers' worth of bytes, a cycle counter, and two bookkeeping fields —
`state` and `nmi_armed` — that we'll unpack properly in week 4 when
interrupts arrive.

What matters right now is what *isn't* there. There is no microcode
table. There is no hidden simulation engine, no instruction pipeline, no
"CPU context" object with a hundred fields of scaffolding. Nine registers
and three bits of housekeeping is the honest description of an MC6809E,
and the struct is the honest description of the struct. If you were
expecting the CPU to be the hard part of this project, adjust that
expectation now: the CPU is roughly three weeks of careful table-copying
from a data sheet. The GIME will take you longer.

Even the one field that isn't a register is smaller than you'd guess.
`state` is a three-variant enum, and its variants are named after two
instructions that rarely turn up in everyday 6809 code, from
[`lib.rs:123`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L123):

```rust
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

The 6809 could stop itself and wait for the outside world, which is more
than most eight-bit CPUs of its generation could manage gracefully. We'll
build both halt states properly in week 4. For now, notice only that a
capability which sounds like it needs special machinery is modelled as
one enum with three cases, and that Rust's `#[default]` attribute lets a
derived `Default` pick `Running` without anyone writing a constructor. If
you find yourself reaching for a state machine class, look again — it's
usually an enum.

### A loop

The second part is the loop, and it is embarrassingly short to describe.
Fetch the byte sitting at `PC`. Decide which instruction that byte names.
Do to the state exactly what the data sheet says that instruction does.
Add the instruction's documented cycle cost to a counter. Advance `PC`
past whatever you consumed. Repeat, forever, or until someone closes the
window.

That's the *fetch-execute cycle*, and when people say "emulator" they
usually mean this loop. It is the thing you will build a toy version of
in exercise 1.1, before you've read a line of `exec.rs`. Everything
sophisticated in a mature emulator is a refinement of when to interrupt
this loop, what to do between iterations, and how to account for the time
each iteration claims to have taken.

### A seam

The third part is the one that determines whether your codebase stays
pleasant for fifty thousand lines or turns into a plate of spaghetti in
month two. The CPU cannot live in a vacuum: it has to touch RAM, ROM, the
keyboard, the video chip, the cassette port. Every one of those touches
has to go somewhere, and the question is *where*.

In this codebase, all of them go through a single narrow interface — a
trait with exactly two required methods. That interface is the *seam*
between the CPU and everything else, and choosing it well is the most
consequential design decision in the whole project. Section 1.3 is
devoted to it, and to the three deliberate choices packed into its ten
lines of source.

### The same shape, all the way down

State, loop, seam. The remaining fifty thousand lines of this repository
are what happens when you take each device on the other side of that seam
seriously, one at a time. And here is the claim that makes the rest of
this course tractable: *each of those devices is also just state, a loop,
and a seam.* The shape recurs at every level.

Take one example on faith for now, since week 12 delivers the details.
Consider the cassette interface — a device that sounds, on the face of
it, like it should require real signal processing. Its **state** is a
decoded byte stream, a playback position, and a motor flag. Its **loop**
is "every N cycles, the current bit's tone flips the input line." Its
**seam** is a single bit that PIA1 hands to the CPU when the ROM polls
it. A tape deck, with motor physics and audio frequencies and the ROM's
own demodulation algorithm sitting on the other end of it, reduces to the
same three-part shape as the CPU does.

So when you meet a new device in this course — and you will meet a dozen
— train yourself to ask three questions before anything else. What is the
state? What is the loop? Where is the seam? If you can answer those, the
implementation is bookkeeping. If you can't, no amount of code will save
you.

### Interpreting, not translating

There is a fork in the road worth naming early, because it determines the
character of everything that follows. This emulator is an *interpreter*.
Every single time the 6809 would execute `LDA $0400`, we re-read the
opcode byte `$B6`, re-decide what it means, and re-dispatch to the code
that implements it. The tenth time through a loop costs exactly what the
first time cost.

The alternative is a *JIT*, a just-in-time translator, which compiles
each emulated instruction into native host machine code once and then
jumps straight to the compiled version on every subsequent execution.
JITs are how you emulate a PlayStation 2 at full speed on commodity
hardware. They are also wildly, comically unnecessary here, and the
arithmetic is worth doing once so you stop worrying about performance for
the rest of the book.

The CoCo 3's CPU runs at 0.895 MHz, or 1.79 MHz after the famous speed
poke. A modern laptop, conservatively, retires on the order of a billion
instructions per second per core. That means the host executes roughly
*ten thousand* instructions in the time the CoCo executes one. An
interpreter that spends fifty host instructions per emulated instruction
— a generous estimate for a `match` on a byte and a handful of field
updates — leaves better than a ninety-nine percent idle margin. You could
be ten times sloppier than that and still not notice.

What you buy with that margin is readability. An interpreted core can be
single-stepped, traced, breakpointed, and read aloud. When week 16 builds
a debugger that stops mid-instruction and shows you the register file, it
can do that because there's no compiled artifact standing between the
source and the behaviour. A course — and a debugger — cares about those
properties enormously, and cares about performance headroom we will never
spend not at all.

### Cycles are the currency

One habit to start building immediately, because it will feel strange for
about a week and then feel obvious forever: emulator code does not think
in seconds. It thinks in *CPU cycles*.

Every 6809 instruction costs a documented number of cycles. `LDA`
extended costs five. `NOP` costs two. Those numbers are printed in the
instruction-set tables on every 6809 reference card ever made, and they
are not decoration — they are the unit of account for the entire
machine. Video timing, audio sample rates, cassette bit
periods, and floppy byte pacing are all downstream conversions from the
cycle count. When week 6 builds the timing loop, "run one scanline" will
literally mean "run instructions until roughly fifty-seven cycles have
elapsed," and "run one video field" will mean doing that 262 times.

Notice, then, where `cycles: u64` sits: right there in the CPU struct,
alongside the registers. It is not debug decoration bolted on for
convenience. It is the machine's clock, and it is stored in the CPU
because the CPU is the only thing in the system that knows how much time
has passed.

---

## 1.2 A tour of the machine

What the CoCo 3 *does* is well known to anyone who ever sat in front of
one. What is far less widely known is who, precisely, was doing each part
of it — which chip drew the characters, which chip read the keyboard,
which chip decided that a byte POKEd at 1024 should appear in the
top-left corner of the screen. Emulating the machine means taking a side
in that division of labour, so let's meet the cast.

Five chips matter. Everything else on the board is glue: address
decoders, buffers, the RF modulator, and a great deal of Tandy's
cost-engineering.

```
                       ┌──────────────────────────────┐
                       │            GIME              │
   ┌─────────┐  bus    │  ┌────────┐  ┌────────────┐  │      ┌─────────┐
   │  6809E  │◄───────►│  │  MMU   │  │   video    │──┼─────►│ TV/RGB  │
   │  CPU    │         │  └────────┘  │  scanout   │  │      │ monitor │
   └────┬────┘         │  ┌────────┐  └────────────┘  │      └─────────┘
        │              │  │ timer/ │   ┌───────────┐  │
     IRQ│FIRQ◄─────────┼──│  IRQs  │   │ SAM-compat│  │
        │              │  └────────┘   └───────────┘  │
        │              └──────────────┬───────────────┘
        │                             │ physical address
        │                        ┌────▼────┐
        │                        │   RAM   │ 128K / 512K (/2MB)
        │                        └─────────┘
        │    ┌──────────┐
        ├───►│ PIA0     │ keyboard matrix, joystick comparator, sync IRQs
        │    │ ($FF00)  │
        │    └──────────┘
        ├───►│ PIA1     │ 6-bit DAC, cassette, serial bit-bang, VDG mode bits
        │    │ ($FF20)  │
        │    └──────────┘
        └───►│ cart port│ ROM paks, disk controller, Multi-Pak ($FF40)
             └──────────┘
```

The **MC6809E** is the CPU, and it is the only chip in the machine that a
6809 programmer already understands from the inside. Its addressing modes
and its flags are familiar ground. When we get to week 2 and start
reading the dispatch `match`, that existing knowledge does most of the
work; the Rust is the easy half.

The **GIME** is the interesting one. The name stands for Graphics
Interrupt Memory Enhancement, which reads like a marketing department
enumerating features rather than naming a chip, and that is more or less
what it is. Tandy commissioned it as the custom LSI part that made the
CoCo 3 a CoCo 3, and it is best understood as three devices sharing a
package:

- An **MMU** that maps the CPU's 64K address space onto 128K, 512K, or
  in principle up to 2MB of physical RAM, in 8K blocks. This is the
  answer to a question the machine's own documentation rarely addressed
  head-on: how did BASIC, a user program, *and* a high-resolution
  screen all coexist in a 64K address space? They didn't. They took turns,
  and the GIME decided whose turn it was, eight kilobytes at a time.
- The **video scanout** hardware. This replaced the CoCo 1 and 2's MC6847
  Video Display Generator while remaining able to imitate it. That
  imitation mode is not a curiosity — it is what the green text screen
  every CoCo 3 booted into actually was. Week 7 has the receipts.
- An **interrupt controller and a 12-bit timer**, plus a compatibility
  layer that answers to the old SAM chip's addresses. The timer is the
  thing that makes cursor blink and 60 Hz music possible without the CPU
  counting instructions.

Then there are **two MC6821 PIAs** — Peripheral Interface Adapters. A PIA
is a deeply unglamorous chip: two 8-bit parallel ports with direction
control and a pair of handshake lines, and nothing else. Tandy wired them
to everything cheap. The keyboard matrix hangs off them. So does the
joystick comparator, the six-bit sound DAC, the cassette input and motor
relay, and the printer's bit-banged serial line. When a BASIC program
did `PRINT PEEK(65280)` to read the keyboard, it was reading PIA0's data
register at `$FF00`. When the cassette relay clicked, PIA1 had just
changed one bit.

**RAM and ROM** round out the memory picture: 32K of Super Extended Color
BASIC in ROM, and RAM sitting behind the MMU where the CPU can only see
64K of it at a time.

Finally, **the cartridge port**, which is barely a device at all — it's a
raw extension of the bus with a chip-select line and two interrupt lines
brought out to the connector. This is worth internalizing early, because
it demystifies a whole category of hardware. A disk controller is not
special hardware as far as the CoCo is concerned. It's a cartridge that
decodes a few addresses in the I/O page and yanks the HALT and NMI lines
at the right moments. Week 13 will build one, and the surprise will be
how little the rest of the machine has to know about it.

### The one table that ties the course together

The 6809 sees 64K, and the top 256 bytes of that space — `$FF00` through
`$FFFF` — are the **I/O page**, where every device in the machine
appears. This one page is the meeting point of every subsystem in this
book. You'll internalize the map properly in week 5 when we implement the
address decoder, but it's worth a bookmark right now, if only so the
chapter numbers give you a sense of the shape of the journey:

| Address       | Device                                | Course week |
|---------------|---------------------------------------|-------------|
| `$FF00–$FF03` | PIA0 — keyboard, joystick, sync IRQs  | 10          |
| `$FF20–$FF23` | PIA1 — DAC, cassette, VDG mode bits   | 10–12       |
| `$FF40–$FF5F` | cartridge / disk controller           | 13          |
| `$FF90–$FF9F` | GIME control: INIT0/1, IRQs, timer, video | 5, 8    |
| `$FFA0–$FFAF` | GIME MMU task registers               | 5           |
| `$FFB0–$FFBF` | GIME palette (16 registers)           | 8           |
| `$FFC0–$FFDF` | SAM-compatibility strobes             | 5           |
| `$FFE0–$FFFF` | ROM: interrupt vectors                | 4           |

`POKE 65497,0` is the poke CoCo owners learned early to make a program
run twice as fast, and the poke that broke something the first time about
as often as it worked. Decimal 65497 is `$FFD9`, which lands squarely in
the SAM-compatibility row. By week 6 you'll know exactly what that poke
does to the emulator's main loop, and the answer is delightfully
anticlimactic: it changes one integer. The thing that felt like magic
turns out to be a multiplier on a scanline's cycle budget.

### Why the GIME answers to a dead chip's addresses

That "SAM-compatibility strobes" row deserves a paragraph of its own,
because it explains a pattern you'll meet again and again in this course
— and, frankly, a pattern you'll meet in every piece of consumer hardware
that ever had a successor.

The CoCo 1 and CoCo 2 had no GIME. They had two separate chips doing the
GIME's jobs. The **MC6883 SAM**, the Synchronous Address Multiplexer,
handled memory control, video addressing, and the CPU clock rate. The
**MC6847 VDG**, the Video Display Generator, produced the actual
characters and graphics. Between them they owned the memory-and-video
half of the machine.

When Tandy designed the CoCo 3, the GIME swallowed both jobs into one
part. That would have been a clean break — except that by 1986 there were
thousands of programs in the wild already poking the SAM's registers in
the `$FFC0–$FFDF` range and flipping the VDG's mode bits through PIA1,
and one of those programs was the BASIC ROM itself. Breaking them was not
an option. So the GIME keeps answering at the old addresses, imitating
the old chips' behaviour well enough that software written for a machine
that no longer exists continues to work.

What is genuinely satisfying is that this codebase mirrors the
silicon's family history rather than papering over it. There is a real
`Sam` type in
[`crates/coco-core/src/sam.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/sam.rs),
and it is used *only* when emulating a CoCo 1 or CoCo 2. The CoCo 3 path
routes the very same addresses into the GIME's own compatibility layer in
[`crates/coco-core/src/gime/sam_compat.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/gime/sam_compat.rs).
Two implementations of one legacy interface, because that is what Tandy
shipped. Backward compatibility is not a footnote in this machine; it is
the reason the CoCo 3 boots into a 1980 video mode (week 7), and it is
why half the GIME's register map exists at all.

### Decoding the odd clock

One more number is worth pulling apart while we're taking inventory,
because it explains something about how all home computers of this era
were built. The CPU clock is usually quoted as 0.895 MHz, which is a
strange enough figure to make you wonder who chose it. The exact value in
this codebase lives in
[`crates/coco-core/src/machine.rs:26`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine.rs#L26):

```rust
/// NTSC CPU clock at normal speed: the 28.636363 MHz crystal / 32 (MAME
/// `coco3.cpp`). The SAM R1 bit doubles it (crystal / 16, ~1.79 MHz).
const CPU_HZ: f64 = 894_886.0;
```

Nobody sat down and decided the CPU should run at 894,886 Hz. What
somebody decided was that the machine would plug into a television, and
that decision cascaded. The 28.636363 MHz master crystal is eight times
the NTSC colour subcarrier of 3.579545 MHz, which is the frequency the
video output has to respect if colour is going to survive the trip to
the television. Divide the crystal by 32 and you get the CPU clock.
Divide it by 16 instead — which is exactly what the SAM's speed bit does
— and you get the 1.79 MHz the speed poke unlocked.

Every timing constant in the machine hangs off that one crystal. The
GIME's fast timer clock, for instance, is documented in the very next
constant in the same file as running at 3.579545 MHz, "exactly 4× the
0.89 MHz CPU clock." Video timing, CPU timing, and cassette baud rates
are all integer relationships to a frequency chosen for television
compatibility. That shared origin is not a coincidence you can ignore;
it's the reason week 6 can drive the entire machine off a single cycle
counter without any of the subsystems drifting apart.

---

## 1.3 Load-bearing abstraction #1: the `Bus` trait

We've established that the CPU needs a seam to reach the outside world.
Now let's look at the actual seam, decide why it is shaped the way it is,
and notice the three separate design decisions hiding in what looks like
a trivial interface.

Here it is in full, from
[`crates/mc6809/src/lib.rs:29`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L29):

```rust
/// The CPU's view of the outside world.
///
/// `read` takes `&mut self` deliberately: reads can have side effects (PIA flags,
/// GIME status registers clear-on-read). See `DESIGN.md` §2a.
pub trait Bus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);

    /// Big-endian 16-bit read (6809 is big-endian).
    fn read_u16(&mut self, addr: u16) -> u16 {
        let hi = self.read(addr) as u16;
        let lo = self.read(addr.wrapping_add(1)) as u16;
        (hi << 8) | lo
    }

    /// Big-endian 16-bit write.
    fn write_u16(&mut self, addr: u16, val: u16) {
        self.write(addr, (val >> 8) as u8);
        self.write(addr.wrapping_add(1), val as u8);
    }
}
```

Two required methods. Read a byte at an address; write a byte at an
address. That is the total extent of what the `mc6809` crate knows about
the universe outside itself. It does not know the memory map. It does not
know a GIME exists. It does not know it's in a CoCo, and in fact it would
be equally happy in a Vectrex or a Dragon 32, both of which also used a
6809. Every `LDA`, every stack push, every interrupt vector fetch, every
indexed-indirect address resolution funnels through `read` and `write`.

That narrowness is not laziness. Three deliberate decisions are packed
into those lines, and each one gets cashed in later in the course.

### Decision 1: `read` takes `&mut self`

At first glance this looks like a mistake, or at best a Rust novice's
over-caution. Reading memory doesn't change anything, does it? You look
at a byte; the byte is still there.

On real hardware, reading absolutely does change things, and the CoCo is
full of examples. Two you'll meet within a few weeks:

- Reading a PIA data register **clears that PIA's interrupt flag**
  (week 10). This isn't a quirk to work around — the ROM's 60 Hz
  interrupt handler *depends* on it. The handler reads `$FF02` for the
  specific purpose of acknowledging the interrupt, and if your emulated
  read doesn't clear the flag, the machine takes the same interrupt again
  immediately and never makes forward progress.
- Reading the GIME's IRQ status register at `$FF92` returns the pending
  interrupt bits *and clears them* in the same access (week 8). Read it
  twice and the second read tells you nothing happened, which is the
  whole point.

Now imagine `read` had been declared `fn read(&self, addr: u16) -> u8`.
You could not model either behaviour without reaching for *interior
mutability* — Rust's escape hatch for mutating through a shared
reference, spelled `Cell` or `RefCell`. Those types would have to appear
inside the PIA, inside the GIME, inside anything that can have a
read side effect, and every field they wrapped would lose the ability to
be read as a plain value. You'd be paying a syntactic and cognitive tax
on every device in the machine to preserve a fiction that the hardware
never honoured in the first place.

By putting mutability in the seam's contract on day one, every device is
free to have honest read side effects with no ceremony at all. A PIA
register read is a method that takes `&mut self` and mutates a flag,
which is exactly what it is.

That leaves one real problem, and the codebase answers it directly.
Sometimes you genuinely need a side-effect-free read: the debugger's
memory viewer, hovering over `$FF02` and refreshing sixty times a second,
had better not eat a pending interrupt every frame. So there is a
*separate* path for that, an explicitly side-effect-free `peek()` in
[`crates/coco-core/src/bus/peek.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/peek.rs),
which week 16 builds on. Two functions with two contracts, and both
contracts visible in the type signatures rather than in a comment nobody
reads. When you're deciding how to model something ugly, "make the ugly
thing explicit and give the clean case its own name" beats "pretend the
ugly thing doesn't exist" almost every time.

### Decision 2: the CPU is *generic* over the bus

The CPU's step function is declared like this, from
[`crates/mc6809/src/exec.rs:30`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs#L30)
— we'll dissect its body next week:

```rust
pub fn step(&mut self, bus: &mut impl Bus) -> u32 {
```

The `impl Bus` in the parameter position means the CPU works against
*any* type that implements the trait, with the concrete type chosen at
each call site. It returns a `u32`: the number of cycles that
instruction consumed.

The payoff is immediate and larger than it looks. Because the CPU only
demands "something with `read` and `write`," the `mc6809` crate can ship
its own trivial bus for testing — a bare 64K array with no CoCo attached
anywhere. Here it is, from
[`crates/mc6809/src/lib.rs:257`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L257):

```rust
/// A flat 64K address space — for unit tests and the flexemu `cputest` harness.
pub struct FlatBus {
    pub mem: Box<[u8; 0x10000]>,
}
```

and its implementation of the trait, a few lines further down at
[`lib.rs:281`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L281):

```rust
impl Bus for FlatBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.mem[addr as usize]
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.mem[addr as usize] = val;
    }
}
```

Six lines, and the entire CPU test suite has somewhere to live. The
harness that the crate's roughly two hundred tests share wraps that bus
and the CPU together in a struct with a wonderfully unambitious name, from
[`crates/mc6809/tests/common/mod.rs:7`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/tests/common/mod.rs#L7):

```rust
pub struct Sys {
    pub cpu: MC6809,
    pub bus: FlatBus,
}
```

with two helpers that between them define what a CPU test even is:

```rust
    /// Build a system with `bytes` loaded at `addr` and PC pointed at it.
    pub fn code(addr: u16, bytes: &[u8]) -> Self {
        let mut s = Self::new();
        s.bus.load(addr, bytes);
        s.cpu.pc = addr;
        s
    }

    /// Execute one instruction; returns cycles consumed.
    pub fn step(&mut self) -> u32 {
        self.cpu.step(&mut self.bus)
    }
```

Read `code` slowly, because it is the shape of every CPU test in the
book. It builds an empty system, drops your hand-assembled bytes at an
address, and points `PC` at them. To test `LDA $0400`, you call
`Sys::code(0x1000, &[0xB6, 0x04, 0x00])`, call `step()`, and assert on
`sys.cpu.a` and the returned cycle count. No ROM, no GIME, no video
timing, no boot sequence. Just three bytes and a question.

Meanwhile the real machine implements the identical trait on
`SystemBus`, which owns all the actual devices — that's week 5's work.
Same CPU code, byte for byte, running in both worlds. The practical
consequence is worth stating plainly, because it will save you days: when
a CPU test fails, you *know* it's the CPU, because there is no machine in
the room to blame.

> **Rust corner: monomorphization, or why this costs nothing.**
> `fn step(&mut self, bus: &mut impl Bus) -> u32` is a generic function,
> and Rust compiles generics by *monomorphization*: the compiler emits a
> separate, fully concrete copy of `step` for each bus type actually used
> in the program. One copy is compiled against `FlatBus`, another against
> `SystemBus`. Inside each copy, `bus.read(...)` is a direct call to a
> known function, and usually an inlined one.
>
> The alternative would have been to store a `&mut dyn Bus` — a *trait
> object*, where the concrete type is erased and every method call goes
> through a pointer in a vtable looked up at run time. That is a
> perfectly reasonable technique in general, and this codebase does use
> trait objects where the flexibility is worth it. But memory access
> happens several times per instruction, on the hottest path in the
> entire program, and there we get the abstraction genuinely for free.
> The cost is paid in compile time and binary size, not in the run loop.
> When you see `impl Trait` in this codebase, read it as "resolved at
> compile time."

> **Rust corner: default methods.** Notice that `read_u16` and
> `write_u16` have bodies *inside the trait declaration*. Anything that
> implements `Bus` gets both for free, defined once in terms of `read`
> and `write`, and can still override them if it has a faster path.
>
> There's a 6809 fact hiding in those bodies, and it is the single
> easiest thing to get wrong in an emulator: the high byte comes first.
> The 6809 is *big-endian*, meaning a 16-bit value at address `$FFFE` is
> stored with its most significant byte at `$FFFE` and its least
> significant at `$FFFF`. Get this backwards and your emulator fetches
> every interrupt vector and every 16-bit operand byte-swapped, which
> means the reset vector alone will send you to a completely arbitrary
> address before you've executed a single useful instruction. Writing
> the rule down once, in a default method, is how you make it impossible
> to get wrong in the other two hundred places that need it.
>
> Note `wrapping_add` too, and take it as a house style. Address
> arithmetic on this machine must wrap from `$FFFF` back to `$0000`,
> because that's what sixteen address lines do — there is no seventeenth
> bit for the carry to go into. Rust's ordinary `+` on a `u16` panics on
> overflow in debug builds and wraps silently in release, and neither of
> those is what you want. Every address computation in this codebase
> uses the `wrapping_*` family. When you're reading emulator code, treat
> a bare `+` on a `u16` address as a bug until proven otherwise.

> **Rust corner: `#![forbid(unsafe_code)]`.** The first line of actual
> code in
> [`crates/mc6809/src/lib.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L11),
> line 11, before a single `mod` or `use`, is `#![forbid(unsafe_code)]`.
>
> The distinction from `#![deny(...)]` matters. A `deny` can be
> overridden further down by an `#[allow]` attribute on some function
> that someone was in a hurry about. A `forbid` cannot — it is a promise
> the entire crate is checked against on every build, with no escape
> hatch anywhere inside it. There are no pointer tricks in this CPU,
> now or ever.
>
> This is a deliberate stake in the ground. An emulator is *exactly* the
> kind of program where the C tradition reaches for casts, unions, and
> aliased buffers: you're modelling raw memory, so why not use raw
> memory? Because the bugs you get from that are precisely the bugs you
> cannot debug — silent corruption that manifests three subsystems away
> as a wrong pixel. This crate takes the opposite position and has never
> needed to walk it back. When you write your own core in exercise 1.1
> and beyond, start with the same line. It converts an entire class of
> emulator bug into a compile error.

### Decision 3: the seam is *tiny*

The third decision is the one you notice by what's absent. There is no
`fetch_opcode` method. No `dma_transfer`. No `get_keyboard_state`, no
`notify_video_start`, no convenience hook that the CPU can use to ask the
machine a question. Everything is a byte at an address.

That restraint is a form of fidelity. The real MC6809E had sixteen
address pins and eight data pins, and one bus transaction was one byte at
one address; the chip had no vocabulary for anything richer. By giving
our emulated CPU exactly the same vocabulary, we guarantee that it cannot
accidentally depend on information the real chip never had. If the real
6809 couldn't express something in a single bus transaction, ours can't
either, and any behaviour we get right, we get right for the right
reason.

There's a practical dividend as well. Because the seam is two methods,
implementing it for a new machine is an afternoon's work rather than a
port. Exercise 1.8 asks you to think about what it would take to run this
CPU in a Vectrex emulator, and the answer is pleasantly short.

---

## 1.4 Load-bearing abstraction #2: the borrow-checker strategy

This is the section where Rust stops being an implementation detail and
starts shaping the architecture. It's also the part that first-time
emulator authors in this language most reliably get wrong — usually
discovering the problem three weeks in, with a half-built machine that
won't compile and no obvious way forward that doesn't involve rewriting
everything.

Far better to meet it in week 1, on paper, than in week 5 with sunk
cost.

### The problem

Think about ownership for a moment. You're building a machine that
contains a CPU and a pile of devices. The obvious design — the one you'd
write in C without a second thought, and the one that feels natural in
any language with unrestricted pointers — is a `Machine` that owns
everything, flat:

```rust
// The design that does NOT work:
pub struct Machine {
    cpu: MC6809,
    ram: Box<[u8]>,
    gime: GIME,
    pia0: MC6821,
    // ...
}
```

Perfectly reasonable. Now try to run one instruction.

The CPU has to mutate itself, so you need `&mut self.cpu`. The CPU also
needs a bus, and the bus is *the rest of the machine* — and by Decision 1
above, reads mutate, so it must be `&mut` too. So you write the obvious
thing:

```rust
self.cpu.step(&mut self)
```

and the borrow checker stops you cold with error E0499: cannot borrow
`self` as mutable more than once at a time. You've already borrowed
`self` mutably by naming `self.cpu`, and you cannot hand out a second
overlapping `&mut self` on top of it.

Here's the part worth sitting with: **the checker is right.** This is not
Rust being pedantic about a pattern that would have been fine. Through
that second `&mut self`, the CPU could reach back into `self.cpu` — the
very thing it's currently mutating — and alias itself. Nothing in the
type system prevents `step` from writing to `self.cpu.pc` through the bus
reference while `step`'s own `&mut self` believes it has exclusive
access. In C you'd get away with it because nobody's checking, right up
until the day you don't.

That traditional C design, where every device holds a pointer back to the
machine so it can reach anything from anywhere, is precisely the aliasing
that Rust exists to reject. You are not going to talk the compiler out of
it.

So what do people do? The first workaround almost everyone reaches for is
to wrap every device in `Rc<RefCell<…>>`, which moves the borrow checking
from compile time to run time. And it works, in the narrow sense that it
compiles. It also does three unpleasant things. It litters every single
device interaction with `.borrow_mut()`, which is noise on every line
forever. It converts aliasing bugs from compile errors into run-time
panics, which is a strictly worse place to find them. And — the quiet
killer, the one nobody sees coming — it turns your machine from a *tree
of owned values* into a *graph of shared pointers*.

Hold that third one. We'll collect on it in a few pages.

### The fix: partition the state along the borrow

The solution in this codebase isn't a trick or a workaround. It's
structural, and it consists of drawing exactly one struct boundary in
exactly the right place. From
[`crates/coco-core/src/machine.rs:61`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine.rs#L61):

```rust
/// The whole emulated machine.
///
/// The CPU is one field and everything else lives in `bus`, so `cpu.step(&mut bus)`
/// borrows two disjoint fields without `Rc`/`RefCell` (`DESIGN.md` §2b).
#[derive(Serialize, Deserialize)]
pub struct Machine {
    pub cpu: MC6809,
    pub bus: crate::SystemBus,
    pub config: MachineConfig,
```

Three fields at the top, and the first two are the whole idea. The CPU is
one field. *Everything else* — RAM, ROM, the GIME, both PIAs, the
cartridge, the keyboard, the cassette, the joysticks — lives inside the
second one. You can see the full inventory in the `SystemBus` declaration
at
[`crates/coco-core/src/bus.rs:37`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L37);
it's a long struct, and every field in it is a device you'll implement
before the course is over.

With the state partitioned that way, running an instruction becomes:

```rust
self.cpu.step(&mut self.bus)
```

and the borrow checker is perfectly happy. Rust's borrow analysis
operates on *field paths*, not just on whole values: it can see that
`self.cpu` and `self.bus` name disjoint pieces of memory, so a shared
borrow of one and a mutable borrow of the other alias nothing and are
allowed to coexist. No `Rc`. No `RefCell`. No `unsafe`. The machine stays
a plain tree of owned values, and a single struct boundary placed along
the borrow line makes the entire architecture compile.

Take a moment to appreciate how little this cost. The fix is not a
pattern, or a framework, or a clever lifetime signature. It's a decision
about which fields live in which struct — made once, in week 1, with the
borrow checker's rules in mind rather than against them.

> **Rust corner: `Box<[u8]>`, not `Vec<u8>`.** Look at how `SystemBus`
> stores memory, from
> [`crates/coco-core/src/bus.rs:44`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L44):
>
> ```rust
>     /// CBOR-native bytes (`#[serde(with = "serde_bytes")]`) — the single
>     /// biggest snapshot payload, up to 2 MB.
>     #[serde(with = "serde_bytes")]
>     pub ram: Box<[u8]>,
>     /// Skipped: COPYRIGHTED ROM bytes never travel through a snapshot;
>     /// re-injected on restore via [`SystemBus::reattach_rom`]
>     /// (`docs/plan-save-states.md`). Deserializes to an empty `Box<[u8]>`
>     /// (every read against it falls through to `OPEN_BUS` until reattached).
>     #[serde(skip)]
>     pub rom: Box<[u8]>,
> ```
>
> A `Vec<u8>` would work here, so why the less common type? A boxed slice
> is a `Vec` with the growability surgically removed. Its length is fixed
> at allocation, it carries no spare-capacity field, and there is no code
> path anywhere that can `.push()` onto it. RAM size is decided exactly
> once, at construction — 128K, 512K, or 2MB — and the type now enforces
> what the hardware guarantees: memory does not grow while the machine is
> running.
>
> The construction itself is a one-liner in `SystemBus::new`:
> `ram: vec![0u8; memory.bytes()].into_boxed_slice()`. You build a `Vec`
> of the right size and then throw away its ability to change size. This
> is the same philosophy as the `cc` module of named bit masks you'll
> meet in week 2: pick the type that says exactly what the hardware does,
> and no more. When you see `Box<[u8]>` in this codebase, read it as "a
> buffer whose size is a *decision*, not a variable."
>
> The two attributes are a preview of week 16, and the `rom` one is more
> interesting than it looks. Save states must not contain copyrighted ROM
> bytes, so the field is skipped on serialization and re-injected from
> your local ROM file on restore. Ownership discipline and licensing
> discipline turn out to want the same thing here.

### The same trick, one level down

The pattern doesn't stop at the `Machine`/`SystemBus` boundary. It
recurs inside the bus, and recognizing it in its second form is what
makes the technique usable rather than a one-off.

During video scanout — week 8's material — the renderer needs three
things simultaneously. It needs the GIME's registers, to know what mode
and palette are in effect. It needs the RAM it's scanning out of, because
that's where the pixels come from. And it needs the framebuffer it's
painting into. Two of those live inside `self.bus`, one lives directly on
`Machine`, and all three are needed at once.

If `paint_scanline` were a method on `Machine` taking `&mut self`, we
would be right back where we started: one giant mutable borrow, and no
way to hand out the pieces. Instead it's a free function that receives
exactly the pieces it needs. Here's the real call site, from
[`crates/coco-core/src/machine/render.rs:56`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/render.rs#L56):

```rust
        gime_video::paint_scanline(
            &self.bus.gime,
            &self.bus.ram,
            scan,
            blink_on,
            row,
            &mut self.framebuffer,
        );
```

Walk the arguments and count the borrows. `&self.bus.gime` is a shared
borrow of one field inside the bus. `&self.bus.ram` is a shared borrow of
a different field inside the same bus. And `&mut self.framebuffer` is a
mutable borrow of a field on `Machine` itself, live at the same time as
the other two. Three simultaneous borrows reaching into `self`, zero
conflicts, because each one names a distinct field path that the compiler
can prove doesn't overlap the others.

When a whole subsystem needs many fields at once and listing them at the
call site gets unwieldy, the same idea scales through destructuring:

```rust
let SystemBus { gime, ram, .. } = &mut self.bus;
```

That single line splits one struct into several independent mutable
borrows, one per named field, and the compiler tracks them separately
from there. You'll see both forms throughout the codebase. Together they
are the reason it contains no interior mutability at all in the machine
state — not one `RefCell` in the devices you'll be implementing.

The rule generalizes, and it's the design lesson to carry out of this
chapter: **in Rust, you partition state by who needs to borrow what
simultaneously, not by what belongs together conceptually.** On paper,
the CPU and the GIME are peers — two chips on one board, drawn side by
side in every block diagram including the one in §1.2. In the struct
layout, the CPU is a top-level field and the GIME is nested a level down
inside the bus, and the *only* reason is who borrows whom. If that feels
like letting the compiler dictate your architecture, well — yes. It does.
And the architecture it dictates turns out to be a good one, for reasons
we're about to get to.

### Why this obsession pays off: save states

Here's the held thought from a few pages back, the one about trees versus
graphs.

Because the machine is a plain owned tree — no `Rc`, no `RefCell`, no
back-pointers from devices to their parent — one derive line on `Machine`
does something remarkable:

```rust
#[derive(Serialize, Deserialize)]
```

That single attribute makes the **entire machine state serializable**.
Freeze the CoCo mid-instruction, write it to disk, come back next week
and restore it exactly where it stopped. That's the `.ccstate` save-state
feature, and week 16 builds it — but the hard part was already paid for,
in week 1, by refusing shared ownership.

Consider what the `Rc<RefCell<…>>` version would have cost. Shared nodes
serialize as duplicates, so a device referenced from two places comes
back as two independent copies with no link between them, and the
restored machine is subtly, unfixably wrong. Reference cycles don't
serialize at all — serde will happily recurse until the stack runs out.
You'd be writing custom serialization code with an interning table, by
hand, for a machine with forty devices in it. Somewhere between painful
and impossible, and either way not a weekend.

Instead, the architecture decision of week 1 quietly purchased the
flagship feature of week 16. This is the single best example in this
codebase of an early constraint paying compound interest, and it's the
argument to make to anyone who thinks the borrow checker is a tax rather
than a design tool.

> **Rust corner: `#[serde(skip)]`.** Not every field belongs in a
> snapshot, and the `Machine` struct spells out which ones don't. Look at
> the field right after `config`, from
> [`crates/coco-core/src/machine.rs:70`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine.rs#L70):
>
> ```rust
>     /// RGBA framebuffer for the active video field (`DESIGN.md` §6). Its size is
>     /// mode-dependent: each renderer fills a native-size buffer and the frontend
>     /// scales to fit (`video-output-architecture` Option A). Skipped: cheap to
>     /// rebuild (it's just the render target), rebuilt to the legacy geometry by
>     /// [`Machine::after_restore`] (`docs/plan-save-states.md`).
>     #[serde(skip)]
>     pub framebuffer: Vec<u8>,
> ```
>
> `#[serde(skip)]` means two things at once: don't write this field into
> the snapshot, and on load, fill it with `Default::default()` instead of
> reading it. Why skip a framebuffer? Because it's *derived* state. The
> next rendered field repaints every pixel from RAM and the GIME's
> registers anyway, so persisting a screenful of RGBA would bloat every
> snapshot to buy nothing.
>
> Sorting every field into "essential" or "derived" is exactly the
> judgment call you'll practice in week 16, and it's less obvious than it
> sounds — get it wrong in the derived direction and restores are subtly
> broken; get it wrong in the essential direction and your snapshots are
> full of scratch buffers. The doc comments on the skipped fields in
> [`machine.rs:70-95`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine.rs#L70-L95)
> spell out the reasoning field by field. Reading them now, before you
> know what half the fields do, is still worthwhile — the *form* of the
> argument is what you're picking up.

### One honest caveat

For the record-keepers, and because a book that claims a codebase is
perfectly pure is a book you should distrust: the "no shared ownership"
rule holds absolutely for the *machine state tree*, and bends at exactly
two host-facing edges.

The printer capture sink is a shared `Rc<RefCell<Vec<u8>>>` handle,
because the frontend and the emulated printer port genuinely both need to
reach the same buffer. And the PTY code that bridges the emulated serial
port to a terminal on the host machine makes `libc` calls inside `unsafe`
blocks, because that's what talking to a Unix pseudo-terminal requires.
Both live precisely at the boundary where the emulator stops and the host
begins, both are excluded from save states, and week 14 examines each of
them in detail.

The lesson survives contact with reality, slightly sharpened: shared
ownership is *banned* from the state you snapshot, and *tolerated* only
where the host forces your hand. That's a rule you can actually follow,
which is more than can be said for purity.

---

## 1.5 Why three crates

We've now met both seams — the `Bus` trait between the CPU and the world,
and the `Machine`/`SystemBus` split between what borrows and what gets
borrowed. It turns out those two seams also explain the shape of the
workspace, because the crate boundaries were drawn along them.

The workspace root
[`Cargo.toml`](https://github.com/sperano/cocovm/blob/main/Cargo.toml)
lists three members:

```
crates/
├─ mc6809/      the CPU. Depends on nothing. Knows only the Bus trait.
├─ coco-core/   the machine: SystemBus, GIME, PIAs, timing, media. Headless.
└─ coco-egui/   the frontend: window, texture, sound device, debugger UI.
```

**`mc6809` is standalone.** The `Bus` trait makes that possible; testing
makes it necessary. CPU bugs are the hardest bugs in an emulator to find,
because a wrong flag in an obscure addressing mode doesn't announce
itself — it shows up four thousand instructions later as BASIC printing
the wrong prompt, or not printing anything at all. Week 4 is largely
about that problem. The defence is a CPU you can test in complete
isolation, with nothing else in the room that could plausibly be at
fault, and that requires the CPU crate to have no idea a CoCo exists.

**`coco-core` is headless.** It renders into a plain `Vec<u8>`, pushes
audio samples into a `Vec<[f32; 2]>`, and never opens a window or touches
a GPU. This is not an aesthetic preference. It is what makes the
repository able to contain a test that boots the real Color BASIC ROM,
types `PRINT 2+2` one simulated keystroke at a time, and asserts on the
resulting pixels — running in CI, on a machine with no display attached
([`crates/coco-core/tests/coco1_boot.rs:93`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/coco1_boot.rs#L93);
you'll step through it in week 6). Think about what that test is actually
checking: the CPU decoded thousands of instructions correctly, the
address decoder mapped ROM and RAM correctly, the keyboard matrix
reported the right rows, the interrupt timing let the ROM's keyboard scan
run, and the video path put the right glyphs at the right addresses. One
assertion, most of the machine.

The frontier between "emulator" and "app" is exactly the frontier between
"testable in CI" and "needs a human looking at a monitor," and the crate
boundary is drawn there deliberately.

**`coco-egui` changes fastest and matters least.** Menus, texture upload,
keymaps, file dialogs, the debugger's UI panels. This is the layer where
you'll fiddle endlessly with pixel-perfect scaling and where none of that
fiddling should ever force a recompile of the GIME. Keeping it out of the
core keeps rebuilds cheap and, more importantly, keeps UI concerns from
leaking into device code — no device in `coco-core` knows what a window
is, and none of them should.

The dependency arrows point one way only: `coco-egui → coco-core →
mc6809`. The CPU doesn't know the CoCo exists. The machine doesn't know
the screen exists. Every week of this course until week 15 lives entirely
in the first two crates, and you could delete the third one and still
have a working, testable emulator — just not one you could play a game
on.

---

## 1.6 What counts as "accurate"? Fidelity is a budget

Before you read another line of emulator code, you need vocabulary for a
question that comes up every single week and that has no universal
answer: *how faithful is faithful enough?*

Emulation fidelity is a spectrum rather than a binary, and every point on
it costs implementation effort, complexity, and often performance. Four
rungs, from loosest to tightest:

1. **Functional.** The device produces the right *results* in the right
   *order*, on a schedule of its own choosing. A disk read returns the
   correct bytes after "some" delay that feels about right.
2. **Instruction- or byte-granular.** The results are right *and* time is
   accounted for at the granularity of whole instructions or whole bytes
   transferred. You know how many cycles an operation took; you don't
   know how they were distributed inside it.
3. **Cycle-accurate.** Every bus cycle happens on exactly the cycle the
   real chip would have produced it, including in the middle of an
   instruction. If the real 6809 spends cycle 3 of a 5-cycle instruction
   doing a dead bus access, so does yours.
4. **Gate-level.** You are simulating the netlist — transistors and
   wires, extracted from die photographs. This is how projects like
   Visual 6502 work. Nobody in this course is doing this; it is named
   here mostly so the ladder's top is visible.

The trap waiting for a first-time emulator author is the belief that
"more accurate" is always better. It isn't. It's *more expensive*, and
software only notices the difference at specific, discoverable points.
Spending week 3's energy on cycle-exact bus timing for the CPU buys you
nothing if no CoCo program in existence can tell — and it costs you week
3.

This codebase makes its fidelity choices explicitly, subsystem by
subsystem, and reading it well means noticing each choice and asking two
questions: what would break if this were sloppier, and what would it cost
to be stricter?

| Subsystem | Fidelity chosen | Where you'll study it |
|-----------|-----------------|----------------------|
| CPU cycles | instruction-granular (no mid-instruction bus timing) | weeks 2, 6 |
| Video | scanline-granular; registers re-read every line | weeks 7–9 |
| Audio DAC | cycle-*timestamped* events, rendered per scanline | week 11 |
| Cassette | cycle-granular FSK edges (the ROM demands it) | week 12 |
| Floppy controller | functional state machine, byte-paced delays | week 13 |
| Serial UART (6551) | byte-granular frames, not bit-serial | week 14 |

Two things in that table are worth pausing on.

First, the fidelity **varies by subsystem**, and the variation is not
arbitrary. The cassette is modelled at cycle granularity while the floppy
controller one row below it is merely functional. Why the asymmetry?
Because BASIC's tape loader demodulates the audio signal by *counting
cycles in a polling loop* — it decides whether a bit was a one or a zero
based on how many times around a loop it got before the input line
flipped. Get the edge timing wrong by ten percent and the ROM decodes
garbage. Its disk driver, by contrast, waits on a status bit and doesn't
care whether the answer arrives in eighty microseconds or a hundred. The
tape needs cycles; the disk needs correctness. So that's what each one
gets.

Second, every choice is **falsifiable**, which is the property that makes
this a budget rather than an excuse. When a real program breaks, you
don't rewrite the emulator — you climb exactly one fidelity rung, in
exactly the place that hurts, and you now have a test case proving it was
necessary. That's the philosophy stated in
[DESIGN.md](https://github.com/sperano/cocovm/blob/main/DESIGN.md) §5,
which says of instruction-granular CPU timing: "Don't try to be
cycle-*exact* mid-instruction at first; instruction-granular cycle counts
are enough to get the ROM booting and sync interrupts roughly right.
Tighten later only if a game needs it."

Accuracy is a budget. Spend it where software can tell the difference.

---

## 1.7 How to study with this book

The syllabus calls the method *archaeology, then surgery*. It's a
four-step loop, and it's worth following in order for every subsystem,
because each step makes the next one safe.

1. **Read the tests before the implementation.** This inverts the
   instinct most people have, and it's the single highest-leverage habit
   in this book. A test named `reset_vector_points_into_rom` states a
   hardware fact in five lines; the implementation of that same fact is
   spread across a decode chain, three constants, and a `match` arm. In
   this codebase the tests *are* the specification — a great many of them
   encode behaviour that was verified against real ROM images, against
   MAME, or against the CoCo 3 service manual, and their names are
   written to say what the hardware does rather than what the function is
   called.
2. **Run them.** `cargo test -p mc6809` this week; add `-p coco-core`
   from week 5 on. Watching two hundred tests go green is not ceremony.
   It establishes the baseline that makes step 3 safe, and it tells you
   your toolchain works before you start changing things.
3. **Break something on purpose.** Flip a flag computation from `>=` to
   `>`. Swap two operands. Delete a line that looks redundant. Run the
   tests again and find out *which* test catches you — and if none does,
   congratulations, you've found a coverage hole, which is worth
   considerably more than a lesson that went smoothly. Several exercises
   in this book are exactly this, and their stated outcomes have been
   verified by actually running them.
4. **Then extend.** Every "build" exercise in this book stands on the
   three steps above. Extending code you haven't broken is how you end up
   with a change you can't debug.

A few practical notes for the labs:

- **The PPM lab bench.** Because the core renders into a plain byte
  buffer, the programs in
  [`crates/coco-core/examples/`](https://github.com/sperano/cocovm/tree/main/crates/coco-core/examples)
  can dump frames straight to `.ppm` image files — no GPU, no window, no
  frontend. PPM is about the simplest image format that exists and every
  image viewer on earth reads it. Weeks 7 through 9 lean on this
  constantly, because "diff two images" beats "squint at a window" every
  time.
- **ROMs are local-only.** The `roms/` directory holds real, copyrighted
  ROM images. It is git-ignored and exists only in the main checkout, so
  fresh clones and git worktrees won't have it. Tests that need a ROM
  either skip themselves or fail loudly when it's missing, and each
  chapter's lab tells you which. The synthetic-ROM tests — the whole CPU
  suite, `bus_map`, `pia_sync`, most of the render tests — need nothing
  but the repository.
- **Keep a trace notebook.** From week 4 onward, the most valuable
  debugging habit available to you is saving instruction traces and
  diffing them: against a reference emulator, or against your own
  last-known-good run from an hour ago. Plain text files and `diff -u`.
  No tooling required, and the first divergent line is almost always the
  bug.

---

## 1.8 Reading assignment

Read these in this order. Each one makes the next more legible, and the
whole assignment is perhaps ninety minutes.

1. **[`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md),
   all of it.** This is the map for the entire course. It is also, and
   unusually, a design document that was written *before* the code and
   then annotated as reality corrected it. Look for the
   "Correction (2026-07…)" notes as you read — the discovery in §6 that
   the CoCo 3 boots into a VDG-compatible mode is a good one to find,
   because it rewrote a chunk of the video plan. Reading a design
   document *with its corrections still attached* is a rare chance to
   watch a design survive contact with hardware, and it teaches far more
   than a tidy retrospective document that pretends the first guess was
   right.
2. **[`crates/mc6809/src/lib.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/lib.rs#L1-L199),
   lines 1–199.** This gets you the `Bus` trait, the condition-code bit
   masks, the interrupt vector constants, the `MC6809` struct, and
   `reset()`. Everything in this chapter's §1.3 lives here, plus a
   preview of week 2 and week 4. Resist the urge to chase into
   [`exec.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec.rs)
   — that's next week's chapter, and it's better with the setup.
3. **[`crates/coco-core/src/machine.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine.rs#L61-L135),
   lines 61–135.** The `Machine` struct in full. You now know enough to
   read every doc comment on it, including the ones that reference weeks
   you haven't had yet — that's what the rest of the course is for, and
   seeing the names in advance is useful. Pay attention to how many
   fields are `#[serde(skip)]` and why each one gives its reason.

While you're reading, run the CPU test suite and watch two hundred tests
pass with no machine attached anywhere. It's the standalone-CPU claim
from §1.5, demonstrated in about four seconds:

```
cargo test -p mc6809
```

---

## 1.9 Exercises

**1.1 — The fetch-execute rhythm (build).** In a scratch project — not
this repository — write the smallest emulator that can honestly be
called one: a `FlatBus` holding `[u8; 65536]`, a CPU struct with nothing
but a `pc`, and a `step()` that implements exactly two instructions,
`NOP` (`$12`, 2 cycles) and `JMP` extended (`$7E hh ll`, 4 cycles). Here
is the skeleton:

```rust
trait Bus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
}

struct FlatBus([u8; 65536]);
impl Bus for FlatBus { /* ... */ }

struct TinyCpu { pc: u16, cycles: u64 }

impl TinyCpu {
    fn step(&mut self, bus: &mut impl Bus) -> u32 {
        let op = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        match op {
            0x12 => { /* NOP */ 2 }
            0x7E => { /* JMP: read 16-bit big-endian target, set pc */ todo!() }
            _ => panic!("unimplemented opcode ${op:02X}"),
        }
    }
}
```

Load the three-instruction program `NOP / NOP / JMP $1000` at `$1000`,
run 3000 steps, and assert that the cycle count is exactly what you
predicted — and predict it by hand *first*, before you run it. The point
of this exercise is not the code, which is about forty lines. The point
is feeling the rhythm every later chapter is built on: the opcode fetch
advances `PC` before execution begins, the operand fetch advances it
further, and the jump overwrites whatever it had become.

**1.2 — Break the endianness (sabotage).** In your toy from 1.1, swap
the operand bytes of `JMP` so that the low byte is read first. Where does
the CPU land, and why is this *the* classic bug when porting an emulator
between processor families? Then go find the one place in the `mc6809`
crate that makes this mistake impossible to write a second time. (Hint:
it's a default method.)

**1.3 — Fight the borrow checker on purpose (read + prove).** Take the
"design that does NOT work" struct from §1.4, give it a stub
`fn step(&mut self)` whose body is `self.cpu.step(&mut self)`, and
compile it. Read the error message carefully and slowly — you want to be
able to recognize E0499 on sight, because you'll meet it again the first
time you try to add a device the wrong way. Then fix it the way this
codebase does, by partitioning into `cpu` plus `bus`, and watch it
compile. Two structs, one lesson, ten minutes.

**1.4 — Map the map (recall).** From memory, write down what lives at
`$FF00`, `$FF20`, `$FF40`, `$FF90`, `$FFA0`, `$FFB0`, and `$FFFE`, then
check yourself against the table in §1.2. You will be using these
addresses weekly for the rest of the course. `$FF90` and `$FFA0` in
particular should end up as familiar as `$0400`, the address of the
top-left character cell of the text screen.

**1.5 — Why not `&self`? (essay, three sentences maximum).** A friend
looks at the `Bus` trait and proposes an improvement: "Make `Bus::read`
take `&self` and use a `Cell` inside the PIA for the flag-clearing case.
Then reads are pure in the common case, and the debugger's peek problem
disappears entirely." Give the two strongest reasons this codebase
rejects that design. One of them is about the honesty of a type
signature; the other is about something week 16 needs. If you find a
third — what does `Cell` fail to compose with, given the derive on
`Machine`? — you're ahead of the class.

**1.6 — Grow the toy (build).** Extend the CPU from 1.1 with three more
instructions and an `a: u8` register: `LDA` immediate (`$86 nn`, 2
cycles), `LDA` extended (`$B6 hh ll`, 5 cycles), and `STA` extended
(`$B7 hh ll`, 5 cycles). Then write the four-instruction program that
copies one byte from `$0400` to `$0500`, and assert on both the copied
byte and the exact total cycle count. You have just written a memory-move
routine one instruction shy of the real thing — and you'll find these
exact opcodes, with these exact cycle counts, in
[`exec_data.rs`](https://github.com/sperano/cocovm/blob/main/crates/mc6809/src/exec/exec_data.rs)
next week.

**1.7 — Find the fidelity line (read).** Pick the cassette row and the
floppy-controller row out of the table in §1.6. Skim the module header
comments of
[`crates/coco-core/src/cassette.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cassette.rs)
and
[`crates/coco-core/src/wd1773.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773.rs)
— headers only; the guts are weeks 12 and 13. Each one states its
fidelity choice and its reason in the first comment block. Write down, in
one sentence each, *what piece of 1980s software forced* that choice. The
habit of asking "who notices?" is this week's real deliverable, and it
will save you more time over the next fifteen weeks than any single
technique in the book.

**1.8 — One-way arrows (recall + verify).** From §1.5: which crate
depends on which? Verify your answer mechanically rather than from
memory — each crate's `Cargo.toml` has a `[dependencies]` section that
takes ten seconds to read. Then answer the interesting question: if you
wanted to reuse the `mc6809` crate in a Vectrex emulator, which was also
a 6809 machine, what exactly would you need to bring along with it? The
answer should be pleasingly short, and its shortness is the entire point
of §1.3.

---

## What's next

Next week we open `MC6809::step()` and then stay inside the CPU for three
solid weeks. Week 2 is the pleasant part: registers, flags, and the
dispatch `match` where every opcode you've ever hand-assembled has a line
of Rust with its name on it. You'll see why `D` isn't a field, why the
condition codes are a byte instead of eight booleans, and how a
thousand-line `match` on a `u8` turns out to be the clearest possible way
to write a CPU.

Bring your 6809 instruction-set card. We'll be checking the emulator
against it, not the other way around.
