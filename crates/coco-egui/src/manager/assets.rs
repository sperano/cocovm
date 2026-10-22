//! The first-run asset download dialog: when startup finds bundle assets
//! missing (`startup::missing_assets`), the window opens dialog-sized and
//! shows only this prompt — the manager UI appears (and the window grows to
//! its real size) only once the download has installed the bundle; a failed
//! NitrOS-9 EOU download alone can be skipped. Each missing file set
//! ([`Source`]) downloads on its own background thread, all at once, with a
//! progress bar per set, so the window stays live; Cancel quits the app.

use std::path::PathBuf;

use eframe::egui;

use super::asset_downloads::{Download, PROGRESS_REPAINT_INTERVAL, Source, megabytes};
use super::{ManagerApp, WINDOW_SIZE};
use crate::nitros9::{EOU_ARCHIVE_BYTES, EOU_HOST, EOU_TITLE};

/// What the dialog asks before fetching anything.
const PROMPT_TEXT: &str =
    "Download the ROMs, cartridges, and images CoCoVM needs to start your machines.";

/// Window size while the dialog is the only content: room for a progress
/// bar per [`Source`], with a failure message under them.
pub(crate) const DIALOG_WINDOW_SIZE: [f32; 2] = [480.0, 376.0];

const CONTENT_MARGIN: i8 = 24;
const FOOTER_MARGIN: i8 = 16;
const SECTION_GAP: f32 = 16.0;
const TEXT_GAP: f32 = 6.0;
const BUTTON_GAP: f32 = 8.0;
const ICON_SIZE: f32 = 56.0;
const TITLE_SIZE: f32 = 22.0;
const BODY_SIZE: f32 = 14.0;
const NOTE_SIZE: f32 = 12.0;
const BUTTON_SIZE: [f32; 2] = [104.0, 32.0];
const BUTTON_RADIUS: u8 = 6;
const PRIMARY_FILL: egui::Color32 = egui::Color32::from_rgb(11, 99, 206);
const SECONDARY_DARK: egui::Color32 = egui::Color32::from_rgb(180, 180, 180);
const SECONDARY_LIGHT: egui::Color32 = egui::Color32::from_rgb(96, 96, 96);
const ICON_BYTES: &[u8] = include_bytes!("../../assets/cocovm-icon.png");

/// What one dialog frame resolved to.
pub(super) enum Verdict {
    Pending,
    /// The required downloads finished and their files are all present.
    /// Optional ones may still be running, or may have failed.
    Installed,
    /// The user declined — the app quits.
    Cancelled,
}

/// State of the dialog phase, held in [`ManagerApp::asset_dialog`] until
/// the download succeeds (Cancel never clears it — the app closes instead).
pub(crate) struct AssetDialog {
    /// Display paths of the absent files of every [`Source`].
    missing: Vec<String>,
    /// Where the bundle is fetched from (`--assets-url`, `cli.rs`).
    assets_url: String,
    /// Where the bundle unpacks (`paths::assets_dir`).
    install_dir: PathBuf,
    /// The sources [`Self::start_download`] would fetch: those with missing
    /// files.
    pending: Vec<Source>,
    /// The latest attempt's downloads, one per source it fetched; empty
    /// before the first.
    downloads: Vec<Download>,
    /// Failure from the last attempt, shown above the buttons for a retry.
    error: Option<String>,
    icon: Option<egui::TextureHandle>,
}

impl AssetDialog {
    /// The dialog for the bundle's `missing` files (`startup::missing_assets`),
    /// which also fetches whichever NitrOS-9 EOU images are missing.
    pub(crate) fn new(mut missing: Vec<String>, assets_url: String, install_dir: PathBuf) -> Self {
        missing.extend(crate::startup::missing_eou_assets());
        Self {
            missing,
            assets_url,
            install_dir,
            pending: pending_sources(),
            downloads: Vec::new(),
            error: None,
            icon: None,
        }
    }

    /// Whether any download is still running.
    fn downloading(&self) -> bool {
        self.downloads.iter().any(Download::is_running)
    }

    /// Whether a required source's download is still running: what the
    /// dialog waits for. An optional one (the NitrOS-9 EOU images, from a
    /// third-party site) never holds the manager back.
    fn installing(&self) -> bool {
        self.downloads
            .iter()
            .any(|download| download.is_running() && download.source().required())
    }

