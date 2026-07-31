//! GIME I/O page decode (`$FF00-$FFBF`) for [`SystemBus`]'s CoCo 3 path:
//! the Becker-port intercept ahead of cartridge dispatch, and the full
//! register read/write match over PIA0/PIA1, the cartridge SCS window, VHD,
//! and the GIME's own registers.

use crate::gime;

use super::regs::{
    BECKER_DATA, BECKER_STATUS, BORDER_REG, CART_BASE, CART_LAST, FIRQENR_REG, GIME_LAST,
    GIME_RESERVED_BASE, GIME_RESERVED_LAST, HOFFSET_REG, INIT0_REG, INIT1_REG, IO_BASE, IRQENR_REG,
    MMU_BASE, MMU_LAST, MPI_CONTROL_REG, OPEN_BUS, PALETTE_BASE, PALETTE_LAST, PIA0_LAST,
    PIA1_BASE, PIA1_LAST, TIMER_LSB_REG, TIMER_MSB_REG, VBANK_REG, VHD_BUFFER_HI, VHD_BUFFER_LO,
    VHD_COMMAND_STATUS, VHD_LRN_HI, VHD_LRN_LO, VHD_LRN_MID, VHD_SELECT, VMODE_REG, VOFFSET0_REG,
    VOFFSET1_REG, VRES_REG, VSCROLL_REG,
};
use super::{SystemBus, mmu_index};

impl SystemBus {
    /// Becker-port read intercept ($FF41/$FF42): `Some` when the Becker port
    /// is enabled and `addr` is one of the two registers, in which case the
    /// caller must return it directly — this takes precedence over
    /// cartridge dispatch (`CART_BASE..=CART_LAST`) on every I/O decode
    /// path, GIME and plain-SAM alike (MAME installs Becker handlers over
    /// the cart range). `None` otherwise, so the caller falls through to its
    /// normal decode (cartridge included).
    pub(super) fn becker_read(&mut self, addr: u16) -> Option<u8> {
        let dw = self.drivewire.as_mut()?;
        match addr {
            BECKER_STATUS => Some(dw.status_read()),
            BECKER_DATA => Some(dw.data_read()),
            _ => None,
        }
    }

    /// Becker-port write intercept: `true` when the Becker port is enabled
    /// and `addr` was one of the two registers (handled — including $FF41,
    /// which is swallowed), so the caller must not fall through to its
    /// normal decode. `false` otherwise.
    pub(super) fn becker_write(&mut self, addr: u16, val: u8) -> bool {
        if self.drivewire.is_none() {
            return false;
        }
        match addr {
            BECKER_STATUS => true, // writes swallowed while Becker is enabled
            BECKER_DATA => {
                let cycle = self.cycle_clock;
                self.drivewire.as_mut().unwrap().data_write(val, cycle);
                true
            }
            _ => false,
        }
    }

    pub(super) fn io_read(&mut self, addr: u16) -> u8 {
        // Becker-port precedence over cartridge dispatch — mirrors MAME's
        // handler-installation order over the SCS window.
        if let Some(v) = self.becker_read(addr) {
            return v;
        }
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

    pub(super) fn io_write(&mut self, addr: u16, val: u8) {
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
                // Cassette record-out is a direct, unconditional tap of the DAC
                // (not gated by SNDEN/the mux — `cassette-verified-facts`), fed
                // on every PIA1 write since any of them (port A output/DDR or
                // CRA, which carries the motor relay) can change it.
                let dac = (self.pia1.a.output & self.pia1.a.ddr & 0xFC) >> 2;
                self.cassette.record_dac(dac, self.pia1.a.c2_output());
                self.note_audio_write(); // DAC / PB1 / SNDEN / relay
            }
            CART_BASE..=CART_LAST => {
                self.cart.write(addr, val);
                self.note_audio_write(); // latched cart DACs (Orchestra-90)
            }
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
}
