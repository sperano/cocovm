//! Simple "About" overlay window.

use eframe::egui;

/// Draw the About window. `open` is toggled by the window's close box.
pub fn window(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new(crate::window_title(ctx, "About cocovm"))
        .open(open)
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("cocovm");
                ui.label("A Tandy Color Computer 3 emulator");
                ui.add_space(4.0);
                ui.label(format!("version {}", env!("CARGO_PKG_VERSION")));
                ui.add_space(6.0);
                ui.small("MC6809 CPU · GIME video · Rust + egui");
                ui.add_space(8.0);
                ui.label("© 2026 Éric Spérano");
            });
        });
}
