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
//!
//! A port's pot is stored at 10-bit resolution ([`POT_MAX`]) even though the
//! stock comparator only ever sees its top 6 bits — a plugged-in Tandy hi-res
//! interface ([`crate::hires_joystick`]) needs the extra precision the DAC
//! sweep alone can't read back.

use serde::{Deserialize, Serialize};

use crate::hires_joystick::{HiResInterface, HiResPort};

/// Pot values the stock comparator sweeps: the DAC's own 6-bit range.
pub const AXIS_MAX: u8 = 63;
/// Idle/center pot value, 6-bit.
pub const AXIS_CENTER: u8 = 32;

/// Full pot resolution a Tandy hi-res port reads (`crate::hires_joystick`'s
/// `PORT_BIT(0x3ff, ...)`).
pub const POT_MAX: u16 = 1023;
/// Idle/center pot value, 10-bit.
pub const POT_CENTER: u16 = 512;

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

/// A plain `#[serde(default)]` would centre nothing (0/0 = full-left/up), so
/// an old snapshot without this field would restore the pots pinned to a
/// corner instead of idle center.
fn default_pots() -> [[u16; 2]; 2] {
    [[POT_CENTER; 2]; 2]
}

/// Both joystick ports: pot positions, button states, and any plugged-in
/// hi-res interface, fed by the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Joysticks {
    /// `pots[stick][axis]` = pot position, 0-1023 (0 = left/up on real
    /// sticks). [`Self::set_axis`] writes only the top 6 bits, matching every
    /// pre-hi-res caller; [`Self::set_pot`] writes the full range.
    #[serde(default = "default_pots")]
    pots: [[u16; 2]; 2],
    /// `buttons[stick][n]` = button n held.
    buttons: [[bool; 2]; 2],
    /// `hires[stick]` = that port's plugged-in hi-res interface, if any.
    /// `HiResPort::default()` is already the correct "no interface" value, so
    /// a plain default (`[HiResInterface::None; _]`-equivalent) is fine here.
    #[serde(default)]
    hires: [HiResPort; 2],
    /// Fractional slow-clock cycle carried across [`Self::tick`] calls while
    /// the double-speed poke is active, so repeated odd-cycle instructions
    /// don't lose precision to integer halving. See [`Self::tick`].
    #[serde(default)]
    fast_carry: bool,
}

impl Default for Joysticks {
    fn default() -> Self {
        Self {
            pots: default_pots(),
            buttons: [[false; 2]; 2],
            hires: [HiResPort::default(), HiResPort::default()],
            fast_carry: false,
        }
    }
}

impl Joysticks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a pot position from a 6-bit value (clamped to [`AXIS_MAX`]),
    /// mapped up to the 10-bit range as `(v << 4) | (v >> 2)` so that
    /// `pot(stick, axis) >> 4 == v` — a no-op for every caller that only
    /// ever produces 6-bit values.
    pub fn set_axis(&mut self, stick: usize, axis: usize, value: u8) {
        let v = u16::from(value.min(AXIS_MAX));
        self.pots[stick & 1][axis & 1] = (v << 4) | (v >> 2);
    }

    /// Set a pot position at full 10-bit resolution (clamped to [`POT_MAX`]).
    pub fn set_pot(&mut self, stick: usize, axis: usize, value: u16) {
        self.pots[stick & 1][axis & 1] = value.min(POT_MAX);
    }

    /// Current pot position, 10-bit.
    pub fn pot(&self, stick: usize, axis: usize) -> u16 {
        self.pots[stick & 1][axis & 1]
    }

    pub fn set_button(&mut self, stick: usize, button: usize, down: bool) {
        self.buttons[stick & 1][button & 1] = down;
    }

    /// Install (or remove, with [`HiResInterface::None`]) a hi-res interface
    /// on `stick`. There's only one physical DAC to time a one-shot off of,
    /// so installing [`HiResInterface::Tandy`] here removes it from the
    /// other port first (MAME refuses a second `coco_tandy_hires_joy`).
    pub fn set_hires(&mut self, stick: usize, kind: HiResInterface) {
        let stick = stick & 1;
        if kind == HiResInterface::Tandy {
            self.hires[stick ^ 1].set_kind(HiResInterface::None);
        }
        self.hires[stick].set_kind(kind);
    }

    /// Which hi-res interface, if any, is plugged into `stick`.
    pub fn hires(&self, stick: usize) -> HiResInterface {
        self.hires[stick & 1].kind()
    }

    /// Advance every installed hi-res interface's one-shot timer by `cycles`
    /// raw CPU cycles. The RC one-shot is a real-time circuit
    /// (`crate::hires_joystick::duration_cycles` is expressed in slow-clock
    /// terms), so under the double-speed poke (`cpu_fast`, selected per
    /// variant exactly like `Machine::cycles_per_field`) each raw cycle is
    /// only half a slow-clock cycle; the odd cycle carries into the next call
    /// via [`Self::fast_carry`] rather than being dropped.
    pub fn tick(&mut self, cycles: u32, cpu_fast: bool) {
        let credited = if cpu_fast {
            let total = cycles + u32::from(self.fast_carry);
            self.fast_carry = total & 1 != 0;
            total / 2
        } else {
            cycles
        };
        for port in &mut self.hires {
            port.tick(credited);
        }
    }

    /// Feed a DAC change or analog-mux address change to `stick`'s hi-res
    /// interface (a no-op if it has none) — MAME's `hires_trigger`, run on
    /// every PIA1 port-A write and every PIA0 CA2/CB2 change
    /// (`SystemBus::write_pia1`/`SystemBus::joystick_mux`).
    pub fn observe_dac(&mut self, dac: u8, stick: usize, axis: usize) {
        let stick = stick & 1;
        let axis = axis & 1;
        let pot = self.pot(stick, axis);
        self.hires[stick].observe(dac == 0, axis, pot);
    }

    /// Comparator output for the mux-selected pot: high while the DAC level
    /// is at or below the pot (MAME `coco.cpp`: `dac_output() <= joyval`),
    /// or, on a port with a hi-res interface installed, that interface's own
    /// comparator readback instead.
    pub fn compare(&self, stick: usize, axis: usize, dac: u8) -> bool {
        let stick = stick & 1;
        let axis = axis & 1;
        let port = &self.hires[stick];
        if port.kind() == HiResInterface::None {
            dac <= (self.pots[stick][axis] >> 4) as u8
        } else {
            port.comparator(axis, dac)
        }
    }

    /// Mask of PIA0 PA row lines the held buttons pull low. Buttons bypass
    /// the keyboard column strobe, so they cannot be masked off.
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
#[path = "joystick_test.rs"]
mod tests;
