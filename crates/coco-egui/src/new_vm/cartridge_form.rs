//! The Cartridge row's combo and its nested MPI-slot/Disk sub-rows —
//! [`super::form`]'s cartridge half, split out once `form.rs` grew past the
//! project's ~500-line ceiling. Free functions over borrowed form fields,
//! like `config_form`'s, rather than `MachineForm` methods.

use eframe::egui;

use super::cartridge::{
    CartridgeChoice, SlotChoice, cartridge_label, gmc_file_dialog, orch90_file_dialog,
    rom_pak_file_dialog, slot_label,
};
use super::{FORM_GRID_SPACING, MediaChoice, disk_file_dialog, media_choice_text, sub_form_row};

/// Whether a disk controller is reachable from the given cartridge/slot
/// picks: the bare FD-502, or one in an MPI slot.
pub(super) fn drives_available(
    cartridge: &CartridgeChoice,
    mpi_slots: &[SlotChoice; crate::MPI_SLOT_COUNT],
) -> bool {
    match cartridge {
        CartridgeChoice::FD502 => true,
        CartridgeChoice::MPI => mpi_slots.contains(&SlotChoice::FD502),
        CartridgeChoice::None
        | CartridgeChoice::ROMPak(_)
        | CartridgeChoice::RTC
        | CartridgeChoice::RS232
        | CartridgeChoice::GamesMaster(_)
        | CartridgeChoice::Orch90(_)
        | CartridgeChoice::SSC => false,
    }
}

/// The Cartridge row (label + combo) and its nested sub-form: FD-502's Disk
/// rows directly, or the MPI's four Slot rows.
pub(super) fn cartridge_row(
    ui: &mut egui::Ui,
    salt: &str,
    font: f32,
    cartridge: &mut CartridgeChoice,
    mpi_slots: &mut [SlotChoice; crate::MPI_SLOT_COUNT],
    disks: &mut [MediaChoice; crate::UI_DRIVES],
) {
    ui.label(egui::RichText::new("Cartridge").size(font));
    cartridge_combo(ui, salt, cartridge);
    ui.end_row();

    match cartridge {
        CartridgeChoice::FD502 => {
            sub_form_row(ui, |ui| disk_rows(ui, salt, font, disks));
        }
        CartridgeChoice::MPI => {
            sub_form_row(ui, |ui| slot_rows(ui, salt, font, mpi_slots, disks));
        }
        CartridgeChoice::None
        | CartridgeChoice::ROMPak(_)
        | CartridgeChoice::RTC
        | CartridgeChoice::RS232
        | CartridgeChoice::GamesMaster(_)
        | CartridgeChoice::Orch90(_)
        | CartridgeChoice::SSC => {}
    }
}

/// The MPI's four Slot rows as their own label+combo grid; the Disk
/// rows nest one level deeper under whichever slot holds the FD-502.
fn slot_rows(
    ui: &mut egui::Ui,
    salt: &str,
    font: f32,
    mpi_slots: &mut [SlotChoice; crate::MPI_SLOT_COUNT],
    disks: &mut [MediaChoice; crate::UI_DRIVES],
) {
    egui::Grid::new((salt, "slots"))
        .num_columns(2)
        .spacing(FORM_GRID_SPACING)
        .show(ui, |ui| {
            for slot in 0..crate::MPI_SLOT_COUNT {
                slot_combo(ui, salt, font, mpi_slots, slot);
                ui.end_row();
                if mpi_slots[slot] == SlotChoice::FD502 {
                    sub_form_row(ui, |ui| disk_rows(ui, salt, font, disks));
                }
            }
        });
}

/// The Disk rows as their own label+combo grid, one row per drive.
fn disk_rows(
    ui: &mut egui::Ui,
    salt: &str,
    font: f32,
    disks: &mut [MediaChoice; crate::UI_DRIVES],
) {
    egui::Grid::new((salt, "disks"))
        .num_columns(2)
        .spacing(FORM_GRID_SPACING)
        .show(ui, |ui| {
            for drive in 0..crate::UI_DRIVES {
                disk_combo(ui, salt, font, disks, drive);
                ui.end_row();
            }
        });
}

