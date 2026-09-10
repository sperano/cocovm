//! Presentation invalidation follows rendered pixels, including partial debugger
//! steps and restored framebuffers. Memory/register writes alone need not render.

use std::time::{Duration, Instant};

use crate::display::{Display, Frame, Processor, TVSettings};

/// Snow advances at 60 Hz, independently of host refresh and incidental repaints.
/// A delayed presentation skips missed ticks instead of slowing the animation.
pub(super) const NOISE_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 60);

#[derive(Clone, Copy, PartialEq, Eq)]
struct PresentationKey {
    display: Display,
    scanline_pct: u8,
    noise_pct: u8,
    seed: u32,
    width: usize,
}

#[derive(Default)]
pub(crate) struct Presentation {
    processor: Processor,
    source: Vec<u8>,
    key: Option<PresentationKey>,
    epoch: Option<Instant>,
}

impl Presentation {
    /// UV cropping and window geometry do not change texture pixels. Monitor
    /// filtering is part of `display`; TV-only settings are ignored on monitors.
    fn key(
        &mut self,
        display: Display,
        settings: TVSettings,
        width: usize,
        now: Instant,
    ) -> PresentationKey {
        let settings = settings.clamped();
        let is_tv = matches!(display, Display::TV(_));
        let seed = if is_tv && settings.noise_pct > 0 {
            let epoch = *self.epoch.get_or_insert(now);
            (now.saturating_duration_since(epoch).as_nanos() / NOISE_INTERVAL.as_nanos()) as u32
        } else {
            0
        };
        PresentationKey {
            display,
            scanline_pct: if is_tv { settings.scanline_pct } else { 0 },
            noise_pct: if is_tv { settings.noise_pct } else { 0 },
            seed,
            width,
        }
    }

    pub(super) fn prepare(
        &mut self,
        display: Display,
        settings: TVSettings,
        width: usize,
        src: &[u8],
        now: Instant,
    ) -> Option<eframe::egui::ColorImage> {
        let key = self.key(display, settings, width, now);
        let pixels_changed = self.source != src;
        if self.key == Some(key) && !pixels_changed {
            return None;
        }
        if pixels_changed {
            self.source.clear();
            self.source.extend_from_slice(src);
        }
        self.key = Some(key);
        let _conversion = crate::perf::span(crate::perf::Stage::DisplayConversion);
        let frame = self
            .processor
            .process(display, settings, key.seed, width, src);
        Some(eframe::egui::ColorImage::from_rgba_unmultiplied(
            [frame.width, frame.height],
            frame.pixels,
        ))
    }

    /// Save previews through the same reusable processor, without claiming that
    /// a texture was uploaded. Freeze the last presented animation seed.
    pub(crate) fn snapshot<'a>(
        &'a mut self,
        display: Display,
        settings: TVSettings,
        width: usize,
        src: &'a [u8],
    ) -> Frame<'a> {
        let seed = self.key.map_or(0, |key| key.seed);
        self.processor.process(display, settings, seed, width, src)
    }

    pub(super) fn invalidate(&mut self) {
        self.key = None;
    }
}

#[cfg(test)]
#[path = "presentation_test.rs"]
mod tests;
