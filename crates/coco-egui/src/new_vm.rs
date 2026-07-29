//! The machine form — model, RAM, cartridge, media, UI preferences.
//!
//! [`MachineForm`] holds the full editable draft and draws every row; it
//! never touches a machine or a file itself. Its one host is the manager's
//! detail pane (`manager::draw_detail_ok`), which draws the form over a
//! saved machine definition and auto-saves each change. (The direct-boot
//! "Machine → New…" dialog that used to co-host it was removed — machine
//! creation belongs to the manager; user decision 2026-07-28.) The
//! `constrain` rules below therefore live in exactly one place.

use std::path::PathBuf;

use coco_core::{
    MachineConfig, MachineVariant, MemorySize, MonitorType, VDGVariant, VideoStandard,
};
use eframe::egui;

mod config_form;
mod form;

/// RAM sizes selectable per machine — the same sets
/// [`MachineConfig::validate`] accepts (the configurations each machine
/// actually shipped in), so every config the form can produce validates.
const COCO1_RAM_CHOICES: &[MemorySize] = &[
    MemorySize::K4,
    MemorySize::K16,
    MemorySize::K32,
    MemorySize::K64,
];
const COCO2_RAM_CHOICES: &[MemorySize] = &[MemorySize::K16, MemorySize::K64];
const COCO3_RAM_CHOICES: &[MemorySize] = &[MemorySize::K128, MemorySize::K512, MemorySize::K2048];

/// The "New machine" shortcut, consumed only by the manager (toolbar
/// "New…" and its ⌘N): ⌘N on macOS, Ctrl+N on Windows/Linux
/// ([`egui::Modifiers::COMMAND`] resolves to the platform's primary
/// modifier). VM windows deliberately have no New shortcut — creating
/// machines is the manager's job.
pub const NEW_MACHINE_SHORTCUT: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::N);

/// Spacing of [`config_form_rows`]'s two-column grid. `pub(crate)`: the
/// manager's detail pane (`manager::draw_detail_ok`) hosts the same shared
/// rows in its own `egui::Grid` and must use this exact value too, or the
/// two hosts render the shared form with mismatched spacing.
pub(crate) const FORM_GRID_SPACING: [f32; 2] = [24.0, 10.0];

/// Horizontal shift of a nested sub-form (the FD-502's Disk rows, the
/// MPI's Slot rows) into its parent's combo column — each nesting level
/// steps this much further right.
const SUB_FORM_INDENT: f32 = 12.0;

/// One outer-grid row holding an indented sub-form: an empty label cell,
/// then the sub-form shifted [`SUB_FORM_INDENT`] into the combo column.
/// Top-aligned (`horizontal_top`): a plain `horizontal` vertically centers
/// the tall nested grid, opening an oversized gap above its first row.
fn sub_form_row(ui: &mut egui::Ui, draw: impl FnOnce(&mut egui::Ui)) {
    ui.label("");
    ui.horizontal_top(|ui| {
        ui.add_space(SUB_FORM_INDENT);
        draw(ui);
    });
    ui.end_row();
}

pub(crate) const fn ram_choices(variant: MachineVariant) -> &'static [MemorySize] {
    match variant {
        MachineVariant::Coco1 => COCO1_RAM_CHOICES,
        MachineVariant::Coco2 => COCO2_RAM_CHOICES,
        MachineVariant::Coco3 => COCO3_RAM_CHOICES,
    }
}

const fn vdg_label(vdg: VDGVariant) -> &'static str {
    match vdg {
        VDGVariant::MC6847 => "MC6847",
        VDGVariant::MC6847T1 => "MC6847T1 (CoCo 2B)",
    }
}

const fn video_label(video: VideoStandard) -> &'static str {
    match video {
        VideoStandard::NTSC => "NTSC",
        VideoStandard::PAL => "PAL",
    }
}

const fn monitor_label(monitor: MonitorType) -> &'static str {
    match monitor {
        MonitorType::RGB => "RGB",
        MonitorType::Composite => "Composite",
    }
}

/// `pub(crate)`: also used by `manager.rs`'s list-row subtitle ("CoCo 3 ·
/// 512K").
pub(crate) const fn ram_label(memory: MemorySize) -> &'static str {
    match memory {
        MemorySize::K4 => "4K",
        MemorySize::K16 => "16K",
        MemorySize::K32 => "32K",
        MemorySize::K64 => "64K",
        MemorySize::K128 => "128K",
        MemorySize::K512 => "512K",
        MemorySize::K2048 => "2048K",
    }
}

/// The dialog's Cartridge row. Not part of [`MachineConfig`] — the
/// cartridge port is populated after machine construction (the same way
/// the CLI and the Machine menu do it) — so it rides alongside the config
/// in [`NewVmAction::Create`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CartridgeChoice {
    #[default]
    None,
    /// FD-502 disk controller (Disk BASIC ROM + WD1773, empty drives).
    FD502,
    /// A program ROM Pak image plugged straight into the port; picked with
    /// a file dialog on selection.
    RomPak(PathBuf),
    /// Disto RTC plugged straight into the port. No boot ROM — pairs with
    /// a VHD boot; for RTC + floppies use an MPI slot.
    RTC,
    /// MultiPak Interface; the dialog then shows its four Slot rows, and
    /// the Disk rows only once a slot holds the FD-502.
    MPI,
}

/// One MultiPak slot's pick in the dialog's Slot rows (shown while the
/// cartridge is the MPI). At most one slot holds the FD-502 (a second
/// disk controller would fight the first for the SCS decode) and at most
/// one the Disto RTC (two would shadow each other at `$FF50`). ROM Paks
/// carry no such conflict: any number of slots may hold one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SlotChoice {
    #[default]
    Empty,
    FD502,
    /// A program ROM Pak image in this slot (see [`CartridgeChoice::RomPak`]).
    RomPak(PathBuf),
    /// Disto RTC in this slot (see [`CartridgeChoice::RTC`]).
    RTC,
}

