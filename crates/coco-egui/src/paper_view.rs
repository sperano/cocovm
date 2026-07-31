//! The "Printer Paper" window (`docs/printer-plan.md` T5): a scrollable,
//! auto-following view of the DMP-105's virtual fanfold paper, built on the
//! pure rasterizer in [`crate::paper_render`].
//!
//! Shown as its own native OS window (an egui *immediate viewport*), so it
//! can be moved, resized, and monitored independently of the emulator
//! screen. Immediate (not deferred) because all paper state lives on the
//! main thread behind `Rc<RefCell<...>>` handles; on backends without
//! multi-window support egui reports `ViewportClass::Embedded` and the view
//! falls back to an in-viewport `egui::Window`.
//!
//! Sink ownership: [`PaperWindow`] never attaches its own [`DMP105Handle`] —
//! that handshake (which touches [`crate::CocoApp::print_capture_path`] and
//! `bus.bitbanger`, both owned by `CocoApp`) lives in
//! [`crate::CocoApp::toggle_paper_window`]. This window just holds whatever
//! handle it's given (via [`PaperWindow::handle`]) and keeps using it across
//! close/reopen — only [`PaperWindow::detach`] (called when print-file-
//! capture yanks the sink away) drops it.

use std::collections::{HashMap, HashSet};

use coco_core::dmp105::DMP105Handle;
use coco_core::printer::{PaperExtent, Y_UNITS_PER_INCH};
use eframe::egui;

use crate::paper_export;
use crate::paper_render::{self, PAGE_HEIGHT_IN, PAPER_WIDTH_IN, RASTER_DPI};

/// Fixed number of pages of look-ahead/behind kept rendered around the
/// visible viewport (see [`PaperWindow::ui`]'s viewport-limited texture
/// cache).
const KEEP_MARGIN_PAGES: f32 = 1.0;

#[derive(Default)]
pub struct PaperWindow {
    /// Whether the window is currently shown. Toggled by the View-menu
    /// checkbox (via [`crate::CocoApp::toggle_paper_window`]) or the
    /// window's own close button — neither touches `handle`.
    pub open: bool,
    /// The live DMP-105 handle this window reads from, if the bit-banger's
    /// sink is currently a DMP-105 (see the module doc comment on who
    /// attaches/detaches this).
    pub handle: Option<DMP105Handle>,
    /// Green-bar banding toggle, exposed in the window's header row.
    green_bar: bool,
    /// Whether `green_bar` changed since the cache below was last built —
    /// used to invalidate every cached page in one shot (banding is a
    /// whole-page rendering choice, not a per-region dirty range).
    cached_green_bar: bool,
    /// Rasterized page textures, keyed by page index
    /// (`floor(y_in / PAGE_HEIGHT_IN)`), viewport-limited: see
    /// [`PaperWindow::ui`].
    pages: HashMap<u32, egui::TextureHandle>,
    /// Page index topmost in the scroll viewport as of the last frame —
    /// what "Save Page as PNG…" exports (T6, "current viewport's page").
    current_page: u32,
    /// Set by the "Tear Off" button; the next frame renders a confirm/
    /// cancel dialog instead of acting immediately (T6).
    pending_tear_off: bool,
}

impl PaperWindow {
    pub fn new() -> Self {
        Self::default()
    }

    /// Detach the current sink handle: closes the window and drops every
    /// cached page texture (a fresh handle later means fresh, unrelated
    /// content). Called when print-file-capture takes over the bit-banger's
    /// sink out from under this window (`CocoApp::start_print_capture`'s
    /// symmetric rule).
    pub fn detach(&mut self) {
        self.handle = None;
        self.open = false;
        self.pages.clear();
        self.current_page = 0;
        self.pending_tear_off = false;
    }

    /// Re-bind this window after a snapshot restore replaces the live
    /// machine (`crate::CocoApp::load_state_from`). `Some(handle)` — the
    /// restored bit-banger's sink came back as a live DMP-105 — drops every
    /// cached page texture (rendered from the old machine's roll, unrelated
    /// to the restored one) but, unlike [`Self::detach`], leaves `open`
    /// alone, so a window the user had open stays open, now showing the
    /// restored paper. `None` — nothing was attached at save time, or it was
    /// a file capture (always restored as stopped) — has nothing to show, so
    /// this just delegates to `detach`.
    pub fn resync(&mut self, handle: Option<DMP105Handle>) {
        match handle {
            Some(handle) => {
                self.handle = Some(handle);
                self.pages.clear();
                self.current_page = 0;
                self.pending_tear_off = false;
            }
            None => self.detach(),
        }
    }

