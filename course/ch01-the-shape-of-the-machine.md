# Chapter 1 — The Shape of the Machine

*Week 1. Goal: understand the whole emulator's shape before touching any
chip. By the end of this chapter you will know what an emulator actually
is (it's smaller than you think), how the CoCo 3's chips divide up the
work, and the two abstractions in this codebase that everything else
hangs off — one of which exists because of the 6809, and one of which
exists because of Rust.*

---

## 1.1 What an emulator actually is

Strip away the mystique first. An emulator is three things:

1. **State.** A handful of plain variables that mirror the registers and
   memories of the real chips. The entire CPU of the machine you grew up
   with is this struct — this is real code from `crates/mc6809/src/lib.rs`,
   not a simplification:

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

   You already know every one of those fields from writing 6809 assembly.
   That's the whole CPU: fourteen registers' worth of bytes, a cycle
   counter, and two bookkeeping fields we'll get to in week 4. There is no
   magic under this struct — no microcode, no hidden simulation engine.

2. **A loop.** Fetch the byte at `PC`, decide what instruction it is, do
   what the data sheet says that instruction does to the state, add the
   instruction's cycle cost to a counter. Repeat, forever. When people say
   "emulator," ninety percent of the time they mean this loop.

3. **A seam.** The CPU has to touch the outside world — RAM, ROM, the
   keyboard, the video chip. Every one of those touches goes through a
   single narrow interface (in this codebase, a two-method trait). That
   seam is the most important design decision in the whole project, and
   §1.3 is devoted to it.

That's it. State, loop, seam. The remaining ~50,000 lines of this
repository are what happens when you take each device on the other side
of the seam seriously — and the point of this course is that each of
those devices is *also* just state, a loop, and a seam, all the way down.

Take one example on faith for now (week 12 delivers the details): the
cassette interface. Its **state** is a decoded byte stream, a playback
position, and a motor flag. Its **loop** is "every N cycles, the current
bit's FSK tone flips the input line." Its **seam** is a single bit that
PIA1 hands to the CPU when the ROM polls it. A tape deck — motor
physics, tone frequencies, the ROM's own demodulation algorithm — and it
reduces to the same three-part shape as the CPU. When you face a new
device in this course, your first question should always be: *what's the
state, what's the loop, where's the seam?*

### Interpreting, not translating

This emulator is an **interpreter**: every time the 6809 would execute
`LDA $0400`, we re-decode the opcode `$B6` and re-dispatch. A JIT
(just-in-time translator) would instead compile that instruction into
host machine code once and jump to it directly on every later execution.
JITs are how you emulate a PlayStation 2; they are wildly unnecessary
here. The CoCo 3's CPU runs at 0.895 MHz (1.79 MHz after the famous
speed poke). Your laptop executes roughly *ten thousand* host
instructions in the time the CoCo executes one. An interpreter spending
50 host instructions per emulated instruction leaves a 99%+ idle margin
— and it stays readable, debuggable, and steppable, which a course (and
a debugger, see week 16) cares about far more than headroom we'll never
use.

### Cycles are the currency

One habit to build immediately: emulator code doesn't think in seconds,
it thinks in **CPU cycles**. Every instruction costs a documented number
of cycles (`LDA` extended: 5). Video, audio, tape, and disk timing are
all downstream conversions from the cycle count. When week 6 builds the
timing loop, "run one scanline" will literally mean "run instructions
until ~57 cycles have elapsed." Notice that `cycles: u64` sits right in
the CPU struct — it is not debug decoration; it is the machine's clock.

---

## 1.2 A tour of the machine you owned

You know what the CoCo 3 *does*. Here is who actually does it. Five
chips matter (everything else on the board is glue):

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

- **MC6809E** — the CPU. You know this one. The only chip in the machine
  you already understand from the inside.
