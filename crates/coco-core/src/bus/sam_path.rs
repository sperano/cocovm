//! Plain-SAM path (CoCo 1/2, no GIME): `Sam::map` does the whole-address
//! decode (RAM/ROM/cart/I/O/open-bus) in one step, unlike the GIME path's
//! separate ROM-window/I/O-page/MMU layers, so there's no need for
//! `phys`/`is_rom_window`/`rom_read` equivalents here. This path never
//! touches `self.gime` — no MMU translate, no interrupt raises, no timer
//! (`docs/coco12-plan.md` Phase 2; the field-loop gating that keeps it that
//! way for `hsync`/`fs_*` is Phase 4).

use crate::sam::SamTarget;

use super::regs::{
    CART_BASE, CART_LAST, IO_BASE, OPEN_BUS, PIA0_LAST, PIA1_BASE, PIA1_LAST, SAM_BAS_ROM_OFFSET,
    SAM_CART_ROM_BASE,
};
use super::SystemBus;

impl SystemBus {
    /// Bounds-check a `Sam::map` RAM target against installed RAM. Unlike the
    /// GIME path (which masks/wraps into a smaller machine's high blocks),
    /// out-of-range plain-SAM RAM is just truncated for now: reads/writes
    /// past the installed size fall off the bus (`docs/coco12-plan.md`).
    pub(super) fn sam_ram_index(&self, phys: usize) -> Option<usize> {
        (phys < self.ram.len()).then_some(phys)
    }

    pub(super) fn sam_read(&mut self, addr: u16) -> u8 {
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

    pub(super) fn sam_write(&mut self, addr: u16, val: u8) {
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
        // Becker-port precedence over cartridge dispatch — mirrors MAME's
        // handler-installation order over the SCS window.
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
            CART_BASE..=CART_LAST => self.cart.read(addr),
            _ => OPEN_BUS, // SAM control strobes ($FFC0-$FFDF): write-only.
        }
    }

    fn sam_io_write(&mut self, addr: u16, val: u8) {
        // Becker-port precedence over cartridge dispatch — mirrors MAME's
        // handler-installation order over the SCS window.
        if self.becker_write(addr, val) {
            return;
        }
        match addr {
            IO_BASE..=PIA0_LAST => {
                self.pia0.write((addr & 0x03) as u8, val);
                self.note_audio_write(); // CA2/CB2 are the sound mux selects
            }
            PIA1_BASE..=PIA1_LAST => {
                self.pia1.write((addr & 0x03) as u8, val);
                let dac = (self.pia1.a.output & self.pia1.a.ddr & 0xFC) >> 2;
                self.cassette.record_dac(dac, self.pia1.a.c2_output());
                self.note_audio_write(); // DAC / PB1 / SNDEN / relay
            }
            CART_BASE..=CART_LAST => {
                self.cart.write(addr, val);
                self.note_audio_write(); // latched cart DACs
            }
            crate::sam::STROBE_BASE..=crate::sam::STROBE_LAST => self.sam.write_strobe(addr),
            _ => { /* unmapped */ }
        }
    }
}
