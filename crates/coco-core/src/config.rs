//! Machine configuration: video standard and installed RAM. See `DESIGN.md` §4, §3.

use serde::{Deserialize, Serialize};

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

    /// Physical scanline (0-based) where the GIME's internal field-sync line
    /// falls: the point where the GIME raises VBORD (Lomont: "VBORD generated
    /// on falling edge of VSYNC") and PIA0 CB1 latches per its selected edge.
    ///
    /// NTSC: MAME `gime.cpp`'s constructor derives the falling edge as top
    /// border (25 lines) + active display (192 lines) + part of the bottom
    /// border (26 lines) + 1 = 244. This is GIME-specific — the plain
    /// MC6847 (CoCo 1/2) falling edge is at line 216 instead
    /// (`mc6847.cpp`/`gime.cpp` header comment).
    pub const fn fs_falling_line(self) -> u32 {
        match self {
            VideoStandard::Ntsc => 244,
            // UNVERIFIED: MAME's PAL timing offsets this edge by
            // `LINES_PADDING_TOP_PAL` (mc6847.cpp), which could not be pinned
            // down cleanly from the source. Keep the pre-fix behaviour (the
            // field-sync edges collapsed to the last scanline of the field)
            // rather than guess a line number.
            VideoStandard::Pal => VideoStandard::Pal.lines_per_field() - 1,
        }
    }

    /// Physical scanline (0-based) where the field-sync line rises again.
    ///
    /// NTSC: MAME `mc6847.cpp` `LINES_UNTIL_RETRACE_NTSC` (243) +
    /// `LINES_VERTICAL_RETRACE` (6) - 1 = 248.
    pub const fn fs_rising_line(self) -> u32 {
        match self {
            VideoStandard::Ntsc => 248,
            // UNVERIFIED, see fs_falling_line: both edges collapse to the
            // same last scanline for PAL until the real offset is confirmed.
            VideoStandard::Pal => VideoStandard::Pal.lines_per_field() - 1,
        }
    }
}

/// Installed RAM. The GIME MMU addresses up to 2 MB; 512K was only Tandy's shipped
/// max, not a chip limit. Note the write-8 / read-low-6 register asymmetry handled
/// in the MMU model. See `DESIGN.md` §3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemorySize {
    K128,
    K512,
    K2048,
}

/// Physical 8K block size.
pub const BLOCK_SIZE: usize = 8 * 1024;

impl MemorySize {
    pub const fn bytes(self) -> usize {
        match self {
            MemorySize::K128 => 128 * 1024,
            MemorySize::K512 => 512 * 1024,
            MemorySize::K2048 => 2048 * 1024,
        }
    }

    /// Number of 8K physical blocks (16 / 64 / 256).
    pub const fn blocks(self) -> usize {
        self.bytes() / BLOCK_SIZE
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MachineConfig {
    pub video: VideoStandard,
    pub memory: MemorySize,
}

impl Default for MachineConfig {
    fn default() -> Self {
        Self {
            video: VideoStandard::Ntsc,
            memory: MemorySize::K512,
        }
    }
}