- **GIME** (Graphics Interrupt Memory Enhancement, the big custom chip
  Tandy added for the CoCo 3) — three devices in a trench coat:
  - an **MMU** that maps the CPU's 64K view onto 128K–2MB of physical
    RAM in 8K blocks (this is how BASIC, your program, and a hi-res
    screen coexisted in a "64K" address space);
  - the **video scanout** hardware, replacing the CoCo 1/2's MC6847 VDG
    while remaining able to imitate it (that imitation mode is what the
    text screen you booted into every day actually was — week 7 has the
    receipts);
  - an **interrupt controller and 12-bit timer**, plus a compatibility
    layer that answers to the old SAM chip's addresses.
- **Two MC6821 PIAs** (Peripheral Interface Adapters) — dumb, general
  purpose parallel ports that Tandy wired to everything cheap: the
  keyboard matrix, the joystick comparator, the 6-bit sound DAC, the
  cassette line, the printer bit-bang line. When your BASIC program did
  `PRINT PEEK(65280)`, it was reading PIA0.
- **RAM and ROM** — 32K of Super Extended Color BASIC in ROM, RAM behind
  the MMU.
- **The cartridge port** — a raw extension of the bus. A disk controller
  is not special hardware to the CoCo; it's just a cartridge that
  decodes a few addresses and yanks two interrupt lines.

One address map ties the whole course together. The 6809 sees 64K; the
top 256 bytes, `$FF00–$FFFF`, are the **I/O page**, where every device
lives. You'll internalize this table in week 5, but here's the skyline
view — worth a bookmark now:

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

If you ever POKEd `65497` for double speed: that's `$FFD9`, one of those
SAM-compatibility strobes. By week 6 you'll know exactly what it does to
the emulator's main loop (spoiler: it changes one integer).

### Why the GIME answers to a dead chip's addresses

That last row of the table — "SAM-compatibility strobes" — deserves a
word, because it explains a pattern you'll meet all through this course.
The CoCo 1 and 2 didn't have a GIME; they had two separate chips: the
**MC6883 SAM** (Synchronous Address Multiplexer — memory control, video
addressing, CPU speed) and the **MC6847 VDG** (Video Display Generator —
the actual character and graphics output). When Tandy built the CoCo 3,
the GIME swallowed both jobs. But thousands of programs — including the
BASIC ROM itself — were already POKEing the SAM's registers at
`$FFC0–$FFDF` and flipping the VDG's mode bits through PIA1. So the GIME
keeps answering at the old addresses, imitating the old chips' behavior.

The codebase mirrors the silicon's family history precisely: a real
`Sam` type (`crates/coco-core/src/sam.rs`) is used *only* for emulated
CoCo 1/2 machines, while the CoCo 3 path routes the same addresses into
the GIME's own compatibility layer
(`crates/coco-core/src/gime/sam_compat.rs`). Two implementations of one
legacy interface — because that's what Tandy shipped. Backward
compatibility is not a footnote in this machine; it is *why the CoCo 3
boots into a 1980 video mode* (week 7) and why half the GIME's register
map exists at all.

One more number worth decoding while we're here: the odd CPU clock,
0.895 MHz. The exact value in the code is 894,886 Hz (`CPU_HZ`,
`crates/coco-core/src/machine.rs:26`) — the NTSC color subcarrier
(3.579545 MHz) divided by 4, truncated. Like almost every home computer
of its era, the CoCo derives *everything* — CPU, video timing, even
cassette baud rates — from one crystal chosen for television
compatibility. That single shared clock is why week 6 can drive the
whole machine off one cycle counter.

---

## 1.3 Load-bearing abstraction #1: the `Bus` trait

Here is the seam, in full, from `crates/mc6809/src/lib.rs:29`:

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

Two methods. The CPU crate knows *nothing* else about the outside world
— not the memory map, not the GIME, not that a CoCo exists at all. Every
`LDA`, every stack push, every vector fetch goes through `read`/`write`.

Three deliberate decisions are packed into those ten lines. Each one is
worth understanding, because each one gets cashed in later in the course.

### Decision 1: `read` takes `&mut self`

This looks wrong at first. Reading memory doesn't change anything…
right? **On real hardware, it does.** Two examples you'll meet soon:

- Reading a PIA data register **clears that PIA's interrupt flag**
  (week 10). The ROM's 60 Hz interrupt handler *depends* on this: it
  reads `$FF02` precisely to acknowledge the interrupt.
