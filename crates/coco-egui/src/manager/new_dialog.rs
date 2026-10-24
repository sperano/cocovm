//! The New Machine dialog behind the toolbar's New tile and the New machine
//! hotkey: a choice between an empty machine and one of the templates
//! (`manager/templates.rs`) in name order, created on Create. Follows the
//! delete confirmation's `egui::Modal` pattern (`manager/delete.rs`).
//!
//! A template whose media folder lacks a file it names is listed but off,
//! with the missing files beneath it. When those are the NitrOS-9 EOU
//! images, the dialog offers their download there instead
//! (`manager/eou_menu.rs`). Create closes the dialog at once; reopened
//! while a large copy from a template is still running, the dialog shows a
//! spinner beneath that template instead (`manager/template_install.rs`).

use eframe::egui;

use super::template_install::COPYING_TEXT;
use super::templates::{Template, media};
use super::{DETAIL_SECTION_GAP, ManagerApp};
use crate::nitros9::EOU_IMAGES;

/// The dialog's heading.
pub(crate) const NEW_DIALOG_TITLE: &str = "New Machine";
/// The choice that creates the default machine rather than a template.
pub(crate) const EMPTY_LABEL: &str = "Empty machine";
/// What [`EMPTY_LABEL`] creates: `MachineConfig::default` with an empty
/// cartridge slot and no disks (the `PeripheralsDTO`, `MediaDTO`, and
/// `DriveWireDTO` defaults; pinned by this module's tests).
const EMPTY_DESCRIPTION: &str = "CoCo 3 with 512K and an RGB monitor, no cartridge, and no \
    disks, to set up yourself";
/// The confirm button.
pub(crate) const CREATE_LABEL: &str = "Create";
/// The dismiss button.
pub(crate) const CANCEL_LABEL: &str = "Cancel";
/// Disabled-hover text of Create when the chosen template can't be created.
const UNAVAILABLE_HOVER: &str = "This template can't be created right now";
/// Beneath a template missing files from its media folder, before their names.
const MISSING_PREFIX: &str = "Missing from the template's folder:";
/// In place of the templates when there are none.
pub(crate) const NO_TEMPLATES_TEXT: &str =
    "No templates. To make one, right-click a powered-off machine and choose Save as Template.";

/// Fixed width of the dialog's contents, so the descriptions wrap and the
/// NitrOS-9 EOU download's progress bar keeps one width.
const DIALOG_WIDTH: f32 = 440.0;
/// Space between the dialog's frame and its contents.
const DIALOG_INNER_MARGIN: i8 = 16;
/// Vertical gap between two choices.
const CHOICE_GAP: f32 = 6.0;
/// Height past which the template choices scroll, so a long list keeps
/// the dialog on screen.
const CHOICES_MAX_HEIGHT: f32 = 360.0;

/// The kind of machine the dialog creates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum Choice {
    /// The default machine (`ManagerApp::create_machine_now`).
    #[default]
    Empty,
    /// A machine made from the template with this slug.
    Template(String),
}

/// The open dialog and its current choice.
#[derive(Debug, Default)]
pub(super) struct NewDialog {
    choice: Choice,
}

/// What the dialog's buttons asked for this frame.
enum NewAction {
    Idle,
    Create(Choice),
    Cancel,
}

impl ManagerApp {
    /// "New…" (toolbar tile and hotkey): open the dialog with the empty
    /// machine chosen, or leave an open one as it is.
    pub(super) fn open_new_dialog(&mut self) {
        if self.new_dialog.is_none() {
            self.templates.reload();
            self.new_dialog = Some(NewDialog::default());
        }
    }

