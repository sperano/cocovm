//! The New Machine dialog's NitrOS-9 EOU download beneath a template
//! whose missing files are the EOU images, in the background with a
//! progress bar. The images go straight into that template's media folder;
//! once they are in, the dialog offers the template itself.

use std::path::Path;

use eframe::egui;

use super::ManagerApp;
use super::asset_downloads::{Download, PROGRESS_REPAINT_INTERVAL, Source, megabytes};
use crate::nitros9::{self, EOU_ARCHIVE_BYTES, EOU_TITLE};

/// Hover text of the entry's download button.
const DOWNLOAD_HOVER: &str = "Download the NitrOS-9 EOU disk images this template copies";

/// Shown under a failed download, above its retry button.
const FAILED_TEXT: &str = "The NitrOS-9 EOU download failed.";

/// Disabled-hover text of the download button while the download runs for
/// another template: one runs at a time.
pub(crate) const OTHER_DOWNLOAD_HOVER: &str =
    "Wait for the NitrOS-9 EOU download of another template to finish";

impl ManagerApp {
    /// Beneath a template missing the NitrOS-9 EOU images: the download's
    /// progress bar while it runs into `media_dir`, the template's media
    /// folder, or a button that starts (or retries) it there. While it
    /// runs for another template, the button is off.
    pub(super) fn eou_download_ui(&mut self, ui: &mut egui::Ui, media_dir: Option<&Path>) {
        let label = format!("Download {EOU_TITLE} ({})", megabytes(EOU_ARCHIVE_BYTES));
        let here = media_dir.is_some() && self.eou_download_dir.as_deref() == media_dir;
        if let Some(download) = &mut self.eou_download {
            download.poll();
            if download.is_running() {
                ui.ctx().request_repaint_after(PROGRESS_REPAINT_INTERVAL);
                if here {
                    download.draw(ui);
                } else {
                    ui.add_enabled(false, egui::Button::new(label))
                        .on_disabled_hover_text(OTHER_DOWNLOAD_HOVER);
                }
                return;
            }
            if here && let Some(error) = download.error() {
                ui.colored_label(ui.visuals().error_fg_color, FAILED_TEXT)
                    .on_hover_text(error);
            }
        }
        let Some(media_dir) = media_dir.map(Path::to_path_buf) else {
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
            self.eou_download_dir = Some(media_dir.clone());
            self.eou_download = Some(Download::start(Source::Eou, ui.ctx(), move |progress| {
                nitros9::download_eou(&url, &media_dir, progress).map_err(|e| e.to_string())
            }));
        }
    }
}

impl ManagerApp {
    /// Whether the EOU download is still writing into `media_dir`, after
    /// taking its result if it has one.
    pub(super) fn eou_downloading_into(&mut self, media_dir: &Path) -> bool {
        let Some(download) = &mut self.eou_download else {
            return false;
        };
        download.poll();
        download.is_running() && self.eou_download_dir.as_deref() == Some(media_dir)
    }
}

#[cfg(test)]
#[path = "eou_menu_test.rs"]
mod tests;
