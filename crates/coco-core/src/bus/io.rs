//! GIME I/O page decode (`$FF00-$FFBF`) for [`SystemBus`]'s CoCo 3 path: the
//! SCS* window (`$FF40-$FF5F`, gated by INIT0 MC2 — see
//! [`crate::gime::GIME::scs_enabled`]) with its Becker-port intercept ahead
//! of cartridge dispatch, the ungated `$FF60-$FF7E` cartridge extension, and
//! the full register read/write match over PIA0/PIA1, VHD, and the GIME's
//! own registers.

use crate::gime;
use crate::hires_joystick::TriggerInputs;

use super::regs::{
    BECKER_DATA, BECKER_STATUS, BORDER_REG, CART_EXT_BASE, CART_EXT_LAST, FIRQENR_REG, GIME_LAST,
    GIME_RESERVED_BASE, GIME_RESERVED_LAST, HOFFSET_REG, INIT0_REG, INIT1_REG, IO_BASE, IRQENR_REG,
    MMU_BASE, MMU_LAST, MPI_CONTROL_REG, OPEN_BUS, PALETTE_BASE, PALETTE_LAST, PIA_REG_MASK,
    PIA0_LAST, PIA0_PORT_A_OFFSET, PIA1_BASE, PIA1_LAST, PIA1_PORT_A_OFFSET, SCS_BASE,
    SCS_GATE_CLOSED, SCS_LAST, TIMER_LSB_REG, TIMER_MSB_REG, VBANK_REG, VHD_BUFFER_HI,
    VHD_BUFFER_LO, VHD_COMMAND_STATUS, VHD_LRN_HI, VHD_LRN_LO, VHD_LRN_MID, VHD_SELECT, VMODE_REG,
    VOFFSET0_REG, VOFFSET1_REG, VRES_REG, VSCROLL_REG,
};
use super::{SystemBus, mmu_index};

impl SystemBus {
    /// Becker-port read intercept ($FF41/$FF42): `Some` when the Becker port
    /// is enabled and `addr` is one of the two registers — takes precedence
    /// over cartridge dispatch within the SCS window (MAME installs Becker
    /// handlers over the cart range). On the GIME path [`SystemBus::read_scs`]
    /// only calls this once INIT0 MC2 has opened the gate; the plain-SAM
    /// (CoCo 1/2) path calls it ungated.
    pub(super) fn becker_read(&mut self, addr: u16) -> Option<u8> {
        let dw = self.drivewire.as_mut()?;
        match addr {
            BECKER_STATUS => Some(dw.status_read()),
            BECKER_DATA => Some(dw.data_read()),
            _ => None,
        }
    }

    /// Becker-port write intercept: `true` when the Becker port is enabled
    /// and `addr` was one of the two registers (handled, including $FF41
    /// which is swallowed). On the GIME path [`SystemBus::write_scs`] only
    /// calls this once INIT0 MC2 has opened the gate; the plain-SAM
    /// (CoCo 1/2) path calls it ungated.
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

    /// SCS* window read (`$FF40-$FF5F`): INIT0 MC2 gates the whole window
    /// shut — a hard 0, not open bus (MAME `coco3_m.cpp` `ff40_read`) — and
    /// the Becker port takes precedence over cartridge dispatch when open.
    fn read_scs(&mut self, addr: u16) -> u8 {
        if !self.gime.scs_enabled() {
            return SCS_GATE_CLOSED;
        }
        if let Some(v) = self.becker_read(addr) {
            return v;
        }
        self.cart.read(addr)
    }

    /// SCS* window write: a closed gate drops the write before it reaches
    /// the Becker port or the cartridge (MAME `coco3_m.cpp` `ff40_write`).
    fn write_scs(&mut self, addr: u16, val: u8) {
        if !self.gime.scs_enabled() {
            return;
        }
        if self.becker_write(addr, val) {
            return;
        }
        self.cart.write(addr, val);
        self.note_audio_write(); // latched cart DACs (Orchestra-90)
    }

    pub(super) fn io_read(&mut self, addr: u16) -> u8 {
        match addr {
            IO_BASE..=PIA0_LAST => {
                // Refresh port A's input pins (keyboard rows + joystick) before the PIA read.
                let reg = (addr & 0x03) as u8;
                self.pia0.a.input = self.pia0_pa_read(reg);
                self.pia0.read(reg)
            }
            PIA1_BASE..=PIA1_LAST => {
                self.pia1.a.input = self.pia1_pa_pins();
                self.pia1.b.input = self.pia1_pb_pins();
                self.pia1.read((addr & 0x03) as u8)
            }
            SCS_BASE..=SCS_LAST => self.read_scs(addr),
            CART_EXT_BASE..=CART_EXT_LAST => self.cart.read(addr),
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
        match addr {
            IO_BASE..=PIA0_LAST => self.write_pia0(addr, val),
            PIA1_BASE..=PIA1_LAST => self.write_pia1(addr, val),
            SCS_BASE..=SCS_LAST => self.write_scs(addr, val),
            CART_EXT_BASE..=CART_EXT_LAST => {
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

    /// PIA0 register write ($FF00-$FF03, mirrored through $FF1F): the PIA
    /// register write itself, the sound-mux touch, and a plugged-in hi-res
    /// interface's trigger observation (mux-address change for any kind, plus
    /// the PA0-3 pin-nibble change for CoCo Max III) — shared by both the
    /// GIME I/O-page path and the plain-SAM path.
    pub(super) fn write_pia0(&mut self, addr: u16, val: u8) {
        let mux_before = self.joystick_mux();
        let nibble_before = self.pia0_pa_nibble();
        let reg = (addr & PIA_REG_MASK) as u8;
        self.pia0.write(reg, val);
        self.note_audio_write(); // CA2/CB2 are the sound mux selects
        let port_a_write = reg == PIA0_PORT_A_OFFSET;
        self.observe_pia0_change(mux_before, nibble_before, port_a_write);
    }

    /// PIA1 register write ($FF20-$FF23, mirrored through $FF3F): the PIA
    /// register write itself, the Port-A-gated cassette DAC tap and hi-res
    /// trigger, and the sound-mux touch — shared by both the GIME I/O-page
    /// path and the plain-SAM path.
    pub(super) fn write_pia1(&mut self, addr: u16, val: u8) {
        let reg = addr & PIA_REG_MASK;
        self.pia1.write(reg as u8, val);
        // Cassette record-out and the hi-res trigger are direct,
        // unconditional taps of the DAC/PA0-3 nibble, but only sample on Port A
        // output/DDR writes, not CRA writes: MAME's `update_cassout()`/
        // `hires_trigger` run only from `pia1_pa_changed()`.
        if reg == PIA1_PORT_A_OFFSET {
            let dac = self.pia1_dac_output();
            self.cassette.record_dac(dac, self.pia1.a.c2_output());
            let (stick, axis) = self.joystick_mux();
            let pa_nibble = self.pia0_pa_nibble();
            self.joysticks
                .observe(stick, axis, TriggerInputs { dac, pa_nibble });
        }
        self.note_audio_write(); // DAC / PB1 / SNDEN / relay
    }
}
