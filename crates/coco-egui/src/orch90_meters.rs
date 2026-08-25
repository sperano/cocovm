//! Orchestra-90/CC level-meter overlay: shows the two DAC latch values
//! (`coco_core::orch90::Orch90::left`/`right`) as horizontal bars. Toggled
//! from the View menu; the caller only draws this window while an Orch90 is
//! actually inserted (see the call site in `main.rs`'s `update`).

use eframe::egui;

/// Width of each level bar.
const METER_WIDTH: f32 = 220.0;

/// One channel's bar: a label plus an egui `ProgressBar` scaled from the
/// raw 0-255 DAC latch value, with the value itself overlaid as text.
fn level_bar(ui: &mut egui::Ui, label: &str, value: u8) {
    ui.horizontal(|ui| {
        ui.label(label);
        let frac = f32::from(value) / f32::from(u8::MAX);
        ui.add(
            egui::ProgressBar::new(frac)
                .desired_width(METER_WIDTH)
                .text(value.to_string()),
        );
    });
}

/// Draws the Orchestra-90 level-meter window; `open` is toggled by the
/// window's close box.
pub fn window(ctx: &egui::Context, open: &mut bool, left: u8, right: u8) {
    egui::Window::new(crate::window_title(ctx, "Orchestra-90"))
        .open(open)
        .resizable(false)
        .collapsible(true)
        .show(ctx, |ui| {
            level_bar(ui, "L", left);
            level_bar(ui, "R", right);
        });
}
