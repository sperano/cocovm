//! [`MachineForm`]'s impl: the row-drawing logic, split into the sections
//! the detail pane lays out ([`MachineForm::machine_rows`],
//! [`MachineForm::display_rows`], [`MachineForm::media_rows`],
//! [`MachineForm::ports_rows`]), plus each combo box's own picker logic. See
//! the parent module doc.

use coco_core::MachineVariant;
use coco_core::joystick::{LEFT, RIGHT};
use eframe::egui;

use crate::display::Display;
use crate::joy::{HiResChoice, JoySource};

use super::cartridge::{CartridgeChoice, RS232EndpointChoice, SlotChoice};
use super::{
    FORM_GRID_SPACING, MachineForm, MediaChoice, SerialChoice, media_choice_text, serial_label,
};
use super::{cartridge_form, config_form};

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
            drivewire: crate::machine_def::DriveWireDTO::default(),
            cartridge: CartridgeChoice::None,
            mpi_slots: std::array::from_fn(|_| SlotChoice::Empty),
            mpi_switch: crate::DEFAULT_MPI_SWITCH_SLOT,
            rs232_endpoint: RS232EndpointChoice::default(),
            disks: std::array::from_fn(|_| MediaChoice::None),
            tape: MediaChoice::None,
            vhds: std::array::from_fn(|_| MediaChoice::None),
            serial: SerialChoice::None,
            // Indexed by `coco_core::joystick::{RIGHT, LEFT}`; both ports off until opted in.
            joy_sources: [JoySource::None, JoySource::None],
            hires: [HiResChoice::None, HiResChoice::None],
            kb_mode: crate::KbMode::Positional,
        }
    }

    /// The Model row — the detail pane hosts it inside its "Machine" titled group. Must be
    /// called inside an already-open two-column [`egui::Grid`] with [`FORM_GRID_SPACING`].
    pub(crate) fn machine_rows(&mut self, ui: &mut egui::Ui) {
        config_form::machine_rows(ui, self.salt, &mut self.config);
    }

    /// The VDG/Display rows and TV controls, hosted in the detail pane's
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
                let mut response = ui.radio(self.display == display, display.label());
                if let Some(note) = display.note(variant) {
                    response = response.on_hover_text(note);
                }
                if response.clicked() {
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
    }

    /// `constrain`'s display-shaped sibling: snaps a pick the model cannot drive (RGB on a
    /// CoCo 1/2) to the model's default, then re-derives `config.monitor` from the pick.
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
            self.mpi_switch = crate::DEFAULT_MPI_SWITCH_SLOT;
        }
        if self.cartridge != CartridgeChoice::RS232 {
            self.rs232_endpoint = RS232EndpointChoice::default();
        }
        if !self.drives_available() {
            self.disks = std::array::from_fn(|_| MediaChoice::None);
        }
        // The CoCo Max module is CoCo 1/2-only hardware (the CoCo 3's GIME
        // owns its $FF90-$FF97 window) — drop an already-picked one, bare
        // port or MPI slot, the moment the model moves to CoCo 3, the same
        // way the resets above drop fields their own picks made irrelevant.
        if self.config.variant == MachineVariant::Coco3 {
            if self.cartridge == CartridgeChoice::CoCoMax {
                self.cartridge = CartridgeChoice::None;
            }
            for slot in &mut self.mpi_slots {
                if *slot == SlotChoice::CoCoMax {
                    *slot = SlotChoice::Empty;
                }
            }
        }
        cartridge_form::cartridge_row(
            ui,
            self.salt,
            font,
            cartridge_form::CartridgeRowState {
                cartridge: &mut self.cartridge,
                mpi_slots: &mut self.mpi_slots,
                mpi_switch: &mut self.mpi_switch,
                rs232_endpoint: &mut self.rs232_endpoint,
                disks: &mut self.disks,
                variant: self.config.variant,
            },
        );

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
        cartridge_form::drives_available(&self.cartridge, &self.mpi_slots)
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

    /// The Joysticks fieldset's rows, between Ports and Keyboard. Left is shown before Right
    /// even though the right port is [`RIGHT`] (the CoCo's primary stick) — Left/Right reads
    /// naturally in that order to a user. The hi-res Interface row only appears on a CoCo 3
    /// ([`Self::constrain_hires`]).
    pub(crate) fn joystick_row(&mut self, ui: &mut egui::Ui) {
        self.constrain_hires();
        ui.horizontal(|ui| {
            ui.label("Left:");
            self.joy_combo(ui, LEFT);
            ui.add_space(FORM_GRID_SPACING[0]);
            ui.label("Right:");
            self.joy_combo(ui, RIGHT);
        });
        if self.config.variant == MachineVariant::Coco3 {
            ui.horizontal(|ui| {
                ui.label("Left interface:");
                self.hires_combo(ui, LEFT);
                ui.add_space(FORM_GRID_SPACING[0]);
                ui.label("Right interface:");
                self.hires_combo(ui, RIGHT);
            });
        }
    }

    /// `constrain`'s hi-res sibling: the Interface picks are offered CoCo 3 only (a form scope
    /// decision, not a hardware limit), so switching models resets both to `None`, like
    /// [`Self::constrain_display`] does for the Display pick.
    pub(crate) fn constrain_hires(&mut self) {
        if self.config.variant != MachineVariant::Coco3 {
            self.hires = [HiResChoice::None, HiResChoice::None];
        }
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

    /// One port's hi-res interface combo. There's only one physical DAC to time a one-shot
    /// off of, so picking [`HiResChoice::Tandy`] here clears a Tandy already on the other port
    /// (`coco_core::joystick::Joysticks::set_hires`'s doc explains why) — but only a Tandy; a
    /// CoCoMax3 on the other port is untouched.
    fn hires_combo(&mut self, ui: &mut egui::Ui, port: usize) {
        let mut choice = self.hires[port];
        egui::ComboBox::from_id_salt((self.salt, "hires", port))
            .selected_text(choice.label())
            .show_ui(ui, |ui| {
                for c in HiResChoice::ALL {
                    ui.selectable_value(&mut choice, c, c.label());
                }
            });
        if choice != self.hires[port] {
            self.hires[port] = choice;
            if choice == HiResChoice::Tandy && self.hires[port ^ 1] == HiResChoice::Tandy {
                self.hires[port ^ 1] = HiResChoice::None;
            }
        }
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
