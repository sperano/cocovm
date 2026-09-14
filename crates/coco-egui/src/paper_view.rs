//! The "Printer Paper" window: a scrollable,
//! auto-following view of the DMP printer's virtual fanfold paper, built on the
//! pure rasterizer in [`crate::paper_render`].
//!
//! Shown as its own native OS window (an egui *immediate viewport*), so it
//! can be moved, resized, and monitored independently of the emulator
//! screen. Immediate (not deferred) because all paper state lives on the
//! main thread behind `Rc<RefCell<...>>` handles; on backends without
//! multi-window support egui reports `ViewportClass::Embedded` and the view
//! falls back to an in-viewport `egui::Window`.
//!
//! Sink ownership: [`PaperWindow`] never attaches its own [`DmpHandle`] —
//! that handshake (which touches [`crate::CocoApp::print_capture_path`] and
//! `bus.bitbanger`, both owned by `CocoApp`) lives in
//! [`crate::CocoApp::toggle_paper_window`]. This window holds whatever
//! handle it's given (through [`PaperWindow::handle`]) and keeps using it across
//! close/reopen — only [`PaperWindow::detach`] (called when print-file-
//! capture yanks the sink away) drops it.

use std::collections::HashMap;

use coco_core::dmp::DmpHandle;
use coco_core::printer::PaperExtent;
use eframe::egui;

use crate::paper_render;

mod export;
mod scroll;

#[cfg(feature = "perf")]
struct PerfScrollRequest {
    operation: u64,
    target_page: u32,
    requested: std::time::Instant,
}

#[cfg(feature = "perf")]
pub(crate) struct PerfScrollResult {
    pub(crate) operation: u64,
    pub(crate) target_page: u32,
    pub(crate) visible_pages: std::ops::Range<u32>,
    pub(crate) request_to_render: std::time::Duration,
    pub(crate) draw_duration: std::time::Duration,
    pub(crate) resident_items: usize,
    pub(crate) resident_bytes: usize,
}

#[derive(Default)]
pub struct PaperWindow {
    /// Whether the window is currently shown. Toggled by the View-menu
    /// checkbox (through [`crate::CocoApp::toggle_paper_window`]) or the
    /// window's own close button — neither touches `handle`.
    pub open: bool,
    /// The live DMP printer handle this window reads from, if the bit-banger's
    /// sink is currently a DMP printer (see the module doc comment on who
    /// attaches/detaches this).
    pub handle: Option<DmpHandle>,
    /// Green-bar banding toggle, exposed in the window's header row.
    green_bar: bool,
    /// Whether `green_bar` changed since the cache was last built —
    /// used to invalidate every cached page in one shot (banding is a
    /// whole-page rendering choice, not a per-region dirty range).
    cached_green_bar: bool,
    /// Rasterized page textures, keyed by page index
    /// ([`paper_render::page_of_units`]), viewport-limited: see
    /// [`PaperWindow::ui`].
    pages: HashMap<u32, egui::TextureHandle>,
    /// Page index topmost in the scroll viewport as of the last frame —
    /// what "Save Page as PNG…" exports (T6, "current viewport's page").
    current_page: u32,
    /// Set by the "Tear Off" button; the next frame renders a confirm/
    /// cancel dialog instead of acting immediately (T6).
    pending_tear_off: bool,
    /// Whether the previous frame drew the window; a rising edge means it just (re)opened.
    shown_last_frame: bool,
    /// One-shot request consumed by the scroll area: start at the roll's top instead of
    /// egui's default of sticking to the end.
    scroll_to_top: bool,
    export: export::Controller,
    #[cfg(feature = "perf")]
    perf_scroll_request: Option<PerfScrollRequest>,
    #[cfg(feature = "perf")]
    perf_scroll_result: Option<PerfScrollResult>,
}

