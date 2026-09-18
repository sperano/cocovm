//! Side-effect-free reads (debugger).
//!
//! `peek` mirrors `Bus::read`'s address decode exactly but takes `&self` and
//! never mutates: no PIA Cx1/Cx2 flag clears, no GIME IRQ/FIRQ status ack, no
//! cartridge register side effects, and no watchpoint hook. Devices whose real
//! read mutates return a last-latched value (GIME status registers, using their
//! public `*_pending` fields) or open bus (most cartridge I/O). The debugger UI
//! uses this for its disassembly, memory, and stack views.

use crate::config::MachineVariant;
use crate::gime;
use crate::sam::SAMTarget;

use super::regs::{
    CART_EXT_BASE, CART_EXT_LAST, FIRQENR_REG, GIME_LAST, HARDWIRED_ROM_BASE, INIT0_REG, INIT1_REG,
    IO_BASE, IRQENR_REG, MMU_BASE, MMU_LAST, MPI_CONTROL_REG, OPEN_BUS, PALETTE_BASE, PALETTE_LAST,
    PIA0_LAST, PIA1_BASE, PIA1_LAST, ROM_WINDOW_BASE, SAM_BAS_ROM_OFFSET, SAM_CART_ROM_BASE,
    SCS_BASE, SCS_GATE_CLOSED, SCS_LAST, TIMER_MSB_REG, VHD_BUFFER_HI, VHD_BUFFER_LO,
    VHD_COMMAND_STATUS, VHD_LRN_HI, VHD_LRN_LO, VHD_LRN_MID, VHD_SELECT,
};
use super::{SystemBus, mmu_index};

impl SystemBus {
    /// Read `addr` with no side effects. Routes identically to [`mc6809::Bus::read`].
    pub fn peek(&self, addr: u16) -> u8 {
        if self.variant != MachineVariant::Coco3 {
            return self.sam_peek(addr);
        }
        if addr >= HARDWIRED_ROM_BASE {
            return self.rom_peek(addr);
        }
        if self.io_enabled && addr >= IO_BASE {
            return self.io_peek(addr);
        }
        if self.is_rom_window(addr) {
            return self.rom_peek(addr);
        }
        let p = self.phys(addr);
        self.ram[p]
    }

    /// Side-effect-free twin of `SystemBus::rom_read`.
    fn rom_peek(&self, addr: u16) -> u8 {
        if addr < HARDWIRED_ROM_BASE && self.gime.rom_is_external(addr) {
            return self.cart.rom_peek(addr);
        }
        let off = (addr - ROM_WINDOW_BASE) as usize;
        self.rom.get(off).copied().unwrap_or(OPEN_BUS)
    }

    /// Side-effect-free twin of `SystemBus::io_read` (GIME I/O page).
    fn io_peek(&self, addr: u16) -> u8 {
        match addr {
            IO_BASE..=PIA0_LAST => {
                // A real read refreshes only port A's input pins; port B keeps its latched `input`.
                self.pia0
                    .peek((addr & 0x03) as u8, self.pia0_pa_pins(), self.pia0.b.input)
            }
            PIA1_BASE..=PIA1_LAST => self.pia1.peek(
                (addr & 0x03) as u8,
                self.pia1_pa_pins(),
                self.pia1_pb_pins(),
            ),
            // Mirrors `io_read`'s INIT0 MC2 gate: closed reads a hard 0.
            SCS_BASE..=SCS_LAST => {
                if self.gime.scs_enabled() {
                    self.cart.peek(addr)
                } else {
                    SCS_GATE_CLOSED
                }
            }
            CART_EXT_BASE..=CART_EXT_LAST => self.cart.peek(addr),
            MPI_CONTROL_REG => self.cart.peek_control(),
            VHD_LRN_HI | VHD_LRN_MID | VHD_LRN_LO | VHD_BUFFER_HI | VHD_BUFFER_LO => {
                self.vhd.read_lrn_or_buffer()
            }
            VHD_COMMAND_STATUS => self.vhd.read_status(),
            VHD_SELECT => OPEN_BUS,
            INIT0_REG => self.gime.init0,
            INIT1_REG => self.gime.init1,
            // Read would clear these (status ack); peek reports them intact.
            IRQENR_REG => self.gime.irq_pending,
            FIRQENR_REG => self.gime.firq_pending,
            TIMER_MSB_REG..=GIME_LAST => 0,
            MMU_BASE..=MMU_LAST => {
                let (task, slot) = mmu_index(addr);
                self.gime.mmu[task][slot] & gime::MMU_READ_MASK
            }
            PALETTE_BASE..=PALETTE_LAST => self.gime.palette[(addr - PALETTE_BASE) as usize],
            _ => OPEN_BUS,
        }
    }

    /// Side-effect-free twin of `SystemBus::sam_read` (plain-SAM path).
    fn sam_peek(&self, addr: u16) -> u8 {
        match self.sam.map(addr) {
            SAMTarget::Ram(phys) => self
                .sam_ram_index(phys)
                .map(|i| self.ram[i])
                .unwrap_or(OPEN_BUS),
            SAMTarget::RomExt(off) => self.rom.get(off).copied().unwrap_or(OPEN_BUS),
            SAMTarget::RomBas(off) => self
                .rom
                .get(SAM_BAS_ROM_OFFSET + off)
                .copied()
                .unwrap_or(OPEN_BUS),
            SAMTarget::Cart(off) => self
                .cart
                .rom_peek(SAM_CART_ROM_BASE.wrapping_add(off as u16)),
            SAMTarget::Io => self.sam_io_peek(addr),
            SAMTarget::CartUpperIo => self.cart.upper_io_peek(addr),
            SAMTarget::OpenBus => OPEN_BUS,
        }
    }

    /// Side-effect-free twin of `SystemBus::sam_io_read`.
    fn sam_io_peek(&self, addr: u16) -> u8 {
        match addr {
            IO_BASE..=PIA0_LAST => {
                self.pia0
                    .peek((addr & 0x03) as u8, self.pia0_pa_pins(), self.pia0.b.input)
            }
            PIA1_BASE..=PIA1_LAST => self.pia1.peek(
                (addr & 0x03) as u8,
                self.pia1_pa_pins(),
                self.pia1_pb_pins(),
            ),
            // Ungated, same as `sam_io_read` — no GIME/MC2 on CoCo 1/2.
            SCS_BASE..=CART_EXT_LAST => self.cart.peek(addr),
            _ => OPEN_BUS,
        }
    }
}
