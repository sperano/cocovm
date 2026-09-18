//! `SystemBus` — address decode for RAM/ROM through the MMU plus the fixed I/O
//! page. Implements the CPU's `Bus` trait. See `DESIGN.md` §3.
//!
//! Decodes the full machine. On CoCo 3: MMU-translated RAM, the ROM windows
//! and hardwired vector fetch, and live peripherals on the I/O page — the
//! PIAs (with the bit-banger and cassette hanging off them), GIME registers,
//! SAM strobes, the cartridge/SCS window (FDC, MPI, paks), and the
//! Becker-port and VHD intercepts. CoCo 1/2 takes the separate plain-SAM
//! whole-address decode (`sam_path`) instead. Enough to boot and run the
//! real Color BASIC ROM family (`tests/` boot suites).

mod audio_bridge;
mod io;
mod peek;
mod pins;
mod regs;
mod sam_path;
mod sync;
mod vhd_bridge;

use mc6809::Bus;
use serde::{Deserialize, Serialize};

use crate::bitbanger::BitBanger;
use crate::cart::Cart;
use crate::cassette::Cassette;
use crate::config::{MachineVariant, MemorySize};
use crate::drivewire::DWServer;
use crate::gime::{self, GIME};
use crate::joystick::Joysticks;
use crate::keyboard::Keyboard;
use crate::pia::MC6821;
use crate::sam::SAM;
use crate::vhd::VHD;

use regs::{
    CONSTANT_RAM_BASE, CONSTANT_RAM_LAST, CONSTANT_RAM_PHYS, HARDWIRED_ROM_BASE, IO_BASE, MMU_BASE,
    OPEN_BUS, ROM_WINDOW_BASE,
};

const PAGE_OFFSET_MASK: usize = 0xFF;

#[derive(Serialize, Deserialize)]
pub struct SystemBus {
    /// Which machine this bus decodes addresses for. `Bus::read`/`Bus::write`
    /// branch on this once, up front, into two independent concrete decode
    /// paths (GIME vs plain SAM) rather than a trait object. Skipped: a copy
    /// of `MachineConfig::variant`, re-derived by `snapshot::restore` so a
    /// payload can't carry two disagreeing variants.
    #[serde(skip, default = "variant_placeholder")]
    pub variant: MachineVariant,
    /// CBOR-native bytes (`#[serde(with = "serde_bytes")]`) — the single
    /// biggest snapshot payload, up to 2 MB.
    #[serde(with = "serde_bytes")]
    pub ram: Box<[u8]>,
    /// Skipped: COPYRIGHTED ROM bytes never travel through a snapshot;
    /// re-injected on restore through [`SystemBus::reattach_rom`].
    /// Deserializes to an empty `Box<[u8]>`
    /// (every read against it falls through to `OPEN_BUS` until reattached).
    #[serde(skip)]
    pub rom: Box<[u8]>,
    pub gime: GIME,
    /// MC6883 SAM primary memory map, used only on [`MachineVariant::Coco1`]/
    /// [`MachineVariant::Coco2`]. Left at its power-on state (and never
    /// consulted) on [`MachineVariant::Coco3`] — the GIME keeps its own
    /// SAM-compatibility overlay (`gime::write_sam`) for that path.
    pub sam: SAM,
    pub pia0: MC6821,
    pub pia1: MC6821,
    pub cart: Cart,
    pub vhd: VHD,
    /// The Becker-port DriveWire server ($FF41/$FF42). `None` = Becker
    /// disabled — $FF41/$FF42 fall through to cartridge dispatch exactly as
    /// before. Public like `vhd` so the frontend and tests reach it
    /// directly (mount images, enable HDB-DOS mode, etc.).
    pub drivewire: Option<DWServer>,
    pub keyboard: Keyboard,
    pub joysticks: Joysticks,
    pub cassette: Cassette,
    pub bitbanger: BitBanger,
    pub io_enabled: bool,
    /// Last sampled state of the GIME keyboard-interrupt input (true = some
    /// PA0–PA6 row line low). The EI1 source fires on its falling edge.
    kbd_line_low: bool,
    /// Monotonic CPU-cycle counter, incremented once per CPU unit in
    /// `Machine::step_cpu_unit` by that unit's cycle cost; used
    /// only to timestamp Becker-port DriveWire writes for `DWServer`'s
    /// transaction timeout — NOT a general-purpose scheduling clock, and NOT
    /// reset by [`SystemBus::new`]/power-on since it only needs to be
    /// monotonic, not meaningful in absolute terms.
    pub(crate) cycle_clock: u64,
    /// Current latched audio-input state (see `crate::audio`): kept in sync
    /// by [`SystemBus::note_audio_write`] so flushes know the line-start
    /// state without recomputing.
    pub(crate) audio_inputs: crate::audio::AudioInputs,
    /// Cycle-timestamped audio-input changes since the last line flush
    /// (`Machine::flush_line_audio` drains these each scanline).
    pub(crate) audio_events: Vec<crate::audio::AudioEvent>,
    /// Last sampled level of the cartridge's level-driven CART* interrupt
    /// ([`Cartridge::cart_interrupt`]), so [`SystemBus::poll_cart_interrupt`]
    /// acts only on transitions.
    ///
    /// [`Cartridge::cart_interrupt`]: crate::cart::Cartridge::cart_interrupt
    prev_cart_int: bool,
    /// Debugger memory watchpoints, installed by
    /// [`crate::debug::Debugger::run_until`] only while a debugged run is in
    /// flight and cleared again afterwards. `None` on the normal run path, so
    /// `read`/`write` pay a single null-check without regressing the hot path.
    /// Skipped because this is debugger-only state; `None` outside a debugged
    /// run, and `Default` (`None`) is exactly right on restore because a
    /// snapshot never resumes mid-`run_until`.
    #[serde(skip)]
    watch: Option<crate::debug::WatchTable>,
    /// First watchpoint access seen since the last [`SystemBus::clear_watch_hit`];
    /// [`SystemBus::take_watch_hit`] drains it. Only ever `Some` while `watch`
    /// is installed. Skipped for the same reason as `watch`.
    #[serde(skip)]
    watch_hit: Option<crate::debug::WatchHit>,
}

