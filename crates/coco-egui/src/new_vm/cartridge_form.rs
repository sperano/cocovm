//! The Cartridge row's combo and its nested MPI-slot/Disk sub-rows —
//! [`super::form`]'s cartridge half, split out once `form.rs` grew past the
//! project's ~500-line ceiling. Free functions over borrowed form fields,
//! like `config_form`'s, rather than `MachineForm` methods.

use crate::machine_def::DosRom;
use coco_core::MachineVariant;
use coco_core::rom_db::CartridgeHardware;
use eframe::egui;

use super::cartridge::{CartridgeChoice, CartridgeImageChoice, RS232EndpointChoice, SlotChoice};
use super::cartridge_combo::{cartridge_combo, disk_combo, slot_combo};
use super::{FORM_GRID_SPACING, MediaChoice, sub_form_row};

/// Whether a disk controller is reachable from the given cartridge/slot
/// picks: the bare FD-502, or one in an MPI slot.
pub(super) fn drives_available(
    cartridge: &CartridgeChoice,
    mpi_slots: &[SlotChoice; crate::MPI_SLOT_COUNT],
) -> bool {
    match cartridge {
        CartridgeChoice::FD502 { .. } => true,
        CartridgeChoice::MPI => mpi_slots
            .iter()
            .any(|slot| matches!(slot, SlotChoice::FD502 { .. })),
        CartridgeChoice::None
        | CartridgeChoice::Image(_)
        | CartridgeChoice::RTC
        | CartridgeChoice::RS232
        | CartridgeChoice::Orch90
        | CartridgeChoice::SoundSpeech
        | CartridgeChoice::CoCoMax => false,
    }
}

/// [`cartridge_row`]'s form-field borrows, bundled to keep the function's
/// own argument count down — one mutable reference per row/sub-form it may
/// need to draw or update.
pub(super) struct CartridgeRowState<'a> {
    pub(super) cartridge: &'a mut CartridgeChoice,
    pub(super) mpi_slots: &'a mut [SlotChoice; crate::MPI_SLOT_COUNT],
    pub(super) mpi_switch: &'a mut usize,
    pub(super) rs232_endpoint: &'a mut RS232EndpointChoice,
    pub(super) disks: &'a mut [MediaChoice; crate::UI_DRIVES],
    /// The draft's Model pick — gates the CoCo Max module combo entry
    /// (CoCo 1/2 only; [`super::form::MachineForm::media_rows`] drops an
    /// already-picked module the same way when the model moves to CoCo 3).
    pub(super) variant: MachineVariant,
}

/// The Cartridge row (label + combo) and its nested sub-form: FD-502's Disk
/// rows, the MPI's Switch/Slot rows, the RS-232 Pak's Endpoint row, or a
/// cartridge image's Hardware row.
pub(super) fn cartridge_row(ui: &mut egui::Ui, salt: &str, font: f32, state: CartridgeRowState) {
    let CartridgeRowState {
        cartridge,
        mpi_slots,
        mpi_switch,
        rs232_endpoint,
        disks,
        variant,
    } = state;

    ui.label(egui::RichText::new("Cartridge").size(font));
    cartridge_combo(ui, salt, cartridge, variant);
    ui.end_row();

    match cartridge {
        CartridgeChoice::FD502 { dos_rom } => {
            sub_form_row(ui, |ui| disk_rows(ui, salt, font, disks, dos_rom, variant));
        }
        CartridgeChoice::MPI => {
            sub_form_row(ui, |ui| {
                mpi_sub_form(ui, salt, font, mpi_switch, mpi_slots, disks, variant)
            });
        }
        CartridgeChoice::RS232 => {
            sub_form_row(ui, |ui| rs232_sub_form(ui, salt, font, rs232_endpoint));
        }
        CartridgeChoice::Image(image) => {
            sub_form_row(ui, |ui| cartridge_image_sub_form(ui, salt, font, image));
        }
        CartridgeChoice::None
        | CartridgeChoice::RTC
        | CartridgeChoice::Orch90
        | CartridgeChoice::SoundSpeech
        | CartridgeChoice::CoCoMax => {}
    }
}

