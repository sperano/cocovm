//! Preview-thumbnail cache management: writing a VM's framebuffer out to
//! its artifact directory at suspend time, and lazily loading a suspended
//! machine's saved preview into a texture for the list row. See
//! [`super::write_thumbnail_png`] for the on-disk format/write contract.
//! Also the preview-drawing helpers shared by the list rows and the detail
//! pane's big screen preview.

use eframe::egui;

use super::{
    CocoApp, MachineEntry, ManagerApp, THUMBNAIL_CORNER_RADIUS, THUMBNAIL_FILE,
    THUMBNAIL_PLACEHOLDER_FILL,
};

/// One entry's preview — live VM framebuffer, else a suspended machine's
/// saved thumbnail (`None` = placeholder) — with its TV-settings crop.
pub(super) fn preview_source(entry: &MachineEntry) -> (Option<&egui::TextureHandle>, egui::Rect) {
    let texture = entry
        .vm
        .as_deref()
        .and_then(CocoApp::framebuffer_texture)
        .or(entry.thumbnail.as_ref().filter(|_| entry.suspended));
    let uv = entry.vm.as_deref().map_or_else(
        || definition_texture_uv(&entry.def),
        |vm| crate::display::texture_uv(vm.display, vm.tv),
    );
    (texture, uv)
}

/// Texture crop for a stopped or window-closed suspended VM, reconstructed
/// from the same persisted preferences that seed a live [`CocoApp`].
fn definition_texture_uv(def: &crate::machine_def::MachineDef) -> egui::Rect {
    let settings = crate::display::TVSettings {
        scanline_pct: def.ui.tv_scanline,
        noise_pct: def.ui.tv_noise,
        overscan_pct: def.ui.tv_overscan,
    }
    .clamped();
    crate::display::texture_uv(def.display(), settings)
}

/// Paint one screen preview at `size`: the black placeholder under the
/// resolved `texture`, if any.
pub(super) fn draw_preview(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    texture: Option<&egui::TextureHandle>,
    uv: egui::Rect,
) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, THUMBNAIL_CORNER_RADIUS, THUMBNAIL_PLACEHOLDER_FILL);
    if let Some(texture) = texture {
        painter.image(texture.id(), rect, uv, egui::Color32::WHITE);
    }
}

impl ManagerApp {
    /// Snapshot `entries[index]`'s live VM screen into its artifact dir and
    /// invalidate the row's cached preview texture. No-op with no VM or no
    /// artifact root.
    pub(super) fn write_entry_thumbnail(&mut self, index: usize) {
        let Some(root) = &self.artifacts_root else {
            return;
        };
        let entry = &mut self.entries[index];
        let Some(vm) = entry.vm.as_mut() else {
            return;
        };
        // Runs the TV-processing chain here too, since the raw framebuffer
        // bypasses `upload_framebuffer_texture`'s.
        let frame = vm.presentation.snapshot(
            vm.display,
            vm.tv,
            vm.machine.fb_width as usize,
            &vm.machine.framebuffer,
        );
        if let Err(e) = super::write_thumbnail_png(
            &root.join(&entry.slug),
            frame.pixels,
            frame.width as u32,
            frame.height as u32,
        ) {
            tracing::warn!("thumbnail for '{}': {e}", entry.slug);
        }
        entry.thumbnail = None;
        entry.thumbnail_load_attempted = false;
    }

    /// Lazily load a suspended, window-closed entry's saved
    /// [`THUMBNAIL_FILE`] into a texture the first time its row draws.
    /// Failures leave the placeholder — the preview is a cache, never
    /// required state.
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
        // The display's own filtering (Monitor = NEAREST), not a hardcoded
        // LINEAR: the detail pane's big preview upscales this texture.
        entry.thumbnail = Some(ctx.load_texture(
            format!("thumbnail-{}", entry.slug),
            pixels,
            crate::display::texture_options(entry.def.display()),
        ));
    }
}
