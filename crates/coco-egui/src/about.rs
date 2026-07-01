//! Simple "About" overlay window, drawn slightly translucent (75% opaque) so the
//! emulator screen shows through behind it.

use eframe::egui;

/// Window opacity: 1.0 = solid, 0.75 = 25% see-through.
const OPACITY: f32 = 0.75;

/// Draw the About window. `open` is toggled by the window's close box.
pub fn window(ctx: &egui::Context, open: &mut bool) {
    let style = ctx.style();
    // Translucent background: the theme's window fill with alpha scaled to OPACITY.
    let base = style.visuals.window_fill;
    let fill = egui::Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), (255.0 * OPACITY) as u8);
    let frame = egui::Frame::window(&style).fill(fill);

    egui::Window::new("About coco-rs")
        .open(open)
        .resizable(false)
        .collapsible(false)
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_opacity(OPACITY); // fade the contents to match the background
            ui.vertical_centered(|ui| {
                ui.heading("coco-rs");
                ui.label("A Tandy Color Computer 3 emulator");
                ui.add_space(4.0);
                ui.label(format!("version {}", env!("CARGO_PKG_VERSION")));
                ui.add_space(6.0);
                ui.small("MC6809 CPU · GIME video · Rust + egui");
            });
        });
}
