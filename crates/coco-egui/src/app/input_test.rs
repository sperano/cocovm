//! Coverage for `scale_active_rect` — the framebuffer→screen mapping behind
//! `CocoApp::active_screen_rect`. Pure geometry, so no `CocoApp` needed.

use coco_core::ActiveRect;
use eframe::egui;

use super::scale_active_rect;

/// The CoCo 3 canvas dims (`raster::CANVAS_W`/`CANVAS_H`) the fixtures scale
/// from.
const FB_W: u32 = 640;
const FB_H: u32 = 240;
/// A non-wide, LPF-192 active rect: 512×192 at (64, 25).
const ACTIVE: ActiveRect = ActiveRect {
    x: 64,
    y: 25,
    width: 512,
    height: 192,
};

fn full_uv() -> egui::Rect {
    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0))
}

#[test]
fn scales_offsets_and_size_into_the_display_rect() {
    // Origin off (0,0) so a forgotten `left_top()` offset would show.
    let display = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(1280.0, 480.0));
    let r = scale_active_rect(ACTIVE, FB_W, FB_H, display, full_uv());
    assert_eq!(r.left_top(), egui::pos2(100.0 + 128.0, 50.0 + 50.0));
    assert_eq!(r.size(), egui::vec2(1024.0, 384.0));
}

#[test]
fn per_axis_scales_are_independent() {
    // 1× horizontally but 2× vertically — a mixed-up `sx`/`sy` fails here.
    let display = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 480.0));
    let r = scale_active_rect(ACTIVE, FB_W, FB_H, display, full_uv());
    assert_eq!(r.left_top(), egui::pos2(64.0, 50.0));
    assert_eq!(r.size(), egui::vec2(512.0, 384.0));
}

#[test]
fn zero_size_framebuffer_falls_back_to_the_display_rect() {
    let display = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 480.0));
    assert_eq!(
        scale_active_rect(ACTIVE, 0, FB_H, display, full_uv()),
        display
    );
    assert_eq!(
        scale_active_rect(ACTIVE, FB_W, 0, display, full_uv()),
        display
    );
}

#[test]
fn crop_maps_only_the_visible_source_span_onto_the_display() {
    const CROP: f32 = 0.05;
    const EPSILON: f32 = 0.001;
    const EXPECTED_LEFT: f32 = 171.111_11;
    const EXPECTED_TOP: f32 = 78.888_885;
    const EXPECTED_WIDTH: f32 = 1_137.777_8;
    const EXPECTED_HEIGHT: f32 = 426.666_66;

    let display = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(1280.0, 480.0));
    let uv = egui::Rect::from_min_max(egui::pos2(CROP, CROP), egui::pos2(1.0 - CROP, 1.0 - CROP));
    let r = scale_active_rect(ACTIVE, FB_W, FB_H, display, uv);

    assert!((r.left() - EXPECTED_LEFT).abs() < EPSILON);
    assert!((r.top() - EXPECTED_TOP).abs() < EPSILON);
    assert!((r.width() - EXPECTED_WIDTH).abs() < EPSILON);
    assert!((r.height() - EXPECTED_HEIGHT).abs() < EPSILON);
}
