//! Input-pin sampling for the two PIAs: keyboard/joystick on PIA0 port A,
//! cassette/RAMSZ/printer-busy on PIA1.

use crate::bitbanger;
use crate::config::MachineVariant;

use super::SystemBus;

impl SystemBus {
    /// PIA0 port-A input pins: keyboard rows for the current column strobe,
    /// fire buttons pulling their rows low regardless of the strobe, and the
    /// joystick comparator on PA7 — high while the 6-bit DAC (PIA1 PA2–PA7)
    /// is at or below the pot the CA2/CB2 mux selects (`DESIGN.md` §7).
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

    /// PIA1 port-A input pins: only bit 0 (cassette data in, `$FF20` —
    /// Service Manual / `cassette-verified-facts`) is driven by anything
    /// emulated; the rest float high like every other unused CoCo input pin
    /// ([`crate::pia::PiaPort`]'s default).
    pub(super) fn pia1_pa_pins(&self) -> u8 {
        const CASSETTE_IN: u8 = 0x01;
        if self.cassette.input_bit() {
            0xFF
        } else {
            !CASSETTE_IN
        }
    }

    /// PIA1 port-B input pins. Two bits are driven by anything emulated; the
    /// rest float high like every other unused CoCo input pin
    /// ([`crate::pia::PiaPort`]'s default).
    ///
    /// Bit 0 (printer BUSY in, `$FF22` — `bitbanger-spec.md` "Register map"),
    /// on every variant. Polarity is 0 = ready, 1 = busy: BASIC's driver
    /// treats bit 0 set as busy and spins (`LDB $FF22 / LSRB / BCS`), so the
    /// not-busy default must present bit 0 clear or every print statement
    /// would hang.
    ///
    /// Bit 2 (RAMSZ, the memory-size sense switch Color BASIC's cold-start
    /// reads to size RAM), on CoCo 1/2 only — the CoCo 3 has no such switch,
    /// so its pin keeps floating high.
    ///
    /// MAME `coco.cpp` `pia1_pb_r`: 16K-32K RAM (`$4000..=$7FFF` bytes) senses
    /// set unconditionally; 64K (`>= $8000`) instead follows PIA0 port B's
    /// *output* register bit 6 (`b_output() & 0x40`, the raw latched value —
    /// not the pin state, so DDR doesn't matter) — Color BASIC's memory-size
    /// probe momentarily drives that bit while sensing, and since Color BASIC
    /// 1.0's sizing routine can only configure 4K/16K banks, this lets the
    /// CoCo 1 (which ships 1.0 but can take a 1.2 upgrade) reach 64K only with
    /// the newer ROM. Below 16K the switch reads clear.
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
    /// register bit is clear (space). The ROM only ever drives PA1 once it
    /// has configured it as an output (DDRA = $FE at `$A048` —
    /// `bitbanger-spec.md` "Register map"); an unconfigured PA1 is treated
    /// as idle mark, matching how a floating output pin would look to a
    /// receiver expecting idle-high (not itself asserted in the spec, since
    /// the ROM always configures DDRA before touching the printer port).
    pub(crate) fn pia1_tx_mark(&self) -> bool {
        if self.pia1.a.ddr & bitbanger::TX_PIN == 0 {
            true
        } else {
            self.pia1.a.output & bitbanger::TX_PIN != 0
        }
    }
}
