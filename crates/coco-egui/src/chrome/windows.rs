use crate::*;

impl CocoApp {
    /// Every optional window and modal dialog drawn over the display.
    pub(crate) fn windows_ui(&mut self, ctx: &egui::Context) {
        if self.show_kbd_help {
            let symbolic = self.kb_mode == KbMode::Symbolic;
            let variant = self.machine.config.variant;
            kbd_help::window(ctx, &mut self.show_kbd_help, symbolic, variant);
        }
        if self.show_about {
            about::window(ctx, &mut self.show_about);
        }
        if self.show_orch90
            && let Some(orch90) = self.machine.bus.cart.as_orch90()
        {
            orch90_meters::window(ctx, &mut self.show_orch90, orch90.left(), orch90.right());
        }
        self.debugger
            .windows_ui(ctx, &mut self.machine, &mut self.running);
        if let Some(err) = self.paper_window.ui(ctx) {
            self.cart_error = Some(err);
        }
        self.cart_error_ui(ctx);
    }

    /// Dismissible banner for the last failed cartridge or media load.
    fn cart_error_ui(&mut self, ctx: &egui::Context) {
        let Some(err) = self.cart_error.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new(window_title(ctx, "Cartridge Error"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(err);
                if ui.button("OK").clicked() {
                    self.cart_error = None;
                }
            });
        if !open {
            self.cart_error = None;
        }
    }
}
