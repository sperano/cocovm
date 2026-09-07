//! The detail/edit pane for the selected machine: drawing the shared
//! [`crate::new_vm::MachineForm`] over a definition and auto-saving every
//! change. The form↔definition mapping that makes that round trip possible
//! ([`ManagerApp::pack_def`], `detail_map::seed_form`) lives in the sibling
//! `manager::detail_map` module — split out once this file grew past the
//! project's ~500-line ceiling.

use eframe::egui;

use crate::{humanize_runtime, machine_def, new_vm, titled_group};

use super::detail_map;
use super::{DETAIL_SECTION_GAP, EditState, ManagerApp, NO_CONFIG_DIR, vm_status_label};

/// One of the pane's two-column form grids ([`new_vm::FORM_GRID_SPACING`],
/// [`new_vm::FORM_LABEL_MIN_WIDTH`]) — the shared floor keeps the sections'
/// combo columns aligned with each other.
fn form_grid(salt: (&str, &str)) -> egui::Grid {
    egui::Grid::new(salt)
        .num_columns(2)
        .spacing(new_vm::FORM_GRID_SPACING)
        .min_col_width(new_vm::FORM_LABEL_MIN_WIDTH)
}

/// The machine form, laid out in sections: Machine, RAM, Display,
/// Peripherals, Ports, Joysticks, and Keyboard — each its own grid, since a
/// `titled_group` can't sit inside a grid row.
fn draw_form_sections(ui: &mut egui::Ui, slug: &str, form: &mut new_vm::MachineForm) {
    titled_group(ui, "Machine", |ui| {
        form_grid(("detail_form_machine", slug)).show(ui, |ui| {
            form.machine_rows(ui);
        });
    });

    ui.add_space(DETAIL_SECTION_GAP);
    titled_group(ui, "RAM", |ui| {
        ui.horizontal(|ui| {
            for &memory in new_vm::ram_choices(form.config.variant) {
                ui.radio_value(&mut form.config.memory, memory, new_vm::ram_label(memory));
            }
        });
    });

    ui.add_space(DETAIL_SECTION_GAP);
    titled_group(ui, "Display", |ui| {
        form_grid(("detail_form_display", slug)).show(ui, |ui| {
            form.display_rows(ui);
        });
    });

    ui.add_space(DETAIL_SECTION_GAP);
    titled_group(ui, "Peripherals", |ui| {
        form_grid(("detail_form_media", slug)).show(ui, |ui| {
            form.media_rows(ui);
        });
    });

    ui.add_space(DETAIL_SECTION_GAP);
    titled_group(ui, "Ports", |ui| {
        form_grid(("detail_form_ports", slug)).show(ui, |ui| {
            form.ports_rows(ui);
        });
    });

    ui.add_space(DETAIL_SECTION_GAP);
    titled_group(ui, "Joysticks", |ui| {
        form.joystick_row(ui);
    });

    ui.add_space(DETAIL_SECTION_GAP);
    titled_group(ui, "Keyboard", |ui| {
        form.keyboard_row(ui);
    });
}

/// How often the detail pane asks for its next repaint while showing a
/// running machine's ticking Runtime row (see [`draw_statistics`]'s call
/// site).
const STATS_REPAINT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// The runtime total to display: a live VM's own `total_runtime` when one
/// exists, else the persisted `[stats].runtime_secs` — the same formula the
/// status bar uses.
fn displayed_runtime_secs(entry: &super::MachineEntry) -> u64 {
    match entry.vm.as_ref() {
        Some(vm) => vm.total_runtime.as_secs(),
        None => entry.def.stats.runtime_secs,
    }
}

/// "Started" row text — `"N times"`, correctly singular for one
/// (`"1 time"`, not "1 times").
fn started_label(starts: u32) -> String {
    if starts == 1 {
        "1 time".to_string()
    } else {
        format!("{starts} times")
    }
}

/// Read-only "Statistics" block: created date (if recorded), cumulative
/// powered-on runtime ([`displayed_runtime_secs`]), and boot count. Never
/// edited here — folding/incrementing happens in `manager::lifecycle`.
fn draw_statistics(ui: &mut egui::Ui, slug: &str, entry: &super::MachineEntry) {
    titled_group(ui, "Statistics", |ui| {
        form_grid(("detail_form_stats", slug)).show(ui, |ui| {
            if let Some(created) = &entry.def.created {
                ui.label("Created");
                ui.label(created);
                ui.end_row();
            }
            ui.label("Runtime");
            ui.label(humanize_runtime(displayed_runtime_secs(entry)));
            ui.end_row();
            ui.label("Started");
            ui.label(started_label(entry.def.stats.starts));
            ui.end_row();
        });
    });
    if entry.is_running() {
        // Native-viewport windows need this to keep Runtime ticking;
        // redundant but harmless under the Embedded fallback.
        ui.ctx().request_repaint_after(STATS_REPAINT_INTERVAL);
    }
}

impl ManagerApp {
    /// Right pane for the selected entry: its edit form.
    pub(super) fn draw_detail(&mut self, ui: &mut egui::Ui, index: usize) {
        let slug = self.entries[index].slug.clone();
        let def = self.entries[index].def.clone();
        self.draw_detail_ok(ui, index, slug, def);
    }

