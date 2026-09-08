use super::*;

#[test]
fn sparse_printer_fixture_reaches_every_requested_page() {
    const MAX_DOTS_PER_PAGE: usize = 1000;
    let mut handle = coco_core::dmp105::DMP105Handle::new();
    fill_printer(&mut handle);
    let extent = handle.paper_extent();
    let page_height =
        (crate::paper_render::PAGE_HEIGHT_IN * coco_core::printer::Y_UNITS_PER_INCH as f32) as u32;
    assert_eq!(extent.max_y / page_height + 1, PRINTER_PAGE_COUNT as u32);
    assert!(extent.dot_count > PRINTER_PAGE_COUNT);
    assert!(extent.dot_count < PRINTER_PAGE_COUNT * MAX_DOTS_PER_PAGE);
}
