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
//! A composite monitor and a TV share a signal path but not a look:
//! [`process`] — the TV chain, which only acts on `TV(_)` — is where the
//! CRT/RF treatment (B&W luma collapse, bandwidth limit, scanlines, and
//! future vintage effects) accumulates, while a monitor stays clean.

use std::borrow::Cow;
use std::sync::LazyLock;

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

/// Every choice, in the display menu's / detail form's display order.
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

    /// Per-variant default when nothing is chosen: RGB monitor on CoCo 3,
    /// color TV on CoCo 1/2.
    pub(crate) const fn default_for(variant: MachineVariant) -> Self {
        match variant {
            MachineVariant::Coco3 => Display::Monitor(MonitorType::RGB),
            MachineVariant::Coco1 | MachineVariant::Coco2 => Display::TV(TV::Color),
        }
    }

    /// The core-side signal (`MachineConfig::monitor`) this display implies:
    /// composite for a CoCo 3 TV, `None` for a CoCo 1/2 TV; a monitor choice
    /// passes through so `validate` can reject it where no port exists.
    pub(crate) const fn to_monitor(self, variant: MachineVariant) -> Option<MonitorType> {
        match self {
            Display::Monitor(monitor) => Some(monitor),
            Display::TV(_) => match variant {
                MachineVariant::Coco3 => Some(MonitorType::Composite),
                MachineVariant::Coco1 | MachineVariant::Coco2 => None,
            },
        }
    }

    /// The display a validated config implies. Lossy one way: a CoCo 3 TV
    /// serializes as composite, so callers with the real choice must
    /// overwrite this afterwards.
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

    /// Status-bar label: just the signal/set — the icon beside it already
    /// distinguishes monitor vs. TV.
    pub(crate) const fn short_label(self) -> &'static str {
        match self {
            Display::Monitor(MonitorType::RGB) => "RGB",
            Display::Monitor(MonitorType::Composite) => "Composite",
            Display::TV(TV::Color) => "TV",
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

static LUMA_TABLES: LazyLock<LumaTables> = LazyLock::new(|| {
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

/// One pixel's grey: Rec.601-weighted luminance computed in linear light,
/// then re-encoded — weighting the gamma-encoded bytes directly reads
/// noticeably too dark on a modern display.
fn luma(r: u8, g: u8, b: u8) -> u8 {
    let t = &LUMA_TABLES;
    let y = t.r[r as usize] + t.g[g as usize] + t.b[b as usize];
    let i = (y * (ENCODE_STEPS - 1) as f32).round() as usize;
    t.encode[i.min(ENCODE_STEPS - 1)]
}

/// How the display's texture scales to the window: monitor stays crisp
/// (`NEAREST`); TV samples bilinearly to mimic a CRT's soft pixel edges.
pub(crate) fn texture_options(display: Display) -> egui::TextureOptions {
    match display {
        Display::Monitor(_) => egui::TextureOptions::NEAREST,
        Display::TV(_) => egui::TextureOptions::LINEAR,
    }
}

/// RGBA8 bytes per pixel, the framebuffer's own layout
/// (`coco_core::video::BYTES_PER_PIXEL`).
const PX: usize = 4;

/// Upper bound of every [`TVSettings`] percentage knob — slider ranges and
/// the clamp loaded values pass through ([`TVSettings::clamped`]).
pub(crate) const MAX_PCT: u8 = 100;

/// Default [`TVSettings::scanline_pct`]: the strength the look was tuned
/// at (the dark half keeps 65% of the line's linear-light brightness).
const DEFAULT_SCANLINE_PCT: u8 = 35;

/// Default [`TVSettings::noise_pct`]: a whisper of snow — present enough
/// to feel like an antenna feed, not enough to obscure anything.
const DEFAULT_NOISE_PCT: u8 = 5;

/// User-adjustable knobs of the TV chain — a UI preference riding along
/// with [`Display`] (display-menu sliders live, `[ui]` keys persisted).
/// Integer percentages, not floats: sliders and TOML both stay clean
/// (`tv_scanline = 35`, no float dust).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TVSettings {
    /// Scanline strength, `0..=100`: how much of a line's linear-light
    /// brightness the dark half between scanlines loses. 0 disables the
    /// pass entirely (no doubling); 100 is black gaps.
    pub(crate) scanline_pct: u8,
    /// RF-noise amount, `0..=100`: the luminance jitter amplitude, from
    /// none (0, pass skipped) to a blizzard of snow (100 ≈ ±[`NOISE_FULL`]
    /// levels). See [`noise_rows`].
    pub(crate) noise_pct: u8,
}

impl TVSettings {
    /// Both knobs clamped to `0..=`[`MAX_PCT`] — enforces the range for
    /// values from outside the sliders (a hand-edited `tv_scanline = 250`).
    pub(crate) fn clamped(self) -> Self {
        Self {
            scanline_pct: self.scanline_pct.min(MAX_PCT),
            noise_pct: self.noise_pct.min(MAX_PCT),
        }
    }
}

impl Default for TVSettings {
    fn default() -> Self {
        Self {
            scanline_pct: DEFAULT_SCANLINE_PCT,
            noise_pct: DEFAULT_NOISE_PCT,
        }
    }
}

/// Dark-half per-byte multiplier, ×256 (`out = in · scale >> 8`). Pure
/// scaling commutes with the gamma curve, so a gamma-space multiply exactly
/// implements the linear-light fraction kept — no round trip needed.
fn scanline_scale(scanline_pct: u8) -> u16 {
    let level = 1.0 - f32::from(scanline_pct.min(MAX_PCT)) / f32::from(MAX_PCT);
    (level.powf(1.0 / GAMMA) * 256.0).round() as u16
}

/// [`blur_rows`]' symmetric FIR taps (normalized by [`BLUR_SUM`]): the
/// mild composite/RF softness of a ~4 MHz NTSC luma channel at these dot
/// rates, not a heavy defocus.
const BLUR_TAPS: [u16; 3] = [1, 2, 1];
/// Derived, so re-tuning the taps can't silently break normalization.
const BLUR_SUM: u16 = BLUR_TAPS[0] + BLUR_TAPS[1] + BLUR_TAPS[2];

/// [`process`]'s output: the frame to actually show, with its own
/// dimensions — the TV chain's scanline pass doubles the height, so the
/// output shape is the chain's to decide, not the caller's. `Cow`: a
/// monitor borrows the framebuffer untouched (zero copies), only a TV
/// owns a transformed buffer.
#[derive(Debug)]
pub(crate) struct Frame<'a> {
    pub(crate) pixels: Cow<'a, [u8]>,
    pub(crate) width: usize,
    pub(crate) height: usize,
}

/// The TV chain: framebuffer to displayed frame. A monitor passes through
/// untouched; a TV gets luma collapse (B&W only), horizontal blur, noise,
/// then scanline doubling, in that order.
pub(crate) fn process<'a>(
    display: Display,
    settings: TVSettings,
    seed: u32,
    width: usize,
    src: &'a [u8],
) -> Frame<'a> {
    let height = src.len() / (width * PX);
    let Display::TV(tv) = display else {
        return Frame {
            pixels: Cow::Borrowed(src),
            width,
            height,
        };
    };
    // B&W collapses to luma first; blurring a grey keeps it grey either way.
    let signal: Cow<[u8]> = match tv {
        TV::Color => Cow::Borrowed(src),
        TV::BW => Cow::Owned(collapse_to_luma(src)),
    };
    let mut pixels = blur_rows(width, &signal);
    if settings.noise_pct > 0 {
        noise_rows(settings.noise_pct, seed, &mut pixels);
    }
    let (pixels, height) = if settings.scanline_pct == 0 {
        (pixels, height)
    } else {
        (
            expand_scanlines(settings.scanline_pct, width, &pixels),
            height * 2,
        )
    };
    Frame {
        pixels: Cow::Owned(pixels),
        width,
        height,
    }
}

