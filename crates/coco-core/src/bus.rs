//! `SystemBus` — address decode for RAM/ROM through the MMU plus the fixed I/O
//! page. Implements the CPU's `Bus` trait. See `DESIGN.md` §3.
//!
//! STATUS: skeleton. Device ranges are decoded to the right peripherals, but most
//! peripheral registers are stubs and ROM mapping / vector fetch is TODO.

use mc6809::Bus;

use crate::cart::{Cartridge, EmptySlot};
use crate::config::MemorySize;
use crate::gime::GIME;
use crate::pia::MC6821;

// I/O page device ranges (`DESIGN.md` §3). PIA0/PIA1 mirror every 4 bytes.
const IO_BASE: u16 = 0xFF00;
const PIA0_LAST: u16 = 0xFF1F;
const PIA1_BASE: u16 = 0xFF20;
const PIA1_LAST: u16 = 0xFF3F;
const CART_BASE: u16 = 0xFF40;
const CART_LAST: u16 = 0xFF5F;
const GIME_BASE: u16 = 0xFF90;
const GIME_LAST: u16 = 0xFF9F;
const MMU_BASE: u16 = 0xFFA0;
const MMU_LAST: u16 = 0xFFAF;
const PALETTE_BASE: u16 = 0xFFB0;
const PALETTE_LAST: u16 = 0xFFBF;

const OPEN_BUS: u8 = 0xFF;

pub struct SystemBus {
    pub ram: Box<[u8]>,
    pub rom: Box<[u8]>,
    pub gime: GIME,
    pub pia0: MC6821,
    pub pia1: MC6821,
    pub cart: Box<dyn Cartridge>,
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
            io_enabled: true,
        }
    }

    /// Physical RAM offset for a CPU address, masked to installed RAM.
    fn phys(&self, addr: u16) -> usize {
        self.gime.translate(addr) % self.ram.len()
    }

    fn io_read(&mut self, addr: u16) -> u8 {
        match addr {
            IO_BASE..=PIA0_LAST => self.pia0.read((addr & 0x03) as u8),
            PIA1_BASE..=PIA1_LAST => self.pia1.read((addr & 0x03) as u8),
            CART_BASE..=CART_LAST => self.cart.read(addr),
            GIME_BASE..=GIME_LAST => 0, // TODO: GIME register reads (status clears)
            MMU_BASE..=MMU_LAST => {
                // TODO: reads return only the low 6 bits reliably (`DESIGN.md` §3).
                0
            }
            PALETTE_BASE..=PALETTE_LAST => self.gime.palette[(addr - PALETTE_BASE) as usize],
            _ => OPEN_BUS, // SAM-compat / vectors / unmapped — TODO
        }
    }

    fn io_write(&mut self, addr: u16, val: u8) {
        match addr {
            IO_BASE..=PIA0_LAST => self.pia0.write((addr & 0x03) as u8, val),
            PIA1_BASE..=PIA1_LAST => self.pia1.write((addr & 0x03) as u8, val),
            CART_BASE..=CART_LAST => self.cart.write(addr, val),
            GIME_BASE..=GIME_LAST => { /* TODO: GIME control registers */ }
            MMU_BASE..=MMU_LAST => {
                let idx = (addr - MMU_BASE) as usize;
                let task = idx / crate::gime::SLOTS_PER_TASK;
                let slot = idx % crate::gime::SLOTS_PER_TASK;
                self.gime.mmu[task][slot] = val; // full 8 bits stored on write
            }
            PALETTE_BASE..=PALETTE_LAST => {
                self.gime.palette[(addr - PALETTE_BASE) as usize] = val;
            }
            _ => { /* SAM-compat / unmapped — TODO */ }
        }
    }
}

impl Bus for SystemBus {
    fn read(&mut self, addr: u16) -> u8 {
        if self.io_enabled && addr >= IO_BASE {
            return self.io_read(addr);
        }
        let p = self.phys(addr);
        self.ram[p]
    }

    fn write(&mut self, addr: u16, val: u8) {
        if self.io_enabled && addr >= IO_BASE {
            self.io_write(addr, val);
            return;
        }
        let p = self.phys(addr);
        self.ram[p] = val;
    }
}
