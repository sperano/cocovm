//! Random-photo loading for the manager window's right pane: picks an image
//! at random from the per-user image assets (`paths::images_dir()`, CoCo 3
//! manual scans from the first-run assets download) and decodes it — a bit
//! of period atmosphere, à la Virtual ][.
//!
//! (This module once showed the photo in its own native OS window at
//! startup; that `PhotoWindow` viewport was replaced by the manager window's
//! image pane — see `manager.rs`.)

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;

/// Extensions the random pick considers (the assets tarball ships PNGs, and
/// the `image` dependency is built with only its `png` feature).
const IMAGE_EXTENSIONS: &[&str] = &["png"];

/// A decoded image asset, ready for a first-frame texture upload.
pub struct Photo {
    /// File stem of the chosen image — texture debug name (and available as
    /// a caption later).
    pub title: String,
    pub pixels: egui::ColorImage,
}

/// Picks a random image from the per-user image assets and decodes it. Missing directory,
/// empty directory, or a decode failure all yield `None` (with a log line), never an error.
pub fn random() -> Option<Photo> {
    random_from_dir(&crate::paths::images_dir()?)
}

/// [`random`] with the directory injected, so tests can point it at a temp
/// dir instead of the real per-user assets.
fn random_from_dir(dir: &Path) -> Option<Photo> {
    let mut files: Vec<_> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                // Skip hidden files: macOS drops AppleDouble sidecars ("._foo.png") that aren't decodable.
                let hidden = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_none_or(|n| n.starts_with('.'));
                !hidden
                    && p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                        IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())
                    })
            })
            .collect(),
        Err(e) => {
            tracing::debug!("no photo: cannot read {}: {e}", dir.display());
            return None;
        }
    };
    if files.is_empty() {
        tracing::debug!("no photo: no images in {}", dir.display());
        return None;
    }
    // Stable order before indexing — read_dir order is arbitrary.
    files.sort();
    // No `rand` dependency: the clock's sub-second nanoseconds are plenty of entropy here.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let path = &files[nanos as usize % files.len()];

    let image = match image::open(path) {
        Ok(image) => image.to_rgba8(),
        Err(e) => {
            tracing::warn!("no photo: cannot decode {}: {e}", path.display());
            return None;
        }
    };
    let size = [image.width() as usize, image.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    let title = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Photo".to_owned());
    Some(Photo { title, pixels })
}

#[cfg(test)]
#[path = "photo_view_test.rs"]
mod tests;
