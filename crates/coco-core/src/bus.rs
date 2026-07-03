//! `SystemBus` — address decode for RAM/ROM through the MMU plus the fixed I/O
//! page. Implements the CPU's `Bus` trait. See `DESIGN.md` §3.
//!
//! STATUS: skeleton. Device ranges are decoded to the right peripherals, but most
//! peripheral registers are stubs and ROM mapping / vector fetch is TODO.

use mc6809::Bus;

use crate::cart::{Cartridge, EmptySlot};
use crate::config::MemorySize;
use crate::gime::{self, GIME};
use crate::joystick::Joysticks;
use crate::keyboard::Keyboard;
use crate::pia::MC6821;

// I/O page device ranges (`DESIGN.md` §3). PIA0/PIA1 mirror every 4 bytes.
const IO_BASE: u16 = 0xFF00;
const PIA0_LAST: u16 = 0xFF1F;
const PIA1_BASE: u16 = 0xFF20;
const PIA1_LAST: u16 = 0xFF3F;
const CART_BASE: u16 = 0xFF40;
const CART_LAST: u16 = 0xFF5F;
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

pub struct SystemBus {
    pub ram: Box<[u8]>,
    pub rom: Box<[u8]>,
    pub gime: GIME,
    pub pia0: MC6821,
    pub pia1: MC6821,
    pub cart: Box<dyn Cartridge>,
    pub keyboard: Keyboard,
    pub joysticks: Joysticks,
    pub io_enabled: bool,
    /// Last sampled state of the GIME keyboard-interrupt input (true = some
    /// PA0–PA6 row line low). The EI1 source fires on its falling edge.
    kbd_line_low: bool,
}

impl SystemBus {
    pub fn new(memory: MemorySize, rom: Box<[u8]>) -> Self {
        Self {
            ram: vec![0u8; memory.bytes()].into_boxed_slice(),
            rom,
            gime: GIME::new(),
            pia0: MC6821::new(),
            pia1: MC6821::new(),
            cart: Box::new(EmptySlot),
            keyboard: Keyboard::new(),
            joysticks: Joysticks::new(),
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

    /// Horizontal-sync edge: latches PIA0 CA1 (control reg $FF01, port A) and
    /// the GIME HBORD source; also the GIME's per-scanline sample point for
    /// the EI1 keyboard-interrupt input (a zero on any PA0–PA6 row while some
    /// column is strobed — SEB Unravelled II), which fires on falling edge.
    ///
    /// Also the sample point for the expansion-port CART* line: auto-start
    /// game paks tie it to the Q clock (~895 kHz), so while one is inserted
    /// this pulses PIA1 CB1 (the legacy FIRQ cart-boot path) and raises the
    /// GIME EI0 source (the same physical pin) every scanline, which is more
    /// than enough cadence to keep either interrupt path continuously fed.
    pub fn hsync(&mut self) {
        self.pia0.a.pulse_c1();
        self.gime.raise(gime::intr::HBORD);
        // Buttons are included: SEB warns joystick fire buttons always trip EI1.
        let line_low = self.pia0_pa_pins() & 0x7F != 0x7F;
        if line_low && !self.kbd_line_low {
            self.gime.raise(gime::intr::EI1);
        }
        self.kbd_line_low = line_low;
        if self.cart.cart_line_ties_q() {
            self.pia1.b.pulse_c1();
            self.gime.raise(gime::intr::EI0);
        }
    }

    /// Field-sync (~60 Hz vertical) edge: latches PIA0 CB1 (control reg $FF03,
    /// port B) — the interrupt that drives BASIC's housekeeping loop — and the
    /// GIME VBORD source.
    pub fn vsync(&mut self) {
        self.pia0.b.pulse_c1();
        self.gime.raise(gime::intr::VBORD);
    }

    /// Instantaneous speaker level, 0.0–1.0.
    ///
    /// Two sources mix on the CoCo 3 (it has no sound chip of its own):
    /// the 6-bit DAC (PIA1 PA2–PA7), routed through the analog mux only when
    /// SNDEN (PIA1 CB2) is high and the SEL2:SEL1 selects (PIA0 CB2:CA2) are
    /// 00 — states 01/10 route cassette/cartridge audio (neither emulated,
    /// silent) and 11 is grounded; and the single-bit sound on PIA1 PB1,
    /// which is always connected. (Tandy Service Manual mux table via MAME
    /// `coco.cpp` `update_sound`; SEB Unravelled II $FF22/$FF23.)
    pub fn sound_sample(&self) -> f32 {
        /// Relative loudness of the full-scale DAC vs the single-bit beeper.
        const DAC_GAIN: f32 = 0.75;
        const SINGLE_BIT_GAIN: f32 = 0.25;
        /// PIA1 PB1: the single-bit sound output.
        const SINGLE_BIT: u8 = 0x02;
        const DAC_MAX: f32 = 63.0;

        let mut level = 0.0;
        let sel = u8::from(self.pia0.b.c2_output()) << 1 | u8::from(self.pia0.a.c2_output());
        if self.pia1.b.c2_output() && sel == 0 {
            let dac = (self.pia1.a.output & self.pia1.a.ddr & 0xFC) >> 2;
            level += DAC_GAIN * f32::from(dac) / DAC_MAX;
        }
        if self.pia1.b.output & self.pia1.b.ddr & SINGLE_BIT != 0 {
            level += SINGLE_BIT_GAIN;
        }
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
            PIA1_BASE..=PIA1_LAST => self.pia1.read((addr & 0x03) as u8),
            CART_BASE..=CART_LAST => self.cart.read(addr),
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
            PIA1_BASE..=PIA1_LAST => self.pia1.write((addr & 0x03) as u8, val),
            CART_BASE..=CART_LAST => self.cart.write(addr, val),
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
}

/// Decode an `$FFA0–$FFAF` MMU register address to `(task, slot)`.
fn mmu_index(addr: u16) -> (usize, usize) {
    let idx = (addr - MMU_BASE) as usize;
    (idx / gime::SLOTS_PER_TASK, idx % gime::SLOTS_PER_TASK)
}

impl Bus for SystemBus {
    fn read(&mut self, addr: u16) -> u8 {
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
