//! John Linville's Games Master Cartridge: a bank-switched ROM pak plus a
//! TI SN76489A PSG for game music.

use serde::{Deserialize, Serialize};

use super::rompak::{BankedPakError, BankedROMPak};
use super::{Cartridge, IO_OPEN_BUS};

/// The SN76489A data port: `$FF41` (MAME `coco_gmc.cpp` `scs_write` case 1).
///
/// ⚠ This address collides with the DriveWire Becker port's data register.
/// MAME resolves it by intercepting Becker *ahead* of the cartridge decode,
/// shadowing the GMC's PSG; when the Becker port lands here
///, the bus must keep that precedence and
/// the UI must refuse to enable both at once.
const GMC_PSG_REG: u16 = 0xFF41;

/// The PSG's crystal on the GMC: 4 MHz (MAME `coco_gmc.cpp`
/// `SN76489A(config, m_psg, 4_MHz_XTAL)`).
const GMC_PSG_CRYSTAL_HZ: f64 = 4_000_000.0;

/// John Linville's Games Master Cartridge (MAME `coco_gmc.cpp`): a
/// [`BankedROMPak`] plus a TI SN76489A PSG for game music. `$FF40` is the
/// ROM bank latch (inherited from the banked pak), `$FF41` writes the PSG's
/// single command port; nothing is readable back (MAME's `scs_read` is the
/// do-nothing base), so reads stay at the I/O window's open-bus value.
///
/// Audio mixes into the speaker unconditionally, not through the analog
/// mux's SEL=10 cartridge-sound input: MAME routes the GMC's PSG to a
/// dedicated speaker device ignoring SNDEN entirely (its mux cart-sound path
/// is an explicit "NYI" stub), and no independent schematic settles what the
/// real cart's SND-pin wiring expects — so we keep MAME's behaviour.
#[derive(Serialize, Deserialize)]
pub struct GamesMasterCartridge {
    rom: BankedROMPak,
    psg: crate::sn76489::SN76489A,
}

impl std::fmt::Debug for GamesMasterCartridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GamesMasterCartridge")
            .field("rom", &self.rom)
            .field("psg", &self.psg)
            .finish()
    }
}

impl GamesMasterCartridge {
    /// Build a GMC from a raw banked-ROM image (same size rules as
    /// [`BankedROMPak::from_bytes`]).
    pub fn from_bytes(bytes: &[u8], autostart: bool) -> Result<Self, BankedPakError> {
        Ok(Self {
            rom: BankedROMPak::from_bytes(bytes, autostart)?,
            psg: crate::sn76489::SN76489A::new(GMC_PSG_CRYSTAL_HZ),
        })
    }

    /// Restore-path-only: re-inject the banked ROM image after a snapshot
    /// restore — delegates to the inner [`BankedROMPak::reattach_image`]
    ///.
    pub fn reattach_rom(&mut self, bytes: &[u8]) -> Result<(), BankedPakError> {
        self.rom.reattach_image(bytes)
    }
}

impl Cartridge for GamesMasterCartridge {
    fn read(&mut self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    fn write(&mut self, addr: u16, val: u8) {
        match addr {
            GMC_PSG_REG => self.psg.write(val),
            _ => self.rom.write(addr, val),
        }
    }
    fn rom_read(&mut self, addr: u16) -> u8 {
        self.rom.rom_read(addr)
    }
    fn cart_line_ties_q(&self) -> bool {
        self.rom.cart_line_ties_q()
    }
    fn generator_sample(&mut self, dt: f64) -> (f32, f32) {
        let level = self.psg.sample(dt);
        (level, level)
    }
    /// Only the bank latch resets — the SN76489A has no reset pin, so the
    /// PSG plays on through a warm reset until software reprograms it, as on
    /// the real cartridge.
    fn reset(&mut self) {
        self.rom.reset();
    }
    /// Rebuilds `psg.vol_table` — pure construction-time scratch, skipped
    /// from the snapshot (`crate::sn76489`'s `after_restore` fixup).
    fn after_restore(&mut self) {
        self.psg.after_restore();
    }
}
