//! Timer 1: an 8-bit decrementer behind a 5-bit prescaler, clocked from
//! either the internal oscillator or an external pin (T1CTL bit 6,
//! SPND001B 3-42 "Clock Source Control"). Internal-clock timing counts CPU
//! cycles rather than MAME's per-event scheduling, which observes the same
//! boundaries since MAME only ever samples the timer between instructions.
//! (MAME's own phase wanders by up to a cycle when other devices' timers
//! chop its timeslices at fractional cycles; this model stays on the
//! instruction grid.) MAME does not implement Event-Counter mode at all
//! (`tms7000.cpp`'s own TODO admits it); this follows the manual instead.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Oscillator clocks per internal-source prescaler pulse at prescaler 0
/// (MAME: `fOSC/16`).
const OSC_CLOCKS_PER_TICK: u32 = 16;
/// Oscillator clocks per CPU cycle (MAME `m_divider = 2`).
const OSC_CLOCKS_PER_CYCLE: u32 = 2;
/// Phase-units per raw prescaler input pulse: that many CPU cycles for the
/// internal oscillator source, or one unit per external EC1 edge (SPND001B
/// 3-42: "each positive transition ... decrements the count chain").
const CYCLES_PER_TICK: u32 = OSC_CLOCKS_PER_TICK / OSC_CLOCKS_PER_CYCLE;

/// T1CTL bits: d0-d4 prescaler reload, d5 (masked off on timer 1), d6
/// clock source (0 = internal oscillator, 1 = external EC1 pin), d7 start.
const PRESCALER_MASK: u8 = 0x1F;
const CASCADE_OR_HALT_BIT: u8 = 0x20;
const SOURCE_BIT: u8 = 0x40;
const START_BIT: u8 = 0x80;

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Clone, Debug, Default)]
pub(crate) struct Timer1 {
    /// T1DATA: reload value for the decrementer.
    data: u8,
    /// T1CTL as written (bit 5 stripped).
    control: u8,
    /// Current count; reloads from `data` on underflow.
    decrementer: u8,
    /// CPU cycles accrued toward the next decrement.
    phase: u32,
    /// Decrementer value latched when INT3 rises; read back through T1CTL.
    capture: u8,
    /// A control write happened during the instruction in progress: its
    /// cycles were charged before the write, so the first period starts
    /// after it (MAME schedules the tick from the write's own time).
    armed_this_step: bool,
}

impl Timer1 {
    pub(crate) fn decrementer(&self) -> u8 {
        self.decrementer
    }

    pub(crate) fn capture_latch(&self) -> u8 {
        self.capture
    }

    pub(crate) fn capture(&mut self) {
        self.capture = self.decrementer;
    }

    pub(crate) fn write_data(&mut self, val: u8) {
        self.data = val;
    }

    /// A control write restarts the timer from the reload value if the start
    /// bit is set, else stops it in place (MAME `timer_reload`).
    pub(crate) fn write_control(&mut self, val: u8) {
        self.control = val & !CASCADE_OR_HALT_BIT;
        self.phase = 0;
        self.armed_this_step = true;
        if self.control & START_BIT != 0 {
            self.decrementer = self.data;
        }
    }

    fn started(&self) -> bool {
        self.control & START_BIT != 0
    }

    fn external_source(&self) -> bool {
        self.control & SOURCE_BIT != 0
    }

    fn running_internal(&self) -> bool {
        self.started() && !self.external_source()
    }

    fn running_external(&self) -> bool {
        self.started() && self.external_source()
    }

    /// CPU cycles between decrements at the current prescaler.
    fn period_cycles(&self) -> u32 {
        CYCLES_PER_TICK * (u32::from(self.control & PRESCALER_MASK) + 1)
    }

    /// Invariants `tick` needs but `Deserialize` can't check: the
    /// cascade/halt bit stays clear, and `phase` stays below the period.
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.control & CASCADE_OR_HALT_BIT != 0 {
            return Err("timer1 control has the hardwired cascade/halt bit set");
        }
        if self.phase >= self.period_cycles() {
            return Err("timer1 phase is not less than its period");
        }
        Ok(())
    }

    /// Advance the prescaler/decrementer chain by `phase_units` (shared by
    /// the internal and external clock sources, SPND001B 3-44/3-45's
    /// four-step description applies identically to both). Returns whether
    /// the decrementer underflowed.
    fn advance(&mut self, phase_units: u32) -> bool {
        let period = self.period_cycles();
        self.phase += phase_units;
        let mut underflowed = false;
        while self.phase >= period {
            self.phase -= period;
            match self.decrementer.checked_sub(1) {
                Some(next) => self.decrementer = next,
                None => {
                    self.decrementer = self.data;
                    underflowed = true;
                }
            }
        }
        underflowed
    }

    /// Advance by `cycles` on the internal oscillator source; true if the
    /// decrementer underflowed (INT2). Two underflows within one step
    /// collapse into one flag, as they would into IOCNT0's single flag bit.
    /// A no-op while the external source is selected (SPND001B 3-42): its
    /// counting is driven by [`Self::ec1_edge`] instead.
    pub(crate) fn tick(&mut self, cycles: u32) -> bool {
        if std::mem::take(&mut self.armed_this_step) || !self.running_internal() {
            return false;
        }
        self.advance(cycles)
    }

    /// One positive transition on Timer 1's external event-counter pin
    /// (Port A7/EC1); a no-op unless the timer is started with the
    /// external source selected (SPND001B 3-42/3-43). True if the
    /// decrementer underflowed (INT2).
    pub(crate) fn ec1_edge(&mut self) -> bool {
        if !self.running_external() {
            return false;
        }
        self.advance(CYCLES_PER_TICK)
    }
}

#[cfg(test)]
#[path = "timer_test.rs"]
mod tests;
