//! Monitor-signal-path colour resolution: RGB output is a straight bit
//! unpack, composite output goes through hand-measured lookup tables.

use serde::{Deserialize, Serialize};

use super::{GIME, vmode};

/// Which monitor signal path resolves 6-bit palette values to RGB: the real
/// GIME drives both an RGB and a composite output simultaneously, and it's
/// the monitor cable — not a GIME register — that decides which one matters.
/// Emulator config/UI choice; default matches prior (RGB-only) behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MonitorType {
    #[default]
    RGB,
    Composite,
}

/// Composite-monitor palette (BPI=0), 64 entries indexed by the 6-bit GIME
/// palette value, `0xRRGGBB`. Hand-measured on real hardware — there is no
/// formula. Verbatim from MAME `src/mame/trs/gime.cpp` `get_composite_color`
/// (BSD-3-Clause, Nathan Woods; see NOTICE.md).
const COMPOSITE_PALETTE: [u32; 64] = [
    0x000000, 0x004c00, 0x004300, 0x0a3100, 0x2f1b00, 0x550100, 0x6c0000, 0x770006, 0x71004b,
    0x5c008b, 0x3b00b8, 0x1100ca, 0x001499, 0x002c62, 0x004011, 0x004b00, 0x2d2d2d, 0x069800,
    0x288f00, 0x537d00, 0x786700, 0xa04c00, 0xb63402, 0xc3224c, 0xbd1693, 0xa814d5, 0x881cfe,
    0x5e2cff, 0x105ee9, 0x0076b2, 0x008b60, 0x009618, 0x747474, 0x41d714, 0x62cf00, 0x8ebd00,
    0xb4a700, 0xdd8c01, 0xf5733a, 0xfe6085, 0xfd53ce, 0xe950ff, 0xc958ff, 0x9e67ff, 0x4e9aff,
    0x36b3f7, 0x26c9a3, 0x2bd558, 0xfdfdfe, 0x88e85a, 0xa1e03f, 0xbed238, 0xd8c342, 0xf1b161,
    0xfea08d, 0xfe95bf, 0xfd8ef1, 0xef8eff, 0xd895ff, 0xb9a1ff, 0x86c4ff, 0x78d4f2, 0x71e2b6,
    0xffffff,
];
/// Composite-monitor palette with $FF98 BPI (burst phase invert) set.
/// Verbatim from MAME `gime.cpp` `get_composite_color`.
const COMPOSITE_PALETTE_180: [u32; 64] = [
    0x000000, 0x5a0e5a, 0x4f0c4f, 0x360f40, 0x0d213c, 0x003334, 0x004141, 0x004943, 0x005409,
    0x005600, 0x114c00, 0x263700, 0x392500, 0x491d00, 0x4f0f3e, 0x590e59, 0x2d2d2d, 0xb11fb7,
    0x9932c1, 0x7248c5, 0x4a5bc2, 0x1a6eba, 0x0077a9, 0x008c62, 0x009619, 0x039700, 0x238f00,
    0x467800, 0x9c4e00, 0xb23c00, 0xb92e59, 0xb6209e, 0x747474, 0xe852ff, 0xcd60ff, 0xa677ff,
    0x7d8aff, 0x4d9eff, 0x32b4ed, 0x29c7a2, 0x2ad459, 0x39d223, 0x50c11a, 0x72a911, 0xcf831e,
    0xf47733, 0xff5f85, 0xfe54d1, 0xfdfdfc, 0xef8fff, 0xd697ff, 0xb8a4ff, 0x9eb3ff, 0x86c6ff,
    0x76d4e7, 0x74ddb3, 0x77e683, 0x80e170, 0x92d56b, 0xacc466, 0xeaac71, 0xffa385, 0xff95c1,
    0xffffff,
];

/// Unpack an `0xRRGGBB` composite-table entry into RGBA, matching
/// [`GIME::rgb_color`]'s return convention (opaque alpha).
fn unpack_rgb(v: u32) -> [u8; 4] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8, 0xFF]
}

impl GIME {
    /// Convert a 6-bit GIME palette value to RGBA. The register format is
    /// `RGBrgb` (two bits per channel); each channel scales `0..3` to `0..0xFF`
    /// via `×0x55` — matching the GIME's RGB output (MAME `gime.cpp`).
    pub fn rgb_color(value: u8) -> [u8; 4] {
        let chan = |hi_bit: u8, lo_bit: u8| {
            let v = ((value >> hi_bit) & 1) << 1 | ((value >> lo_bit) & 1);
            v * 0x55
        };
        [chan(5, 2), chan(4, 1), chan(3, 0), 0xFF]
    }

    /// Resolve a 6-bit GIME palette value to RGBA through the currently
    /// selected monitor path (`self.monitor`). RGB mode is [`Self::rgb_color`]
    /// unconditionally; composite mode picks [`COMPOSITE_PALETTE`] or
    /// [`COMPOSITE_PALETTE_180`] per $FF98 BPI, then averages channels to
    /// grey when $FF98 MOCH is set (MAME `gime.cpp` `update_composite`).
    pub fn color(&self, value: u8) -> [u8; 4] {
        match self.monitor {
            MonitorType::RGB => Self::rgb_color(value),
            MonitorType::Composite => {
                let table = if self.vmode & vmode::BPI != 0 {
                    &COMPOSITE_PALETTE_180
                } else {
                    &COMPOSITE_PALETTE
                };
                let [r, g, b, a] = unpack_rgb(table[value as usize & 0x3F]);
                if self.vmode & vmode::MOCH != 0 {
                    let avg = ((r as u16 + g as u16 + b as u16) / 3) as u8;
                    [avg, avg, avg, a]
                } else {
                    [r, g, b, a]
                }
            }
        }
    }
}