    /// Total pages currently spanning the roll for a given [`PaperExtent`]:
    /// always at least one page shown, always at least one blank page
    /// beyond the last printed line — the same "+2 pages" rule
    /// `examples/paper_preview.rs` uses. Pure function of the extent (not
    /// `&self`) so it's independently testable without a live handle.
    fn total_pages_for_extent(extent: PaperExtent) -> u32 {
        let last_content_page = if extent.dot_count == 0 {
            0
        } else {
            let max_y_in = extent.max_y as f32 / Y_UNITS_PER_INCH as f32;
            (max_y_in / PAGE_HEIGHT_IN).floor() as u32
        };
        last_content_page + 2
    }

    /// Tear off: discard the printed roll ([`DMP105Handle::tear_off`]) and
    /// reset every bit of view state that referred to the old roll's
    /// content, so the next `ui()` frame (or a export call) sees a fresh,
    /// blank single page rather than stale cached textures or an
    /// out-of-range `current_page`. No-op if no handle is attached.
    fn perform_tear_off(&mut self) {
        if let Some(handle) = &self.handle {
            handle.tear_off();
        }
        self.pages.clear();
        self.current_page = 0;
        self.pending_tear_off = false;
    }

    /// Draw the window if open and a handle is attached; a no-op frame
    /// otherwise (called unconditionally once per `update()`, like the
    /// other optional windows in `main.rs`). Returns an error message to
    /// surface (e.g. via `CocoApp::cart_error`, the app's shared error
    /// banner) if a PNG/PDF export failed.
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<String> {
        let handle = self.handle.clone()?;
        if !self.open {
            return None;
        }
        let mut error: Option<String> = None;

        self.invalidate_dirty_pages(&handle);
        self.invalidate_on_green_bar_change();

        let extent = handle.paper_extent();
        let total_pages = Self::total_pages_for_extent(extent);
        // Pages with any ink on them. `total_pages - 1` (dropping the
        // trailing blank page) only counts ink pages when there IS ink —
        // a blank roll still shows paper (`total_pages == 2`) but has zero
        // printed pages, and the tear-off confirm must not claim otherwise.
        let printed_pages = if extent.dot_count == 0 {
            0
        } else {
            total_pages - 1
        };

        // One stable ID so egui reuses the same native OS window across
        // frames instead of spawning a new one.
        let viewport_id = egui::ViewportId::from_hash_of("printer-paper");
        let builder = egui::ViewportBuilder::default()
            .with_title("Printer Paper")
            .with_inner_size([520.0, 700.0])
            .with_min_inner_size([280.0, 220.0]);
        ctx.show_viewport_immediate(viewport_id, builder, |ctx, class| {
            let fill = egui::Color32::from_rgba_unmultiplied(
                paper_render::WINDOW_BG_COLOR[0],
                paper_render::WINDOW_BG_COLOR[1],
                paper_render::WINDOW_BG_COLOR[2],
                paper_render::WINDOW_BG_COLOR[3],
            );
            if class == egui::ViewportClass::Embedded {
                // Backend without native multi-window support: fall back to
                // the embedded in-viewport window this view used before it
                // became a native one.
                let mut open = self.open;
                egui::Window::new(crate::window_title(ctx, "Printer Paper"))
                    .open(&mut open)
                    .default_width(520.0)
                    .default_height(700.0)
                    .resizable(true)
                    .frame(egui::Frame::window(&ctx.style()).fill(fill))
                    .show(ctx, |ui| {
                        self.contents(ui, &handle, total_pages, printed_pages, &mut error);
                    });
                self.open = open;
            } else {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default().fill(fill).inner_margin(6))
                    .show(ctx, |ui| {
                        self.contents(ui, &handle, total_pages, printed_pages, &mut error);
                    });
                // The OS close button: accept the close by not showing the
                // viewport next frame (mirrors the View-menu checkbox).
                if ctx.input(|i| i.viewport().close_requested()) {
                    self.open = false;
                }
            }

            if self.pending_tear_off {
                self.tear_off_dialog(ctx, printed_pages);
            }
        });

