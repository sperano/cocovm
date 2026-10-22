//! Creating a NitrOS-9 EOU machine off the UI thread. [`presets::install`]
//! copies the template's 128 MB system disk, which takes a fraction of a
//! second on an SSD but can freeze the window for seconds on a hard disk or
//! a network home directory. So the New menu runs the install on a thread of
//! its own, shows a spinner in place of the template meanwhile, and inserts
//! the machine once it is saved.

use std::path::PathBuf;
use std::sync::mpsc;

use eframe::egui;

use super::ManagerApp;
use super::asset_downloads::PROGRESS_REPAINT_INTERVAL;
use super::lifecycle::created_today;
use super::presets::{self, Preset};
use crate::machine_def::MachineDef;

/// The New menu's text while the install runs.
pub(crate) const COPYING_TEXT: &str = "Copying the NitrOS-9 EOU disk images…";

/// A NitrOS-9 EOU machine being written on its own thread.
pub(crate) struct PendingInstall {
    /// The slug the machine is saved under.
    slug: String,
    /// The install's outcome, once it has one.
    result: mpsc::Receiver<Result<MachineDef, String>>,
}

impl PendingInstall {
    /// An install of `slug` whose outcome arrives on `result`.
    pub(crate) fn new(slug: String, result: mpsc::Receiver<Result<MachineDef, String>>) -> Self {
        Self { slug, result }
    }
}

impl ManagerApp {
    /// Install the NitrOS-9 EOU template as machine `slug` under
    /// `machines_dir` on a new thread; [`Self::poll_eou_install`] inserts it.
    pub(super) fn start_eou_install(&mut self, machines_dir: PathBuf, slug: String) {
        let artifacts_root = self.artifacts_root.clone();
        let eou_dir = self.eou_dir.clone();
        let created = Some(created_today());
        let thread_slug = slug.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let dirs = presets::InstallDirs {
                machines_dir: &machines_dir,
                artifacts_root: artifacts_root.as_deref(),
                eou_dir: eou_dir.as_deref(),
            };
            let _ = tx.send(presets::install(
                Preset::NitrOS9Eou,
                &dirs,
                &thread_slug,
                created,
            ));
        });
        self.eou_install = Some(PendingInstall::new(slug, rx));
    }

    /// Once per frame: insert the machine a finished install saved, or
    /// report its error; while it runs, keep frames coming to notice the end.
    pub(super) fn poll_eou_install(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.eou_install else {
            return;
        };
        let saved = match pending.result.try_recv() {
            Ok(saved) => saved,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(PROGRESS_REPAINT_INTERVAL);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("the NitrOS-9 EOU install exited unexpectedly".to_string())
            }
        };
        if let Some(pending) = self.eou_install.take() {
            self.insert_created(pending.slug, saved);
        }
    }

    /// On quit: let a running install finish, so its copy is never cut
    /// short and its definition is saved for the next launch.
    pub(super) fn finish_eou_install(&mut self) {
        if let Some(pending) = self.eou_install.take() {
            let _ = pending.result.recv();
        }
    }

    /// The New menu's NitrOS-9 EOU entry while its install runs.
    pub(super) fn eou_install_ui(ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(COPYING_TEXT);
        });
    }
}

#[cfg(test)]
#[path = "eou_install_test.rs"]
mod tests;
