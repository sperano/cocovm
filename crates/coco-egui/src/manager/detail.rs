//! The detail/edit pane for the selected machine: drawing the shared
//! [`crate::new_vm::MachineForm`] over a definition and auto-saving every
//! change. The form↔definition mapping that makes that round trip possible
//! ([`ManagerApp::pack_def`], `detail_map::seed_form`) lives in the sibling
//! `manager::detail_map` module — split out once this file grew past the
//! project's ~500-line ceiling.

use eframe::egui;

use crate::{humanize_runtime, machine_def, new_vm, titled_group};

use super::{
    DETAIL_SECTION_GAP, ManagerApp, NO_CONFIG_DIR, detail_map, roms, thumbnails, vm_status_label,
};

/// The detail pane's working state for the selected entry: the shared
/// [`new_vm::MachineForm`] over its definition, auto-saved on every change
/// (macOS System Settings style — no Save/Revert).
pub(super) struct EditState {
    /// Which entry this state belongs to — a mismatch (a different row was
    /// clicked) means it must be reseeded before it's shown again.
    pub(super) slug: String,
    /// The Name field's draft. Unlike the form, it only commits (saves, and
    /// migrates the slug — [`ManagerApp::commit_name`]) on focus loss/Enter,
    /// so half-typed names aren't saved keystroke by keystroke.
    pub(super) name: String,
    pub(super) form: new_vm::MachineForm,
    /// The definition the form's picks last packed into ([`ManagerApp::pack_def`]) —
    /// the auto-save baseline. Seeded from the freshly seeded form (NOT
    /// from the entry's definition): packing normalizes (explicit `vdg`,
    /// re-seated MPI slots, dropped conflicting flags), and merely
    /// selecting a row must never rewrite a hand-edited file. Only a real
    /// user change makes the repack differ from this and triggers a save.
    pub(super) packed: machine_def::MachineDef,
    /// The ROMs group's rows (`roms::rom_rows`), recomputed when the
    /// definition changes rather than every frame: each row reads and
    /// checksums its file.
    pub(super) roms: Vec<roms::ROMRow>,
}

/// One of the pane's two-column form grids ([`new_vm::FORM_GRID_SPACING`],
/// [`new_vm::FORM_LABEL_MIN_WIDTH`]) — the shared floor keeps the sections'
/// combo columns aligned with each other.
fn form_grid(salt: (&str, &str)) -> egui::Grid {
    egui::Grid::new(salt)
        .num_columns(2)
        .spacing(new_vm::FORM_GRID_SPACING)
        .min_col_width(new_vm::FORM_LABEL_MIN_WIDTH)
}

/// The header's left-column form sections — Machine and RAM
/// ([`ManagerApp::draw_header_with_preview`]).
fn draw_machine_ram_sections(ui: &mut egui::Ui, slug: &str, form: &mut new_vm::MachineForm) {
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
}

/// The full-width sections below the header: Display through Keyboard —
/// each its own grid, since a `titled_group` can't sit inside a grid row.
fn draw_form_sections(ui: &mut egui::Ui, slug: &str, form: &mut new_vm::MachineForm) {
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
            let runtime = ui.label(humanize_runtime(displayed_runtime_secs(entry)));
            if ui.is_rect_visible(runtime.rect)
                && !entry.suspended
                && entry.vm.as_ref().is_some_and(|vm| vm.running)
            {
                crate::app::scheduling::request_repaint_at(
                    ui.ctx(),
                    std::time::Instant::now() + STATS_REPAINT_INTERVAL,
                );
            }
            ui.end_row();
            ui.label("Started");
            ui.label(started_label(entry.def.stats.starts));
            ui.end_row();
        });
    });
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
                roms: roms::rom_rows(&packed, &slug, self.roms_dir.as_deref()),
                packed,
            });
            self.save_error = None;
        }
        let mut edit = self.edit.take().expect("just ensured above");

        // The selected preview stays eligible even when its list row is offscreen.
        self.prepare_detail_thumbnail(&ui.ctx().clone(), index);
        self.draw_header_with_preview(ui, index, &slug, &mut edit);
        ui.add_space(DETAIL_SECTION_GAP);

        draw_form_sections(ui, &slug, &mut edit.form);
        ui.add_space(DETAIL_SECTION_GAP);
        roms::draw_roms(ui, &slug, &edit.roms);
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

    /// The pane's header: identity, status, Machine and RAM in a half-width
    /// left column; the big screen preview on the right spans its height.
    fn draw_header_with_preview(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        slug: &str,
        edit: &mut EditState,
    ) {
        ui.horizontal_top(|ui| {
            let left_width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
            let left = ui.vertical(|ui| {
                ui.set_max_width(left_width);
                self.draw_name_field(ui, index, edit);
                ui.label(egui::RichText::new(format!("Slug ID: {slug}")).strong());
                ui.add_space(DETAIL_SECTION_GAP);
                // The transport buttons moved to the toolbar — this pane keeps
                // only the status, plus the last launch failure.
                ui.label(egui::RichText::new(vm_status_label(&self.entries[index])).strong());
                if let Some(err) = &self.entries[index].launch_error {
                    ui.colored_label(ui.visuals().error_fg_color, err);
                }
                ui.add_space(DETAIL_SECTION_GAP);
                draw_machine_ram_sections(ui, slug, &mut edit.form);
            });
            draw_screen_preview(ui, left.response.rect.height(), &self.entries[index]);
        });
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
                            edit.roms = roms::rom_rows(&new_def, slug, self.roms_dir.as_deref());
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

    /// Commit a Name-field draft. A slug-preserving edit saves only the
    /// definition; a slug-changing edit queues one pre-draw transaction.
    fn commit_name(&mut self, index: usize, edit: &mut EditState) {
        let trimmed = edit.name.trim().to_string();
        if trimmed.is_empty() || trimmed == self.entries[index].def.name {
            edit.name = self.entries[index].def.name.clone();
            return;
        }
        edit.name = trimmed.clone();
        if self.resolved_rename_slug(&self.entries[index].slug, &trimmed)
            != self.entries[index].slug
        {
            self.queue_rename(edit.slug.clone(), trimmed);
            return;
        }
        match self.save_name_only(index, trimmed) {
            Ok(()) => {
                edit.packed.name = self.entries[index].def.name.clone();
                self.save_error = None;
            }
            Err(error) => self.save_error = Some(error),
        }
    }
}

/// The header's screen preview: `height` tall at [`super::THUMBNAIL_ASPECT`],
/// centered in the right half, shrunk ratio-kept when the pane is narrow.
fn draw_screen_preview(ui: &mut egui::Ui, height: f32, entry: &super::MachineEntry) {
    let width = (height * super::THUMBNAIL_ASPECT).min(ui.available_width());
    let height = width / super::THUMBNAIL_ASPECT;
    ui.add_space((ui.available_width() - width) / 2.0);
    let (texture, uv) = thumbnails::preview_source(entry);
    thumbnails::draw_preview(ui, egui::vec2(width, height), texture, uv);
}

#[cfg(test)]
#[path = "detail_test.rs"]
mod tests;
