//! The detail/edit pane for the selected machine: drawing the shared
//! [`crate::new_vm::MachineForm`] over a definition and auto-saving every
//! change. The form↔definition mapping that makes that round trip possible
//! ([`ManagerApp::pack_def`], `detail_map::seed_form`) lives in the sibling
//! `manager::detail_map` module — split out once this file grew past the
//! project's ~500-line ceiling.

use eframe::egui;

use crate::{humanize_runtime, machine_def, new_vm, titled_group};

use super::{
    DETAIL_SECTION_GAP, ManagerApp, NO_CONFIG_DIR, detail_map, live_drivewire, roms, thumbnails,
    vm_status_label,
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
    /// The active properties tab. The edit draft stays shared across tabs.
    tab: DetailTab,
}

/// The five persistent categories in the machine properties pane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DetailTab {
    #[default]
    General,
    Display,
    Devices,
    Input,
    DriveWire,
}

impl DetailTab {
    const ALL: [Self; 5] = [
        Self::General,
        Self::Display,
        Self::Devices,
        Self::Input,
        Self::DriveWire,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Display => "Display",
            Self::Devices => "Devices",
            Self::Input => "Input",
            Self::DriveWire => "DriveWire",
        }
    }
}

/// Height of each property tab's clickable face.
const DETAIL_TAB_HEIGHT: f32 = 30.0;
/// Gap between adjacent property tabs. A narrow gap keeps the labels grouped
/// as one navigation strip without turning them into a segmented control.
const DETAIL_TAB_GAP: f32 = 2.0;
/// Width of the accent rule beneath the active property tab.
const DETAIL_TAB_INDICATOR_WIDTH: f32 = 2.0;

/// One of the pane's two-column form grids ([`new_vm::FORM_GRID_SPACING`],
/// [`new_vm::FORM_LABEL_MIN_WIDTH`]) — the shared floor keeps the sections'
/// combo columns aligned with each other.
fn form_grid(salt: (&str, &str)) -> egui::Grid {
    egui::Grid::new(salt)
        .num_columns(2)
        .spacing(new_vm::FORM_GRID_SPACING)
        .min_col_width(new_vm::FORM_LABEL_MIN_WIDTH)
}

/// One property tab. The active label gets the strip's accent color and
/// underline; idle labels remain unframed until hover.
fn detail_tab(ui: &mut egui::Ui, tab: &mut DetailTab, choice: DetailTab) -> egui::Response {
    let selected = *tab == choice;
    let label = if selected {
        egui::RichText::new(choice.label())
            .strong()
            .color(ui.visuals().selection.stroke.color)
    } else {
        egui::RichText::new(choice.label())
    };
    let button = egui::Button::new(label)
        .min_size(egui::vec2(0.0, DETAIL_TAB_HEIGHT))
        .frame(true)
        .frame_when_inactive(false);
    let mut response = ui.add(button);
    if response.clicked() && !selected {
        *tab = choice;
        response.mark_changed();
    }
    let active = *tab == choice;
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::RadioButton,
            ui.is_enabled(),
            active,
            choice.label(),
        )
    });
    response
}

/// The persistent selector for the five property categories. A baseline ties
/// the labels together, while an accent rule marks the active tab. Wrapping
/// keeps every category reachable in a narrow manager window.
fn draw_tab_bar(ui: &mut egui::Ui, tab: &mut DetailTab) {
    let available_width = ui.available_width();
    let mut selected_rect = None;
    let bar = ui.horizontal_wrapped(|ui| {
        ui.set_min_width(available_width);
        ui.spacing_mut().item_spacing.x = DETAIL_TAB_GAP;
        for choice in DetailTab::ALL {
            let response = detail_tab(ui, tab, choice);
            if *tab == choice {
                selected_rect = Some(response.rect);
            }
        }
    });
    bar.response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::RadioGroup,
            ui.is_enabled(),
            "Machine properties",
        )
    });
    let baseline = ui.visuals().widgets.noninteractive.bg_stroke;
    let baseline_y = bar.response.rect.bottom();
    ui.painter().line_segment(
        [
            egui::pos2(bar.response.rect.left(), baseline_y),
            egui::pos2(bar.response.rect.right(), baseline_y),
        ],
        baseline,
    );
    if let Some(rect) = selected_rect {
        ui.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            egui::Stroke::new(
                DETAIL_TAB_INDICATOR_WIDTH,
                ui.visuals().selection.stroke.color,
            ),
        );
    }
}

