//! The VM window's menus. The window has no menu bar: each menu pops up from
//! its status-bar entry (`chrome::status_bar`). A section of a menu that is
//! more than a handful of items lives in its own submodule (the Printer
//! menu's print capture); the rest (keyboard, display, tape, disks, and
//! printer) stay here.

use crate::*;

use super::status_bar;

mod print_capture;

/// The Sound menu's toggle for the Orchestra-90 level meters window
/// ([`orch90_meters::window`]).
const ORCH90_LEVELS_LABEL: &str = "Orchestra-90 Levels";
/// Hover text of [`ORCH90_LEVELS_LABEL`].
const ORCH90_LEVELS_HOVER: &str = "Show the Orchestra-90's left and right DAC levels";

impl CocoApp {
    /// The Keyboard menu, popped up from the status bar's keyboard entry.
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
        let hotkey = ui
            .ctx()
            .format_shortcut(&self.hotkeys.key_layout.shortcut());
        if ui.button(format!("Key layout ({hotkey})")).clicked() {
            self.show_kbd_help = !self.show_kbd_help;
            ui.close();
        }
    }

    /// The Sound menu, popped up from the status bar's sound entry: the host
    /// audio controls, plus the Orchestra-90 level meters toggle while one is
    /// inserted. Shown even with no audio device: the meters read the
    /// cartridge's DAC latches, not the host output.
    pub(super) fn sound_menu_ui(&mut self, ui: &mut egui::Ui) {
        self.audio.menu_ui(ui);
        // `as_orch90` searches both MPI slots as well as the port.
        if self.machine.bus.cart.as_orch90().is_some() {
            ui.separator();
            ui.checkbox(&mut self.show_orch90, ORCH90_LEVELS_LABEL)
                .on_hover_text(ORCH90_LEVELS_HOVER);
        }
    }

    /// Display choice and TV chain knobs, popped up from the status bar's
    /// display entry. Applies live — swapping the cable doesn't erase machine state.
    pub(super) fn display_menu_ui(&mut self, ui: &mut egui::Ui) {
        let variant = self.machine.config.variant;
        for &display in Display::choices(variant) {
            let mut response = ui.selectable_label(self.display == display, display.label());
            if let Some(note) = display.note(variant) {
                response = response.on_hover_text(note);
            }
            if response.clicked() {
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
            ui.add(
                egui::Slider::new(
                    &mut self.tv.overscan_pct,
                    0..=crate::display::MAX_OVERSCAN_PCT,
                )
                .text("Overscan")
                .suffix("%"),
            );
        }
    }

    /// The cassette deck menu: insert, create, rewind, seek, eject, and the
    /// `.wav` save toggle. It pops up from the status bar's tape entry.
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
            Some(p) => format!("Eject Tape ({})", status_bar::file_name(p)),
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

    /// The disks menu, popped up from any of the status bar's disk entries: every
    /// FD-502 drive's section, `first` on top. An FD-502 is always present here.
    pub(super) fn disks_menu_ui(&mut self, ui: &mut egui::Ui, first: usize) {
        for i in 0..UI_DRIVES {
            if i > 0 {
                ui.separator();
            }
            self.drive_menu_section(ui, (first + i) % UI_DRIVES);
        }
    }

    /// One drive's section of [`Self::disks_menu_ui`]: insert, format blank, and eject.
    fn drive_menu_section(&mut self, ui: &mut egui::Ui, drive: usize) {
        if ui
            .button(format!("Insert Disk in Drive {drive}…"))
            .clicked()
        {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Disk image", &["dsk", "jvc", "os9"])
                .pick_file()
            {
                self.insert_disk(drive, path);
            }
        }
        if ui
            .button(format!("New Blank Disk in Drive {drive}…"))
            .clicked()
        {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Disk image", &["dsk"])
                .set_file_name("untitled.dsk")
                .save_file()
            {
                self.new_blank_disk(drive, path);
            }
        }
        let label = match &self.disk_paths[drive] {
            Some(p) => format!("Eject Drive {drive} ({})", status_bar::file_name(p)),
            None => format!("Eject Drive {drive}"),
        };
        let mounted = self.disk_paths[drive].is_some();
        if ui.add_enabled(mounted, egui::Button::new(label)).clicked() {
            self.eject_disk(drive);
            ui.close();
        }
    }

    /// Printer menu: toggle the printer paper window, and start, stop, or
    /// open a print capture.
    pub(super) fn printer_menu_ui(&mut self, ui: &mut egui::Ui) {
        if ui.button("View Papers").clicked() {
            self.toggle_paper_window();
            ui.close();
        }
        ui.separator();
        self.print_capture_items(ui);
    }
}
