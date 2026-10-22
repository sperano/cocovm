//! Creating a NitrOS-9 EOU machine off the UI thread. [`presets::install`]
//! copies the template's 128 MB system disk, which takes a fraction of a
//! second on an SSD but can freeze the window for seconds on a hard disk or
//! a network home directory. So the New menu runs the install on a thread of
//! its own, shows a spinner in place of the template meanwhile, and inserts
//! the machine once it is saved.

use std::path::PathBuf;
use std::sync::mpsc;

use eframe::egui;

use super::asset_downloads::PROGRESS_REPAINT_INTERVAL;
use super::lifecycle::created_today;
use super::presets::{self, Preset};
use super::selection::SelectionSnapshot;
use super::{MachineEntry, ManagerApp};
use crate::machine_def::MachineDef;

/// The New menu's text while the install runs.
pub(crate) const COPYING_TEXT: &str = "Copying the NitrOS-9 EOU disk images…";

/// The error of an install whose thread ended without a result.
const INSTALL_EXITED: &str = "the NitrOS-9 EOU install exited unexpectedly";

/// A NitrOS-9 EOU machine being written on its own thread.
pub(crate) struct PendingInstall {
    /// The slug the machine is saved under.
    slug: String,
    /// The list's selection when the install started. If it has changed by
    /// the end, the user has moved on, and the new machine doesn't take the
    /// selection.
    selection: SelectionSnapshot,
    /// The install's outcome, once it has one.
    result: mpsc::Receiver<Result<MachineDef, String>>,
}

impl PendingInstall {
    /// An install of `slug`, started with nothing selected, whose outcome
    /// arrives on `result`.
    pub(crate) fn new(slug: String, result: mpsc::Receiver<Result<MachineDef, String>>) -> Self {
        Self {
            slug,
            selection: SelectionSnapshot::default(),
            result,
        }
    }
}

impl ManagerApp {
    /// Install the NitrOS-9 EOU template as machine `slug` under
    /// `machines_dir` on a new thread; [`Self::poll_eou_install`] inserts it.
    /// Does nothing while another install runs (the New menu hides the
    /// template meanwhile).
    pub(super) fn start_eou_install(&mut self, machines_dir: PathBuf, slug: String) {
        if self.eou_install.is_some() {
            return;
        }
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
        self.eou_install = Some(PendingInstall {
            selection: self.selection.snapshot(&self.entries),
            ..PendingInstall::new(slug, rx)
        });
    }

    /// The slug a running install writes, which no other machine may take.
    pub(super) fn installing_slug(&self) -> Option<&str> {
        self.eou_install
            .as_ref()
            .map(|pending| pending.slug.as_str())
    }

    /// Once per frame: insert the machine a finished install saved, or
    /// report its error; while it runs, keep frames coming to notice the end.
    pub(super) fn poll_eou_install(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.eou_install.take() else {
            return;
        };
        let saved = match pending.result.try_recv() {
            Ok(saved) => saved,
            Err(mpsc::TryRecvError::Empty) => {
                self.eou_install = Some(pending);
                ctx.request_repaint_after(PROGRESS_REPAINT_INTERVAL);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(INSTALL_EXITED.into()),
        };
        self.insert_installed(pending, saved);
    }

    /// Insert the machine `pending` `saved` like any new machine, unless
    /// the user moved on while it was written (selected something else, or
    /// is typing a name): then list it without selecting it.
    fn insert_installed(&mut self, pending: PendingInstall, saved: Result<MachineDef, String>) {
        let moved_on = self.selection.snapshot(&self.entries) != pending.selection
            || self.name_draft_pending();
        match saved {
            Ok(def) if moved_on => {
                self.entries.push(MachineEntry::new(pending.slug, def));
                self.apply_manager_sort(self.manager_sort);
            }
            saved => self.insert_created(pending.slug, saved),
        }
    }

    /// Whether the Name field holds a draft not committed yet, which
    /// selecting another machine would drop.
    fn name_draft_pending(&self) -> bool {
        self.edit.as_ref().is_some_and(|edit| {
            self.entries
                .iter()
                .any(|entry| entry.slug == edit.slug && entry.def.name != edit.name)
        })
    }

    /// On quit: let a running install finish, so its copy is never cut
    /// short and, unless it fails, its definition is saved for the next
    /// launch. The window stays up, unresponsive, until the copy is done.
    pub(super) fn finish_eou_install(&mut self) {
        let Some(pending) = self.eou_install.take() else {
            return;
        };
        let error = match pending.result.recv() {
            Ok(Ok(_)) => return,
            Ok(Err(error)) => error,
            Err(mpsc::RecvError) => INSTALL_EXITED.to_string(),
        };
        eprintln!("coco: creating machine '{}': {error}", pending.slug);
    }
}

/// The New menu's NitrOS-9 EOU entry while its install runs.
pub(super) fn eou_install_ui(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(COPYING_TEXT);
    });
}

#[cfg(test)]
#[path = "eou_install_test.rs"]
mod tests;
