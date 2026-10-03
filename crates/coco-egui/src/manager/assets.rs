//! The first-run asset download dialog: when startup finds bundle assets
//! missing (`startup::missing_assets`), the window opens dialog-sized and
//! shows only this prompt — the manager UI appears (and the window grows to
//! its real size) only after a successful download. Download runs on a
//! background thread so the window stays live; Cancel quits the app.

use std::path::PathBuf;
use std::sync::mpsc;

use eframe::egui;

use super::{DETAIL_SECTION_GAP, ManagerApp, WINDOW_SIZE};

/// What the dialog asks before fetching anything.
const PROMPT_TEXT: &str = "CoCoVM needs to download some copyrighted assets (ROMs, cartridges, images) to function properly.";

/// Window size while the dialog is the only content.
pub(super) const DIALOG_WINDOW_SIZE: [f32; 2] = [500.0, 190.0];

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
}

impl AssetDialog {
    pub(crate) fn new(missing: Vec<String>, assets_url: String, install_dir: PathBuf) -> Self {
        Self {
            missing,
            assets_url,
            install_dir,
            job: None,
            error: None,
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
        let mut verdict = Verdict::Pending;
        egui::CentralPanel::default().show(ctx, |ui| {
            verdict = self.draw_body(ui);
        });
        verdict
    }

    /// The dialog's contents: heading, prompt, missing count, any error from
    /// the last attempt, then either a progress row or the button row.
    fn draw_body(&mut self, ui: &mut egui::Ui) -> Verdict {
        ui.heading("Download assets");
        ui.add_space(DETAIL_SECTION_GAP);
        ui.label(PROMPT_TEXT);
        let count = self.missing.len();
        ui.label(format!(
            "{count} file{} missing.",
            if count == 1 { " is" } else { "s are" }
        ));
        if let Some(err) = &self.error {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("Download failed: {err}"),
            );
        }
        ui.add_space(DETAIL_SECTION_GAP);
        if self.job.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Downloading {}…", self.assets_url));
            });
            return Verdict::Pending;
        }
        let mut verdict = Verdict::Pending;
        ui.horizontal(|ui| {
            if ui.button("Download").clicked() {
                self.start_download(ui.ctx());
            }
            if ui.button("Cancel").clicked() {
                verdict = Verdict::Cancelled;
            }
        });
        verdict
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
