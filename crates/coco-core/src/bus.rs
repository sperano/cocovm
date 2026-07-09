//! `SystemBus` — address decode for RAM/ROM through the MMU plus the fixed I/O
//! page. Implements the CPU's `Bus` trait. See `DESIGN.md` §3.
//!
//! STATUS: skeleton. Device ranges are decoded to the right peripherals, but most
//! peripheral registers are stubs and ROM mapping / vector fetch is TODO.

use mc6809::Bus;

use crate::bitbanger::{self, BitBanger};
use crate::cart::{Cartridge, EmptySlot};
use crate::cassette::Cassette;
use crate::config::{MachineVariant, MemorySize};
use crate::gime::{self, GIME};
use crate::joystick::Joysticks;
use crate::keyboard::Keyboard;
use crate::pia::MC6821;
use crate::sam::{Sam, SamTarget};
use crate::vhd::{self, Vhd};

// I/O page device ranges (`DESIGN.md` §3). PIA0/PIA1 mirror every 4 bytes.
const IO_BASE: u16 = 0xFF00;
const PIA0_LAST: u16 = 0xFF1F;
const PIA1_BASE: u16 = 0xFF20;
const PIA1_LAST: u16 = 0xFF3F;
const CART_BASE: u16 = 0xFF40;
const CART_LAST: u16 = 0xFF5F;
// TODO: MAME gates the whole $FF40-$FF5F SCS window on GIME INIT0 MC2
// ("standard SCS" width control); not modeled here — every cartridge always
// sees the full window regardless of MC2.
/// Multi-Pak Interface select register: decoded by the MPI itself (when one
/// is inserted), never by the plugged-in cartridges' own `read`/`write` — see
/// [`Cartridge::control_read`]. `$FF60-$FF7E` stays open bus.
const MPI_CONTROL_REG: u16 = 0xFF7F;
// VHD (virtual hard disk, NitrOS-9 `emudsk`) register window — see `vhd.rs`.
// $FF87-$FF8F stays open-bus/unmapped.
const VHD_LRN_HI: u16 = 0xFF80;
const VHD_LRN_MID: u16 = 0xFF81;
const VHD_LRN_LO: u16 = 0xFF82;
const VHD_COMMAND_STATUS: u16 = 0xFF83;
const VHD_BUFFER_HI: u16 = 0xFF84;
const VHD_BUFFER_LO: u16 = 0xFF85;
const VHD_SELECT: u16 = 0xFF86;
const INIT0_REG: u16 = 0xFF90;
const INIT1_REG: u16 = 0xFF91;
/// IRQ enable/status register (write = enables, read = latched status).
const IRQENR_REG: u16 = 0xFF92;
/// FIRQ enable/status register (write = enables, read = latched status).
const FIRQENR_REG: u16 = 0xFF93;
const TIMER_MSB_REG: u16 = 0xFF94;
const TIMER_LSB_REG: u16 = 0xFF95;
/// $FF96/$FF97 are reserved on the GIME.
const GIME_RESERVED_BASE: u16 = 0xFF96;
const GIME_RESERVED_LAST: u16 = 0xFF97;
const VMODE_REG: u16 = 0xFF98;
const VRES_REG: u16 = 0xFF99;
const BORDER_REG: u16 = 0xFF9A;
const VBANK_REG: u16 = 0xFF9B;
const VSCROLL_REG: u16 = 0xFF9C;
const VOFFSET1_REG: u16 = 0xFF9D;
const VOFFSET0_REG: u16 = 0xFF9E;
const HOFFSET_REG: u16 = 0xFF9F;
const GIME_LAST: u16 = 0xFF9F;
const MMU_BASE: u16 = 0xFFA0;
const MMU_LAST: u16 = 0xFFAF;
const PALETTE_BASE: u16 = 0xFFB0;
const PALETTE_LAST: u16 = 0xFFBF;

