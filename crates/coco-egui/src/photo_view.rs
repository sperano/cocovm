//! The "photo" window: a second window shown at startup with an image picked
//! at random from the per-user image assets (`paths::images_dir()`, CoCo 3
//! manual scans from the first-run assets download) — a bit of period
//! atmosphere next to the emulator, à la Virtual ][.
//!
//! Shown as its own native OS window (an egui *immediate viewport*, same
//! pattern as [`crate::paper_view`]); on backends without multi-window
//! support (e.g. the `egui_kittest` harness) egui reports
//! `ViewportClass::Embedded` and the view falls back to an in-viewport
//! `egui::Window`.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;

/// Largest native-window size the image is fitted into at open; the window
/// stays resizable and the image rescales to fit.
const MAX_WINDOW_SIZE: [f32; 2] = [560.0, 720.0];

/// Extensions the random pick considers (the assets tarball ships PNGs, and
/// the `image` dependency is built with only its `png` feature).
const IMAGE_EXTENSIONS: &[&str] = &["png"];

pub struct PhotoWindow {
    /// Whether the window is currently shown. Starts open when an image was
    /// found; toggled by the View-menu checkbox or the window's close button.
    pub open: bool,
    /// File stem of the chosen image, used as the window title.
    title: String,
    /// Decoded pixels, pending first-frame upload into `texture`.
    pending: Option<egui::ColorImage>,
    /// The uploaded texture, once a frame has run.
    texture: Option<egui::TextureHandle>,
}

impl Default for PhotoWindow {
    /// A closed window with nothing loaded — what [`crate::CocoApp::new`]
    /// starts with; only `main()`'s creation closure swaps in
    /// [`PhotoWindow::random`].
    fn default() -> Self {
        Self::closed()
    }
}

impl PhotoWindow {
    /// A window that never opens: used when no image asset can be found (or
    /// decoded) — startup proceeds without a photo rather than erroring.
    fn closed() -> Self {
        Self {
            open: false,
            title: String::new(),
            pending: None,
            texture: None,
        }
    }

    /// Pick a random image from the per-user image assets and decode it.
    /// Missing directory, empty directory, or a decode failure all yield a
    /// window that stays closed (with a log line), never a startup error.
    pub fn random() -> Self {
        let Some(dir) = crate::paths::images_dir() else {
            return Self::closed();
        };
        Self::random_from_dir(&dir)
    }

