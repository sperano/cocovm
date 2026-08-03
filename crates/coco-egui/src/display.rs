//! What the machine's video output is plugged into — a monitor (CoCo 3
//! only) or a TV, color or black & white — and the frontend post-pass that
//! look implies.
//!
//! A real CoCo 3 drives its RGB, composite, and RF outputs simultaneously;
//! the TV hangs off the RF modulator, which is fed the composite signal, so
//! a TV — color or B&W — always sees the composite palette, never the RGB
//! unpack. A CoCo 1/2 has no monitor port at all: RF to a TV is its only
//! output. Making the display device a sum type over both facts means the
//! one illegal combination (RGB signal into a B&W TV) simply cannot be
//! expressed — no invariant to enforce anywhere.
//!
//! [`Display::TV`] and [`Display::Monitor`]`(Composite)` render identically
//! today, but they are distinct states everywhere (enum, `[hardware].display`
//! TOML value, View-menu entry) because they diverge later: [`apply`] — the
//! TV chain, which only acts on `TV(_)` — is where RF-degradation/vintage
//! effects will accumulate, while a composite monitor stays clean.

use coco_core::{MachineConfig, MachineVariant, MonitorType};
use eframe::egui;

/// A television's rendition of the RF signal: as-is, or collapsed to the
/// luma a black & white set recovers ([`luma`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TV {
    Color,
    BW,
}

/// The display device on the other end of the video cable. A UI-level
/// preference (like `CocoApp::aspect_correct`): the core's own
/// [`MonitorType`] stays a plain RGB/composite signal-path choice, and save
/// states never carry this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Display {
    /// A monitor on the CoCo 3's RGB or composite port. Invalid on a CoCo
    /// 1/2 (no monitor port) — [`Self::to_monitor`] passes the choice
    /// through so `MachineConfig::validate` rejects it with the real reason.
    Monitor(MonitorType),
    /// A TV on the RF output — every machine has one of those.
    TV(TV),
}

/// Every choice, in the View menu's / detail form's display order.
const ALL: [Display; 4] = [
    Display::Monitor(MonitorType::RGB),
    Display::Monitor(MonitorType::Composite),
    Display::TV(TV::Color),
    Display::TV(TV::BW),
];

/// The TV-only tail of [`ALL`], for machines without a monitor port.
const TV_ONLY: [Display; 2] = [Display::TV(TV::Color), Display::TV(TV::BW)];

impl Display {
    /// The choices `variant` can actually drive: all four on a CoCo 3, just
    /// the two TVs on a CoCo 1/2.
    pub(crate) const fn choices(variant: MachineVariant) -> &'static [Display] {
        match variant {
            MachineVariant::Coco3 => &ALL,
            MachineVariant::Coco1 | MachineVariant::Coco2 => &TV_ONLY,
        }
    }

    /// Per-variant default when nothing is chosen: the RGB monitor a CoCo 3
    /// config already defaulted to before this type existed, the color TV
    /// that was a CoCo 1/2's only possible display.
    pub(crate) const fn default_for(variant: MachineVariant) -> Self {
        match variant {
            MachineVariant::Coco3 => Display::Monitor(MonitorType::RGB),
            MachineVariant::Coco1 | MachineVariant::Coco2 => Display::TV(TV::Color),
        }
    }

    /// The core-side signal path this display decodes —
    /// `MachineConfig::monitor`'s value. A TV is downstream of the composite
    /// signal on a CoCo 3 and of the bare RF output (`None`) on a CoCo 1/2;
    /// a monitor choice passes through regardless of variant so
    /// `MachineConfig::validate` can reject it where no port exists.
    pub(crate) const fn to_monitor(self, variant: MachineVariant) -> Option<MonitorType> {
        match self {
            Display::Monitor(monitor) => Some(monitor),
            Display::TV(_) => match variant {
                MachineVariant::Coco3 => Some(MonitorType::Composite),
                MachineVariant::Coco1 | MachineVariant::Coco2 => None,
            },
        }
    }

    /// The display a validated config implies, for paths that predate this
    /// type (a save state's config, `MachineConfig::default()`). Lossy in
    /// one direction only: a CoCo 3 TV serializes as composite, so callers
    /// holding the real choice (a definition's `[hardware].display`, the
    /// CLI's `--display`) must overwrite this afterwards.
    pub(crate) const fn from_config(config: &MachineConfig) -> Self {
        match config.monitor {
            Some(monitor) => Display::Monitor(monitor),
            None => Display::TV(TV::Color),
        }
    }

    /// Menu / form label.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Display::Monitor(MonitorType::RGB) => "RGB monitor",
            Display::Monitor(MonitorType::Composite) => "Composite monitor",
            Display::TV(TV::Color) => "Color TV",
            Display::TV(TV::BW) => "B&W TV",
        }
    }
}

/// Rec.601 weights, the standard-definition coefficients NTSC weighs the
/// three channels with — far from a flat average: green carries most of
/// the perceived brightness, blue almost none.
const LUMA_R: f32 = 0.299;
const LUMA_G: f32 = 0.587;
const LUMA_B: f32 = 0.114;

/// Display transfer-function exponent for the linear-light round trip
/// below (the conventional CRT approximation).
const GAMMA: f32 = 2.2;

