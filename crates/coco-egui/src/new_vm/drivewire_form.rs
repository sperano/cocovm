//! Persistent DriveWire startup settings. Host shares and service options
//! belong in this section as their implementations become available.

use std::path::Path;

use eframe::egui;

use super::{CartridgeChoice, FORM_GRID_SPACING, MachineForm, SlotChoice, pack_peripherals};
use crate::machine_def::DosRom;

const DISK_IMAGE_EXTENSIONS: &[&str] = &["dsk", "os9", "img", "vhd"];
const GMC_CONFLICT_HINT: &str =
    "DriveWire and the Games Master Cartridge cannot be enabled together.";
const HDBDOS_REQUIRED_HINT: &str = "Required by the selected HDB-DOS DW3 ROM.";
const STARTUP_HINT: &str = "DriveWire changes apply at the next start from power off. \
    Resume keeps the saved session.";
const HDBDOS_MODE_HINT: &str = "Enable for HDB-DOS BASIC disk commands. Translates HDB-DOS \
    sector addresses into separate disk images in DW0–DW3. Leave disabled for NitrOS-9, \
    which uses its own DriveWire driver.";

impl MachineForm {
    /// Settings edit the startup definition only. They never inspect or
    /// overwrite guest-selected mounts or a running protocol transaction.
    pub(crate) fn drivewire_rows(&mut self, ui: &mut egui::Ui) {
        self.constrain_dos_rom();
        let cartridge = pack_peripherals(
            &self.cartridge,
            &self.mpi_slots,
            self.mpi_switch,
            &self.rs232_endpoint,
        )
        .cartridge;
        let conflict = cartridge.contains_games_master();
        let required = cartridge.uses_hdbdos();
        ui.add_enabled(
            !required && (self.drivewire.enabled || !conflict),
            egui::Checkbox::new(&mut self.drivewire.enabled, "Enable DriveWire"),
        )
        .on_disabled_hover_text(if required { HDBDOS_REQUIRED_HINT } else { GMC_CONFLICT_HINT });
        if conflict {
            ui.colored_label(ui.visuals().warn_fg_color, GMC_CONFLICT_HINT);
        }
        ui.add_enabled(
            self.drivewire.enabled && !required,
            egui::Checkbox::new(&mut self.drivewire.hdbdos_mode, "HDB-DOS mode"),
        )
        .on_hover_text(HDBDOS_MODE_HINT)
        .on_disabled_hover_text(if required { HDBDOS_REQUIRED_HINT } else { HDBDOS_MODE_HINT });
        self.drivewire_disks(ui);
        ui.small(STARTUP_HINT);
    }

    pub(crate) fn constrain_dos_rom(&mut self) {
        let supported = self.config.variant == coco_core::MachineVariant::Coco3;
        if let CartridgeChoice::FD502 { dos_rom } = &mut self.cartridge
            && !supported
        {
            *dos_rom = DosRom::DiskBasic;
        }
        for slot in &mut self.mpi_slots {
            if let SlotChoice::FD502 { dos_rom } = slot
                && !supported
            {
                *dos_rom = DosRom::DiskBasic;
            }
        }
        let cartridge = pack_peripherals(
            &self.cartridge,
            &self.mpi_slots,
            self.mpi_switch,
            &self.rs232_endpoint,
        )
        .cartridge;
        if cartridge.uses_hdbdos() {
            self.drivewire.enabled = true;
            self.drivewire.hdbdos_mode = true;
        }
    }

    fn drivewire_disks(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(self.drivewire.enabled, |ui| {
            egui::Grid::new((self.salt, "drivewire_disks"))
                .num_columns(2)
                .spacing(FORM_GRID_SPACING)
                .show(ui, |ui| {
                    for (drive, path) in [
                        &mut self.drivewire.disk0,
                        &mut self.drivewire.disk1,
                        &mut self.drivewire.disk2,
                        &mut self.drivewire.disk3,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        disk_row(ui, drive, path);
                    }
                });
        });
    }
}

fn disk_row(ui: &mut egui::Ui, drive: usize, path: &mut Option<String>) {
    ui.label(format!("DW{drive}"));
    ui.horizontal(|ui| {
        if ui.button(format!("Mount DW{drive}…")).clicked()
            && let Some(selected) = rfd::FileDialog::new()
                .add_filter("Disk image", DISK_IMAGE_EXTENSIONS)
                .pick_file()
        {
            *path = Some(selected.display().to_string());
        }
        if ui
            .add_enabled(
                path.is_some(),
                egui::Button::new(format!("Eject DW{drive}")),
            )
            .clicked()
        {
            *path = None;
        }
        if let Some(path) = path {
            let name = Path::new(path.as_str())
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path);
            ui.label(name).on_hover_text(path.as_str());
        } else {
            ui.label("Empty");
        }
    });
    ui.end_row();
}
