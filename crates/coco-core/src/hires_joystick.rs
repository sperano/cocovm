//! Tandy Hi-Res Joystick Interface (26-3025) and CoCo Max III hi-res interface: both an RC
//! one-shot timed off a trigger source and read back through the stock PA7 comparator. No
//! datasheet exists; ported from MAME's `coco_tandy_hires_joy`/`coco_cm3_hires_joy` devices
//! (`src/mame/trs/coco.cpp`, `hires_trigger`).

use serde::{Deserialize, Serialize};

/// One-shot delay curve, per MAME `coco_tandy_hires_joy`'s ctor.
const TANDY_OFFSET_US: f64 = 560.0;
/// See [`TANDY_OFFSET_US`].
const TANDY_SPAN_US: f64 = 4856.0;

/// One-shot delay curve, per MAME `coco_cm3_hires_joy`'s ctor.
const COCOMAX3_OFFSET_US: f64 = 500.0;
/// See [`COCOMAX3_OFFSET_US`].
const COCOMAX3_SPAN_US: f64 = 2624.0;

/// Which hi-res joystick interface, if any, is plugged into a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HiResInterface {
    #[default]
    None,
    /// Tandy 26-3025.
    Tandy,
    /// CoCo Max III.
    CoCoMax3,
}

/// One port's pending one-shot: which axis is charging, and how many
/// slow-clock cycles remain until it saturates.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct ChargeTimer {
    axis: usize,
    remaining: u32,
}

/// The two candidate trigger sources a PIA0/PIA1 write can carry: the 6-bit DAC reading
/// (Tandy's trigger) and the PIA0 port-A PA0-PA3 pin nibble (CoCo Max III's trigger).
/// [`HiResPort::observe`] picks whichever its own `kind` cares about.
#[derive(Debug, Clone, Copy)]
pub struct TriggerInputs {
    pub dac: u8,
    pub pa_nibble: u8,
}

/// One joystick port's hi-res interface state (Tandy or CoCo Max III): MAME's
/// `hires_trigger(state, now, axis, joy_val)` reimplemented with an elapsed-cycle counter
/// standing in for its absolute `now`.
///
/// Doesn't model MAME's mux-output re-entrancy quirk (it re-enters
/// `hires_trigger` when a saturation flips the mux output it reads back — an
/// emergent effect of its device wiring, not modeled here). Also unlike MAME,
/// a fresh charge below clears a previously-saturated axis's slot rather than
/// leaving it latched — the more defensible read of a retriggered one-shot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HiResPort {
    kind: HiResInterface,
    /// Level of this port's trigger source as of the last [`Self::observe`] call.
    was_low: bool,
    /// Cycles elapsed since the current charge cycle started
    /// (MAME's `now - charge_start`).
    charge_elapsed: u32,
    timer: Option<ChargeTimer>,
    /// Per-axis "slot": whether that axis's one-shot has saturated (read
    /// back as pot 0x3F) since it last started charging (read back as
    /// pot 0). Indexed like [`crate::joystick::AXIS_X`]/`AXIS_Y`.
    saturated: [bool; 2],
}

impl Default for HiResPort {
    fn default() -> Self {
        Self {
            kind: HiResInterface::None,
            was_low: false,
            charge_elapsed: 0,
            timer: None,
            saturated: [false; 2],
        }
    }
}

impl HiResPort {
    pub fn kind(&self) -> HiResInterface {
        self.kind
    }

    /// Install (or remove) this port's interface, resetting all timing state.
    pub fn set_kind(&mut self, kind: HiResInterface) {
        *self = Self {
            kind,
            ..Self::default()
        };
    }

    /// MAME `hires_trigger`: called on every DAC write, PIA0 port-A pin-nibble write, and
    /// analog-mux address change while this port is CB2-selected. `trigger` carries both
    /// candidate trigger sources; this port's own `kind` picks the one it cares about (the
    /// DAC for Tandy, the PA0-PA3 nibble for CoCo Max III) and is low when that reading is
    /// exactly 0. `axis` is the CA2-selected axis; `pot` is that axis's current pot position.
    pub fn observe(&mut self, trigger: TriggerInputs, axis: usize, pot: u16) {
        let trigger_low = match self.kind {
            HiResInterface::None => return,
            HiResInterface::Tandy => trigger.dac == 0,
            HiResInterface::CoCoMax3 => trigger.pa_nibble == 0,
        };
        let axis = axis & 1;
        if trigger_low {
            let duration = duration_cycles(self.kind, pot);
            let fresh_charge = !self.was_low || self.charge_elapsed >= duration;
            if fresh_charge {
                // high -> low, or low -> low after a full period already elapsed: the mux
                // switched to a dormant axis, so the one-shot restarts against it. Clears the
                // axis's slot even if it was still latched saturated — see this struct's doc.
                self.charge_elapsed = 0;
                self.timer = Some(ChargeTimer {
                    axis,
                    remaining: duration,
                });
                self.saturated[axis] = false;
            } else {
                // low -> low, mid-charge: reschedule against the (possibly new) axis/duration.
                self.timer = Some(ChargeTimer {
                    axis,
                    remaining: duration - self.charge_elapsed,
                });
            }
        } else {
            // High: cancel the pending one-shot and un-saturate the now-selected axis only.
            self.timer = None;
            self.saturated[axis] = false;
        }
        self.was_low = trigger_low;
    }

    /// Advance `cycles` slow-clock cycles: the elapsed-charge counter, and
    /// the pending one-shot's countdown to saturation. `cycles` must already
    /// be in slow-clock terms — the double-speed conversion happens once, in
    /// [`crate::joystick::Joysticks::tick`], not here.
    pub fn tick(&mut self, cycles: u32) {
        if self.kind == HiResInterface::None {
            return;
        }
        self.charge_elapsed = self.charge_elapsed.saturating_add(cycles);
        if let Some(timer) = &mut self.timer {
            if cycles >= timer.remaining {
                self.saturated[timer.axis] = true;
                self.timer = None;
            } else {
                timer.remaining -= cycles;
            }
        }
    }

    /// PA7 comparator readback for `axis`: high while the substituted pot
    /// (0x3F once saturated, else 0) is strictly greater than `dac`
    /// (`slot > dac`, MAME's readback for a hi-res port).
    pub fn comparator(&self, axis: usize, dac: u8) -> bool {
        const SATURATED_POT: u8 = 0x3F;
        self.saturated[axis & 1] && dac < SATURATED_POT
    }
}

/// One-shot delay for `pot` (0..=[`crate::joystick::POT_MAX`]) on `kind`, in
/// slow-clock CPU cycles (`crate::CPU_HZ`) — a real-time RC delay, unaffected
/// by the double-speed poke.
pub fn duration_cycles(kind: HiResInterface, pot: u16) -> u32 {
    let (offset_us, span_us) = match kind {
        HiResInterface::None => return 0,
        HiResInterface::Tandy => (TANDY_OFFSET_US, TANDY_SPAN_US),
        HiResInterface::CoCoMax3 => (COCOMAX3_OFFSET_US, COCOMAX3_SPAN_US),
    };
    let duration_us = f64::from(pot) / f64::from(crate::joystick::POT_MAX) * span_us + offset_us;
    (duration_us * crate::CPU_HZ / 1_000_000.0).round() as u32
}

#[cfg(test)]
#[path = "hires_joystick_test.rs"]
mod tests;
