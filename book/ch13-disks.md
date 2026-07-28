# Chapter 13 — Disks: the WD1773 State Machine, and Three Ways to Store Bytes

*Week 13. Goal: device protocol emulation in the large. Every chapter since
week 5 has been about hardware the CoCo *had*: chips soldered to the board,
with a fixed job. This week is different — it's about a *job* (get a
256-byte sector from a spinning disk, or from something pretending to be
one, into the CPU's hands) implemented three separate times in this
codebase, by three devices that share nothing except the job. By the end
you'll know the WD1773 floppy controller's command state machine well
enough to trace a sector read byte by byte, you'll finally see the payoff
of week 6's HALT-before-interrupt promise, and you'll have a vocabulary —
real chip, register interface, wire protocol — for classifying every
storage device an emulator author is likely to meet afterwards.*

---

Every device in this course so far has been self-contained in a
particular way. Give the 6809 a clock and a bus and its behaviour
follows. Give the GIME a register file and a block of RAM and it will
paint a screen. Even the cassette deck of week 12, for all its analogue
trappings, turns out to be a waveform computed from an array of bytes the
emulator already holds. Storage breaks that pattern, because the bytes
have to come from somewhere the emulated machine cannot see: a file on a
host operating system that postdates the hardware by decades. Building
that bridge is this week's work, and the interesting part is that the
CoCo world never settled on one way to do it.

Most emulator codebases have exactly one storage path, which makes its
design choices read like *the* way to do it rather than like choices at
all. This one has three, and the differences between them are not
accidents of authorship. Section 13.1 turns those differences into a
vocabulary, and the rest of the chapter earns it a line of code at a
time.

There is a second reason this chapter is the longest in the book. The
WD1773 is the first device in the course that can *stop the CPU*. Every
chip so far has been a passive participant in the machine's timing —
asked for a byte, told about a scanline, handed a cycle count. The
floppy controller reaches back through the cartridge port and holds the
6809's HALT\* pin low until it has a byte ready, which means the disk
story cannot be told inside one module. It spans the controller chip, a
latch on the cartridge that is not part of that chip at all, a pair of
control lines, and one specific ordering rule buried in the machine's run
loop. Correctness lives in the seams between those four things, and week
6 deliberately left the last of them unexplained. Section 13.7 collects
the debt.

The practical news is better than that sounds. The command state machine,
the byte pacing, the image geometry and the format-stream parser are all
exercised by tests that need nothing but this repository — no ROM images,
no disk images — so most of §13.13's lab work runs in any checkout.

---

## 13.1 Three philosophies for one job

Strip away the acronyms and every storage device this chapter covers does
exactly one thing: hand the CPU 256 bytes it asked for, identified by some
address, eventually. That's it. A floppy disk, a hard disk image, and a
network socket to a PC all reduce to the same contract. What differs is
*how much of 1980s reality* each device's protocol makes the software
negotiate — and this codebase happens to implement all three points on
that spectrum, side by side, for the same machine:

1. **The WD1773** ([`crates/coco-core/src/wd1773.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773.rs) +
   [`wd1773/command.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/command.rs) + [`wd1773/transfer.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/transfer.rs)) is a real chip Western
   Digital sold in 1980. Software talks to it the way you'd talk to any
   piece of hardware with inertia: write a command byte, wait for a
   status bit, handle bytes one at a time as they become physically
   available, at a pace the media dictates. There is no "give me the
   sector" instruction — there is a *state machine* you drive one register
   access at a time, and if you show up late for a byte, the byte is
   gone. Emulating it means emulating the state machine, not just the
   sector.
2. **VHD** ([`crates/coco-core/src/vhd.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/vhd.rs)) is not a real chip — MAME
   invented it as an emulator-native shortcut, and NitrOS-9's `emudsk`
   driver was written specifically to exploit it. It has no timing, no
   command dispatch beyond a single byte, no byte-at-a-time handshake.
   You write a 24-bit sector number and a buffer address into seven
   registers, write one command byte, and the *entire* sector has already
   moved. Zero ceremony, because nothing on the other end has physical
   inertia to model.
