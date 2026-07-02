//! `SystemBus` — address decode for RAM/ROM through the MMU plus the fixed I/O
//! page. Implements the CPU's `Bus` trait. See `DESIGN.md` §3.
//!
//! STATUS: skeleton. Device ranges are decoded to the right peripherals, but most
//! peripheral registers are stubs and ROM mapping / vector fetch is TODO.

use mc6809::Bus;

use crate::cart::{Cartridge, EmptySlot};
use crate::config::MemorySize;
use crate::gime::{self, GIME};
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
/// GIME control registers past INIT0/INIT1: IRQ/FIRQ enables, timer, video, border.
const GIME_CTRL_BASE: u16 = 0xFF92;
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
/// `$FE00–$FEFF` is always RAM — the "constant" interrupt-vector page the ROM
/// routes NMI/IRQ/FIRQ/SWI through (SEB Unravelled II; INIT0 MC3 controls whether
/// it is fixed at physical `$7FE00` or follows MMU logical block 7). It sits inside
/// the ROM window address range but is *not* ROM: BASIC writes JMP trampolines here.
const CONSTANT_RAM_BASE: u16 = 0xFE00;
const CONSTANT_RAM_LAST: u16 = 0xFEFF;
/// Physical base of the constant `$FE00` page when INIT0 MC3 is set.
const CONSTANT_RAM_PHYS: usize = 0x7_FE00;
/// The 6809 hardware vectors (`$FFF0–$FFFF`) are always fetched from internal ROM,
/// even in the "32K external" ROM configuration (SEB Unravelled II ROM-map table).
const VECTOR_BASE: u16 = 0xFFF0;

const OPEN_BUS: u8 = 0xFF;

pub struct SystemBus {
    pub ram: Box<[u8]>,
    pub rom: Box<[u8]>,
    pub gime: GIME,
    pub pia0: MC6821,
    pub pia1: MC6821,
    pub cart: Box<dyn Cartridge>,
    pub keyboard: Keyboard,
    pub io_enabled: bool,
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
            io_enabled: true,
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

    /// True when `addr` reads internal ROM (the `$8000–$FDFF` window when ROM is
    /// mapped). `$FE00–$FEFF` is RAM; `$FF00+` is the I/O page / vectors.
    fn is_rom_window(&self, addr: u16) -> bool {
        (ROM_WINDOW_BASE..CONSTANT_RAM_BASE).contains(&addr) && self.gime.rom_enabled()
    }

    /// True when any interrupt source is holding the CPU IRQ line low.
    ///
    /// At the BASIC prompt the stock ROM runs the legacy PIA path (INIT0 IEN=0),
    /// so IRQ comes from PIA0's field/horizontal sync. GIME-sourced IRQ (timer,
    /// VBORD) will OR in here once wired (`DESIGN.md` §4).
    pub fn irq_asserted(&self) -> bool {
        self.pia0.irq() || self.pia1.irq()
    }

    /// True when any interrupt source is holding the CPU FIRQ line low.
    /// Cartridge/GIME FIRQ sources are TODO.
    pub fn firq_asserted(&self) -> bool {
        false
    }

    /// Horizontal-sync edge: latches PIA0 CA1 (control reg $FF01, port A).
    pub fn hsync(&mut self) {
        self.pia0.a.pulse_c1();
    }

    /// Field-sync (~60 Hz vertical) edge: latches PIA0 CB1 (control reg $FF03,
    /// port B) — the interrupt that drives BASIC's housekeeping loop.
    pub fn vsync(&mut self) {
        self.pia0.b.pulse_c1();
    }

    /// Read internal ROM for a logical address in the `$8000–$FFFF` window.
    ///
    /// The 32K image sits at offset `addr - $8000`. A cartridge ROM overlay on
    /// the upper half (INIT0 MC1=0, "16K external") plugs in here — deferred until
    /// a real `Cartridge` provides ROM; `EmptySlot` yields pure internal ROM, which
    /// is what boots a diskless CoCo 3.
    fn rom_read(&self, addr: u16) -> u8 {
        let off = (addr - ROM_WINDOW_BASE) as usize;
        self.rom.get(off).copied().unwrap_or(OPEN_BUS)
    }

    fn io_read(&mut self, addr: u16) -> u8 {
        match addr {
            IO_BASE..=PIA0_LAST => {
                // Port A senses the keyboard rows for the current port-B column
                // strobe. Refresh its input pins before the PIA read.
                self.pia0.a.input = self.keyboard.sense(self.pia0.b.output);
                self.pia0.read((addr & 0x03) as u8)
            }
            PIA1_BASE..=PIA1_LAST => self.pia1.read((addr & 0x03) as u8),
            CART_BASE..=CART_LAST => self.cart.read(addr),
            INIT0_REG => self.gime.init0,
            INIT1_REG => self.gime.init1,
            GIME_CTRL_BASE..=GIME_LAST => 0, // TODO: IRQ/timer/video status regs (clear-on-read)
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
            GIME_CTRL_BASE..=GIME_LAST => { /* TODO: IRQ/timer control regs */ }
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
        if self.io_enabled && addr >= IO_BASE {
            // Vectors are pulled from internal ROM; the rest of the page is I/O.
            if addr >= VECTOR_BASE {
                return self.rom_read(addr);
            }
            return self.io_read(addr);
        }
        if self.is_rom_window(addr) {
            return self.rom_read(addr);
        }
        let p = self.phys(addr);
        self.ram[p]
    }

    fn write(&mut self, addr: u16, val: u8) {
        if self.io_enabled && addr >= IO_BASE {
            // The vector page is ROM: writes there fall through to shadow RAM.
            if addr < VECTOR_BASE {
                self.io_write(addr, val);
                return;
            }
        }
        // ROM is read-only; writes to the ROM window reach the RAM mapped beneath it.
        let p = self.phys(addr);
        self.ram[p] = val;
    }
}
