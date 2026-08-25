//! Register read/write internals: the four `$FF68`-`$FF6B` offsets'
//! side-effecting behavior, dispatched from [`ACIA6551::read`]/
//! [`ACIA6551::write`].

use super::{ACIA6551, command, irq_source, status, tx_control};

impl ACIA6551 {
    /// RDR read: returns the byte, clears RDRF and the error bits — does
    /// *not* touch the IRQ output (needs a status read or disabling RDRF's IRQ source).
    pub(super) fn read_rdr(&mut self) -> u8 {
        let val = self.rdr;
        self.status &=
            !(status::RDRF | status::PARITY_ERROR | status::FRAMING_ERROR | status::OVERRUN);
        val
    }

    /// Status read: returns the pre-clear snapshot, then clears every armed
    /// IRQ source and the IRQ output bit; leaves other status bits untouched.
    pub(super) fn read_status(&mut self) -> u8 {
        let val = self.status;
        self.irq_sources = 0;
        self.update_irq_output();
        val
    }

    /// TDR write: latches the byte and clears TDRE (a mid-frame rewrite is
    /// picked up at frame-end), then attempts an immediate consume if idle.
    pub(super) fn write_tdr(&mut self, val: u8) {
        self.tdr = val;
        self.status &= !status::TDRE;
        self.start_tx_frame_if_ready();
    }

    /// Command register write: replaces the whole register, clears the
    /// RDRF/TDRE IRQ sources if their arming condition is now false, and
    /// attempts an immediate transmit-start (DTR/BREAK may have changed).
    pub(super) fn write_command(&mut self, val: u8) {
        self.command = val;
        self.rts = self.tx_control() != tx_control::RTS_OFF;
        if !self.rx_irq_enabled() {
            self.irq_sources &= !irq_source::RDRF;
        }
        if !self.tx_irq_enabled() {
            self.irq_sources &= !irq_source::TDRE;
        }
        self.update_irq_output();
        self.start_tx_frame_if_ready();
    }

    /// Programmed reset (write to reg 1, value ignored): clears only OVERRUN
    /// and the DCD/DSR IRQ sources — RDRF/TDRE IRQ sources deliberately
    /// survive, unlike [`ACIA6551::write_command`]'s general rule. Command
    /// bits 0-4 clear (parity bits 7:5 survive); control is untouched.
    pub(super) fn programmed_reset(&mut self) {
        self.status &= !status::OVERRUN;
        self.irq_sources &= !(irq_source::DCD | irq_source::DSR);
        self.update_irq_output();

        const RESET_MASK: u8 =
            command::DTR | command::RX_IRQ_DISABLE | command::TX_CONTROL_MASK | command::ECHO;
        self.command &= !RESET_MASK;
        self.rts = false;
    }
}
