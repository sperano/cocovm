use super::*;

const TOTAL_PAGES: u32 = 2_000;
const PAGE_SPACING: f32 = 8.0;
const VIEW_WIDTH: f32 = 400.0;

#[test]
fn visible_range_depends_on_viewport_not_roll_length() {
    let layout = PageLayout::new(VIEW_WIDTH, PAGE_SPACING, TOTAL_PAGES);
    let first_page = 1_500;
    let top = first_page as f32 * layout.stride;
    let viewport = egui::Rect::from_min_size(
        egui::pos2(0.0, top),
        egui::vec2(VIEW_WIDTH, layout.size.y * 2.2),
    );

    assert_eq!(layout.visible_pages(viewport, TOTAL_PAGES), 1_500..1_503);
    assert_eq!(layout.current_page(viewport, TOTAL_PAGES), 1_500);
}

#[test]
fn keep_range_adds_one_page_each_side_and_clamps() {
    assert_eq!(keep_page_range(10..13, TOTAL_PAGES), 9..14);
    assert_eq!(keep_page_range(0..2, TOTAL_PAGES), 0..3);
    assert_eq!(
        keep_page_range(1_998..TOTAL_PAGES, TOTAL_PAGES),
        1_997..TOTAL_PAGES
    );
}

#[test]
fn content_height_reserves_every_page_without_constructing_it() {
    let layout = PageLayout::new(VIEW_WIDTH, PAGE_SPACING, TOTAL_PAGES);
    let expected = layout.stride * TOTAL_PAGES as f32 - PAGE_SPACING;

    assert_eq!(layout.content_height, expected);
}

#[test]
fn page_rect_uses_stride_and_preserves_the_gap() {
    let layout = PageLayout::new(VIEW_WIDTH, PAGE_SPACING, TOTAL_PAGES);
    let first = layout.page_rect(egui::Pos2::ZERO, 0);
    let second = layout.page_rect(egui::Pos2::ZERO, 1);

    assert_eq!(second.top() - first.bottom(), PAGE_SPACING);
}

#[test]
fn performance_scroll_offset_targets_requested_page() {
    let layout = PageLayout::new(VIEW_WIDTH, PAGE_SPACING, TOTAL_PAGES);

    assert_eq!(layout.scroll_offset_for_page(0), 0.0);
    assert_eq!(
        layout.scroll_offset_for_page(TOTAL_PAGES / 2),
        layout.stride * (TOTAL_PAGES / 2) as f32
    );
    assert_eq!(
        layout.scroll_offset_for_page(TOTAL_PAGES - 1),
        layout.stride * (TOTAL_PAGES - 1) as f32
    );
}
