//! The delete confirmation flow behind the list's "Delete…" (single- or
//! multi-row): the modal itself ([`ManagerApp::draw_delete_confirmation`]) and
//! the confirmed deletion it
//! commits ([`ManagerApp::commit_pending_delete`]) — split out of
//! `lifecycle.rs` to keep that module to the single-machine start/stop/
//! rename lifecycle.

use std::fs;

use eframe::egui;

use crate::machine_def;

use super::{DETAIL_SECTION_GAP, ManagerApp, NO_CONFIG_DIR};

impl ManagerApp {
    /// The confirmation modal behind "Delete…", drawn once per `update()`
    /// while [`ManagerApp::pending_delete`] holds at least one slug. Esc,
    /// Cancel, and a click outside all dismiss without deleting.
    pub(super) fn draw_delete_confirmation(&mut self, ctx: &egui::Context) {
        if self.pending_delete.is_empty() {
            return;
        }
        let indices: Vec<usize> = self
            .pending_delete
            .iter()
            .filter_map(|slug| self.entries.iter().position(|e| &e.slug == slug))
            .collect();
        if indices.is_empty() {
            self.pending_delete.clear();
            return;
        }
        let names: Vec<String> = indices
            .iter()
            .map(|&i| self.entries[i].def.name.clone())
            .collect();
        let any_running = self.bulk_flags(&indices).any_running;
        let any_suspended = indices.iter().any(|&i| self.entries[i].suspended);
        let mut dismissed = false;
        let modal = egui::Modal::new(egui::Id::new("confirm_delete_machine")).show(ctx, |ui| {
            dismissed = self.draw_delete_confirmation_body(ui, &names, any_running, any_suspended);
        });
        if dismissed || modal.should_close() {
            self.pending_delete.clear();
            self.delete_error = None;
        }
    }

    /// The modal's contents: heading, [`draw_delete_warnings`], any error
    /// from a previous attempt, and the confirm/cancel row. Returns whether
    /// Cancel was clicked.
    fn draw_delete_confirmation_body(
        &mut self,
        ui: &mut egui::Ui,
        names: &[String],
        running: bool,
        suspended: bool,
    ) -> bool {
        if let [name] = names {
            ui.heading(format!("Delete “{name}”?"));
        } else {
            ui.heading(format!("Delete {} machines?", names.len()));
        }
        if names.len() > 1 {
            for name in names {
                ui.label(format!("• {name}"));
            }
        }
        ui.add_space(DETAIL_SECTION_GAP);
        ui.label(
            "The machine's definition is removed. Its disk, tape, and other \
             media files stay on disk.",
        );
        draw_delete_warnings(ui, names.len() > 1, running, suspended);
        if let Some(err) = &self.delete_error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }
        ui.add_space(DETAIL_SECTION_GAP);
        let mut dismissed = false;
        ui.horizontal(|ui| {
            let confirm = if running { "Stop and Delete" } else { "Delete" };
            if ui.button(confirm).clicked() {
                self.commit_pending_delete();
            }
            if ui.button("Cancel").clicked() {
                dismissed = true;
            }
        });
        dismissed
    }

    /// Confirmed delete: works through [`ManagerApp::pending_delete`] in
    /// order, resolving each slug to its current index right before
    /// deleting it, since each removal shifts indices after it. On the
    /// first failure, the failed slug and everything after it go back into
    /// `pending_delete` so the still-open modal can retry.
    fn commit_pending_delete(&mut self) {
        let mut slugs = std::mem::take(&mut self.pending_delete);
        for i in 0..slugs.len() {
            let Some(index) = self.entries.iter().position(|e| e.slug == slugs[i]) else {
                continue; // already gone
            };
            if let Err(e) = self.delete_machine(index) {
                self.delete_error = Some(e);
                self.pending_delete = slugs.split_off(i);
                return;
            }
        }
        self.delete_error = None;
    }

    /// Delete `entries[index]`: stop its VM if running, remove its
    /// `<slug>.toml`, drop the row, and fix up selection/edit state.
    /// Media/artifact files are deliberately left on disk.
    fn delete_machine(&mut self, index: usize) -> Result<(), String> {
        let Some(dir) = self.machines_dir.clone() else {
            return Err(NO_CONFIG_DIR.to_string());
        };
        let slug = self.entries[index].slug.clone();
        let path = machine_def::def_path(&dir, &slug);
        self.stop_vm(index);
        // A file already gone is fine — the goal state is reached either way.
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
        self.entries.remove(index);
        self.selection.remove_index(index);
        if self.edit.as_ref().is_some_and(|e| e.slug == slug) {
            self.edit = None;
        }
        Ok(())
    }
}

/// The two state warnings a delete confirmation shows — running machines are
/// shut down first, suspended ones lose their frozen state — worded plural
/// when more than one machine is involved.
fn draw_delete_warnings(ui: &mut egui::Ui, plural: bool, running: bool, suspended: bool) {
    if running {
        let text = if plural {
            "One or more of these machines are running — they will be shut down first, \
             like flipping the power switch; unsaved work inside them is lost."
        } else {
            "This machine is running — it will be shut down first, like flipping the \
             power switch; unsaved work inside it is lost."
        };
        ui.label(egui::RichText::new(text).strong());
    }
    if suspended {
        let text = if plural {
            "One or more of these machines are suspended — deleting discards their \
             frozen state."
        } else {
            "This machine is suspended — deleting discards its frozen state."
        };
        ui.label(egui::RichText::new(text).strong());
    }
}

#[cfg(test)]
#[path = "delete_test.rs"]
mod tests;
