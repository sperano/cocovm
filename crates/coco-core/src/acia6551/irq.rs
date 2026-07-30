//! IRQ bookkeeping: recomputing [`status::IRQ`] from the armed-source
//! bitmask, the small command-register predicates that gate IRQ arming, and
//! the DCD/DSR change-detector [`ACIA6551::tick`] drives every tick.

use super::{ACIA6551, command, irq_source, status, tx_control};

impl ACIA6551 {
    /// Recompute the [`status::IRQ`] bit from `irq_sources`.
    pub(super) fn update_irq_output(&mut self) {
        if self.irq_sources != 0 {
            self.status |= status::IRQ;
        } else {
            self.status &= !status::IRQ;
        }
    }

    pub(super) fn dtr_enabled(&self) -> bool {
        self.command & command::DTR != 0
    }

    /// Receiver IRQ armed: command bit 1 clear (enabled) AND DTR enabled
    /// (module doc: DTR disabled gates off rx-IRQ).
    pub(super) fn rx_irq_enabled(&self) -> bool {
        self.dtr_enabled() && self.command & command::RX_IRQ_DISABLE == 0
    }

    /// Transmitter IRQ armed: transmitter control = `IRQ_ENABLED` AND DTR
    /// enabled.
    pub(super) fn tx_irq_enabled(&self) -> bool {
        self.dtr_enabled() && self.tx_control() == tx_control::IRQ_ENABLED
    }

    pub(super) fn tx_control(&self) -> u8 {
        (self.command & command::TX_CONTROL_MASK) >> command::TX_CONTROL_SHIFT
    }

    pub(super) fn break_active(&self) -> bool {
        self.tx_control() == tx_control::BREAK
    }

    /// Check the DCD/DSR inputs for a change since the last tick and arm
    /// their IRQ source if DTR is enabled (module doc: MAME ties this to
    /// the RX clock with admittedly-unresolved exact timing; this model
    /// resolves it once per `tick` call instead).
    pub(super) fn tick_modem_lines(&mut self) {
        if self.dcd_level != self.dcd_checked {
            self.dcd_checked = self.dcd_level;
            if self.dtr_enabled() {
                self.irq_sources |= irq_source::DCD;
                self.update_irq_output();
            }
        }
        if self.dsr_level != self.dsr_checked {
            self.dsr_checked = self.dsr_level;
            if self.dtr_enabled() {
                self.irq_sources |= irq_source::DSR;
                self.update_irq_output();
            }
        }
    }
}
