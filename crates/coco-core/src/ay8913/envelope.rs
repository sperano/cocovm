//! The single shared envelope generator (R11-R13): all three channels that
//! select envelope mode (R8/R9/R10 bit 4) read the same [`Envelope::volume`].

use serde::{Deserialize, Serialize};

use super::ENV_STEP_MASK;

/// Envelope-shape register (R13) bit layout (MAME `ay8910.h`
/// `envelope_t::set_shape`).
mod shape {
    pub const HOLD: u8 = 0x01;
    pub const ALTERNATE: u8 = 0x02;
    pub const ATTACK: u8 = 0x04;
    pub const CONTINUE: u8 = 0x08;
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub(super) struct Envelope {
    /// Internal-step counter toward the next level change.
    count: u32,
    /// Current level, counting down from [`ENV_STEP_MASK`] to 0 each ramp
    /// (signed so the "just went negative" transition — MAME's `step < 0`
    /// check — is representable before it gets masked/clamped back into
    /// range).
    step: i32,
    /// XORed with `step` to produce [`Envelope::volume`] — the mechanism
    /// that turns a plain down-ramp into attack (rising) shapes and flips
    /// direction for the alternating shapes (MAME `envelope_t::volume =
    /// step ^ attack`).
    attack: u8,
    /// Shape bit 0 (post CONT=0 folding): stick at the final level instead
    /// of repeating.
    hold: bool,
    /// Shape bit 1 (post CONT=0 folding): invert `attack` at the end of
    /// each ramp, turning a sawtooth into a triangle.
    alternate: bool,
    /// Set once `hold` has stopped the ramp; while set, `step`/`attack`
    /// don't change.
    holding: bool,
}

impl Envelope {
    /// Current output level, 0-15 ([`ENV_STEP_MASK`]).
    pub(super) fn volume(&self) -> u8 {
        (self.step as u8) ^ self.attack
    }

    /// R13 write: (re)starts the envelope at the top of a fresh ramp (MAME
    /// `envelope_t::set_shape`). CONT=0 shapes fold to their CONT=1 equivalent
    /// (hold forced on) since real AY-3-8910 silicon only implements 10 of
    /// the 16 shape codes distinctly.
    pub(super) fn set_shape(&mut self, shape_byte: u8) {
        self.attack = if shape_byte & shape::ATTACK != 0 {
            ENV_STEP_MASK as u8
        } else {
            0
        };
        if shape_byte & shape::CONTINUE == 0 {
            self.hold = true;
            self.alternate = self.attack != 0;
        } else {
            self.hold = shape_byte & shape::HOLD != 0;
            self.alternate = shape_byte & shape::ALTERNATE != 0;
        }
        self.step = ENV_STEP_MASK;
        self.holding = false;
    }

    /// One internal step (master_clock/8) of envelope pacing; `period` internal
    /// steps per level (already multiplied by `ENVELOPE_STEP_MULTIPLIER` by the caller).
    pub(super) fn step_once(&mut self, period: u32) {
        if self.holding {
            return;
        }
        self.count += 1;
        if self.count < period {
            return;
        }
        self.count = 0;
        self.step -= 1;
        if self.step >= 0 {
            return;
        }
        if self.hold {
            if self.alternate {
                self.attack ^= ENV_STEP_MASK as u8;
            }
            self.holding = true;
            self.step = 0;
        } else {
            // MAME re-checks `alternate` against masked `step`, always true
            // for the only reachable negative value (-1) — fires once per
            // ramp when set.
            if self.alternate && (self.step & (ENV_STEP_MASK + 1)) != 0 {
                self.attack ^= ENV_STEP_MASK as u8;
            }
            self.step &= ENV_STEP_MASK;
        }
    }
}
