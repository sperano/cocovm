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

/// Pick a random image from the per-user image assets and decode it.
/// Missing directory, empty directory, or a decode failure all yield `None`
/// (with a log line), never an error — callers show nothing instead.
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

        let photo = random_from_dir(&dir).expect("an image exists, so a photo is decoded");
        assert_eq!(photo.title, "page-1");
        assert_eq!(photo.pixels.size, [4, 6]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn random_from_dir_yields_none_when_no_images() {
        let dir = std::env::temp_dir().join("coco-photo-view-test-empty");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(random_from_dir(&dir).is_none());
        std::fs::remove_dir_all(&dir).unwrap();

        assert!(random_from_dir(Path::new("/nonexistent-dir")).is_none());
    }
}