    /// The editable form for a successfully-parsed definition. Split out of
    /// [`Self::draw_detail`] so the `Ok` branch can freely borrow the rest of
    /// `self` (`self.machines_dir`, `self.entries`) while `self.edit` is
    /// temporarily taken out — an `&mut EditDraft` alongside those would
    /// otherwise conflict with the single `&mut self` this method needs.
    fn draw_detail_ok(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        slug: String,
        def: machine_def::MachineDef,
    ) {
        if self.edit.as_ref().is_none_or(|e| e.slug != slug) {
            let mut form = detail_map::seed_form(&def);
            // The auto-save baseline is the seeded form's own repack — see
            // `EditState::packed`'s doc for why it must not be `def`
            // itself. A freshly seeded form holds no Blank picks and at
            // most one pak, so this pack can't fail or touch a file.
            let packed = self
                .pack_def(&def, &slug, &mut form)
                .expect("a seeded form always packs");
            self.edit = Some(EditState {
                slug: slug.clone(),
                name: def.name.clone(),
                form,
                packed,
            });
            self.save_error = None;
        }
        let mut edit = self.edit.take().expect("just ensured above");

        self.draw_name_field(ui, index, &mut edit);
        ui.weak(format!("ID: {slug}"));
        ui.add_space(DETAIL_SECTION_GAP);
        // The transport buttons moved to the toolbar — this pane keeps only
        // the status, plus the last launch failure.
        ui.label(egui::RichText::new(vm_status_label(&self.entries[index])).strong());
        if let Some(err) = &self.entries[index].launch_error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }
        ui.add_space(DETAIL_SECTION_GAP);

        draw_form_sections(ui, &slug, &mut edit.form);
        if self.entries[index].is_alive() {
            ui.add_space(DETAIL_SECTION_GAP);
            // Resume restores the frozen snapshot's hardware wholesale, so
            // for a suspended machine even Resume won't pick edits up —
            // only a cold start from power off does.
            ui.small("Changes apply the next time this machine starts from power off.");
        }

        ui.add_space(DETAIL_SECTION_GAP);
        draw_statistics(ui, &slug, &self.entries[index]);

        self.autosave(&slug, index, &mut edit);
        if let Some(err) = &self.save_error {
            ui.add_space(DETAIL_SECTION_GAP);
            ui.colored_label(ui.visuals().error_fg_color, err);
        }

        self.edit = Some(edit);
    }

    /// The Name field: commits on focus loss/Enter — not per keystroke, so
    /// "C", "Co", "CoC"… aren't each saved (and re-slugified) on the way to
    /// the real name.
    fn draw_name_field(&mut self, ui: &mut egui::Ui, index: usize, edit: &mut EditState) {
        let name_response =
            ui.add(egui::TextEdit::singleline(&mut edit.name).font(egui::TextStyle::Heading));
        if self.focus_name {
            name_response.request_focus();
            self.focus_name = false;
        }
        if name_response.lost_focus() {
            self.commit_name(index, edit);
        }
    }

    /// Auto-save: every change writes straight back to the definition file
    /// (no Save/Revert; the write is atomic,
    /// `machine_def::save`). On a persistent failure this retries every
    /// frame — harmless for a tiny file, and it keeps the error label
    /// current.
    fn autosave(&mut self, slug: &str, index: usize, edit: &mut EditState) {
        match self.pack_def(&self.entries[index].def, slug, &mut edit.form) {
            Ok(new_def) => {
                if new_def != edit.packed {
                    let result = match self.machines_dir.clone() {
                        Some(dir) => machine_def::save(&dir, slug, &new_def),
                        None => Err(NO_CONFIG_DIR.to_string()),
                    };
                    match result {
                        Ok(()) => {
                            self.entries[index].def = new_def.clone();
                            edit.packed = new_def;
                            self.save_error = None;
                        }
                        Err(e) => self.save_error = Some(e),
                    }
                }
            }
            Err(e) => self.save_error = Some(e),
        }
    }

    /// The Name field committed ([`Self::draw_name_field`] — focus left it):
    /// an empty draft reverts to the saved name; a change saves immediately
    /// under the *current* slug, then the file/artifact names follow the
    /// new name using `migrate_slug` — deferred to
    /// [`ManagerApp::apply_pending_renames`] (next frame, or after Stop for
    /// a running machine).
    fn commit_name(&mut self, index: usize, edit: &mut EditState) {
        let trimmed = edit.name.trim().to_string();
        if trimmed.is_empty() || trimmed == self.entries[index].def.name {
            edit.name = self.entries[index].def.name.clone();
            return;
        }
        self.entries[index].def.name = trimmed.clone();
        edit.name = trimmed;
        // Keep the auto-save baseline in step: the name isn't one of the
        // form's fields, and a stale `packed.name` would make the next
        // repack look changed and re-save redundantly.
        edit.packed.name = self.entries[index].def.name.clone();
        let result = match self.machines_dir.clone() {
            Some(dir) => machine_def::save(&dir, &edit.slug, &self.entries[index].def),
            None => Err(NO_CONFIG_DIR.to_string()),
        };
        match result {
            Ok(()) => {
                self.save_error = None;
                self.entries[index].rename_pending = true;
            }
            Err(e) => self.save_error = Some(e),
        }
    }
}

#[cfg(test)]
#[path = "detail_test.rs"]
mod tests;
