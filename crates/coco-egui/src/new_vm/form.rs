//! [`MachineForm`]'s impl: the row-drawing logic, split into the sections
//! the detail pane lays out ([`MachineForm::machine_rows`],
//! [`MachineForm::display_rows`], [`MachineForm::media_rows`],
//! [`MachineForm::ports_rows`]), plus each combo box's own picker logic. See
//! the parent module doc.

use coco_core::joystick::{LEFT, RIGHT};
use eframe::egui;

use crate::display::Display;
use crate::joy::JoySource;

use super::config_form;
use super::{
    CartridgeChoice, FORM_GRID_SPACING, MachineForm, MediaChoice, SerialChoice, SlotChoice,
    cartridge_label, disk_file_dialog, media_choice_text, rom_pak_file_dialog, serial_label,
    slot_label, sub_form_row,
};

impl MachineForm {
    /// An all-defaults form. `salt` is a per-host constant — see the
    /// field doc.
    pub(crate) fn new(salt: &'static str) -> Self {
        let config = coco_core::MachineConfig::default();
        Self {
            salt,
            // The default config's implied display (RGB monitor, since MachineConfig::default()
            // is a CoCo 3).
            display: Display::from_config(&config),
            tv: crate::display::TVSettings::default(),
            config,
            cartridge: CartridgeChoice::None,
            mpi_slots: std::array::from_fn(|_| SlotChoice::Empty),
            disks: std::array::from_fn(|_| MediaChoice::None),
            tape: MediaChoice::None,
            vhds: std::array::from_fn(|_| MediaChoice::None),
            // Matches `CocoApp::new`'s boot defaults.
            aspect_correct: true,
            serial: SerialChoice::None,
            // Indexed by `coco_core::joystick::{RIGHT, LEFT}`; both ports off until opted in.
            joy_sources: [JoySource::None, JoySource::None],
            kb_mode: crate::KbMode::Positional,
        }
    }

    /// The Model row — the detail pane hosts it inside its "Machine" titled group. Must be
    /// called inside an already-open two-column [`egui::Grid`] with [`FORM_GRID_SPACING`].
    pub(crate) fn machine_rows(&mut self, ui: &mut egui::Ui) {
        config_form::machine_rows(ui, self.salt, &mut self.config);
    }

    /// The VDG/Video/Display rows and the 4:3 aspect checkbox, hosted in the detail pane's
    /// "Display" titled group. Re-constrains the Display pick first, since it owns
    /// `config.monitor` and must stay valid for the current variant.
    pub(crate) fn display_rows(&mut self, ui: &mut egui::Ui) {
        self.constrain_display();
        let variant = self.config.variant;

        config_form::display_rows(ui, self.salt, &mut self.config);

        let font = ui.style().text_styles[&egui::TextStyle::Button].size;
        ui.label(egui::RichText::new("Display").size(font));
        ui.horizontal(|ui| {
            for &display in Display::choices(variant) {
                if ui.radio(self.display == display, display.label()).clicked() {
                    self.display = display;
                    self.config.monitor = display.to_monitor(variant);
                }
            }
        });
        ui.end_row();

        // The TV chain's knobs stay enabled only for a TV pick; the value is kept either way,
        // so switching back restores the tuned strength.
        let is_tv = matches!(self.display, Display::TV(_));
        ui.label(egui::RichText::new("Scanlines").size(font));
        ui.add_enabled(
            is_tv,
            egui::Slider::new(&mut self.tv.scanline_pct, 0..=crate::display::MAX_PCT).suffix("%"),
        );
        ui.end_row();

        ui.label(egui::RichText::new("RF noise").size(font));
        ui.add_enabled(
            is_tv,
            egui::Slider::new(&mut self.tv.noise_pct, 0..=crate::display::MAX_PCT).suffix("%"),
        );
        ui.end_row();

        ui.label(egui::RichText::new("Overscan").size(font));
        ui.add_enabled(
            is_tv,
            egui::Slider::new(
                &mut self.tv.overscan_pct,
                0..=crate::display::MAX_OVERSCAN_PCT,
            )
            .suffix("%"),
        );
        ui.end_row();

        ui.label("");
        ui.checkbox(&mut self.aspect_correct, "4:3 aspect correction");
        ui.end_row();
    }

    /// `constrain`'s display-shaped sibling: snaps a monitor pick to the default TV where the
    /// model has no monitor port, then re-derives `config.monitor` from the pick.
    pub(crate) fn constrain_display(&mut self) {
        let variant = self.config.variant;
        if !Display::choices(variant).contains(&self.display) {
            self.display = Display::default_for(variant);
        }
        self.config.monitor = self.display.to_monitor(variant);
    }

    /// The media rows: Cassette, Cartridge (with its nested MPI-slot/Disk sub-rows), and the
    /// VHD rows, hosted in the detail pane's "Peripherals" titled group.
    pub(crate) fn media_rows(&mut self, ui: &mut egui::Ui) {
        let font = ui.style().text_styles[&egui::TextStyle::Button].size;

        // Form-only rows: the cartridge and media aren't part of `MachineConfig` — see
        // [`CartridgeChoice`].
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

        // The cartridge's own rows nest below as an indented sub-form: FD-502's Disk rows
        // directly, or the MPI's four Slot rows.
        match self.cartridge {
            CartridgeChoice::FD502 => {
                sub_form_row(ui, |ui| self.disk_rows(ui, font));
            }
            CartridgeChoice::MPI => {
                sub_form_row(ui, |ui| self.slot_rows(ui, font));
            }
            CartridgeChoice::None
            | CartridgeChoice::ROMPak(_)
            | CartridgeChoice::RTC
            | CartridgeChoice::RS232 => {}
        }

        // The VHD hard disks, below removable media. Always shown, no cartridge required.
        for drive in 0..crate::UI_DRIVES {
            ui.label(egui::RichText::new(format!("VHD {drive}")).size(font));
            self.vhd_combo(ui, drive);
            ui.end_row();
        }
    }

