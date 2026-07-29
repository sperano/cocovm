use crate::*;

impl CocoApp {
    /// Every optional window and modal dialog drawn over the display.
    pub(crate) fn windows_ui(&mut self, ctx: &egui::Context) {
        if self.show_kbd_help {
            let symbolic = self.kb_mode == KbMode::Symbolic;
            kbd_help::window(ctx, &mut self.show_kbd_help, symbolic);
        }
        if self.show_about {
            about::window(ctx, &mut self.show_about);
        }
        if self.show_orch90
            && let Some(orch90) = self.machine.bus.cart.as_orch90()
        {
            orch90_meters::window(ctx, &mut self.show_orch90, orch90.left(), orch90.right());
        }
        self.debugger.windows_ui(ctx, &mut self.machine, &mut self.running);
        if let Some(err) = self.paper_window.ui(ctx) {
            self.cart_error = Some(err);
        }
        self.disk_controller_prompt_ui(ctx);
        self.cart_error_ui(ctx);
    }

    /// Confirmation for a disk action that needs an FD-502 the machine
    /// doesn't have yet — installing one cold-restarts the machine.
    fn disk_controller_prompt_ui(&mut self, ctx: &egui::Context) {
        if self.pending_disk_action.is_none() {
            return;
        }
        // Match the dialog body to the button font (egui's default body
        // text is a touch smaller) and give the text room.
        let font = ctx.style().text_styles[&egui::TextStyle::Button].size;
        const DIALOG_MARGIN: i8 = 16;
        egui::Window::new(window_title(ctx, "Insert disk controller?"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Frame::NONE.inner_margin(DIALOG_MARGIN).show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(
                            "The FD-502 disk controller isn't installed yet. Installing \
                             it swaps the cartridge and cold-restarts the machine — any \
                             unsaved work in memory will be lost.",
                        )
                        .size(font),
                    );
                    ui.add_space(DIALOG_MARGIN as f32);
                    ui.horizontal(|ui| {
                        // Roomier buttons: pad text away from the button edge.
                        ui.spacing_mut().button_padding = egui::vec2(12.0, 6.0);
                        if ui.button("Insert & Restart").clicked() {
                            match self.pending_disk_action.take() {
                                Some(PendingDiskAction::Insert { drive, path }) => {
                                    self.insert_disk(drive, path)
                                }
                                Some(PendingDiskAction::NewBlank { drive, path }) => {
                                    self.new_blank_disk(drive, path)
                                }
                                None => {}
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.pending_disk_action = None;
                        }
                    });
                });
            });
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
