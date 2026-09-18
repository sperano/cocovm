//! The Machine menu: save state, disk drives, and
//! print capture. Peripherals live in `[peripherals]`, not here.

use crate::*;

impl CocoApp {
    /// The Machine menu: disk drives and print capture. Reset lives on the
    /// toolbar; VHD and DriveWire startup mounts live in the machine definition.
    /// Peripherals (cartridges, MultiPak, RS-232, RTC) are configured only
    /// through the machine definition's `[peripherals]` and mounted at
    /// launch — there is no runtime insert/eject here. The cassette deck
    /// lives in the status bar's tape entry, not here.
    pub(super) fn machine_menu_ui(&mut self, ui: &mut egui::Ui) {
        self.draw_save_state_menu(ui);
        ui.separator();
        self.machine_disk_items(ui);
        ui.separator();
        self.machine_print_items(ui);
    }

    /// The FD-502 floppy drives: insert, format blank, and eject. Insert/New Blank stay
    /// disabled until an FD-502 is actually present — see [`NO_FD502_HINT`].
    fn machine_disk_items(&mut self, ui: &mut egui::Ui) {
        let has_fd502 = self.machine.bus.cart.as_disk_cart().is_some();
        for drive in 0..UI_DRIVES {
            if ui
                .add_enabled(
                    has_fd502,
                    egui::Button::new(format!("Insert Disk in Drive {drive}…")),
                )
                .on_disabled_hover_text(NO_FD502_HINT)
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
                .add_enabled(
                    has_fd502,
                    egui::Button::new(format!("New Blank Disk in Drive {drive}…")),
                )
                .on_disabled_hover_text(NO_FD502_HINT)
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
                Some(p) => format!(
                    "Eject Drive {drive} ({})",
                    p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                ),
                None => format!("Eject Drive {drive}"),
            };
            let mounted = self.disk_paths[drive].is_some();
            if ui.add_enabled(mounted, egui::Button::new(label)).clicked() {
                self.eject_disk(drive);
                ui.close();
            }
        }
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