- Reading the GIME's IRQ status register `$FF92` returns *and clears*
  the pending-interrupt bits (week 8).

If `read` took `&self`, you could not model any of that without interior
mutability tricks (`Cell`, `RefCell`) scattered through every device. By
making mutation part of the seam's contract from day one, every device
is free to have honest read side effects with no ceremony. When a read
must *not* have side effects — the debugger's memory viewer hovering
over `$FF02` had better not eat a pending interrupt — the codebase
provides a separate, explicitly side-effect-free `peek()` path
(`crates/coco-core/src/bus/peek.rs`, week 16). Two functions, two
contracts, both visible in the type signatures.

### Decision 2: the CPU is *generic* over the bus

The CPU's step function has this shape (we'll dissect its body next
week):

```rust
impl MC6809 {
    pub fn step(&mut self, bus: &mut impl Bus) -> u32 { /* fetch, decode, execute */ }
}
```

`impl Bus` means the CPU works against *any* type implementing the
trait. The payoff is enormous and immediate: the `mc6809` crate's ~200
tests run the CPU against a `FlatBus` — a bare 64K array with no CoCo
attached:

```rust
// crates/mc6809/tests/common/mod.rs — the whole test machine
pub struct Sys { pub cpu: MC6809, pub bus: FlatBus }

let mut sys = Sys::code(0x1000, &[0xB6, 0x04, 0x00]);  // LDA $0400
sys.step();
```

The real machine implements the same trait on `SystemBus` (all the
devices, week 5). Same CPU code, byte for byte, in both worlds. When a
CPU test fails you *know* it's the CPU, because there is no machine in
the room.

> **Rust corner: monomorphization, or why this costs nothing.**
> `fn step(&mut self, bus: &mut impl Bus)` is generic, and Rust compiles
> generics by *monomorphization*: it emits a separate, fully concrete
> copy of `step` for each bus type actually used — one compiled against
> `FlatBus`, one against `SystemBus`. Inside each copy, `bus.read(...)`
> is a direct (usually inlined) call. The alternative — storing
> `&mut dyn Bus`, a *trait object* — would route every single memory
> access through a vtable pointer at runtime. On the hottest path in the
> entire program (memory access happens several times per instruction),
> we get the abstraction for free. The cost is paid at compile time and
> in binary size, not at run time. When you see `impl Trait` in this
> codebase, read it as "resolved at compile time."

> **Rust corner: default methods.** `read_u16`/`write_u16` have bodies
> *inside the trait*. Implementors get them for free (defined once, in
> terms of `read`/`write`) but may override them. Note the 6809 detail
> hiding in there: high byte first — the 6809 is big-endian, and getting
> this wrong produces an emulator that fetches every vector and every
> 16-bit operand byte-swapped. Also note `wrapping_add`: address
> arithmetic must wrap at `$FFFF` → `$0000`, and Rust's default `+`
> would panic in debug builds instead. Every address computation in the
> codebase uses the `wrapping_*` family; treat a bare `+` on a `u16`
> address as a bug when you read emulator code.

> **Rust corner: `#![forbid(unsafe_code)]`.** The very first line of
> code in `crates/mc6809/src/lib.rs` (line 11) is
> `#![forbid(unsafe_code)]`. Unlike `#![deny(...)]`, `forbid` cannot be
> overridden further down, even by an `#[allow]` — it is a promise the
> whole crate is checked against: *no pointer tricks anywhere in the
> CPU.* An emulator is exactly the kind of program where C tradition
> reaches for casts and aliasing; this crate stakes out the opposite
> position, and nothing in it has ever needed to walk that back. When you
> write your own core (exercise 1.1 and onward), start with the same
> line — it turns a class of emulator bugs into compile errors.

### Decision 3: the seam is *tiny*

No `fetch_opcode`, no `dma_transfer`, no `get_keyboard`. Everything is a
byte at an address, because that is all the real chip's 16 address pins
and 8 data pins could express. Fidelity to the hardware interface keeps
the abstraction honest: if the real 6809 couldn't do something in one
bus transaction, our CPU can't either.

---

