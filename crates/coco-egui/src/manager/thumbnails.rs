//! Preview-thumbnail cache management: writing a VM's framebuffer out to
//! its artifact directory at suspend time, and lazily loading a suspended
//! machine's saved preview into a texture for the list row. See
//! [`super::write_thumbnail_png`] for the on-disk format/write contract.

use eframe::egui;

use super::{ManagerApp, THUMBNAIL_FILE};

impl ManagerApp {
    /// Snapshot `entries[index]`'s live VM screen into its artifact dir
    /// (see [`super::write_thumbnail_png`]) and invalidate the row's cached preview
    /// texture so the next draw reloads the fresh file — the screenshot
    /// half of Suspend (`super::lifecycle`'s `suspend_vm`). No-op for an
    /// entry with no VM or when no artifact root exists. Capture happens
    /// between update frames, so the framebuffer always holds a whole
    /// rendered field — never a torn, mid-render frame.
    pub(super) fn write_entry_thumbnail(&mut self, index: usize) {
        let Some(root) = &self.artifacts_root else {
            return;
        };
        let entry = &mut self.entries[index];
        let Some(vm) = entry.vm.as_ref() else {
            return;
        };
        // The raw framebuffer bypasses `upload_framebuffer_texture`'s TV
        // chain, so run it here too — a B&W machine's saved preview must
        // not come back in color (nor a TV's without its scanlines).
        let frame = crate::display::process(
            vm.display,
            vm.tv,
            vm.machine.fb_width as usize,
            &vm.machine.framebuffer,
        );
        if let Err(e) = super::write_thumbnail_png(
            &root.join(&entry.slug),
            &frame.pixels,
            frame.width as u32,
            frame.height as u32,
        ) {
            tracing::warn!("thumbnail for '{}': {e}", entry.slug);
        }
        entry.thumbnail = None;
        entry.thumbnail_load_attempted = false;
    }

    /// Lazily load a suspended, window-closed entry's saved
    /// [`THUMBNAIL_FILE`] into a texture the first time its row draws (and
    /// again after [`Self::write_entry_thumbnail`] invalidates the cache).
    /// Only suspended machines show a saved preview — a powered-off row is
    /// deliberately black — so nothing else ever loads one. Failures just
    /// leave the placeholder — the preview is a cache, never required state.
    pub(super) fn ensure_row_thumbnail(&mut self, ctx: &egui::Context, index: usize) {
        let entry = &mut self.entries[index];
        if entry.vm.is_some() || !entry.suspended || entry.thumbnail_load_attempted {
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
