//! [`MachineForm`]'s impl: the full row-drawing logic (hardware rows via
//! [`super::config_form::config_form_rows`], then Cassette, Cartridge with
//! its nested MPI-slot/Disk sub-rows, the HD rows, and the UI rows) plus
//! each combo box's own picker logic. See the parent module doc for how the
//! two hosts share this.

use eframe::egui;

use super::config_form::config_form_rows;
use super::{
    cartridge_label, disk_file_dialog, media_choice_text, rom_pak_file_dialog, slot_label,
    sub_form_row, CartridgeChoice, MachineForm, MediaChoice, SlotChoice, FORM_GRID_SPACING,
};

impl MachineForm {
    /// An all-defaults form. `salt` and `auto_place_blanks` are per-host
    /// constants — see the field docs.
    pub fn new(salt: &'static str, auto_place_blanks: bool) -> Self {
        Self {
            salt,
            auto_place_blanks,
            config: coco_core::MachineConfig::default(),
            cartridge: CartridgeChoice::None,
            mpi_slots: std::array::from_fn(|_| SlotChoice::Empty),
            disks: std::array::from_fn(|_| MediaChoice::None),
            tape: MediaChoice::None,
            vhds: std::array::from_fn(|_| MediaChoice::None),
            // The same starting values `CocoApp::new` boots with and
            // `machine_def::UIDTO::default()` records.
            aspect_correct: true,
            kb_mode: crate::KbMode::Positional,
        }
    }

    /// All form rows. Must be called inside an already-open two-column
    /// [`egui::Grid`] with [`FORM_GRID_SPACING`], like [`config_form_rows`].
    pub fn rows(&mut self, ui: &mut egui::Ui) {
        let font = ui.style().text_styles[&egui::TextStyle::Button].size;
        config_form_rows(ui, self.salt, &mut self.config);

        // Form-only rows (not `config_form_rows`): the cartridge and media
        // aren't part of `MachineConfig` — see [`CartridgeChoice`].
        ui.label(egui::RichText::new("Cassette").size(font));
        self.tape_combo(ui);
        ui.end_row();

        if self.cartridge != CartridgeChoice::MPI {
            self.mpi_slots = std::array::from_fn(|_| SlotChoice::Empty);
        }
        if !self.drives_available() {
            self.disks = std::array::from_fn(|_| MediaChoice::None);
        }
        ui.label(egui::RichText::new("Cartridge").size(font));
        self.cartridge_combo(ui);
        ui.end_row();

        // The cartridge's own rows nest below it as an indented label+combo
        // sub-form: the FD-502's Disk rows directly, the MPI's four Slot
        // rows (with the Disk rows one level deeper, under whichever slot
        // holds the FD-502).
        match self.cartridge {
            CartridgeChoice::FD502 => {
                sub_form_row(ui, |ui| self.disk_rows(ui, font));
            }
            CartridgeChoice::MPI => {
                sub_form_row(ui, |ui| self.slot_rows(ui, font));
            }
            CartridgeChoice::None | CartridgeChoice::RomPak(_) | CartridgeChoice::RTC => {}
        }

        // The VHD hard disks, below the removable media. Always shown, no
        // cartridge required — see [`super::NewMachineSpec::vhds`].
        for drive in 0..crate::UI_DRIVES {
            ui.label(egui::RichText::new(format!("HD {drive}")).size(font));
            self.vhd_combo(ui, drive);
            ui.end_row();
        }

        // UI preferences, the `[ui]` section's fields: the launched
        // window's *starting* state; F9/F12 keep working as live toggles.
        ui.label(egui::RichText::new("Display").size(font));
        ui.checkbox(&mut self.aspect_correct, "4:3 aspect correction");
        ui.end_row();

        ui.label(egui::RichText::new("Keyboard").size(font));
        ui.horizontal(|ui| {
            for mode in [crate::KbMode::Positional, crate::KbMode::Symbolic] {
                ui.radio_value(&mut self.kb_mode, mode, mode.label());
            }
        });
        ui.end_row();
    }

    /// [`super::NewMachineSpec::has_drives`] over the form's own picks.
    pub fn drives_available(&self) -> bool {
        match self.cartridge {
            CartridgeChoice::FD502 => true,
            CartridgeChoice::MPI => self.mpi_slots.contains(&SlotChoice::FD502),
            CartridgeChoice::None | CartridgeChoice::RomPak(_) | CartridgeChoice::RTC => false,
        }
    }