/// Base of the ROM window. `$8000–$FFFF` reads return ROM when it is mapped, with
/// the fixed I/O page overlaid on top of `$FF00–$FFEF` (`DESIGN.md` §3).
const ROM_WINDOW_BASE: u16 = 0x8000;
/// `$FE00–$FEFF` — the interrupt-vector page. INIT0 MC3 selects its mapping
/// (SEB Unravelled II; MAME `gime.cpp update_memory` bank 8): MC3=1 pins it to
/// constant RAM at physical `$7FE00` regardless of the MMU or ROM mode (BASIC
/// boots with MC3 set and writes its JMP trampolines here); MC3=0 makes it
/// follow the normal map like the rest of the `$8000+` window — MMU RAM in
/// all-RAM mode, ROM in ROM mode (internal or cartridge per MC1:MC0, as the
/// tail of the `$E000` bank). Sokoban relies on the MC3=0 ROM path: it is the
/// only way to address the last `$200` bytes of a pak image (CTS stops at
/// `$FDFF`), where it keeps its palette tables.
const CONSTANT_RAM_BASE: u16 = 0xFE00;
const CONSTANT_RAM_LAST: u16 = 0xFEFF;
/// Physical base of the constant `$FE00` page when INIT0 MC3 is set.
const CONSTANT_RAM_PHYS: usize = 0x7_FE00;
/// `$FFE0–$FFFF` — the top 32 bytes of the `$8000–$FFFF` window, including the
/// 6809 hardware vectors — is hardwired to internal ROM on every read,
/// regardless of INIT0 MC1:MC0, the SAM TY map-type bit (`$FFDE`/`$FFDF`,
/// all-RAM mode), MMU state, or any inserted cartridge. MAME `coco3.cpp:53-58`
/// documents this as verified by William Astle's real-hardware test, which
/// refutes SEB Unravelled II p.28's claim that this range aliases `$BFFx`.
/// Writes here are dropped (`SystemBus::write`) — it isn't backed by RAM.
const HARDWIRED_ROM_BASE: u16 = 0xFFE0;

const OPEN_BUS: u8 = 0xFF;

/// Plain-SAM path only: flat 32K ROM image offset where Color BASIC starts —
/// extbas at 0, bas at $2000 (`docs/coco12-plan.md` "ROM files").
const SAM_BAS_ROM_OFFSET: usize = 0x2000;
/// Plain-SAM path only: base CPU address of the cartridge CTS* ROM window,
/// added back to a `SamTarget::Cart` offset before calling
/// [`Cartridge::rom_read`].
const SAM_CART_ROM_BASE: u16 = 0xC000;

pub struct SystemBus {
    /// Which machine this bus decodes addresses for. `Bus::read`/`Bus::write`
    /// branch on this once, up front, into two independent concrete decode
    /// paths (GIME vs plain SAM) rather than a trait object — see
    /// `docs/coco12-plan.md` Phase 2.
    pub variant: MachineVariant,
    pub ram: Box<[u8]>,
    pub rom: Box<[u8]>,
    pub gime: GIME,
    /// MC6883 SAM primary memory map, used only on [`MachineVariant::Coco1`]/
    /// [`MachineVariant::Coco2`]. Left at its power-on state (and never
    /// consulted) on [`MachineVariant::Coco3`] — the GIME keeps its own
    /// SAM-compatibility overlay (`gime::write_sam`) for that path.
    pub sam: Sam,
    pub pia0: MC6821,
    pub pia1: MC6821,
    pub cart: Box<dyn Cartridge>,
    pub vhd: Vhd,
    pub keyboard: Keyboard,
    pub joysticks: Joysticks,
    pub cassette: Cassette,
    pub bitbanger: BitBanger,
    pub io_enabled: bool,
    /// Last sampled state of the GIME keyboard-interrupt input (true = some
    /// PA0–PA6 row line low). The EI1 source fires on its falling edge.
    kbd_line_low: bool,
}

impl SystemBus {
    pub fn new(variant: MachineVariant, memory: MemorySize, rom: Box<[u8]>) -> Self {
        Self {
            variant,
            ram: vec![0u8; memory.bytes()].into_boxed_slice(),
            rom,
            gime: GIME::new(),
            sam: Sam::new(),
            pia0: MC6821::new(),
            pia1: MC6821::new(),
            cart: Box::new(EmptySlot),
            vhd: Vhd::new(),
            keyboard: Keyboard::new(),
            joysticks: Joysticks::new(),
            cassette: Cassette::new(),
            bitbanger: BitBanger::new(),
            io_enabled: true,
            kbd_line_low: false,
        }
    }

    /// PIA0 port-A input pins: keyboard rows for the current column strobe,
    /// fire buttons pulling their rows low regardless of the strobe, and the
    /// joystick comparator on PA7 — high while the 6-bit DAC (PIA1 PA2–PA7)
    /// is at or below the pot the CA2/CB2 mux selects (`DESIGN.md` §7).
    fn pia0_pa_pins(&self) -> u8 {
        const COMPARATOR_BIT: u8 = 0x80;
        let mut pa = self.keyboard.sense(self.pia0.b.output);
        pa &= !self.joysticks.button_rows();
        let axis = usize::from(self.pia0.a.c2_output()); // SEL1: 0 = X, 1 = Y
        let stick = usize::from(self.pia0.b.c2_output()); // SEL2: 0 = right
        let dac = (self.pia1.a.output & 0xFC) >> 2;
        if self.joysticks.compare(stick, axis, dac) {
            pa |= COMPARATOR_BIT;
        } else {
            pa &= !COMPARATOR_BIT;
        }
        pa
    }

