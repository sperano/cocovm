//! MC6821 Peripheral Interface Adapter. See `DESIGN.md` §7.
//!
//! Models the two independent sides (A/B), each with an output register, a data
//! direction register, a control register, and the Cx1 interrupt line. The
//! DDR/data access split (control bit 2) and the Cx1 flag (bit 7) cleared by
//! reading the data register are what the CoCo relies on: the 60 Hz field sync
//! and the horizontal sync are wired to PIA0's CB1/CA1 and drive the CPU IRQ.
//!
//! Cx2 is modelled as a set/reset output only (`c2_output`, the mode the CoCo
//! uses for the joystick mux and sound enable); Cx2 interrupt-input and
//! handshake/pulse strobe modes are not modelled.

use serde::{Deserialize, Serialize};

/// Control-register bit assignments (identical for CRA and CRB).
pub mod cr {
    /// Cx1 interrupt enable — 1 = an active Cx1 edge asserts IRQ.
    pub const C1_IRQ_ENABLE: u8 = 0x01;
    /// Cx1 active-edge select — 0 = high→low, 1 = low→high.
    pub const C1_EDGE_HIGH: u8 = 0x02;
    /// Data/DDR access select — 1 = data register, 0 = data-direction register.
    pub const DDR_ACCESS: u8 = 0x04;
    /// Cx2 output level when bits 5:4 select set/reset output mode.
    pub const C2_SET: u8 = 0x08;
    /// Cx2 output-mode select — with [`C2_OUTPUT`], 1 = set/reset (static level
    /// from [`C2_SET`]), 0 = handshake/pulse strobes (unmodelled).
    pub const C2_SET_RESET: u8 = 0x10;
    /// Cx2 direction — 1 = Cx2 is an output pin.
    pub const C2_OUTPUT: u8 = 0x20;
    /// Cx2 interrupt flag (read-only). Modelled as storage only.
    pub const C2_FLAG: u8 = 0x40;
    /// Cx1 interrupt flag (read-only) — set by an active Cx1 edge.
    pub const C1_FLAG: u8 = 0x80;
}

/// One side (A or B) of an MC6821.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiaPort {
    /// Output register (drives the pins selected as outputs by `ddr`).
    pub output: u8,
    /// Data direction register — 1 bit = output pin, 0 = input pin.
    pub ddr: u8,
    /// Control register (CRA/CRB).
    pub control: u8,
    /// State of the input pins (what the outside world drives).
    pub input: u8,
}

impl Default for PiaPort {
    fn default() -> Self {
        // Idle input pins float high on the CoCo (keyboard rows read $FF = no key).
        Self { output: 0, ddr: 0, control: 0, input: 0xFF }
    }
}

impl PiaPort {
    /// Value seen when reading the data register: output bits on output pins,
    /// live input on the rest.
    fn data(&self) -> u8 {
        (self.output & self.ddr) | (self.input & !self.ddr)
    }

    /// Pulse the Cx1 line with its active edge, latching the Cx1 flag. The CoCo
    /// wires horizontal sync to PIA0 CA1 and field sync to PIA0 CB1.
    pub fn pulse_c1(&mut self) {
        self.control |= cr::C1_FLAG;
    }

    /// True when this side is asserting IRQ (Cx1 flag set and its enable on).
    fn irq(&self) -> bool {
        self.control & cr::C1_FLAG != 0 && self.control & cr::C1_IRQ_ENABLE != 0
    }

    /// The Cx2 pin level when programmed as a set/reset output (control bits
    /// 5:4 = 11, the only Cx2 output mode the CoCo uses — the ROM's standard
    /// control values are $34/$3C). The CoCo hangs the joystick-mux selects on
    /// PIA0 CA2/CB2 and the sound enable on PIA1 CB2. Handshake/pulse strobe
    /// modes (bit 4 = 0) are not modelled; they'd need bus-cycle hooks.
    pub fn c2_output(&self) -> bool {
        self.control & cr::C2_SET != 0
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MC6821 {
    pub a: PiaPort,
    pub b: PiaPort,
}

impl MC6821 {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read one of the 4 PIA registers (`addr & 0x03`). Reading a data register
    /// clears that side's Cx1/Cx2 interrupt flags.
    pub fn read(&mut self, reg: u8) -> u8 {
        match reg & 0x03 {
            0 => Self::read_side(&mut self.a),
            1 => self.a.control,
            2 => Self::read_side(&mut self.b),
            _ => self.b.control,
        }
    }

    fn read_side(port: &mut PiaPort) -> u8 {
        if port.control & cr::DDR_ACCESS != 0 {
            // Reading the peripheral data register clears the interrupt flags.
            port.control &= !(cr::C1_FLAG | cr::C2_FLAG);
            port.data()
        } else {
            port.ddr
        }
    }

    /// Write one of the 4 PIA registers (`addr & 0x03`).
    pub fn write(&mut self, reg: u8, val: u8) {
        match reg & 0x03 {
            0 => Self::write_side(&mut self.a, val),
            1 => Self::write_control(&mut self.a, val),
            2 => Self::write_side(&mut self.b, val),
            _ => Self::write_control(&mut self.b, val),
        }
    }

    fn write_side(port: &mut PiaPort, val: u8) {
        if port.control & cr::DDR_ACCESS != 0 {
            port.output = val;
        } else {
            port.ddr = val;
        }
    }

    fn write_control(port: &mut PiaPort, val: u8) {
        // Bits 7/6 are read-only interrupt flags; the CPU can't set them.
        const WRITABLE: u8 = !(cr::C1_FLAG | cr::C2_FLAG);
        port.control = (port.control & !WRITABLE) | (val & WRITABLE);
    }

    /// Combined IRQ output of both sides.
    pub fn irq(&self) -> bool {
        self.a.irq() || self.b.irq()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c2_output_follows_set_reset_level() {
        let mut pia = MC6821::new();
        // The ROM's standard idle control value: C2 set/reset output, level 0.
        pia.write(1, 0x34);
        assert!(!pia.a.c2_output());
        // Level bit raised (e.g. selecting the other joystick mux input).
        pia.write(1, 0x3C);
        assert!(pia.a.c2_output());
    }

    #[test]
    fn c2_flags_are_not_writable() {
        let mut pia = MC6821::new();
        pia.write(1, 0xFF);
        assert_eq!(pia.a.control & (cr::C1_FLAG | cr::C2_FLAG), 0);
    }
}
