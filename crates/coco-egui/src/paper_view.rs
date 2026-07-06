//! The "Printer Paper" window (`docs/printer-plan.md` T5): a scrollable,
//! auto-following view of the DMP-105's virtual fanfold paper, built on the
//! pure rasterizer in [`crate::paper_render`].
//!
//! Sink ownership: [`PaperWindow`] never attaches its own [`Dmp105Handle`] —
//! that handshake (which touches [`crate::CocoApp::print_capture_path`] and
//! `bus.bitbanger`, both owned by `CocoApp`) lives in
//! [`crate::CocoApp::toggle_paper_window`]. This window just holds whatever
//! handle it's given (via [`PaperWindow::handle`]) and keeps using it across
//! close/reopen — only [`PaperWindow::detach`] (called when print-file-
//! capture yanks the sink away) drops it.

use std::collections::{HashMap, HashSet};

use coco_core::dmp105::Dmp105Handle;
use coco_core::printer::Y_UNITS_PER_INCH;
use eframe::egui;

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
    pub handle: Option<Dmp105Handle>,
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
    }

    /// Draw the window if open and a handle is attached; a no-op frame
    /// otherwise (called unconditionally once per `update()`, like the
    /// other optional windows in `main.rs`).
    pub fn ui(&mut self, ctx: &egui::Context) {
        let Some(handle) = self.handle.clone() else {
            return;
        };
        if !self.open {
            return;
        }

        // Dirty invalidation: drop every cached page whose range intersects
        // what changed since the last poll, forcing a re-rasterize next
        // time that page is visible.
        if let Some((y0, y1)) = handle.take_dirty() {
            let y0_in = y0 as f32 / Y_UNITS_PER_INCH as f32;
            let y1_in = y1 as f32 / Y_UNITS_PER_INCH as f32;
            let first_page = (y0_in / PAGE_HEIGHT_IN).floor() as u32;
            let last_page = (y1_in / PAGE_HEIGHT_IN).floor() as u32;
            self.pages
                .retain(|&page, _| page < first_page || page > last_page);
        }

        // Green-bar toggle is a whole-page rendering choice: invalidate the
        // whole cache in one shot when it changes, rather than tracking it
        // per page.
        if self.green_bar != self.cached_green_bar {
            self.pages.clear();
            self.cached_green_bar = self.green_bar;
        }

        let extent = handle.paper_extent();
        let last_content_page = if extent.dot_count == 0 {
            0
        } else {
            let max_y_in = extent.max_y as f32 / Y_UNITS_PER_INCH as f32;
            (max_y_in / PAGE_HEIGHT_IN).floor() as u32
        };
        // Always at least one page shown, always at least one blank page
        // beyond the last printed line.
        let total_pages = last_content_page + 2;

        let mut open = self.open;
        egui::Window::new(crate::window_title(ctx, "Printer Paper"))
            .open(&mut open)
            .default_width(520.0)
            .default_height(700.0)
            .resizable(true)
            .frame(
                egui::Frame::window(&ctx.style()).fill(egui::Color32::from_rgba_unmultiplied(
                    paper_render::WINDOW_BG_COLOR[0],
                    paper_render::WINDOW_BG_COLOR[1],
                    paper_render::WINDOW_BG_COLOR[2],
                    paper_render::WINDOW_BG_COLOR[3],
                )),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{total_pages} page{}",
                        if total_pages == 1 { "" } else { "s" }
                    ));
                    ui.separator();
                    ui.checkbox(&mut self.green_bar, "Green bar");
                    ui.separator();
                    ui.add_enabled(false, egui::Button::new("Tear Off"))
                        .on_disabled_hover_text("coming in T6");
                });
                ui.separator();

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

                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show_viewport(ui, |ui, viewport| {
                        let keep_min = viewport.min.y - keep_margin_px;
                        let keep_max = viewport.max.y + keep_margin_px;

                        for page in 0..total_pages {
                            let y0 = page as f32 * page_size.y;
                            let y1 = y0 + page_size.y;
                            let (rect, _resp) =
                                ui.allocate_exact_size(page_size, egui::Sense::hover());

                            let in_keep_range = y1 >= keep_min && y0 <= keep_max;
                            if !in_keep_range {
                                continue;
                            }
                            keep_pages.insert(page);

                            let texture = self.pages.entry(page).or_insert_with(|| {
                                let img = paper_render::rasterize(
                                    &handle,
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
            });
        self.open = open;
    }
}
