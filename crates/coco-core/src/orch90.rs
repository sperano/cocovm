//! Orchestra-90/CC cartridge (Tandy 26-3143): two 8-bit R-2R DACs for
//! software-driven stereo digital audio, plus an 8K program ROM.
//!
//! Facts verified against MAME `src/devices/bus/coco/coco_orch90.cpp` and
//! `docs/Lomont_CoCoHardware.pdf` (see `docs/plan-orchestra-90.md`):
//! `$FF7A` latches the left channel, `$FF7B` the right — both write-only
//! (74LS374 octal latch feeding an R-2R ladder per channel; no read path).
//! There is no on-cart timer or interrupt: sample timing is entirely the
//! CPU's delay loops. The pak autostarts by tying CART* to Q, exactly the
//! [`ROMPak`] autostart mechanism.
//!
//! Tier 1 (this module): latch decode + mono mix via
//! [`crate::cart::Cartridge::sound_level`]. Stereo output at full sample
//! rate is the audio-pipeline plan (`docs/plan-audio-pipeline.md`).

use serde::{Deserialize, Serialize};

use crate::cart::{Cartridge, IO_OPEN_BUS, ROMPak, ROMPakError};

/// Left-channel DAC latch (write-only).
pub const LEFT_DAC_REG: u16 = 0xFF7A;
/// Right-channel DAC latch (write-only).
pub const RIGHT_DAC_REG: u16 = 0xFF7B;

/// The Orchestra-90/CC cartridge.
#[derive(Serialize, Deserialize)]
pub struct Orch90 {
    /// The 8K program ROM in the CTS window ([`ROMPak`] reused for the
    /// MAME-compatible mirror-fill and half-swap indexing; its own autostart
    /// flag is irrelevant — [`Cartridge::cart_line_ties_q`] is overridden
    /// unconditionally here).
    rom: ROMPak,
    left: u8,
    right: u8,
}

impl std::fmt::Debug for Orch90 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Orch90")
            .field("left", &self.left)
            .field("right", &self.right)
            .finish_non_exhaustive()
    }
}

impl Orch90 {
    /// Build from the raw 8K ROM image (`orch90.rom`); same size validation
    /// and mirror-fill as any pak image.
    pub fn from_rom_bytes(bytes: &[u8]) -> Result<Self, ROMPakError> {
        Ok(Self {
            rom: ROMPak::from_bytes(bytes, true)?,
            left: 0,
            right: 0,
        })
    }

    /// Current left-channel latch value, for UI level meters.
    pub fn left(&self) -> u8 {
        self.left
    }

    /// Current right-channel latch value, for UI level meters.
    pub fn right(&self) -> u8 {
        self.right
    }

    /// Restore-path-only: re-inject the 8K program ROM after a snapshot
    /// restore — delegates to the inner [`ROMPak::reattach_image`]
    /// (`docs/plan-save-states.md`).
    pub fn reattach_rom(&mut self, bytes: &[u8]) -> Result<(), ROMPakError> {
        self.rom.reattach_image(bytes)
    }
}

impl Cartridge for Orch90 {
    /// The DAC latches are write-only — no read path back from a 74LS374's
    /// inputs — so every read in the I/O window floats.
    fn read(&mut self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }

    fn write(&mut self, addr: u16, val: u8) {
        match addr {
            LEFT_DAC_REG => self.left = val,
            RIGHT_DAC_REG => self.right = val,
            _ => {}
        }
    }

    fn rom_read(&mut self, addr: u16) -> u8 {
        self.rom.rom_read(addr)
    }

    fn cart_line_ties_q(&self) -> bool {
        true
    }

    /// The two DAC latches as a true stereo pair, 0.0–1.0 per channel
    /// (Tier 2: the event-timestamped pipeline carries them separately, so
    /// left/right writes hard-pan and hold exactly between writes).
    fn sound_levels(&self) -> (f32, f32) {
        const DAC_MAX: f32 = u8::MAX as f32;
        (
            f32::from(self.left) / DAC_MAX,
            f32::from(self.right) / DAC_MAX,
        )
    }
}
