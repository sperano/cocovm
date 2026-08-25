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

#[test]
fn scales_offsets_and_size_into_the_display_rect() {
    // Origin off (0,0) so a forgotten `left_top()` offset would show.
    let display = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(1280.0, 480.0));
    let r = scale_active_rect(ACTIVE, FB_W, FB_H, display);
    assert_eq!(r.left_top(), egui::pos2(100.0 + 128.0, 50.0 + 50.0));
    assert_eq!(r.size(), egui::vec2(1024.0, 384.0));
}

#[test]
fn per_axis_scales_are_independent() {
    // 1× horizontally but 2× vertically — a mixed-up `sx`/`sy` fails here.
    let display = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 480.0));
    let r = scale_active_rect(ACTIVE, FB_W, FB_H, display);
    assert_eq!(r.left_top(), egui::pos2(64.0, 50.0));
    assert_eq!(r.size(), egui::vec2(512.0, 384.0));
}

#[test]
fn zero_size_framebuffer_falls_back_to_the_display_rect() {
    let display = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 480.0));
    assert_eq!(scale_active_rect(ACTIVE, 0, FB_H, display), display);
    assert_eq!(scale_active_rect(ACTIVE, FB_W, 0, display), display);
}