    /// The New Machine modal, drawn once per `update()` while
    /// [`ManagerApp::new_dialog`] is `Some`. Esc, Cancel, and a click
    /// outside all dismiss without creating anything.
    pub(super) fn draw_new_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.new_dialog.take() else {
            return;
        };
        let mut action = NewAction::Idle;
        let modal = egui::Modal::new(egui::Id::new("new_machine_dialog"))
            .frame(egui::Frame::popup(&ctx.style()).inner_margin(DIALOG_INNER_MARGIN))
            .show(ctx, |ui| {
                action = self.new_dialog_body(ui, &mut dialog);
            });
        match action {
            NewAction::Create(Choice::Empty) => self.create_machine_now(),
            NewAction::Create(Choice::Template(slug)) => self.create_machine_from_template(&slug),
            NewAction::Idle if !modal.should_close() => self.new_dialog = Some(dialog),
            NewAction::Idle | NewAction::Cancel => {}
        }
    }

    /// The modal's contents: heading, one radio choice per kind of machine,
    /// and the Create/Cancel row.
    fn new_dialog_body(&mut self, ui: &mut egui::Ui, dialog: &mut NewDialog) -> NewAction {
        ui.set_width(DIALOG_WIDTH);
        ui.heading(NEW_DIALOG_TITLE);
        ui.add_space(DETAIL_SECTION_GAP);
        radio_choice(
            ui,
            &mut dialog.choice,
            Choice::Empty,
            EMPTY_LABEL,
            EMPTY_DESCRIPTION,
            true,
        );
        // Taken for the frame, so each choice can borrow `self` mutably;
        // nothing a choice calls reads the list.
        let templates = std::mem::take(&mut self.templates.list);
        let mut creatable = dialog.choice == Choice::Empty;
        egui::ScrollArea::vertical()
            .max_height(CHOICES_MAX_HEIGHT)
            .show(ui, |ui| {
                for template in &templates {
                    ui.add_space(CHOICE_GAP);
                    let available = self.template_choice(ui, &mut dialog.choice, template);
                    if matches!(&dialog.choice, Choice::Template(slug) if *slug == template.slug) {
                        creatable = available;
                    }
                }
            });
        if templates.is_empty() {
            ui.add_space(CHOICE_GAP);
            ui.add(egui::Label::new(egui::RichText::new(NO_TEMPLATES_TEXT).weak()).wrap());
        }
        ui.add_space(DETAIL_SECTION_GAP);
        self.templates.list = templates;
        let mut action = NewAction::Idle;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(creatable, egui::Button::new(CREATE_LABEL))
                .on_disabled_hover_text(UNAVAILABLE_HOVER)
                .clicked()
            {
                action = NewAction::Create(dialog.choice.clone());
            }
            if ui.button(CANCEL_LABEL).clicked() {
                action = NewAction::Cancel;
            }
        });
        action
    }

    /// `template`'s choice, off while it can't be created, with the
    /// spinner of a copy from it, the download of its missing NitrOS-9 EOU
    /// images, or the names of its other missing files beneath it. Returns
    /// whether it can be created now: no file it names is missing, and no
    /// machine made from it is being written.
    fn template_choice(
        &mut self,
        ui: &mut egui::Ui,
        chosen: &mut Choice,
        template: &Template,
    ) -> bool {
        let installing = self.template_installing(&template.slug);
        let missing = self.missing_media(template);
        let creatable = missing.is_empty() && !installing;
        radio_choice(
            ui,
            chosen,
            Choice::Template(template.slug.clone()),
            template.name(),
            &template.summary(),
            creatable,
        );
        if creatable {
            return true;
        }
        ui.indent(&template.slug, |ui| {
            if installing {
                super::template_install::copy_spinner(ui, COPYING_TEXT);
            } else if missing
                .iter()
                .all(|file| EOU_IMAGES.contains(&file.as_str()))
            {
                let media_dir = self.templates.dirs.media_dir(&template.slug);
                self.eou_download_ui(ui, media_dir.as_deref());
            } else {
                let text = format!("{MISSING_PREFIX} {}", missing.join(", "));
                ui.colored_label(ui.visuals().warn_fg_color, text);
            }
        });
        false
    }

    /// The files `template` names that its media folder lacks; all of them
    /// without a data directory.
    fn missing_media(&self, template: &Template) -> Vec<String> {
        match self.templates.dirs.media_dir(&template.slug) {
            Some(dir) => media::missing_files(&template.def, &dir),
            None => media::relative_media(&template.def).files,
        }
    }
}

/// One radio choice, setting `chosen` to `value`, with its `description`
/// beneath it in weak text.
fn radio_choice(
    ui: &mut egui::Ui,
    chosen: &mut Choice,
    value: Choice,
    label: &str,
    description: &str,
    enabled: bool,
) {
    ui.add_enabled_ui(enabled, |ui| {
        ui.radio_value(chosen, value, label);
        ui.indent(label, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(description).weak()).wrap());
        });
    });
}

#[cfg(test)]
#[path = "new_dialog_test.rs"]
mod tests;
