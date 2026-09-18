//! Input-pin sampling for the two PIAs: keyboard/joystick on PIA0 port A,
//! cassette/RAMSZ/printer-busy on PIA1.

use crate::bitbanger;
use crate::config::MachineVariant;

use super::SystemBus;

/// Mask isolating PIA0 port-A pins PA0-PA3 — the joystick fire-button lines a CoCo Max III
/// hi-res unit reads as its trigger nibble (MAME `coco_cm3_hires_joy`: `a_output() & 0x0f`).
const PA_NIBBLE_MASK: u8 = 0x0F;

impl SystemBus {
    /// PIA0 port-A input pins: keyboard rows for the current column strobe,
    /// fire buttons pulling their rows low, and the joystick comparator on
    /// PA7, high while the 6-bit DAC is at or below the pot selected by the
    /// mux (`DESIGN.md` §7).
    pub(super) fn pia0_pa_pins(&self) -> u8 {
        const COMPARATOR_BIT: u8 = 0x80;
        let mut pa = self.keyboard.sense(self.pia0.b.output);
        pa &= !self.joysticks.button_rows();
        let (stick, axis) = self.joystick_mux();
        let dac = (self.pia1.a.output & 0xFC) >> 2;
        if self.joysticks.compare(stick, axis, dac) {
            pa |= COMPARATOR_BIT;
        } else {
            pa &= !COMPARATOR_BIT;
        }
        pa
    }

    /// Current analog-mux selection: SEL1/CA2 (PIA0 side A) picks the axis,
    /// SEL2/CB2 (side B) picks the stick — the same pair [`Self::pia0_pa_pins`]
    /// reads, exposed so a PIA0 write can detect when it changes (a plugged-in
    /// hi-res interface's trigger observes every such change, not just DAC/PA0-3
    /// writes — see `crate::hires_joystick`).
    pub(super) fn joystick_mux(&self) -> (usize, usize) {
        let axis = usize::from(self.pia0.a.c2_output()); // SEL1: 0 = X, 1 = Y
        let stick = usize::from(self.pia0.b.c2_output()); // SEL2: 0 = right
        (stick, axis)
    }

    /// PIA1 port-A's 6-bit DAC value (`$FF20`, bits 7:2), gated by DDR — the
    /// reading both the cassette record tap and the Tandy hi-res trigger
    /// observe (MAME `update_cassout()`/`hires_trigger` both read the same
    /// masked value).
    pub(super) fn pia1_dac_output(&self) -> u8 {
        (self.pia1.a.output & self.pia1.a.ddr & 0xFC) >> 2
    }

    /// PIA0 port-A PA0-PA3 pin nibble as a CoCo Max III hi-res unit's trigger sees it: the PIN
    /// state (driven output on output-configured lines, live input pins on the rest), not just
    /// the output register — a button held on an input-configured line pulls it low too.
    pub(super) fn pia0_pa_nibble(&self) -> u8 {
        let pins =
            (self.pia0.a.output & self.pia0.a.ddr) | (self.pia0_pa_pins() & !self.pia0.a.ddr);
        pins & PA_NIBBLE_MASK
    }

    /// After a PIA0 write, re-run the mux-selected port's hi-res trigger if the write moved the
    /// analog mux (any kind), or — CoCoMax3 only — changed the PA0-PA3 pin nibble via a port-A
    /// data/DDR write, not CRA (see [`super::regs::PIA0_PORT_A_OFFSET`]'s doc).
    /// `mux_before`/`nibble_before` are this module's readings from just before the write;
    /// `port_a_write` is whether the write's register was PIA0's port-A data/DDR ($FF00).
    pub(super) fn observe_pia0_change(
        &mut self,
        mux_before: (usize, usize),
        nibble_before: u8,
        port_a_write: bool,
    ) {
        self.joysticks
            .observe_pia0(crate::joystick::PIA0WriteObservation {
                mux_before,
                mux_after: self.joystick_mux(),
                nibble_before,
                nibble_after: self.pia0_pa_nibble(),
                port_a_write,
                dac: self.pia1_dac_output(),
            });
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