    /// PIA1 port-A input pins: only bit 0 (cassette data in, `$FF20` —
    /// Service Manual / `cassette-verified-facts`) is driven by anything
    /// emulated; the rest float high like every other unused CoCo input pin
    /// ([`crate::pia::PiaPort`]'s default).
    fn pia1_pa_pins(&self) -> u8 {
        const CASSETTE_IN: u8 = 0x01;
        if self.cassette.input_bit() {
            0xFF
        } else {
            !CASSETTE_IN
        }
    }

    /// PIA1 port-B input pins. Two bits are driven by anything emulated; the
    /// rest float high like every other unused CoCo input pin
    /// ([`crate::pia::PiaPort`]'s default).
    ///
    /// Bit 0 (printer BUSY in, `$FF22` — `bitbanger-spec.md` "Register map"),
    /// on every variant. Polarity is 0 = ready, 1 = busy: BASIC's driver
    /// treats bit 0 set as busy and spins (`LDB $FF22 / LSRB / BCS`), so the
    /// not-busy default must present bit 0 clear or every print statement
    /// would hang.
    ///
    /// Bit 2 (RAMSZ, the memory-size sense switch Color BASIC's cold-start
    /// reads to size RAM), on CoCo 1/2 only — the CoCo 3 has no such switch,
    /// so its pin keeps floating high.
    ///
    /// MAME `coco.cpp` `pia1_pb_r`: 16K-32K RAM (`$4000..=$7FFF` bytes) senses
    /// set unconditionally; 64K (`>= $8000`) instead follows PIA0 port B's
    /// *output* register bit 6 (`b_output() & 0x40`, the raw latched value —
    /// not the pin state, so DDR doesn't matter) — Color BASIC's memory-size
    /// probe momentarily drives that bit while sensing, and since Color BASIC
    /// 1.0's sizing routine can only configure 4K/16K banks, this lets the
    /// CoCo 1 (which ships 1.0 but can take a 1.2 upgrade) reach 64K only with
    /// the newer ROM. Below 16K the switch reads clear.
    fn pia1_pb_pins(&self) -> u8 {
        const RAMSZ_BIT: u8 = 0x04; // PB2
        /// PIA0 port B bit 6 — Color BASIC's memory-size probe pin.
        const PIA0_PROBE_BIT: u8 = 0x40;
        const RAMSZ_16K_32K: std::ops::RangeInclusive<usize> = 0x4000..=0x7FFF;

        let mut pins = 0xFF;
        if !self.bitbanger.busy() {
            pins &= !bitbanger::BUSY_PIN;
        }
        if matches!(self.variant, MachineVariant::Coco1 | MachineVariant::Coco2) {
            let ram_len = self.ram.len();
            let memory_sense = RAMSZ_16K_32K.contains(&ram_len)
                || (ram_len >= 0x8000 && self.pia0.b.output & PIA0_PROBE_BIT != 0);
            if !memory_sense {
                pins &= !RAMSZ_BIT;
            }
        }
        pins
    }

    /// PA1 ($FF20) as the bit-banger's TX line sees it: mark (idle-high)
    /// unless PIA1 DDRA bit 1 is set to make PA1 an output and its output
    /// register bit is clear (space). The ROM only ever drives PA1 once it
    /// has configured it as an output (DDRA = $FE at `$A048` —
    /// `bitbanger-spec.md` "Register map"); an unconfigured PA1 is treated
    /// as idle mark, matching how a floating output pin would look to a
    /// receiver expecting idle-high (not itself asserted in the spec, since
    /// the ROM always configures DDRA before touching the printer port).
    pub(crate) fn pia1_tx_mark(&self) -> bool {
        if self.pia1.a.ddr & bitbanger::TX_PIN == 0 {
            true
        } else {
            self.pia1.a.output & bitbanger::TX_PIN != 0
        }
    }

    /// Physical RAM offset for a CPU address, masked to installed RAM.
    fn phys(&self, addr: u16) -> usize {
        // MC3: hold the $FE00 page constant at physical $7FE00 regardless of the MMU.
        if (CONSTANT_RAM_BASE..=CONSTANT_RAM_LAST).contains(&addr)
            && self.gime.init0 & gime::init0::MC3 != 0
        {
            return (CONSTANT_RAM_PHYS | (addr as usize & 0xFF)) % self.ram.len();
        }
        self.gime.translate(addr) % self.ram.len()
    }