/// Resolution of [`LumaTables::encode`]. High enough that its linear-light
/// quantization never moves the 8-bit result by more than one level even at
/// the dark end, where the encode curve is steepest.
const ENCODE_STEPS: usize = 1 << 16;

/// Lookup tables for [`luma`], built once on first use: per-channel
/// pre-weighted linearization (`w · (v/255)^γ`) and the `y^(1/γ)` re-encode,
/// so the per-pixel cost is three lookups, two adds, and one more lookup —
/// no `powf` on the frame path.
struct LumaTables {
    r: [f32; 256],
    g: [f32; 256],
    b: [f32; 256],
    encode: Box<[u8; ENCODE_STEPS]>,
}

static LUMA_TABLES: std::sync::LazyLock<LumaTables> = std::sync::LazyLock::new(|| {
    let channel = |weight: f32| std::array::from_fn(|v| weight * (v as f32 / 255.0).powf(GAMMA));
    let mut encode = Box::new([0u8; ENCODE_STEPS]);
    for (i, out) in encode.iter_mut().enumerate() {
        let y = i as f32 / (ENCODE_STEPS - 1) as f32;
        *out = (y.powf(1.0 / GAMMA) * 255.0).round() as u8;
    }
    LumaTables {
        r: channel(LUMA_R),
        g: channel(LUMA_G),
        b: channel(LUMA_B),
        encode,
    }
});

/// One pixel's grey: Rec.601-weighted luminance computed in **linear
/// light**, then re-encoded. Weighting the gamma-encoded bytes directly
/// (analog luma, what the composite Y signal literally carries) reads
/// noticeably too dark on a modern display — saturated green lands at 149
/// instead of the ~200 the eye expects (user feedback 2026-08-02).
fn luma(r: u8, g: u8, b: u8) -> u8 {
    let t = &*LUMA_TABLES;
    let y = t.r[r as usize] + t.g[g as usize] + t.b[b as usize];
    let i = (y * (ENCODE_STEPS - 1) as f32).round() as usize;
    t.encode[i.min(ENCODE_STEPS - 1)]
}

/// How the display's texture scales to the window: a monitor keeps the
/// crisp integer-pixel look (`NEAREST`); a CRT TV has no sharp pixel edges
/// at all, so TVs sample bilinearly — the cheapest single step of the TV
/// look, done by the GPU during normal drawing rather than in [`apply`].
pub(crate) fn texture_options(display: Display) -> egui::TextureOptions {
    match display {
        Display::Monitor(_) => egui::TextureOptions::NEAREST,
        Display::TV(_) => egui::TextureOptions::LINEAR,
    }
}

/// RGBA8 bytes per pixel, the framebuffer's own layout
/// (`coco_core::video::BYTES_PER_PIXEL`).
const PX: usize = 4;

/// [`blur_rows`]' symmetric FIR taps (normalized by [`BLUR_SHIFT`]): the
/// mild composite/RF softness of a ~4 MHz NTSC luma channel at these dot
/// rates, not a heavy defocus.
const BLUR_TAPS: [u16; 3] = [1, 2, 1];
const BLUR_SHIFT: u16 = 2;

/// The TV chain, applied in place to an RGBA8 frame about to be shown —
/// the texture upload (`CocoApp::upload_framebuffer_texture`) and the
/// thumbnail-PNG capture (`manager::thumbnails`). `width` is the frame's
/// width in pixels (rows are `width · 4` bytes). Monitors pass through
/// untouched; both TVs get the composite/RF horizontal bandwidth limit
/// ([`blur_rows`]), and the B&W set collapses to luma first (blurring a
/// grey keeps it grey, so the order only matters for color).
pub(crate) fn apply(display: Display, width: usize, bytes: &mut [u8]) {
    let Display::TV(tv) = display else {
        return;
    };
    if tv == TV::BW {
        for px in bytes.chunks_exact_mut(PX) {
            let y = luma(px[0], px[1], px[2]);
            px[0] = y;
            px[1] = y;
            px[2] = y;
        }
    }
    blur_rows(width, bytes);
}

/// The horizontal bandwidth limit: a [`BLUR_TAPS`] FIR across each row —
/// horizontal only, because that's what an analog TV signal is: each
/// scanline is a band-limited waveform, so detail smears along the line
/// while rows stay perfectly separate. Edges clamp (the border color
/// extends past the frame). Alpha is untouched.
fn blur_rows(width: usize, bytes: &mut [u8]) {
    let row_len = width * PX;
    let mut scratch = vec![0u8; row_len];
    for row in bytes.chunks_exact_mut(row_len) {
        scratch.copy_from_slice(row);
        for x in 0..width {
            let prev = &scratch[x.saturating_sub(1) * PX..];
            let cur = &scratch[x * PX..];
            let next = &scratch[(x + 1).min(width - 1) * PX..];
            for c in 0..3 {
                let sum = prev[c] as u16 * BLUR_TAPS[0]
                    + cur[c] as u16 * BLUR_TAPS[1]
                    + next[c] as u16 * BLUR_TAPS[2];
                // +half for round-to-nearest rather than truncation.
                row[x * PX + c] = ((sum + (1 << (BLUR_SHIFT - 1))) >> BLUR_SHIFT) as u8;
            }
        }
    }
}

#[cfg(test)]
#[path = "display_test.rs"]
mod tests;
