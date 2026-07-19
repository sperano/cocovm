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
    /// Scanlines per field. Provisional (standard video values; see `DESIGN.md` §4).
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
            VideoStandard::NTSC => match variant {
                MachineVariant::Coco3 => 244,
                MachineVariant::Coco1 | MachineVariant::Coco2 => 216,
            },
            // UNVERIFIED: MAME's PAL timing offsets this edge by
            // `LINES_PADDING_TOP_PAL` (mc6847.cpp), which could not be pinned
            // down cleanly from the source. Keep the pre-fix behaviour (the
            // field-sync edges collapsed to the last scanline of the field)
            // rather than guess a line number. CoCo 1/2 + PAL is rejected by
            // [`MachineConfig::validate`] before this is ever consulted.
            VideoStandard::PAL => VideoStandard::PAL.lines_per_field() - 1,
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
            VideoStandard::NTSC => 248,
            // UNVERIFIED, see fs_falling_line: both edges collapse to the
            // same last scanline for PAL until the real offset is confirmed.
            VideoStandard::PAL => VideoStandard::PAL.lines_per_field() - 1,
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
    /// Reject variant/video/memory combinations the emulator doesn't (or
    /// can't, on real hardware) support:
    ///
    /// - RAM is limited to the configurations each machine actually shipped
    ///   in: 4K/16K/32K/64K for the CoCo 1, 16K/64K for the CoCo 2 (base
    ///   16K×1 DRAMs plus the factory 64K upgrade — CoCo 2 service manual
    ///   26-3026/26-3027 §3.3; no 4K or 32K CoCo 2 ever shipped), and the
    ///   128K/512K/2048K sizes the CoCo 3's GIME MMU addresses
    ///   (`docs/coco12-plan.md`).
    /// - CoCo 1/2 are NTSC-only for now: PAL VDG timing is out of scope
    ///   (`docs/coco12-plan.md` "What's missing").
    /// - [`VDGVariant::MC6847T1`] is only valid on [`MachineVariant::Coco2`]:
    ///   CoCo 1 never had a T1 board, and CoCo 3 has no real MC6847 at all
    ///   (the GIME does its own text character generation).
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
                        "{:?} has no monitor port (RF TV output only); monitor must be None",
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
                    return Err(
                        "Coco3 needs a monitor type (RGB or composite cable)".to_string()
                    );
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
                return Err(format!("{:?} needs a VDG chip (vdg must be set)", self.variant));
            }
            (MachineVariant::Coco1, Some(VDGVariant::MC6847T1)) => {
                return Err(
                    "Coco1 does not support VdgVariant::Mc6847T1 (only Coco2 had a T1 board)"
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
            video: VideoStandard::NTSC,
            memory: MemorySize::K64,
            monitor: Some(MonitorType::RGB),
            vdg: None,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn coco1_rejects_coco3_memory_sizes() {
        let cfg = MachineConfig {
            variant: MachineVariant::Coco1,
            video: VideoStandard::NTSC,
            memory: MemorySize::K128,
            monitor: None,
            vdg: Some(VDGVariant::MC6847),
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
                video: VideoStandard::NTSC,
                memory,
                monitor: None,
                vdg: Some(VDGVariant::MC6847),
            };
            assert!(
                cfg.validate().is_ok(),
                "{memory:?} should be valid for Coco1"
            );
        }
    }

    #[test]
    fn coco2_only_accepts_shipped_memory_sizes() {
        for (memory, ok) in [
            (MemorySize::K4, false),
            (MemorySize::K16, true),
            (MemorySize::K32, false),
            (MemorySize::K64, true),
        ] {
            let cfg = MachineConfig {
                variant: MachineVariant::Coco2,
                video: VideoStandard::NTSC,
                memory,
                monitor: None,
                vdg: Some(VDGVariant::MC6847),
            };
            assert_eq!(
                cfg.validate().is_ok(),
                ok,
                "{memory:?} for Coco2: expected valid={ok}"
            );
        }
    }

    #[test]
    fn coco2_rejects_pal() {
        let cfg = MachineConfig {
            variant: MachineVariant::Coco2,
            video: VideoStandard::PAL,
            memory: MemorySize::K64,
            monitor: None,
            vdg: Some(VDGVariant::MC6847),
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn coco3_accepts_pal() {
        // PAL is only out of scope for the plain-SAM variants; the GIME path
        // already models a (partially unverified) PAL timing branch.
        let cfg = MachineConfig {
            variant: MachineVariant::Coco3,
            video: VideoStandard::PAL,
            memory: MemorySize::K512,
            monitor: Some(MonitorType::RGB),
            vdg: None,
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn coco2_accepts_mc6847t1() {
        let cfg = MachineConfig {
            variant: MachineVariant::Coco2,
            video: VideoStandard::NTSC,
            memory: MemorySize::K64,
            monitor: None,
            vdg: Some(VDGVariant::MC6847T1),
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn coco2_accepts_plain_mc6847() {
        let cfg = MachineConfig {
            variant: MachineVariant::Coco2,
            video: VideoStandard::NTSC,
            memory: MemorySize::K64,
            monitor: None,
            vdg: Some(VDGVariant::MC6847),
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn coco1_rejects_mc6847t1() {
        let cfg = MachineConfig {
            variant: MachineVariant::Coco1,
            video: VideoStandard::NTSC,
            memory: MemorySize::K64,
            monitor: None,
            vdg: Some(VDGVariant::MC6847T1),
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn coco3_rejects_any_vdg() {
        for vdg in [VDGVariant::MC6847, VDGVariant::MC6847T1] {
            let cfg = MachineConfig {
                variant: MachineVariant::Coco3,
                video: VideoStandard::NTSC,
                memory: MemorySize::K512,
                monitor: Some(MonitorType::RGB),
                vdg: Some(vdg),
            };
            assert!(cfg.validate().is_err(), "Coco3 has no VDG, {vdg:?} must be rejected");
        }
    }

    #[test]
    fn coco12_rejects_monitor_and_coco3_requires_one() {
        let mut cfg = MachineConfig {
            variant: MachineVariant::Coco2,
            video: VideoStandard::NTSC,
            memory: MemorySize::K64,
            monitor: Some(MonitorType::RGB),
            vdg: Some(VDGVariant::MC6847),
        };
        assert!(cfg.validate().is_err(), "Coco2 has no monitor port");
        cfg.monitor = None;
        assert!(cfg.validate().is_ok());

        let mut cfg = MachineConfig::default();
        assert!(cfg.validate().is_ok());
        cfg.monitor = None;
        assert!(cfg.validate().is_err(), "Coco3 needs a cable choice");
    }

    #[test]
    fn fs_falling_line_differs_between_gime_and_plain_vdg() {
        assert_eq!(
            VideoStandard::NTSC.fs_falling_line(MachineVariant::Coco3),
            244
        );
        assert_eq!(
            VideoStandard::NTSC.fs_falling_line(MachineVariant::Coco1),
            216
        );
        assert_eq!(
            VideoStandard::NTSC.fs_falling_line(MachineVariant::Coco2),
            216
        );
    }
}