    /// True when `addr` reads ROM: the `$8000–$FDFF` window when ROM is mapped,
    /// plus the `$FE00–$FEFF` vector page when INIT0 MC3 is clear (see
    /// [`CONSTANT_RAM_BASE`] — MC3 set diverts that page to constant RAM via
    /// [`SystemBus::phys`] instead). `$FF00+` is the I/O page / vectors.
    fn is_rom_window(&self, addr: u16) -> bool {
        if !self.gime.rom_enabled() {
            return false;
        }
        if (CONSTANT_RAM_BASE..=CONSTANT_RAM_LAST).contains(&addr) {
            return self.gime.init0 & gime::init0::MC3 == 0;
        }
        (ROM_WINDOW_BASE..CONSTANT_RAM_BASE).contains(&addr)
    }

    /// True when any interrupt source is holding the CPU IRQ line low.
    ///
    /// PIA0's output and the GIME's IRQ output are wired-OR on the CPU pin: at
    /// the BASIC prompt the stock ROM runs the legacy PIA path (INIT0 IEN=0)
    /// off PIA0's field/horizontal sync, while GIME-native software enables
    /// IEN and the $FF92 sources instead (`DESIGN.md` §4).
    pub fn irq_asserted(&self) -> bool {
        self.pia0.irq() || self.gime.irq_asserted()
    }

    /// True when any interrupt source is holding the CPU FIRQ line low: PIA1
    /// (the legacy cartridge FIRQ path) wired-OR with the GIME's FIRQ output
    /// ($FF93 sources gated by INIT0 FEN).
    pub fn firq_asserted(&self) -> bool {
        self.pia1.irq() || self.gime.firq_asserted()
    }

    /// True while the cartridge holds the CPU HALT* line low (the FD-502's
    /// sector-transfer handshake). The only HALT source on a stock CoCo 3 is
    /// the expansion port.
    pub fn halt_asserted(&self) -> bool {
        self.cart.halt_asserted()
    }

    /// Consume a pending NMI edge (the FD-502 gates FDC INTRQ onto NMI).
    pub fn take_nmi(&mut self) -> bool {
        self.cart.take_nmi()
    }

    /// Horizontal-sync line: the GIME HS pin idles high and pulses low for 16
    /// of 228 pixel clocks at line end (~4.5 µs; MAME `mc6847.cpp`
    /// `TIMER_HSYNC_OFF_TIME`=212/`ON_TIME`=228). Our per-line model has no
    /// resolution below one scanline, so both the falling and rising edges
    /// are emitted back-to-back here rather than timed within the line; each
    /// still only latches PIA0 CA1 (control reg $FF01, port A) and PIA1 CB1
    /// (see below) if it matches that side's selected edge
    /// ([`crate::pia::cr::C1_EDGE_HIGH`]), so software polling either edge —
    /// stock BASIC's falling-edge CRA or NitrOS-9's rising-edge one — still
    /// gets exactly one flag per line. The GIME HBORD source is raised once
    /// per line regardless of edge selection: GIME border sources are
    /// hardwired falling-edge only, not selectable (Lomont).
    ///
    /// Also the GIME's per-scanline sample point for the EI1
    /// keyboard-interrupt input (a zero on any PA0–PA6 row while some column
    /// is strobed — SEB Unravelled II), which fires on falling edge.
    ///
    /// Also the sample point for the expansion-port CART* line: auto-start
    /// game paks tie it to the Q clock (~895 kHz), so while one is inserted
    /// this pulses PIA1 CB1 (the legacy FIRQ cart-boot path) and raises the
    /// GIME EI0 source (the same physical pin) every scanline, which is more
    /// than enough cadence to keep either interrupt path continuously fed.
    pub fn hsync(&mut self) {
        // No GIME on the plain-SAM path (CoCo 1/2): the PIA0/PIA1 Cx1 pulses
        // below stay exactly as-is, but the GIME border/keyboard/cart-EI0
        // interrupt sources it would also raise here don't exist — the GIME
        // struct must stay completely inert on that path
        // (`docs/coco12-plan.md` Phase 4).
        let is_gime = self.variant == MachineVariant::Coco3;
        self.pia0.a.set_c1(false);
        if is_gime {
            self.gime.raise(gime::intr::HBORD);
        }
        self.pia0.a.set_c1(true);
        // Buttons are included: SEB warns joystick fire buttons always trip EI1.
        let line_low = self.pia0_pa_pins() & 0x7F != 0x7F;
        if is_gime && line_low && !self.kbd_line_low {
            self.gime.raise(gime::intr::EI1);
        }
        self.kbd_line_low = line_low;
        if self.cart.cart_line_ties_q() {
            self.pia1.b.set_c1(false);
            if is_gime {
                self.gime.raise(gime::intr::EI0);
            }
            self.pia1.b.set_c1(true);
        }
    }

