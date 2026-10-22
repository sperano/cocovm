//! The New Machine dialog behind the toolbar's New tile and the New machine
//! hotkey: a choice between an empty machine and one of the built-in
//! templates (`manager/presets.rs`), created on Create. Follows the delete
//! confirmation's `egui::Modal` pattern (`manager/delete.rs`).
//!
//! The NitrOS-9 EOU template needs downloaded images: while they are
//! missing, its choice is off and the dialog offers their download beneath
//! it (`manager/eou_menu.rs`). Create closes the dialog at once; reopened
//! while a machine made from that template is still being written, the
//! dialog shows a spinner there instead (`manager/eou_install.rs`).

use eframe::egui;

use super::presets::Preset;
use super::{DETAIL_SECTION_GAP, ManagerApp};

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

/// Fixed width of the dialog's contents, so the descriptions wrap and the
/// NitrOS-9 EOU download's progress bar keeps one width.
const DIALOG_WIDTH: f32 = 440.0;
/// Space between the dialog's frame and its contents.
const DIALOG_INNER_MARGIN: i8 = 16;
/// Vertical gap between two choices.
const CHOICE_GAP: f32 = 6.0;

/// The kind of machine the dialog creates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Choice {
    /// The default machine (`ManagerApp::create_machine_now`).
    #[default]
    Empty,
    /// A machine built from a built-in template.
    Template(Preset),
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
        self.new_dialog.get_or_insert_default();
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
            NewAction::Create(Choice::Template(preset)) => self.create_machine_from_preset(preset),
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
        for preset in Preset::ALL {
            ui.add_space(CHOICE_GAP);
            self.template_choice(ui, &mut dialog.choice, preset);
        }
        ui.add_space(DETAIL_SECTION_GAP);
        let creatable = match dialog.choice {
            Choice::Empty => true,
            Choice::Template(preset) => self.template_creatable(preset),
        };
        let mut action = NewAction::Idle;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(creatable, egui::Button::new(CREATE_LABEL))
                .on_disabled_hover_text(UNAVAILABLE_HOVER)
                .clicked()
            {
                action = NewAction::Create(dialog.choice);
            }
            if ui.button(CANCEL_LABEL).clicked() {
                action = NewAction::Cancel;
            }
        });
        action
    }

    /// `preset`'s choice, off while it can't be created, with the NitrOS-9
    /// EOU install's spinner or the download of its images beneath it.
    fn template_choice(&mut self, ui: &mut egui::Ui, chosen: &mut Choice, preset: Preset) {
        let creatable = self.template_creatable(preset);
        radio_choice(
            ui,
            chosen,
            Choice::Template(preset),
            preset.name(),
            preset.description(),
            creatable,
        );
        if creatable {
            return;
        }
        ui.indent(preset.name(), |ui| {
            if self.eou_installing(preset) {
                super::eou_install::eou_install_ui(ui);
            } else {
                self.eou_download_ui(ui);
            }
        });
    }

    /// Whether `preset` can be created now: its images are in, and it isn't
    /// the NitrOS-9 EOU template while a machine made from it is being
    /// written.
    fn template_creatable(&self, preset: Preset) -> bool {
        preset.available(self.eou_dir.as_deref()) && !self.eou_installing(preset)
    }

    /// Whether `preset` is NitrOS-9 EOU and a machine made from it is being
    /// written.
    fn eou_installing(&self, preset: Preset) -> bool {
        preset == Preset::NitrOS9Eou && self.eou_install.is_some()
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
