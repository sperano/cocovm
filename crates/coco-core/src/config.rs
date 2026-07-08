//! Machine configuration: machine variant, video standard, and installed RAM.
//! See `DESIGN.md` §4, §3 and `docs/coco12-plan.md`.

use serde::{Deserialize, Serialize};

use crate::gime::MonitorType;

/// Which physical machine is emulated. CoCo 1 and CoCo 2 are software- and
/// timing-identical (same SAM, same plain MC6847, same PIA wiring — MAME uses
/// one `coco` driver for both, per `docs/coco12-plan.md`); the variant only
/// changes default RAM size and ROM set. The CoCo 2B's MC6847T1 (lowercase,
/// SG6 removal) is a deliberately deferred follow-up, not modeled here yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MachineVariant {
    /// SAM (MC6883) + plain MC6847 VDG, no GIME.
    Coco1,
    /// Same core as [`MachineVariant::Coco1`] — see the type doc.
    Coco2,
    /// GIME (MC6883-compatible SAM overlay + native video/MMU/timer).
    Coco3,
}

/// Master video standard — fixed by the machine's crystal, chosen at construction.
///
/// Distinct from the GIME's 50/60 Hz *mode* bit, which retimes the display *within*
/// a standard. This enum is the physical crystal. See `DESIGN.md` §4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoStandard {
    Ntsc,
    Pal,
}

impl VideoStandard {
    /// Scanlines per field. Provisional (standard video values; see `DESIGN.md` §4).
    pub const fn lines_per_field(self) -> u32 {
        match self {
            VideoStandard::Ntsc => 262,
            VideoStandard::Pal => 312,
        }
    }

    /// Nominal field (refresh) rate in Hz.
    pub const fn field_rate_hz(self) -> f64 {
        match self {
            VideoStandard::Ntsc => 59.94,
            VideoStandard::Pal => 50.0,
        }
    }

    /// Physical scanline (0-based) where the field-sync line falls: the point
    /// where PIA0 CB1 latches per its selected edge, and — on the GIME — the
    /// VBORD source is raised (Lomont: "VBORD generated on falling edge of
    /// VSYNC").
    ///
    /// NTSC, [`MachineVariant::Coco3`]: MAME `gime.cpp`'s constructor derives
    /// the falling edge as top border (25 lines) + active display (192
    /// lines) + part of the bottom border (26 lines) + 1 = 244. This is
    /// GIME-specific — the plain MC6847 (CoCo 1/2) falling edge is at line
    /// 216 instead (`mc6847.cpp`/`gime.cpp` header comment;
    /// `docs/coco12-plan.md`).
    pub const fn fs_falling_line(self, variant: MachineVariant) -> u32 {
        match self {
            VideoStandard::Ntsc => match variant {
                MachineVariant::Coco3 => 244,
                MachineVariant::Coco1 | MachineVariant::Coco2 => 216,
            },
            // UNVERIFIED: MAME's PAL timing offsets this edge by
            // `LINES_PADDING_TOP_PAL` (mc6847.cpp), which could not be pinned
            // down cleanly from the source. Keep the pre-fix behaviour (the
            // field-sync edges collapsed to the last scanline of the field)
            // rather than guess a line number. CoCo 1/2 + PAL is rejected by
            // [`MachineConfig::validate`] before this is ever consulted.
            VideoStandard::Pal => VideoStandard::Pal.lines_per_field() - 1,
        }
    }

    /// Physical scanline (0-based) where the field-sync line rises again.
    ///
    /// NTSC: MAME `mc6847.cpp` `LINES_UNTIL_RETRACE_NTSC` (243) +
    /// `LINES_VERTICAL_RETRACE` (6) - 1 = 248. Unlike
    /// [`Self::fs_falling_line`], the existing derivation of this edge already
    /// cites the plain MC6847's own timing file directly rather than a
    /// GIME-specific border-sum approximation, so — absent a source that
    /// shows the GIME diverging from the real chip on this edge the way it
    /// does on the falling one — the same value is used for every variant.
    /// The parameter exists for API symmetry with `fs_falling_line` and so a
    /// real per-variant number can be dropped in later without a signature
    /// change.
    pub const fn fs_rising_line(self, _variant: MachineVariant) -> u32 {
        match self {
            VideoStandard::Ntsc => 248,
            // UNVERIFIED, see fs_falling_line: both edges collapse to the
            // same last scanline for PAL until the real offset is confirmed.
            VideoStandard::Pal => VideoStandard::Pal.lines_per_field() - 1,
        }
    }
}