    /// Field-sync falling edge (~60/50 Hz vertical, at
    /// [`crate::config::VideoStandard::fs_falling_line`] scanlines into the
    /// field, not at end-of-field): latches PIA0 CB1 (control reg $FF03, port
    /// B) — the interrupt that drives BASIC's housekeeping loop — per its
    /// selected edge, and raises the GIME VBORD source (Lomont: "VBORD
    /// generated on falling edge of VSYNC").
    pub fn fs_falling(&mut self) {
        self.pia0.b.set_c1(false);
        // No GIME (hence no VBORD source) on the plain-SAM path — Phase 4.
        if self.variant == MachineVariant::Coco3 {
            self.gime.raise(gime::intr::VBORD);
        }
    }

    /// Field-sync rising edge, at
    /// [`crate::config::VideoStandard::fs_rising_line`] scanlines into the
    /// field: latches PIA0 CB1 per its selected edge (e.g. NitrOS-9-style
    /// rising-edge polling). No GIME border source is tied to this edge.
    pub fn fs_rising(&mut self) {
        self.pia0.b.set_c1(true);
    }

    /// Instantaneous speaker level, 0.0–1.0.
    ///
    /// Sources mix on the CoCo 3 (it has no sound chip of its own): the 6-bit
    /// DAC (PIA1 PA2–PA7), routed through the analog mux only when SNDEN
    /// (PIA1 CB2) is high and the SEL2:SEL1 selects (PIA0 CB2:CA2) are 00;
    /// mux state 01 routes cassette playback (the squared tape signal — the
    /// key-click of a real CLOAD), 10 routes the cartridge SND pin (not
    /// emulated, silent — same stub as MAME `coco.cpp` `update_sound`), 11
    /// is grounded; and the single-bit sound on PIA1 PB1, which is always
    /// connected. (Tandy Service Manual mux table via MAME `coco.cpp`
    /// `update_sound`; SEB Unravelled II $FF22/$FF23.)
    ///
    /// Cartridge audio via [`Cartridge::sound_level`] (the GMC's SN76489A)
    /// mixes in unconditionally: MAME routes such carts to a speaker of
    /// their own, ignoring SOUND_ENABLE and the mux entirely (its mux
    /// cart-sound path is an explicit "NYI" stub), and we keep that
    /// behaviour.
    ///
    /// [`Cartridge::sound_level`]: crate::cart::Cartridge::sound_level
    pub fn sound_sample(&self) -> f32 {
        /// Relative loudness of the full-scale DAC vs the single-bit beeper.
        const DAC_GAIN: f32 = 0.75;
        const SINGLE_BIT_GAIN: f32 = 0.25;
        /// Cartridge audio at the same full-scale loudness as the internal
        /// 6-bit DAC.
        const CART_GAIN: f32 = 0.75;
        /// PIA1 PB1: the single-bit sound output.
        const SINGLE_BIT: u8 = 0x02;
        const DAC_MAX: f32 = 63.0;
        /// Tape playback through the mux: a square wave (the SALT detector's
        /// output), kept below the DAC's full scale like the real attenuated
        /// tape level.
        const CASSETTE_GAIN: f32 = 0.35;
        /// SEL2:SEL1 = 01: the mux's cassette input.
        const SEL_CASSETTE: u8 = 0b01;

        let mut level = 0.0;
        let sel = u8::from(self.pia0.b.c2_output()) << 1 | u8::from(self.pia0.a.c2_output());
        if self.pia1.b.c2_output() && sel == 0 {
            let dac = (self.pia1.a.output & self.pia1.a.ddr & 0xFC) >> 2;
            level += DAC_GAIN * f32::from(dac) / DAC_MAX;
        }
        if self.pia1.b.c2_output()
            && sel == SEL_CASSETTE
            && self.pia1.a.c2_output()
            && self.cassette.playing()
            && self.cassette.input_bit()
        {
            level += CASSETTE_GAIN;
        }
        if self.pia1.b.output & self.pia1.b.ddr & SINGLE_BIT != 0 {
            level += SINGLE_BIT_GAIN;
        }
        level += CART_GAIN * self.cart.sound_level();
        level
    }

    /// Read ROM for a logical address in the `$8000–$FFFF` window.
    ///
    /// INIT0 MC1:MC0 splits the window between internal ROM (image offset
    /// `addr - $8000`) and the external cartridge ROM (CTS*), which an empty
    /// slot answers with open-bus $00 — matching MAME trace-diff behaviour.
    /// `$FFE0–$FFFF` is exempt: it always reads internal ROM (see
    /// [`HARDWIRED_ROM_BASE`]), so callers route that range here directly and
    /// we skip the external-cartridge check for it.
    fn rom_read(&mut self, addr: u16) -> u8 {
        if addr < HARDWIRED_ROM_BASE && self.gime.rom_is_external(addr) {
            return self.cart.rom_read(addr);
        }
        let off = (addr - ROM_WINDOW_BASE) as usize;
        self.rom.get(off).copied().unwrap_or(OPEN_BUS)
    }

