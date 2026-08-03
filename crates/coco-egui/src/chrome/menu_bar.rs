//! The menu bar. Each menu that is more than a handful of items lives in its
//! own submodule; the short ones (Keyboard, View, Help) stay here.

use crate::*;

mod drivewire;
mod machine;
mod mpi;
mod rs232;

impl CocoApp {
    /// The menu bar and all of its menus.
    pub(crate) fn menu_bar_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("Machine", |ui| self.machine_menu_ui(ui));
                ui.menu_button("View", |ui| self.view_menu_ui(ui));
                ui.menu_button("Joysticks", |ui| self.joysticks.menu_ui(ui));
                ui.menu_button("Sound", |ui| self.audio.menu_ui(ui));
                ui.menu_button("Help", |ui| self.help_menu_ui(ui));
            });
        });
    }

    /// The Keyboard menu: positional/symbolic mode and the key map. It has
    /// no menu-bar button — the status bar's keyboard entry pops it up
    /// (`chrome::status_bar`'s `keyboard_status`).
    pub(super) fn keyboard_menu_ui(&mut self, ui: &mut egui::Ui) {
        for mode in [KbMode::Positional, KbMode::Symbolic] {
            if ui
                .selectable_label(self.kb_mode == mode, mode.label())
                .clicked()
            {
                self.set_mode(mode);
            }
        }
        ui.separator();
        if ui.button("Key layout (F10)").clicked() {
            self.show_kbd_help = !self.show_kbd_help;
            ui.close();
        }
    }

    /// The View menu: scaling, aspect correction, and the optional
    /// Orchestra-90 level meters and monitor type.
    fn view_menu_ui(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.aspect_correct, "4:3 aspect (F9)");
        ui.separator();
        ui.checkbox(&mut self.debugger.open, "Debugger (F11)");
        ui.separator();
        let mut paper_open = self.paper_window.open;
        if ui.checkbox(&mut paper_open, "Printer Paper").changed() {
            self.toggle_paper_window();
        }
        // Only meaningful with an Orchestra-90 cartridge actually inserted
        // (direct port or in an MPI slot) — `as_orch90` searches both.
        let orch90_present = self.machine.bus.cart.as_orch90().is_some();
        ui.add_enabled(
            orch90_present,
            egui::Checkbox::new(&mut self.show_orch90, "Orchestra-90 Levels"),
        );
        ui.separator();
        // Swapping the display cable doesn't erase machine state, so this
        // takes effect live rather than requiring a power cycle. Monitor
        // choices exist only where a monitor port does (CoCo 3); a CoCo 1/2
        // offers just the two TVs (`Display::choices`).
        let variant = self.machine.config.variant;
        for display in Display::choices(variant) {
            if ui
                .selectable_label(self.display == *display, display.label())
                .clicked()
            {
                self.display = *display;
                // A CoCo 1/2 has no GIME palette to steer (`None`) — its
                // renderer never consults `gime.monitor`.
                if let Some(monitor) = display.to_monitor(variant) {
                    self.machine.bus.gime.monitor = monitor;
                }
            }
        }
        // The TV chain's knobs, live like the display choice itself —
        // drawn only while they'd have a visible effect.
        if matches!(self.display, Display::TV(_)) {
            ui.add(
                egui::Slider::new(&mut self.tv.scanline_pct, 0..=100)
                    .text("Scanlines")
                    .suffix("%"),
            );
        }
    }

    /// The Help menu.
    fn help_menu_ui(&mut self, ui: &mut egui::Ui) {
        if ui.button("About").clicked() {
            self.show_about = !self.show_about;
            ui.close();
        }
    }
}