/// Serde default for the skipped [`SystemBus::variant`]; `snapshot::restore`
/// overwrites it from `MachineConfig::variant` before the bus is used.
fn variant_placeholder() -> MachineVariant {
    MachineVariant::Coco3
}

impl SystemBus {
    pub fn new(variant: MachineVariant, memory: MemorySize, rom: Box<[u8]>) -> Self {
        Self {
            variant,
            ram: vec![0u8; memory.bytes()].into_boxed_slice(),
            rom,
            gime: GIME::new(),
            sam: SAM::new(),
            pia0: MC6821::new(),
            pia1: MC6821::new(),
            cart: Cart::default(),
            vhd: VHD::new(),
            drivewire: None,
            keyboard: Keyboard::new(),
            joysticks: Joysticks::new(),
            cassette: Cassette::new(),
            bitbanger: BitBanger::new(),
            io_enabled: true,
            kbd_line_low: false,
            cycle_clock: 0,
            audio_inputs: crate::audio::AudioInputs::default(),
            audio_events: Vec::new(),
            prev_cart_int: false,
            watch: None,
            watch_hit: None,
        }
    }

    /// Restore-time fixups for `#[serde(skip)]` fields, after a snapshot
    /// round-trip. Delegates into the cartridge tree, whose skipped fields
    /// also need rebuilding.
    pub fn after_restore(&mut self) {
        self.cart.after_restore();
        if let Some(dw) = self.drivewire.as_mut() {
            dw.after_restore();
        }
    }

    /// Restore-time payload-shape validation, called once before any media
    /// is reattached. Only the cart tree needs a walk here.
    pub(crate) fn validate_restored(&self) -> Result<(), String> {
        self.cart.validate_restored()
    }

    /// Restore-path-only: re-inject the internal ROM image after a snapshot
    /// restore (`rom` is `#[serde(skip)]` — COPYRIGHTED bytes never travel through a snapshot).
    pub fn reattach_rom(&mut self, rom: Box<[u8]>) {
        self.rom = rom;
    }

    /// Enable the Becker port, if not already enabled. Idempotent — does
    /// nothing if a `DWServer` is already installed.
    pub fn enable_drivewire(&mut self) {
        if self.drivewire.is_none() {
            self.drivewire = Some(DWServer::new());
        }
    }

    /// Install the debugger's memory-watch table. An empty table installs
    /// `None` so a debugged run with no watchpoints keeps the fast path.
    pub fn install_watches(&mut self, table: crate::debug::WatchTable) {
        self.watch = (!table.is_empty()).then_some(table);
        self.watch_hit = None;
    }

    /// Remove the watch table, returning `read`/`write` to their zero-overhead
    /// path. Called on every exit from `run_until`.
    pub fn uninstall_watches(&mut self) {
        self.watch = None;
        self.watch_hit = None;
    }

    /// Forget any recorded watch hit before stepping one instruction.
    pub fn clear_watch_hit(&mut self) {
        self.watch_hit = None;
    }