impl PaperWindow {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "perf")]
    pub(crate) fn request_perf_scroll(&mut self, operation: u64, target_page: u32) {
        self.perf_scroll_request = Some(PerfScrollRequest {
            operation,
            target_page,
            requested: std::time::Instant::now(),
        });
    }

    #[cfg(feature = "perf")]
    pub(crate) fn take_perf_scroll_result(&mut self) -> Option<PerfScrollResult> {
        self.perf_scroll_result.take()
    }

    /// Detaches the current sink handle: closes the window and drops every cached page texture
    /// (a fresh handle means fresh, unrelated content). Called when print-file-capture takes
    /// the bit-banger's sink out from under this window.
    pub fn detach(&mut self) {
        self.handle = None;
        self.open = false;
        self.pages.clear();
        self.current_page = 0;
        self.pending_tear_off = false;
    }

    /// Re-binds this window after a snapshot restore replaces the live machine. `Some(handle)`
    /// drops every cached page texture but leaves `open` alone; `None` delegates to
    /// [`Self::detach`].
    pub fn resync(&mut self, handle: Option<DmpHandle>) {
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

    /// Total pages currently spanning the roll for a given [`PaperExtent`]: always at least one
    /// page shown, plus one blank page beyond the last printed line. Pure function so it's
    /// testable without a live handle.
    fn total_pages_for_extent(extent: PaperExtent) -> u32 {
        let last_content_page = if extent.dot_count == 0 {
            0
        } else {
            paper_render::page_of_units(extent.max_y)
        };
        last_content_page + 2
    }

    /// Tears off: discards the printed roll ([`DmpHandle::tear_off`]) and resets every view
    /// state that referred to the old roll, so the next frame sees a fresh blank page. No-op if
    /// no handle is attached.
    fn perform_tear_off(&mut self) {
        if let Some(handle) = &self.handle {
            handle.tear_off();
        }
        self.pages.clear();
        self.current_page = 0;
        self.pending_tear_off = false;
    }

    /// Records whether this frame draws the window; returns `true` on the frame it (re)opens.
    fn note_shown(&mut self, shown: bool) -> bool {
        let was_shown = std::mem::replace(&mut self.shown_last_frame, shown);
        shown && !was_shown
    }

    /// Draws the window if open and a handle is attached; a no-op frame otherwise. Returns an
    /// error message to surface if a PNG/PDF export failed.
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<String> {
        let mut error = self.export.poll();
        if self.export.is_active() {
            ctx.request_repaint_after(export::EXPORT_REPAINT_INTERVAL);
        }
        let shown = self.open && self.handle.is_some();
        if self.note_shown(shown) {
            self.scroll_to_top = true;
        }
        let Some(handle) = self.handle.clone().filter(|_| shown) else {
            return error;
        };

        self.invalidate_dirty_pages(&handle);
        self.invalidate_on_green_bar_change();

        let extent = handle.paper_extent();
        let total_pages = Self::total_pages_for_extent(extent);
        // Pages with ink: `total_pages - 1` only counts them when there IS ink — a blank roll
        // still shows paper but has zero printed pages.
        let printed_pages = if extent.dot_count == 0 {
            0
        } else {
            total_pages - 1
        };

        // One stable ID so egui reuses the same native OS window across frames.
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
                // Backend without native multi-window support: fall back to the embedded
                // in-viewport window.
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
                // The OS close button: accept the close by not showing the viewport next frame.
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
    fn invalidate_dirty_pages(&mut self, handle: &DmpHandle) {
        let Some((y0, y1)) = handle.take_dirty() else {
            return;
        };
        // Widen by the rasterizer's dot-bleed pad: a dot near a page edge also renders into the
        // adjacent page's texture.
        let first_page =
            paper_render::page_of_units(y0.saturating_sub(paper_render::DOT_QUERY_PAD_Y_UNITS));
        let last_page =
            paper_render::page_of_units(y1.saturating_add(paper_render::DOT_QUERY_PAD_Y_UNITS));
        self.pages
            .retain(|&page, _| page < first_page || page > last_page);
    }

    /// Green-bar toggle is a whole-page rendering choice: invalidates the whole cache in one
    /// shot when it changes, rather than tracking it per page.
    fn invalidate_on_green_bar_change(&mut self) {
        if self.green_bar != self.cached_green_bar {
            self.pages.clear();
            self.cached_green_bar = self.green_bar;
        }
    }

    /// Confirm/cancel modal for "Tear Off", shown when [`Self::pending_tear_off`] is set.
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

    /// Everything inside the paper window: the header row (page count, green bar, Export menu,
    /// Tear Off) and the scrolling fanfold view. Shared between the native-viewport and
    /// embedded-fallback paths of [`Self::ui`].
    fn contents(
        &mut self,
        ui: &mut egui::Ui,
        handle: &DmpHandle,
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
            if let Some(export_error) = export::menu(
                ui,
                &mut self.export,
                handle,
                self.current_page,
                total_pages,
                self.green_bar,
            ) {
                *error = Some(export_error);
            }
            self.export.status(ui);
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
}

#[cfg(test)]
#[path = "paper_view_test.rs"]
mod tests;
