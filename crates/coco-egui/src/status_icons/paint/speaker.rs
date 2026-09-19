use eframe::egui;

use super::{begin_icon, icon_size};

/// Status-bar speaker icon size.
const SPEAKER_ICON_SIZE: egui::Vec2 = icon_size(16.0, 13.0);
/// Width of the speaker's rectangular back section.
const SPEAKER_BACK_WIDTH: f32 = SPEAKER_ICON_SIZE.x * 0.20;
/// Width of the speaker cone.
const SPEAKER_CONE_WIDTH: f32 = SPEAKER_ICON_SIZE.x * 0.25;
/// Half-height of the speaker cone.
const SPEAKER_CONE_HALF_HEIGHT: f32 = SPEAKER_ICON_SIZE.y * 0.36;
/// Radius of the inner sound wave.
const INNER_WAVE_RADIUS: f32 = SPEAKER_ICON_SIZE.y * 0.30;
/// Radius of the outer sound wave.
const OUTER_WAVE_RADIUS: f32 = SPEAKER_ICON_SIZE.y * 0.48;
/// Half of each sound wave's angular span.
const WAVE_HALF_ANGLE: f32 = std::f32::consts::FRAC_PI_3;
/// Line segments used to approximate each sound wave.
const WAVE_SEGMENTS: usize = 6;
/// Stroke width of each sound wave.
const WAVE_STROKE_WIDTH: f32 = 1.25;

/// A speaker silhouette with two sound waves, used for the sound controls.
pub(crate) fn speaker_icon(ui: &mut egui::Ui) -> egui::Response {
    let icon = begin_icon(ui, SPEAKER_ICON_SIZE, false);
    let center = icon.rect.center();
    let back = egui::Rect::from_center_size(
        egui::pos2(icon.rect.left() + SPEAKER_BACK_WIDTH / 2.0, center.y),
        egui::vec2(SPEAKER_BACK_WIDTH, SPEAKER_CONE_HALF_HEIGHT),
    );
    icon.painter.rect_filled(back, 0.0, icon.shell);
    icon.painter.add(egui::Shape::convex_polygon(
        vec![
            back.right_top(),
            egui::pos2(
                back.right() + SPEAKER_CONE_WIDTH,
                center.y - SPEAKER_CONE_HALF_HEIGHT,
            ),
            egui::pos2(
                back.right() + SPEAKER_CONE_WIDTH,
                center.y + SPEAKER_CONE_HALF_HEIGHT,
            ),
            back.right_bottom(),
        ],
        icon.shell,
        egui::Stroke::NONE,
    ));
    let wave_center = egui::pos2(back.right() + SPEAKER_CONE_WIDTH, center.y);
    draw_wave(icon.painter, wave_center, INNER_WAVE_RADIUS, icon.shell);
    draw_wave(icon.painter, wave_center, OUTER_WAVE_RADIUS, icon.shell);
    icon.response
}

fn draw_wave(painter: &egui::Painter, center: egui::Pos2, radius: f32, color: egui::Color32) {
    let points = (0..=WAVE_SEGMENTS).map(|segment| {
        let fraction = segment as f32 / WAVE_SEGMENTS as f32;
        let angle = -WAVE_HALF_ANGLE + fraction * WAVE_HALF_ANGLE * 2.0;
        center + radius * egui::vec2(angle.cos(), angle.sin())
    });
    painter.add(egui::Shape::line(
        points.collect(),
        egui::Stroke::new(WAVE_STROKE_WIDTH, color),
    ));
}