    fn io_read(&mut self, addr: u16) -> u8 {
        match addr {
            IO_BASE..=PIA0_LAST => {
                // Refresh port A's input pins (keyboard rows + joystick
                // comparator/buttons) before the PIA read.
                self.pia0.a.input = self.pia0_pa_pins();
                self.pia0.read((addr & 0x03) as u8)
            }
            PIA1_BASE..=PIA1_LAST => {
                self.pia1.a.input = self.pia1_pa_pins();
                self.pia1.b.input = self.pia1_pb_pins();
                self.pia1.read((addr & 0x03) as u8)
            }
            CART_BASE..=CART_LAST => self.cart.read(addr),
            MPI_CONTROL_REG => self.cart.control_read(),
            VHD_LRN_HI | VHD_LRN_MID | VHD_LRN_LO | VHD_BUFFER_HI | VHD_BUFFER_LO => {
                self.vhd.read_lrn_or_buffer()
            }
            VHD_COMMAND_STATUS => self.vhd.read_status(),
            VHD_SELECT => OPEN_BUS, // always open bus, unconditionally (spec)
            INIT0_REG => self.gime.init0,
            INIT1_REG => self.gime.init1,
            IRQENR_REG => self.gime.read_irq_status(),
            FIRQENR_REG => self.gime.read_firq_status(),
            TIMER_MSB_REG..=GIME_LAST => 0, // timer/video regs are write-only on HW
            MMU_BASE..=MMU_LAST => {
                let (task, slot) = mmu_index(addr);
                self.gime.mmu[task][slot] & gime::MMU_READ_MASK
            }
            PALETTE_BASE..=PALETTE_LAST => self.gime.palette[(addr - PALETTE_BASE) as usize],
            _ => OPEN_BUS, // SAM-compat / spare / unmapped — TODO
        }
    }

    fn io_write(&mut self, addr: u16, val: u8) {
        match addr {
            IO_BASE..=PIA0_LAST => self.pia0.write((addr & 0x03) as u8, val),
            PIA1_BASE..=PIA1_LAST => {
                self.pia1.write((addr & 0x03) as u8, val);
                // Cassette record-out is a direct, unconditional tap of the DAC
                // (not gated by SNDEN/the mux — `cassette-verified-facts`), fed
                // on every PIA1 write since any of them (port A output/DDR or
                // CRA, which carries the motor relay) can change it.
                let dac = (self.pia1.a.output & self.pia1.a.ddr & 0xFC) >> 2;
                self.cassette.record_dac(dac, self.pia1.a.c2_output());
            }
            CART_BASE..=CART_LAST => self.cart.write(addr, val),
            MPI_CONTROL_REG => self.cart.control_write(val),
            VHD_LRN_HI => self.vhd.write_lrn_hi(val),
            VHD_LRN_MID => self.vhd.write_lrn_mid(val),
            VHD_LRN_LO => self.vhd.write_lrn_lo(val),
            VHD_COMMAND_STATUS => self.vhd_execute_command(val),
            VHD_BUFFER_HI => self.vhd.write_buffer_hi(val),
            VHD_BUFFER_LO => self.vhd.write_buffer_lo(val),
            VHD_SELECT => self.vhd.write_select(val),
            INIT0_REG => self.gime.write_init0(val),
            INIT1_REG => self.gime.write_init1(val),
            IRQENR_REG => self.gime.write_irq_enable(val),
            FIRQENR_REG => self.gime.write_firq_enable(val),
            TIMER_MSB_REG => self.gime.write_timer_msb(val),
            TIMER_LSB_REG => self.gime.write_timer_lsb(val),
            VMODE_REG => self.gime.vmode = val,
            VRES_REG => self.gime.vres = val,
            BORDER_REG => self.gime.border = val,
            VBANK_REG => self.gime.video_bank = val,
            VSCROLL_REG => self.gime.vertical_scroll = val,
            VOFFSET1_REG => {
                self.gime.vertical_offset =
                    (self.gime.vertical_offset & 0x00FF) | u16::from(val) << 8;
            }
            VOFFSET0_REG => {
                self.gime.vertical_offset = (self.gime.vertical_offset & 0xFF00) | u16::from(val);
            }
            HOFFSET_REG => self.gime.horizontal_offset = val,
            GIME_RESERVED_BASE..=GIME_RESERVED_LAST => {}
            MMU_BASE..=MMU_LAST => {
                let (task, slot) = mmu_index(addr);
                self.gime.mmu[task][slot] = val; // full 8 bits stored on write
            }
            PALETTE_BASE..=PALETTE_LAST => {
                self.gime.palette[(addr - PALETTE_BASE) as usize] = val;
            }
            gime::SAM_BASE..=gime::SAM_LAST => self.gime.write_sam(addr),
            _ => { /* unmapped — TODO */ }
        }
    }

