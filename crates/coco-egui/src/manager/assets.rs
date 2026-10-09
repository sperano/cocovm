//! The first-run asset download dialog: when startup finds bundle assets
//! missing (`startup::missing_assets`), the window opens dialog-sized and
//! shows only this prompt — the manager UI appears (and the window grows to
//! its real size) only after a successful download. Download runs on a
//! background thread so the window stays live; Cancel quits the app.

use std::path::PathBuf;
use std::sync::mpsc;

use eframe::egui;

use super::{ManagerApp, WINDOW_SIZE};

/// What the dialog asks before fetching anything.
const PROMPT_TEXT: &str =
    "Download the ROMs, cartridges, and images CoCoVM needs to start your machines.";

/// Window size while the dialog is the only content.
pub(crate) const DIALOG_WINDOW_SIZE: [f32; 2] = [480.0, 280.0];
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
    /// Download finished and every missing file is now present.
    Installed,
    /// The user declined — the app quits.
    Cancelled,
}

/// State of the dialog phase, held in [`ManagerApp::asset_dialog`] until
/// the download succeeds (Cancel never clears it — the app closes instead).
pub(crate) struct AssetDialog {
    /// Display paths of the absent files, from `startup::missing_assets`.
    missing: Vec<String>,
    /// Where the bundle is fetched from (`--assets-url`, `cli.rs`).
    assets_url: String,
    /// Where the bundle unpacks (`paths::assets_dir`).
    install_dir: PathBuf,
    /// Completion channel of the running download thread; `Some` while it runs.
    job: Option<mpsc::Receiver<Result<(), String>>>,
    /// Failure from the last attempt, shown above the buttons for a retry.
    error: Option<String>,
    icon: Option<egui::TextureHandle>,
}

impl AssetDialog {
    pub(crate) fn new(missing: Vec<String>, assets_url: String, install_dir: PathBuf) -> Self {
        Self {
            missing,
            assets_url,
            install_dir,
            job: None,
            error: None,
            icon: None,
        }
    }

    /// Fold a finished download thread's result back in. Returns whether
    /// the download completed successfully (the dialog's work is done). An
    /// unpack that still leaves files missing — a stale bundle behind a new
    /// [`crate::startup::BUNDLED_ROMS`] or cartridge-manifest entry — reads as a failure, not a
    /// silent close followed by a re-prompt on every start.
    fn poll(&mut self) -> bool {
        let Some(rx) = &self.job else {
            return false;
        };
        match rx.try_recv() {
            Ok(Ok(())) => {
                let still_missing = crate::missing_assets();
                if still_missing.is_empty() {
                    return true;
                }
                self.missing = still_missing;
                self.error =
                    Some("the downloaded bundle did not provide every missing file".to_string());
            }
            Ok(Err(e)) => self.error = Some(e),
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.error = Some("download thread exited unexpectedly".to_string());
            }
        }
        self.job = None;
        false
    }

    /// Spawn the download thread. Its result comes back through [`Self::job`];
    /// the repaint wakes the UI so [`Self::poll`] sees it promptly.
    fn start_download(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let url = self.assets_url.clone();
        let dest = self.install_dir.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result =
                crate::startup::download_and_unpack_assets(&url, &dest).map_err(|e| e.to_string());
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.job = Some(rx);
        self.error = None;
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
        if let Some(err) = &self.error {
            ui.add_space(TEXT_GAP);
            ui.colored_label(ui.visuals().error_fg_color, "Download failed. Try again.");
            ui.collapsing("Show details", |ui| {
                ui.label(err);
            });
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
        if self.job.is_some() {
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
            if ui.add(download).clicked() {
                self.start_download(ui.ctx());
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
                self.asset_dialog = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(WINDOW_SIZE.into()));
                if self.welcome_image.is_blank() {
                    self.welcome_image.load_random();
                }
            }
            Verdict::Cancelled => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }
}
