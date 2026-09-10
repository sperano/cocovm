//! The manager's welcome image: the random photo filling the right pane
//! while no machine is selected (`photo_view.rs`), and the optional timer
//! (`welcome_image_cycle` / `welcome_image_cycle_secs` /
//! `welcome_image_shuffle`, `config.rs`) that changes it, in file-name
//! order or at random, with a short crossfade.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::photo_view::{self, Photo};

/// How long a change crossfades from the old image to the new one.
const FADE_DURATION: Duration = Duration::from_millis(400);

/// A crossfade in progress: the image being replaced and when it started.
struct Fade {
    previous: egui::TextureHandle,
    started: Instant,
}

impl Fade {
    /// 0.0 at the start, 1.0 once [`FADE_DURATION`] has passed.
    fn progress(&self, now: Instant) -> f32 {
        (now.saturating_duration_since(self.started).as_secs_f32() / FADE_DURATION.as_secs_f32())
            .min(1.0)
    }
}

/// The welcome image, its change timer, and the `welcome_image_*` settings
/// with their CLI/env override flags (`Config::*_overridden` semantics).
pub(crate) struct WelcomeImage {
    /// Decoded photo pending its first-frame texture upload.
    pub(crate) photo: Option<Photo>,
    /// The uploaded texture, once a frame has run.
    texture: Option<egui::TextureHandle>,
    /// Per-user image assets (`paths::images_dir()`); `None` when no home
    /// directory exists, and in tests, which must never read the real assets.
    pub(crate) images_dir: Option<PathBuf>,
    /// `welcome_image_cycle`: change the image on a timer.
    pub(crate) cycle: bool,
    pub(crate) cycle_overridden: bool,
    /// `welcome_image_cycle_secs`: the timer's interval.
    pub(crate) cycle_secs: NonZeroU32,
    pub(crate) cycle_secs_overridden: bool,
    /// `welcome_image_shuffle`: pick the next image at random rather than
    /// in file-name order.
    pub(crate) shuffle: bool,
    pub(crate) shuffle_overridden: bool,
    /// When the next change is due; `None` while the timer is disarmed
    /// (cycle off, or a machine selected).
    due: Option<Instant>,
    /// The crossfade from the previous image, while one is running.
    fade: Option<Fade>,
}

impl WelcomeImage {
    /// Settings at their built-in defaults, no images directory, and
    /// `photo` (if any) queued for the first frame's upload.
    pub(crate) fn new(photo: Option<Photo>) -> Self {
        Self {
            photo,
            texture: None,
            images_dir: None,
            cycle: crate::config::DEFAULT_WELCOME_IMAGE_CYCLE,
            cycle_overridden: false,
            cycle_secs: crate::config::DEFAULT_WELCOME_IMAGE_CYCLE_SECS,
            cycle_secs_overridden: false,
            shuffle: crate::config::DEFAULT_WELCOME_IMAGE_SHUFFLE,
            shuffle_overridden: false,
            due: None,
            fade: None,
        }
    }

    /// True while nothing has been loaded or queued.
    pub(crate) fn is_blank(&self) -> bool {
        self.photo.is_none() && self.texture.is_none()
    }

    /// Queues a random image from `images_dir`: the startup pick, and the
    /// reseed after the first-run asset download.
    pub(crate) fn load_random(&mut self) {
        if let Some(dir) = self.images_dir.as_deref()
            && let Some(photo) = photo_view::random_from_dir(dir, None)
        {
            self.photo = Some(photo);
        }
    }

    /// Disarms the timer so the next tick starts a fresh interval — after a
    /// Settings save, so a new interval counts from now.
    pub(crate) fn rearm(&mut self) {
        self.due = None;
    }

    /// Once per frame, before the panels draw: runs the timer (only while
    /// `showing`, i.e. nothing is selected) and uploads any pending photo so
    /// the pane shows it this same frame.
    pub(crate) fn service(&mut self, ctx: &egui::Context, showing: bool) {
        let now = Instant::now();
        if let Some(due) = self.tick(now, showing) {
            crate::app::scheduling::request_repaint_at(ctx, due);
        }
        self.upload(ctx, now);
    }

    /// The change timer. While `showing` and the cycle is on, queues the
    /// next image once `cycle_secs` have passed and returns when the change
    /// after that is due (for the repaint scheduler). Disarms otherwise, so
    /// re-showing the pane starts a fresh interval.
    fn tick(&mut self, now: Instant, showing: bool) -> Option<Instant> {
        if !self.cycle || !showing {
            self.due = None;
            return None;
        }
        if let Some(due) = self.due
            && now < due
        {
            return Some(due);
        }
        if self.due.is_some() {
            self.pick_next();
        }
        let due = now + Duration::from_secs(u64::from(self.cycle_secs.get()));
        self.due = Some(due);
        Some(due)
    }

    /// Queues the image after the one showing: a random other one when
    /// shuffling, else the next in file-name order (wrapping around). Keeps
    /// the current one when nothing can be loaded.
    fn pick_next(&mut self) {
        let Some(dir) = self.images_dir.as_deref() else {
            return;
        };
        let current = self.texture.as_ref().map(|texture| texture.name());
        let next = if self.shuffle {
            photo_view::random_from_dir(dir, current.as_deref())
        } else {
            photo_view::next_in_dir(dir, current.as_deref())
        };
        if next.is_some() {
            self.photo = next;
        }
    }

    /// Uploads a pending photo. When one was already showing, it becomes
    /// the outgoing half of a crossfade starting `now`.
    fn upload(&mut self, ctx: &egui::Context, now: Instant) {
        let Some(photo) = self.photo.take() else {
            return;
        };
        let texture = ctx.load_texture(&photo.title, photo.pixels, egui::TextureOptions::LINEAR);
        if let Some(previous) = self.texture.replace(texture) {
            self.fade = Some(Fade {
                previous,
                started: now,
            });
        }
    }

    /// Advances the crossfade: its progress this frame (1.0 with none
    /// running), dropping it once complete. Repaints continuously until then.
    fn fade_progress(&mut self, ctx: &egui::Context, now: Instant) -> f32 {
        let Some(fade) = &self.fade else {
            return 1.0;
        };
        let progress = fade.progress(now);
        if progress >= 1.0 {
            self.fade = None;
        } else {
            ctx.request_repaint();
        }
        progress
    }

    /// The right pane while nothing is selected: the photo, letterboxed to
    /// fit, with the previous one fading out over it during a change.
    /// Nothing at all when no image asset could be loaded.
    pub(crate) fn draw(&mut self, ui: &mut egui::Ui) {
        let progress = self.fade_progress(ui.ctx(), Instant::now());
        let Some(texture) = &self.texture else {
            return;
        };
        let rect = ui.available_rect_before_wrap();
        if let Some(fade) = &self.fade {
            ui.put(rect, fitted_image(&fade.previous, rect, 1.0 - progress));
        }
        ui.put(rect, fitted_image(texture, rect, progress));
    }
}

/// `texture` letterboxed into `rect` at `opacity` (0.0 invisible, 1.0 opaque).
fn fitted_image(texture: &egui::TextureHandle, rect: egui::Rect, opacity: f32) -> egui::Image<'_> {
    egui::Image::new(texture)
        .max_size(rect.size())
        .maintain_aspect_ratio(true)
        .tint(egui::Color32::WHITE.gamma_multiply(opacity))
}

#[cfg(test)]
#[path = "welcome_image_test.rs"]
mod tests;