## 1.4 Load-bearing abstraction #2: the borrow-checker strategy

This is the section where Rust shapes the architecture, and it's the
part most first-time emulator authors get wrong in Rust — usually
discovering the problem three weeks in, with a half-built machine that
won't compile.

### The problem

Think about ownership for a second. The obvious design is a `Machine`
that owns everything, flat:

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

Now try to run an instruction. The CPU needs to mutate itself (`&mut
self.cpu`) *and* it needs the bus — which is the rest of the machine —
as `&mut` too (remember Decision 1: reads mutate). So you write
`self.cpu.step(&mut self)`… and the borrow checker stops you cold:
`self` is already mutably borrowed through `self.cpu`. You cannot hand
out a second `&mut self` that overlaps the first. And the checker is
*right*: through that second borrow, the CPU could reach into
`self.cpu` and alias itself.

The traditional C design — every device holds a pointer back to the
machine — is exactly the aliasing Rust exists to reject. Most people's
first workaround is to wrap every device in `Rc<RefCell<…>>` and move
the borrow checking to run time. It compiles. It also litters every
device interaction with `.borrow_mut()`, turns aliasing bugs into
run-time panics, and — the quiet killer — makes the machine a graph of
shared pointers instead of a tree of values, which poisons
serialization. Hold that thought for two paragraphs.

### The fix: partition the state along the borrow

The solution in this codebase is structural, from
`crates/coco-core/src/machine.rs:61`:

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
    #[serde(skip)]
    pub framebuffer: Vec<u8>,
    // ... scanline counters, audio buffer (weeks 6 and 11)
}
```

`SystemBus` is "everything that isn't the CPU": RAM, ROM, GIME, both
PIAs, the cartridge, keyboard, cassette (see the full struct at
`crates/coco-core/src/bus.rs:37`). Now the step is:

```rust
self.cpu.step(&mut self.bus)
```

> **Rust corner: `Box<[u8]>`, not `Vec<u8>`.** Look at how `SystemBus`
> stores memory: `ram: Box<[u8]>` and `rom: Box<[u8]>`
> (`crates/coco-core/src/bus.rs:46,52`). A `Vec<u8>` would also work —
> so why the less common type? A boxed slice is a `Vec` with the
> *growability removed*: its length is fixed at allocation, there is no
> spare capacity field, and no code path can ever `.push()` onto it. RAM
> size is decided once, at construction (128K, 512K, or 2MB), and the
> type now enforces what the hardware guarantees — memory doesn't grow
> at runtime. This is the same philosophy as the `cc` module of
> constants in week 2: pick the type that says exactly what the hardware
> does, no more. When you see `Box<[u8]>` in this codebase, read it as
> "a buffer whose size is a *decision*, not a variable."

and the borrow checker is *happy*, because Rust can see that `self.cpu`
and `self.bus` are **disjoint fields** — borrowing them mutably at the
same time aliases nothing. No `Rc`. No `RefCell`. No unsafe. The
machine is a plain tree of owned values, and one struct boundary placed
exactly along the borrow line makes the whole architecture compile.

That's the design rule to take away: **in Rust, you partition state by
who needs to borrow what simultaneously — not by what "belongs
together" conceptually.** On paper, the CPU and the GIME are peers; in
the struct layout, the CPU is one field and the GIME is nested a level
down, purely because of who borrows whom.

### The same trick, one level down

The pattern recurs inside the bus. During video scanout (week 8) the
renderer needs the GIME's registers, the RAM it scans out of, and the
framebuffer it paints into — three references, two of them living
inside `self.bus`, all at once. Same solution, one level down: borrow
disjoint *fields* rather than the whole struct. Real call, from
`crates/coco-core/src/machine/render.rs:56`:

```rust
gime_video::paint_scanline(
    &self.bus.gime,        // shared borrow of one bus field
    &self.bus.ram,         // shared borrow of another
    scan,
    blink_on,
    row,
    &mut self.framebuffer, // mutable borrow of a Machine field
);
```

Three simultaneous borrows into `self`, zero conflicts, because each
names a distinct field path and the free function `paint_scanline`
receives exactly the pieces it needs — instead of a method on `Machine`
taking all of `&mut self` and re-triggering the original problem. When
a whole subsystem needs many fields, the same idea scales via
destructuring: `let SystemBus { gime, ram, .. } = &mut self.bus;` splits
one struct into several independent borrows. You will see both forms
throughout the codebase; they are the reason it contains no interior
mutability at all.

### Why this obsession pays off: save states

Here's the held thought. Because the machine is a plain owned tree —
no `Rc`, no `RefCell`, no back-pointers — this one derive line on
`Machine`:

```rust
#[derive(Serialize, Deserialize)]
```

makes the **entire machine state serializable**. Freeze the CoCo
mid-instruction, write it to disk, restore it next week: that's the
`.ccstate` save-state feature (week 16), and it falls out of the
ownership discipline nearly for free. A `Rc<RefCell<…>>` graph would
have made serde somewhere between painful and impossible (shared nodes
serialize as duplicates, cycles don't serialize at all). The
architecture decision of week 1 quietly purchased the flagship feature
of week 16 — this is the single best example in the codebase of an
early constraint paying compound interest.

> **Rust corner: `#[serde(skip)]`.** Look back at the `framebuffer`
> field. It's marked `#[serde(skip)]`: don't write it into save states,
> and on load, fill it with `Default::default()`. Why skip it? It's
> *derived* state — the next rendered field repaints every pixel from
> RAM and GIME registers, so persisting it would bloat every snapshot
> for nothing. Deciding "essential vs derived" for every field is
> exactly the kind of judgment you'll practice in week 16; the comments
> on the skipped fields in `machine.rs:70-95` show the reasoning
> spelled out per field.

