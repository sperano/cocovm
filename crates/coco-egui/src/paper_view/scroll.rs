//! Virtualized layout and page-texture retention for printer paper.

use std::ops::Range;

use coco_core::dmp::DmpHandle;
use eframe::egui;

use super::PaperWindow;
use crate::paper_render::{self, PAGE_HEIGHT_IN, PAPER_WIDTH_IN, RASTER_DPI};

/// One page before and after the visible range hides rasterization latency
/// during ordinary scrolling. Retention is therefore bounded to the visible
/// page count plus two, independent of the roll's total page count.
const KEEP_MARGIN_PAGE_COUNT: u32 = 1;
/// Protect fit-width calculations during transient zero-width layout frames.
const MIN_PAPER_DISPLAY_WIDTH: f32 = 1.0;

#[derive(Clone, Copy, Debug)]
struct PageLayout {
    size: egui::Vec2,
    stride: f32,
    content_height: f32,
}

impl PageLayout {
    fn new(available_width: f32, page_spacing: f32, total_pages: u32) -> Self {
        let width = available_width.max(MIN_PAPER_DISPLAY_WIDTH);
        let height = width * PAGE_HEIGHT_IN / PAPER_WIDTH_IN;
        let stride = height + page_spacing;
        let content_height = stride * total_pages as f32 - page_spacing;
        Self {
            size: egui::vec2(width, height),
            stride,
            content_height,
        }
    }

    fn visible_pages(self, viewport: egui::Rect, total_pages: u32) -> Range<u32> {
        let start = (viewport.min.y / self.stride).floor().max(0.0) as u32;
        let end = (viewport.max.y / self.stride).ceil().max(1.0) as u32;
        start.min(total_pages)..end.min(total_pages)
    }

    fn current_page(self, viewport: egui::Rect, total_pages: u32) -> u32 {
        (viewport.min.y / self.stride)
            .floor()
            .max(0.0)
            .min(total_pages.saturating_sub(1) as f32) as u32
    }

    fn page_rect(self, origin: egui::Pos2, page: u32) -> egui::Rect {
        let top = origin + egui::vec2(0.0, page as f32 * self.stride);
        egui::Rect::from_min_size(top, self.size)
    }

    #[cfg(any(feature = "perf", test))]
    fn scroll_offset_for_page(self, page: u32) -> f32 {
        page as f32 * self.stride
    }
}

fn keep_page_range(visible: Range<u32>, total_pages: u32) -> Range<u32> {
    visible.start.saturating_sub(KEEP_MARGIN_PAGE_COUNT)
        ..visible
            .end
            .saturating_add(KEEP_MARGIN_PAGE_COUNT)
            .min(total_pages)
}

impl PaperWindow {
    /// Reserves the full roll height, but constructs and paints only pages in
    /// the viewport. The cache retains that range plus a one-page margin.
    pub(super) fn fanfold_scroll_area(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        handle: &DmpHandle,
        total_pages: u32,
    ) {
        let layout = PageLayout::new(
            ui.available_width(),
            ui.spacing().item_spacing.y,
            total_pages,
        );
        let scroll_to_top = std::mem::take(&mut self.scroll_to_top);
        let mut scroll_area = egui::ScrollArea::vertical().stick_to_bottom(!scroll_to_top);
        if scroll_to_top {
            scroll_area = scroll_area.vertical_scroll_offset(0.0);
        }
        #[cfg(feature = "perf")]
        let perf_request = self.perf_scroll_request.take();
        #[cfg(feature = "perf")]
        if let Some(request) = &perf_request {
            scroll_area = scroll_area
                .vertical_scroll_offset(layout.scroll_offset_for_page(request.target_page));
        }
        #[cfg(feature = "perf")]
        let draw_started = std::time::Instant::now();
        let mut keep_pages = 0..0;
        #[cfg(feature = "perf")]
        let mut visible_pages_rendered = 0..0;
        scroll_area.show_viewport(ui, |ui, viewport| {
            ui.set_height(layout.content_height);
            let visible_pages = layout.visible_pages(viewport, total_pages);
            #[cfg(feature = "perf")]
            {
                visible_pages_rendered = visible_pages.clone();
            }
            keep_pages = keep_page_range(visible_pages.clone(), total_pages);
            self.cache_page_range(ctx, handle, keep_pages.clone());
            self.paint_page_range(ui, layout, visible_pages.clone());
            self.current_page = layout.current_page(viewport, total_pages);
        });
        self.pages.retain(|page, _| keep_pages.contains(page));
        #[cfg(feature = "perf")]
        if let Some(request) = perf_request {
            self.perf_scroll_result = Some(super::PerfScrollResult {
                operation: request.operation,
                target_page: request.target_page,
                visible_pages: visible_pages_rendered,
                request_to_render: request.requested.elapsed(),
                draw_duration: draw_started.elapsed(),
                resident_items: self.pages.len(),
                resident_bytes: self
                    .pages
                    .values()
                    .map(egui::TextureHandle::byte_size)
                    .sum(),
            });
        }
    }

    fn cache_page_range(&mut self, ctx: &egui::Context, handle: &DmpHandle, pages: Range<u32>) {
        let green_bar = self.green_bar;
        for page in pages {
            self.pages.entry(page).or_insert_with(|| {
                let image = paper_render::rasterize(
                    handle,
                    page as f32 * PAGE_HEIGHT_IN,
                    PAGE_HEIGHT_IN,
                    RASTER_DPI,
                    green_bar,
                );
                let pixels = egui::ColorImage::from_rgba_unmultiplied(
                    [image.width as usize, image.height as usize],
                    &image.pixels,
                );
                ctx.load_texture(
                    format!("paper-page-{page}"),
                    pixels,
                    egui::TextureOptions::NEAREST,
                )
            });
        }
    }

    fn paint_page_range(&self, ui: &mut egui::Ui, layout: PageLayout, pages: Range<u32>) {
        let origin = ui.max_rect().min;
        for page in pages {
            let texture = self.pages.get(&page).expect("visible page is cached");
            let rect = layout.page_rect(origin, page);
            let sized = egui::load::SizedTexture::new(texture.id(), rect.size());
            ui.put(rect, egui::Image::new(sized));
        }
    }
}

#[cfg(test)]
#[path = "scroll_test.rs"]
mod tests;
