//! The detail/edit pane for the selected machine: drawing the shared
//! [`crate::new_vm::MachineForm`] over a definition and auto-saving every
//! change, plus the form↔definition mapping ([`ManagerApp::pack_def`],
//! [`seed_form`]) that makes that round trip possible.

use std::fs;
use std::path::PathBuf;

use eframe::egui;

use crate::{machine_def, new_vm};

use super::{
    vm_status_label, EditState, ManagerApp, DETAIL_SECTION_GAP, NO_CONFIG_DIR, PLAY_GLYPH,
    RESET_GLYPH, STOP_GLYPH, SUSPEND_GLYPH, SUSPEND_HOVER,
};

/// [`ManagerApp::record_media_choice`]'s auto-placed cassette file name, for
/// `[media].tape`.
const BLANK_TAPE_FILE: &str = "tape.cas";

/// File name of the auto-placed blank image the detail pane's
/// Disk N = Blank pick creates in the machine's artifact directory,
/// recorded in `[media].diskN` as a relative path.
fn blank_disk_file(drive: usize) -> String {
    format!("disk{drive}.dsk")
}

/// [`blank_disk_file`]'s VHD sibling, for `[media].vhdN`.
fn blank_vhd_file(drive: usize) -> String {
    format!("hd{drive}.vhd")
}

/// Fat transport-button geometry: minimum button size and glyph point size.
const TRANSPORT_BUTTON_SIZE: egui::Vec2 = egui::vec2(56.0, 40.0);
const TRANSPORT_GLYPH_SIZE: f32 = 24.0;
/// Gap separating the transport row from the status label.
const TRANSPORT_GROUP_GAP: f32 = 12.0;

/// One fat transport button ([`TRANSPORT_BUTTON_SIZE`]). `label` is the
/// accessible name (what a screen reader announces and what `ui_tests`
/// address nodes by) — without it the name would be the raw glyph, and
/// "clockwise open circle arrow" is nobody's idea of a Reset button.
fn transport_button(
    ui: &mut egui::Ui,
    glyph: &str,
    label: &str,
    enabled: bool,
) -> egui::Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(glyph).size(TRANSPORT_GLYPH_SIZE))
            .min_size(TRANSPORT_BUTTON_SIZE),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label)
    });
    response
}

