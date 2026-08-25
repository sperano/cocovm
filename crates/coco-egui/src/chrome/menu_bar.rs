//! The menu bar: Machine, View, Sound, and Help. Each menu that is more
//! than a handful of items lives in its own submodule; the short ones
//! (Keyboard, View, Help, and the status bar's display and tape menus) stay
//! here. The keyboard, display, tape, and joysticks menus have no menu-bar
//! button — each pops up from its status-bar entry (`chrome::status_bar`).

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
                ui.menu_button("Sound", |ui| self.audio.menu_ui(ui));
                ui.menu_button("Help", |ui| self.help_menu_ui(ui));
            });
        });
    }

    /// The Keyboard menu. Has no menu-bar button; the status bar's keyboard
    /// entry pops it up.
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

    /// The View menu: aspect correction and optional windows. Display choice
    /// lives in [`Self::display_menu_ui`], not here.
    fn view_menu_ui(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.aspect_correct, "4:3 aspect (F9)");
        ui.separator();
        let mut paper_open = self.paper_window.open;
        if ui.checkbox(&mut paper_open, "Printer Paper").changed() {
            self.toggle_paper_window();
        }
        // Only meaningful with an Orchestra-90 cartridge inserted; `as_orch90` searches both slots.
        let orch90_present = self.machine.bus.cart.as_orch90().is_some();
        ui.add_enabled(
            orch90_present,
            egui::Checkbox::new(&mut self.show_orch90, "Orchestra-90 Levels"),
        );
    }

    /// Display choice and TV chain knobs, popped up from the status bar's
    /// display entry. Applies live — swapping the cable doesn't erase machine state.
    pub(super) fn display_menu_ui(&mut self, ui: &mut egui::Ui) {
        let variant = self.machine.config.variant;
        for &display in Display::choices(variant) {
            if ui
                .selectable_label(self.display == display, display.label())
                .clicked()
            {
                self.display = display;
                // CoCo 1/2 has no GIME palette to steer; renderer never consults `gime.monitor`.
                if let Some(monitor) = display.to_monitor(variant) {
                    self.machine.bus.gime.monitor = monitor;
                }
            }
        }
        if matches!(self.display, Display::TV(_)) {
            ui.add(
                egui::Slider::new(&mut self.tv.scanline_pct, 0..=crate::display::MAX_PCT)
                    .text("Scanlines")
                    .suffix("%"),
            );
            ui.add(
                egui::Slider::new(&mut self.tv.noise_pct, 0..=crate::display::MAX_PCT)
                    .text("RF noise")
                    .suffix("%"),
            );
        }
    }

    /// The cassette deck menu: insert, create, rewind, seek, eject, and the
    /// .wav save toggle. Popped up from the status bar's tape entry.
    pub(super) fn tape_menu_ui(&mut self, ui: &mut egui::Ui) {
        if ui.button("Insert Tape…").clicked() {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Cassette image", &["cas", "wav"])
                .pick_file()
            {
                self.insert_tape(path);
            }
        }
        if ui.button("New Tape…").clicked() {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Cassette image", &["cas"])
                .set_file_name("untitled.cas")
                .save_file()
            {
                self.new_tape(path);
            }
        }
        let tape_mounted = self.tape_path.is_some();
        if ui
            .add_enabled(tape_mounted, egui::Button::new("Rewind Tape"))
            .clicked()
        {
            self.machine.bus.cassette.rewind();
            ui.close();
        }
        self.tape_seek_ui(ui, tape_mounted);
        let label = match &self.tape_path {
            Some(p) => format!(
                "Eject Tape ({})",
                p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
            ),
            None => "Eject Tape".to_string(),
        };
        if ui
            .add_enabled(tape_mounted, egui::Button::new(label))
            .clicked()
        {
            self.eject_tape();
            ui.close();
        }
        ui.checkbox(&mut self.save_tape_wav, "Also save tape audio (.wav)");
    }

    /// The "Seek to byte" row of [`Self::tape_menu_ui`]: a text field
    /// committed with Enter.
    fn tape_seek_ui(&mut self, ui: &mut egui::Ui, tape_mounted: bool) {
        ui.add_enabled_ui(tape_mounted, |ui| {
            ui.horizontal(|ui| {
                ui.label("Seek to byte:");
                let response = ui
                    .add(egui::TextEdit::singleline(&mut self.tape_seek_text).desired_width(60.0));
                if response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && let Ok(pos) = self.tape_seek_text.trim().parse::<usize>()
                {
                    self.machine.bus.cassette.seek(pos);
                    self.tape_seek_text.clear();
                    ui.close();
                }
            });
        });
    }

    /// The Help menu.
    fn help_menu_ui(&mut self, ui: &mut egui::Ui) {
        if ui.button("About").clicked() {
            self.show_about = !self.show_about;
            ui.close();
        }
    }
}
