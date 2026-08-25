//! The machine form — model, RAM, cartridge, media, UI preferences.
//!
//! [`MachineForm`] holds the full editable draft and draws every row; it
//! never touches a machine or a file itself. Its one host is the manager's
//! detail pane (`manager::draw_detail_ok`), which draws the form over a
//! saved machine definition and auto-saves each change. Machine creation
//! belongs to the manager. The `constrain` rules
//! below therefore live in exactly one place.

use std::path::PathBuf;

use coco_core::{MachineConfig, MachineVariant, MemorySize, VDGVariant, VideoStandard};
use eframe::egui;

use crate::display::Display;
use crate::machine_def::SerialDTO;

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

/// Spacing of every form grid — the detail pane's section grids
/// (`manager::draw_detail_ok`) and the nested Slot/Disk sub-grids alike —
/// so the sections render as one visually continuous form.
pub(crate) const FORM_GRID_SPACING: [f32; 2] = [24.0, 10.0];

/// Minimum column width of the detail pane's section grids. Each
/// `egui::Grid` sizes its label column from its own cells only, so
/// without a shared floor the Machine grid's combos (label "Machine")
/// would start at a different x than the media grid's (label
/// "Cartridge") — the sections must line up like the single grid they
/// replaced.
pub(crate) const FORM_LABEL_MIN_WIDTH: f32 = 70.0;

/// Horizontal shift of a nested sub-form (the FD-502's Disk rows, the
/// MPI's Slot rows) into its parent's combo column — each nesting level
/// steps this much further right.
const SUB_FORM_INDENT: f32 = 12.0;

/// One outer-grid row holding an indented sub-form: an empty label cell, then the sub-form
/// shifted [`SUB_FORM_INDENT`] into the combo column. Top-aligned — a plain `horizontal` would vertically center the tall nested grid, opening a gap above its first row.
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

/// The form's Cartridge row. Not part of [`MachineConfig`] — the
/// cartridge port is populated after machine construction (the same way
/// `launch::mount_peripherals` and the Machine menu do it) — so it rides
/// alongside the config in [`MachineForm`] and is packed into the definition's
/// `[peripherals]`/`[media]` sections by the manager
/// (`manager::detail::pack_def`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CartridgeChoice {
    #[default]
    None,
    /// FD-502 disk controller (Disk BASIC ROM + WD1773, empty drives).
    FD502,
    /// A program ROM Pak image plugged straight into the port; picked with
    /// a file dialog on selection.
    ROMPak(PathBuf),
    /// Disto RTC plugged straight into the port. No boot ROM — pairs with
    /// a VHD boot; for RTC + floppies use an MPI slot.
    RTC,
    /// Deluxe RS-232 Pak plugged straight into the port. Bare-port only —
    /// unlike the RTC/FD-502 there's no `mpi_insert_rs232`, so this choice
    /// isn't offered in the MPI's Slot combos.
    RS232,
    /// MultiPak Interface; the form then shows its four Slot rows, and
    /// the Disk rows only once a slot holds the FD-502.
    MPI,
}

/// One MultiPak slot's pick in the form's Slot rows (shown while the
/// cartridge is the MPI). At most one slot holds the FD-502 (a second
/// disk controller would fight the first for the SCS decode) and at most
/// one the Disto RTC (two would shadow each other at `$FF50`). ROM Paks
/// carry no such conflict: any number of slots may hold one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SlotChoice {
    #[default]
    Empty,
    FD502,
    /// A program ROM Pak image in this slot (see [`CartridgeChoice::ROMPak`]).
    ROMPak(PathBuf),
    /// Disto RTC in this slot (see [`CartridgeChoice::RTC`]).
    RTC,
}

fn slot_label(slot: &SlotChoice) -> String {
    match slot {
        SlotChoice::Empty => "Empty".to_string(),
        SlotChoice::FD502 => "FD-502".to_string(),
        SlotChoice::ROMPak(path) => pak_file_name(path),
        SlotChoice::RTC => "Disto RTC".to_string(),
    }
}

fn cartridge_label(cartridge: &CartridgeChoice) -> String {
    match cartridge {
        CartridgeChoice::None => "None".to_string(),
        CartridgeChoice::FD502 => "FD-502".to_string(),
        CartridgeChoice::ROMPak(path) => pak_file_name(path),
        CartridgeChoice::RTC => "Disto RTC".to_string(),
        CartridgeChoice::RS232 => "RS-232 Pak".to_string(),
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

/// One media pick — a drive's disk (shown when a disk controller is
/// reachable, [`MachineForm::drives_available`]), a VHD, or the cassette.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MediaChoice {
    /// Empty drive.
    #[default]
    None,
    /// A fresh blank image (0-track disk / empty tape / 0-sector VHD).
    /// Blank media is file-backed (the machine writes back to the host
    /// file); the backing file is auto-placed in the machine's artifact
    /// directory when the definition saves
    /// (`manager::detail::record_media_choice`).
    Blank,
    /// An existing image picked with the file dialog.
    File(PathBuf),
}

/// Closed-combo text: the choice name, or the chosen file's name.
fn media_choice_text(media: &MediaChoice) -> String {
    match media {
        MediaChoice::None => "None".to_string(),
        MediaChoice::Blank => "Blank".to_string(),
        MediaChoice::File(path) => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Disk".to_string()),
    }
}

/// The same filter the Machine-menu disk items use.
fn disk_file_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("Disk image", &["dsk", "jvc", "os9"])
}

