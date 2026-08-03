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
/// look, done by the GPU during normal drawing rather than in [`process`].
pub(crate) fn texture_options(display: Display) -> egui::TextureOptions {
    match display {
        Display::Monitor(_) => egui::TextureOptions::NEAREST,
        Display::TV(_) => egui::TextureOptions::LINEAR,
    }
}

/// RGBA8 bytes per pixel, the framebuffer's own layout
/// (`coco_core::video::BYTES_PER_PIXEL`).
const PX: usize = 4;

/// Default [`TVSettings::scanline_pct`]: the strength the look was tuned
/// at (the dark half keeps 65% of the line's linear-light brightness).
const DEFAULT_SCANLINE_PCT: u8 = 35;

/// Default [`TVSettings::noise_pct`]: a whisper of snow — present enough
/// to feel like an antenna feed, not enough to obscure anything.
const DEFAULT_NOISE_PCT: u8 = 5;

/// User-adjustable knobs of the TV chain — a UI preference riding along
/// with [`Display`] (View-menu sliders live, `[ui]` keys persisted).
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

impl Default for TVSettings {
    fn default() -> Self {
        Self {
            scanline_pct: DEFAULT_SCANLINE_PCT,
            noise_pct: DEFAULT_NOISE_PCT,
        }
    }
}

/// The dark scanline half's per-byte multiplier, ×256
/// (`out = in · scale >> 8`): the strength is a **linear-light** fraction
/// kept, and pure scaling commutes with the gamma curve
/// (`(x^γ·k)^(1/γ) = x·k^(1/γ)`), so one gamma-space multiply per byte is
/// exact — no linear round trip.
fn scanline_scale(scanline_pct: u8) -> u16 {
    let level = 1.0 - f32::from(scanline_pct.min(100)) / 100.0;
    (level.powf(1.0 / GAMMA) * 256.0).round() as u16
}

/// [`blur_rows`]' symmetric FIR taps (normalized by [`BLUR_SHIFT`]): the
/// mild composite/RF softness of a ~4 MHz NTSC luma channel at these dot
/// rates, not a heavy defocus.
const BLUR_TAPS: [u16; 3] = [1, 2, 1];
const BLUR_SHIFT: u16 = 2;

/// [`process`]'s output: the frame to actually show, with its own
/// dimensions — the TV chain's scanline pass doubles the height, so the
/// output shape is the chain's to decide, not the caller's. `Cow`: a
/// monitor borrows the framebuffer untouched (zero copies), only a TV
/// owns a transformed buffer.
pub(crate) struct Frame<'a> {
    pub(crate) pixels: std::borrow::Cow<'a, [u8]>,
    pub(crate) width: usize,
    pub(crate) height: usize,
}

/// The TV chain, turning the machine's RGBA8 framebuffer into the frame to
/// show — the texture upload (`CocoApp::upload_framebuffer_texture`) and
/// the thumbnail-PNG capture (`manager::thumbnails`). `width` is the
/// source's width in pixels (rows are `width · 4` bytes). Monitors pass
/// through untouched. Both TVs get, in order: the B&W luma collapse
/// (B&W set only — blurring a grey keeps it grey, so the order only
/// matters for color), the composite/RF horizontal bandwidth limit
/// ([`blur_rows`]), the RF noise ([`noise_rows`], varied per frame by
/// `seed`), and finally the scanline doubling ([`expand_scanlines`]) at
/// `settings`' strength.
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
            pixels: std::borrow::Cow::Borrowed(src),
            width,
            height,
        };
    };
    let mut pixels = src.to_vec();
    if tv == TV::BW {
        for px in pixels.chunks_exact_mut(PX) {
            let y = luma(px[0], px[1], px[2]);
            px[0] = y;
            px[1] = y;
            px[2] = y;
        }
    }
    blur_rows(width, &mut pixels);
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
        pixels: std::borrow::Cow::Owned(pixels),
        width,
        height,
    }
}

/// Noise amplitude at `noise_pct = 100`, in 8-bit levels: full snow that
/// still leaves the picture faintly underneath rather than pure static.
const NOISE_FULL: i32 = 128;

/// The RF noise: per-pixel **luminance** jitter — the same offset on all
/// three channels, because antenna noise rides the luma of the signal —
/// varied frame to frame by `seed` so it shimmers instead of sitting like
/// dirt on the glass. Runs before the scanline doubling: both halves of a
/// scanline carry the same signal, so they share the same noise. The PRNG
/// is a plain xorshift32 — decorrelated neighbors are all snow needs.
fn noise_rows(noise_pct: u8, seed: u32, bytes: &mut [u8]) {
    let amp = i32::from(noise_pct.min(100)) * NOISE_FULL / 100;
    // Mix the seed so consecutive frame counters land far apart; `| 1`
    // keeps xorshift out of its zero fixed point.
    let mut s = seed.wrapping_mul(0x9E37_79B9) | 1;
    for px in bytes.chunks_exact_mut(PX) {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        // High 16 bits as a signed fraction of `amp`: n ∈ [-amp, amp].
        let n = (i32::from((s >> 16) as u16 as i16) * amp) >> 15;
        for c in 0..3 {
            px[c] = (i32::from(px[c]) + n).clamp(0, 255) as u8;
        }
    }
}

/// The scanline pass: each source row becomes a full-brightness row plus a
/// [`scanline_scale`]-dimmed copy — the visible line structure of a CRT
/// raster, where the beam lights a line and the gap between lines stays
/// darker. Doubling (rather than darkening rows in place) is what makes
/// this possible at all: the source rows *are* the scanlines, so an
/// in-place version would delete half the picture.
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
