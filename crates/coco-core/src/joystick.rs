//! CoCo analog joysticks (`DESIGN.md` §7).
//!
//! There is no joystick register: software ramps the 6-bit DAC (PIA1 PA2–PA7,
//! $FF20) and reads the comparator on PIA0 PA7 ($FF00 bit 7), which is high
//! while the DAC level is at or below the selected pot. The pot is chosen by
//! an analog mux driven by PIA0's CA2 (SEL1: 0 = X, 1 = Y) and CB2 (SEL2:
//! 0 = right stick, 1 = left stick). Fire buttons sit on the keyboard row
//! lines PA0–PA3 and pull them low regardless of the column strobe.
//! (Verified: SEB Unravelled II Appendix A $FF00/$FF01/$FF03; MAME `coco.cpp`
//! `poll_keyboard`/`joyin` — PA7 = `dac_output() <= joyval`.)

use serde::{Deserialize, Serialize};

/// Pot values are 6-bit, matching the DAC range software sweeps.
pub const AXIS_MAX: u8 = 63;
/// Idle/center pot value.
pub const AXIS_CENTER: u8 = 32;

/// Stick selector (SEL2 = PIA0 CB2): 0 = right port, 1 = left port.
pub const RIGHT: usize = 0;
pub const LEFT: usize = 1;
/// Axis selector (SEL1 = PIA0 CA2): 0 = X, 1 = Y.
pub const AXIS_X: usize = 0;
pub const AXIS_Y: usize = 1;

/// PIA0 port-A row bits pulled low by fire buttons, per SEB Unravelled II:
/// PA0 = right button 1, PA1 = left button 1, PA2/PA3 = the CoCo 3 second
/// buttons (right/left).
const BUTTON_ROW_BITS: [[u8; 2]; 2] = [[0x01, 0x04], [0x02, 0x08]];

/// Both joystick ports: pot positions and button states, fed by the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Joysticks {
    /// `axes[stick][axis]` = pot value 0–63 (0 = left/up on real sticks).
    axes: [[u8; 2]; 2],
    /// `buttons[stick][n]` = button n held.
    buttons: [[bool; 2]; 2],
}

impl Default for Joysticks {
    fn default() -> Self {
        Self {
            axes: [[AXIS_CENTER; 2]; 2],
            buttons: [[false; 2]; 2],
        }
    }
}

impl Joysticks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a pot position (clamped to the 6-bit range).
    pub fn set_axis(&mut self, stick: usize, axis: usize, value: u8) {
        self.axes[stick & 1][axis & 1] = value.min(AXIS_MAX);
    }

    pub fn set_button(&mut self, stick: usize, button: usize, down: bool) {
        self.buttons[stick & 1][button & 1] = down;
    }

    /// Comparator output for the mux-selected pot: high while the DAC level
    /// is at or below the pot (MAME `coco.cpp`: `dac_output() <= joyval`).
    pub fn compare(&self, stick: usize, axis: usize, dac: u8) -> bool {
        dac <= self.axes[stick & 1][axis & 1]
    }

    /// Mask of PIA0 PA row lines the held buttons pull low. Buttons bypass
    /// the keyboard column strobe (and so also trip the GIME EI1 source —
    /// SEB: they cannot be masked off).
    pub fn button_rows(&self) -> u8 {
        let mut mask = 0;
        for (stick, rows) in BUTTON_ROW_BITS.iter().enumerate() {
            for (button, &bit) in rows.iter().enumerate() {
                if self.buttons[stick][button] {
                    mask |= bit;
                }
            }
        }
        mask
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparator_is_high_while_dac_at_or_below_pot() {
        let mut j = Joysticks::new();
        j.set_axis(RIGHT, AXIS_X, 40);
        assert!(j.compare(RIGHT, AXIS_X, 0));
        assert!(j.compare(RIGHT, AXIS_X, 40));
        assert!(!j.compare(RIGHT, AXIS_X, 41));
    }

    #[test]
    fn axes_clamp_to_six_bits() {
        let mut j = Joysticks::new();
        j.set_axis(LEFT, AXIS_Y, 200);
        assert!(j.compare(LEFT, AXIS_Y, AXIS_MAX));
    }

    #[test]
    fn button_rows_match_seb_wiring() {
        let mut j = Joysticks::new();
        j.set_button(RIGHT, 0, true);
        assert_eq!(j.button_rows(), 0x01);
        j.set_button(LEFT, 0, true);
        j.set_button(RIGHT, 1, true);
        j.set_button(LEFT, 1, true);
        assert_eq!(j.button_rows(), 0x0F);
    }
}