/// The MPI's Switch row, then its four Slot rows; the Disk rows nest one
/// level deeper under whichever slot holds the FD-502, and a cartridge image
/// slot's Hardware row nests the same way.
fn mpi_sub_form(
    ui: &mut egui::Ui,
    salt: &str,
    font: f32,
    mpi_switch: &mut usize,
    mpi_slots: &mut [SlotChoice; crate::MPI_SLOT_COUNT],
    disks: &mut [MediaChoice; crate::UI_DRIVES],
    variant: MachineVariant,
) {
    egui::Grid::new((salt, "mpi"))
        .num_columns(2)
        .spacing(FORM_GRID_SPACING)
        .show(ui, |ui| {
            switch_combo(ui, salt, font, mpi_switch);
            ui.end_row();
            for slot in 0..crate::MPI_SLOT_COUNT {
                slot_combo(ui, salt, font, mpi_slots, slot, variant);
                ui.end_row();
                match &mut mpi_slots[slot] {
                    SlotChoice::FD502 { dos_rom } => {
                        sub_form_row(ui, |ui| disk_rows(ui, salt, font, disks, dos_rom, variant));
                    }
                    SlotChoice::RS232(endpoint) => {
                        sub_form_row(ui, |ui| rs232_sub_form(ui, salt, font, endpoint));
                    }
                    SlotChoice::Image(image) => {
                        sub_form_row(ui, |ui| {
                            cartridge_image_sub_form(ui, (salt, "slot", slot), font, image)
                        });
                    }
                    _ => {}
                }
            }
        });
}

/// The MPI's front-panel Switch row: which slot the power-on SCS/CTS decode
/// selects — a running program's own `$FF7F` write overrides it until the
/// next reset ([`crate::CocoApp::mpi_set_switch`]'s doc).
fn switch_combo(ui: &mut egui::Ui, salt: &str, font: f32, switch: &mut usize) {
    ui.label(egui::RichText::new("Switch").size(font));
    egui::ComboBox::from_id_salt((salt, "mpi_switch"))
        .selected_text(format!("Slot {}", *switch + 1))
        .show_ui(ui, |ui| {
            for slot in 0..crate::MPI_SLOT_COUNT {
                combo_item(ui, &format!("Slot {}", slot + 1), *switch == slot, || {
                    *switch = slot;
                });
            }
        });
}

/// The RS-232 Pak's Endpoint row, plus the TCP endpoint's Listen address row.
fn rs232_sub_form(ui: &mut egui::Ui, salt: &str, font: f32, endpoint: &mut RS232EndpointChoice) {
    egui::Grid::new((salt, "rs232"))
        .num_columns(2)
        .spacing(FORM_GRID_SPACING)
        .show(ui, |ui| {
            ui.label(egui::RichText::new("Endpoint").size(font));
            rs232_endpoint_combo(ui, salt, endpoint);
            ui.end_row();
            if let RS232EndpointChoice::TCP { listen } = endpoint {
                ui.label(egui::RichText::new("Listen address").size(font));
                ui.text_edit_singleline(listen);
                ui.end_row();
            }
        });
}

fn rs232_endpoint_label(endpoint: &RS232EndpointChoice) -> &'static str {
    match endpoint {
        RS232EndpointChoice::Loopback => "Loopback",
        RS232EndpointChoice::TCP { .. } => "TCP",
        #[cfg(unix)]
        RS232EndpointChoice::PTY => "PTY",
    }
}

fn rs232_endpoint_combo(ui: &mut egui::Ui, salt: &str, endpoint: &mut RS232EndpointChoice) {
    egui::ComboBox::from_id_salt((salt, "rs232_endpoint"))
        .selected_text(rs232_endpoint_label(endpoint))
        .show_ui(ui, |ui| {
            combo_item(
                ui,
                "Loopback",
                matches!(endpoint, RS232EndpointChoice::Loopback),
                || *endpoint = RS232EndpointChoice::Loopback,
            );
            combo_item(
                ui,
                "TCP",
                matches!(endpoint, RS232EndpointChoice::TCP { .. }),
                || {
                    *endpoint = RS232EndpointChoice::TCP {
                        listen: crate::RS232_TCP_DEFAULT_ADDR.to_string(),
                    };
                },
            );
            #[cfg(unix)]
            combo_item(
                ui,
                "PTY",
                matches!(endpoint, RS232EndpointChoice::PTY),
                || *endpoint = RS232EndpointChoice::PTY,
            );
        });
}