    /// `$FF83` write: execute a VHD command on the selected drive
    /// (`vhd::command`), synchronously, then latch the resulting status for
    /// the next `$FF83` read.
    ///
    /// Order, per spec: no drive selected -> the write is a no-op entirely
    /// (no status changes anywhere). Reentrant call (from within our own
    /// transfer loop below, via a buffer address that lands back on this same
    /// register) -> also a no-op, so the outer call's result isn't clobbered.
    /// Otherwise: an unmounted drive always reports `NO_VHD`, regardless of
    /// which command was written; only a mounted drive dispatches on the
    /// command byte.
    fn vhd_execute_command(&mut self, cmd: u8) {
        let Some(drive) = self.vhd.selected_drive() else {
            return;
        };
        if self.vhd.busy {
            return;
        }
        self.vhd.busy = true;

        if self.vhd.drives[drive].image.is_none() {
            self.vhd.drives[drive].status = vhd::status::NO_VHD;
        } else {
            match cmd {
                vhd::command::READ => self.vhd_read_sector(drive),
                vhd::command::WRITE => self.vhd_write_sector(drive),
                vhd::command::FLUSH => self.vhd_flush(drive),
                _ => self.vhd.drives[drive].status = vhd::status::UNKNOWN_COMMAND,
            }
        }

        self.vhd.busy = false;
    }

    /// READ (`vhd::command::READ`): fetch the sector at `drive`'s LRN from
    /// its image (zero-padding any short/EOF tail) and transfer all
    /// [`vhd::SECTOR_SIZE`] bytes to `drive`'s buffer address through the
    /// CPU's logical address space (MMU-translated, one byte at a time,
    /// wrapping at 64K) — exactly the path real CPU-driven code would take.
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

    /// WRITE (`vhd::command::WRITE`): zero-extend the image up to `drive`'s
    /// LRN offset first, THEN fetch [`vhd::SECTOR_SIZE`] bytes from `drive`'s
    /// buffer address through the CPU's logical address space, THEN write
    /// them into the image. This exact order matters: MAME performs the
    /// zero-extend before touching the CPU bus, which is observable if the
    /// buffer address happens to overlap the VHD's own I/O registers.
    fn vhd_write_sector(&mut self, drive: usize) {
        let offset = vhd_sector_offset(self.vhd.drives[drive].lrn);
        if self.vhd_image_mut(drive).extend_to(offset).is_err() {
            self.vhd.drives[drive].status = vhd::status::IO_ERROR;
            return;
        }

        let buffer_addr = self.vhd.drives[drive].buffer_addr;
        let mut buf = [0u8; vhd::SECTOR_SIZE];
        for (i, byte) in buf.iter_mut().enumerate() {
            *byte = self.read(buffer_addr.wrapping_add(i as u16));
        }

        let write_result = self.vhd_image_mut(drive).write_at(offset, &buf);
        self.vhd.drives[drive].status =
            if write_result.is_ok() { vhd::status::OK } else { vhd::status::IO_ERROR };
    }

    /// FLUSH (`vhd::command::FLUSH`): flush the backing file to disk. Mapping
    /// a flush I/O error to `IO_ERROR` (like read/write) is this
    /// implementation's own extension, not a separately verified MAME fact.
    fn vhd_flush(&mut self, drive: usize) {
        let flush_result = self.vhd_image_mut(drive).flush();
        self.vhd.drives[drive].status =
            if flush_result.is_ok() { vhd::status::OK } else { vhd::status::IO_ERROR };
    }

    /// The image mounted in `drive`, for the command bodies above. Panics if
    /// called on an unmounted drive — every call site is guarded by
    /// `vhd_execute_command`'s own mounted check first.
    fn vhd_image_mut(&mut self, drive: usize) -> &mut vhd::VhdImage {
        self.vhd.drives[drive].image.as_mut().expect("checked mounted")
    }

    // ---- Plain-SAM path (CoCo 1/2, no GIME) --------------------------------
    //
    // `Sam::map` does the whole-address decode (RAM/ROM/cart/I/O/open-bus) in
    // one step, unlike the GIME path's separate ROM-window/I/O-page/MMU
    // layers, so there's no need for `phys`/`is_rom_window`/`rom_read`
    // equivalents here. This path never touches `self.gime` — no MMU
    // translate, no interrupt raises, no timer (`docs/coco12-plan.md` Phase
    // 2; the field-loop gating that keeps it that way for `hsync`/`fs_*` is
    // Phase 4).

