use super::*;

#[test]
fn initial_vm_window_height_tracks_toolbar_mode() {
    let labeled = vm_window_inner_size(false);
    let compact = vm_window_inner_size(true);
    let image_height = coco_core::raster::CANVAS_H as f32 * crate::SCALE;

    assert_eq!(labeled.x, compact.x);
    assert_eq!(
        labeled.y,
        image_height + crate::toolbar_height(false) + crate::STATUS_BAR_H
    );
    assert_eq!(
        compact.y,
        image_height + crate::toolbar_height(true) + crate::STATUS_BAR_H
    );
    assert_eq!(
        labeled.y - compact.y,
        crate::widgets::BUTTON_SIZE.y - crate::widgets::ICON_ONLY_BUTTON_SIZE.y
    );
}