        error
    }

    /// Drop every cached page whose range intersects what changed since the
    /// last poll, forcing a re-rasterize next time that page is visible.
    fn invalidate_dirty_pages(&mut self, handle: &DMP105Handle) {
        let Some((y0, y1)) = handle.take_dirty() else {
            return;
        };
        // Widen by the rasterizer's dot-bleed pad: a dot near a page
        // edge also renders into the adjacent page's texture, which
        // must be invalidated too or it keeps a stale sliver at the
        // seam.
        let y0 = y0.saturating_sub(paper_render::DOT_QUERY_PAD_Y_UNITS);
        let y1 = y1.saturating_add(paper_render::DOT_QUERY_PAD_Y_UNITS);
        let y0_in = y0 as f32 / Y_UNITS_PER_INCH as f32;
        let y1_in = y1 as f32 / Y_UNITS_PER_INCH as f32;
        let first_page = (y0_in / PAGE_HEIGHT_IN).floor() as u32;
        let last_page = (y1_in / PAGE_HEIGHT_IN).floor() as u32;
        self.pages
            .retain(|&page, _| page < first_page || page > last_page);
    }

    /// Green-bar toggle is a whole-page rendering choice: invalidate the
    /// whole cache in one shot when it changes, rather than tracking it per
    /// page.
    fn invalidate_on_green_bar_change(&mut self) {
        if self.green_bar != self.cached_green_bar {
            self.pages.clear();
            self.cached_green_bar = self.green_bar;
        }
    }

    /// Confirm/cancel modal for "Tear Off", shown when [`Self::pending_tear_off`]
    /// is set. Same confirm/cancel modal pattern as `CocoApp`'s
    /// `pending_disk_action` dialog in `main.rs`.
    fn tear_off_dialog(&mut self, ctx: &egui::Context, printed_pages: u32) {
        let font = ctx.style().text_styles[&egui::TextStyle::Button].size;
        const DIALOG_MARGIN: i8 = 16;
        egui::Window::new(crate::window_title(ctx, "Tear off paper?"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .inner_margin(DIALOG_MARGIN)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "Tear off and discard {printed_pages} page{} of printed output?",
                                if printed_pages == 1 { "" } else { "s" }
                            ))
                            .size(font),
                        );
                        ui.add_space(DIALOG_MARGIN as f32);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().button_padding = egui::vec2(12.0, 6.0);
                            if ui.button("Tear Off").clicked() {
                                self.perform_tear_off();
                            }
                            if ui.button("Cancel").clicked() {
                                self.pending_tear_off = false;
                            }
                        });
                    });
            });
    }

    /// Everything inside the paper window: the header row (page count,
    /// green bar, Export menu, Tear Off) and the scrolling fanfold view.
    /// Shared verbatim between the native-viewport and embedded-fallback
    /// paths of [`Self::ui`].
    fn contents(
        &mut self,
        ui: &mut egui::Ui,
        handle: &DMP105Handle,
        total_pages: u32,
        printed_pages: u32,
        error: &mut Option<String>,
    ) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.label(format!(
                "{total_pages} page{}",
                if total_pages == 1 { "" } else { "s" }
            ));
            ui.separator();
            ui.checkbox(&mut self.green_bar, "Green bar");
            ui.separator();
            self.export_menu(ui, handle, total_pages, error);
            ui.separator();
            // Nothing to tear off a blank roll.
            if ui
                .add_enabled(printed_pages > 0, egui::Button::new("Tear Off"))
                .clicked()
            {
                self.pending_tear_off = true;
            }
        });
        ui.separator();

        self.fanfold_scroll_area(ui, &ctx, handle, total_pages);
    }

    /// The header row's "Export" menu button: PNG (current page or whole
    /// roll) and PDF (fanfold with tractor strips, or trimmed 8.5x11)
    /// exports (T6).
    fn export_menu(
        &mut self,
        ui: &mut egui::Ui,
        handle: &DMP105Handle,
        total_pages: u32,
        error: &mut Option<String>,
    ) {
        ui.menu_button("Export", |ui| {
            if ui.button("Save Page as PNG…").clicked() {
                ui.close();
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("PNG image", &["png"])
                    .set_file_name(format!("page-{}.png", self.current_page + 1))
                    .save_file()
                {
                    let img = paper_render::rasterize(
                        handle,
                        self.current_page as f32 * PAGE_HEIGHT_IN,
                        PAGE_HEIGHT_IN,
                        RASTER_DPI,
                        self.green_bar,
                    );
                    if let Err(e) = paper_export::save_png(&img, &path) {
                        *error = Some(e);
                    }
                }
            }
            if ui.button("Save Roll as PNG…").clicked() {
                ui.close();
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("PNG image", &["png"])
                    .set_file_name("roll.png")
                    .save_file()
                {
                    // The whole printed roll plus the trailing
                    // blank page, so the image ends on a page
                    // boundary (T6).
                    let img = paper_render::rasterize(
                        handle,
                        0.0,
                        total_pages as f32 * PAGE_HEIGHT_IN,
                        RASTER_DPI,
                        self.green_bar,
                    );
                    if let Err(e) = paper_export::save_png(&img, &path) {
                        *error = Some(e);
                    }
                }
            }
            ui.separator();
            if ui
                .button("Save as PDF (fanfold, with tractor strips)…")
                .clicked()
            {
                ui.close();
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("PDF document", &["pdf"])
                    .set_file_name("printout.pdf")
                    .save_file()
                {
                    let pages: Vec<_> = (0..total_pages)
                        .map(|page| {
                            paper_render::rasterize(
                                handle,
                                page as f32 * PAGE_HEIGHT_IN,
                                PAGE_HEIGHT_IN,
                                RASTER_DPI,
                                self.green_bar,
                            )
                        })
                        .collect();
                    if let Err(e) = paper_export::save_pdf(&pages, RASTER_DPI, &path) {
                        *error = Some(e);
                    }
                }
            }
            if ui.button("Save as PDF (trimmed, 8.5×11)…").clicked() {
                ui.close();
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("PDF document", &["pdf"])
                    .set_file_name("printout-trimmed.pdf")
                    .save_file()
                {
                    let pages: Vec<_> = (0..total_pages)
                        .map(|page| {
                            let img = paper_render::rasterize(
                                handle,
                                page as f32 * PAGE_HEIGHT_IN,
                                PAGE_HEIGHT_IN,
                                RASTER_DPI,
                                self.green_bar,
                            );
                            paper_export::crop_to_trimmed_width(&img, RASTER_DPI)
                        })
                        .collect();
                    if let Err(e) = paper_export::save_pdf(&pages, RASTER_DPI, &path) {
                        *error = Some(e);
                    }
                }
            }
        });
    }

    /// The scrolling fanfold view: allocates one rect per page at fit-width
    /// scale, lazily rasterizing/uploading a texture for any page that
    /// enters the viewport-plus-[`KEEP_MARGIN_PAGES`] keep range, and
    /// evicting cached textures for pages that fall back outside it.
    fn fanfold_scroll_area(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        handle: &DMP105Handle,
        total_pages: u32,
    ) {
        // Fit-width scaling: no horizontal scrolling at default zoom.
        let scale = ui.available_width() / (PAPER_WIDTH_IN * RASTER_DPI);
        let page_size = egui::vec2(
            PAPER_WIDTH_IN * RASTER_DPI * scale,
            PAGE_HEIGHT_IN * RASTER_DPI * scale,
        );
        let keep_margin_px = page_size.y * KEEP_MARGIN_PAGES;
        // Copied out before the loop so the per-page texture-building
        // closure below doesn't need to re-borrow `self` while
        // `self.pages.entry(...)` already holds a mutable borrow of
        // the `pages` field.
        let green_bar = self.green_bar;

        let mut keep_pages: HashSet<u32> = HashSet::new();
        // Page topmost in the viewport this frame — see
        // `current_page`'s field doc comment.
        let mut current_page_local = self.current_page;

        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show_viewport(ui, |ui, viewport| {
                let keep_min = viewport.min.y - keep_margin_px;
                let keep_max = viewport.max.y + keep_margin_px;
                current_page_local = (viewport.min.y / page_size.y)
                    .floor()
                    .clamp(0.0, (total_pages - 1) as f32)
                    as u32;

                for page in 0..total_pages {
                    let y0 = page as f32 * page_size.y;
                    let y1 = y0 + page_size.y;
                    let (rect, _resp) = ui.allocate_exact_size(page_size, egui::Sense::hover());

                    let in_keep_range = y1 >= keep_min && y0 <= keep_max;
                    if !in_keep_range {
                        continue;
                    }
                    keep_pages.insert(page);

                    let texture = self.pages.entry(page).or_insert_with(|| {
                        let img = paper_render::rasterize(
                            handle,
                            page as f32 * PAGE_HEIGHT_IN,
                            PAGE_HEIGHT_IN,
                            RASTER_DPI,
                            green_bar,
                        );
                        let color_image = egui::ColorImage::from_rgba_unmultiplied(
                            [img.width as usize, img.height as usize],
                            &img.pixels,
                        );
                        ctx.load_texture(
                            format!("paper-page-{page}"),
                            color_image,
                            egui::TextureOptions::NEAREST,
                        )
                    });
                    let sized = egui::load::SizedTexture::new(texture.id(), rect.size());
                    ui.put(rect, egui::Image::new(sized));
                }
            });

        // Evict cached textures outside the keep-set.
        self.pages.retain(|page, _| keep_pages.contains(page));
        self.current_page = current_page_local;
    }
}

#[cfg(test)]
#[path = "paper_view_test.rs"]
mod tests;