    /// Fold finished download threads' results back in. Returns whether
    /// the required downloads completed successfully (the dialog's work is
    /// done). One that still leaves files missing — a stale bundle behind a
    /// new [`crate::startup::BUNDLED_ROMS`] or cartridge-manifest entry —
    /// reads as a failure, not a silent close followed by a re-prompt on
    /// every start.
    fn poll(&mut self) -> bool {
        let was_installing = self.installing();
        for download in &mut self.downloads {
            download.poll();
        }
        if !was_installing || self.installing() {
            return false;
        }
        self.pending = pending_sources();
        if !self.pending.iter().any(|source| source.required()) {
            return true;
        }
        self.missing = self
            .pending
            .iter()
            .flat_map(|source| source.missing())
            .collect();
        self.error = Some(self.failure_details());
        false
    }

    /// The NitrOS-9 EOU download, running or finished, for the New menu to
    /// carry on showing once the dialog closes (`manager/eou_menu.rs`).
    fn into_eou_download(self) -> Option<Download> {
        self.downloads
            .into_iter()
            .find(|download| download.source() == Source::Eou)
    }

    /// What went wrong in the finished attempt: each failed download's
    /// error, or, when none failed, that the files are still not all there.
    fn failure_details(&self) -> String {
        let errors: Vec<String> = self
            .downloads
            .iter()
            .filter_map(|download| {
                let error = download.error()?;
                Some(format!("{}: {error}", download.source().label()))
            })
            .collect();
        if errors.is_empty() {
            "the downloaded files did not provide every missing file".to_string()
        } else {
            errors.join("\n")
        }
    }

    /// Start a download thread for every [`Self::pending`] source not
    /// already downloading (a retry leaves a running EOU download alone).
    /// Each thread's result comes back through its [`Download`]; its repaint
    /// wakes the UI so [`Self::poll`] sees it promptly. Returns `false` when
    /// no required file is missing any more, so there is nothing to wait for.
    fn start_download(&mut self, ctx: &egui::Context) -> bool {
        self.downloads.retain(Download::is_running);
        let running: Vec<Source> = self.downloads.iter().map(Download::source).collect();
        let idle = self
            .pending
            .iter()
            .filter(|source| !running.contains(source));
        for &source in idle {
            let url = self.assets_url.clone();
            let install_dir = self.install_dir.clone();
            let download = Download::start(source, ctx, move |progress| {
                source.fetch(&url, &install_dir, progress)
            });
            self.downloads.push(download);
        }
        self.error = None;
        self.installing()
    }

    /// Draw the dialog as the window's only content for one frame.
    fn draw(&mut self, ctx: &egui::Context) -> Verdict {
        if self.poll() {
            println!(" Assets installed in {}", self.install_dir.display());
            println!(
                " {} found.",
                crate::startup::asset_inventory(crate::rom_count(), crate::cartridge_count())
            );
            return Verdict::Installed;
        }
        if self.downloading() {
            ctx.request_repaint_after(PROGRESS_REPAINT_INTERVAL);
        }
        let verdict = egui::TopBottomPanel::bottom("asset_actions")
            .frame(
                egui::Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(egui::Margin::symmetric(CONTENT_MARGIN, FOOTER_MARGIN)),
            )
            .show(ctx, |ui| self.draw_actions(ui))
            .inner;
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&ctx.style()).inner_margin(CONTENT_MARGIN))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.draw_body(ui));
            });
        verdict
    }

    fn draw_header(&mut self, ui: &mut egui::Ui) {
        let icon = self.icon.get_or_insert_with(|| {
            let image = image::load_from_memory(ICON_BYTES)
                .expect("embedded icon PNG is valid")
                .into_rgba8();
            let size = [image.width() as usize, image.height() as usize];
            ui.ctx().load_texture(
                "asset_dialog_icon",
                egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
                egui::TextureOptions::LINEAR,
            )
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SECTION_GAP;
            ui.add(egui::Image::new(&*icon).fit_to_exact_size(egui::Vec2::splat(ICON_SIZE)));
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("Set up CoCoVM")
                        .size(TITLE_SIZE)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new("Download required files")
                        .size(BODY_SIZE)
                        .color(secondary_color(ui)),
                );
            });
        });
    }

    fn draw_body(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = TEXT_GAP;
        self.draw_header(ui);
        ui.add_space(SECTION_GAP);
        ui.label(
            egui::RichText::new(PROMPT_TEXT)
                .size(BODY_SIZE)
                .color(ui.visuals().strong_text_color()),
        );
        if self.pending.contains(&Source::Eou) {
            // The EOU images come from a third-party site and add most of
            // the download.
            let note = format!(
                "Includes {EOU_TITLE} ({}) from {EOU_HOST}. CoCoVM opens without \
                waiting for it.",
                megabytes(EOU_ARCHIVE_BYTES)
            );
            ui.label(egui::RichText::new(note).color(secondary_color(ui)));
        }
        if !self.downloads.is_empty() {
            ui.add_space(TEXT_GAP);
            for download in &self.downloads {
                download.draw(ui);
            }
        }
        if let Some(err) = &self.error {
            ui.add_space(TEXT_GAP);
            ui.colored_label(ui.visuals().error_fg_color, "Download failed. Try again.");
            ui.collapsing("Show details", |ui| {
                ui.label(err);
            });
            return;
        }
        if self.downloading() {
            return;
        }
        let count = self.missing.len();
        ui.label(
            egui::RichText::new(format!(
                "{count} file{} to install.",
                if count == 1 { "" } else { "s" }
            ))
            .color(secondary_color(ui)),
        );
        ui.add_space(TEXT_GAP);
        ui.label(
            egui::RichText::new("These files include copyrighted software and artwork.")
                .size(NOTE_SIZE)
                .color(secondary_color(ui)),
        );
    }

    /// Keep the actions visible even when a long error needs to scroll.
    fn draw_actions(&mut self, ui: &mut egui::Ui) -> Verdict {
        if self.installing() {
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), BUTTON_SIZE[1]),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.spinner();
                    ui.label("Downloading and installing…");
                },
            );
            return Verdict::Pending;
        }
        let mut verdict = Verdict::Pending;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = BUTTON_GAP;
            let label = if self.error.is_some() {
                "Try again"
            } else {
                "Download"
            };
            let download =
                egui::Button::new(egui::RichText::new(label).color(egui::Color32::WHITE))
                    .fill(PRIMARY_FILL)
                    .corner_radius(BUTTON_RADIUS)
                    .min_size(BUTTON_SIZE.into());
            if ui.add(download).clicked() && !self.start_download(ui.ctx()) {
                verdict = Verdict::Installed;
            }
            let cancel = egui::Button::new("Cancel")
                .corner_radius(BUTTON_RADIUS)
                .min_size(BUTTON_SIZE.into());
            if ui.add(cancel).clicked() {
                verdict = Verdict::Cancelled;
            }
        });
        verdict
    }
}

