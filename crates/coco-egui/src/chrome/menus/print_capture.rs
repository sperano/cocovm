//! The Printer menu's print capture section: bit-banger output to a host
//! text file.

use crate::*;

impl CocoApp {
    /// Start, stop, and open a print capture, and pick its line endings.
    pub(super) fn print_capture_items(&mut self, ui: &mut egui::Ui) {
        self.start_print_capture_item(ui);
        self.stop_print_capture_item(ui);
        self.open_print_capture_item(ui);
        ui.checkbox(&mut self.print_capture_lf, "Translate CR to LF")
            .on_hover_text(
                "Rewrite the CoCo's CR line endings as LF so the capture reads as \
                 normal text. Takes effect when a capture starts.",
            );
    }

    fn start_print_capture_item(&mut self, ui: &mut egui::Ui) {
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
    }

    fn stop_print_capture_item(&mut self, ui: &mut egui::Ui) {
        let label = match &self.print_capture_path {
            Some(p) => format!(
                "Stop Print Capture ({})",
                crate::chrome::status_bar::file_name(p)
            ),
            None => "Stop Print Capture".to_string(),
        };
        if ui
            .add_enabled(self.print_capture_path.is_some(), egui::Button::new(label))
            .clicked()
        {
            self.stop_print_capture();
            ui.close();
        }
    }

    /// Opens the capture file in the host's default viewer.
    fn open_print_capture_item(&mut self, ui: &mut egui::Ui) {
        if ui
            .add_enabled(
                self.print_capture_path.is_some(),
                egui::Button::new("Open Print Capture"),
            )
            .clicked()
        {
            if let Some(path) = &self.print_capture_path {
                let result = {
                    #[cfg(target_os = "macos")]
                    {
                        std::process::Command::new("open").arg(path).spawn()
                    }
                    #[cfg(target_os = "linux")]
                    {
                        std::process::Command::new("xdg-open").arg(path).spawn()
                    }
                    #[cfg(target_os = "windows")]
                    {
                        std::process::Command::new("cmd")
                            .args(["/C", "start", ""])
                            .arg(path)
                            .spawn()
                    }
                };
                if let Err(e) = result {
                    self.cart_error = Some(format!("Failed to open: {}", e));
                }
            }
            ui.close();
        }
    }
}
