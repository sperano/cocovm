//! Register read/write internals: the four `$FF68`-`$FF6B` offsets'
//! side-effecting behavior, dispatched from [`Acia6551::read`]/
//! [`Acia6551::write`].

use super::{Acia6551, command, irq_source, status, tx_control};

impl Acia6551 {
    /// RDR read: returns the byte, then clears RDRF and all three error
    /// bits together (MAME `mos6551.cpp` `read_receive_data_register` — does
    /// *not* touch the IRQ output; that needs a status read or disabling
    /// the RDRF IRQ source via a command write).
    pub(super) fn read_rdr(&mut self) -> u8 {
        let val = self.rdr;
        self.status &=
            !(status::RDRF | status::PARITY_ERROR | status::FRAMING_ERROR | status::OVERRUN);
        val
    }

    /// Status read: returns the pre-clear snapshot, then (side effect)
    /// clears every armed IRQ source at once and drops the IRQ output bit.
    /// Does not touch parity/framing/overrun/RDRF/DCD/DSR (MAME
    /// `mos6551.cpp` `read_status_register`).
    pub(super) fn read_status(&mut self) -> u8 {
        let val = self.status;
        self.irq_sources = 0;
        self.update_irq_output();
        val
    }

    /// TDR write: latches the byte and clears TDRE immediately, even if a
    /// frame is already shifting (the 1-deep holding register — a rewrite
    /// before the current frame ends is picked up when it does, in
    /// [`Acia6551::tick_tx`]). Then attempts an immediate consume — if the
    /// transmitter happens to already be idle, MAME picks a freshly loaded
    /// TDR up as soon as it's written rather than waiting for the next
    /// clock tick.
    pub(super) fn write_tdr(&mut self, val: u8) {
        self.tdr = val;
        self.status &= !status::TDRE;
        self.start_tx_frame_if_ready();
    }

    /// Command register write (MAME `mos6551.cpp` `write_command`):
    /// replaces the whole register, then re-evaluates the two IRQ sources
    /// whose arming condition is a direct function of the command bits —
    /// disabling rx-IRQ clears the RDRF source, disabling tx-IRQ clears the
    /// TDRE source — and attempts an immediate transmit-start (DTR/BREAK
    /// may have just changed).
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

    /// Programmed reset (any write to reg 1, value ignored): clears *only*
    /// the overrun status bit and *only* the DCD/DSR IRQ-source bits — the
    /// RDRF/TDRE IRQ sources deliberately survive, unlike the general
    /// command-write rule in [`Acia6551::write_command`] (verified MAME
    /// `mos6551.cpp` `write_status_command_register` behavior: this is a
    /// narrower reset than a full command-register disable would produce).
    /// Command bits 0-4 are cleared (DTR off, rx-IRQ enabled, transmitter
    /// control = RTS_OFF, echo off); parity bits 7:5 survive. Control is
    /// untouched.
    pub(super) fn programmed_reset(&mut self) {
        self.status &= !status::OVERRUN;
        self.irq_sources &= !(irq_source::DCD | irq_source::DSR);
        self.update_irq_output();

        const RESET_MASK: u8 = command::DTR
            | command::RX_IRQ_DISABLE
            | command::TX_CONTROL_MASK
            | command::ECHO;
        self.command &= !RESET_MASK;
        self.rts = false;
    }
}