/// The Cartridge-row combo. Every image-backed pick ("ROM Pak…", "Games
/// Master…", "Orchestra-90…") opens a file dialog on the spot; a cancelled
/// dialog keeps the previous choice.
fn cartridge_combo(ui: &mut egui::Ui, salt: &str, cartridge: &mut CartridgeChoice) {
    egui::ComboBox::from_id_salt((salt, "cartridge"))
        .selected_text(cartridge_label(cartridge))
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(*cartridge == CartridgeChoice::None, "None")
                .clicked()
            {
                *cartridge = CartridgeChoice::None;
            }
            if ui
                .selectable_label(*cartridge == CartridgeChoice::FD502, "FD-502")
                .clicked()
            {
                *cartridge = CartridgeChoice::FD502;
            }
            if ui
                .selectable_label(matches!(cartridge, CartridgeChoice::ROMPak(_)), "ROM Pak…")
                .clicked()
                && let Some(path) = rom_pak_file_dialog().pick_file()
            {
                *cartridge = CartridgeChoice::ROMPak(path);
            }
            if ui
                .selectable_label(*cartridge == CartridgeChoice::RTC, "Disto RTC")
                .clicked()
            {
                *cartridge = CartridgeChoice::RTC;
            }
            if ui
                .selectable_label(*cartridge == CartridgeChoice::RS232, "RS-232 Pak")
                .clicked()
            {
                *cartridge = CartridgeChoice::RS232;
            }
            if ui
                .selectable_label(
                    matches!(cartridge, CartridgeChoice::GamesMaster(_)),
                    "Games Master…",
                )
                .clicked()
                && let Some(path) = gmc_file_dialog().pick_file()
            {
                *cartridge = CartridgeChoice::GamesMaster(path);
            }
            if ui
                .selectable_label(
                    matches!(cartridge, CartridgeChoice::Orch90(_)),
                    "Orchestra-90…",
                )
                .clicked()
                && let Some(path) = orch90_file_dialog().pick_file()
            {
                *cartridge = CartridgeChoice::Orch90(path);
            }
            if ui
                .selectable_label(*cartridge == CartridgeChoice::SSC, "Sound/Speech Cartridge")
                .clicked()
            {
                *cartridge = CartridgeChoice::SSC;
            }
            if ui
                .selectable_label(*cartridge == CartridgeChoice::MPI, "MultiPak Interface")
                .clicked()
            {
                *cartridge = CartridgeChoice::MPI;
            }
        });
}

/// One "Slot N:" label + combo, drawn while the MPI is selected. Claiming
/// the FD-502 or RTC releases it from any other slot (one controller/clock
/// max — [`SlotChoice`]'s doc); ROM Paks, the Games Master, Orchestra-90,
/// and the Sound/Speech Cartridge may each fill any number of slots.
fn slot_combo(
    ui: &mut egui::Ui,
    salt: &str,
    font: f32,
    mpi_slots: &mut [SlotChoice; crate::MPI_SLOT_COUNT],
    slot: usize,
) {
    ui.label(egui::RichText::new(format!("Slot {}:", slot + 1)).size(font));
    egui::ComboBox::from_id_salt((salt, "mpi_slot", slot))
        .selected_text(slot_label(&mpi_slots[slot]))
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(mpi_slots[slot] == SlotChoice::Empty, "Empty")
                .clicked()
            {
                mpi_slots[slot] = SlotChoice::Empty;
            }
            if ui
                .selectable_label(mpi_slots[slot] == SlotChoice::FD502, "FD-502")
                .clicked()
            {
                for other in mpi_slots.iter_mut() {
                    if *other == SlotChoice::FD502 {
                        *other = SlotChoice::Empty;
                    }
                }
                mpi_slots[slot] = SlotChoice::FD502;
            }
            if ui
                .selectable_label(matches!(mpi_slots[slot], SlotChoice::ROMPak(_)), "ROM Pak…")
                .clicked()
                && let Some(path) = rom_pak_file_dialog().pick_file()
            {
                mpi_slots[slot] = SlotChoice::ROMPak(path);
            }
            if ui
                .selectable_label(mpi_slots[slot] == SlotChoice::RTC, "Disto RTC")
                .clicked()
            {
                // One clock max: two would shadow each other at $FF50.
                for other in mpi_slots.iter_mut() {
                    if *other == SlotChoice::RTC {
                        *other = SlotChoice::Empty;
                    }
                }
                mpi_slots[slot] = SlotChoice::RTC;
            }
            if ui
                .selectable_label(
                    matches!(mpi_slots[slot], SlotChoice::GamesMaster(_)),
                    "Games Master…",
                )
                .clicked()
                && let Some(path) = gmc_file_dialog().pick_file()
            {
                mpi_slots[slot] = SlotChoice::GamesMaster(path);
            }
            if ui
                .selectable_label(
                    matches!(mpi_slots[slot], SlotChoice::Orch90(_)),
                    "Orchestra-90…",
                )
                .clicked()
                && let Some(path) = orch90_file_dialog().pick_file()
            {
                mpi_slots[slot] = SlotChoice::Orch90(path);
            }
            if ui
                .selectable_label(mpi_slots[slot] == SlotChoice::SSC, "Sound/Speech Cartridge")
                .clicked()
            {
                mpi_slots[slot] = SlotChoice::SSC;
            }
        });
}

/// One "Disk N:" label + combo, drawn while the FD-502 is selected. "Select…" opens a file
/// dialog on the spot; "Blank" is auto-placed in the machine's artifact directory at save time.
fn disk_combo(
    ui: &mut egui::Ui,
    salt: &str,
    font: f32,
    disks: &mut [MediaChoice; crate::UI_DRIVES],
    drive: usize,
) {
    ui.label(egui::RichText::new(format!("Disk {drive}:")).size(font));
    egui::ComboBox::from_id_salt((salt, "disk", drive))
        .selected_text(media_choice_text(&disks[drive]))
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(disks[drive] == MediaChoice::None, "None")
                .clicked()
            {
                disks[drive] = MediaChoice::None;
            }
            if ui
                .selectable_label(disks[drive] == MediaChoice::Blank, "Blank")
                .clicked()
            {
                disks[drive] = MediaChoice::Blank;
            }
            if ui
                .selectable_label(matches!(disks[drive], MediaChoice::File(_)), "Select…")
                .clicked()
                && let Some(path) = disk_file_dialog().pick_file()
            {
                disks[drive] = MediaChoice::File(path);
            }
        });
}
