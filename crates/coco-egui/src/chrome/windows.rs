use crate::*;

impl CocoApp {
    /// Every optional window and modal dialog drawn over the display.
    pub(crate) fn windows_ui(&mut self, ctx: &egui::Context) {
        self.keyboard_window_ui(ctx);
        if self.show_orch90
            && let Some(orch90) = self.machine.bus.cart.as_orch90()
        {
            orch90_meters::window(ctx, &mut self.show_orch90, orch90.left(), orch90.right());
        }
        let was_running = self.running;
        self.debugger.windows_ui(
            ctx,
            &mut self.machine,
            &mut self.running,
            self.hotkeys.debugger,
        );
        if was_running != self.running {
            self.reset_emulation_clock();
            ctx.request_repaint();
        }
        if let Some(err) = self.paper_window.ui(ctx) {
            self.cart_error = Some(err);
        }
        self.pending_load_ui(ctx);
        self.cart_error_ui(ctx);
    }

    fn keyboard_window_ui(&mut self, ctx: &egui::Context) {
        if self.show_kbd_help {
            let enabled = !self.suspended
                && !self.remote_type_ahead.is_active()
                && self.remote_held.is_none();
            let taps = kbd_help::window(
                ctx,
                &mut self.show_kbd_help,
                self.kb_mode == KbMode::Symbolic,
                self.machine.config.variant,
                enabled,
                &mut self.keyboard_modifiers,
                &self.hotkeys,
            );
            if !taps.is_empty() {
                // Start clicks from a clean matrix; stale host keys must not become
                // part of the chord. An existing queued hold keeps its timing.
                if !self.type_ahead.is_active() {
                    self.machine.bus.keyboard.release_all();
                }
                self.type_ahead.queue.extend(taps);
            }
        }
        if !self.show_kbd_help {
            self.keyboard_modifiers = typeahead::KeyModifiers::default();
        }
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
