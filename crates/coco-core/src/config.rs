//! Machine configuration: machine variant, video standard, and installed RAM.
//! See `DESIGN.md` §3 (address decoding) and §4 (timing model).

use serde::{Deserialize, Serialize};

use crate::gime::MonitorType;

/// Which physical machine is emulated. CoCo 1 and CoCo 2 are software- and
/// timing-identical (same SAM, same plain MC6847, same PIA wiring — MAME uses
/// one `coco` driver for both); the variant only changes default RAM size
/// and ROM set. The CoCo 2B's MC6847T1 (lowercase, SG6 removal) is a
/// deliberately deferred follow-up and is not modeled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MachineVariant {
    /// SAM (MC6883) + plain MC6847 VDG, no GIME.
    Coco1,
    /// Same core as [`MachineVariant::Coco1`] — see the type doc.
    Coco2,
    /// GIME (MC6883-compatible SAM overlay + native video/MMU/timer).
    Coco3,
}

impl MachineVariant {
    /// Every variant, oldest first, in the order that the UI offers them.
    /// Keep this list beside the enum so callers do not omit later variants.
    pub const ALL: [MachineVariant; 3] = [
        MachineVariant::Coco1,
        MachineVariant::Coco2,
        MachineVariant::Coco3,
    ];
}

/// Which VDG chip is installed — only meaningfully distinct on
/// [`MachineVariant::Coco2`] (CoCo 1 always shipped the plain chip; CoCo 3
/// uses the GIME's own character generator, not a real MC6847 at all).
/// MAME machine pairing: `coco`/`coco2` drivers = plain [`VDGVariant::MC6847`];
/// `coco2b`/`deluxecoco` = [`VDGVariant::MC6847T1`] (`mc6847.cpp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VDGVariant {
    /// Original MC6847: no true lowercase, codes $00-$1F always show as
    /// inverse-video uppercase regardless of PIA1 $FF22 bit 4.
    MC6847,
    /// MC6847T1 ("MC6847T1 (CoCo 2B)"): PIA1 $FF22 bit 4 doubles as GM0 in
    /// alpha mode, enabling true lowercase glyphs for codes $00-$1F.
    MC6847T1,
}

/// Master video standard — fixed by the machine's crystal, chosen at construction.
///
/// Distinct from the GIME's 50/60 Hz *mode* bit, which retimes the display *within*
/// a standard. This enum is the physical crystal. See `DESIGN.md` §4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoStandard {
    NTSC,
    PAL,
}

impl VideoStandard {
    /// Number of scanlines per field. These standard video values are provisional.
    pub const fn lines_per_field(self) -> u32 {
        match self {
            VideoStandard::NTSC => 262,
            VideoStandard::PAL => 312,
        }
    }

    /// Nominal field (refresh) rate in Hz.
    pub const fn field_rate_hz(self) -> f64 {
        match self {
            VideoStandard::NTSC => 59.94,
            VideoStandard::PAL => 50.0,
        }
    }

    /// Physical scanline (0-based) where the field-sync line falls: PIA0
    /// CB1's latch point, and (on the GIME) where VBORD rises (MAME
    /// `gime.cpp`/`mc6847.cpp`). 244 = 25 top border + 192 active + 26 bottom + 1.
    pub const fn fs_falling_line(self, variant: MachineVariant) -> u32 {
        match self {
            VideoStandard::NTSC => match variant {
                MachineVariant::Coco3 => 244,
                MachineVariant::Coco1 | MachineVariant::Coco2 => 216,
            },
            // UNVERIFIED: PAL offset unclear from MAME source; falls back
            // to the last scanline (Coco1/2+PAL is rejected earlier by
            // validate()).
            VideoStandard::PAL => VideoStandard::PAL.lines_per_field() - 1,
        }
    }

    /// Physical scanline (0-based) where the field-sync line rises again.
    /// 248 (MAME `mc6847.cpp`) for every variant; `_variant` exists only for
    /// symmetry with [`Self::fs_falling_line`].
    pub const fn fs_rising_line(self, _variant: MachineVariant) -> u32 {
        match self {
            VideoStandard::NTSC => 248,
            // UNVERIFIED, see fs_falling_line: PAL edges collapse to the
            // same last scanline until the real offset is confirmed.
            VideoStandard::PAL => VideoStandard::PAL.lines_per_field() - 1,
        }
    }
}

/// Installed RAM. `K128`/`K512`/`K2048` are the GIME (CoCo 3) sizes — its MMU
/// addresses up to 2 MB; 512K was only Tandy's shipped max, not a chip limit.
/// Note the write-8 / read-low-6 register asymmetry handled in the MMU model.
/// `K4`/`K16`/`K32`/`K64` are the plain-SAM (CoCo 1/2) sizes the real MC6883
/// supports. See `DESIGN.md` §3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemorySize {
    /// CoCo 1/2 only.
    K4,
    /// CoCo 1/2 only.
    K16,
    /// CoCo 1/2 only.
    K32,
    /// CoCo 1/2 only.
    K64,
    /// CoCo 3 only.
    K128,
    /// CoCo 3 only.
    K512,
    /// CoCo 3 only.
    K2048,
}