fn slot_label(slot: &SlotChoice) -> String {
    match slot {
        SlotChoice::Empty => "Empty".to_string(),
        SlotChoice::FD502 => "FD-502".to_string(),
        SlotChoice::RomPak(path) => pak_file_name(path),
        SlotChoice::RTC => "Disto RTC".to_string(),
    }
}

fn cartridge_label(cartridge: &CartridgeChoice) -> String {
    match cartridge {
        CartridgeChoice::None => "None".to_string(),
        CartridgeChoice::FD502 => "FD-502".to_string(),
        CartridgeChoice::RomPak(path) => pak_file_name(path),
        CartridgeChoice::RTC => "Disto RTC".to_string(),
        CartridgeChoice::MPI => "MultiPak Interface".to_string(),
    }
}

/// Combo text for a picked ROM Pak ([`media_choice_text`]'s pak sibling).
fn pak_file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "ROM Pak".to_string())
}

/// The same filter the Machine-menu "Insert Cartridge…" item uses.
fn rom_pak_file_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("ROM Pak", &["rom", "ccc", "bin"])
}

/// One media pick — a drive's disk (Cartridge row, when the cartridge
/// [`CartridgeChoice::has_drives`]) or the cassette: what to mount at
/// create time.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MediaChoice {
    /// Empty drive.
    #[default]
    None,
    /// A fresh blank image (0-track disk / empty tape). Blank media is
    /// file-backed (the machine writes back to the host file): direct boot
    /// picks the backing file with a save dialog when this is selected
    /// (`Some(path)`); the manager auto-places a file in the machine's
    /// artifact directory (`None`).
    Blank(Option<PathBuf>),
    /// An existing image picked with the file dialog.
    File(PathBuf),
}

/// Closed-combo text: the choice name, or the chosen file's name.
fn media_choice_text(media: &MediaChoice) -> String {
    match media {
        MediaChoice::None => "None".to_string(),
        MediaChoice::Blank(None) => "Blank".to_string(),
        MediaChoice::Blank(Some(path)) | MediaChoice::File(path) => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Disk".to_string()),
    }
}

/// The same filter the Machine-menu disk items use.
fn disk_file_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("Disk image", &["dsk", "jvc", "os9"])
}

/// Re-constrain a draft after a model change: snap RAM to the new family's
/// default when the current pick isn't valid for it, and force NTSC where
/// PAL isn't modeled ([`MachineConfig::validate`]'s rules).
fn constrain(draft: &mut MachineConfig) {
    if !ram_choices(draft.variant).contains(&draft.memory) {
        // Same per-family default `main.rs`'s CLI path seeds `--ram` from.
        draft.memory = crate::default_ram(draft.variant);
    }
    if draft.variant != MachineVariant::Coco3 {
        draft.video = VideoStandard::NTSC;
        // A stock CoCo 1/2's only output is the RF modulator into a TV —
        // there is no monitor port ([`MachineConfig::validate`]).
        draft.monitor = None;
    } else {
        draft.monitor = Some(draft.monitor.unwrap_or(MonitorType::RGB));
    }
    // Only runs on model-change clicks, so an explicit MC6847 pick made
    // while staying on CoCo 2 sticks; switching models re-seeds the
    // family default (the T1 "CoCo 2B" for CoCo 2, the only-possible
    // plain MC6847 on CoCo 1, no VDG at all on CoCo 3 —
    // `MachineConfig::validate`).
    draft.vdg = crate::default_vdg(draft.variant);
}

/// The full machine form: the hardware rows ([`config_form_rows`]), then
/// Cassette, Cartridge (with the nested MPI-slot and Disk sub-rows), the
/// HD rows, and the UI rows (Display/Keyboard). The manager's detail pane
/// (`manager::draw_detail_ok`) auto-saves it back into the machine's
/// definition on every change — the rows, ordering, and constraint rules
/// live here exactly once.
pub struct MachineForm {
    /// Distinguishes the combos' persistent egui ids between hosts drawing
    /// the form more than once in the same frame.
    salt: &'static str,
    /// A "Blank" media pick auto-places its file in the machine's artifact
    /// directory ([`MediaChoice::Blank`]`(None)`) rather than opening a
    /// save dialog for the backing file.
    auto_place_blanks: bool,
    pub config: MachineConfig,
    /// The Cartridge-row pick.
    pub cartridge: CartridgeChoice,
    /// The MPI Slot picks (indented rows under the Cartridge combo);
    /// reset whenever the cartridge isn't the MPI.
    pub mpi_slots: [SlotChoice; crate::MPI_SLOT_COUNT],
    /// The per-drive disk picks, shown while a disk controller is
    /// reachable (bare FD-502, or FD-502 in an MPI slot); reset whenever no
    /// controller is reachable.
    pub disks: [MediaChoice; crate::UI_DRIVES],
    /// The Cassette-row pick.
    pub tape: MediaChoice,
    /// The HD-row (VHD) picks. Always shown: the VHD is a bus device
    /// (`$FF80-$FF86`, `SystemBus::vhd`), not cartridge hardware, so the
    /// HD rows need no controller.
    pub vhds: [MediaChoice; crate::UI_DRIVES],
    /// The Display row: 4:3 aspect correction (`[ui].aspect_correct`).
    pub aspect_correct: bool,
    /// The Keyboard row (`[ui].kb_mode`).
    pub kb_mode: crate::KbMode,
}

#[cfg(test)]
#[path = "new_vm_test.rs"]
mod tests;