3. **DriveWire** ([`crates/coco-core/src/drivewire.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire.rs) +
   [`drivewire/protocol.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire/protocol.rs) + [`drivewire/transfer.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire/transfer.rs)) isn't local hardware
   at all. It's an RPC protocol carried over a two-byte "Becker port" at
   `$FF41`/`$FF42`, originally designed to let a real CoCo talk to a *PC*
   holding the actual disk images over a serial cable (today, in this
   emulator, the "PC" is just in-process Rust, but the protocol doesn't
   know that). Because the two ends can't see each other's state, every
   transaction needs framing, a checksum, and a timeout — problems neither
   the WD1773 nor VHD have to solve, because on real hardware, the WD1773
   *is* physically wired to the drive, and VHD is a polite fiction that
   assumes the same.

Put them in one sentence: the WD1773 emulates *inertia* (a real chip with
real timing constraints), VHD emulates *nothing* (a register file with
teleportation), and DriveWire emulates *distance* (a conversation that can
be misheard). Every storage device you will ever add to an emulator — a
CD-ROM, a cartridge flash chip, a modern USB mass-storage class handler —
is a point somewhere in that same triangle. Once you've read this chapter,
you'll be able to say which corner it's closest to before you write a line
of code, and that answers most of the hard design questions up front:
does it need a byte-pacing timer? Does it need a checksum? Does it need a
timeout state?

One more thing worth noticing before we go byte-by-byte: these three
devices don't even plug into `SystemBus` the same way, and that
difference *is* the taxonomy made structural. The WD1773 lives inside
`DiskCart`, which implements the `Cartridge` trait (`crate::cart`) and
occupies the cartridge port's SCS window, `$FF40`–`$FF7E`
([`crates/coco-core/src/bus/regs.rs:10-18`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/regs.rs#L10-L18)) — to the bus, a floppy
controller is just another expansion-port device, indistinguishable in
principle from a ROM pak. VHD is *not* a `Cartridge` at all: its seven
registers, `$FF80`–`$FF86`, are decoded directly inside `SystemBus`'s own
`io_read`/`io_write` ([`crates/coco-core/src/bus/io.rs:74-78,119-125`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L74-L78,L119-L125)),
alongside the GIME's own registers — MAME wired it straight into the
CoCo 3's motherboard I/O decode, no expansion port involved. DriveWire is
stranger still: its two registers, `$FF41`/`$FF42`, sit *inside* the
cartridge port's address range but are intercepted a layer *above*
cartridge dispatch, before a `DiskCart` (or any other cartridge) ever
sees the access — you'll read the exact precedence check in §13.11.
Three devices, three different answers to "how does this reach the CPU,"
and the answer already tells you which corner of the triangle you're in.

---

## 13.2 The WD1773: anatomy of a real chip

Start with the module's own fidelity statement, because it sets the rules
for everything that follows ([`crates/coco-core/src/wd1773.rs:1-14`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773.rs#L1-L14)):
"Modelled functionally rather than cycle-exact: command completion and
byte transfers are paced by `WD1773::tick` against fixed cycle counts
(spec: 'model FUNCTIONALLY, not cycle-exact'), not the real chip's
per-command timing tables. Register semantics (status bit layout, side
effects of register access, command dispatch) are taken from the
verified spec handed to this implementation (MAME `wd_fdc.cpp`/
`coco_fdc.cpp`)."

Two claims worth separating immediately. *Register semantics* — what each
bit means, what side effects a read or write has, which command dispatches
to which behavior — are modeled precisely, cross-checked against MAME's
own WD1773 emulation. *Timing* is not: real seeks take milliseconds and
depend on a step-rate field this emulator never reads; what you get
instead is "a short, deterministic delay, paced by `tick()`." That split
— "this behavior is right" vs. "this behavior takes exactly the right
amount of time" — recurs through the whole chapter, and this codebase is
honest about which promise each pacing constant is making, in a doc
comment attached to the constant itself.

### The four registers, from the CPU's side

Before opening the chip up, it is worth fixing exactly what a 6809 program
can *do* to it, because every disk driver ever written for this machine —
Disk BASIC's DSKCON, NitrOS-9's `rb1773`, every boot loader in between —
is expressed in terms of four addresses and nothing else. `DiskCart`
(§13.8) exposes the WD1773 at four consecutive addresses inside the SCS
window ([`crates/coco-core/src/fdc/disk_cart.rs:77-80`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/disk_cart.rs#L77-L80)):

| Address | A write does | A read does |
|---------|--------------|-------------|
| `$FF48` | load the command register (dispatch) | return the status byte, clearing INTRQ |
| `$FF49` | set the track register | return the track register |
| `$FF4A` | set the sector register | return the sector register |
| `$FF4B` | supply a transfer's next byte | take the staged byte, clearing DRQ |

`$FF48` is two registers sharing one address: write it and you're loading
the *command* register (dispatch — §13.3–13.6); read it and you get the
*status* register, next. `$FF49`/`$FF4A` are the track and sector
registers — plain latches software pokes directly to name a target, no
side effects. `$FF4B` is the data register, and it has real consequences
on every access: writing it during a Write Sector/Write Track transfer
*supplies the next byte*; reading it during a Read transfer *consumes*
the byte the controller just staged and clears DRQ (§13.4).

Two of the four are inert latches, then, and two are ports where the mere
act of touching them advances the machine. That asymmetry is most of the
reason a floppy driver is harder to read than it looks: half its register
accesses are configuration and half are protocol steps, and nothing in the
assembly listing distinguishes them. `STA $FF4A` is bookkeeping, free to
happen whenever the driver gets around to it; `LDA $FF4B` two lines later
is a promise to keep pace with media that will not wait.

The cartridge's side of those four addresses is short enough to read
whole, and what surrounds the `match` matters as much as the arms inside
it ([`disk_cart.rs:238-259`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/disk_cart.rs#L238-L259)):

```rust
    fn write(&mut self, addr: u16, val: u8) {
        match addr {
            DSKREG_BASE..=DSKREG_LAST => self.dskreg = val,
            STATUS_COMMAND_REG => {
                self.fdc.set_double_density(self.dskreg & dskreg::DENSITY_AND_NMI_ENABLE != 0);
                let side = self.side();
                let idx = self.drive_index();
                let disk = selected_disk(&mut self.drives, idx);
                self.fdc.write_command(val, disk, side);
            }
            TRACK_REG => self.fdc.track = val,
            SECTOR_REG => self.fdc.sector = val,
            DATA_REG => {
                let side = self.side();
                let idx = self.drive_index();
                let disk = selected_disk(&mut self.drives, idx);
                self.fdc.write_data(val, disk, side);
            }
            _ => {}
        }
        self.update_lines();
    }
```

Three things happen here that the WD1773 itself knows nothing about, and
each one is a fact about the *cartridge* rather than the chip. Before
every command dispatch, `set_double_density` pushes DSKREG's density bit
into the controller, because on real hardware that bit reaches the chip as
a pin it samples rather than a register it owns — §13.8 has the bit and
§13.6 has the one command whose behaviour turns on it. The selected drive
and side are then resolved *fresh on every single access*, out of DSKREG,
and handed in as ordinary parameters, because a real WD1773 has no idea
which of four drives it is wired to either; that resolution is the
FD-502's job, done in glue logic. And after the `match` runs — whichever
arm it took, including the do-nothing one — `update_lines()` recomputes
the HALT and NMI outputs. That is what makes those two lines *live*
rather than something refreshed on a timer, and §13.7 is entirely about
what the recomputation does.

One more thing `DiskCart` is, which is easy to miss because none of the
four registers hint at it: a ROM pak. The same cartridge serves Disk
Extended Color BASIC — `disk11.rom` — through the expansion port's CTS
window, mirror-filled exactly the way a plain `RomPak` is
([`disk_cart.rs:111-120`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/disk_cart.rs#L111-L120)).
It deliberately does *not* tie the CART\* line to the Q clock the way an
autostart game pak does, and the doc comment on that trait method
explains what stands in its place: BASIC's cold-start code probes
`$C000`/`$C001` looking for the two bytes `'D'`,`'K'` and hands control to
whatever answers
([`crates/coco-core/src/cart.rs:68-78`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cart.rs#L68-L78)).
That probe is why the machine's sign-on message changes the moment a disk
controller is plugged in. The integration test in
[`tests/fdc/boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/boot.rs)
inserts a `DiskCart`, resets, and asserts the screen reads `DISK EXTENDED
COLOR BASIC`, where the cartridge-less boot test in
[`tests/alive.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/alive.rs)
asserts plain `EXTENDED COLOR BASIC`. Same ROM, same reset, one extra
word — because the cold-start code found a signature at a fixed address
and jumped into it.

> **Rust corner: a free function to keep two field borrows apart.**
> Look again at how the `write` arms above reach the selected drive. They
> don't call a method — they call a bare function and pass it a field:
>
> ```rust
> fn selected_disk(
>     drives: &mut [Option<JvcDisk>; DRIVE_COUNT],
>     drive: Option<usize>,
> ) -> Option<&mut JvcDisk> {
>     drive.and_then(|i| drives[i].as_mut())
> }
> ```
>
> The doc comment on it
> ([`disk_cart.rs:64-73`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/disk_cart.rs#L64-L73))
> states the reason outright: it is a free function "so callers can borrow
> `drives` mutably alongside a disjoint mutable borrow of
> `DiskCart::fdc` — going through a `&mut self` method here would make the
> borrow checker see the whole `DiskCart` as borrowed instead of just this
> one field."
>
> That is week 1's partitioning lesson turning up one more level down, and
> in a slightly different disguise. Section 1.4 drew a struct boundary
> between `cpu` and `bus` so that `cpu.step(&mut bus)` could borrow two
> disjoint fields at once. Here the two fields that must be borrowed
> simultaneously are `self.drives` and `self.fdc`, and the line
> `self.fdc.write_command(val, disk, side)` needs both live at the same
> instant — `disk` is a `&mut` into the first, and the method receiver is
> a `&mut` into the second. Rust's borrow analysis on field paths handles
> that without complaint, but only as long as nothing coarsens the borrow
> back up to the whole struct. A hypothetical `self.selected_disk()`
> method would do exactly that coarsening, because its signature says
> `&mut self`, and the compiler believes signatures rather than bodies.
>
> The general rule is worth keeping: when a method's `&mut self` is
> stopping you from using another field, the fix is usually not a
> `RefCell` and never `unsafe` — it is to demote the method to a function
> that names the fields it actually wants. The signature becomes longer
> and the borrow becomes narrower, and narrow borrows are the currency
> this codebase runs on.

### The chip's own state

The Rust struct is a fair inventory of what a real WD1773 datasheet would
list as internal state, plus the bookkeeping an emulator needs on top of
it ([`crates/coco-core/src/wd1773.rs:196-233`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773.rs#L196-L233)):

```rust
pub struct WD1773 {
    pub track: u8,
    pub sector: u8,
    pub data: u8,
    pub busy: bool,
    pub drq: bool,
    pub intrq: bool,
    physical_track: u8,
    last_step_direction: StepDirection,
    last_was_type1: bool,
    status_lost_data: bool,
    status_crc_error: bool,
    status_record_not_found: bool,
    status_write_protect: bool,
    op: Op,
    density_double: bool,
}
```

Two fields are worth pausing on before you read another line, because
they carry the module's most subtle correctness argument.

**`track` vs. `physical_track`.** The WD1773 datasheet distinguishes the
head's *actual* position from the *track register*'s belief about it,
because a Step/Step-In/Step-Out command can move the head without
updating the visible register — that's what the command's "update track
register" (T) bit controls. Read/Write Sector and Read Address always
operate on `physical_track`, the real head position; only Restore and Seek
force `track` to follow it unconditionally (there's no non-updating
variant of either). Software that steps with T clear and later issues a
Read Sector is relying on exactly this split; get it wrong and you'll
silently read the wrong track while `$FF49` still reports the old one.

**`drq`'s reset default is `true`, not `false`.** This looks backwards —
shouldn't "no data ready" be the natural power-on state? — until you read
the doc comment attached to the field:

```rust
    /// Data request line. Reset state is `true` (spec) — see `crate::fdc`'s
    /// HALT-line doc comment for why: DSKREG's halt-enable bit is asserted
    /// asynchronously by boot code before any command has run, and a
    /// default-`false` DRQ would spuriously assert HALT.
    pub drq: bool,
```

Hold that thought — it's the first thread of §13.7's HALT/NMI story, and
it's a perfect example of a fact you cannot derive from the WD1773's own
datasheet: it only makes sense once you know what DSKREG (a *different*
chip, on the FD-502 cartridge, not the WD1773 itself) does with the DRQ
line the instant boot code touches it.

### The status register: two chips' worth of bits in one byte

The status byte is the single most information-dense register in this
whole subsystem, because the WD1773 datasheet reuses bit positions between
command families:

```rust
pub mod status {
    pub const BUSY: u8 = 0x01;
    pub const DRQ: u8 = 0x02;
    pub const INDEX_PULSE: u8 = 0x02;
    pub const TRACK0: u8 = 0x04;
    pub const LOST_DATA: u8 = 0x04;
    pub const CRC_ERROR: u8 = 0x08;
    pub const RECORD_NOT_FOUND: u8 = 0x10;
    pub const RECORD_TYPE: u8 = 0x20;
    pub const WRITE_PROTECT: u8 = 0x40;
    pub const NOT_READY: u8 = 0x80;
}
```

Notice bits 1 and 2 are each defined *twice*, under two names, at the same
value. That's not a mistake — it's the real chip's own design. Bit 1 means
INDEX_PULSE after a Type I command, or DRQ after a Type II/III command;
bit 2 means TRACK0 after Type I, or LOST_DATA after Type II/III. The
meaning depends entirely on what kind of command last ran, which is why
`WD1773` keeps `last_was_type1` around — not as a convenience, but because
without it the status byte is genuinely ambiguous. `status_byte` branches
on exactly that flag: TRACK0 (from `physical_track == 0`) when Type I,
else DRQ and LOST_DATA — with INDEX_PULSE (bit 1 under Type I) left
permanently 0, since Disk BASIC never needs it. The remaining bits are
unambiguous across command types — CRC_ERROR, RECORD_NOT_FOUND (RNF),
WRITE_PROTECT, and NOT_READY (`!disk_present || !motor_on`) always mean
the same thing regardless of `last_was_type1`. `RECORD_TYPE` (bit 5, the
deleted-data-mark bit on a real Type II read) is simply never set — this
emulator doesn't model deleted-data sectors, and the comment says so
plainly rather than pretending otherwise.

Reading the status register has a side effect real software depends on:
it clears INTRQ (`pub fn read_status(&mut self, ...) -> u8 { let s =
self.status_byte(...); self.intrq = false; s }`). This is the exact same
pattern you learned in week 1 and met concretely in week 10 (reading a
PIA's data register clears its interrupt flag): a *read* changes state,
which is precisely why `Bus::read` takes `&mut self`. INTRQ-clear-on-
status-read is how DECB's disk driver acknowledges "yes, I saw the
command finish" without a separate acknowledgment register. `read_data`
has the matching pattern one register over — it clears DRQ, not INTRQ:
`let val = self.data; self.drq = false; val`. Two side-effecting reads,
two different flags, and a driver loop that polls status for BUSY, then
polls (or is halted on) DRQ, then reads data — that loop is the entire
subject of §13.4.

> **Rust corner: `std::mem::replace` as the state-machine ownership
> dance.** Look ahead for a moment at how `advance_transfer` in
> [`wd1773/transfer.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/transfer.rs) gets at the `Transfer` payload sitting inside
> `self.op: Op`:
>
> ```rust
> let Op::Transfer(t) = std::mem::replace(&mut self.op, Op::Idle) else {
>     unreachable!("advance_transfer only called from the Op::Transfer arm");
> };
> ```
>
> Why not just `if let Op::Transfer(ref mut t) = self.op { ... }` and
> mutate in place? Because several of these methods need to *consume* `t`
> by value — pass ownership of its `buf: Vec<u8>` onward, or replace the
> whole `Transfer` with a differently-shaped one for the next sector — and
> Rust will not let you move a value out of a struct field behind a `&mut
> self` reference; the field must stay initialized at every moment, even
> mid-function, in case a panic unwinds through it. `mem::replace` is the
> escape hatch: swap in a placeholder (`Op::Idle`, always cheap to
> construct) and get full, owned access to what was there before. You'll
> see the identical shape in `write_data` (this module) and in
> `DwServer::feed` (§13.11) — any time a Rust state machine needs to
> *consume and replace* its own current state rather than mutate it in
> place, expect this pattern.

---

## 13.3 Type I commands: seeks and the track register dance

The WD1773's entire instruction set is one nibble wide. Sixteen values,
grouped into four families, and that grouping is not merely a way of
organizing a datasheet — it is *state the chip has to keep*, for the
reason §13.2 already established: two bits of the status register change
meaning depending on which family last ran. "What type was the last
command" is therefore a field in the struct rather than a heading in a
manual, and every dispatch path in this section sets it. The place to
start is the family that moves the head, because nothing it does involves
data at all: no DRQ, no byte pacing, no HALT line. Just a target track, a
short delay, and an interrupt when the head gets there.

Command dispatch happens on a write to `$FF48`, and the WD1773 groups its
sixteen possible command-byte top nibbles into four numbered *types*
([`crates/coco-core/src/wd1773/command.rs:15-32`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/command.rs#L15-L32)): Type I is `RESTORE =
0x0`, `SEEK = 0x1`, `STEP[_T] = 0x2/0x3`, `STEP_IN[_T] = 0x4/0x5`,
`STEP_OUT[_T] = 0x6/0x7`; Type II is `READ_SECTOR[_M] = 0x8/0x9`,
`WRITE_SECTOR[_M] = 0xA/0xB`; Type III is `READ_ADDRESS = 0xC`,
`READ_TRACK = 0xE`, `WRITE_TRACK = 0xF`; Type IV (Force Interrupt) is
`0xD` alone.

Type I (`$0`–`$7`) moves the head: Restore, Seek, and the three Step
variants. All five share the same completion shape — a fixed settle delay,
then INTRQ — which is why `write_command`'s top-level dispatch handles
them as one family before falling into type-specific per-command
functions ([`command.rs:51-103`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/command.rs#L51-L103), abridged):

```rust
    pub fn write_command(&mut self, cmd: u8, disk: Option<&mut JvcDisk>, side: u8) {
        let type_nibble = cmd >> 4;
        if type_nibble == cmd_type::FORCE_INTERRUPT {
            self.force_interrupt(cmd);
            return;
        }
        if self.busy {
            return;
        }
        self.busy = true;
        self.status_lost_data = false;
        self.status_crc_error = false;
        self.status_record_not_found = false;
        self.status_write_protect = false;
        match type_nibble {
            cmd_type::RESTORE => self.start_restore(cmd, disk),
            cmd_type::SEEK => self.start_seek(cmd, disk),
            cmd_type::STEP | cmd_type::STEP_T => self.start_step(cmd, None, disk),
            /* STEP_IN/STEP_OUT: start_step with a forced direction */
            /* Type II/III arms: §13.4-13.6 */
            _ => unreachable!("4-bit nibble: all 16 values are matched above"),
        }
    }
```

Three facts worth reading twice here. First: **Force Interrupt (Type IV)
runs even while `busy`** — the `if self.busy { return; }` guard sits
*after* the Force Interrupt check, not before, because Force Interrupt is
how a driver cancels a command that's stuck. Second: **every other command
written while `busy` is simply dropped** — no queue, no error, nothing;
the comment says so ("spec"), and it matches the real chip. Third: the
four status flags are cleared unconditionally at the top of every fresh
dispatch, before the type-specific code runs — a Type I command clears
CRC_ERROR and WRITE_PROTECT just as readily as a Type II one, even though
neither is relevant to a seek, because that's what "starting a new
command" means to the real register. One more habit worth noticing: the
exhaustive `match` ends in `_ => unreachable!("4-bit nibble: all 16
values are matched above")` — `type_nibble` comes from `cmd >> 4` on a
`u8`, so it's provably in `0..16`, and the message is a *proof*, not
defensive programming, that the match is total.

### Restore, Seek, and the settle delay

Restore always drives the head to physical track 0 — the WD1773's
equivalent of "home"; Seek drives it to whatever's already sitting in the
data register. Both functions are nearly identical ([`command.rs:110-128`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/command.rs#L110-L128)):

```rust
    fn start_seek(&mut self, cmd: u8, disk: Option<&mut JvcDisk>) {
        self.last_was_type1 = true;
        let target = self.data;
        self.physical_track = target;
        self.track = target;
        let verify = cmd & type1::VERIFY != 0;
        self.status_record_not_found = verify && !Self::track_readable(disk.as_deref(), target);
        self.op = Op::SettlingTypeOne { remaining: COMMAND_SETTLE_CYCLES };
    }
```

`start_restore` is the same shape with the target hardcoded to `0`
instead of read from `self.data`. Both immediately move `physical_track`
*and* `track` to the target — there is no non-updating variant of either
— and both leave `op` in `Op::SettlingTypeOne`, a fixed 64-cycle delay
(`COMMAND_SETTLE_CYCLES`) before `WD1773::tick` sets `intrq = true` and
returns to `Idle` (quoted in full in §13.7). The V (verify) bit, if set,
checks the target track actually exists on the mounted image —
`track_readable` is nothing more than "is this track number less than
the image's track count" — and sets RECORD_NOT_FOUND if it doesn't, but
the command still completes with INTRQ either way: RNF is a status flag
to be polled, not a different completion path.

### Step, Step-In, Step-Out, and the direction the chip remembers

Bare `Step` (`$2`/`$3`) has no direction bits of its own — it repeats
whichever direction the *last* Step-In or Step-Out used, which is real
WD1773 behavior, not an emulator convenience: `start_step` takes an
`Option<StepDirection>` (`Some` for Step-In/Step-Out, `None` for bare
Step), updates `self.last_step_direction` only when given one, then
always moves by that remembered direction —
`self.physical_track.saturating_add(1)` or `saturating_sub(1)`.

This is where `physical_track` and `track` finally diverge: the head
*always* moves (`physical_track` always updates), but the *visible* track
register only follows if the command's T bit
(`type1::UPDATE_TRACK_REG`, `0x10`) is set. A driver doing a
multi-track seek by hand — the way boot-loader code sometimes does,
avoiding the Seek command's implicit verify cost — can step the head
several times with T clear and only set it on the final step, and this
struct faithfully tracks the difference the whole way.

### Force Interrupt: the command that runs anyway

Type IV has exactly one member, `$D`, and it belongs in this section
rather than alongside the data-transfer commands for a reason the code
states more clearly than any datasheet could: when it finishes, the chip
presents *Type I* status. Force Interrupt is the WD1773's abort button,
and after an abort the sensible thing to report is where the head is, not
what a cancelled transfer was doing
([`command.rs:300-311`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/command.rs#L300-L311)):

```rust
    /// Force Interrupt (Type IV, `0xD`): cancel any command in progress. Low
    /// nibble bit3 (I3) forces an immediate INTRQ; low nibble 0 just cancels.
    /// Runs even while busy, and the status presented afterward is Type-I
    /// style (spec).
    fn force_interrupt(&mut self, cmd: u8) {
        self.busy = false;
        self.op = Op::Idle;
        self.last_was_type1 = true;
        if cmd & type4::IMMEDIATE_INTRQ != 0 {
            self.intrq = true;
        }
    }
```

Four lines of body, and two distinct behaviours packed into them. The
unconditional part is the cancellation: `busy` drops, `op` returns to
`Idle`, and whatever transfer was in flight simply ceases to exist —
there is no partial-completion bookkeeping, no half-written sector to
unwind, because the real chip's abort is equally blunt. The conditional
part is the interrupt. Bit 3 of the low nibble, named `IMMEDIATE_INTRQ`
in [`command.rs:46-49`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/command.rs#L46-L49),
asks for an INTRQ on the spot; `$D0` — Force Interrupt with a zero low
nibble — cancels silently and leaves the interrupt line alone.

Two tests in [`tests/fdc/wd1773.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/wd1773.rs)
pin the two halves separately
([`tests/fdc/wd1773.rs:219-242`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/wd1773.rs#L219-L242)).
`force_interrupt_cancels_a_pending_command` starts a Write Sector, checks
`busy`, cancels with `$D0`, and then does the assertion that gives the
test its teeth: it ticks *ten thousand* cycles and confirms neither `busy`
nor `intrq` ever comes back. A cancellation that merely paused the state
machine would fail there. Its sibling
`force_interrupt_with_i3_sets_intrq_even_while_idle` writes `$D8` to a
chip that has never run a command at all and confirms INTRQ rises anyway
— Force Interrupt is not "abort the current command," it's "assert the
completion signal, whether or not there was anything to complete."

That second behaviour is more useful than it sounds, and the test suite
itself is the proof. Look at how the DSKREG tests read a marker byte off
whichever drive is currently selected
([`tests/fdc/common.rs:39-51`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/common.rs#L39-L51)):
the helper writes `$D0` to `$FF48` *before* every read, because these
tests only tick long enough for one byte rather than a whole sector, so
the previous call's command is very likely still in flight — and the
dispatch rule above drops any command written while `busy` is set. The
test harness has to speak the abort protocol to get a clean start, for
exactly the same reason a real driver does. When a device's dispatch rule
is "ignore me if I'm busy," somebody has to own the recovery path, and
Force Interrupt is it.

---

## 13.4 Type II: Read Sector, walked byte by byte

This is the heart of the chapter — the part of the WD1773 a CoCo program
actually spends its time inside. Dispatch starts the transfer
([`command.rs:158-187`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/command.rs#L158-L187)):

```rust
    fn start_read_sector(&mut self, cmd: u8, disk: Option<&mut JvcDisk>, side: u8) {
        // No data yet: DRQ low so a HALT-enabled driver stalls at its LDA
        // DATAREG loop until the first byte lands (see FIRST_BYTE_LATENCY_CYCLES).
        self.drq = false;
        let multiple = cmd & type1::UPDATE_TRACK_REG != 0; // bit4, same physical bit as T
        match disk {
            Some(d) => match d.sector_offset(self.physical_track, side, self.sector) {
                Some(offset) => {
                    let total = d.sector_size();
                    let buf = d.read_bytes(offset, total).to_vec();
                    self.op = Op::Transfer(Transfer {
                        kind: TransferKind::ReadSector,
                        remaining: FIRST_BYTE_LATENCY_CYCLES,
                        index: 0,
                        total,
                        multiple,
                        offset,
                        buf,
                        first_byte: true,
                        /* format_state/last_id_field/format_enabled: unused for reads */
                    });
                }
                None => self.start_not_found(),
            },
            None => self.start_not_found(),
        }
    }
```

Two things to notice before the byte loop even starts. First, `drq` is
explicitly forced low the instant the command dispatches — the *first*
DRQ event is still `FIRST_BYTE_LATENCY_CYCLES` away, and any driver
polling (or halted on) DRQ before that must see it low, not stale-true
from a previous command. Second, the sector lookup happens *once*, up
front: `d.sector_offset(physical_track, side, sector)` either resolves to
a byte offset in the image, or the command becomes "not found" before a
single DRQ ever fires — no image, no sector, and the whole transfer never
begins.

### The two pacing constants that shape every transfer

Everything about *when* bytes arrive comes down to two numbers, and both
carry their derivation in their doc comments rather than existing as bare
literals ([`wd1773.rs:41-62`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773.rs#L41-L62)):

```rust
/// Cycles between successive DRQ byte events during a Type II/III transfer:
/// double-density byte time at ~32µs, 0.895 MHz CPU clock (spec-provided).
const DRQ_INTERVAL_CYCLES: u32 = 30;

/// MFM byte times a Read/Write Sector command spends searching from the current
/// head position to the target sector's DATA field before the first byte is
/// available: the ID address mark, its 4-byte ID field and 2 CRC bytes, the
/// ~22-byte Gap 2, and the data address mark. This is the *minimum* — a real
/// rotation adds up to a full revolution on top — but it is already far longer
/// than the microseconds-scale setup a driver runs between writing the command
/// and enabling its byte-transfer handshake.
///
/// Load-bearing for polled/HALT drivers that issue the command, run a short
/// fixed delay, THEN arm the transfer (NitrOS-9 `boot_1773`'s ~54-cycle
/// `Delay2` before it sets HALT-enable and enters its `LDA DATAREG` loop). If
/// the first DRQ fires during that delay window the driver never collects those
/// bytes and the transfer trips LOST DATA — which `boot_1773`'s NMI handler
/// reads as `E$Read` and the boot fails. Pacing the *first* byte by one
/// [`DRQ_INTERVAL_CYCLES`] (as every earlier command did) put it inside the
/// window; DSKCON only escaped because it arms HALT before issuing the command.
const FIRST_SECTOR_SEARCH_BYTES: u32 = 30;
const FIRST_BYTE_LATENCY_CYCLES: u32 = FIRST_SECTOR_SEARCH_BYTES * DRQ_INTERVAL_CYCLES;
```

`DRQ_INTERVAL_CYCLES` is the steady-state pace: one byte every 30 CPU
cycles, which at the 0.895 MHz CoCo clock works out to almost exactly the
~32 µs a double-density byte actually takes to spin past the head.
`FIRST_BYTE_LATENCY_CYCLES` — 30 byte-times, not one — is the *search*
delay: the head has to pass the ID address mark, the four-byte ID field
and its CRC, a gap, and the data address mark before the first *data*
byte streams. That's already a real number pulled from an MFM track
layout, not a fudge factor — but the doc comment's second paragraph is
the more interesting fact for an emulator author: this constant is
*load-bearing for a specific boot ROM's timing assumption*, not just "more
realistic." NitrOS-9's `boot_1773` writes the Read Sector command, then
runs a fixed ~54-cycle delay of its own before it even arms the
HALT-based collection loop — and if the emulator's first DRQ arrived
*before* that 54-cycle window closed (as it did with the naive "pace
every byte the same" version this comment describes), the ROM would
simply never notice, the byte would count as lost, and the boot would
print `FAILED`. You'll walk the regression test that pins this exact
fact in §13.7.

### The transfer, one byte at a time

`WD1773::tick` is the engine every transfer runs on — it's short enough
to read in full ([`wd1773.rs:382-414`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773.rs#L382-L414)):

```rust
    pub fn tick(&mut self, mut cycles: u32, mut disk: Option<&mut JvcDisk>, side: u8) {
        while cycles > 0 {
            let consumed = match &mut self.op {
                Op::Idle => break,
                Op::SettlingTypeOne { remaining } | Op::SettlingNotFound { remaining } => {
                    let step = cycles.min(*remaining);
                    *remaining -= step;
                    if *remaining == 0 {
                        self.busy = false;
                        self.intrq = true;
                        self.op = Op::Idle;
                    }
                    step
                }
                Op::Transfer(t) => {
                    let step = cycles.min(t.remaining);
                    t.remaining -= step;
                    if t.remaining == 0 {
                        self.advance_transfer(disk.as_deref_mut(), side);
                    }
                    step
                }
            };
            if consumed == 0 {
                break;
            }
            cycles -= consumed;
        }
    }
```

This is called once per CPU instruction (or once per burned HALT cycle —
§13.7) from `DiskCart::tick`, itself called from `Machine::step_cpu_unit`
after every unit of CPU work. `cycles.min(*remaining)` is the whole trick:
`tick` never overshoots a pending event — if 20 cycles remain until the
next DRQ and this call brings 57 (a whole scanline's worth), it consumes
exactly 20, fires the event, and the `while` loop immediately continues
consuming the remaining 37 against whatever the *next* event's
`remaining` is. One call can cross several byte boundaries in a single
scanline; the loop makes that automatic rather than a special case.

> **Rust corner: reborrowing an `Option<&mut T>` you have to use twice.**
> There is a small piece of Rust ceremony inside that loop worth naming,
> because it comes up in every device that takes borrowed hardware as a
> parameter. `tick` receives `mut disk: Option<&mut JvcDisk>` and may need
> to hand it to `advance_transfer` on several iterations of the `while` —
> but it passes `disk.as_deref_mut()` rather than `disk`.
>
> The reason is that `Option<&mut T>` is not `Copy`. Mutable references
> are unique by construction, so an `Option` wrapping one cannot be
> duplicated; writing `self.advance_transfer(disk, side)` would *move* the
> whole option out of the local variable, and the second time around the
> loop there would be nothing left to pass. `as_deref_mut` solves it by
> producing a fresh, shorter-lived `Option<&mut JvcDisk>` that borrows
> from the local rather than consuming it — a *reborrow*. The callee gets
> exclusive access for the duration of the call, and when the call
> returns the original is usable again.
>
> The same shape appears in the read direction as `as_deref()`, for
> instance where `start_seek` hands the disk to `track_readable`
> ([`command.rs:126`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/command.rs#L126)).
> The rule of thumb: when a `&mut` parameter needs to survive being passed
> onward, reborrow it rather than moving it, and reach for `as_deref_mut`
> whenever it's wrapped in an `Option`.

`advance_transfer` ([`transfer.rs:79-89`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/transfer.rs#L79-L89)) routes by kind, and for a Read
Sector or Read Address it lands in `advance_read_transfer` — the function
that is, in a real sense, the entire reason this chapter exists
([`transfer.rs:91-137`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/transfer.rs#L91-L137)):

```rust
    fn advance_read_transfer(&mut self, mut t: Transfer, disk: Option<&mut JvcDisk>, side: u8) {
        if t.index >= t.total {
            // The CRC trailer elapsed after the final data byte; a
            // still-unread final byte is a genuine overrun.
            if self.drq {
                self.status_lost_data = true;
            }
            self.finish_transfer(t, disk, side);
            return;
        }
        // Spec: "if the previous byte was never taken, set LOST
        // DATA but keep going (do not stall)." Exempt only the
        // very first byte of a fresh command — see `first_byte`.
        if self.drq && !t.first_byte {
            self.status_lost_data = true;
        }
        /* side-resolution for the very first byte: see below */
        self.data = t.buf[t.index];
        self.drq = true;
        t.index += 1;
        t.first_byte = false;
        t.remaining = if t.index >= t.total { CRC_TRAILER_CYCLES } else { DRQ_INTERVAL_CYCLES };
        self.op = Op::Transfer(t);
    }
```

Read it as the software's contract with the chip, stated as code: every
`DRQ_INTERVAL_CYCLES`, one more byte becomes available (`self.data =
t.buf[t.index]`, `self.drq = true`). If the CPU hasn't collected the
*previous* byte by the time the next one is ready — `self.drq` is still
`true` when this fires — that's a real overrun, and the WD1773's own
documented behavior is exactly what the comment says: **flag it and keep
going**, don't stall the transfer waiting for a slow driver. The `!
t.first_byte` exemption exists because `drq`'s reset-state default is
`true` (§13.2) — without excluding the very first byte of a *fresh*
command, that stale leftover `true` would spuriously accuse the command's
own first byte of being an overrun before the driver ever had a chance to
read anything.

### The side the head is actually over

That elided block in the middle of `advance_read_transfer` is the one
piece of this function that cannot be derived from the WD1773's own
behaviour, and it is worth restoring in full because it answers a question
every emulator author eventually has to ask about every input: *when do
you sample it?* ([`transfer.rs:110-127`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/transfer.rs#L110-L127)):

```rust
        // The WD1773 has no side input: head (side) select is the
        // external DSKREG bit, and the controller reads the data field
        // off whatever side the head sits over *when the field streams*
        // — after the ID-address-mark search ([`FIRST_BYTE_LATENCY_CYCLES`]),
        // not when the command was written. OS-9's RBF driver relies on
        // this: it issues the Read Sector command, *then* flips DSKREG to
        // the next side, before the (halting) DATAREG read. So resolve a
        // Read Sector's bytes from the live side at first delivery, not at
        // dispatch. (Multiple-sector continuations already re-resolve in
        // `finish_transfer`; this covers the first/only sector.)
        if t.first_byte
            && t.kind == TransferKind::ReadSector
            && let Some(d) = disk.as_deref()
            && let Some(offset) = d.sector_offset(self.physical_track, side, self.sector)
        {
            t.offset = offset;
            t.buf = d.read_bytes(offset, t.total).to_vec();
        }
```

The tempting design — and the one this code had first — is to resolve the
sector at command dispatch, in `start_read_sector`, and stage its bytes
there once. That reads naturally, keeps the transfer machinery simple, and
is wrong, for a reason that comes down to what the chip physically has.
The WD1773 has no side-select pin. Side is chosen *outside* the
controller, by a bit in DSKREG driving the drive's head-select line, and
the controller reads whatever surface happens to be under the head at the
moment the data field spins past it. There is no latch inside the chip
for a driver's earlier intention to have been recorded in.

Software noticed. The doc comment on the regression test spells out which
software and what it costs
([`tests/fdc/wd1773.rs:121-132`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/wd1773.rs#L121-L132)):
"NitrOS-9 Level 2's RBF driver relies on this when a sequential read
crosses a side boundary: it writes the Read Sector command with the *old*
side still latched in DSKREG, then flips DSKREG to the new side before its
(halting) `LDA DATAREG` loop collects the first byte. Sampling the side at
command dispatch instead reads the wrong physical side — off by one full
track's worth of sectors — silently corrupting every module whose body
straddles a side boundary (e.g. `rb1773`), which wedges the boot at
'NITROS9 BOOT'."

Read that failure mode carefully, because it is the archetype of the
hardest class of emulator bug. Nothing errors. No status bit is set. The
read succeeds, returns 256 perfectly valid bytes, and they are the wrong
256 bytes — sectors from the other side of the platter. The damage
surfaces later, somewhere else, as an operating-system module that loads
without complaint and then misbehaves, and the boot stops at a message
that says nothing about disks. Bugs like this are why the fidelity
question is never "is this close enough" but "which specific observation
does the software make."

The test that pins it is a small, exact reproduction of the driver's
sequence ([`tests/fdc/wd1773.rs:133-166`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/wd1773.rs#L133-L166)).
It builds a two-sided image whose (track 0, sector 1) is filled with
`0xAA` on side 0 and `0x55` on side 1 — so the byte that comes back names
the side that answered — writes the Read Sector command with side 0
selected, then ticks the full first-byte latency with side *1* passed in,
modelling DSKREG flipping during the ID-address-mark search. The
assertion is one byte:

```rust
    assert_eq!(
        wd.read_data(),
        SIDE1_MARK,
        "the data field must come from the side selected when it streams (side 1), \
         not the side latched at command dispatch (side 0)"
    );
```

The lesson reaches well past floppies. Every emulated device has inputs
that arrive from outside itself, and for each one there is a choice of
when to read them: at command time, at completion time, or continuously.
The right answer is not a matter of taste — it is whatever the silicon
physically does, and the tell is whether the real chip has anywhere to
*store* an early sample. A pin with no latch behind it must be sampled
late. When in doubt, ask what the datasheet's block diagram would have to
contain for the early answer to be possible, and if it isn't there,
neither is the latch.

### The full 256-byte walk, in a test

[`tests/fdc/wd1773.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/wd1773.rs) drives exactly this loop, one byte at a time, and
doubles as the clearest possible description of the read-sector protocol
in prose form ([`tests/fdc/wd1773.rs:49-77`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/wd1773.rs#L49-L77)):

```rust
#[test]
fn read_sector_delivers_256_correct_bytes_paced_by_drq_then_intrq() {
    let mut wd = WD1773::new();
    let mut disk = index_pattern_disk();
    wd.track = 0;
    wd.sector = 1;
    wd.write_command(0x80, Some(&mut disk), 0); // Read Sector, no multiple
    assert!(wd.busy);
    for expected in 0..256u32 {
        // The first byte waits out the sector-search latency; the rest pace at
        // one DRQ interval each.
        let step = if expected == 0 { FIRST_BYTE_LATENCY } else { DRQ_INTERVAL };
        wd.tick(step, Some(&mut disk), 0);
        assert!(wd.drq, "DRQ must be asserted for byte {expected}");
        // INTRQ must trail the final byte's DRQ by the CRC-read time: if it
        // rose together with it, the FD-502's NMI would preempt the halt
        // loop's collection of the last byte of every sector.
        assert!(!wd.intrq, "INTRQ before byte {expected} was collected");
        assert_eq!(wd.read_data(), expected as u8, "byte {expected}");
    }
    wd.tick(CRC_TRAILER, Some(&mut disk), 0);
    assert!(!wd.busy, "busy must clear once all 256 bytes are delivered");
    assert!(wd.intrq);
    assert_eq!(
        wd.read_status(true, true) & status::LOST_DATA,
        0,
        "a fully-serviced read must not report LOST DATA"
    );
}
```

`index_pattern_disk()` ([`tests/fdc/common.rs:55-58`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/common.rs#L55-L58)) builds a track whose
sector 1 contains the bytes `0, 1, 2, …, 255` in order, so this single
test simultaneously proves *pacing* (one `tick` per byte, at the right
interval), *ordering* (the value read really is byte `expected`), and the
*INTRQ-trails-DRQ* guarantee, all at once — three separate hardware facts
that would be three separate tests in a less careful codebase.

That comment on the last assertion — "if it rose together with it, the
FD-502's NMI would preempt the halt loop's collection of the last byte" —
names the exact reason `CRC_TRAILER_CYCLES` exists at all
([`wd1773.rs:78-85`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773.rs#L78-L85)):

```rust
/// Delay between a read-direction transfer's LAST data-byte DRQ and command
/// completion (INTRQ): the real chip reads the sector's two CRC bytes off the
/// media first, so INTRQ trails the final DRQ by ~2 byte times. Load-bearing
/// for the FD-502 halt handshake: INTRQ clears DSKREG's halt-enable and fires
/// the NMI that ends DSKCON's transfer loop — if it rose together with the
/// final DRQ, the NMI could preempt the `LDA $FF4B` that collects the last
/// byte of every sector.
const CRC_TRAILER_CYCLES: u32 = 2 * DRQ_INTERVAL_CYCLES;
```

You'll meet this exact mechanism again, at the machine level rather than
the chip level, in §13.7 — this is the WD1773 half of a promise the
FD-502 cartridge (the other half) and the CPU's own interrupt-recognition
rule (a third half, in [`machine/run.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs)) all have to keep together.

### Reading a second regression test: the setup-delay window

[`tests/fdc/wd1773.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/wd1773.rs)'s
`read_sector_first_byte_waits_out_the_driver_setup_delay` exists purely
to pin the fact `FIRST_BYTE_LATENCY_CYCLES`'s doc comment describes. It
issues the same Read Sector command, ticks a 70-cycle
`DRIVER_SETUP_DELAY` (standing in for NitrOS-9's real ~54-cycle `Delay2`)
and asserts DRQ is *still* low at that point — no byte may have arrived
yet — then collects all 256 bytes the way the HALT loop actually does:
`while !wd.drq { wd.tick(DRQ_INTERVAL, ...) }`, spinning until each DRQ
rather than assuming a fixed latency. This is the regression test for the
bug the doc comment narrates: before the fix, the first byte was paced
at one plain `DRQ_INTERVAL_CYCLES` (30 cycles), landing *inside* the
54-cycle setup window, and the driver's own NMI handler reported the
resulting lost byte as `E$Read`.

### The m bit: one command, a whole track

There is one field of `Transfer` this section has quietly skipped past.
`multiple` comes from bit 4 of the Type II command byte, and
`start_read_sector` reads it with a constant borrowed from the Type I
module and a comment explaining the theft: `let multiple = cmd &
type1::UPDATE_TRACK_REG != 0; // bit4, same physical bit as T`. Two
different command families reuse one bit position for two unrelated
purposes, exactly as the status register reuses bits 1 and 2 — the
WD1773's designers were plainly working with a very small nibble budget.

With `m` set, a Read or Write Sector command doesn't stop at one sector.
It rolls onto the next, and the next, until it runs off the end of the
track. That rolling happens in `finish_transfer`, the function every
completed transfer of every kind passes through
([`transfer.rs:148-188`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/transfer.rs#L148-L188), abridged):

```rust
    fn finish_transfer(&mut self, t: Transfer, disk: Option<&mut JvcDisk>, side: u8) {
        if t.multiple {
            let next_sector = self.sector.wrapping_add(1);
            if let Some(d) = disk
                && let Some(offset) = d.sector_offset(self.physical_track, side, next_sector)
            {
                self.sector = next_sector;
                let total = d.sector_size();
                let buf = match t.kind {
                    TransferKind::ReadSector => d.read_bytes(offset, total).to_vec(),
                    _ => Vec::new(),
                };
                self.op = Op::Transfer(Transfer {
                    kind: t.kind,
                    remaining: DRQ_INTERVAL_CYCLES,
                    index: 0,
                    total,
                    multiple: true,
                    offset,
                    buf,
                    first_byte: false,
                    /* format_state/last_id_field/format_enabled: never Write Track */
                });
                return;
            }
            self.status_record_not_found = true;
        }
        self.busy = false;
        self.intrq = true;
        self.op = Op::Idle;
    }
```

Three decisions in that continuation are worth reading deliberately.
First, the *sector register itself* advances — `self.sector =
next_sector`, a value software can read back at `$FF4A` afterwards to
learn where the run stopped. The chip is not keeping a private counter;
it is walking the same visible register the driver used to name the
starting point. Second, the next sector's first byte is paced at one
plain `DRQ_INTERVAL_CYCLES`, not at `FIRST_BYTE_LATENCY_CYCLES`. That is
the honest physical answer: the search latency models a head hunting for
an address mark from an unknown starting position, and by the time a
multiple-sector read reaches its second sector the head is already in the
middle of the track with the next ID field arriving momentarily. Third —
and this is the detail the source comment flags explicitly — the
continuation sets `first_byte: false`, because "this continues the same
multiple-sector transfer, so a still-unread last byte of the previous
sector is a genuine overrun." The `first_byte` exemption exists for a
*fresh* command's stale reset-state DRQ, and a sector boundary inside one
long command is not a fresh command.

When the lookup finally fails, the command ends the only way it can. The
sector register has walked past the last sector on the track,
`sector_offset` returns `None`, and the `if let` falls through to
`self.status_record_not_found = true` followed by the ordinary completion
path. Running off the end of a track is not an error condition a driver
has to avoid; it is how a multiple-sector command is *supposed* to
terminate, and RNF is the flag that says "that's all there was."

`multiple_read_increments_the_sector_register_then_rnf_past_the_last_sector`
([`tests/fdc/wd1773.rs:253-283`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/wd1773.rs#L253-L283))
walks the whole arc on a deliberately tiny disk — a 3-sector track, built
with an explicit JVC header so the run-off-the-end case arrives in three
sectors instead of eighteen — filling each sector with its own number so
the delivered bytes identify which sector answered. It asserts the sector
register's value *before* each sector's bytes, collects all 256 of them,
then ticks one `CRC_TRAILER` and lets the roll-on happen. The comment on
that tick names a small piece of hardware realism that falls out of the
design for free: "the CRC trailer after the sector's last byte doubles as
the inter-sector gap." The two-byte-time delay that exists so INTRQ can
trail the final DRQ is the same delay a real drive spends crossing the
gap between one sector's CRC and the next one's address mark. One
constant, two jobs, and both of them right.

---

## 13.5 Write Sector, and what LOST DATA really means

Write Sector is Read Sector's mirror image, and dispatch starts the same
way — locate the sector, arm a transfer, force DRQ low — with one added
wrinkle: write-protect is checked immediately, before any transfer even
begins ([`command.rs:189-223`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/command.rs#L189-L223)). If `disk.write_protected()`, the command
completes on the spot (`status_write_protect = true; busy = false; intrq
= true; op = Idle`) with no transfer ever armed; otherwise it looks up
`sector_offset` exactly like Read Sector and starts a `Transfer` — except
with `buf: Vec::new()`, since a Write Sector transfer never stages bytes
the way a Read does; there's nothing to stage, since the bytes are coming
*from* the CPU. Instead, the DRQ event on the write side just requests
the next byte and waits — literally waits, with no timeout of its own
([`transfer.rs:139-146`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/transfer.rs#L139-L146), `self.drq = true; t.remaining =
AWAITING_HOST_CYCLES;`).

`AWAITING_HOST_CYCLES` is `u32::MAX` ([`wd1773.rs:64-68`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773.rs#L64-L68)) — a sentinel
chosen specifically so `tick`'s `cycles.min(t.remaining)` can never reach
zero on its own. Nothing times out a write-direction byte; only an
explicit call to `WD1773::write_data` moves the transfer forward, which
is exactly right: a real WD1773 will happily wait forever for the host to
supply the next byte of a write, and a driver that's late doesn't lose
data the way a *read*'s driver does — it just makes the disk spin longer
before the sector's CRC gets written. `write_data` itself both supplies
the byte and re-arms the next DRQ interval ([`transfer.rs:39-75`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/transfer.rs#L39-L75),
abridged):

```rust
    pub fn write_data(&mut self, val: u8, mut disk: Option<&mut JvcDisk>, side: u8) {
        self.data = val;
        self.drq = false;
        let Op::Transfer(mut t) = std::mem::replace(&mut self.op, Op::Idle) else {
            return;
        };
        if t.index < t.total {
            if let (TransferKind::WriteSector, Some(d)) = (t.kind, disk.as_deref_mut()) {
                d.write_byte(t.offset + t.index, val);
            }
            t.index += 1;
        }
        if t.index >= t.total {
            self.finish_transfer(t, disk, side);
        } else {
            t.remaining = DRQ_INTERVAL_CYCLES;
            self.op = Op::Transfer(t);
        }
    }
```

Each byte lands straight in the disk image (`d.write_byte(t.offset +
t.index, val)`) the instant it's written — there's no in-memory sector
buffer being assembled and flushed at the end; every CPU write to `$FF4B`
during an active Write Sector is a real, immediate mutation of `JvcDisk`'s
backing bytes.

[`tests/fdc/wd1773.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/wd1773.rs) proves both the happy path
(`write_sector_round_trips_into_the_image`: write 256 bytes `0..256` one
DRQ at a time, then confirm `disk.read_bytes(...)` at the target sector's
offset matches exactly) and the write-protect short-circuit:

```rust
#[test]
fn write_sector_to_a_write_protected_image_sets_status_and_does_not_transfer() {
    let mut wd = WD1773::new();
    let mut disk = JvcDisk::from_bytes(vec![0xAAu8; ONE_TRACK_BYTES]).unwrap();
    disk.set_write_protected(true);
    wd.track = 0;
    wd.sector = 1;
    wd.write_command(0xA0, Some(&mut disk), 0);
    assert!(!wd.busy, "write-protected write must not transfer");
    assert!(wd.intrq);
    assert_eq!(wd.read_status(true, true) & status::WRITE_PROTECT, status::WRITE_PROTECT);
    let off = disk.sector_offset(0, 0, 1).unwrap();
    assert_eq!(disk.read_bytes(off, 1)[0], 0xAA, "image must be untouched");
}
```

The shape is the tell: `!wd.busy` is asserted *immediately* after
`write_command` returns, with no `tick()` call in between at all.
Write-protect isn't a transfer that starts and then fails partway — it's
detected at dispatch time, before `busy` is ever set past the point where
a transfer would begin, and the pre-existing image byte (`0xAA`) proves
nothing touched it.

---

## 13.6 Type III: Read Address, and Write Track's MFM parser

Types I and II between them cover everything a filesystem does: move the
head, read a sector, write a sector. Type III is what remains — the two
commands that deal with the *track* as a physical object rather than as a
container of sectors, and they are where the abstraction of "a disk is an
array of 256-byte blocks" finally leaks. One of them asks the media what
is written on it; the other writes the media's own structure from
scratch. Type III has two commands this emulator implements (Read Track,
the third, is an unconditional not-found — "optional, RNF is
acceptable," the dispatch comment says — track reads aren't modeled at
all).

### Read Address: six bytes, no data field

Read Address answers "what sector is under the head right now" without
naming one — useful for a driver that's lost track of where it is
([`command.rs:225-259`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/command.rs#L225-L259), abridged):

```rust
    fn start_read_address(&mut self, disk: Option<&mut JvcDisk>, side: u8) {
        self.drq = false;
        match disk {
            Some(d) if (self.physical_track as usize) < d.track_count() => {
                let buf = vec![
                    self.physical_track,
                    side,
                    d.first_sector_id(),
                    d.size_code(),
                    0,
                    0,
                ];
                self.op = Op::Transfer(Transfer {
                    kind: TransferKind::ReadAddress,
                    remaining: FIRST_BYTE_LATENCY_CYCLES,
                    index: 0,
                    total: READ_ADDRESS_LEN,
                    /* ... */
                });
            }
            _ => self.start_not_found(),
        }
    }
```

Six bytes, paced through the exact same `advance_read_transfer` machinery
as a sector read: track, side, the disk's *first* sector ID (not
necessarily the sector the head happens to be nearest — this emulator
doesn't model rotational position, so it always answers with the track's
first sector), a size code, and two CRC bytes that are always zero (CRC
isn't modeled). Worth remembering as a general lesson: "the sector
register is deliberately left alone (spec: 'not needed')" — Read
Address, unlike Read/Write Sector, never touches `self.sector`, because
the whole point of the command is to discover addressing information,
not to act on an already-known one.

Notice how much of that reply is *inferred* rather than read. On real
media the six bytes come off the platter: they are literally the next ID
field the head passes, CRC included, which is why a driver that has lost
its bearings can use Read Address to find out where it is. Here, four of
the six are computed from the mounted image's declared geometry and two
are hardcoded zeros. The size code in particular is derived rather than
stored — `JvcDisk::size_code` inverts the `128 << code` relationship with
`((self.sector_size / 128) as u32).trailing_zeros() as u8`
([`fdc/jvc.rs:260-263`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/jvc.rs#L260-L263))
— because a JVC image records a sector *size*, while the wire format
wants the exponent.

That is a defensible answer as long as you know what it is answering.
A driver asking "which track am I on" gets the truth. A driver asking
"which sector is under the head right now," hoping to schedule its reads
to minimize rotational delay, gets a polite fiction — and any software
that tried to measure rotation by issuing Read Address twice and
comparing would find the platter apparently frozen. Section 13.12 puts
that in the ledger explicitly. The point to carry forward is that a
functional model doesn't have to answer every question a real chip could;
it has to answer the ones its software actually asks, and be legible
enough that the next reader can tell which is which.

### Write Track: formatting, and the MFM stream you have to actually parse

Write Track — DSKINI's command — is the most involved thing the WD1773
does, because unlike every other command, its *payload* isn't sector data
at all: it's a raw byte stream shaped like what a real drive's write head
would lay onto the media, gaps, sync bytes, address marks and all, and the
controller has to *find the sector boundaries inside that stream itself*.

Dispatch is almost trivial — write-protect check exactly like Write
Sector's, then arm a fixed-length transfer of `WRITE_TRACK_BYTE_COUNT`
bytes ([`command.rs:261-293`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/command.rs#L261-L293)) — 6,400, headroom above DSKINI 1.1's own
6,280-byte double-density track template, verified against the real
`disk11.rom`'s format code at `$D6D4`. Every one of those bytes arrives
through `write_data` exactly like a Write Sector byte would, but instead
of writing straight to the disk image, a `WriteTrack` transfer routes each
byte through a small parser when `format_enabled` (the density bit DSKREG
carried at dispatch time, §13.8) is set — single-density (FM) streams are
still just discarded, since this emulator's parser only understands MFM
control-byte conventions ([`transfer.rs:60-62`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/transfer.rs#L60-L62), abridged):

```rust
                TransferKind::WriteTrack if t.format_enabled => {
                    feed_write_track_byte(&mut t, val, disk.as_deref_mut(), side);
                }
                _ => {} // FM: discard, unimplemented
```

The parser itself is a small hand-rolled state machine, one byte in, one
state transition out, recognizing the MFM control bytes `SYNC = $F5`,
`ID_AM = $FE`, `DATA_AM = $FB`/`DELETED_DATA_AM = $F8`, and
`WRITE_CRC = $F7` ([`transfer.rs:14-37,191-291`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/transfer.rs#L14-L37,L191-L291)). Its states form a small
chain: `Gap` waits for a run of `$F5` sync bytes; `Sync` then dispatches
on the byte immediately following the run — `$FE` starts gathering a
4-byte ID field (`IdField`), `$FB`/`$F8` starts gathering the sector's
actual payload (`DataField`), anything else is filler. Once an ID field's
four literal bytes (track, side, sector, size code) are collected, the
parser consumes bytes until it sees `$F7` — the "write CRC" byte, which
on real hardware tells the chip to *compute and emit* two CRC bytes, but
here just marks "this field is done" — and latches `(track, sector,
size_code)` into `last_id_field`. When the data field's terminating `$F7`
arrives next, the buffered payload gets written into the disk image at
exactly that `(track, sector, size_code)`, and — this is the detail worth
lingering on — at the *hardware* side select passed in from outside, not
the side byte the stream itself carried:

```rust
fn step_data_field_term(
    buf: Vec<u8>, val: u8, disk: Option<&mut JvcDisk>,
    hw_side: u8, last_id_field: Option<(u8, u8, u8)>,
) -> FormatState {
    if val == mfm::WRITE_CRC {
        if let (Some(d), Some((track, sector, size_code))) = (disk, last_id_field) {
            d.format_sector(track, hw_side, sector, size_code, &buf);
        }
        FormatState::Gap
    } else {
        FormatState::DataFieldTerm(buf)
    }
}
```

The doc comment on `feed_write_track_byte` explains why: "the WD1773
never derives side from the ID field on Write Track" — a real controller
has no way to read the side byte back out of what it's currently
*writing*; side comes from wherever the physical head actually sits,
which is the DSKREG-controlled hardware side select, full stop.
[`tests/fdc/write_track.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/write_track.rs)'s
`write_track_parses_a_synthetic_dskini_stream_into_the_image` pins this
down directly: it builds a synthetic two-sector DSKINI-style stream whose
ID fields carry `LITERAL_SIDE: u8 = 0x99` — a value that would be
nonsense on any real disk (sides are 0 or 1) — feeds it through
`wd.write_data` one byte at a time at a fixed hardware side (`HW_SIDE =
0`), and confirms both sectors land at `sector_offset(TRACK, HW_SIDE,
sector)`, not anywhere `0x99` could plausibly mean. If the parser were
reading the stream's own side byte, the format would either panic or
silently drop the sector. The same test also confirms the image *grows*
to include the newly-formatted track (`JvcDisk::format_sector` calls
`grow_to_track`, zero-filling up to `MAX_FORMAT_TRACKS` = 82 if the
target track doesn't exist yet) — DSKINI formatting a blank, zero-track
image from nothing is exactly how [`fdc/boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/boot.rs)'s
`dskini_formats_a_blank_disk_and_dir_reports_no_io_error` test exercises
the real `disk11.rom` end to end (§13.9 skips the ROM-dependent walk of
that one, but it's worth reading once you have the ROM locally).

---

## 13.7 The HALT/NMI choreography

Everything in §13.2–13.6 described the WD1773 as a self-contained state
machine you could drive from a debugger, one register access at a time.
Real Disk BASIC doesn't do that — it can't afford to poll a status
register in a tight loop and still keep up with a byte arriving every 32
µs on a CPU that only executes an instruction every few microseconds
itself. Instead, the FD-502 wires the WD1773's DRQ line to the CPU's
*HALT\** pin, through a latch called DSKREG. This section is where week
6's promise — "the FD-502 disk handshake forced that exact [interrupt
recognition] ordering" — finally gets paid off in full.

### Two lines, one cartridge

`DiskCart` computes two control-line outputs from the WD1773's own state
plus one latch bit, in a function run after *every* register access or
`tick` ([`fdc/disk_cart.rs:199-217`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/disk_cart.rs#L199-L217)):

```rust
    /// Recompute the control lines the DSKREG/WD1773 pair drives, per MAME
    /// `coco_fdc.cpp update_lines`: called after every event that could change
    /// INTRQ, DRQ, or DSKREG (register access or a `tick`).
    ///
    /// 1. A high INTRQ clears DSKREG's halt-enable bit (hardware does this).
    /// 2. The NMI line is `intrq && DENSITY_AND_NMI_ENABLE`; an edge on *that*
    ///    line (not on INTRQ itself) is what queues an NMI.
    /// 3. HALT* is `!drq && HALT_ENABLE` — read live by [`Cartridge::halt_asserted`],
    ///    not cached here.
    fn update_lines(&mut self) {
        if self.fdc.intrq {
            self.dskreg &= !dskreg::HALT_ENABLE;
        }
        let nmi_line = self.fdc.intrq && self.dskreg & dskreg::DENSITY_AND_NMI_ENABLE != 0;
        if nmi_line && !self.nmi_line {
            self.nmi_pending = true;
        }
        self.nmi_line = nmi_line;
    }
```

And the two lines it maintains, exposed through the `Cartridge` trait:

```rust
    fn halt_asserted(&self) -> bool {
        !self.fdc.drq && self.dskreg & dskreg::HALT_ENABLE != 0
    }

    fn take_nmi(&mut self) -> bool {
        std::mem::replace(&mut self.nmi_pending, false)
    }
```

Put in words: **HALT\* is asserted whenever DRQ is low and halt-enable is
armed.** That's the whole mechanism that makes a byte-transfer loop
possible on hardware too slow to poll fast enough — the CPU literally
cannot execute another instruction while a byte isn't ready; the WD1773's
own DRQ line, wired through this one gate, stops the clock for it. The
moment `read_data` clears DRQ (§13.2), HALT\* releases and the CPU resumes
— for exactly as long as it takes to loop back to the next `LDA $FF4B`,
at which point DRQ is very likely still low again and it re-halts. An
entire 256-byte transfer can happen inside what *looks* like a handful of
instructions, because most of the wall-clock time isn't CPU time — it's
HALT time, and the CPU isn't running.

**NMI fires on the rising edge of `intrq && DENSITY_AND_NMI_ENABLE`**, not
on INTRQ alone — DSKREG's bit 5 (§13.8) has to be set for a completed
command to actually interrupt the CPU. This is why `nmi_line` is tracked
as its own field, separate from `self.fdc.intrq`: an edge detector needs
to remember the *previous* value of exactly the signal it's watching, and
that signal is a two-input AND, not a chip register. And the first rule
above — "a high INTRQ clears DSKREG's halt-enable bit" — is what lets a
driver stop halting once a command has genuinely completed, without
having to explicitly write DSKREG itself; real hardware behavior (MAME's
`update_lines`), not an emulator convenience.
[`tests/fdc/dskreg.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/dskreg.rs)'s `intrq_high_clears_dskreg_halt_enable` pins it
directly: arm halt-enable, clear DRQ via a data-register read (asserting
HALT\*), then Force-Interrupt with I3 set to raise INTRQ, and confirm
`halt_asserted()` flips back to false.

### The machine loop's side of the deal

`halt_asserted`/`take_nmi` are `Cartridge` trait methods
([`crates/coco-core/src/cart.rs:106-115`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/cart.rs#L106-L115)) with default `false`/`false`
implementations — every cartridge *can* hold the CPU's HALT line and gate
an NMI, but on a stock CoCo 3 only the FD-502 ever does. `SystemBus`
exposes them one level up as plain one-line forwards
(`self.cart.halt_asserted()`, `self.cart.take_nmi()`,
[`bus/sync.rs:28-38`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/sync.rs#L28-L38)), and this is where week 6's `step_cpu_unit` finally
gets its full
explanation. Read the doc comment first — it's dense, and every clause
answers a question the earlier chapters deliberately left open
([`machine/run.rs:86-100`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs#L86-L100)):

```rust
    /// One iteration of the old `run_cycles` inner loop: burn a HALT* cycle or
    /// execute one instruction, then tick the per-cycle peripherals. Returns
    /// `(cycles, was_instruction)`.
    ///
    /// The cartridge HALT* line has priority over everything (MC6809 pin
    /// behaviour): while a device holds it — the FD-502's sector-transfer
    /// handshake — the CPU sits at an instruction boundary burning cycles and
    /// pending interrupts wait. The cartridge is ticked either way so it can
    /// pace the very work (DRQ cadence) that releases the line.
    ///
    /// The MC6809 recognizes interrupts only at the *end* of an instruction, so
    /// the first instruction after HALT* releases must execute before any
    /// pending NMI/IRQ/FIRQ is serviced. Skipping this lets the completion NMI
    /// of an FD-502 sector read preempt the DSKCON copy loop's `STB ,X+` that
    /// stores the sector's final byte — dropping one byte per sector on load.
```

And the implementation that keeps that promise ([`machine/run.rs:101-123`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs#L101-L123)):

```rust
    fn step_cpu_unit(&mut self) -> (u32, bool) {
        let (cycles, was_instruction) = if self.bus.halt_asserted() {
            self.prev_halted = true;
            (1, false)
        } else {
            // Coming straight out of HALT, run one instruction before
            // acknowledging interrupts (they stay pending for next loop).
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
        self.bus.cart.tick(cycles);
        self.bus.cassette.tick(cycles, self.bus.pia1.a.c2_output());
        self.bus.bitbanger.tick(cycles, self.bus.pia1_tx_mark());
        self.bus.cycle_clock = self.bus.cycle_clock.wrapping_add(u64::from(cycles));
        (cycles, was_instruction)
    }
```

Walk the two branches slowly, because the ordering inside the `else`
branch is the entire point. While `halt_asserted()` is true, the function
does nothing but burn one cycle, mark `prev_halted = true`, and — crucially
— still ticks `self.bus.cart` with that one cycle, every single iteration.
This is why HALT time isn't wasted time from the *cartridge's* point of
view: the WD1773's own `tick` (§13.4) is what eventually clears DRQ (by
delivering the next byte) and releases HALT\* in the first place — the
cartridge has to keep running precisely *because* it's the thing holding
the CPU still.

The moment `halt_asserted()` goes false, we're in the `else` branch — and
`prev_halted` gates whether interrupts get serviced *this* call. On the
very first call after HALT\* releases, `prev_halted` is still `true` (set
by the halted call immediately before), so the `if !self.prev_halted`
body is *skipped*: no NMI check, no IRQ/FIRQ check, just `cpu.step`. Only
on the *next* call — with `prev_halted` now `false` — does the function
poll for a pending NMI and (if none) service ordinary interrupts, and
only then does it step the CPU again. In other words: **the very first
instruction the CPU executes after HALT\* releases is guaranteed to run
to completion before any interrupt — including the NMI that HALT\*'s own
release probably just queued — is allowed to vector.**

Why does that one-instruction delay matter so specifically? DSKCON's
transfer loop reads the sector's final byte with something like `LDA
$FF4B` (clearing DRQ, releasing HALT\*), then stores it with `STB ,X+`.
On real 6809 hardware, HALT\* stalls the CPU *between* instructions —
never mid-flight — so once it releases, the very next instruction fetched
is guaranteed to be that `STB`. If this emulator serviced the completion
NMI immediately on HALT\* release, the NMI handler would run *first*, and
whatever it touches on its way through could clobber the register state
or loop bookkeeping the still-pending `STB` depends on. `prev_halted`'s
one-instruction grace period is the emulator honoring the same
instruction-boundary rule the real 6809 enforces in hardware — a
*consequence* of getting instruction boundaries right, not a workaround.

### Reading the synthetic test

[`tests/halt.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/halt.rs) proves the mechanism end to end with a synthetic
cartridge and a tiny hand-assembled ROM — no `roms/` directory required,
which makes it one of the few tests in this whole chapter you can run in
*any* checkout:

```rust
/// Synthetic 32K ROM: reset → `$8000` `LDS #$5EFF` (arms NMI recognition)
/// then `INC $0400; BRA *-3` (a visible-progress loop), NMI → `$8100`
/// `LDA #$A5; STA $0401; RTI`.
fn test_rom() -> Box<[u8]> {
    let mut rom = vec![0u8; ROM_SIZE];
    rom[0x0000..0x0009]
        .copy_from_slice(&[0x10, 0xCE, 0x5E, 0xFF, 0x7C, 0x04, 0x00, 0x20, 0xFB]);
    rom[0x0100..0x0106].copy_from_slice(&[0x86, NMI_MARKER, 0xB7, 0x04, 0x01, 0x3B]);
    rom[0x7FFC..0x7FFE].copy_from_slice(&[0x81, 0x00]); // NMI vector → $8100
    rom[0x7FFE..0x8000].copy_from_slice(&[0x80, 0x00]); // RESET vector → $8000
    rom.into_boxed_slice()
}
```

```rust
impl Cartridge for HaltCart {
    fn tick(&mut self, cycles: u32) {
        let before = self.ticks.get();
        self.ticks.set(before + cycles);
        // The NMI edge fires at the moment the halt releases.
        if before < self.halt_until && before + cycles >= self.halt_until {
            self.nmi_pending.set(true);
        }
    }
    fn halt_asserted(&self) -> bool {
        (self.halt_from..self.halt_until).contains(&self.ticks.get())
    }
    fn take_nmi(&mut self) -> bool {
        self.nmi_pending.replace(false)
    }
}
```

`HaltCart` is a deliberately minimal stand-in for the FD-502: it doesn't
know anything about a WD1773, DSKREG, or DRQ — it just asserts HALT\*
between two fixed cycle counts (`halt_from..halt_until`, both plain
`Cell<u32>` cycle counts the test controls) and raises one NMI edge the
instant HALT\* releases, exactly the shape `update_lines` produces from
the real chip. The test's assertions, across two `run_field()` calls,
check four separate hardware facts in one place: the CPU actually stops during the
halt window (the loop counter barely moves); the cartridge keeps getting
ticked *while* the CPU is halted (`ticks.get()` reaches the release point
even though the CPU itself made almost no progress); no NMI vectors while
still halted; and the pending NMI vectors exactly once, consumed, the
instant the halt releases. What this test does *not* prove — and this
matters for exercise 13.5 — is the one-instruction-delay ordering itself:
the synthetic ROM's loop body (`INC`/`BRA`) doesn't care which specific
instruction executes when NMI fires, so a version of `step_cpu_unit` that
serviced the NMI one instruction too early would still pass every
assertion here. The test that actually catches *that* regression needs a
program that behaves like DSKCON — collect a byte, then store it — which
is exactly what [`fdc/boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/boot.rs)'s `loadm_preserves_every_sector_byte_across_
the_halt_nmi_handshake` test does, against the real ROM.

---

## 13.8 DSKREG ($FF40), bit by bit

DSKREG is the FD-502's own latch — not part of the WD1773 at all, but the
glue register the *cartridge* exposes to select a drive, turn the motor
on, choose a density, and arm the HALT/NMI wiring you just read about
([`fdc/disk_cart.rs:13-34`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/disk_cart.rs#L13-L34)):

```rust
pub mod dskreg {
    /// Halt-enable: while set, the HALT* control line asserts whenever DRQ is
    /// low (see `DiskCart`'s `Cartridge::halt_asserted` implementation below).
    pub const HALT_ENABLE: u8 = 0x80;
    /// Drive-select 3 when no lower drive-select bit is set, else the side
    /// (head) select for drives 0-2.
    pub const DRIVE3_OR_SIDE: u8 = 0x40;
    /// Density select (1 = double). MAME wires this same bit to gate NMI on
    /// INTRQ; the `dden` pin the WD1773 actually sees is the inverse of this
    /// bit, which doesn't matter here since FM/MFM density isn't modelled —
    /// only the NMI-enable use of this bit is implemented.
    pub const DENSITY_AND_NMI_ENABLE: u8 = 0x20;
    /// Write precompensation select — stored, not acted on (spec).
    pub const WRITE_PRECOMP: u8 = 0x10;
    /// Motor on, all drives.
    pub const MOTOR_ON: u8 = 0x08;
    pub const DRIVE2: u8 = 0x04;
    pub const DRIVE1: u8 = 0x02;
    pub const DRIVE0: u8 = 0x01;
}
```

Eight bits, eight jobs — and bit 6 has *two* jobs depending on context,
which is the single trickiest fact in this register. Drive selection
resolves by priority, not by treating the three low bits as a binary
number ([`fdc/disk_cart.rs:39-51`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/disk_cart.rs#L39-L51)):

```rust
fn selected_drive(reg: u8) -> Option<usize> {
    if reg & dskreg::DRIVE2 != 0 {
        Some(2)
    } else if reg & dskreg::DRIVE1 != 0 {
        Some(1)
    } else if reg & dskreg::DRIVE0 != 0 {
        Some(0)
    } else if reg & dskreg::DRIVE3_OR_SIDE != 0 {
        Some(3)
    } else {
        None
    }
}
```

Bit 2 wins if set, regardless of what else is set; failing that, bit 1;
failing that, bit 0; and only if *none* of the three drive-select bits are
set does bit 6 mean "select drive 3." That last case is why bit 6 is
named `DRIVE3_OR_SIDE` rather than just `DRIVE3` or just `SIDE` — its
meaning depends on whether a *different* drive is already selected:

```rust
fn selected_side(reg: u8, drive: Option<usize>) -> u8 {
    if reg & dskreg::DRIVE3_OR_SIDE != 0 && drive != Some(3) { 1 } else { 0 }
}
```

For drives 0–2, bit 6 set means "side 1." For drive 3 — which, by
`selected_drive`'s own logic, is *only* ever selected when bit 6 is the
thing selecting it — that same bit means something else entirely (the
drive-select itself), so `selected_side` explicitly excludes it: side is
always 0 for drive 3 in this emulator, because there's no remaining bit
left to carry a side-select for it. [`tests/fdc/dskreg.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/dskreg.rs)'s
`drive_select_priority_bit2_then_bit1_then_bit0_then_bit6` exercises the
priority chain directly: it mounts a differently-marked disk in all four
slots, then walks four DSKREG values — `DRIVE2|DRIVE1|DRIVE0` (bit 2
wins, drive 2 answers), `DRIVE1|DRIVE0` (bit 1 wins), `DRIVE0` alone, and
`DRIVE3_OR_SIDE` alone (falls through to drive 3) — confirming each one
reads back its own drive's marker byte.

Two more facts round out the register. First, DSKREG *mirrors* across the
entire `$FF40`–`$FF47` range — writing any of those eight addresses hits
the same latch (`DSKREG_BASE..=DSKREG_LAST => self.dskreg = val` on
write) — while reads anywhere in that range are open bus, not an echo of
the last write (`DSKREG_BASE..=DSKREG_LAST => IO_OPEN_BUS`). Second,
NOT_READY in the WD1773's own status register — a bit that looks like it
belongs entirely to the chip — is actually computed from DSKREG context
the WD1773 itself has no way to know: `DiskCart::read`'s
`STATUS_COMMAND_REG` arm computes `disk_present` (is a disk mounted in
the selected drive) and `motor_on` (DSKREG's motor bit) fresh on every
access and *passes them in* to `self.fdc.read_status(disk_present,
motor_on)`. The WD1773 has no concept of "is a disk physically in the
drive" or "is the motor spinning" — those are facts about the FD-502
cartridge's own drive bay, not the chip. It's a clean illustration of the
boundary this chapter has been drawing all along: the WD1773 struct
models the chip; `DiskCart` models everything *around* the chip that a
real cartridge schematic would show as separate ICs and switches.

---

## 13.9 JVC images: geometry from a file, geometry from a guess

Every disk image this emulator mounts is a `JvcDisk`
([`crates/coco-core/src/fdc/jvc.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/jvc.rs)) — the de facto standard format for
CoCo emulators, named for Jeff Vavasour: about as close to "no format at
all" as a disk image gets. **A headerless JVC image is just the raw
sector bytes, in order, with nothing describing its own shape.** Geometry
comes either from an optional short header, or — if there's no header —
from defaults plus one clever heuristic.

### The formula

Every sector access, read or write, MFM-parsed or driven straight from
the WD1773, eventually calls one function ([`fdc/jvc.rs:284-302`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/jvc.rs#L284-L302)):

```rust
    /// Byte offset of `(track, side, sector_id)` in the in-memory image, or
    /// `None` if out of range.
    ///
    /// `offset = header + ((track * sides + side) * spt + (sector_id -
    /// first_id)) * sector_size` (spec-provided formula, matching MAME
    /// `jvc_dsk.cpp`'s sector lookup).
    pub fn sector_offset(&self, track: u8, side: u8, sector_id: u8) -> Option<usize> {
        let track = track as usize;
        let side = side as usize;
        if track >= self.track_count || side >= self.sides {
            return None;
        }
        let sector_index = sector_id.checked_sub(self.first_sector_id)? as usize;
        if sector_index >= self.sectors_per_track {
            return None;
        }
        let row = track * self.sides + side;
        Some(self.header_len + (row * self.sectors_per_track + sector_index) * self.sector_size)
    }
```

Read it as: a two-sided image lays its sectors out **track-major, side
interleaved within a track** — track 0 side 0, then track 0 side 1, then
track 1 side 0, and so on — never "all of side 0, then all of side 1."
`row = track * sides + side` is exactly that interleaving, and everything
after it is ordinary row/column arithmetic: which sector-sized slot within
the row (`sector_index`, after subtracting whatever sector-numbering base
the image uses — 1, on nearly every real CoCo disk), times the sector
size, plus however many header bytes sit in front of the whole thing.
`sector_id.checked_sub(self.first_sector_id)?` is worth a second look too:
if the requested sector ID is *below* the image's first sector ID, the
subtraction would underflow a `usize` — `checked_sub` turns that into a
clean `None` via the `?` operator instead of panicking or wrapping to a
huge, wrong offset. [`tests/fdc/image_geometry.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/image_geometry.rs)'s
`two_sided_image_interleaves_track0_side0_track0_side1_track1_side0`
checks the interleaving directly by content, not just arithmetic: a
1-spt, 2-sided, 128-byte-sector image with markers `0,1,2,3` laid out in
row order, and `(track, side)` pairs `(0,0)→0, (0,1)→1, (1,0)→2, (1,1)→3`
each asserted through `sector_offset`.

### The optional header, and the defaults it fills in

`JvcDisk::from_bytes` computes the header length as `file_len % 256` —
which sounds almost too clever until you notice it's really just "a
proper JVC image's data region is always a whole number of 256-byte
blocks; whatever's left over at the front is the header." A header can be
0 to 5 bytes, and each byte you *do* supply overrides exactly one
default: sectors/track (18, the RS-DOS standard), sides (1), sector-size
code (1, meaning `128 << 1 = 256`), first sector ID (1) — and a 5th byte
this implementation doesn't support at all: a nonzero attribute-byte flag
(some JVC variants prepend an extra byte to every sector) is rejected
outright (`JvcError::AttributeBytesUnsupported`) rather than silently
misreading the geometry. Most real CoCo disk images in the wild are
headerless (`header_len == 0`, an exact multiple of 256 bytes) and rely
on every one of those five defaults, which happen to describe a standard
single-sided 35-track RS-DOS disk.

### Two policies for bad input, in one file

`from_bytes` returns a `Result`, and the error type is worth reading
because it is a compact statement of everything the parser refuses to
guess at ([`fdc/jvc.rs:88-107`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/jvc.rs#L88-L107)):

```rust
pub enum JvcError {
    /// Header byte 4 (sector attribute flag) was nonzero: every sector would
    /// carry an extra prepended attribute byte, a JVC variant this
    /// implementation doesn't support.
    AttributeBytesUnsupported,
    /// The data portion (file length minus header) doesn't divide evenly into
    /// whole tracks of `sectors_per_track * sector_size * sides` bytes, or
    /// yields zero tracks.
    InvalidGeometry {
        file_len: usize,
        header_len: usize,
        sectors_per_track: usize,
        sides: usize,
        sector_size: usize,
    },
    /// [`JvcDisk::reattach_data`] only: the reattached file parses to a
    /// different geometry than the snapshot recorded — it changed shape
    /// (was reformatted, truncated, grown, …) since the snapshot was taken.
    GeometryChanged,
}
```

`InvalidGeometry` carries every input to the decision, not just a message,
which is a habit worth stealing: an error that reports `file_len=184320,
sectors_per_track=18, sides=1, sector_size=256` lets a user work out for
themselves that they have an 80-track image being read as something else,
where "invalid geometry" alone would send them to a forum. `GeometryChanged`
belongs to week 16's material and is a nice illustration of the same
principle applied to time rather than to shape: a snapshot records the
geometry it saw, and if the file on disk has been reformatted since, the
restore refuses rather than resuming a transfer into an image that has
moved underneath it.

Now compare that strictness to how the *same file* treats a bad format
request. `JvcDisk::format_sector` (§13.6's Write Track destination) is
handed a track, side, sector ID and size code straight out of an MFM
stream, and any of them can be nonsense. Its response is to do nothing at
all — silently ([`fdc/jvc.rs:313-323`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/jvc.rs#L313-L323)):
"Silently does nothing if the geometry doesn't match (foreign sector ID,
wrong size code, side >= sides(), including a side-1 write on a
single-sided image) or the cap is exceeded — real hardware has no error
path for this, and JvcDisk can't represent a sector outside its own
geometry (spec)."

Two opposite policies, one file, and the rule that separates them is not
taste. Mounting an image is a *host* operation: a user picked a file, and
if it can't be interpreted the honest thing is to say so, loudly, before
anything else happens. Formatting a sector is an *emulated* operation
being driven by 6809 code, and the real FD-502 has no channel through
which to report "that sector ID is not one this drive can lay down" — the
write head simply writes flux that no subsequent read will recognize.
Inventing an error there would be inventing hardware. When you're deciding
how strict a component should be, ask which side of the emulation boundary
the caller is standing on, and give the emulated side exactly the failure
modes the silicon had.

### The OS-9 sniff: a heuristic with guardrails

Here's the problem a bare headerless parse can't solve on its own: a
40-track, 2-sided NitrOS-9 disk, dumped without a header, is
*indistinguishable by size alone* from an 80-track, 1-sided disk — both
are exactly `80 × 18 × 256` bytes. The JVC defaults assume 1 side, so a
naive parse of that file gets the track count wrong by a factor of two,
and every `sector_offset` call after that reads the wrong physical
location. OS-9 disks, though, always carry a self-describing "LSN0"
identification sector at the very start of the image, and this parser
sniffs it — but *only* when there's no explicit header to override, and
only when the sniffed fields are fully self-consistent with what's
already known ([`fdc/jvc.rs:42-84`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/jvc.rs#L42-L84)):

```rust
/// Trusted only if: DD.SPT equals the JVC default (18 — this crate doesn't
/// support other sniffed geometries), `DD.TOT * 256 == file_len`, DD.TOT
/// divides evenly by `DD.SPT * sides`, and the implied track count is nonzero
/// and within [`MAX_FORMAT_TRACKS`] (MAME's largest floppy table entry is 80
/// tracks). This rejects both non-OS-9 images (an all-zero LSN0 fails the SPT
/// check) and disk-shaped-but-not-floppy images like a 1024-track cocosdc
/// dump (fails the track-count cap).
fn sniff_os9_sides(bytes: &[u8], file_len: usize) -> Option<usize> {
    let lsn0 = bytes.get(..os9_lsn0::LEN)?;
    let dd_tot = /* 24-bit big-endian at TOT_OFFSET */;
    let sides = if lsn0[os9_lsn0::FMT_OFFSET] & os9_lsn0::FMT_SIDES_BIT != 0 { 2 } else { 1 };
    let dd_spt = /* 16-bit big-endian at SPT_OFFSET */;

    if dd_spt != DEFAULT_SECTORS_PER_TRACK {
        return None;
    }
    if dd_tot as usize * (128usize << DEFAULT_SECTOR_SIZE_CODE) != file_len {
        return None;
    }
    let sectors_per_side_group = dd_spt * sides;
    if !(dd_tot as usize).is_multiple_of(sectors_per_side_group) {
        return None;
    }
    let implied_tracks = dd_tot as usize / sectors_per_side_group;
    if implied_tracks == 0 || implied_tracks > MAX_FORMAT_TRACKS {
        return None;
    }
    Some(sides)
}
```

Four independent checks, all of which have to pass before this function
returns `Some`, and each rejects a specific way a non-OS-9 (or corrupted)
image could accidentally look plausible: `DD.SPT` has to equal the same
18 this parser already assumes elsewhere (a genuine OS-9 disk formatted
with a different geometry falls back to the naive defaults, since "trust
a sniffed geometry this crate can't otherwise represent" isn't an
option); `DD.TOT * 256` has to equal the file's actual length exactly (a
corrupted or truncated LSN0 fails here); and the implied track count has
to divide evenly and land inside the range MAME's own floppy geometry
table allows, which is precisely what rejects a "disk-shaped-but-not-
floppy" image like an oversized `cocosdc` dump that happens to also be a
multiple of 256 bytes. The call site makes the "only when nothing else
already claimed authority" rule explicit:

```rust
        if header_len == 0
            && sides == DEFAULT_SIDES
            && sniff_os9_sides(&bytes, file_len) == Some(2)
        {
            sides = 2;
            track_count /= 2;
        }
```

An *explicit* header always wins outright — even a 1-byte JVC header
saying "18 sectors/track" and nothing else skips the sniff entirely,
because `header_len == 0` is false. Only a fully headerless image, whose
naive parse landed on the single-sided default, ever gets a second
opinion, and that opinion only overrides the parse if all four guardrails
agree. [`tests/fdc/image_geometry.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/image_geometry.rs) proves both the adoption and the
rejection paths, plus the case that matters most for correctness — a
disk that merely *resembles* an OS-9 signature by accident:
`os9_lsn0_with_mismatched_tot_keeps_naive_defaults` stamps a synthetic
LSN0 claiming 2 sides but with a deliberately corrupted `DD.TOT`, and
asserts the sniff is rejected outright — `sides() == 1`, the naive
default, exactly as if no LSN0 were there at all.

**Every test in [`tests/fdc/image_geometry.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/image_geometry.rs), [`tests/fdc/wd1773.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/wd1773.rs), and
[`tests/fdc/write_track.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/write_track.rs) needs no ROM and no external disk image at
all** — they construct their `JvcDisk`s and `WD1773`s directly, in
memory. The one exception, `real_nitros9_40_track_disk_parses_as_
40_tracks_2_sides`, checks the same sniff logic against a real NitrOS-9
disk and skips gracefully with an `eprintln!` if
`disks/NOS9_6809_L2_v030300_coco3_40d_1.dsk` isn't present. `cargo test -p
coco-core --test fdc` in a checkout that has no `roms/`
directory at all proves the split cleanly: 29 tests pass outright, and
exactly 11 fail, every one of them in [`fdc/dskreg.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/dskreg.rs) or [`fdc/boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/boot.rs),
both of which construct a `DiskCart` via `DiskCart::new(load_rom
("disk11.rom"))` — and `load_rom` panics with a plain "cannot read" error
the moment that file doesn't exist, no graceful skip. If you run this
suite yourself without `roms/disk11.rom` present, that's the failure
you should expect, and it tells you nothing about the WD1773 or JVC code
itself.

---

## 13.10 VHD: the minimal counterexample

Everything in §13.2–13.9 was in service of one argument: a real chip's
protocol is *work*, because the chip has physical constraints the
software has to respect. VHD exists to show what's left once you remove
every one of those constraints. It isn't real 1980s hardware — MAME's
authors invented it, purely as an emulator convenience ([`vhd.rs:1-13`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/vhd.rs#L1-L13)),
and gave NitrOS-9's `emudsk.asm` driver a reason to exist by writing a
driver against it.

The whole device is seven registers, and they fit in a table small enough
to hold in your head — which, against the four WD1773 registers plus
DSKREG plus two control lines of the last eight sections, is already the
point being made:

| Address | The driver writes | The driver reads |
|---------|-------------------|------------------|
| `$FF80`–`$FF82` | a 24-bit logical record number, high byte first | `0` while a drive is selected, open bus otherwise |
| `$FF83` | a command byte, which executes immediately | the last command's status code, or open bus while deselected |
| `$FF84`–`$FF85` | a 16-bit CPU buffer address | `0` while a drive is selected, open bus otherwise |
| `$FF86` | a drive select: `0` or `1`, anything else deselects both | open bus, unconditionally |

The command vocabulary is three bytes wide — `READ = 0x00`, `WRITE =
0x01`, `FLUSH = 0x02` — and the status vocabulary five: `OK = 0x00`,
`NO_VHD = 0x02`, `IO_ERROR = 0x05`, `UNKNOWN_COMMAND = 0xFE`, and
`POWER_ON = 0xFF`, the state a freshly mounted drive reports before any
command has run. The LRN is VHD's word for "sector number," 24 bits of
it — one bit shy of 17 million sectors, or four gigabytes at 256 bytes
each, which is a great deal more storage than any CoCo could plausibly
attach.

Everything interesting about the device is in what that table *doesn't*
contain. There is no busy flag, no data-ready flag, no data register, and
no interrupt. Write a command byte to `$FF83` and by the time that write
returns, the entire 256-byte sector has already moved between the image
and CPU memory. No DRQ. No BUSY. No byte-pacing timer. No HALT line.
Compare the register write methods to the WD1773's command dispatch and
the contrast is the whole lesson:

```rust
    /// `$FF80` write (LRN high byte, bits 16–23). Dropped while deselected.
    pub fn write_lrn_hi(&mut self, val: u8) {
        if let Some(drive) = self.selected_drive() {
            let d = &mut self.drives[drive];
            d.lrn = (d.lrn & 0x00FFFF) | (u32::from(val) << 16);
        }
    }
```

Every register write here is exactly what it looks like: update a field,
guarded only by "is a drive currently selected" — no state machine, no
`Op` enum, no `Transfer` struct tracking progress through a multi-step
protocol, because there *is* no multi-step protocol on the register side.
A write that arrives while `$FF86` holds anything other than `0` or `1`
is simply dropped, and dropped silently: `selected_drive()` returns
`None`, the `if let` doesn't fire, and the byte is gone. There is nowhere
for it to have been stored, because the LRN lives inside a drive rather
than in the device.

The read side is stranger, and it is the sharpest contrast with the
WD1773. Those five address registers do not read back what was written to
them
([`vhd.rs:249-263`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/vhd.rs#L249-L263)):
the doc comment records that "MAME implements no readback for these
registers," so they answer `0` while a drive is selected and open bus
while none is. A driver cannot ask VHD what sector number it last set. On
the floppy side, `$FF49` and `$FF4A` read back faithfully, and §13.4's
multiple-sector walk depends on it — the sector register is how a driver
learns where a run stopped. VHD's registers are, in the strictest sense,
a place to put arguments before a call, and a call's arguments are not
usually something you interrogate afterwards.

One doc comment in this module deserves attention for a reason that has
nothing to do with disks. `Vhd::new` picks drive 0 as the power-on
selection, and rather than let that pass as a fact, the comment labels it
([`vhd.rs:191-201`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/vhd.rs#L191-L201)):
"This is an inferred default, not a verified hardware fact: no source
available for this implementation states the drive-select latch's
power-on value. Drive 0 selected matches a typical zeroed-register reset
state and is the natural default a DOS would assume, but should be
treated as low-confidence until confirmed against real hardware or MAME's
device reset code."

Every emulator contains guesses. What distinguishes a maintainable one is
whether the guesses are *labelled*, because an unlabelled guess is
indistinguishable from a verified fact six months later, and the person
chasing a bug through this code will waste a day proving something nobody
ever claimed. Writing "low-confidence until confirmed" costs one sentence
and buys the next reader a shortlist of the places worth suspecting. It's
a habit worth carrying into your own emulator from the first module.

### Where the actual work happens

Because VHD is wired directly into `SystemBus`'s own I/O decode rather
than living behind the `Cartridge` trait (§13.1), command execution lives
in a private bus method, not in `vhd.rs` at all — the module doc comment
says so explicitly: it needs to "transfer sector data through the
GIME-translated logical address space," which is `SystemBus`'s job, not
a standalone device's. Here's the READ command's entire implementation
([`bus/vhd_bridge.rs:51-65`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/vhd_bridge.rs#L51-L65)):

```rust
    fn vhd_read_sector(&mut self, drive: usize) {
        let offset = vhd_sector_offset(self.vhd.drives[drive].lrn);
        let mut buf = [0u8; vhd::SECTOR_SIZE];
        let read_result = self.vhd_image_mut(drive).read_at(offset, &mut buf);
        match read_result {
            Ok(_) => {
                let buffer_addr = self.vhd.drives[drive].buffer_addr;
                for (i, byte) in buf.iter().enumerate() {
                    self.write(buffer_addr.wrapping_add(i as u16), *byte);
                }
                self.vhd.drives[drive].status = vhd::status::OK;
            }
            Err(_) => self.vhd.drives[drive].status = vhd::status::IO_ERROR,
        }
    }
```

Read the sector from the backing file straight into a stack buffer, then
copy that buffer into CPU memory one `self.write(...)` call at a time —
each of those calls goes through the *full* MMU-translated bus path,
exactly as if the CPU itself had executed 256 `STA` instructions, but
without spending a single CPU cycle to do it. This isn't a shortcut that
skips the memory model — the transfer still respects the MMU, still
wraps at 64K (`buffer_addr.wrapping_add(i as u16)`), still would hit
whatever's actually mapped into that logical address range — the *only*
thing missing is time and byte-by-byte handshaking. The doc comment on
the write path calls out one more piece of hardware-accurate ordering
worth noticing even in a device this simple:

```rust
    /// WRITE (`vhd::command::WRITE`): zero-extend the image up to `drive`'s
    /// LRN offset first, THEN fetch [`vhd::SECTOR_SIZE`] bytes from `drive`'s
    /// buffer address through the CPU's logical address space, THEN write
    /// them into the image. This exact order matters: MAME performs the
    /// zero-extend before touching the CPU bus, which is observable if the
    /// buffer address happens to overlap the VHD's own I/O registers.
```

Even a device with "zero ceremony" still has *one* piece of ordering that
matters — the CPU bus read that fetches the sector data could, in
principle, land back on the VHD's own registers (if a program pointed the
buffer address at `$FF80`–`$FF86` itself), and MAME's own implementation
order is what this codebase matches. "Simple" and "order-independent"
aren't the same claim.

That reentrancy possibility is real enough that VHD guards against it
explicitly — a `busy: bool` flag, entirely this implementation's own
addition, not something MAME's hardware model needed:

```rust
    pub(super) fn vhd_execute_command(&mut self, cmd: u8) {
        let Some(drive) = self.vhd.selected_drive() else {
            return;
        };
        if self.vhd.busy {
            return;
        }
        self.vhd.busy = true;
        /* ... dispatch ... */
        self.vhd.busy = false;
    }
```

A program that (accidentally or maliciously) points its buffer address at
`$FF83` itself would, without this guard, trigger `vhd_execute_command`
*from inside* its own byte-copy loop — Rust doesn't have a stack overflow
guard for "logically reentering the same mutable-state function," so this
is an emulator-author's own defensive addition, guarding only command
*execution* (register writes during a transfer are unaffected), not a
hardware fact translated from a datasheet.

Because VHD lives inline in `SystemBus`'s own decode rather than behind a
trait, its address ranges show up as ordinary match arms right next to
the GIME's own registers ([`bus/io.rs:74-78`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L74-L78)) — and `$FF86` (drive select)
is one of the odder registers in the whole codebase for a reason worth
remembering: it's *write-only* in the strictest possible sense
(`VHD_SELECT => OPEN_BUS`, unconditionally). Software can select a drive;
it can never ask VHD "which drive is currently selected?" through this
register. [`tests/vhd.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/vhd.rs) covers the full lifecycle this section described
— status transitions, short/EOF reads, zero-extend-on-write, drive
independence, MMU-translated transfers, and the reentrancy guard — but
every one of its tests boots a real CoCo 3 ROM through
`Machine::new(MachineConfig::default(), load_rom("coco3.rom"))`
([`tests/vhd.rs:36-38`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/vhd.rs#L36-L38)), with **no graceful skip** if that file is
missing. Without that file, `cargo test -p coco-core --test vhd` fails all
14 of its tests with the same `cannot read .../roms/coco3.rom` panic —
worth knowing before you run it, so you don't mistake "no ROM available"
for "VHD is broken."

---

## 13.11 DriveWire: a protocol, not a chip

The third corner of the triangle isn't local hardware at all — the
module doc comment says what it actually is plainly ([`drivewire.rs:1-18`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire.rs#L1-L18)):

```rust
//! DriveWire 4 — an in-process implementation of the *server* side of the
//! DriveWire protocol: a byte-stream RPC that lets NitrOS-9 (or DECB, via
//! HDB-DOS) address disk images living on the host instead of real
//! hardware, one 256-byte sector at a time.
```

On a real CoCo, DriveWire is a serial cable running from the "Becker
port" (a modest hardware hack — later, on real machines, a genuine serial
adapter) to a PC, and a *server program* on that PC — the DriveWire
server — holds the actual `.dsk` files and answers requests over the
wire. That server is a real, still-actively-used piece of the CoCo
community's infrastructure today: it's how people run NitrOS-9 or HDB-DOS
on original hardware with modern storage, no floppy drive required. This
codebase implements the server side entirely in Rust, in-process — the
"wire" is just a Becker-port register pair, and the "PC" is the same
process as the emulator — but the *protocol* the driver code speaks is
unchanged: framing, opcodes, and checksums, exactly as if bytes really
were crossing a cable that could drop or corrupt them.

### The Becker port, and precedence over the cartridge

`$FF41`/`$FF42` sit inside the cartridge port's address range (`$FF40`–
`$FF7E`), and that placement is the whole reason precedence has to be
handled explicitly. `SystemBus::io_read`/`io_write` check the Becker port
*first*, before cartridge dispatch, on every I/O decode path — GIME and
plain-SAM CoCo 1/2 alike — via a small pair of methods returning
`Option<u8>`/`bool` so the caller can tell "the Becker port answered"
apart from "fall through" ([`bus/io.rs:19-59`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/io.rs#L19-L59), abridged):

```rust
    pub(super) fn becker_read(&mut self, addr: u16) -> Option<u8> {
        let dw = self.drivewire.as_mut()?;
        match addr {
            BECKER_STATUS => Some(dw.status_read()),
            BECKER_DATA => Some(dw.data_read()),
            _ => None,
        }
    }
    // io_read: `if let Some(v) = self.becker_read(addr) { return v; }`
    // BEFORE the match that includes `CART_BASE..=CART_LAST => self.cart.read(addr)`.
```

`becker_write` has the same shape (`BECKER_STATUS` writes are silently
swallowed; `BECKER_DATA` feeds `DwServer::data_write`), guarded
`if self.drivewire.is_none() { return false; }`, checked before
`io_write`'s own `match`. This means a genuine `DiskCart` (§13.2–13.9,
occupying that same address range through its own
`$FF48`/`$FF49`/`$FF4A`/`$FF4B` registers, which sit outside
`$FF41`/`$FF42`) and a live Becker-port DriveWire server can, in
principle, coexist in the same `SystemBus` — the two-byte Becker window
is carved out of the cartridge's address space before the cartridge ever
sees it. [`tests/drivewire_bus.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/drivewire_bus.rs)'s `becker_takes_precedence_over_
cartridge` proves the carve-out with a synthetic marker cartridge that
would otherwise answer every read with a recognizable byte: `$FF41`/
`$FF42` go to Becker (idle status `0x00`) while `$FF40`/`$FF43` — just
outside the two Becker registers, still inside the cartridge's own range
— reach the marker cartridge's fixed reply.

`enable_drivewire()` is worth noting too: DriveWire, unlike a `DiskCart`,
isn't a cartridge you insert; it's a bus-level feature you switch on
([`bus.rs:173-176`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus.rs#L173-L176)). When `self.drivewire: Option<DwServer>` is `None`,
`becker_read`'s `self.drivewire.as_mut()?` short-circuits before the
address is even inspected, and `$FF41`/`$FF42` fall straight through to
cartridge dispatch exactly as if the Becker port didn't exist — disabled
DriveWire costs nothing beyond one `Option` check per I/O access.

### The state machine, one byte at a time

Every DriveWire transaction is driven by feeding one byte at a time into
`DwServer::data_write`, which is what a write to `$FF42` calls. The
protocol's entire vocabulary of "what am I in the middle of" lives in one
enum ([`drivewire/protocol.rs:24-87`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire/protocol.rs#L24-L87), abridged to the variants a READ
transaction actually visits):

```rust
pub(super) enum State {
    Idle,
    /* AwaitDwInitVersion, AwaitDiscard, AwaitSerReadM, AwaitSerSetStat: */
    /* handshake and virtual-serial bookkeeping, not shown here */

    /// A READ-family opcode sent; awaiting the rest of the 4-byte header.
    AwaitReadHeader { ex: bool, buf: Vec<u8> },
    /// A READEX-family read's 256 data bytes have been sent; awaiting the
    /// client's 2-byte checksum.
    AwaitReadExChecksum { expected: u16, pending_error: u8, buf: Vec<u8> },
    /// A WRITE-family opcode sent; awaiting the rest of the 262-byte body.
    AwaitWriteBody { buf: Vec<u8> },
}
```

Walk a plain `READ` transaction (`opcode::READ`, not the checksum-
verified `READEX` variant) through this machine byte by byte. `feed`
dispatches purely on `self.state` ([`drivewire/protocol.rs:107-125`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire/protocol.rs#L107-L125)), one
byte in, one state transition out — the same shape as the Write Track
parser in §13.6. Byte 1 (the opcode) arrives while `State::Idle`, and
`handle_opcode` routes `READ`/`REREAD` to `self.state =
State::AwaitReadHeader { ex: false, buf: Vec::with_capacity(HEADER_LEN)
}`. Bytes 2–5 are the 4-byte header — drive number, then a 24-bit
big-endian LSN — and each one lands in `feed_read_header`, which appends
to `buf` (`buf.push(byte)`) until it reaches `HEADER_LEN`, then calls
`execute_read(ex, &buf)`. That function ([`drivewire/transfer.rs:66-96`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire/transfer.rs#L66-L96),
abridged) reads the sector and immediately queues the *entire* reply —
status byte, 256 data bytes, and a 2-byte checksum — into the outgoing
FIFO:

```rust
    pub(super) fn execute_read(&mut self, ex: bool, header: &[u8]) {
        let (drive, lsn) = self.decode_header(header);
        match self.read_sector(drive, lsn) {
            Ok(sector) => {
                self.sectors_read += 1;
                self.reply.push_back(error::OK);
                let checksum = checksum_of(&sector);
                self.reply.extend(sector);
                self.reply.push_back((checksum >> 8) as u8);
                self.reply.push_back((checksum & 0xFF) as u8);
            }
            Err(status) => self.reply.push_back(status),
        }
    }
```

And back on the CoCo side, the client driver drains that reply one byte
at a time from `$FF41`/`$FF42`: `status_read()` reports whether *any*
reply byte is queued, and `data_read()` pops one. Five bytes sent
(opcode + 4-byte header), 259 bytes received (status + 256 data + 2-byte
checksum) — the whole transaction, from the CPU's point of view, is a
loop polling `$FF41` and reading `$FF42` whenever it's nonzero, no
different in spirit from polling the WD1773's DRQ bit, except there's no
chip-level pacing at all: every reply byte is already sitting in the
queue the instant the header's last byte arrives.

### One LSN space, or several: HDB-DOS mode

The header those four bytes carry looks unambiguous — a drive number and
a 24-bit sector number — but the server interprets it two different ways
depending on which client is talking, and the reason is a piece of CoCo
history worth knowing
([`drivewire/transfer.rs:14-24`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire/transfer.rs#L14-L24)):

```rust
    fn decode_header(&self, header: &[u8]) -> (usize, u64) {
        let wire_drive = header[0] as usize;
        let lsn = (u64::from(header[1]) << 16) | (u64::from(header[2]) << 8) | u64::from(header[3]);
        if self.hdbdos {
            let drive = (lsn / super::HDBDOS_SECTORS_PER_DISK) as usize;
            let local_lsn = lsn % super::HDBDOS_SECTORS_PER_DISK;
            (drive, local_lsn)
        } else {
            (wire_drive, lsn)
        }
    }
```

In plain DriveWire mode the wire drive byte selects a mount slot and the
LSN indexes within that slot's image, which is what you'd design if you
were designing it today. HDB-DOS mode ignores the drive byte entirely and
carves one flat sector space into fixed-size slices instead:
`HDBDOS_SECTORS_PER_DISK` is 630, and the constant's doc comment names
the arithmetic behind that number — "35 tracks × 18 sectors/track, a
standard DECB `.dsk` geometry"
([`drivewire.rs:173-178`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire.rs#L173-L178)).
Divide the incoming LSN by 630 to get the virtual floppy; take the
remainder to get the sector within it.

That is a workaround wearing a protocol's clothes, and it exists because
Disk Extended Color BASIC was written in 1981 against a world of
35-track floppies and cannot be talked out of it. HDB-DOS is a
replacement Disk BASIC ROM that hands DECB a single enormous linear
address space and lets the far end chop it back up into disk-shaped
pieces. Two clients, one wire format, and a mode bit deciding which of
two arithmetics applies to the same four bytes. Section 13.13's reading
lab uses `hdbdw3bc3.rom` for exactly this path.

### The checksum that isn't a CRC

One small, deliberately honest naming mismatch is worth calling out,
because it's exactly the kind of detail that trips up anyone who assumes
a name describes an algorithm:

```rust
/// Plain 16-bit sum of a 256-byte sector's bytes. Despite [`error::CRC`]'s
/// name, DriveWire's "checksum" is this trivial running sum, not a CRC: all
/// bytes 0xFF sums to `256 * 255 = 65_280`, which fits in a `u16` with no
/// wraparound possible, so this never needs `wrapping_add`.
fn checksum_of(sector: &[u8]) -> u16 {
    sector.iter().map(|&b| u16::from(b)).sum()
}
```

The wire protocol's own error code is called `CRC` (`0xF3`), a name that
survives from whatever DriveWire's original authors decided to call it
decades ago — but the actual algorithm behind it is the plainest checksum
imaginable, no polynomial, no table, nothing collision-resistant about
it. This is the honest cost of the "distance" corner of the triangle made
concrete: DriveWire needs *some* way to detect a corrupted transmission,
because unlike the WD1773 (wired to the media it reads) or VHD (a direct
function call), a client and server genuinely can't see each other's
state — but "some way" doesn't have to be sophisticated to do its job.
`execute_write` ([`drivewire/transfer.rs:98-113`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire/transfer.rs#L98-L113)) shows the other half of
the same defense: the *server* verifies the checksum the client sent —
`if received != checksum_of(sector) { self.reply.push_back(error::CRC);
return; }` — and rejects the write outright on a mismatch, before
touching the image at all.

### READEX: making the client prove it heard correctly

A plain `READ` puts the burden of verification on the client. The server
sends its status byte, 256 data bytes, and its own two-byte sum, and
considers the transaction finished; if the client's arithmetic disagrees
it retries with `REREAD`, which has identical wire behaviour and exists
purely so the two ends can tell a retry from a fresh request. The
`READEX` family inverts that. The server sends 256 bytes with no status
byte in front of them, waits for the *client's* checksum, compares, and
only then answers with a single byte saying how the whole thing went
([`drivewire.rs:88-93`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire.rs#L88-L93)).

Two states of the protocol machine implement that, and the second one
carries the pending verdict across the gap
([`drivewire/protocol.rs:177-186`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire/protocol.rs#L177-L186)):

```rust
    fn feed_read_ex_checksum(&mut self, expected: u16, pending_error: u8, mut buf: Vec<u8>, byte: u8) {
        buf.push(byte);
        if buf.len() == 2 {
            let client_sum = (u16::from(buf[0]) << 8) | u16::from(buf[1]);
            let status = if client_sum != expected { error::CRC } else { pending_error };
            self.reply.push_back(status);
        } else {
            self.state = State::AwaitReadExChecksum { expected, pending_error, buf };
        }
    }
```

`expected` is the server's own sum of what it sent; `pending_error` is
what the read actually did. A checksum mismatch overrides the outcome
with `error::CRC`, because a client that misheard the data has no
business being told the read succeeded. Everything else passes through.

The detail that makes this a genuine wire protocol rather than a function
call in disguise is what `execute_read`'s `ex` branch does when the read
*fails*: it sends 256 zero bytes anyway
(`Err(status) => ([0u8; SECTOR_SIZE], status)`), then goes on waiting for
the client's checksum. There is no way to shorten the reply. The client
driver is already committed — it has issued a READEX and will pull
exactly 256 data bytes off the wire before it sends anything back — and a
server that reported the error by sending fewer bytes would leave the two
ends permanently disagreeing about where the next transaction starts.
Reporting the failure one byte
late, in the slot the protocol reserved for it, is strictly better than
reporting it early and desynchronizing the stream. That constraint has no
analogue anywhere in §13.2–13.10: the WD1773 can just set a status bit
and stop, because nothing is counting its bytes from the other end of a
cable.

### The timeout: recovering from a byte that never arrives

The last problem unique to this corner of the triangle: what happens if
the client sends a `READ` opcode, starts a header, and then — client
crash, cable pulled, whatever — simply never sends the rest? A real chip
doesn't have this problem (there's no "the wire went quiet mid-command"
state for a WD1773; the CPU is the one driving every access).

Worse, the state machine cannot rescue itself, and that is by design
rather than by oversight. The `State` enum's own doc comment states the
rule
([`drivewire/protocol.rs:16-23`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire/protocol.rs#L16-L23)):
"An opcode byte is only ever parsed from `State::Idle` — a byte arriving
mid-transaction is always consumed as more of that transaction's payload,
never reinterpreted as a fresh opcode." That rule is not negotiable,
because sector data is arbitrary bytes: a WRITE body's 256 payload bytes
will routinely contain values that also happen to be valid opcodes, and a
server that watched for them would corrupt every write containing the
byte `$52`. The same comment draws the pleasant consequence — the three
`RESET` opcodes "need no special handling beyond being normal opcodes,"
since by the time any opcode is parsed at all, whatever transaction there
was has already ended.

So the resynchronization has to come from outside the byte stream, and
DriveWire gets it from the one thing both ends share: elapsed time. The
mechanism is a cycle-timestamped watchdog:

```rust
/// DriveWire transaction timeout, in seconds: 250 ms of CPU time with no
/// byte from the client aborts an in-progress transaction.
const TRANSACTION_TIMEOUT_SECONDS: f64 = 0.25;
pub const TRANSACTION_TIMEOUT_CYCLES: u64 = (MAX_CPU_HZ * TRANSACTION_TIMEOUT_SECONDS) as u64;

    pub fn data_write(&mut self, byte: u8, cycle: u64) {
        if let Some(prev) = self.last_byte_cycle {
            let idle = matches!(self.state, State::Idle);
            if !idle && cycle.wrapping_sub(prev) > TRANSACTION_TIMEOUT_CYCLES {
                self.state = State::Idle;
            }
        }
        self.last_byte_cycle = Some(cycle);
        self.feed(byte);
    }
```

`TRANSACTION_TIMEOUT_CYCLES` is `1_789_772.5 * 0.25 = 447_443.125`,
truncated — a quarter second of CPU time at the fastest CoCo 3 speed
poke. Every byte fed in carries the CPU's own cycle count, and if the
*previous* byte arrived longer ago than that while a transaction was
still mid-flight, the server gives up on it — resets to `Idle` — before
parsing the new byte, so an abandoned transaction can't wedge the server
into permanently misreading every future byte as leftover payload from a
conversation that's over. `cycle.wrapping_sub(prev)`, not a bare
subtraction, is the same defensive habit you've seen since week 1:
correct under `u64` wraparound, where a bare `-` would panic in a debug
build the one time it mattered.

> **Rust corner: enum states with embedded payload make illegal states
> unrepresentable.** A C implementation of this protocol would likely use
> a byte counter (`bytes_expected: usize`) plus a separate opcode-type
> field plus a shared scratch buffer, trusting all three to stay in sync
> by hand — nothing stops `bytes_expected` from disagreeing with what the
> opcode field says it should be. Rust's enum lets each state *carry
> exactly the data that state needs, and no other data*:
> `AwaitReadHeader { ex: bool, buf: Vec<u8> }` can't accidentally be
> inspected for a WRITE body's partial buffer, because there is no field
> for one — the compiler's exhaustiveness check on `match state { ... }`
> forces every state to be handled. Same "make the type say what's true"
> discipline as `Box<[u8]>` in week 1, applied to *control flow*.

> **Rust corner: injecting the outside world through a boxed closure.**
> `coco-core` is headless (week 1's crate-boundary argument), but
> `opcode::TIME` needs *some* notion of wall-clock time:
> `pub type DwClock = Box<dyn FnMut() -> DwTime + Send>;` is a trait
> object over *any* closure with that signature — `coco-egui`, which
> knows how to ask the OS for the time, constructs one and hands it to
> `DwServer::set_clock`; `coco-core` itself only ever uses
> `default_clock`, a fixed stand-in date, so tests stay reproducible
> without a real clock dependency. `Send` matters because a `Machine`
> might cross threads; a closure capturing non-`Send` state couldn't make
> that trip. And because a closure has no serializable shape, the field
> needs its own save-state escape hatch, `#[serde(skip, default =
> "default_dw_clock")]` — one layer past week 1's plain `#[serde(skip)]`
> (the framebuffer, §1.4): `skip` alone would require `Default`, which a
> boxed closure doesn't implement, so `default = "path"` names the
> function to call instead when a restored snapshot needs a value for a
> field it never serialized.

---

## 13.12 Fidelity: functional, not cycle-exact — and what that costs

Every module in this chapter states the same fidelity choice in its own
words, and it's worth collecting them side by side now that you've read
the code behind each one:

- WD1773: "Modelled functionally rather than cycle-exact: command
  completion and byte transfers are paced by `tick()` against fixed cycle
  counts... not the real chip's per-command timing tables."
- Type I settle delay: "Not a hardware timing figure — real seeks take
  milliseconds and depend on the step-rate field we don't model; this
  just keeps BUSY observably nonzero for a short, deterministic span."
- Write Track byte budget: "headroom above DSKINI 1.1's own 6,280-byte
  double-density track template" — a real number, but a *ceiling*, not a
  claim about exactly how many gap bytes a real drive writes.

What **is** paced, precisely enough that real ROM code depends on it
working: the ~32 µs byte interval during a Type II/III transfer
(`DRQ_INTERVAL_CYCLES`), the search latency before a transfer's first byte
(`FIRST_BYTE_LATENCY_CYCLES`), and the trailing gap between the last DRQ
and INTRQ (`CRC_TRAILER_CYCLES`) — three numbers, each one load-bearing
for a specific documented driver behavior (a boot ROM's setup delay, the
FD-502's halt/NMI ordering), not "realism" for its own sake.

What is **not** paced at all: rotational latency (how long the head waits
for the target sector to spin under it, which on real media is anywhere
from zero to a full revolution depending on luck), head settle time after
a seek (Restore and Seek both complete in the same fixed 64 cycles
regardless of how many tracks the head actually crossed), and anything
resembling the real chip's internal timing tables for different step
rates — `COMMAND_SETTLE_CYCLES`'s own doc comment says the quiet part
outright: its only job is to keep BUSY observably nonzero for "a short,
deterministic span," long enough that software polling BUSY sees it set
at least once, short enough that no test suite waits around for realism
nobody asked for.

What would notice? Software that assumes seeking N tracks takes
proportionally longer than seeking 1 (some copy-protection schemes timed
exactly this, historically, to detect an emulator or a modified drive) —
this emulator's fixed 64-cycle settle for every seek regardless of
distance would make such a check trivially fail. Likewise, anything that
reads the gap *between* sectors deliberately, or depends on a specific
rotational position rather than "whichever sector I asked for,
eventually" — this emulator has no notion of rotational position at all;
`sector_offset` answers instantly and correctly regardless of where a
real head would physically be. Those are explicitly out of scope, the
same way [`DESIGN.md`](https://github.com/sperano/cocovm/blob/main/DESIGN.md)'s fidelity philosophy frames every choice in this
codebase: tighten a fidelity level only when a real, specific piece of
software needs it, and until then spend the emulation-effort budget where
it's already proven to matter — which for disks turned out to be exactly
the three constants named above, each one earned by a real boot failure
this codebase's own commit history fixed.

---

## 13.13 Reading assignment

In this order:

1. **[`crates/coco-core/src/wd1773.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773.rs), all of it** — module doc comment,
   status-bit layout, pacing constants with their doc comments (§13.2,
   §13.4), the struct, and `tick()`. Every other file refers back to it.
2. **[`crates/coco-core/src/wd1773/command.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/command.rs)** — command dispatch, Type
   I in full (§13.3), and the start of Types II/III (§13.4–13.6).
3. **[`crates/coco-core/src/wd1773/transfer.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773/transfer.rs)** — the byte-pacing
   machinery and the Write Track MFM parser (§13.6).
4. **[`crates/coco-core/src/fdc/disk_cart.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/disk_cart.rs)** — DSKREG (§13.8) and
   `update_lines`, the exact mechanism §13.7 walks.
5. **[`crates/coco-core/src/machine/run.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs), `step_cpu_unit`** — reread it
   now that you know what's on the other side of `halt_asserted`. This is
   the payoff of week 6's setup; read it slowly.
6. **[`crates/coco-core/src/fdc/jvc.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/jvc.rs)** — geometry, the sector-offset
   formula, and the OS-9 sniff (§13.9).
7. **[`crates/coco-core/src/vhd.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/vhd.rs) and [`crates/coco-core/src/bus/
   vhd_bridge.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/bus/vhd_bridge.rs)** — the minimal counterexample (§13.10).
8. **[`crates/coco-core/src/drivewire.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire.rs),
   [`crates/coco-core/src/drivewire/protocol.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire/protocol.rs),
   [`crates/coco-core/src/drivewire/transfer.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/drivewire/transfer.rs)** — the wire protocol
   (§13.11). Stop at the module boundary — worth noticing how far you can
   read before you hit anything CPU-shaped, since the protocol engine
   proper is host- and bus-free by design.

While reading, run what you can without `roms/`: `cargo test -p coco-core
--test fdc wd1773::`, `--test fdc image_geometry::`, `--test fdc
write_track::`, `--test halt`, and `--test drivewire_bus` — all five pass
with nothing beyond this repository checkout. If you have
`roms/disk11.rom`, `roms/coco3.rom`, and (for one test)
`disks/NOS9_6809_L2_v030300_coco3_40d_1.dsk` available locally, add
`--test fdc` (now passes in full, including [`dskreg.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/dskreg.rs) and [`boot.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/boot.rs)),
`--test vhd` (needs `roms/coco3.rom` for every single one of its tests,
no graceful skip, §13.10), and `--test drivewire_boot` (needs
`roms/coco3.rom` plus `roms/hdbdw3bc3.rom` plus two `.dsk` assets and
skips gracefully, per-test, whichever are missing).

---

## 13.14 Exercises

**13.1 — Sector-address arithmetic (recall + verify).** Using the formula
from §13.9 (`JvcDisk::sector_offset`), compute by hand the byte offset of
track 17, sector 3, side 0 in a headerless, single-sided, 35-track JVC
image (defaults: `sectors_per_track = 18`, `sector_size = 256`,
`first_sector_id = 1`, `header_len = 0`). Show your work — track-major row
number, sector index within the row, byte offset — then check it by
writing a two-line test against `JvcDisk::from_bytes(vec![0u8; 35 * 18 *
256])` and `disk.sector_offset(17, 0, 3)`. Then answer: which single field
in the formula would you need to know to redo this calculation for a
*two-sided* disk, and why does track-major/side-interleaved ordering (not
"all of side 0, then all of side 1") make that field's placement in the
formula non-obvious the first time you read it?

**13.2 — DSKREG decode drill (recall).** Without looking at
[`fdc/disk_cart.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/fdc/disk_cart.rs) again, decode DSKREG value `$AC` (binary `1010 1100`)
by hand: which drive is selected, what side, is the motor on, is
halt-enable armed, is density/NMI-enable set? Check your answer against
`selected_drive`/`selected_side`/the `dskreg` bit constants. Then decode
`$40` alone (bit 6 only, nothing else set) two different ways — once
pretending drive 0, 1, or 2 is also concurrently selected by another bit,
and once as written (no other drive-select bit set) — and explain in one
sentence why the second case means something completely different from
the first, even though bit 6 itself never changes value.

**13.3 — Sabotage: mispace the DRQ interval (sabotage — run it
yourself).** In [`crates/coco-core/src/wd1773.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/wd1773.rs), change
`const DRQ_INTERVAL_CYCLES: u32 = 30;` to `40`. This single constant also
feeds `FIRST_BYTE_LATENCY_CYCLES` (`= FIRST_SECTOR_SEARCH_BYTES *
DRQ_INTERVAL_CYCLES`) and `CRC_TRAILER_CYCLES` (`= 2 *
DRQ_INTERVAL_CYCLES`), both computed from it — but the *test* file,
[`tests/fdc/common.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/common.rs), mirrors the pacing as independent literal
constants (`DRQ_INTERVAL: u32 = 30`, and `FIRST_BYTE_LATENCY`/
`CRC_TRAILER` derived from *that* copy), not by importing the real ones.
Run `cargo test -p coco-core --test fdc` and `cargo test -p coco-core
--test halt` and `cargo test -p coco-core --test drivewire_bus`. Confirm:
exactly five *additional* tests fail in [`fdc/wd1773.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/wd1773.rs) (beyond whatever
already fails for missing ROMs in your checkout) —
`read_sector_delivers_256_correct_bytes_paced_by_drq_then_intrq`,
`read_sector_first_byte_waits_out_the_driver_setup_delay`,
`read_sector_samples_side_when_the_data_field_streams_not_at_dispatch`,
`write_sector_round_trips_into_the_image`, and
`multiple_read_increments_the_sector_register_then_rnf_past_the_last_
sector` — while `write_track.rs`, `image_geometry.rs`, `halt.rs`, and
`drivewire_bus.rs` all stay fully green. Read the first assertion failure
(`"DRQ must be asserted for byte 0"`) and explain in your own words why a
test that ticks by a *mirrored* constant, not the module's real one,
is exactly the right test to catch a pacing regression like this — and
why the fact that `write_track.rs`/`halt.rs`/`drivewire_bus.rs` are
*unaffected* is itself evidence for this chapter's three-corners
argument. Then revert your one-character edit and confirm `git status`
shows a clean tree again.

**13.4 — Build: a new JVC geometry test (build).** Using only the public
`JvcDisk` API, write a new test in [`tests/fdc/image_geometry.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/image_geometry.rs) for a
geometry this file doesn't already cover: an explicit 4-byte header
(`sectors_per_track=10, sides=1, size_code=0` → 128-byte sectors,
`first_sector_id=5`) and confirm `sector_offset` resolves sector IDs 5
through 14 correctly and rejects 4 and 15 (headerless images can't
express a non-default `first_sector_id`, which is why this needs an
explicit header). This needs no ROM and runs in any checkout. If you
*do* have `roms/` available, a further build worth attempting: add a
`sectors_read: u64` counter to `WD1773` (direct precedent to copy from:
`DwServer::sectors_read`, §13.11), increment it in `finish_transfer` on a
successful `ReadSector` completion, expose it through `DiskCart`, and use
[`examples/disk_boot_probe.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/examples/disk_boot_probe.rs) to print how many sectors a Disk BASIC
`DIR` actually reads.

**13.5 — Read and predict: the one-instruction HALT/NMI delay (read +
predict).** Reread [`machine/run.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/src/machine/run.rs)'s `step_cpu_unit` (§13.7) and its
`prev_halted` guard closely, without running anything yet. Predict: if
you deleted the `if !self.prev_halted { ... }` check (so interrupts are
always polled immediately after HALT* releases, even on the very first
post-halt instruction), would [`tests/halt.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/halt.rs)'s
`halt_line_stops_the_cpu_and_nmi_fires_on_release` test still pass? Write
down your reasoning — specifically, what does the synthetic ROM's `INC
$0400; BRA *-3` loop body have (or not have) in common with DSKCON's
`LDA $FF4B` / `STB ,X+` pair, and does that difference matter to this
particular test's assertions? Then — only if you have `roms/coco3.rom`
and `roms/disk11.rom` locally — actually make the edit and run both
`cargo test -p coco-core --test halt` and `cargo test -p coco-core --test
fdc boot::loadm_preserves_every_sector_byte_across_the_halt_nmi_handshake`
to check your prediction against both tests. Either way, revert the edit
before moving on, and if you don't have the ROMs, say so honestly and
leave your prediction as reasoned-through rather than verified — the same
distinction §13.9 draws about `real_nitros9_40_track_disk_parses_as_
40_tracks_2_sides`.

**13.6 — Read the trace, predict the failure: DSKREG halt-enable
(read + predict, no ROM needed).** Without running anything, read
[`tests/fdc/dskreg.rs`](https://github.com/sperano/cocovm/blob/main/crates/coco-core/tests/fdc/dskreg.rs)'s `halt_line_is_not_drq_and_halt_enable` test and
`DiskCart::halt_asserted`'s one-line implementation (`!self.fdc.drq &&
self.dskreg & dskreg::HALT_ENABLE != 0`) side by side. The test writes
`dskreg::HALT_ENABLE` to `$FF40`, asserts `halt_asserted()` is still
*false*, then reads `$FF4B` (clearing DRQ as a side effect) and asserts
it's now *true*. Predict, before running it: what would this test's two
assertions become — which one would flip, and why — if `WD1773::
default()`'s `drq` field were changed from `true` to `false`? (Recall
`drq`'s reset-state doc comment from §13.2 before you answer, and connect
it explicitly to what "spuriously assert HALT" would mean for a freshly
reset machine that hasn't issued a single disk command yet.) Then run
`cargo test -p coco-core --test fdc dskreg::` with that one-line change
to confirm your prediction, and revert it.

**13.7 — Essay: three protocol philosophies for one job (essay,
150–250 words).** You now know the WD1773's byte-paced state machine, the
VHD's zero-ceremony register file, and DriveWire's checksummed wire
protocol well enough to compare them directly. Write a short essay
answering: if you were adding a *fourth* storage device to this emulator
— say, a modern SD-card-based interface like CoCoSDC, which real hardware
implements as a handful of registers a driver polls, backed by a
microcontroller with its own internal (but real, not instant) latency —
which corner of the triangle would it sit closest to, and why? Name at
least one design decision from each of §13.2–13.11 (a pacing constant, a
register-only command, a checksum/timeout pair) that you would or
wouldn't reuse for it, and justify each choice in terms of what real
software talking to that device would actually notice — the same
"who notices?" habit week 1's fidelity-budget exercise asked you to
build.

---

## What's next

You've now closed the loop week 6 opened: the HALT-before-interrupt
ordering in `step_cpu_unit` wasn't an arbitrary implementation detail —
it's the exact shape a real 6809's instruction-boundary interrupt
recognition forces once a device (the FD-502) is allowed to stop the CPU
mid-loop. Notice, too, that this chapter's fidelity story rhymes with
week 12's cassette chapter while landing in the opposite place: the tape
ROM counted cycles, so the cassette model had to be cycle-accurate; Disk
BASIC's driver counts *bytes*, not cycles, so the disk model only had to
be byte-paced — "functional, not cycle-exact" isn't a lower standard,
it's the standard the actual software being emulated demands, discovered
the same way every fidelity decision in this course has been: by finding
the ROM code that would notice, and matching exactly that.

Week 14 stays in Part V and finishes the storage-and-serial story, but
narrows scope in a specific way worth naming now: DriveWire's *wire
protocol* — opcodes, checksums, transactions — was this week's material,
because it's fundamentally a storage question. How that protocol's bytes
actually travel between a CoCo and a PC — bit-banged GPIO timing, a real
6551 UART, a dot-matrix printer's control codes — belongs to a different
question entirely (how do two chips exchange *arbitrary* bytes, not
specifically disk sectors), and that's week 14's subject: the bitbanger,
the ACIA 6551, and the DMP-105 printer, three rungs of the same serial
ladder DriveWire happened to ride on without ever needing to know it.