/// Installed RAM. `K128`/`K512`/`K2048` are the GIME (CoCo 3) sizes — its MMU
/// addresses up to 2 MB; 512K was only Tandy's shipped max, not a chip limit.
/// Note the write-8 / read-low-6 register asymmetry handled in the MMU model.
/// `K4`/`K16`/`K32`/`K64` are the plain-SAM (CoCo 1/2) sizes the real MC6883
/// supports. See `DESIGN.md` §3 and `docs/coco12-plan.md`.
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
    /// [`MonitorType`].
    pub monitor: MonitorType,
}

impl MachineConfig {
    /// Reject variant/video/memory combinations the emulator doesn't (or
    /// can't, on real hardware) support:
    ///
    /// - CoCo 1/2 (plain SAM) only take the 4K/16K/32K/64K sizes the real
    ///   MC6883 supports; CoCo 3 (GIME) only takes the 128K/512K/2048K sizes
    ///   its MMU addresses (`docs/coco12-plan.md`).
    /// - CoCo 1/2 are NTSC-only for now: PAL VDG timing is out of scope
    ///   (`docs/coco12-plan.md` "What's missing").
    pub fn validate(&self) -> Result<(), String> {
        match self.variant {
            MachineVariant::Coco1 | MachineVariant::Coco2 => {
                if !matches!(
                    self.memory,
                    MemorySize::K4 | MemorySize::K16 | MemorySize::K32 | MemorySize::K64
                ) {
                    return Err(format!(
                        "{:?} supports 4K/16K/32K/64K RAM, not {:?}",
                        self.variant, self.memory
                    ));
                }
                if self.video == VideoStandard::Pal {
                    return Err(format!(
                        "{:?} PAL is out of scope (plain MC6847 PAL timing not modeled)",
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
            }
        }
        Ok(())
    }
}

impl Default for MachineConfig {
    fn default() -> Self {
        Self {
            variant: MachineVariant::Coco3,
            video: VideoStandard::Ntsc,
            memory: MemorySize::K512,
            monitor: MonitorType::Rgb,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        assert!(MachineConfig::default().validate().is_ok());
    }

    #[test]
    fn coco3_rejects_coco12_memory_sizes() {
        let cfg = MachineConfig {
            variant: MachineVariant::Coco3,
            video: VideoStandard::Ntsc,
            memory: MemorySize::K64,
            monitor: MonitorType::Rgb,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn coco1_rejects_coco3_memory_sizes() {
        let cfg = MachineConfig {
            variant: MachineVariant::Coco1,
            video: VideoStandard::Ntsc,
            memory: MemorySize::K128,
            monitor: MonitorType::Rgb,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn coco1_accepts_every_plain_sam_memory_size() {
        for memory in [
            MemorySize::K4,
            MemorySize::K16,
            MemorySize::K32,
            MemorySize::K64,
        ] {
            let cfg = MachineConfig {
                variant: MachineVariant::Coco1,
                video: VideoStandard::Ntsc,
                memory,
                monitor: MonitorType::Rgb,
            };
            assert!(
                cfg.validate().is_ok(),
                "{memory:?} should be valid for Coco1"
            );
        }
    }

    #[test]
    fn coco2_rejects_pal() {
        let cfg = MachineConfig {
            variant: MachineVariant::Coco2,
            video: VideoStandard::Pal,
            memory: MemorySize::K64,
            monitor: MonitorType::Rgb,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn coco3_accepts_pal() {
        // PAL is only out of scope for the plain-SAM variants; the GIME path
        // already models a (partially unverified) PAL timing branch.
        let cfg = MachineConfig {
            variant: MachineVariant::Coco3,
            video: VideoStandard::Pal,
            memory: MemorySize::K512,
            monitor: MonitorType::Rgb,
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn fs_falling_line_differs_between_gime_and_plain_vdg() {
        assert_eq!(
            VideoStandard::Ntsc.fs_falling_line(MachineVariant::Coco3),
            244
        );
        assert_eq!(
            VideoStandard::Ntsc.fs_falling_line(MachineVariant::Coco1),
            216
        );
        assert_eq!(
            VideoStandard::Ntsc.fs_falling_line(MachineVariant::Coco2),
            216
        );
    }
}