    /// Take the first watchpoint access observed since the last
    /// [`SystemBus::clear_watch_hit`], if any.
    pub fn take_watch_hit(&mut self) -> Option<crate::debug::WatchHit> {
        self.watch_hit.take()
    }

    /// Record a watchpoint access if `watch` is installed and `addr`/`kind`
    /// match an enabled watch. Keeps the FIRST hit within a step.
    fn note_watch(&mut self, addr: u16, kind: crate::debug::WatchKind) {
        if self.watch_hit.is_some() {
            return;
        }
        if matches!(&self.watch, Some(w) if w.matches(addr, kind)) {
            self.watch_hit = Some(crate::debug::WatchHit { addr, kind });
        }
    }

    /// Physical RAM offset for a CPU address, masked to installed RAM.
    fn phys(&self, addr: u16) -> usize {
        debug_assert!(self.ram.len().is_power_of_two());
        let ram_mask = self.ram.len() - 1;
        // MC3: hold the $FE00 page constant at physical $7FE00 regardless of the MMU.
        if (CONSTANT_RAM_BASE..=CONSTANT_RAM_LAST).contains(&addr)
            && self.gime.init0 & gime::init0::MC3 != 0
        {
            return (CONSTANT_RAM_PHYS | (addr as usize & PAGE_OFFSET_MASK)) & ram_mask;
        }
        self.gime.translate(addr) & ram_mask
    }

    /// True when `addr` reads ROM: the `$8000–$FDFF` window when ROM is mapped,
    /// plus `$FE00–$FEFF` when INIT0 MC3 is clear (MC3 set diverts that page
    /// to constant RAM instead).
    fn is_rom_window(&self, addr: u16) -> bool {
        if !self.gime.rom_enabled() {
            return false;
        }
        if (CONSTANT_RAM_BASE..=CONSTANT_RAM_LAST).contains(&addr) {
            return self.gime.init0 & gime::init0::MC3 == 0;
        }
        (ROM_WINDOW_BASE..CONSTANT_RAM_BASE).contains(&addr)
    }

    /// Read ROM for a logical address in the `$8000–$FFFF` window. INIT0
    /// MC1:MC0 splits the window between internal ROM and external cartridge
    /// ROM (CTS*); `$FFE0–$FFFF` is exempt and always reads internal ROM
    /// (see [`HARDWIRED_ROM_BASE`]).
    fn rom_read(&mut self, addr: u16) -> u8 {
        if addr < HARDWIRED_ROM_BASE && self.gime.rom_is_external(addr) {
            return self.cart.rom_read(addr);
        }
        let off = (addr - ROM_WINDOW_BASE) as usize;
        self.rom.get(off).copied().unwrap_or(OPEN_BUS)
    }
}

/// Decode an `$FFA0–$FFAF` MMU register address to `(task, slot)`.
fn mmu_index(addr: u16) -> (usize, usize) {
    let idx = (addr - MMU_BASE) as usize;
    (idx / gime::SLOTS_PER_TASK, idx % gime::SLOTS_PER_TASK)
}

impl Bus for SystemBus {
    fn read(&mut self, addr: u16) -> u8 {
        // A single null check keeps the hot path unchanged when no watchpoints
        // are installed.
        if self.watch.is_some() {
            self.note_watch(addr, crate::debug::WatchKind::Read);
        }
        // Branch once between the two concrete decode paths instead of using a
        // trait object.
        if self.variant != MachineVariant::Coco3 {
            return self.sam_read(addr);
        }
        // Hardwired to internal ROM ahead of I/O decode, ROM mapping, and MMU state.
        if addr >= HARDWIRED_ROM_BASE {
            return self.rom_read(addr);
        }
        if self.io_enabled && addr >= IO_BASE {
            return self.io_read(addr);
        }
        if self.is_rom_window(addr) {
            return self.rom_read(addr);
        }
        let p = self.phys(addr);
        self.ram[p]
    }

    fn write(&mut self, addr: u16, val: u8) {
        if self.watch.is_some() {
            self.note_watch(addr, crate::debug::WatchKind::Write);
        }
        if self.variant != MachineVariant::Coco3 {
            self.sam_write(addr, val);
            return;
        }
        // $FFE0–$FFFF is ROM, not RAM: writes there are dropped.
        if addr >= HARDWIRED_ROM_BASE {
            return;
        }
        if self.io_enabled && addr >= IO_BASE {
            self.io_write(addr, val);
            return;
        }
        // ROM is read-only; writes to the ROM window reach the RAM mapped beneath it.
        let p = self.phys(addr);
        self.ram[p] = val;
    }
}
