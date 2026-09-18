//! The Machine menu: save state and print capture. Peripherals live in
//! `[peripherals]`, not here; disks live in the status bar's drive entries.

use crate::*;

impl CocoApp {
    /// The Machine menu: save state and print capture. Reset lives on the
    /// toolbar; VHD and DriveWire startup mounts live in the machine definition.
    /// Peripherals (cartridges, MultiPak, RS-232, RTC) are configured only
    /// through the machine definition's `[peripherals]` and mounted at
    /// launch — there is no runtime insert/eject here. The cassette deck and
    /// the floppy drives live in the status bar's tape and disk entries.
    pub(super) fn machine_menu_ui(&mut self, ui: &mut egui::Ui) {
        self.draw_save_state_menu(ui);
        ui.separator();
        self.machine_print_items(ui);
    }

    /// Bit-banger print capture to a host text file.
    fn machine_print_items(&mut self, ui: &mut egui::Ui) {
        let capturing = self.print_capture_path.is_some();
        if ui
            .add_enabled(!capturing, egui::Button::new("Start Print Capture…"))
            .clicked()
        {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Text file", &["txt"])
                .set_file_name("printout.txt")
                .save_file()
            {
                self.start_print_capture(path);
            }
        }
        let label = match &self.print_capture_path {
            Some(p) => format!(
                "Stop Print Capture ({})",
                p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
            ),
            None => "Stop Print Capture".to_string(),
        };
        if ui
            .add_enabled(capturing, egui::Button::new(label))
            .clicked()
        {
            self.stop_print_capture();
            ui.close();
        }
        ui.checkbox(&mut self.print_capture_lf, "Translate CR to LF")
            .on_hover_text(
                "Rewrite the CoCo's CR line endings as LF so the capture reads as \
                 normal text. Takes effect when a capture starts.",
            );
    }
}
