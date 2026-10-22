//! The New Machine dialog's NitrOS-9 EOU download while its images are missing: an
//! install that predates the template, or one whose first-run download of
//! them is still running (the asset dialog hands it over) or failed. The
//! entry downloads the images in the background with a progress bar; once
//! they are in, the dialog offers the template itself.

use eframe::egui;

use super::ManagerApp;
use super::asset_downloads::{Download, PROGRESS_REPAINT_INTERVAL, Source, megabytes};
use crate::nitros9::{self, EOU_ARCHIVE_BYTES, EOU_TITLE};

/// Hover text of the entry's download button.
const DOWNLOAD_HOVER: &str = "Download the disk images the NitrOS-9 EOU template copies";

/// Shown under a failed download, above its retry button.
const FAILED_TEXT: &str = "The NitrOS-9 EOU download failed.";

impl ManagerApp {
    /// The NitrOS-9 EOU entry while its images are missing: the running
    /// download's progress bar, or a button that starts (or retries) it.
    pub(super) fn eou_download_ui(&mut self, ui: &mut egui::Ui) {
        if let Some(download) = &mut self.eou_download {
            download.poll();
            if download.is_running() {
                download.draw(ui);
                ui.ctx().request_repaint_after(PROGRESS_REPAINT_INTERVAL);
                return;
            }
            if let Some(error) = download.error() {
                ui.colored_label(ui.visuals().error_fg_color, FAILED_TEXT)
                    .on_hover_text(error);
            }
        }
        let label = format!("Download {EOU_TITLE} ({})", megabytes(EOU_ARCHIVE_BYTES));
        let Some(eou_dir) = self.eou_dir.clone() else {
            ui.add_enabled(false, egui::Button::new(label))
                .on_disabled_hover_text(super::NO_DATA_DIR);
            return;
        };
        if ui
            .button(label)
            .on_hover_text(format!(
                "{DOWNLOAD_HOVER}, from {}",
                crate::download::host(&self.eou_url)
            ))
            .clicked()
        {
            let url = self.eou_url.clone();
            self.eou_download = Some(Download::start(Source::Eou, ui.ctx(), move |progress| {
                nitros9::download_eou(&url, &eou_dir, progress).map_err(|e| e.to_string())
            }));
        }
    }
}

#[cfg(test)]
#[path = "eou_menu_test.rs"]
mod tests;
