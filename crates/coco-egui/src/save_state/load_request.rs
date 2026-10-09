//! Loads the user asks for — a quick state from the toolbar or a numbered
//! chord, or a file from Load from File… — checked against this window's
//! machine type first. A state saved on another type would turn this window
//! into that type, so it waits in [`CocoApp::pending_load`] behind a
//! confirmation instead of loading straight away. Resume restores the
//! machine's own suspend file and skips this check.

use std::path::{Path, PathBuf};

use coco_core::snapshot::SnapshotPayload;
use eframe::egui;

use crate::{CocoApp, machine_label};

use super::quick::state_name;
use super::restore::read_state;

/// Confirm button of the machine-type prompt.
pub(crate) const LOAD_ANYWAY: &str = "Load Anyway";
/// Cancel button of the machine-type prompt.
pub(crate) const CANCEL: &str = "Cancel";
/// Space between the prompt's text and its buttons.
const PROMPT_GAP: f32 = 8.0;

/// Where a requested load comes from: names it in the toast and the
/// prompt, and decides whether the State selector follows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LoadSource {
    /// A quick state (0-based); loading it selects it.
    Quick(usize),
    /// A file picked by the user; the selection stays where it is.
    File(PathBuf),
}

impl LoadSource {
    /// "State 2" or the file's name.
    fn name(&self) -> String {
        match self {
            Self::Quick(slot) => state_name(*slot),
            Self::File(path) => file_name(path),
        }
    }
}

/// `path`'s final component for toasts, or the whole path when it has none.
pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// The button the user clicked in the machine-type prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PromptChoice {
    LoadAnyway,
    Cancel,
}

/// A decoded state saved on a different machine type, held until the user
/// confirms or cancels the prompt.
pub(crate) struct PendingLoad {
    source: LoadSource,
    payload: Box<SnapshotPayload>,
}

impl CocoApp {
    /// Load the `.ccstate` at `path` for `source`. A state saved on this
    /// window's machine type loads now; one from another type waits for
    /// the prompt ([`Self::pending_load_ui`]). A read or decode failure
    /// goes to the error dialog.
    pub(crate) fn request_load(&mut self, source: LoadSource, path: &Path, ctx: &egui::Context) {
        match read_state(path) {
            Err(e) => self.cart_error = Some(e),
            Ok(payload) if payload.machine.config.variant != self.machine.config.variant => {
                self.pending_load = Some(PendingLoad {
                    source,
                    payload: Box::new(payload),
                });
            }
            Ok(payload) => self.finish_load(source, payload, ctx),
        }
    }

    /// Restore `payload`: "Loaded <name>" plus any restore notes on
    /// success, selecting a quick state; the error dialog on failure.
    fn finish_load(&mut self, source: LoadSource, payload: SnapshotPayload, ctx: &egui::Context) {
        match self.restore_payload(payload) {
            Ok(notes) => {
                let head = format!("Loaded {}", source.name());
                self.set_toast(super::with_notes(&head, &notes));
                self.refresh_window_title(ctx);
                if let LoadSource::Quick(slot) = source {
                    self.selected_quick_state = slot;
                }
            }
            Err(e) => self.cart_error = Some(e),
        }
    }

    /// The machine-type prompt, drawn while [`Self::pending_load`] holds a
    /// state. Esc, Cancel, and a click outside all drop it unloaded, and so
    /// does a suspend (`app/frame.rs`): loading would diverge the machine
    /// from its frozen state.
    pub(crate) fn pending_load_ui(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_load else {
            return;
        };
        let from = machine_label(pending.payload.machine.config.variant);
        let here = machine_label(self.machine.config.variant);
        let name = pending.source.name();
        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::new("confirm_state_variant")).show(ctx, |ui| {
            choice = variant_prompt(ui, &name, from, here);
        });
        match choice {
            Some(PromptChoice::LoadAnyway) => {
                if let Some(pending) = self.pending_load.take() {
                    self.finish_load(pending.source, *pending.payload, ctx);
                }
            }
            Some(PromptChoice::Cancel) => self.pending_load = None,
            None if modal.should_close() => self.pending_load = None,
            None => {}
        }
    }
}

/// The prompt's contents for state `name`, saved on a `from` machine, about
/// to load into a `here` machine. Returns the button clicked this frame,
/// if any.
fn variant_prompt(ui: &mut egui::Ui, name: &str, from: &str, here: &str) -> Option<PromptChoice> {
    ui.heading(format!("Load a {from} state?"));
    ui.label(format!(
        "{name} was saved on a {from}, and this window runs a {here}. Loading it \
         turns this window into a {from}. The machine's settings don't change."
    ));
    ui.add_space(PROMPT_GAP);
    let mut choice = None;
    ui.horizontal(|ui| {
        if ui.button(LOAD_ANYWAY).clicked() {
            choice = Some(PromptChoice::LoadAnyway);
        }
        if ui.button(CANCEL).clicked() {
            choice = Some(PromptChoice::Cancel);
        }
    });
    choice
}