    /// The MPI's four Slot rows as their own label+combo grid; the Disk
    /// rows nest one level deeper under whichever slot holds the FD-502.
    fn slot_rows(&mut self, ui: &mut egui::Ui, font: f32) {
        egui::Grid::new((self.salt, "slots"))
            .num_columns(2)
            .spacing(FORM_GRID_SPACING)
            .show(ui, |ui| {
                for slot in 0..crate::MPI_SLOT_COUNT {
                    self.slot_combo(ui, font, slot);
                    ui.end_row();
                    if self.mpi_slots[slot] == SlotChoice::FD502 {
                        sub_form_row(ui, |ui| self.disk_rows(ui, font));
                    }
                }
            });
    }

    /// The Disk rows as their own label+combo grid, one row per drive.
    fn disk_rows(&mut self, ui: &mut egui::Ui, font: f32) {
        egui::Grid::new((self.salt, "disks"))
            .num_columns(2)
            .spacing(FORM_GRID_SPACING)
            .show(ui, |ui| {
                for drive in 0..crate::UI_DRIVES {
                    self.disk_combo(ui, font, drive);
                    ui.end_row();
                }
            });
    }

    /// The Cartridge-row combo. "ROM Pak…" opens a file dialog on the spot
    /// (like the media combos' Select…); a cancelled dialog keeps the
    /// previous choice.
    fn cartridge_combo(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::from_id_salt((self.salt, "cartridge"))
            .selected_text(cartridge_label(&self.cartridge))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(self.cartridge == CartridgeChoice::None, "None")
                    .clicked()
                {
                    self.cartridge = CartridgeChoice::None;
                }
                if ui
                    .selectable_label(self.cartridge == CartridgeChoice::FD502, "FD-502")
                    .clicked()
                {
                    self.cartridge = CartridgeChoice::FD502;
                }
                if ui
                    .selectable_label(
                        matches!(self.cartridge, CartridgeChoice::RomPak(_)),
                        "ROM Pak…",
                    )
                    .clicked()
                    && let Some(path) = rom_pak_file_dialog().pick_file()
                {
                    self.cartridge = CartridgeChoice::RomPak(path);
                }
                if ui
                    .selectable_label(self.cartridge == CartridgeChoice::RTC, "Disto RTC")
                    .clicked()
                {
                    self.cartridge = CartridgeChoice::RTC;
                }
                if ui
                    .selectable_label(self.cartridge == CartridgeChoice::MPI, "MultiPak Interface")
                    .clicked()
                {
                    self.cartridge = CartridgeChoice::MPI;
                }
            });
    }

    /// One "Slot N:" label + combo (Empty / FD-502 / ROM Pak… / Disto RTC),
    /// drawn while the MPI is selected. Claiming the FD-502 or the RTC
    /// releases it from any other slot — one of each max (see
    /// [`SlotChoice`]); ROM Paks may fill any number of slots.
    fn slot_combo(&mut self, ui: &mut egui::Ui, font: f32, slot: usize) {
        ui.label(egui::RichText::new(format!("Slot {}:", slot + 1)).size(font));
        egui::ComboBox::from_id_salt((self.salt, "mpi_slot", slot))
            .selected_text(slot_label(&self.mpi_slots[slot]))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(self.mpi_slots[slot] == SlotChoice::Empty, "Empty")
                    .clicked()
                {
                    self.mpi_slots[slot] = SlotChoice::Empty;
                }
                if ui
                    .selectable_label(self.mpi_slots[slot] == SlotChoice::FD502, "FD-502")
                    .clicked()
                {
                    for other in &mut self.mpi_slots {
                        if *other == SlotChoice::FD502 {
                            *other = SlotChoice::Empty;
                        }
                    }
                    self.mpi_slots[slot] = SlotChoice::FD502;
                }
                if ui
                    .selectable_label(
                        matches!(self.mpi_slots[slot], SlotChoice::RomPak(_)),
                        "ROM Pak…",
                    )
                    .clicked()
                    && let Some(path) = rom_pak_file_dialog().pick_file()
                {
                    self.mpi_slots[slot] = SlotChoice::RomPak(path);
                }
                if ui
                    .selectable_label(self.mpi_slots[slot] == SlotChoice::RTC, "Disto RTC")
                    .clicked()
                {
                    // One clock max: two would shadow each other at $FF50.
                    for other in &mut self.mpi_slots {
                        if *other == SlotChoice::RTC {
                            *other = SlotChoice::Empty;
                        }
                    }
                    self.mpi_slots[slot] = SlotChoice::RTC;
                }
            });
    }

    /// The Cassette-row combo: the same None / Blank / Select… protocol as
    /// the disks' ([`Self::disk_combo`]) with tape semantics — Select…
    /// accepts `.cas` and WAV, Blank is a fresh `.cas` (empty file), and
    /// the manager flow auto-places `tape.cas` in the artifact directory.
    fn tape_combo(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::from_id_salt((self.salt, "tape"))
            .selected_text(media_choice_text(&self.tape))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(self.tape == MediaChoice::None, "None")
                    .clicked()
                {
                    self.tape = MediaChoice::None;
                }
                if ui
                    .selectable_label(matches!(self.tape, MediaChoice::Blank(_)), "Blank")
                    .clicked()
                {
                    self.tape = if self.auto_place_blanks {
                        MediaChoice::Blank(None)
                    } else {
                        match rfd::FileDialog::new()
                            .add_filter("Cassette image", &["cas"])
                            .set_file_name("blank.cas")
                            .save_file()
                        {
                            Some(path) => MediaChoice::Blank(Some(path)),
                            None => MediaChoice::None,
                        }
                    };
                }
                if ui
                    .selectable_label(matches!(self.tape, MediaChoice::File(_)), "Select…")
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Cassette image", &["cas", "wav"])
                        .pick_file()
                {
                    self.tape = MediaChoice::File(path);
                }
            });
    }

    /// One "HD N"-row combo — the VHD hard-disk image for `drive`, with
    /// the disks' None / Blank / Select… protocol ([`Self::disk_combo`]).
    /// A blank is a 0-byte file: `VhdImage::File` extends on write, so no
    /// preallocation is needed.
    fn vhd_combo(&mut self, ui: &mut egui::Ui, drive: usize) {
        egui::ComboBox::from_id_salt((self.salt, "vhd", drive))
            .selected_text(media_choice_text(&self.vhds[drive]))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(self.vhds[drive] == MediaChoice::None, "None")
                    .clicked()
                {
                    self.vhds[drive] = MediaChoice::None;
                }
                if ui
                    .selectable_label(matches!(self.vhds[drive], MediaChoice::Blank(_)), "Blank")
                    .clicked()
                {
                    self.vhds[drive] = if self.auto_place_blanks {
                        MediaChoice::Blank(None)
                    } else {
                        match rfd::FileDialog::new()
                            .add_filter("VHD image", &["vhd"])
                            .set_file_name(format!("blank{drive}.vhd"))
                            .save_file()
                        {
                            Some(path) => MediaChoice::Blank(Some(path)),
                            None => MediaChoice::None,
                        }
                    };
                }
                if ui
                    .selectable_label(matches!(self.vhds[drive], MediaChoice::File(_)), "Select…")
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("VHD image", &["vhd"])
                        .pick_file()
                {
                    self.vhds[drive] = MediaChoice::File(path);
                }
            });
    }

    /// One "Disk N:" label + combo, drawn while the FD-502 is selected
    /// (indented rows under the Cartridge combo — one level deeper when
    /// nested under an MPI slot). "Blank" and "Select…" open native file
    /// dialogs on the spot
    /// (save-file and open-file respectively) — except the manager flow's
    /// "Blank" (`show_name_field`), which is auto-placed in the machine's
    /// artifact directory at create time and needs no path here. A
    /// cancelled dialog falls back to None rather than keeping a pathless
    /// choice.
    fn disk_combo(&mut self, ui: &mut egui::Ui, font: f32, drive: usize) {
        ui.label(egui::RichText::new(format!("Disk {drive}:")).size(font));
        egui::ComboBox::from_id_salt((self.salt, "disk", drive))
            .selected_text(media_choice_text(&self.disks[drive]))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(self.disks[drive] == MediaChoice::None, "None")
                    .clicked()
                {
                    self.disks[drive] = MediaChoice::None;
                }
                if ui
                    .selectable_label(matches!(self.disks[drive], MediaChoice::Blank(_)), "Blank")
                    .clicked()
                {
                    self.disks[drive] = if self.auto_place_blanks {
                        MediaChoice::Blank(None)
                    } else {
                        match disk_file_dialog()
                            .set_file_name(format!("blank{drive}.dsk"))
                            .save_file()
                        {
                            Some(path) => MediaChoice::Blank(Some(path)),
                            None => MediaChoice::None,
                        }
                    };
                }
                if ui
                    .selectable_label(matches!(self.disks[drive], MediaChoice::File(_)), "Select…")
                    .clicked()
                    && let Some(path) = disk_file_dialog().pick_file()
                {
                    self.disks[drive] = MediaChoice::File(path);
                }
            });
    }
}
