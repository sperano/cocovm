//! Creating a machine from a template (`manager/templates.rs`): the New
//! Machine dialog's Create. The machine gets its own copy of the template's
//! relative media (`templates/media.rs`). A copy larger than
//! [`media::BACKGROUND_COPY_BYTES`] (the NitrOS-9 EOU template's 128 MB
//! system disk) runs on a thread of its own, since it can freeze the
//! window for seconds on a hard disk or a network home directory; the
//! dialog shows a spinner beneath the template when reopened meanwhile,
//! and the machine is inserted once it is saved.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use eframe::egui;

use super::asset_downloads::PROGRESS_REPAINT_INTERVAL;
use super::lifecycle::created_today;
use super::selection::SelectionSnapshot;
use super::templates::media::{self, MediaCopy};
use super::{MachineEntry, ManagerApp, NO_CONFIG_DIR, NO_DATA_DIR};
use crate::machine_def::{self, MachineDef};

/// The New Machine dialog's text beneath a template while a machine made
/// from it is copied.
pub(crate) const COPYING_TEXT: &str = "Copying the template's disk images…";

/// Why Create failed for a template that is no longer listed.
const TEMPLATE_GONE: &str = "the template no longer exists";

/// The error of an install whose thread ended without a result.
const INSTALL_EXITED: &str = "the template copy exited unexpectedly";

/// A machine being written from a template on its own thread.
pub(crate) struct PendingInstall {
    /// The slug the machine is saved under.
    slug: String,
    /// The slug of the template it is made from.
    template: String,
    /// The list's selection when the install started. If it has changed by
    /// the end, the user has moved on, and the new machine doesn't take the
    /// selection.
    selection: SelectionSnapshot,
    /// The install's outcome, once it has one.
    result: mpsc::Receiver<Result<MachineDef, String>>,
}

impl PendingInstall {
    /// An install of machine `slug` from `template`, started with nothing
    /// selected, whose outcome arrives on `result`.
    pub(crate) fn new(
        slug: String,
        template: String,
        result: mpsc::Receiver<Result<MachineDef, String>>,
    ) -> Self {
        Self {
            slug,
            template,
            selection: SelectionSnapshot::default(),
            result,
        }
    }
}

/// Copy `copy`, then save `def` as machine `slug` in `machines_dir`, so a
/// failure never leaves a definition pointing at a missing image. A failure
/// removes the artifact dir the copy went into, which the caller claimed.
fn install(
    copy: &MediaCopy,
    machines_dir: &Path,
    slug: &str,
    def: MachineDef,
) -> Result<MachineDef, String> {
    let saved = copy
        .run()
        .and_then(|()| machine_def::save(machines_dir, slug, &def));
    if saved.is_err() {
        media::discard(&copy.to);
    }
    saved.map(|()| def)
}

impl ManagerApp {
    /// The New Machine dialog's templates: create, save, insert, and select
    /// a machine made from template `template_slug`, named after it, with
    /// its own copy of the template's media.
    pub(super) fn create_machine_from_template(&mut self, template_slug: &str) {
        let Some(machines_dir) = self.machines_dir.clone() else {
            self.save_error = Some(NO_CONFIG_DIR.to_string());
            return;
        };
        let Some(template) = self.templates.find(template_slug) else {
            self.save_error = Some(TEMPLATE_GONE.to_string());
            return;
        };
        let def = template.machine_def(Some(created_today()));
        let base = machine_def::slugify(template.name());
        if media::relative_media(&template.def).is_empty() {
            let slug = machine_def::unique_slug(&base, &|candidate| {
                self.slug_taken(&machines_dir, candidate)
            });
            let saved = machine_def::save(&machines_dir, &slug, &def).map(|()| def);
            self.insert_created(slug, saved);
            return;
        }
        let (Some(root), Some(media_dir)) = (
            self.artifacts_root.clone(),
            self.templates.dirs.media_dir(template_slug),
        ) else {
            self.save_error = Some(NO_DATA_DIR.to_string());
            return;
        };
        match self.prepare_install(&machines_dir, &root, &base, &template.def, media_dir) {
            Ok((slug, copy)) if copy.in_background() => {
                self.start_install(machines_dir, slug, template_slug, copy, def);
            }
            Ok((slug, copy)) => {
                let saved = install(&copy, &machines_dir, &slug, def);
                self.insert_created(slug, saved);
            }
            Err(error) => self.save_error = Some(error),
        }
    }