/// Seed the detail pane's [`new_vm::MachineForm`] from a saved definition —
/// the inverse of [`ManagerApp::pack_def`], reconstructing the cartridge
/// picture the same way `crate::launch_machine` mounts it: with an MPI,
/// `[media].cart` re-seats in slot 0, the FD-502 in the last slot, the RTC
/// in its default slot; without one, the single port shows whichever of
/// pak/RTC/FD-502 the definition claims, in that priority (launch rejects a
/// conflicting combination outright — seeding at least shows one of them).
/// Disk media implies the FD-502 even when the flag is off (older files —
/// `launch_machine`'s rule).
fn seed_form(def: &machine_def::MachineDef) -> new_vm::MachineForm {
    let mut form = new_vm::MachineForm::new("detail", true);
    form.config = def.to_machine_config().expect("list entries are validated on load/save");
    let media = &def.media;
    let fd502 = def.peripherals.fd502 || media.disk0.is_some() || media.disk1.is_some();
    let cart = media.cart.as_deref().map(PathBuf::from);
    if def.peripherals.mpi {
        form.cartridge = new_vm::CartridgeChoice::MPI;
        if let Some(path) = cart {
            form.mpi_slots[0] = new_vm::SlotChoice::RomPak(path);
        }
        if fd502 {
            form.mpi_slots[crate::MPI_SLOT_COUNT - 1] = new_vm::SlotChoice::FD502;
        }
        if def.peripherals.rtc {
            form.mpi_slots[crate::DEFAULT_RTC_SLOT] = new_vm::SlotChoice::RTC;
        }
    } else if let Some(path) = cart {
        form.cartridge = new_vm::CartridgeChoice::RomPak(path);
    } else if def.peripherals.rtc {
        form.cartridge = new_vm::CartridgeChoice::RTC;
    } else if fd502 {
        form.cartridge = new_vm::CartridgeChoice::FD502;
    }
    let media_choice = |raw: &Option<String>| match raw {
        Some(s) => new_vm::MediaChoice::File(PathBuf::from(s)),
        None => new_vm::MediaChoice::None,
    };
    form.disks = [media_choice(&media.disk0), media_choice(&media.disk1)];
    form.tape = media_choice(&media.tape);
    form.vhds = [media_choice(&media.vhd0), media_choice(&media.vhd1)];
    form.aspect_correct = def.ui.aspect_correct;
    form.kb_mode = match def.ui.kb_mode {
        machine_def::KbModeDTO::Positional => crate::KbMode::Positional,
        machine_def::KbModeDTO::Symbolic => crate::KbMode::Symbolic,
    };
    form
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
            let mut form = seed_form(&def);
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
        ui.add_space(DETAIL_SECTION_GAP);
        self.draw_transport_row(ui, index);
        if let Some(err) = &self.entries[index].launch_error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }
        ui.add_space(DETAIL_SECTION_GAP);

        // The shared machine form (`new_vm::MachineForm`), hosted in the
        // pane's own grid.
        egui::Grid::new(("detail_form", slug.clone()))
            .num_columns(2)
            .spacing(new_vm::FORM_GRID_SPACING)
            .show(ui, |ui| {
                edit.form.rows(ui);
            });

        // THE RAM control (the form's grid deliberately has no RAM row —
        // `config_form_rows`'s doc): a `widgets::titled_group` fieldset
        // holding one radio button per size the selected model shipped
        // with, no field label (the group's title says it all). Edits the
        // same draft as every grid row, so the autosave below picks the
        // change up like any other form edit.
        ui.add_space(DETAIL_SECTION_GAP);
        crate::widgets::titled_group(ui, "RAM", |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                for &memory in new_vm::ram_choices(edit.form.config.variant) {
                    ui.radio_value(
                        &mut edit.form.config.memory,
                        memory,
                        new_vm::ram_label(memory),
                    );
                }
            });
        });
        if self.entries[index].vm.is_some() || self.entries[index].suspended {
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

    /// Run controls: the fat deck-style transport covers the three machine
    /// states — ▶ powers on
    /// (or resumes a suspended machine), ⏸ suspends, ⏹ powers off — with
    /// the console Reset (↻) as a fourth transport-style button after it.
    /// State is copied out before the buttons so the click handlers below
    /// can freely call `&mut self` methods (`start_vm`/`resume_vm`/
    /// `suspend_vm`/`stop_vm`) without fighting a borrow of
    /// `self.entries[index]`.
    fn draw_transport_row(&mut self, ui: &mut egui::Ui, index: usize) {
        let suspended = self.entries[index].suspended;
        let vm_alive = self.entries[index].vm.is_some();
        let running = vm_alive && !suspended;
        ui.horizontal(|ui| {
            // Each button also explains itself while disabled
            // (`on_disabled_hover_text` — a disabled `Response` never shows
            // the plain hover), so the transport teaches the state model
            // from any starting state.
            let play_hover =
                if suspended { "Resume the machine from its frozen state" } else { "Start the machine" };
            if transport_button(ui, PLAY_GLYPH, "Play", !running)
                .on_hover_text(play_hover)
                .on_disabled_hover_text("The machine is already running")
                .clicked()
            {
                if suspended {
                    self.resume_vm(index);
                } else {
                    self.start_vm(index);
                }
            }
            if transport_button(ui, SUSPEND_GLYPH, "Suspend", running)
                .on_hover_text(SUSPEND_HOVER)
                .on_disabled_hover_text(SUSPEND_HOVER)
                .clicked()
            {
                self.suspend_vm(index);
            }
            const STOP_HOVER: &str =
                "Shut down the machine — like flipping the power switch; \
                 unsaved work inside it (and any suspended state) is lost";
            if transport_button(ui, STOP_GLYPH, "Stop", vm_alive || suspended)
                .on_hover_text(STOP_HOVER)
                .on_disabled_hover_text(STOP_HOVER)
                .clicked()
            {
                self.stop_vm(index);
            }
            const RESET_HOVER: &str = "Press the machine's reset button";
            if transport_button(ui, RESET_GLYPH, "Reset", running)
                .on_hover_text(RESET_HOVER)
                .on_disabled_hover_text(RESET_HOVER)
                .clicked()
                && let Some(vm) = self.entries[index].vm.as_mut()
            {
                vm.machine.reset();
            }

            ui.add_space(TRANSPORT_GROUP_GAP);
            ui.label(egui::RichText::new(vm_status_label(&self.entries[index])).strong());
        });
    }

    /// Auto-save: every change writes straight back to the definition file
    /// (no Save/Revert — user decision 2026-07-24; the write is atomic,
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
    /// new name via `migrate_slug` — deferred to
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

    /// Resolve one of the edit form's media picks to the string recorded in
    /// the definition's `[media]` section, creating the backing file for a
    /// Blank pick: auto-placed in `slug`'s artifact dir as `auto_file`
    /// (recorded relative — `machine_def::resolve_media_path`). Blank media
    /// is a 0-byte file — a blank 0-track JVC disk, an empty `.cas` tape or
    /// `.vhd`, the same starting point `CocoApp::{new_blank_disk, new_tape}`
    /// use; a leftover file under the same slug is reused rather than
    /// clobbered. The pick is rewritten to `File(recorded)` afterwards so
    /// the combo shows the placed file, not a stale "Blank".
    fn record_media_choice(
        &self,
        slug: &str,
        choice: &mut new_vm::MediaChoice,
        auto_file: String,
    ) -> Result<Option<String>, String> {
        let (path, recorded) = match &*choice {
            new_vm::MediaChoice::None => return Ok(None),
            new_vm::MediaChoice::File(path) => return Ok(Some(path.display().to_string())),
            new_vm::MediaChoice::Blank(Some(path)) => {
                (path.clone(), path.display().to_string())
            }
            new_vm::MediaChoice::Blank(None) => {
                let Some(root) = self.artifacts_root.clone() else {
                    return Err(NO_CONFIG_DIR.to_string());
                };
                let artifact_dir = root.join(slug);
                fs::create_dir_all(&artifact_dir)
                    .map_err(|e| format!("{}: {e}", artifact_dir.display()))?;
                (artifact_dir.join(&auto_file), auto_file)
            }
        };
        match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
        *choice = new_vm::MediaChoice::File(PathBuf::from(&recorded));
        Ok(Some(recorded))
    }

    /// Pack the edit form back into a definition, starting from `base` (the
    /// entry's current definition) so everything the form doesn't edit —
    /// `name`, `created`, `[hardware].rom`, unknown keys — passes through
    /// untouched. Blank media picks create their backing files here (see
    /// [`Self::record_media_choice`]); this is the write moment, since with
    /// auto-save every change *is* a save. Errors (an unrepresentable form,
    /// a failed file creation) leave the definition unwritten and land in
    /// the pane's error label.
    fn pack_def(
        &self,
        base: &machine_def::MachineDef,
        slug: &str,
        form: &mut new_vm::MachineForm,
    ) -> Result<machine_def::MachineDef, String> {
        let mut def = base.clone();
        def.hardware =
            machine_def::HardwareDTO::from_config(&form.config, base.hardware.rom.clone());
        // The definition schema has no slot layout (yet): an FD-502 in an
        // MPI slot is recorded as fd502 = true, a slotted RTC as rtc = true,
        // and launch_machine re-seats them in their default slots.
        def.peripherals.fd502 = form.drives_available();
        def.peripherals.mpi = form.cartridge == new_vm::CartridgeChoice::MPI;
        def.peripherals.rtc = form.cartridge == new_vm::CartridgeChoice::RTC
            || form.mpi_slots.contains(&new_vm::SlotChoice::RTC);
        // A ROM Pak — in the port or slotted in the MPI — is recorded as
        // [media].cart. The schema holds a single pak and no slot layout
        // (launch_machine re-seats a slotted one in slot 0), so more than
        // one slotted pak cannot be represented.
        let mut slotted_paks = form.mpi_slots.iter().filter_map(|slot| match slot {
            new_vm::SlotChoice::RomPak(path) => Some(path),
            _ => None,
        });
        def.media.cart = match &form.cartridge {
            new_vm::CartridgeChoice::RomPak(path) => Some(path.display().to_string()),
            new_vm::CartridgeChoice::MPI => slotted_paks.next().map(|p| p.display().to_string()),
            _ => None,
        };
        if slotted_paks.next().is_some() {
            return Err(
                "a machine definition records a single ROM Pak — leave at most one slot \
                 with a pak"
                    .to_string(),
            );
        }
        for drive in 0..crate::UI_DRIVES {
            let recorded =
                self.record_media_choice(slug, &mut form.disks[drive], blank_disk_file(drive))?;
            match drive {
                0 => def.media.disk0 = recorded,
                _ => def.media.disk1 = recorded,
            }
        }
        def.media.tape =
            self.record_media_choice(slug, &mut form.tape, BLANK_TAPE_FILE.to_string())?;
        for drive in 0..crate::UI_DRIVES {
            let recorded =
                self.record_media_choice(slug, &mut form.vhds[drive], blank_vhd_file(drive))?;
            match drive {
                0 => def.media.vhd0 = recorded,
                _ => def.media.vhd1 = recorded,
            }
        }
        def.ui.aspect_correct = form.aspect_correct;
        def.ui.kb_mode = match form.kb_mode {
            crate::KbMode::Positional => machine_def::KbModeDTO::Positional,
            crate::KbMode::Symbolic => machine_def::KbModeDTO::Symbolic,
        };
        Ok(def)
    }
}