    /// [`PhotoWindow::random`] with the directory injected, so tests can
    /// point it at a temp dir instead of the real per-user assets.
    fn random_from_dir(dir: &Path) -> Self {
        let mut files: Vec<_> = match std::fs::read_dir(dir) {
            Ok(entries) => entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    // Skip hidden files: macOS drops AppleDouble sidecars
                    // ("._foo.png") next to the real assets, and they are
                    // not decodable PNGs.
                    let hidden = p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_none_or(|n| n.starts_with('.'));
                    !hidden
                        && p.extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| {
                                IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())
                            })
                })
                .collect(),
            Err(e) => {
                tracing::debug!("no photo window: cannot read {}: {e}", dir.display());
                return Self::closed();
            }
        };
        if files.is_empty() {
            tracing::debug!("no photo window: no images in {}", dir.display());
            return Self::closed();
        }
        // Stable order before indexing — read_dir order is arbitrary.
        files.sort();
        // No `rand` dependency: the clock's sub-second nanoseconds are
        // plenty of entropy to pick one of a few dozen images.
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let path = &files[nanos as usize % files.len()];

        let image = match image::open(path) {
            Ok(image) => image.to_rgba8(),
            Err(e) => {
                tracing::warn!("no photo window: cannot decode {}: {e}", path.display());
                return Self::closed();
            }
        };
        let size = [image.width() as usize, image.height() as usize];
        let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let title = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Photo".to_owned());
        Self {
            open: true,
            title,
            pending: Some(pixels),
            texture: None,
        }
    }

    /// Native-window size fitting the image inside [`MAX_WINDOW_SIZE`]
    /// without upscaling, aspect ratio preserved.
    fn fitted_size(image_size: [usize; 2]) -> [f32; 2] {
        let [w, h] = [image_size[0] as f32, image_size[1] as f32];
        let scale = (MAX_WINDOW_SIZE[0] / w)
            .min(MAX_WINDOW_SIZE[1] / h)
            .min(1.0);
        [w * scale, h * scale]
    }

    /// Draw the window if open; a no-op frame otherwise (called
    /// unconditionally once per `update()`, like the other optional windows
    /// in `main.rs`).
    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        if let Some(pixels) = self.pending.take() {
            self.texture =
                Some(ctx.load_texture(&self.title, pixels, egui::TextureOptions::LINEAR));
        }
        let Some(texture) = self.texture.clone() else {
            // Nothing decoded (should not happen while `open`): close for good.
            self.open = false;
            return;
        };

        // One stable ID so egui reuses the same native OS window across
        // frames instead of spawning a new one.
        let viewport_id = egui::ViewportId::from_hash_of("photo-view");
        let size = texture.size();
        let builder = egui::ViewportBuilder::default()
            .with_title(&self.title)
            .with_inner_size(Self::fitted_size(size));
        ctx.show_viewport_immediate(viewport_id, builder, |ctx, class| {
            if class == egui::ViewportClass::Embedded {
                // Backend without native multi-window support: fall back to
                // an embedded in-viewport window.
                let mut open = self.open;
                egui::Window::new(crate::window_title(ctx, &self.title))
                    .open(&mut open)
                    .default_size(Self::fitted_size(size))
                    .show(ctx, |ui| {
                        ui.add(
                            egui::Image::new(&texture)
                                .max_size(ui.available_size())
                                .maintain_aspect_ratio(true),
                        );
                    });
                self.open = open;
            } else {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default().fill(egui::Color32::BLACK))
                    .show(ctx, |ui| {
                        ui.centered_and_justified(|ui| {
                            ui.add(
                                egui::Image::new(&texture)
                                    .max_size(ui.available_size())
                                    .maintain_aspect_ratio(true),
                            );
                        });
                    });
                // The OS close button: accept the close by not showing the
                // viewport next frame (mirrors the View-menu checkbox).
                if ctx.input(|i| i.viewport().close_requested()) {
                    self.open = false;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a tiny valid PNG the `image` crate can round-trip.
    fn write_test_png(path: &Path, w: u32, h: u32) {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        img.save(path).unwrap();
    }

    #[test]
    fn random_from_dir_picks_and_decodes_an_image() {
        let dir = std::env::temp_dir().join("coco-photo-view-test-picks");
        std::fs::create_dir_all(&dir).unwrap();
        write_test_png(&dir.join("page-1.png"), 4, 6);
        std::fs::write(dir.join("not-an-image.txt"), b"ignored").unwrap();
        // AppleDouble sidecar: right extension, but hidden and not a PNG.
        std::fs::write(dir.join("._page-1.png"), b"AppleDouble junk").unwrap();

        let photo = PhotoWindow::random_from_dir(&dir);
        assert!(photo.open, "an image exists, so the window starts open");
        assert_eq!(photo.title, "page-1");
        assert_eq!(
            photo.pending.as_ref().map(|p| p.size),
            Some([4, 6]),
            "decoded pixels are staged for first-frame upload"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn random_from_dir_stays_closed_when_no_images() {
        let dir = std::env::temp_dir().join("coco-photo-view-test-empty");
        std::fs::create_dir_all(&dir).unwrap();
        let photo = PhotoWindow::random_from_dir(&dir);
        assert!(!photo.open);
        assert!(photo.pending.is_none());
        std::fs::remove_dir_all(&dir).unwrap();

        let photo = PhotoWindow::random_from_dir(Path::new("/nonexistent-dir"));
        assert!(!photo.open);
    }

    #[test]
    fn fitted_size_shrinks_to_fit_but_never_upscales() {
        // Taller than the cap: scaled down, aspect preserved.
        let [w, h] = PhotoWindow::fitted_size([1000, 2000]);
        assert_eq!(h, MAX_WINDOW_SIZE[1]);
        assert_eq!(w, MAX_WINDOW_SIZE[1] / 2.0);
        // Smaller than the cap: left at native size.
        assert_eq!(PhotoWindow::fitted_size([300, 200]), [300.0, 200.0]);
    }
}
