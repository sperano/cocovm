//! Timer 1: an 8-bit decrementer behind a 5-bit prescaler, clocked from the
//! oscillator (MAME `timer_run`/`timer_reload`/`timer_tick_low`). MAME
//! schedules one event per decrement; this counts CPU cycles instead, which
//! observes the same boundaries since MAME's timer is only ever sampled
//! between instructions. (MAME's own phase wanders by up to a cycle when
//! other devices' timers chop its timeslices at fractional cycles; this
//! model stays on the instruction grid.)

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Oscillator clocks per decrement at prescaler 0 (MAME: `fOSC/16`).
const OSC_CLOCKS_PER_TICK: u32 = 16;
/// Oscillator clocks per CPU cycle (MAME `m_divider = 2`).
const OSC_CLOCKS_PER_CYCLE: u32 = 2;
/// CPU cycles per decrement at prescaler 0.
const CYCLES_PER_TICK: u32 = OSC_CLOCKS_PER_TICK / OSC_CLOCKS_PER_CYCLE;

/// T1CTL bits: d0-d4 prescaler reload, d5 (masked off on timer 1), d6
/// clock source (0 = internal), d7 start.
const PRESCALER_MASK: u8 = 0x1F;
const CASCADE_OR_HALT_BIT: u8 = 0x20;
const RUN_MODE_MASK: u8 = 0xE0;
/// Started, internal source, not cascaded: the only mode MAME runs.
const RUN_MODE_INTERNAL: u8 = 0x80;
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

    fn running(&self) -> bool {
        self.control & RUN_MODE_MASK == RUN_MODE_INTERNAL
    }

    /// CPU cycles between decrements at the current prescaler.
    fn period_cycles(&self) -> u32 {
        CYCLES_PER_TICK * (u32::from(self.control & PRESCALER_MASK) + 1)
    }

    /// Advance by `cycles`; true if the decrementer underflowed (INT2). Two
    /// underflows within one step collapse into one flag, as they would into
    /// IOCNT0's single flag bit.
    pub(crate) fn tick(&mut self, cycles: u32) -> bool {
        if std::mem::take(&mut self.armed_this_step) || !self.running() {
            return false;
        }
        let period = self.period_cycles();
        self.phase += cycles;
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
}
