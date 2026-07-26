//! TX/RX frame engine and the baud/frame-timing math that sizes it: the
//! byte-level stand-in for the real 6551's bit-serial shifter (see the
//! `acia6551` module doc's "byte-level timing divergence").

use super::{
    ACIA_CRYSTAL_HZ, Acia6551, BAUD_CLOCK_DIVISOR, BAUD_DIVIDER, CPU_HZ, START_BITS, command,
    control, irq_source, status,
};

impl Acia6551 {
    /// If the transmitter is idle and a byte is pending (TDRE clear) and
    /// DTR is enabled and BREAK is not active: consume TDR into the shifter,
    /// set TDRE (freeing TDR for a new write) and arm the TDRE IRQ source if
    /// enabled, and start the frame timer. This is the "consume-at-start"
    /// moment MAME fires TDRE/IRQ at — deliberately *not* at frame end (see
    /// [`Acia6551::complete_tx_frame`]).
    pub(super) fn start_tx_frame_if_ready(&mut self) {
        if self.tx_timer.is_some() {
            return;
        }
        if !self.dtr_enabled() || self.break_active() {
            return;
        }
        if self.status & status::TDRE != 0 {
            return; // no pending byte
        }
        self.tx_shift_byte = self.tdr;
        self.status |= status::TDRE;
        if self.tx_irq_enabled() {
            self.irq_sources |= irq_source::TDRE;
            self.update_irq_output();
        }
        self.tx_timer = Some(self.cycles_per_frame());
    }

    /// Frame-end: the shifted byte becomes available on the wire and the
    /// transmitter is free again (the next consume, if TDR was rewritten
    /// mid-frame, happens on the following [`Acia6551::tick_tx`] iteration).
    fn complete_tx_frame(&mut self) {
        self.tx_output.push_back(self.tx_shift_byte);
    }

    pub(super) fn tick_rx(&mut self, cycles: u32) {
        if let Some(remaining) = self.rx_timer {
            if cycles >= remaining {
                self.rx_timer = None;
                self.complete_rx_frame();
            } else {
                self.rx_timer = Some(remaining - cycles);
            }
        }
    }

    /// Runs the TX frame timer to completion, possibly chaining straight
    /// into the next frame within the same `tick` call if a byte was
    /// already pending and `cycles` outlasts the current frame.
    pub(super) fn tick_tx(&mut self, mut cycles: u32) {
        loop {
            if self.tx_timer.is_none() {
                self.start_tx_frame_if_ready();
            }
            let Some(remaining) = self.tx_timer else {
                break;
            };
            if cycles >= remaining {
                cycles -= remaining;
                self.tx_timer = None;
                self.complete_tx_frame();
            } else {
                self.tx_timer = Some(remaining - cycles);
                break;
            }
        }
    }

    /// RX frame completion: the pending byte always replaces RDR. Overrun
    /// is set if RDRF was already set (the previous byte was never read).
    /// RDRF is set unconditionally; its IRQ source is armed only if rx-IRQ
    /// is enabled. In echo mode the byte is also queued onto the TX wire
    /// (module doc's byte-level echo approximation — this skips MAME's
    /// force-to-mark-during-overrun nuance).
    fn complete_rx_frame(&mut self) {
        let byte = self.rx_pending_byte;
        if self.status & status::RDRF != 0 {
            self.status |= status::OVERRUN;
        }
        self.rdr = byte;
        self.status |= status::RDRF;
        if self.rx_irq_enabled() {
            self.irq_sources |= irq_source::RDRF;
            self.update_irq_output();
        }
        if self.command & command::ECHO != 0 {
            self.tx_output.push_back(byte);
        }
    }

    fn baud_divider(&self) -> u32 {
        BAUD_DIVIDER[(self.control & control::BAUD_MASK) as usize]
    }

    /// `8 - field`, field = control bits 6:5 (MAME `mos6551.cpp`
    /// `write_control`: 00=8, 01=7, 10=6, 11=5 data bits).
    fn word_length(&self) -> u32 {
        8 - u32::from((self.control & control::WORD_LENGTH_MASK) >> control::WORD_LENGTH_SHIFT)
    }

    /// Any odd parity-field value (odd/even/mark/space) adds one bit to the
    /// frame; even values mean parity disabled (see [`command::PARITY_MASK`]
    /// doc).
    fn parity_enabled(&self) -> bool {
        let field = (self.command & command::PARITY_MASK) >> command::PARITY_SHIFT;
        field & 1 != 0
    }

    /// 1, or 2 with [`control::STOP_BITS_2`] set. MAME's real 6551 gives a
    /// 5-bit word + 2-stop-bits combination 1.5 stop bits instead of 2; this
    /// byte-level model collapses that case to a plain 2 (module doc).
    fn stop_bits(&self) -> u32 {
        if self.control & control::STOP_BITS_2 != 0 {
            2
        } else {
            1
        }
    }

    fn frame_bits(&self) -> u32 {
        START_BITS + self.word_length() + u32::from(self.parity_enabled()) + self.stop_bits()
    }

    /// CPU cycles (crate clock, [`CPU_HZ`]) for one complete RX or TX frame
    /// at the currently configured baud/word-length/parity/stop-bits:
    /// `cycles_per_frame = frame_bits * divider * `[`BAUD_CLOCK_DIVISOR`]`
    /// `* `[`CPU_HZ`]` / `[`ACIA_CRYSTAL_HZ`]`` (u64 math to avoid overflow
    /// and rounding surprises before the final division).
    pub(super) fn cycles_per_frame(&self) -> u32 {
        let frame_bits = u64::from(self.frame_bits());
        let divider = u64::from(self.baud_divider());
        let cycles = frame_bits * divider * BAUD_CLOCK_DIVISOR * CPU_HZ / ACIA_CRYSTAL_HZ;
        cycles as u32
    }
}