/// The B&W set's picture: every pixel replaced by its [`luma`] grey,
/// alpha carried through.
fn collapse_to_luma(src: &[u8]) -> Vec<u8> {
    let mut out = src.to_vec();
    for px in out.chunks_exact_mut(PX) {
        let y = luma(px[0], px[1], px[2]);
        px[..3].fill(y);
    }
    out
}

/// Noise amplitude at `noise_pct = 100`, in 8-bit levels: full snow that
/// still leaves the picture faintly underneath rather than pure static.
const NOISE_FULL: i32 = 128;

/// RF noise: the same luminance jitter on all three channels (antenna
/// noise rides luma, not chroma), via a plain xorshift32 PRNG seeded per
/// frame so it shimmers rather than sitting still.
fn noise_rows(noise_pct: u8, seed: u32, bytes: &mut [u8]) {
    let amp = i32::from(noise_pct.min(MAX_PCT)) * NOISE_FULL / i32::from(MAX_PCT);
    // `| 1` keeps xorshift out of its zero fixed point.
    let mut s = seed.wrapping_mul(0x9E37_79B9) | 1;
    for px in bytes.chunks_exact_mut(PX) {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        // High 16 bits as a signed fraction of `amp`: n ∈ [-amp, amp].
        let n = (i32::from((s >> 16) as i16) * amp) >> 15;
        for c in &mut px[..3] {
            *c = (i32::from(*c) + n).clamp(0, 255) as u8;
        }
    }
}

/// Scanline pass: each row becomes a full-brightness row plus a
/// [`scanline_scale`]-dimmed copy, mimicking a CRT raster's line/gap
/// structure. Must double rather than darken in place — the source rows
/// *are* the scanlines.
fn expand_scanlines(scanline_pct: u8, width: usize, src: &[u8]) -> Vec<u8> {
    let row_len = width * PX;
    let scale = scanline_scale(scanline_pct);
    // +128 for round-to-nearest; scale ≤ 256 keeps the product in u16.
    let dark = |v: u8| ((u16::from(v) * scale + 128) >> 8) as u8;
    let mut out = Vec::with_capacity(src.len() * 2);
    for row in src.chunks_exact(row_len) {
        out.extend_from_slice(row);
        for px in row.chunks_exact(PX) {
            out.extend_from_slice(&[dark(px[0]), dark(px[1]), dark(px[2]), px[3]]);
        }
    }
    out
}

/// Horizontal bandwidth limit: a [`BLUR_TAPS`] FIR across each row only —
/// an analog signal band-limits per scanline, so rows stay separate while
/// detail smears along the line. Edges clamp; alpha passes through.
fn blur_rows(width: usize, src: &[u8]) -> Vec<u8> {
    let row_len = width * PX;
    let mut out = Vec::with_capacity(src.len());
    for row in src.chunks_exact(row_len) {
        for x in 0..width {
            let window = |i: usize| &row[i * PX..][..PX];
            let prev = window(x.saturating_sub(1));
            let cur = window(x);
            let next = window((x + 1).min(width - 1));
            for c in 0..3 {
                let sum = u16::from(prev[c]) * BLUR_TAPS[0]
                    + u16::from(cur[c]) * BLUR_TAPS[1]
                    + u16::from(next[c]) * BLUR_TAPS[2];
                // +half for round-to-nearest rather than truncation.
                out.push(((sum + BLUR_SUM / 2) / BLUR_SUM) as u8);
            }
            out.push(cur[3]);
        }
    }
    out
}

#[cfg(test)]
#[path = "display_test.rs"]
mod tests;