/// Viewport commands, in order, that turn the fixed-size dialog window into
/// the manager's resizable one.
///
/// `Resizable(true)` alone is not enough on Linux: winit 0.30 makes a window
/// created non-resizable (`run.rs`) fixed by recording its size as the
/// window's min and max size, on X11 (`x11/window.rs`, window creation) and
/// Wayland (`WindowState::set_resizable`). `set_resizable(true)` restores
/// those recorded limits, so the window stays pinned at
/// [`DIALOG_WINDOW_SIZE`] and the window manager clamps the `InnerSize`
/// request to it. Clearing both limits afterwards matches a normal start,
/// which sets neither: egui-winit maps `MinInnerSize(Vec2::ZERO)` and
/// `MaxInnerSize(Vec2::INFINITY)` to "no limit".
fn grow_to_manager_commands() -> [egui::ViewportCommand; 4] {
    [
        egui::ViewportCommand::Resizable(true),
        egui::ViewportCommand::MinInnerSize(egui::Vec2::ZERO),
        egui::ViewportCommand::MaxInnerSize(egui::Vec2::INFINITY),
        egui::ViewportCommand::InnerSize(WINDOW_SIZE.into()),
    ]
}

/// The sources with files missing on disk, in [`Source::ALL`] order.
fn pending_sources() -> Vec<Source> {
    Source::ALL
        .into_iter()
        .filter(|source| !source.missing().is_empty())
        .collect()
}

fn secondary_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        SECONDARY_DARK
    } else {
        SECONDARY_LIGHT
    }
}

impl ManagerApp {
    /// Drive the dialog phase for one frame. Success grows the window to
    /// the manager's size and reseeds the central pane's photo (on a true
    /// first run it was drawn from an empty images directory); Cancel
    /// closes the window, quitting the app.
    pub(super) fn draw_asset_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.asset_dialog.as_mut() else {
            return;
        };
        match dialog.draw(ctx) {
            Verdict::Pending => {}
            Verdict::Installed => {
                self.eou_download = self
                    .asset_dialog
                    .take()
                    .and_then(AssetDialog::into_eou_download);
                for command in grow_to_manager_commands() {
                    ctx.send_viewport_cmd(command);
                }
                if self.welcome_image.is_blank() {
                    self.welcome_image.load_random();
                }
            }
            Verdict::Cancelled => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }
}

#[cfg(test)]
#[path = "assets_test.rs"]
mod tests;
