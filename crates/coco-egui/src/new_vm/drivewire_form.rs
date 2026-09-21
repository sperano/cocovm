//! Persistent DriveWire startup settings. Host shares and service options
//! belong in this section as their implementations become available.

use std::path::Path;

use eframe::egui;

use super::{CartridgeChoice, FORM_GRID_SPACING, MachineForm, SlotChoice, pack_peripherals};
use crate::machine_def::DosRom;

const DISK_IMAGE_EXTENSIONS: &[&str] = &["dsk", "os9", "img", "vhd"];
const DRIVE_LABEL_WIDTH: f32 = 32.0;
const BROWSE_BUTTON_WIDTH: f32 = 76.0;
const CLEAR_BUTTON_WIDTH: f32 = 22.0;
const PATH_MARGIN: egui::Margin = egui::Margin {
    left: 4,
    right: 26,
    top: 3,
    bottom: 3,
};
const CLEAR_BUTTON_INSET: f32 = 2.0;
const ROW_GAPS: f32 = 2.0;
const EMPTY_PATH_HINT: &str = "No disk image";
const CLEAR_PATH_HINT: &str = "Clear a path to leave the drive empty.";
const GMC_CONFLICT_HINT: &str =
    "DriveWire and the Games Master Cartridge cannot be enabled together.";
const HDBDOS_REQUIRED_HINT: &str = "Required by the selected HDB-DOS DW3 ROM.";
const STARTUP_HINT: &str = "DriveWire changes apply at the next start from power off. \
    Resume keeps the saved session.";
const HDBDOS_MODE_HINT: &str = "Enable for HDB-DOS BASIC disk commands. Translates HDB-DOS \
    sector addresses into separate disk images in DW0–DW3. Leave disabled for NitrOS-9, \
    which uses its own DriveWire driver.";

impl MachineForm {
    /// The enable and HDB-DOS mode switches, drawn bare at the top of the tab.
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
        ui.spacing_mut().item_spacing.y = FORM_GRID_SPACING[1];
        ui.add_enabled(
            !required && (self.drivewire.enabled || !conflict),
            egui::Checkbox::new(&mut self.drivewire.enabled, "Enable DriveWire"),
        )
        .on_disabled_hover_text(if required {
            HDBDOS_REQUIRED_HINT
        } else {
            GMC_CONFLICT_HINT
        });
        if conflict {
            ui.colored_label(ui.visuals().warn_fg_color, GMC_CONFLICT_HINT);
        }
        ui.add_enabled(
            self.drivewire.enabled && !required,
            egui::Checkbox::new(&mut self.drivewire.hdbdos_mode, "HDB-DOS mode"),
        )
        .on_hover_text(HDBDOS_MODE_HINT)
        .on_disabled_hover_text(if required {
            HDBDOS_REQUIRED_HINT
        } else {
            HDBDOS_MODE_HINT
        });
        ui.small(STARTUP_HINT);
    }

    /// The "Disk images" section: one editable path per startup drive.
    /// Drawn after [`Self::drivewire_rows`], which already constrained `enabled`.
    pub(crate) fn drivewire_disk_rows(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = FORM_GRID_SPACING[1];
        self.drivewire_disks(ui);
        ui.small(CLEAR_PATH_HINT);
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
            ui.push_id((self.salt, "drivewire_disks"), |ui| {
                for (drive, path) in [
                    &mut self.drivewire.disk0,
                    &mut self.drivewire.disk1,
                    &mut self.drivewire.disk2,
                    &mut self.drivewire.disk3,
                ]
                .into_iter()
                .enumerate()
                {
                    ui.push_id(drive, |ui| disk_row(ui, drive, path));
                }
            });
        });
    }
}

fn disk_row(ui: &mut egui::Ui, drive: usize, path: &mut Option<String>) {
    let field_width = (ui.available_width()
        - DRIVE_LABEL_WIDTH
        - BROWSE_BUTTON_WIDTH
        - ROW_GAPS * ui.spacing().item_spacing.x)
        .max(CLEAR_BUTTON_WIDTH + PATH_MARGIN.sum().x);
    let height = ui.spacing().interact_size.y;
    ui.horizontal(|ui| {
        ui.add_sized(
            [DRIVE_LABEL_WIDTH, height],
            egui::Label::new(format!("DW{drive}")),
        );
        disk_path_input(ui, drive, path, egui::vec2(field_width, height));
        let browse = ui.add_sized([BROWSE_BUTTON_WIDTH, height], egui::Button::new("Browse…"));
        browse.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                format!("Browse DW{drive}…"),
            )
        });
        if browse.clicked()
            && let Some(selected) = disk_dialog(path.as_deref()).pick_file()
        {
            *path = Some(selected.to_string_lossy().into_owned());
        }
    });
}

fn disk_path_input(ui: &mut egui::Ui, drive: usize, path: &mut Option<String>, size: egui::Vec2) {
    let mut text = path.clone().unwrap_or_default();
    let response = ui.add_sized(
        size,
        egui::TextEdit::singleline(&mut text)
            .id_salt("path")
            .hint_text(EMPTY_PATH_HINT)
            .margin(PATH_MARGIN),
    );
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::text_edit(
            ui.is_enabled(),
            path.as_deref().unwrap_or_default(),
            &text,
            EMPTY_PATH_HINT,
        );
        info.label = Some(format!("DW{drive} disk image"));
        info
    });
    if !text.is_empty() {
        let mut clear_rect = response.rect.shrink(CLEAR_BUTTON_INSET);
        clear_rect.min.x = clear_rect.max.x - CLEAR_BUTTON_WIDTH;
        let clear = ui
            .place(clear_rect, egui::Button::new("×").frame(false))
            .on_hover_text(format!("Clear DW{drive}"));
        clear.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                format!("Clear DW{drive}"),
            )
        });
        if clear.clicked() {
            text.clear();
        }
    }
    *path = (!text.is_empty()).then_some(text);
}

fn disk_dialog(path: Option<&str>) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new().add_filter("Disk image", DISK_IMAGE_EXTENSIONS);
    if let Some(path) = path.map(Path::new) {
        if let Some(parent) = path.parent().filter(|parent| parent.is_dir()) {
            dialog = dialog.set_directory(parent);
        }
        if let Some(name) = path.file_name() {
            dialog = dialog.set_file_name(name.to_string_lossy());
        }
    }
    dialog
}
