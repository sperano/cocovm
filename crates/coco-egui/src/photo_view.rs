//! Photo loading for the manager window's right pane: picks an image from
//! the per-user image assets (`paths::images_dir()`, CoCo 3 manual scans
//! from the first-run assets download) — at random, or the next in
//! file-name order — and decodes it. A bit of period atmosphere, à la
//! Virtual ][.
//!
//! (This module once showed the photo in its own native OS window at
//! startup; that `PhotoWindow` viewport was replaced by the manager window's
//! image pane — see `manager/welcome_image.rs`.)

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;

/// Extensions the pick considers (the assets tarball ships PNGs, and the
/// `image` dependency is built with only its `png` feature).
const IMAGE_EXTENSIONS: &[&str] = &["png"];

/// A decoded image asset, ready for a first-frame texture upload.
pub struct Photo {
    /// File stem of the chosen image — texture debug name (and available as
    /// a caption later).
    pub title: String,
    pub pixels: egui::ColorImage,
}

/// Picks a random image from `dir` and decodes it. `exclude` is the title
/// of the image currently shown: skipped whenever another candidate exists,
/// so a shuffled change always shows something new. Missing directory,
/// empty directory, or a decode failure all yield `None` (with a log line),
/// never an error.
pub(crate) fn random_from_dir(dir: &Path, exclude: Option<&str>) -> Option<Photo> {
    let mut files = image_files(dir)?;
    // Only drop the excluded stem when another candidate remains: a repeat
    // beats an empty list (and its modulo-by-zero below).
    if let Some(exclude) = exclude
        && files.iter().any(|p| !has_stem(p, exclude))
    {
        files.retain(|p| !has_stem(p, exclude));
    }
    // No `rand` dependency: the clock's sub-second nanoseconds are plenty of entropy here.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    decode(&files[nanos as usize % files.len()])
}

/// The image after `current` in file-name order, wrapping around to the
/// first; the first when `current` is `None` or no longer in `dir`. Same
/// `None` cases as [`random_from_dir`].
pub(crate) fn next_in_dir(dir: &Path, current: Option<&str>) -> Option<Photo> {
    let files = image_files(dir)?;
    let index = current
        .and_then(|current| files.iter().position(|p| has_stem(p, current)))
        .map_or(0, |i| (i + 1) % files.len());
    decode(&files[index])
}

/// The image files in `dir`, sorted by name (read_dir order is arbitrary).
/// `None`, logged, when the directory can't be read or holds none.
fn image_files(dir: &Path) -> Option<Vec<PathBuf>> {
    let mut files: Vec<_> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                // Skip hidden files: macOS drops AppleDouble sidecars ("._foo.png") that aren't
                // decodable.
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
    files.sort();
    Some(files)
}

fn decode(path: &Path) -> Option<Photo> {
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

fn has_stem(path: &Path, stem: &str) -> bool {
    path.file_stem().and_then(|s| s.to_str()) == Some(stem)
}

#[cfg(test)]
#[path = "photo_view_test.rs"]
mod tests;