/// Physical 8K block size.
pub const BLOCK_SIZE: usize = 8 * 1024;

impl MemorySize {
    pub const fn bytes(self) -> usize {
        match self {
            MemorySize::K4 => 4 * 1024,
            MemorySize::K16 => 16 * 1024,
            MemorySize::K32 => 32 * 1024,
            MemorySize::K64 => 64 * 1024,
            MemorySize::K128 => 128 * 1024,
            MemorySize::K512 => 512 * 1024,
            MemorySize::K2048 => 2048 * 1024,
        }
    }

    /// Number of 8K physical blocks (used by the GIME MMU model; the plain-SAM
    /// sizes below 64K aren't a whole number of 8K blocks and don't use this).
    pub const fn blocks(self) -> usize {
        self.bytes() / BLOCK_SIZE
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MachineConfig {
    pub variant: MachineVariant,
    pub video: VideoStandard,
    pub memory: MemorySize,
    /// Which monitor cable is plugged in (RGB vs composite decode of the
    /// GIME's 6-bit palette values). Not a hardware register — see
    /// [`MonitorType`]. `None` on machines with no monitor port at all:
    /// a stock CoCo 1/2's only video output is the RF modulator into a TV
    /// (RGB and composite ports are CoCo 3 additions), enforced by
    /// [`Self::validate`].
    pub monitor: Option<MonitorType>,
    /// Which VDG chip is installed. See [`VDGVariant`]. `None` on the
    /// CoCo 3, which has no MC6847 at all — the GIME does its own character
    /// generation. Enforced by [`Self::validate`].
    pub vdg: Option<VDGVariant>,
}

impl MachineConfig {
    /// Reject variant/video/memory/VDG/monitor combinations the emulator
    /// doesn't support, or that real hardware never shipped, such as a 4K or
    /// 32K CoCo 2 — CoCo 2 service manual 26-3026/26-3027 §3.3); see the
    /// error messages for specifics.
    pub fn validate(&self) -> Result<(), String> {
        match self.variant {
            MachineVariant::Coco1 | MachineVariant::Coco2 => {
                let shipped = match self.variant {
                    MachineVariant::Coco1 => matches!(
                        self.memory,
                        MemorySize::K4 | MemorySize::K16 | MemorySize::K32 | MemorySize::K64
                    ),
                    _ => matches!(self.memory, MemorySize::K16 | MemorySize::K64),
                };
                if !shipped {
                    let sizes = match self.variant {
                        MachineVariant::Coco1 => "4K/16K/32K/64K",
                        _ => "16K/64K",
                    };
                    return Err(format!(
                        "{:?} shipped with {sizes} RAM, not {:?}",
                        self.variant, self.memory
                    ));
                }
                if self.video == VideoStandard::PAL {
                    return Err(format!(
                        "{:?} PAL is out of scope (plain MC6847 PAL timing not modeled)",
                        self.variant
                    ));
                }
                if self.monitor.is_some() {
                    return Err(format!(
                        "{:?} has no GIME signal path to select (VDG composite/RF only); monitor must be None",
                        self.variant
                    ));
                }
            }
            MachineVariant::Coco3 => {
                if !matches!(
                    self.memory,
                    MemorySize::K128 | MemorySize::K512 | MemorySize::K2048
                ) {
                    return Err(format!(
                        "Coco3 supports 128K/512K/2048K RAM, not {:?}",
                        self.memory
                    ));
                }
                if self.monitor.is_none() {
                    return Err("Coco3 needs a monitor type (RGB or composite cable)".to_string());
                }
            }
        }
        match (self.variant, self.vdg) {
            (MachineVariant::Coco3, Some(_)) => {
                return Err(
                    "Coco3 has no VDG (the GIME does its own character generation); \
                     vdg must be None"
                        .to_string(),
                );
            }
            (MachineVariant::Coco1 | MachineVariant::Coco2, None) => {
                return Err(format!(
                    "{:?} needs a VDG chip (vdg must be set)",
                    self.variant
                ));
            }
            (MachineVariant::Coco1, Some(VDGVariant::MC6847T1)) => {
                return Err(
                    "Coco1 does not support VDGVariant::MC6847T1 (only Coco2 had a T1 board)"
                        .to_string(),
                );
            }
            _ => {}
        }
        Ok(())
    }
}

impl Default for MachineConfig {
    fn default() -> Self {
        Self {
            variant: MachineVariant::Coco3,
            video: VideoStandard::NTSC,
            memory: MemorySize::K512,
            monitor: Some(MonitorType::RGB),
            vdg: None,
        }
    }
}

#[cfg(test)]
#[path = "config_test.rs"]
mod tests;