fn cartridge_hardware_label(hardware: CartridgeHardware) -> &'static str {
    match hardware {
        CartridgeHardware::RomPak => "ROM Pak",
        CartridgeHardware::BankedRomPak => "Banked ROM Pak",
        CartridgeHardware::GamesMaster => "Games Master Cartridge",
    }
}

fn cartridge_hardware_combo(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    hardware: &mut CartridgeHardware,
) {
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(format!(
            "{} (fallback)",
            cartridge_hardware_label(*hardware)
        ))
        .show_ui(ui, |ui| {
            for choice in [
                CartridgeHardware::RomPak,
                CartridgeHardware::BankedRomPak,
                CartridgeHardware::GamesMaster,
            ] {
                combo_item(
                    ui,
                    cartridge_hardware_label(choice),
                    *hardware == choice,
                    || *hardware = choice,
                );
            }
        });
}

/// Shows detected hardware as read-only. Unknown ROMs get a fallback selector.
fn cartridge_image_sub_form(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    font: f32,
    image: &mut CartridgeImageChoice,
) {
    egui::Grid::new(("cartridge_image", &id_salt))
        .num_columns(2)
        .spacing(FORM_GRID_SPACING)
        .show(ui, |ui| {
            ui.label(egui::RichText::new("Hardware").size(font));
            if image.hardware_detected {
                ui.label(format!(
                    "{} (detected)",
                    cartridge_hardware_label(image.hardware)
                ));
            } else {
                cartridge_hardware_combo(ui, &id_salt, &mut image.hardware);
            }
            ui.end_row();
        });
}

/// The Disk rows as their own label+combo grid, one row per drive.
fn disk_rows(
    ui: &mut egui::Ui,
    salt: &str,
    font: f32,
    disks: &mut [MediaChoice; crate::UI_DRIVES],
    dos_rom: &mut DosRom,
    variant: MachineVariant,
) {
    egui::Grid::new((salt, "disks"))
        .num_columns(2)
        .spacing(FORM_GRID_SPACING)
        .show(ui, |ui| {
            dos_rom_row(ui, salt, dos_rom, variant);
            for drive in 0..crate::UI_DRIVES {
                disk_combo(ui, salt, font, disks, drive);
                ui.end_row();
            }
        });
}

fn dos_rom_row(ui: &mut egui::Ui, salt: &str, dos_rom: &mut DosRom, variant: MachineVariant) {
    ui.label("DOS ROM");
    egui::ComboBox::from_id_salt((salt, "dos_rom"))
        .selected_text(dos_rom.label())
        .show_ui(ui, |ui| {
            ui.selectable_value(dos_rom, DosRom::DiskBasic, DosRom::DiskBasic.label());
            ui.add_enabled_ui(variant == MachineVariant::Coco3, |ui| {
                ui.selectable_value(dos_rom, DosRom::HdbDosDw3, DosRom::HdbDosDw3.label())
                    .on_disabled_hover_text("This HDB-DOS ROM requires a CoCo 3.");
            });
        });
    ui.end_row();
}

/// One combo entry: a label, whether it's the current selection, and what
/// picking it does — the shape shared by every Cartridge/Slot combo row, in
/// place of a repeated `selectable_label(...).clicked()` chase per variant.
/// `pub(super)` — [`super::cartridge_combo`]'s combo boxes use it too.
pub(super) fn combo_item(ui: &mut egui::Ui, label: &str, selected: bool, on_click: impl FnOnce()) {
    if ui.selectable_label(selected, label).clicked() {
        on_click();
    }
}
