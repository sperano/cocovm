//! Preview-thumbnail cache management: writing a VM's framebuffer out to
//! its artifact directory at suspend time, and lazily loading a suspended
//! machine's saved preview into a texture for the list row. See
//! [`super::write_thumbnail_png`] for the on-disk format/write contract.
//! Also the preview-drawing helpers shared by the list rows and the detail
//! pane's big screen preview.

use std::ops::Range;
use std::path::Path;

use eframe::egui;

use super::{
    CocoApp, MachineEntry, ManagerApp, THUMBNAIL_CORNER_RADIUS, THUMBNAIL_FILE,
    THUMBNAIL_PLACEHOLDER_FILL,
};

/// Saved previews are produced from CoCoVM's bounded framebuffer pipeline.
/// Rejecting larger replacements keeps one synchronous decode from allocating
/// an arbitrary image before the cache policy can account for it.
const THUMBNAIL_MAX_WIDTH: u32 = 640;
const THUMBNAIL_MAX_HEIGHT: u32 = 480;
/// Decoder working storage, including the 1.17 MiB maximum RGBA output.
const THUMBNAIL_DECODE_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Approximate retained GPU allocation for saved RGBA8 previews. At the
/// maximum 640 x 480 size this holds 27 textures (31.6 MiB). Live VM
/// framebuffer textures are shared with the VM and don't count toward it.
const THUMBNAIL_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;
/// Maximum synchronous PNG decodes in one manager update. Visible rows are
/// serviced before the near-range prefetch. Four maximum-size previews keep
/// transient decode/output storage bounded while the cold cache warms over
/// subsequent frames.
pub(super) const THUMBNAIL_LOADS_PER_UPDATE: usize = 4;