    /// Bounds-check a `Sam::map` RAM target against installed RAM. Unlike the
    /// GIME path (which masks/wraps into a smaller machine's high blocks),
    /// out-of-range plain-SAM RAM is just truncated for now: reads/writes
    /// past the installed size fall off the bus (`docs/coco12-plan.md`).
    fn sam_ram_index(&self, phys: usize) -> Option<usize> {
        (phys < self.ram.len()).then_some(phys)
    }

    fn sam_read(&mut self, addr: u16) -> u8 {
        match self.sam.map(addr) {
            SamTarget::Ram(phys) => self
                .sam_ram_index(phys)
                .map(|i| self.ram[i])
                .unwrap_or(OPEN_BUS),
            SamTarget::RomExt(off) => self.rom.get(off).copied().unwrap_or(OPEN_BUS),
            SamTarget::RomBas(off) => self
                .rom
                .get(SAM_BAS_ROM_OFFSET + off)
                .copied()
                .unwrap_or(OPEN_BUS),
            SamTarget::Cart(off) => self
                .cart
                .rom_read(SAM_CART_ROM_BASE.wrapping_add(off as u16)),
            SamTarget::Io => self.sam_io_read(addr),
            SamTarget::OpenBus => OPEN_BUS,
        }
    }

    fn sam_write(&mut self, addr: u16, val: u8) {
        match self.sam.map(addr) {
            SamTarget::Ram(phys) => {
                if let Some(i) = self.sam_ram_index(phys) {
                    self.ram[i] = val;
                }
            }
            // ROM/cart/open-bus targets: while TY=0 writes to $8000-$FEFF do
            // not write through to the RAM underneath (MAME gates
            // write-through on TY) — there's no RAM there at all in our
            // model, so these are simply dropped.
            SamTarget::RomExt(_)
            | SamTarget::RomBas(_)
            | SamTarget::Cart(_)
            | SamTarget::OpenBus => {}
            SamTarget::Io => self.sam_io_write(addr, val),
        }
    }

    /// The `SamTarget::Io` sub-decode: PIA0, PIA1, cart SCS*, and the SAM
    /// control strobes (read-only in effect — a strobe read falls through to
    /// open bus, matching the plan's memory map).
    fn sam_io_read(&mut self, addr: u16) -> u8 {
        match addr {
            IO_BASE..=PIA0_LAST => {
                self.pia0.a.input = self.pia0_pa_pins();
                self.pia0.read((addr & 0x03) as u8)
            }
            PIA1_BASE..=PIA1_LAST => {
                self.pia1.a.input = self.pia1_pa_pins();
                self.pia1.b.input = self.pia1_pb_pins();
                self.pia1.read((addr & 0x03) as u8)
            }
            CART_BASE..=CART_LAST => self.cart.read(addr),
            _ => OPEN_BUS, // SAM control strobes ($FFC0-$FFDF): write-only.
        }
    }

    fn sam_io_write(&mut self, addr: u16, val: u8) {
        match addr {
            IO_BASE..=PIA0_LAST => self.pia0.write((addr & 0x03) as u8, val),
            PIA1_BASE..=PIA1_LAST => {
                self.pia1.write((addr & 0x03) as u8, val);
                let dac = (self.pia1.a.output & self.pia1.a.ddr & 0xFC) >> 2;
                self.cassette.record_dac(dac, self.pia1.a.c2_output());
            }
            CART_BASE..=CART_LAST => self.cart.write(addr, val),
            crate::sam::STROBE_BASE..=crate::sam::STROBE_LAST => self.sam.write_strobe(addr),
            _ => { /* unmapped */ }
        }
    }
}

/// Byte offset of logical record `lrn` within a VHD image ([`vhd::SECTOR_SIZE`]
/// bytes/sector). `u64` arithmetic avoids overflow even though `lrn` is only
/// ever up to 24 bits wide.
fn vhd_sector_offset(lrn: u32) -> u64 {
    vhd::SECTOR_SIZE as u64 * u64::from(lrn)
}

/// Decode an `$FFA0–$FFAF` MMU register address to `(task, slot)`.
fn mmu_index(addr: u16) -> (usize, usize) {
    let idx = (addr - MMU_BASE) as usize;
    (idx / gime::SLOTS_PER_TASK, idx % gime::SLOTS_PER_TASK)
}

impl Bus for SystemBus {
    fn read(&mut self, addr: u16) -> u8 {
        // Two independent concrete decode paths, branched once up front
        // (`docs/coco12-plan.md` Phase 2) — not a trait object, so both stay
        // cycle-honest and the GIME path is untouched by the plain-SAM one.
        if self.variant != MachineVariant::Coco3 {
            return self.sam_read(addr);
        }
        // Hardwired to internal ROM ahead of everything else — I/O decode,
        // ROM mapping, and MMU state all take a back seat here (fact behind
        // `HARDWIRED_ROM_BASE`).
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
