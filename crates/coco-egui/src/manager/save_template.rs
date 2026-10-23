//! "Save as Template…" in a powered-off machine's context menu: the modal
//! that stores a snapshot of the machine as a new template
//! (`manager/templates.rs`). The machine itself is unchanged.
//!
//! The snapshot keeps the definition's settings, takes the name and
//! description typed here, is dated today, and starts with zeroed
//! `[stats]` ([`snapshot`]). The files the machine names in its own data
//! folder are copied into the template's media folder; files elsewhere
//! stay shared references (`templates/media.rs`). The suspend state,
//! thumbnail, and print jobs are never copied. A name another template
//! already has turns Save into Replace, which writes the new template
//! first and deletes the old one only once that succeeded.

use std::path::PathBuf;
use std::sync::mpsc;

use eframe::egui;

use super::asset_downloads::PROGRESS_REPAINT_INTERVAL;
use super::lifecycle::created_today;
use super::templates::media::{self, MediaCopy, relative_media};
use super::templates::{self, Template, TemplateDirs};
use super::{DETAIL_SECTION_GAP, ManagerApp, NO_CONFIG_DIR, NO_DATA_DIR};
use crate::machine_def::{self, MachineDef};

mod snapshot;
pub(crate) use snapshot::{FileFate, FileLine, file_lines, snapshot};

/// The context menu item.
pub(crate) const SAVE_TEMPLATE_LABEL: &str = "Save as Template…";
/// Its disabled-hover text while the machine runs or is suspended: the
/// disks on the host may not hold what the VM has in memory.
pub(crate) const STOP_FIRST_HOVER: &str = "Stop the machine first";
/// The dialog's field labels.
pub(crate) const NAME_LABEL: &str = "Template name";
pub(crate) const DESCRIPTION_LABEL: &str = "Description";
/// The confirm button, and its label when the name is taken.
pub(crate) const SAVE_LABEL: &str = "Save";
pub(crate) const REPLACE_LABEL: &str = "Replace";
/// The dismiss button.
pub(crate) const CANCEL_LABEL: &str = "Cancel";
/// Shown while a large copy runs.
pub(crate) const COPYING_TEXT: &str = "Copying the disk images into the template…";

/// Fixed width of the dialog's contents.
const DIALOG_WIDTH: f32 = 440.0;
/// Space between the dialog's frame and its contents.
const DIALOG_INNER_MARGIN: i8 = 16;
/// Rows of the description field.
const DESCRIPTION_ROWS: usize = 3;

/// The open dialog.
pub(crate) struct SaveTemplateDialog {
    /// The machine's display name, for the heading.
    machine_name: String,
    /// The machine's definition when the dialog opened.
    def: MachineDef,
    /// The machine's data folder, where its relative files are.
    artifact_dir: Option<PathBuf>,
    /// The Name field.
    pub(crate) name: String,
    /// The Description field.
    pub(crate) description: String,
    /// What saving does with each file.
    files: Vec<FileLine>,
    /// The last failed save's error.
    error: Option<String>,
    /// A large copy running on its own thread.
    pending: Option<mpsc::Receiver<Result<(), String>>>,
}

/// What the dialog's buttons asked for this frame.
enum DialogAction {
    Idle,
    Save,
    Cancel,
}

impl SaveTemplateDialog {
    /// The dialog for machine `def`, whose data folder is `artifact_dir`.
    fn new(def: &MachineDef, artifact_dir: Option<PathBuf>) -> Self {
        let preview = snapshot(def, artifact_dir.as_deref(), def.name.clone(), None);
        Self {
            machine_name: def.name.clone(),
            def: def.clone(),
            files: file_lines(&preview, artifact_dir.as_deref()),
            artifact_dir,
            name: def.name.clone(),
            description: String::new(),
            error: None,
            pending: None,
        }
    }

    /// The template the name field names already, which Save replaces.
    fn clash<'a>(&self, templates: &'a [Template]) -> Option<&'a Template> {
        let name = self.name.trim();
        templates
            .iter()
            .find(|template| template.name().trim().eq_ignore_ascii_case(name))
    }
}

impl ManagerApp {
    /// The context menu's "Save as Template…" for machine `slug`: opened
    /// before the next frame draws ([`Self::open_requested_save_template`]).
    pub(super) fn request_save_template(&mut self, slug: String) {
        self.templates.save_request = Some(slug);
    }