/// The form's Ports/Serial row: what host sink the built-in bit-banger
/// serial port (the 4-pin DIN every CoCo has, distinct from the Deluxe
/// RS-232 Pak's cartridge-port ACIA) starts wired to. No path payload — the
/// file mode always captures to the auto-named `printout.txt` in the
/// machine's artifact directory; a custom capture path remains a
/// runtime-menu-only feature (`CocoApp::start_print_capture`'s file dialog).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SerialChoice {
    #[default]
    None,
    /// A DMP-105 dot-matrix printer, shown in the Printer Paper window.
    Printer,
    /// Plain text capture to `printout.txt`.
    PrintFile,
}

impl SerialChoice {
    pub const ALL: [Self; 3] = [Self::None, Self::Printer, Self::PrintFile];
}

fn serial_label(serial: SerialChoice) -> &'static str {
    match serial {
        SerialChoice::None => "None",
        SerialChoice::Printer => "Printer (DMP-105)",
        SerialChoice::PrintFile => "Print to file",
    }
}

// `[ports].serial`'s `From` impls, kept here (rather than beside
// `machine_def::SerialDTO` itself) so `manager::detail_map` (form ⇄
// definition, its only caller) shares one conversion instead of hand-rolling
// its own match — the same reasoning as `JoySource`'s pair in `joy.rs`.
// `SerialDTO` has no `None` variant of its own (absence is the section's
// `Option`, not a variant — `PortsDTO::serial`'s doc), so the DTO side of
// this pair is `Option<SerialDTO>` rather than `SerialDTO` itself.
impl From<SerialChoice> for Option<SerialDTO> {
    fn from(serial: SerialChoice) -> Self {
        match serial {
            SerialChoice::None => None,
            SerialChoice::Printer => Some(SerialDTO::Printer),
            SerialChoice::PrintFile => Some(SerialDTO::File),
        }
    }
}

impl From<Option<SerialDTO>> for SerialChoice {
    fn from(serial: Option<SerialDTO>) -> Self {
        match serial {
            None => Self::None,
            Some(SerialDTO::Printer) => Self::Printer,
            Some(SerialDTO::File) => Self::PrintFile,
        }
    }
}

/// Re-constrains a draft after a model change: snaps RAM to the new family's default when
/// invalid, and forces NTSC where PAL isn't modeled.
fn constrain(draft: &mut MachineConfig) {
    if !ram_choices(draft.variant).contains(&draft.memory) {
        draft.memory = crate::default_ram(draft.variant);
    }
    if draft.variant != MachineVariant::Coco3 {
        draft.video = VideoStandard::NTSC;
    }
    // `draft.monitor` isn't touched here — the form's Display pick owns it, since config alone can't tell a CoCo 3 TV from a composite monitor.
    // Only runs on model-change clicks, so an explicit MC6847 pick while staying on CoCo 2 sticks.
    draft.vdg = crate::default_vdg(draft.variant);
}

/// The full machine form, drawn in sections ([`MachineForm::machine_rows`],
/// [`MachineForm::display_rows`], [`MachineForm::media_rows`],
/// [`MachineForm::ports_rows`], [`MachineForm::joystick_row`],
/// [`MachineForm::keyboard_row`]) so the detail pane can interleave its
/// titled groups between them. The manager's detail pane
/// (`manager::draw_detail_ok`) auto-saves it back into the machine's
/// definition on every change — the rows, ordering, and constraint rules
/// live here exactly once.
pub struct MachineForm {
    /// Distinguishes the combos' persistent egui ids between hosts drawing
    /// the form more than once in the same frame.
    salt: &'static str,
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
    /// The VHD-row picks. Always shown: the VHD is a bus device
    /// (`$FF80-$FF86`, `SystemBus::vhd`), not cartridge hardware, so the
    /// VHD rows need no controller.
    pub vhds: [MediaChoice; crate::UI_DRIVES],
    /// The Display-row pick (`[hardware].display`): monitor or (B&W) TV.
    /// Owns `config.monitor` — see [`MachineForm::display_rows`].
    pub display: Display,
    /// The TV chain's knobs (`[ui].tv_scanline`, …), edited under the
    /// Display row while a TV is picked.
    pub tv: crate::display::TVSettings,
    /// The Display group's 4:3 checkbox (`[ui].aspect_correct`).
    pub aspect_correct: bool,
    /// The Ports fieldset's Serial-row pick (`[ports].serial`).
    pub serial: SerialChoice,
    /// The Joysticks fieldset's picks (`[ui].joy_left`/`joy_right`), indexed
    /// by `coco_core::joystick::{RIGHT, LEFT}` like
    /// `crate::joy::JoystickInputs::sources`.
    pub joy_sources: [crate::joy::JoySource; 2],
    /// The Keyboard fieldset's pick (`[ui].kb_mode`).
    pub kb_mode: crate::KbMode,
}

#[cfg(test)]
#[path = "new_vm_test.rs"]
mod tests;