/// The General tab's model and RAM controls.
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

/// The rows sit bare — the tab's own label already says "Display".
fn draw_display_tab(ui: &mut egui::Ui, slug: &str, form: &mut new_vm::MachineForm) {
    form_grid(("detail_form_display", slug)).show(ui, |ui| {
        form.display_rows(ui);
    });
}

fn draw_devices_tab(ui: &mut egui::Ui, slug: &str, form: &mut new_vm::MachineForm) {
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
}

fn draw_input_tab(ui: &mut egui::Ui, form: &mut new_vm::MachineForm) {
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
            form.normalize();
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
                tab: DetailTab::default(),
            });
            self.save_error = None;
        }
        let mut edit = self.edit.take().expect("just ensured above");

        // The selected preview stays eligible even when its list row is offscreen.
        self.prepare_detail_thumbnail(&ui.ctx().clone(), index);
        edit.form.normalize();
        self.draw_detail_header(ui, index, &slug, &mut edit);
        ui.add_space(DETAIL_SECTION_GAP);

        draw_tab_bar(ui, &mut edit.tab);
        ui.add_space(DETAIL_SECTION_GAP);
        match edit.tab {
            DetailTab::General => self.draw_general_tab(ui, index, &slug, &mut edit),
            DetailTab::Display => draw_display_tab(ui, &slug, &mut edit.form),
            DetailTab::Devices => draw_devices_tab(ui, &slug, &mut edit.form),
            DetailTab::Input => draw_input_tab(ui, &mut edit.form),
            DetailTab::DriveWire => {
                let entry = &self.entries[index];
                live_drivewire::draw_drivewire_tab(ui, &mut edit.form, &slug, entry);
            }
        }
        // Constraints cross tab boundaries: a General-tab model change, for
        // example, must normalize Display, Input, Devices, and DriveWire.
        edit.form.normalize();
        // The DriveWire tab carries its own hint: its edits apply live.
        if self.entries[index].is_alive() && edit.tab != DetailTab::DriveWire {
            ui.add_space(DETAIL_SECTION_GAP);
            // Resume restores the frozen snapshot's hardware wholesale, so
            // for a suspended machine even Resume won't pick edits up —
            // only a cold start from power off does.
            ui.small("Changes apply the next time this machine starts from power off.");
        }

        self.autosave(&slug, index, &mut edit);
        if let Some(err) = &self.save_error {
            ui.add_space(DETAIL_SECTION_GAP);
            ui.colored_label(ui.visuals().error_fg_color, err);
        }

        self.edit = Some(edit);
    }

    /// Identity, current state, and launch failure remain visible above every
    /// tab. The preview belongs only to General, so it does not consume space
    /// in the settings tabs.
    fn draw_detail_header(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        slug: &str,
        edit: &mut EditState,
    ) {
        /// Indent that lines the slug up with the name field's text.
        const SLUG_LEFT_MARGIN: f32 = 4.0;
        /// Explains the unlabelled slug.
        const SLUG_HOVER_TEXT: &str =
            "Slug ID: the stable identifier control tools use for this machine";

        ui.horizontal_wrapped(|ui| {
            self.draw_name_field(ui, index, edit);
            ui.separator();
            ui.label(egui::RichText::new(vm_status_label(&self.entries[index])).strong());
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(SLUG_LEFT_MARGIN);
            ui.label(egui::RichText::new(slug).weak())
                .on_hover_text(SLUG_HOVER_TEXT);
        });
        if let Some(err) = &self.entries[index].launch_error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }
    }

    fn draw_general_tab(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        slug: &str,
        edit: &mut EditState,
    ) {
        ui.horizontal_top(|ui| {
            let details_width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
            let details = ui.vertical(|ui| {
                ui.set_max_width(details_width);
                draw_machine_ram_sections(ui, slug, &mut edit.form);
            });
            draw_screen_preview(ui, details.response.rect.height(), &self.entries[index]);
        });
        ui.add_space(DETAIL_SECTION_GAP);
        roms::draw_roms(ui, slug, &edit.roms);
        ui.add_space(DETAIL_SECTION_GAP);
        draw_statistics(ui, slug, &self.entries[index]);
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
                            let drivewire_changed = new_def.drivewire != edit.packed.drivewire;
                            self.entries[index].def = new_def.clone();
                            if drivewire_changed {
                                self.apply_live_drivewire(index);
                            }
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