    /// Claim the new machine's slug and artifact dir under `root`
    /// ([`media::claim`]), and plan the copy of `template_def`'s media from
    /// `media_dir` into it.
    fn prepare_install(
        &self,
        machines_dir: &Path,
        root: &Path,
        base: &str,
        template_def: &MachineDef,
        media_dir: PathBuf,
    ) -> Result<(String, MediaCopy), String> {
        let slug = media::claim(root, base, &|candidate| {
            self.slug_taken(machines_dir, candidate)
        })?;
        let artifact_dir = root.join(&slug);
        MediaCopy::new(template_def, media_dir, artifact_dir.clone())
            .map(|copy| (slug, copy))
            .inspect_err(|_| media::discard(&artifact_dir))
    }

    /// Run [`install`] on a new thread; [`Self::poll_template_installs`]
    /// inserts the machine.
    fn start_install(
        &mut self,
        machines_dir: PathBuf,
        slug: String,
        template_slug: &str,
        copy: MediaCopy,
        def: MachineDef,
    ) {
        let thread_slug = slug.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(install(&copy, &machines_dir, &thread_slug, def));
        });
        self.templates.installs.push(PendingInstall {
            selection: self.selection.snapshot(&self.entries),
            ..PendingInstall::new(slug, template_slug.to_string(), rx)
        });
    }

    /// Whether `slug` is the slug a running install writes, which no other
    /// machine may take.
    pub(super) fn installing_slug(&self, slug: &str) -> bool {
        self.templates
            .installs
            .iter()
            .any(|pending| pending.slug == slug)
    }

    /// Whether a machine made from template `template_slug` is being written.
    pub(super) fn template_installing(&self, template_slug: &str) -> bool {
        self.templates
            .installs
            .iter()
            .any(|pending| pending.template == template_slug)
    }

    /// Once per frame: insert the machines finished installs saved, or
    /// report their errors; while any runs, keep frames coming to notice
    /// the end.
    pub(super) fn poll_template_installs(&mut self, ctx: &egui::Context) {
        for pending in std::mem::take(&mut self.templates.installs) {
            let saved = match pending.result.try_recv() {
                Ok(saved) => saved,
                Err(mpsc::TryRecvError::Empty) => {
                    self.templates.installs.push(pending);
                    continue;
                }
                Err(mpsc::TryRecvError::Disconnected) => Err(self.exited(&pending.slug)),
            };
            self.insert_installed(pending, saved);
        }
        if !self.templates.installs.is_empty() {
            ctx.request_repaint_after(PROGRESS_REPAINT_INTERVAL);
        }
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
    pub(super) fn name_draft_pending(&self) -> bool {
        self.edit.as_ref().is_some_and(|edit| {
            self.entries
                .iter()
                .any(|entry| entry.slug == edit.slug && entry.def.name != edit.name)
        })
    }

    /// On quit: let running installs finish, so no copy is cut short and,
    /// unless one fails, its definition is saved for the next launch. The
    /// window stays up, unresponsive, until the copies are done.
    pub(super) fn finish_template_installs(&mut self) {
        for pending in std::mem::take(&mut self.templates.installs) {
            let error = match pending.result.recv() {
                Ok(Ok(_)) => continue,
                Ok(Err(error)) => error,
                Err(mpsc::RecvError) => self.exited(&pending.slug),
            };
            eprintln!("coco: creating machine '{}': {error}", pending.slug);
        }
    }

    /// The error of `slug`'s install whose thread ended without a result
    /// (it panicked), after removing the artifact dir it claimed, which
    /// would otherwise keep `slug` taken.
    fn exited(&self, slug: &str) -> String {
        if let Some(root) = &self.artifacts_root {
            media::discard(&root.join(slug));
        }
        INSTALL_EXITED.to_string()
    }
}

/// A spinner with `text` beside it, while a copy runs: beneath a template
/// in the New Machine dialog, and in the Save as Template dialog.
pub(super) fn copy_spinner(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(text);
    });
}

#[cfg(test)]
#[path = "template_install_test.rs"]
mod tests;