    /// Open the dialog a [`Self::request_save_template`] asked for. Runs
    /// before any panel draws, so a Name-field draft can be committed
    /// first, renaming the machine if its slug changes. A machine started
    /// since the request is left alone.
    pub(super) fn open_requested_save_template(&mut self) {
        let Some(mut slug) = self.templates.save_request.take() else {
            return;
        };
        if let Some(mut edit) = self.edit.take_if(|edit| edit.slug == slug) {
            if let Some(index) = self.entry_index(&slug) {
                self.commit_name(index, &mut edit);
            }
            self.edit = Some(edit);
            self.apply_pending_rename();
            // A rename rekeys the edit state to the new slug.
            if let Some(edit) = &self.edit {
                slug.clone_from(&edit.slug);
            }
        }
        let Some(entry) = self.entries.iter().find(|entry| entry.slug == slug) else {
            return;
        };
        if entry.is_alive() {
            return;
        }
        let artifact_dir = self.artifacts_root.as_ref().map(|root| root.join(&slug));
        self.templates.reload();
        self.templates.save_dialog = Some(SaveTemplateDialog::new(&entry.def, artifact_dir));
    }

    /// The Save as Template modal, drawn once per `update()` while it is
    /// open. Esc, Cancel, and a click outside dismiss it, except while a
    /// copy runs.
    pub(super) fn draw_save_template_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.templates.save_dialog.take() else {
            return;
        };
        if self.poll_save_template(ctx, &mut dialog) {
            return;
        }
        let mut action = DialogAction::Idle;
        let modal = egui::Modal::new(egui::Id::new("save_template_dialog"))
            .frame(egui::Frame::popup(&ctx.style()).inner_margin(DIALOG_INNER_MARGIN))
            .show(ctx, |ui| {
                action = self.save_template_body(ui, &mut dialog);
            });
        let busy = dialog.pending.is_some();
        match action {
            DialogAction::Save => {
                if !self.commit_save_template(&mut dialog) {
                    self.templates.save_dialog = Some(dialog);
                }
            }
            DialogAction::Cancel => {}
            DialogAction::Idle if busy || !modal.should_close() => {
                self.templates.save_dialog = Some(dialog);
            }
            DialogAction::Idle => {}
        }
    }

    /// Take the result of `dialog`'s running copy, if it has one: success
    /// closes the dialog (returns true), a failure shows in it.
    fn poll_save_template(&mut self, ctx: &egui::Context, dialog: &mut SaveTemplateDialog) -> bool {
        let Some(pending) = &dialog.pending else {
            return false;
        };
        let result = match pending.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(PROGRESS_REPAINT_INTERVAL);
                return false;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(COPY_EXITED.to_string()),
        };
        dialog.pending = None;
        self.finish_save_template(dialog, result)
    }

    /// Close the dialog after a save that worked and list the new
    /// template (returns true), or keep it open with the error.
    fn finish_save_template(
        &mut self,
        dialog: &mut SaveTemplateDialog,
        result: Result<(), String>,
    ) -> bool {
        self.templates.reload();
        match result {
            Ok(()) => true,
            Err(error) => {
                dialog.error = Some(error);
                false
            }
        }
    }

    /// The modal's contents. Returns what its buttons asked for.
    fn save_template_body(
        &self,
        ui: &mut egui::Ui,
        dialog: &mut SaveTemplateDialog,
    ) -> DialogAction {
        ui.set_width(DIALOG_WIDTH);
        ui.heading(format!("Save “{}” as a Template", dialog.machine_name));
        ui.add_space(DETAIL_SECTION_GAP);
        let busy = dialog.pending.is_some();
        ui.add_enabled_ui(!busy, |ui| draw_fields(ui, dialog));
        ui.add_space(DETAIL_SECTION_GAP);
        draw_file_lines(ui, &dialog.files);
        let clash = dialog.clash(&self.templates.list);
        if let Some(template) = clash {
            ui.add_space(DETAIL_SECTION_GAP);
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "A template named “{}” already exists. Replace deletes it and saves this one.",
                    template.name()
                ),
            );
        }
        if let Some(error) = &dialog.error {
            ui.add_space(DETAIL_SECTION_GAP);
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.add_space(DETAIL_SECTION_GAP);
        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(COPYING_TEXT);
            });
            return DialogAction::Idle;
        }
        let missing = dialog
            .files
            .iter()
            .any(|line| line.fate == FileFate::Missing);
        let savable = !dialog.name.trim().is_empty() && !missing;
        draw_buttons(ui, savable, clash.is_some())
    }

    /// Save `dialog`'s template: claim its slug and media folder, then copy
    /// the machine's files and write the file, on a thread of its own when
    /// the copy is large. Returns whether the save is done, so the dialog
    /// closes; a failure shows in it.
    fn commit_save_template(&mut self, dialog: &mut SaveTemplateDialog) -> bool {
        dialog.error = None;
        let replaced = dialog.clash(&self.templates.list).map(|t| t.slug.clone());
        let job = replaced
            .as_deref()
            .map_or(Ok(()), |slug| self.check_template_unused(slug))
            .and_then(|()| self.prepare_save_template(dialog, replaced));
        let job = match job {
            Ok(job) => job,
            Err(error) => {
                dialog.error = Some(error);
                return false;
            }
        };
        if job.copy.in_background() {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(job.run());
            });
            dialog.pending = Some(rx);
            return false;
        }
        let result = job.run();
        self.finish_save_template(dialog, result)
    }

    /// Claim a slug and media folder for `dialog`'s template and plan the
    /// work that fills them.
    fn prepare_save_template(
        &self,
        dialog: &SaveTemplateDialog,
        replaced: Option<String>,
    ) -> Result<SaveJob, String> {
        let dirs = self.templates.dirs.clone();
        let defs = dirs.defs.clone().ok_or(NO_CONFIG_DIR)?;
        let media_root = dirs.media_root.clone().ok_or(NO_DATA_DIR)?;
        let name = dialog.name.trim().to_string();
        let def = snapshot(
            &dialog.def,
            dialog.artifact_dir.as_deref(),
            name.clone(),
            Some(created_today()),
        );
        let from = match &dialog.artifact_dir {
            Some(dir) => dir.clone(),
            None if relative_media(&def) == media::RelativeMedia::default() => PathBuf::new(),
            None => return Err(NO_DATA_DIR.to_string()),
        };
        let slug = media::claim(&media_root, &machine_def::slugify(&name), &|candidate| {
            dirs.slug_taken(candidate)
        })?;
        let to = media_root.join(&slug);
        let copy = MediaCopy::new(&def, from, to.clone()).inspect_err(|_| media::discard(&to))?;
        Ok(SaveJob {
            copy,
            defs,
            template: Template {
                slug,
                def,
                description: dialog.description.trim().to_string(),
            },
            dirs,
            replaced,
        })
    }

    /// On quit: let a running save finish, so its copy is never cut short.
    pub(super) fn finish_template_save(&mut self) {
        let Some(pending) = self
            .templates
            .save_dialog
            .take()
            .and_then(|dialog| dialog.pending)
        else {
            return;
        };
        if let Ok(Err(error)) = pending.recv() {
            eprintln!("coco: saving template: {error}");
        }
    }
}

