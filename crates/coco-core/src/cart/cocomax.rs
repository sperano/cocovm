//! Colorware's CoCo Max Hi-Res Input Module (1985): a mouse/joystick pak
//! wired to a multi-channel 8-bit ADC, decoded at `$FF90-$FF97` on the plain
//! CoCo 1/2 bus only (verified against the local MAME clone,
//! `src/devices/bus/coco/coco_max.cpp` `coco_pak_max_device` — the only
//! reference; there is no comparator, DAC, or joystick-port wiring on this
//! pak, unlike the CoCo's built-in mouse support).
//!
//! Each address in the window selects one ADC channel and starts a
//! conversion on it; a read returns the *previous* conversion's result
//! (`coco_pak_max_device::read`). Channels 0/1 are the Y/X axes, 2/3 are the
//! left/right buttons (active-low polarity: `$00` pressed, `$FF` released),
//! and 4-7 are not connected — selecting one leaves the latched result
//! unchanged. The CoCo 3's GIME owns this address range instead, so the
//! module is CoCo 1/2-only hardware; nothing here is reachable from the
//! GIME bus path.

use serde::{Deserialize, Serialize};

use super::{Cartridge, IO_OPEN_BUS};

/// First address of the ADC channel-select window.
pub const COCOMAX_IO_BASE: u16 = 0xFF90;
/// Last address of the ADC channel-select window.
pub const COCOMAX_IO_LAST: u16 = 0xFF97;

/// Bits of `addr` that select the ADC channel (`coco_pak_max_device::read`'s
/// `offset & 7`).
const CHANNEL_MASK: u16 = 0x7;

/// Y-axis ADC channel (`$FF90`).
const CHANNEL_Y: u16 = 0;
/// X-axis ADC channel (`$FF91`).
const CHANNEL_X: u16 = 1;
/// Left-button ADC channel (`$FF92`).
const CHANNEL_LEFT_BUTTON: u16 = 2;
/// Right-button ADC channel (`$FF93`).
const CHANNEL_RIGHT_BUTTON: u16 = 3;

/// Button-pressed ADC reading (active-low).
const BUTTON_PRESSED: u8 = 0x00;
/// Button-released ADC reading.
const BUTTON_RELEASED: u8 = 0xFF;

/// Power-on/reset axis position: dead-center.
const AXIS_CENTER: u8 = 0x80;

/// Colorware's CoCo Max Hi-Res Input Module: two 8-bit axes and two buttons,
/// read back one channel at a time through the latched-ADC protocol
/// described in the module doc.
#[derive(Debug, Serialize, Deserialize)]
pub struct CoCoMaxModule {
    /// Current X-axis reading (0-255), set by the frontend from the host
    /// pointer.
    x: u8,
    /// Current Y-axis reading (0-255).
    y: u8,
    /// Current button state: `[left, right]`.
    buttons: [bool; 2],
    /// The latched result of the last-started conversion — what the next
    /// read on any channel returns before starting its own conversion.
    result: u8,
}

impl CoCoMaxModule {
    /// A freshly plugged-in module: axes centered, buttons up, no
    /// conversion latched yet (matches [`Cartridge::reset`]).
    pub fn new() -> Self {
        Self {
            x: AXIS_CENTER,
            y: AXIS_CENTER,
            buttons: [false, false],
            result: 0,
        }
    }

    /// Set the current pointer position (0-255 per axis) — called by the
    /// frontend every frame from the host mouse.
    pub fn set_position(&mut self, x: u8, y: u8) {
        self.x = x;
        self.y = y;
    }

    /// Set the current left/right button state — called by the frontend
    /// from the host mouse's primary/secondary buttons.
    pub fn set_buttons(&mut self, left: bool, right: bool) {
        self.buttons = [left, right];
    }

    /// The live ADC reading for `channel` (0-7), or `None` for a
    /// not-connected channel (4-7), whose conversion leaves the latched
    /// result unchanged.
    fn channel_value(&self, channel: u16) -> Option<u8> {
        match channel {
            CHANNEL_Y => Some(self.y),
            CHANNEL_X => Some(self.x),
            CHANNEL_LEFT_BUTTON => Some(button_reading(self.buttons[0])),
            CHANNEL_RIGHT_BUTTON => Some(button_reading(self.buttons[1])),
            _ => None,
        }
    }
}

impl Default for CoCoMaxModule {
    fn default() -> Self {
        Self::new()
    }
}

fn button_reading(pressed: bool) -> u8 {
    if pressed {
        BUTTON_PRESSED
    } else {
        BUTTON_RELEASED
    }
}

impl Cartridge for CoCoMaxModule {
    /// Not decoded in the SCS window at all — the module lives entirely at
    /// `$FF90-$FF97` (see [`Cartridge::upper_io_read`]).
    fn read(&mut self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    fn write(&mut self, _addr: u16, _val: u8) {}

    /// A read returns the previous conversion's latched result, then starts
    /// a new one on `addr`'s channel (`addr & 7`). A not-connected channel
    /// (4-7) still returns the latched result but leaves it unchanged.
    fn upper_io_read(&mut self, addr: u16) -> u8 {
        let latched = self.result;
        if let Some(value) = self.channel_value(addr & CHANNEL_MASK) {
            self.result = value;
        }
        latched
    }

    /// Side-effect-free twin of [`Cartridge::upper_io_read`]: reports the
    /// latched result without starting a conversion.
    fn upper_io_peek(&self, _addr: u16) -> u8 {
        self.result
    }

    /// Only the latched result clears — the axes/buttons keep tracking
    /// whatever the frontend last set them to, like a real pointer's
    /// position surviving a reset.
    fn reset(&mut self) {
        self.result = 0;
    }
}

#[cfg(test)]
#[path = "cocomax_test.rs"]
mod tests;