---

## 1.5 Why three crates

The workspace (`Cargo.toml` at the repo root) splits the project into
three crates, and the boundaries are the two seams you just learned:

```
crates/
├─ mc6809/      the CPU. Depends on nothing. Knows only the Bus trait.
├─ coco-core/   the machine: SystemBus, GIME, PIAs, timing, media. Headless.
└─ coco-egui/   the frontend: window, texture, sound device, debugger UI.
```

- **`mc6809` is standalone** because the `Bus` trait makes it possible
  and testing makes it necessary: CPU bugs are the hardest emulator bugs
  to find (week 4), so the CPU must be testable with zero machine
  attached.
- **`coco-core` is headless** — it renders into a `Vec<u8>`, pushes
  audio into a `Vec<[f32; 2]>`, and never opens a window. This is why
  the repository can have a test that *boots the real BASIC ROM, types
  `PRINT 2+2`, and asserts on the pixels* — in CI, with no GPU
  (`crates/coco-core/tests/coco1_boot.rs:93`; you'll step through it in
  week 6). The frontier between "emulator" and "app" is exactly the
  frontier between "testable in CI" and "needs a human with a monitor."
- **`coco-egui` changes fastest and matters least** — menus, textures,
  keymaps. Keeping it out of the core keeps recompiles cheap and keeps
  UI concerns from leaking into device code.

The dependency arrows only point one way: `coco-egui → coco-core →
mc6809`. The CPU doesn't know the CoCo exists; the machine doesn't know
the screen exists. Every week of this course until week 15 lives
entirely in the first two crates.

---

## 1.6 What counts as "accurate"? Fidelity is a budget

Before you read another line of emulator code, you need a vocabulary
for a question that will come up every single week: *how faithful is
faithful enough?* Emulation fidelity is a spectrum, and every point on
it costs implementation effort and complexity:

1. **Functional** — the device produces the right *results* in the
   right order, on its own schedule. (A disk read returns the right
   bytes after "some" delay.)
2. **Instruction/byte-granular** — results are right *and* time is
   accounted at the granularity of whole instructions or whole bytes.
3. **Cycle-accurate** — every bus cycle happens at the exact cycle the
   real chip would produce it, including mid-instruction.