/// The error of a save whose thread ended without a result.
const COPY_EXITED: &str = "the template copy exited unexpectedly";

/// The work behind Save: copy, write, and delete the replaced template.
struct SaveJob {
    copy: MediaCopy,
    defs: PathBuf,
    template: Template,
    dirs: TemplateDirs,
    replaced: Option<String>,
}

impl SaveJob {
    /// Copy the media, then write the template, so a failure never leaves a
    /// template without its files; a failure removes the claimed folder.
    /// Then delete the replaced template.
    fn run(self) -> Result<(), String> {
        let saved = self
            .copy
            .run()
            .and_then(|()| templates::save(&self.defs, &self.template));
        if saved.is_err() {
            media::discard(&self.copy.to);
        }
        saved?;
        match &self.replaced {
            Some(slug) => templates::delete(&self.dirs, slug).map_err(|error| {
                format!("The template is saved, but the one it replaces is not deleted: {error}")
            }),
            None => Ok(()),
        }
    }
}

/// Save (Replace when `replaces`, on while `savable`) and Cancel.
fn draw_buttons(ui: &mut egui::Ui, savable: bool, replaces: bool) -> DialogAction {
    let confirm = if replaces { REPLACE_LABEL } else { SAVE_LABEL };
    let mut action = DialogAction::Idle;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(savable, egui::Button::new(confirm))
            .clicked()
        {
            action = DialogAction::Save;
        }
        if ui.button(CANCEL_LABEL).clicked() {
            action = DialogAction::Cancel;
        }
    });
    action
}

/// The Name and Description fields.
fn draw_fields(ui: &mut egui::Ui, dialog: &mut SaveTemplateDialog) {
    let label = ui.label(NAME_LABEL);
    ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(f32::INFINITY))
        .labelled_by(label.id);
    ui.add_space(DETAIL_SECTION_GAP);
    let label = ui.label(DESCRIPTION_LABEL);
    ui.add(
        egui::TextEdit::multiline(&mut dialog.description)
            .desired_rows(DESCRIPTION_ROWS)
            .desired_width(f32::INFINITY),
    )
    .labelled_by(label.id);
}

/// What saving does with each file, a missing one in the error color.
fn draw_file_lines(ui: &mut egui::Ui, files: &[FileLine]) {
    if files.is_empty() {
        ui.weak("The machine names no files.");
        return;
    }
    ui.label("Files");
    for line in files {
        let text = line.text();
        if line.fate == FileFate::Missing {
            ui.colored_label(ui.visuals().error_fg_color, text);
        } else {
            ui.add(egui::Label::new(egui::RichText::new(text).weak()).wrap());
        }
    }
}

#[cfg(test)]
#[path = "save_template_test.rs"]
mod tests;
