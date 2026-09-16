//! Input-pin sampling for the two PIAs: keyboard/joystick on PIA0 port A,
//! cassette/RAMSZ/printer-busy on PIA1.

use crate::bitbanger;
use crate::config::MachineVariant;

use super::SystemBus;

impl SystemBus {
    /// PIA0 port-A input pins: keyboard rows for the current column strobe,
    /// fire buttons pulling their rows low, and the joystick comparator on
    /// PA7, high while the 6-bit DAC is at or below the pot selected by the
    /// mux (`DESIGN.md` §7).
    pub(super) fn pia0_pa_pins(&self) -> u8 {
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

    /// [`Self::pia0_pa_pins`] for a CPU read of PIA0 register `reg`, which
    /// also counts a keyboard scan when `reg` is the port-A data register.
    pub(super) fn pia0_pa_read(&mut self, reg: u8) -> u8 {
        const PORT_A: u8 = 0;
        let data_selected = self.pia0.a.control & crate::pia::cr::DDR_ACCESS != 0;
        if reg == PORT_A && data_selected {
            self.keyboard.note_read(self.pia0.b.output);
        }
        self.pia0_pa_pins()
    }

    /// PIA1 port-A input pins: only bit 0 (cassette data in, `$FF20`) is
    /// driven by anything emulated. The rest float high like every other
    /// unused CoCo input pin.
    pub(super) fn pia1_pa_pins(&self) -> u8 {
        const CASSETTE_IN: u8 = 0x01;
        if self.cassette.input_bit() {
            0xFF
        } else {
            !CASSETTE_IN
        }
    }

    /// PIA1 port-B input pins. Two bits are driven by anything emulated; the
    /// rest float high like every other unused CoCo input pin.
    ///
    /// Bit 0: printer BUSY in (`$FF22`). Polarity is 0 = ready, 1 = busy —
    /// BASIC's driver spins while it reads busy, so the not-busy default must
    /// present bit 0 clear.
    ///
    /// Bit 2: RAMSZ, the memory-size sense switch, on CoCo 1/2 only (CoCo 3
    /// has no such switch and floats high). Per MAME `coco.cpp` `pia1_pb_r`:
    /// 16K-32K RAM senses set unconditionally; 64K instead follows PIA0 port
    /// B's output register bit 6, since Color BASIC's memory-size probe
    /// drives that bit while sensing.
    pub(super) fn pia1_pb_pins(&self) -> u8 {
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
    /// register bit is clear (space). Unconfigured PA1 is treated as idle mark,
    /// matching a floating output pin (the ROM sets DDRA = $FE at `$A048`
    /// before use).
    pub(crate) fn pia1_tx_mark(&self) -> bool {
        if self.pia1.a.ddr & bitbanger::TX_PIN == 0 {
            true
        } else {
            self.pia1.a.output & bitbanger::TX_PIN != 0
        }
    }
}
