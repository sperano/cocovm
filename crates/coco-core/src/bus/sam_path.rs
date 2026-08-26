//! Plain-SAM path (CoCo 1/2, no GIME): `SAM::map` does the whole-address
//! decode (RAM/ROM/cart/I/O/open-bus) in one step, unlike the GIME path's
//! separate ROM-window/I/O-page/MMU layers, so there's no need for
//! `phys`/`is_rom_window`/`rom_read` equivalents here. This path never
//! touches `self.gime` — no MMU translate, no interrupt raises, no timer;
//! the `hsync`/`fs_*` edge handlers (`sync.rs`) gate their GIME raises on
//! the variant the same way, so the GIME struct stays completely inert on a
//! CoCo 1/2.

use crate::sam::SAMTarget;

use super::SystemBus;
use super::regs::{
    CART_EXT_LAST, IO_BASE, OPEN_BUS, PIA0_LAST, PIA1_BASE, PIA1_LAST, SAM_BAS_ROM_OFFSET,
    SAM_CART_ROM_BASE, SCS_BASE,
};

impl SystemBus {
    /// Bounds-checks a `SAM::map` RAM target against installed RAM. Unlike
    /// the GIME path, out-of-range plain-SAM RAM is truncated, not wrapped.
    pub(super) fn sam_ram_index(&self, phys: usize) -> Option<usize> {
        (phys < self.ram.len()).then_some(phys)
    }

    /// Read the physical RAM address produced by the discrete MC6883 video
    /// counter. This bypasses the CPU ROM/I/O/P1 decode and applies the SAM's
    /// memory-size mask, matching its separate display DMA path.
    pub(crate) fn sam_video_read(&self, addr: u16) -> u8 {
        let phys = usize::from(addr & self.sam.video_address_mask());
        self.sam_ram_index(phys)
            .map(|i| self.ram[i])
            .unwrap_or(OPEN_BUS)
    }

    pub(super) fn sam_read(&mut self, addr: u16) -> u8 {
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
                .rom_read(SAM_CART_ROM_BASE.wrapping_add(off as u16)),
            SAMTarget::Io => self.sam_io_read(addr),
            SAMTarget::OpenBus => OPEN_BUS,
        }
    }

    pub(super) fn sam_write(&mut self, addr: u16, val: u8) {
        match self.sam.map(addr) {
            SAMTarget::Ram(phys) => {
                if let Some(i) = self.sam_ram_index(phys) {
                    self.ram[i] = val;
                }
            }
            // TY=0 writes to $8000-$FEFF don't write through to RAM (MAME
            // gates write-through on TY); dropped here.
            SAMTarget::RomExt(_)
            | SAMTarget::RomBas(_)
            | SAMTarget::Cart(_)
            | SAMTarget::OpenBus => {}
            SAMTarget::Io => self.sam_io_write(addr, val),
        }
    }

    /// The `SAMTarget::Io` sub-decode: PIA0, PIA1, cart SCS*, and SAM control
    /// strobes (strobe reads fall through to open bus).
    fn sam_io_read(&mut self, addr: u16) -> u8 {
        // Becker port takes precedence over cartridge dispatch (mirrors MAME's handler order).
        if let Some(v) = self.becker_read(addr) {
            return v;
        }
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
            // No GIME on a real CoCo 1/2, so no INIT0 MC2 to gate this —
            // unlike the GIME path's io_read, SCS and its $FF60-$FF7E
            // extension are one ungated range here.
            SCS_BASE..=CART_EXT_LAST => self.cart.read(addr),
            _ => OPEN_BUS, // SAM control strobes ($FFC0-$FFDF): write-only.
        }
    }

    fn sam_io_write(&mut self, addr: u16, val: u8) {
        // Becker port takes precedence over cartridge dispatch (mirrors MAME's handler order).
        if self.becker_write(addr, val) {
            return;
        }
        match addr {
            IO_BASE..=PIA0_LAST => {
                self.pia0.write((addr & 0x03) as u8, val);
                self.note_audio_write(); // CA2/CB2 are the sound mux selects
            }
            PIA1_BASE..=PIA1_LAST => self.write_pia1(addr, val),
            // Ungated, same as sam_io_read above — no GIME/MC2 on CoCo 1/2.
            SCS_BASE..=CART_EXT_LAST => {
                self.cart.write(addr, val);
                self.note_audio_write(); // latched cart DACs
            }
            crate::sam::STROBE_BASE..=crate::sam::STROBE_LAST => self.sam.write_strobe(addr),
            _ => { /* unmapped */ }
        }
    }
}