4. **Gate-level** — you are simulating the netlist. (Nobody in this
   course is doing this; it's how projects like Visual 6502 work.)

The trap for a first-time emulator author is believing that "more
accurate" is always better. It isn't — it's *more expensive*, and
software only notices the difference at specific, discoverable points.
This codebase makes its fidelity choices explicitly, and part of
reading it well is noticing each one and asking "what would break if
this were sloppier? what would it cost to be stricter?":

| Subsystem | Fidelity chosen | Where you'll study it |
|-----------|-----------------|----------------------|
| CPU cycles | instruction-granular (no mid-instruction bus timing) | weeks 2, 6 |
| Video | scanline-granular; registers re-read every line | weeks 7–9 |
| Audio DAC | cycle-*timestamped* events, rendered per scanline | week 11 |
| Cassette | cycle-granular FSK edges (the ROM demands it) | week 12 |
| Floppy controller | functional state machine, byte-paced delays | week 13 |
| Serial UART (6551) | byte-granular frames, not bit-serial | week 14 |

Two things to notice in that table. First, the fidelity varies *by
subsystem* — the cassette is modeled at cycle granularity while the FDC
next to it is functional, because BASIC's tape loader counts cycles and
its disk driver doesn't. Second, every choice is falsifiable: when a
real program breaks, the fix is to climb one fidelity level exactly
where it hurts (that's the DESIGN.md §5 philosophy — "tighten later
only if a game needs it"). Accuracy is a budget. Spend it where
software can tell the difference.

---

## 1.7 How to study with this book

The syllabus calls the method *archaeology, then surgery*. Concretely,
for every subsystem, in this order:

1. **Read the tests before the implementation.** A test like
   `reset_vector_points_into_rom` states a hardware fact in five lines;
   the implementation spreads it across a decode chain. In this
   codebase the tests are the specification — many encode facts
   verified against real ROMs, MAME, or service manuals, and their
   names say what the hardware does.
2. **Run them.** `cargo test -p mc6809` now, `-p coco-core` from week
   5. Watching 200 green tests is not ceremony: it establishes the
   baseline that makes step 3 safe.
3. **Break something on purpose.** Flip a flag computation, swap an
   operand, delete a line. Run the tests again. *Which* test catches it
   — and if none does, you've found a coverage hole, which is worth
   more than a lesson that went smoothly. Several exercises in this
   book are exactly this, with the answers verified.
4. **Then extend.** Every "build" exercise stands on the previous
   three steps.

Practical notes for the labs:

- **The PPM lab bench.** The core renders into a plain byte buffer, so
  `crates/coco-core/examples/` can write frames to `.ppm` image files
  with no GPU and no window. Weeks 7–9 lean on this hard.
- **ROMs are local-only.** The `roms/` directory (real, copyrighted ROM
  images) is git-ignored and lives only in the main checkout — clones
  and worktrees won't have it. Tests that need a ROM either skip or
  fail loudly when it's absent; each chapter's lab says which. Synthetic
  ROM tests (the CPU suite, `bus_map`, `pia_sync`, most render tests)
  need nothing.
- **Keep a trace notebook.** From week 4 on, the single most valuable
  debugging habit is saving instruction traces and diffing them —
  against a reference emulator, or against your own last-known-good
  run. Plain text files, `diff -u`, no tooling required.

---

## 1.8 Reading assignment

In this order — earlier items make later ones legible:

1. **`DESIGN.md`, all of it.** It's the map for the entire course, and
   unusually, it was written *before* the code and then annotated as
   reality corrected it (look for the "Correction (2026-07…)" notes —
   e.g. the discovery that the CoCo 3 boots into a VDG-compatible mode,
   §6). Reading a design document *with its corrections* is a rare
   chance to watch design survive contact with hardware.
2. **`crates/mc6809/src/lib.rs`, lines 1–199** — the Bus trait, the CC
   bit masks, the vector constants, the `MC6809` struct, and `reset()`.
   Don't chase into `exec.rs` yet; that's next week.
3. **`crates/coco-core/src/machine.rs`, lines 61–135** — the `Machine`
   struct. You now know enough to read every doc comment on it, even
   where it references weeks you haven't had (that's what the course
   is for).

While reading, run the CPU test suite and watch 200 tests pass with no
machine attached — the standalone-CPU claim, demonstrated:

```
cargo test -p mc6809
```

---

## 1.9 Exercises

**1.1 — The fetch-execute rhythm (build).** In a scratch project (not
this repo), write the smallest possible emulator: a `FlatBus` holding
`[u8; 65536]`, a CPU struct with just `pc`, and a `step()` that
implements exactly two instructions — `NOP` (`$12`, 2 cycles) and `JMP
extended` (`$7E hh ll`, 4 cycles). Skeleton:

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

Load `NOP / NOP / JMP $1000` at `$1000`, run 3000 steps, and assert the
cycle count is exactly what you predict (compute it by hand first). The
point is not the code — it's ~40 lines — but feeling the rhythm every
later chapter builds on: *fetch advances PC before execute; the operand
fetch advances it further; the jump overwrites it.*

**1.2 — Break the endianness (sabotage).** In your toy from 1.1, swap
the operand bytes of `JMP` (read low byte first). Where does the CPU
land, and why is this *the* classic port-an-emulator bug? Now find the
one place in `mc6809` that makes this mistake impossible to write
twice. (Hint: it's a default method.)

**1.3 — Fight the borrow checker on purpose (read + prove).** Take the
"design that does NOT work" struct from §1.4, give it a stub
`fn step(&mut self)` that calls `self.cpu.step(&mut self)`, and read
the compiler error carefully — you want to recognize E0499 on sight.
Then fix it the codebase's way (partition into `cpu` + `bus`) and watch
it compile. Two structs, one lesson, ten minutes.

**1.4 — Map the map (recall).** From memory, write down what lives at
`$FF00`, `$FF20`, `$FF40`, `$FF90`, `$FFA0`, `$FFB0`, and `$FFFE`.
Check against the table in §1.2. You'll use these addresses weekly for
the rest of the course; `$FF90` and `$FFA0` especially should become as
familiar as `$0400` was when you were twelve.

**1.5 — Why not `&self`? (essay, three sentences max).** A friend
proposes "make `Bus::read` take `&self` and use `Cell` inside the PIA
for the flag-clearing case; then reads are usually pure and the
debugger problem disappears." Give the two strongest reasons this
codebase rejects that design. (One is about honesty of the type
signature; one is about what week 16 needs. If you find a third —
`Cell` doesn't compose with what derive? — you're ahead of the class.)

**1.6 — Grow the toy (build).** Extend your CPU from 1.1 with three more
instructions: `LDA immediate` (`$86 nn`, 2 cycles), `LDA extended`
(`$B6 hh ll`, 5 cycles), and `STA extended` (`$B7 hh ll`, 5 cycles), plus
an `a: u8` register. Now write the four-instruction program that copies
one byte from `$0400` to `$0500` and assert both the copied byte and the
exact total cycle count. You have just written a memory-move one
instruction shy of the real thing — and you'll find these exact opcodes,
with these exact cycle counts, in `exec_data.rs` next week.

**1.7 — Find the fidelity line (read).** Pick the cassette row and the
FDC row from the table in §1.6. Skim the module headers of
`crates/coco-core/src/cassette.rs` and `crates/coco-core/src/wd1773.rs`
(headers only — the guts are weeks 12–13). Each one states its fidelity
choice and its reason in the first comment block. Write down, in one
sentence each, *what piece of 1980s software forced* the choice. The
habit of asking "who notices?" is the week's real deliverable.

**1.8 — One-way arrows (recall + verify).** From §1.5: which crate
depends on which? Verify your answer mechanically — each crate's
`Cargo.toml` `[dependencies]` section takes ten seconds to read. Then
answer: if you wanted to reuse the `mc6809` crate in a Vectrex emulator
(also a 6809 machine!), what would you need to bring along? (The answer
should be pleasingly short, and it is the whole point of §1.3.)

---

## What's next

Next week we open `MC6809::step()` and stay inside the CPU for three
weeks. Week 2 is the pleasant part — registers, flags, and the dispatch
`match` where every opcode you've ever hand-assembled has a line of
Rust with its name on it. Bring your 6809 instruction-set card: we'll
be checking the emulator against it, not the other way around.
