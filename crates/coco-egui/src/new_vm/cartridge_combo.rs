//! The Cartridge/Slot/Disk combo boxes themselves — split out of
//! `cartridge_form.rs` once that file grew past the project's ~500-line
//! ceiling. `cartridge_form.rs` keeps the row layout, the MPI/RS-232/image
//! sub-forms, and [`super::cartridge_form::combo_item`] (shared with those
//! sub-forms' own combos); this file is only the three combo boxes that
//! list a device's full choice set.

use std::path::PathBuf;

use coco_core::MachineVariant;
use eframe::egui;

use super::cartridge::{
    COCOMAX_LABEL, CartridgeChoice, RS232EndpointChoice, RTC_LABEL, SlotChoice, cartridge_label,
    cartridge_rom, cartridge_rom_file_dialog, slot_cartridge_rom, slot_label,
};
use super::cartridge_form::combo_item;
use super::known_cartridges::known_cartridge_submenu;
use super::{MediaChoice, disk_file_dialog, media_choice_text};

/// [`combo_item`]'s image-backed sibling: opens `dialog` on click and, unless
/// it's cancelled, calls `set` with the picked path — the ROM Pak/Games
/// Master combo entries' shared shape.
fn image_combo_item(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    dialog: impl FnOnce() -> rfd::FileDialog,
    set: impl FnOnce(PathBuf),
) {
    combo_item(ui, label, selected, || {
        if let Some(path) = dialog().pick_file() {
            set(path);
        }
    });
}

/// Release `kind` from every slot — the FD-502/RTC's one-max rule
/// ([`SlotChoice`]'s doc) before a slot claims it.
fn release_slot(mpi_slots: &mut [SlotChoice; crate::MPI_SLOT_COUNT], kind: SlotChoice) {
    release_slot_matching(mpi_slots, |other| *other == kind);
}

/// [`release_slot`]'s predicate-based sibling, for a kind whose payload
/// varies (the RS-232 Pak's endpoint); returns the released choice so a
/// pak moved between slots keeps its settings.
fn release_slot_matching(
    mpi_slots: &mut [SlotChoice; crate::MPI_SLOT_COUNT],
    predicate: impl Fn(&SlotChoice) -> bool,
) -> Option<SlotChoice> {
    let mut released = None;
    for other in mpi_slots.iter_mut() {
        if predicate(other) {
            released = Some(std::mem::replace(other, SlotChoice::Empty));
        }
    }
    released
}

/// The Cartridge-row combo. Cartridge hardware is detected after the unified
/// cartridge ROM picker returns a file. `variant` gates the CoCo Max module
/// entry — CoCo 1/2 hardware only, since the CoCo 3's GIME owns its
/// `$FF90-$FF97` window.
pub(super) fn cartridge_combo(
    ui: &mut egui::Ui,
    salt: &str,
    cartridge: &mut CartridgeChoice,
    variant: MachineVariant,
) {
    egui::ComboBox::from_id_salt((salt, "cartridge"))
        .selected_text(cartridge_label(cartridge))
        .show_ui(ui, |ui| {
            combo_item(ui, "None", *cartridge == CartridgeChoice::None, || {
                *cartridge = CartridgeChoice::None
            });
            combo_item(
                ui,
                "FD-502",
                matches!(cartridge, CartridgeChoice::FD502 { .. }),
                || {
                    if !matches!(cartridge, CartridgeChoice::FD502 { .. }) {
                        *cartridge = CartridgeChoice::FD502 {
                            dos_rom: Default::default(),
                        };
                    }
                },
            );
            image_combo_item(
                ui,
                "Cartridge ROM…",
                matches!(cartridge, CartridgeChoice::Image(_)),
                cartridge_rom_file_dialog,
                |path| *cartridge = cartridge_rom(path),
            );
            let current_image_path = match cartridge {
                CartridgeChoice::Image(image) => Some(image.path.clone()),
                _ => None,
            };
            known_cartridge_submenu(ui, current_image_path.as_deref(), |path| {
                *cartridge = cartridge_rom(path)
            });
            combo_item(ui, RTC_LABEL, *cartridge == CartridgeChoice::RTC, || {
                *cartridge = CartridgeChoice::RTC
            });
            combo_item(
                ui,
                "RS-232 Pak",
                *cartridge == CartridgeChoice::RS232,
                || *cartridge = CartridgeChoice::RS232,
            );
            combo_item(
                ui,
                "Orchestra-90",
                *cartridge == CartridgeChoice::Orch90,
                || *cartridge = CartridgeChoice::Orch90,
            );
            combo_item(
                ui,
                "Sound/Speech Cartridge",
                *cartridge == CartridgeChoice::SoundSpeech,
                || *cartridge = CartridgeChoice::SoundSpeech,
            );
            if variant != MachineVariant::Coco3 {
                combo_item(
                    ui,
                    COCOMAX_LABEL,
                    *cartridge == CartridgeChoice::CoCoMax,
                    || *cartridge = CartridgeChoice::CoCoMax,
                );
            }
            combo_item(
                ui,
                "MultiPak Interface",
                *cartridge == CartridgeChoice::MPI,
                || *cartridge = CartridgeChoice::MPI,
            );
        });
}