    /// Whether a disk controller is reachable from the form's own picks:
    /// the bare FD-502, or one in an MPI slot.
    pub(crate) fn drives_available(&self) -> bool {
        match self.cartridge {
            CartridgeChoice::FD502 => true,
            CartridgeChoice::MPI => self.mpi_slots.contains(&SlotChoice::FD502),
            CartridgeChoice::None
            | CartridgeChoice::ROMPak(_)
            | CartridgeChoice::RTC
            | CartridgeChoice::RS232 => false,
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

    /// The Cartridge-row combo. "ROM Pak…" opens a file dialog on the spot; a cancelled dialog
    /// keeps the previous choice.
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
                        matches!(self.cartridge, CartridgeChoice::ROMPak(_)),
                        "ROM Pak…",
                    )
                    .clicked()
                    && let Some(path) = rom_pak_file_dialog().pick_file()
                {
                    self.cartridge = CartridgeChoice::ROMPak(path);
                }
                if ui
                    .selectable_label(self.cartridge == CartridgeChoice::RTC, "Disto RTC")
                    .clicked()
                {
                    self.cartridge = CartridgeChoice::RTC;
                }
                if ui
                    .selectable_label(self.cartridge == CartridgeChoice::RS232, "RS-232 Pak")
                    .clicked()
                {
                    self.cartridge = CartridgeChoice::RS232;
                }
                if ui
                    .selectable_label(self.cartridge == CartridgeChoice::MPI, "MultiPak Interface")
                    .clicked()
                {
                    self.cartridge = CartridgeChoice::MPI;
                }
            });
    }

    /// One "Slot N:" label + combo (Empty / FD-502 / ROM Pak… / Disto RTC), drawn while the
    /// MPI is selected. Claiming the FD-502 or RTC releases it from any other slot; ROM Paks
    /// may fill any number of slots.
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
                        matches!(self.mpi_slots[slot], SlotChoice::ROMPak(_)),
                        "ROM Pak…",
                    )
                    .clicked()
                    && let Some(path) = rom_pak_file_dialog().pick_file()
                {
                    self.mpi_slots[slot] = SlotChoice::ROMPak(path);
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

    /// The Cassette-row combo: the same None / Blank / Select… protocol as the disks' with tape
    /// semantics — Select… accepts `.cas` and WAV; Blank is a fresh empty `.cas`.
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
                    .selectable_label(self.tape == MediaChoice::Blank, "Blank")
                    .clicked()
                {
                    self.tape = MediaChoice::Blank;
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

    /// One "VHD N" combo — the VHD image for `drive`, with the disks' None / Blank / Select…
    /// protocol. A blank is a 0-byte file: `VHDImage::File` extends on write.
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
                    .selectable_label(self.vhds[drive] == MediaChoice::Blank, "Blank")
                    .clicked()
                {
                    self.vhds[drive] = MediaChoice::Blank;
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

    /// One "Disk N:" label + combo, drawn while the FD-502 is selected. "Select…" opens a file
    /// dialog on the spot; "Blank" is auto-placed in the machine's artifact directory at save time.
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
                    .selectable_label(self.disks[drive] == MediaChoice::Blank, "Blank")
                    .clicked()
                {
                    self.disks[drive] = MediaChoice::Blank;
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

    /// The Ports row: the built-in Serial port's host sink, hosted in the detail pane's own
    /// "Ports" titled group, below Peripherals.
    pub(crate) fn ports_rows(&mut self, ui: &mut egui::Ui) {
        ui.label("Serial");
        egui::ComboBox::from_id_salt((self.salt, "serial"))
            .selected_text(serial_label(self.serial))
            .show_ui(ui, |ui| {
                for source in SerialChoice::ALL {
                    ui.selectable_value(&mut self.serial, source, serial_label(source));
                }
            });
        ui.end_row();
    }

    /// The Joysticks fieldset's one row, between Ports and Keyboard. Left is shown before Right
    /// even though the right port is [`RIGHT`] (the CoCo's primary stick) — Left/Right reads
    /// naturally in that order to a user.
    pub(crate) fn joystick_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Left:");
            self.joy_combo(ui, LEFT);
            ui.add_space(FORM_GRID_SPACING[0]);
            ui.label("Right:");
            self.joy_combo(ui, RIGHT);
        });
    }

    /// One port's source combo, over every [`JoySource::ALL`] choice.
    fn joy_combo(&mut self, ui: &mut egui::Ui, port: usize) {
        egui::ComboBox::from_id_salt((self.salt, "joy", port))
            .selected_text(self.joy_sources[port].label())
            .show_ui(ui, |ui| {
                for source in JoySource::ALL {
                    ui.selectable_value(&mut self.joy_sources[port], source, source.label());
                }
            });
    }

    /// The Keyboard fieldset's one row, hosted last in the detail pane's own "Keyboard" titled
    /// group. A `[ui]` preference: the launched window's starting state; F12 keeps working as a
    /// live toggle afterwards.
    pub(crate) fn keyboard_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for mode in [crate::KbMode::Positional, crate::KbMode::Symbolic] {
                ui.radio_value(&mut self.kb_mode, mode, mode.label());
            }
        });
    }
}