impl MachineEntry {
    /// Drops all saved-preview cache state after the backing PNG changes or
    /// stops belonging to the machine.
    pub(super) fn invalidate_thumbnail(&mut self) {
        self.thumbnail = None;
        self.thumbnail_load_attempted = false;
        self.thumbnail_known_available = false;
        self.thumbnail_last_used = 0;
    }
}

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
        entry.invalidate_thumbnail();
    }

    /// Prepares saved previews in the near-visible range, then marks visible
    /// previews newer so deterministic least-recently-used eviction favors
    /// what this frame paints. Missing and failed loads remain remembered.
    pub(super) fn prepare_row_thumbnails(
        &mut self,
        ctx: &egui::Context,
        near_rows: Range<usize>,
        visible_rows: Range<usize>,
    ) {
        let near_stamp = self.next_thumbnail_use_stamp();
        let visible_stamp = self.next_thumbnail_use_stamp();
        self.touch_resident_thumbnails(near_rows.clone(), near_stamp);
        self.touch_resident_thumbnails(visible_rows.clone(), visible_stamp);
        let mut loads_remaining = std::mem::take(&mut self.thumbnail_loads_remaining);
        self.load_thumbnail_range(ctx, visible_rows, visible_stamp, &mut loads_remaining);
        self.load_thumbnail_range(ctx, near_rows, near_stamp, &mut loads_remaining);
        self.thumbnail_loads_remaining = loads_remaining;
        self.enforce_thumbnail_cache_budget(THUMBNAIL_CACHE_MAX_BYTES);
    }

    /// Prepares the selected machine's large detail preview even when its
    /// list row is offscreen.
    pub(super) fn prepare_detail_thumbnail(&mut self, ctx: &egui::Context, index: usize) {
        let stamp = self.next_thumbnail_use_stamp();
        let rows = index..index.saturating_add(1);
        self.touch_resident_thumbnails(rows.clone(), stamp);
        let mut loads_remaining = std::mem::take(&mut self.thumbnail_loads_remaining);
        self.load_thumbnail_range(ctx, rows, stamp, &mut loads_remaining);
        self.thumbnail_loads_remaining = loads_remaining;
        self.enforce_thumbnail_cache_budget(THUMBNAIL_CACHE_MAX_BYTES);
    }

    fn touch_resident_thumbnails(&mut self, rows: Range<usize>, stamp: u64) {
        for index in rows {
            let entry = &mut self.entries[index];
            if entry.vm.is_none() && entry.suspended && entry.thumbnail.is_some() {
                entry.thumbnail_last_used = stamp;
            }
        }
    }

    fn load_thumbnail_range(
        &mut self,
        ctx: &egui::Context,
        rows: Range<usize>,
        stamp: u64,
        loads_remaining: &mut usize,
    ) {
        for index in rows {
            if *loads_remaining == 0 {
                break;
            }
            if self.thumbnail_needs_load(index) {
                *loads_remaining -= 1;
                self.load_thumbnail(ctx, index, stamp);
            }
        }
    }

    fn thumbnail_needs_load(&self, index: usize) -> bool {
        let entry = &self.entries[index];
        entry.vm.is_none()
            && entry.suspended
            && entry.thumbnail.is_none()
            && (!entry.thumbnail_load_attempted || entry.thumbnail_known_available)
    }

    fn next_thumbnail_use_stamp(&mut self) -> u64 {
        if self.thumbnail_use_clock == u64::MAX {
            for entry in &mut self.entries {
                entry.thumbnail_last_used = 0;
            }
            self.thumbnail_use_clock = 0;
        }
        self.thumbnail_use_clock += 1;
        self.thumbnail_use_clock
    }

    /// Loads or reloads one eligible saved preview. A failed first load is a
    /// negative-cache entry; an evicted successful load remains reloadable.
    fn load_thumbnail(&mut self, ctx: &egui::Context, index: usize, stamp: u64) {
        let entry = &mut self.entries[index];
        debug_assert!(entry.vm.is_none() && entry.suspended && entry.thumbnail.is_none());
        entry.thumbnail_load_attempted = true;
        let Some(root) = &self.artifacts_root else {
            return;
        };
        let path = root.join(&entry.slug).join(THUMBNAIL_FILE);
        let Ok(image) = load_thumbnail(&path) else {
            entry.thumbnail_known_available = false;
            return;
        };
        let image = image.to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        // The display's own filtering (Monitor = NEAREST), not a hardcoded
        // LINEAR: the detail pane's big preview upscales this texture.
        let texture = ctx.load_texture(
            format!("thumbnail-{}", entry.slug),
            pixels,
            crate::display::texture_options(entry.def.display()),
        );
        entry.thumbnail_known_available = true;
        entry.thumbnail_last_used = stamp;
        entry.thumbnail = Some(texture);
    }

    /// Enforces the saved-preview GPU estimate with deterministic LRU order.
    /// Equal use stamps evict lower row indices first. Eviction drops only the
    /// texture, preserving successful-load knowledge for later reload.
    fn enforce_thumbnail_cache_budget(&mut self, max_bytes: usize) {
        let mut resident: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                entry
                    .thumbnail
                    .as_ref()
                    .map(|texture| (entry.thumbnail_last_used, index, texture.byte_size()))
            })
            .collect();
        let mut total_bytes = resident.iter().map(|item| item.2).sum::<usize>();
        if total_bytes <= max_bytes {
            return;
        }
        resident.sort_unstable_by_key(|&(stamp, index, _)| (stamp, index));
        for (_, index, bytes) in resident {
            if total_bytes <= max_bytes {
                break;
            }
            self.entries[index].thumbnail = None;
            total_bytes = total_bytes.saturating_sub(bytes);
        }
    }
}

fn load_thumbnail(path: &Path) -> image::ImageResult<image::DynamicImage> {
    let mut reader = image::ImageReader::open(path)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(THUMBNAIL_MAX_WIDTH);
    limits.max_image_height = Some(THUMBNAIL_MAX_HEIGHT);
    limits.max_alloc = Some(THUMBNAIL_DECODE_MAX_BYTES);
    reader.limits(limits);
    reader.decode()
}

#[cfg(test)]
#[path = "thumbnails_test.rs"]
mod tests;
