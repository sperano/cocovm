//! Toolbar tile and panel geometry in both modes, including live switches.

use egui_kittest::kittest::Queryable;

use crate::*;

use super::harness::*;

const GEOMETRY_TOLERANCE: f32 = 1.0;

fn near(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= GEOMETRY_TOLERANCE,
        "{actual} != {expected}"
    );
}

#[test]
fn manager_toolbar_resizes_in_both_directions() {
    let mut harness = manager_harness(None, Vec::new());
    let labeled_sort_y = harness.get_by_label("Sort by").rect().min.y;
    let labeled_button = harness.get_by_label("New").rect();
    near(
        harness.get_by_label("New").rect().height(),
        widgets::BUTTON_SIZE.y,
    );

    harness.state_mut().toolbar_icons_only = true;
    harness.step();
    let compact_button = harness.get_by_label("New").rect();
    assert!(compact_button.width() < labeled_button.width());
    assert!(compact_button.height() < labeled_button.height());
    near(compact_button.height(), widgets::ICON_ONLY_BUTTON_SIZE.y);
    near(
        labeled_sort_y - harness.get_by_label("Sort by").rect().min.y,
        toolbar_height(false) - toolbar_height(true),
    );

    harness.state_mut().toolbar_icons_only = false;
    harness.step();
    near(
        harness.get_by_label("New").rect().height(),
        widgets::BUTTON_SIZE.y,
    );
    near(harness.get_by_label("Sort by").rect().min.y, labeled_sort_y);
}

#[test]
fn vm_toolbar_resizes_in_both_directions() {
    let mut harness = boot_harness();
    let labeled_display_y = harness.state().display_rect.min.y;
    near(
        harness.get_by_label("Stop").rect().height(),
        widgets::BUTTON_SIZE.y,
    );

    harness.state_mut().toolbar_icons_only = true;
    harness.step();
    near(
        harness.get_by_label("Stop").rect().height(),
        widgets::ICON_ONLY_BUTTON_SIZE.y,
    );
    near(
        labeled_display_y - harness.state().display_rect.min.y,
        toolbar_height(false) - toolbar_height(true),
    );

    harness.state_mut().toolbar_icons_only = false;
    harness.step();
    near(
        harness.get_by_label("Stop").rect().height(),
        widgets::BUTTON_SIZE.y,
    );
    near(harness.state().display_rect.min.y, labeled_display_y);
}
