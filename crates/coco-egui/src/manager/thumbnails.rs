//! Preview-thumbnail cache management: writing a running VM's framebuffer
//! out to its artifact directory on a cadence, and lazily loading a stopped
//! machine's saved preview into a texture for the list row. See
//! [`super::write_thumbnail_png`] for the on-disk format/write contract.

use eframe::egui;

use super::{ManagerApp, THUMBNAIL_FILE, THUMBNAIL_REFRESH};

impl ManagerApp {
    /// Snapshot `entries[index]`'s running VM screen into its artifact dir
    /// (see [`super::write_thumbnail_png`]) and invalidate the row's cached preview
    /// texture so the next draw reloads the fresh file. No-op for a stopped
    /// entry or when no artifact root exists. Capture happens between update
    /// frames, so the framebuffer always holds a whole rendered field —
    /// never a torn, mid-render frame.
    pub(super) fn write_entry_thumbnail(&mut self, index: usize) {
        let Some(root) = &self.artifacts_root else {
            return;
        };
        let entry = &mut self.entries[index];
        let Some(vm) = entry.vm.as_ref() else {
            return;
        };
        let (w, h) = (vm.machine.fb_width, vm.machine.fb_height);
        if let Err(e) =
            super::write_thumbnail_png(&root.join(&entry.slug), &vm.machine.framebuffer, w, h)
        {
            tracing::warn!("thumbnail for '{}': {e}", entry.slug);
        }
        entry.thumbnail = None;
        entry.thumbnail_load_attempted = false;
        entry.last_thumbnail_write = Some(std::time::Instant::now());
    }

    /// [`THUMBNAIL_REFRESH`] cadence for every running VM — called once per
    /// `update()`. The first write happens right after Start
    /// (`last_thumbnail_write` starts `None`), so even a young machine has
    /// an on-disk preview if the process dies.
    pub(super) fn refresh_due_thumbnails(&mut self) {
        if self.artifacts_root.is_none() {
            return;
        }
        for i in 0..self.entries.len() {
            if self.entries[i].vm.is_none() {
                continue;
            }
            let due = self.entries[i]
                .last_thumbnail_write
                .is_none_or(|last| last.elapsed() >= THUMBNAIL_REFRESH);
            if due {
                self.write_entry_thumbnail(i);
            }
        }
    }

    /// Lazily load a stopped entry's saved [`THUMBNAIL_FILE`] into a texture
    /// the first time its row draws (and again after
    /// [`Self::write_entry_thumbnail`] invalidates the cache). Failures just
    /// leave the placeholder — the preview is a cache, never required state.
    pub(super) fn ensure_row_thumbnail(&mut self, ctx: &egui::Context, index: usize) {
        let entry = &mut self.entries[index];
        if entry.vm.is_some() || entry.thumbnail_load_attempted {
            return;
        }
        entry.thumbnail_load_attempted = true;
        let Some(root) = &self.artifacts_root else {
            return;
        };
        let Ok(image) = image::open(root.join(&entry.slug).join(THUMBNAIL_FILE)) else {
            return;
        };
        let image = image.to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        entry.thumbnail = Some(ctx.load_texture(
            format!("thumbnail-{}", entry.slug),
            pixels,
            egui::TextureOptions::LINEAR,
        ));
    }
}