/// One "Slot N:" label + combo, drawn while the MPI is selected. Claiming
/// the FD-502 or RTC releases it from any other slot (one controller/clock
/// max — [`SlotChoice`]'s doc); cartridge ROMs, Orchestra-90, and the
/// Sound/Speech Cartridge may each fill any number of slots.
pub(super) fn slot_combo(
    ui: &mut egui::Ui,
    salt: &str,
    font: f32,
    mpi_slots: &mut [SlotChoice; crate::MPI_SLOT_COUNT],
    slot: usize,
    variant: MachineVariant,
) {
    ui.label(egui::RichText::new(format!("Slot {}:", slot + 1)).size(font));
    egui::ComboBox::from_id_salt((salt, "mpi_slot", slot))
        .selected_text(slot_label(&mpi_slots[slot]))
        .show_ui(ui, |ui| {
            combo_item(ui, "Empty", mpi_slots[slot] == SlotChoice::Empty, || {
                mpi_slots[slot] = SlotChoice::Empty
            });
            combo_item(
                ui,
                "FD-502",
                matches!(mpi_slots[slot], SlotChoice::FD502 { .. }),
                || {
                    let controller = release_slot_matching(mpi_slots, |slot| {
                        matches!(slot, SlotChoice::FD502 { .. })
                    })
                    .unwrap_or(SlotChoice::FD502 {
                        dos_rom: Default::default(),
                    });
                    mpi_slots[slot] = controller;
                },
            );
            image_combo_item(
                ui,
                "Cartridge ROM…",
                matches!(mpi_slots[slot], SlotChoice::Image(_)),
                cartridge_rom_file_dialog,
                |path| mpi_slots[slot] = slot_cartridge_rom(path),
            );
            let current_image_path = match &mpi_slots[slot] {
                SlotChoice::Image(image) => Some(image.path.clone()),
                _ => None,
            };
            known_cartridge_submenu(ui, current_image_path.as_deref(), |path| {
                mpi_slots[slot] = slot_cartridge_rom(path)
            });
            combo_item(ui, RTC_LABEL, mpi_slots[slot] == SlotChoice::RTC, || {
                release_slot(mpi_slots, SlotChoice::RTC);
                mpi_slots[slot] = SlotChoice::RTC;
            });
            combo_item(
                ui,
                "RS-232 Pak",
                matches!(mpi_slots[slot], SlotChoice::RS232(_)),
                || {
                    let endpoint = match release_slot_matching(mpi_slots, |s| {
                        matches!(s, SlotChoice::RS232(_))
                    }) {
                        Some(SlotChoice::RS232(endpoint)) => endpoint,
                        _ => RS232EndpointChoice::default(),
                    };
                    mpi_slots[slot] = SlotChoice::RS232(endpoint);
                },
            );
            combo_item(
                ui,
                "Orchestra-90",
                mpi_slots[slot] == SlotChoice::Orch90,
                || mpi_slots[slot] = SlotChoice::Orch90,
            );
            combo_item(
                ui,
                "Sound/Speech Cartridge",
                mpi_slots[slot] == SlotChoice::SoundSpeech,
                || mpi_slots[slot] = SlotChoice::SoundSpeech,
            );
            if variant != MachineVariant::Coco3 {
                combo_item(
                    ui,
                    COCOMAX_LABEL,
                    mpi_slots[slot] == SlotChoice::CoCoMax,
                    || mpi_slots[slot] = SlotChoice::CoCoMax,
                );
            }
        });
}

/// One "Disk N:" label + combo, drawn while the FD-502 is selected. "Select…" opens a file
/// dialog on the spot; "Blank" is auto-placed in the machine's artifact directory at save time.
pub(super) fn disk_combo(
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
