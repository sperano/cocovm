//! The machine form — model, RAM, cartridge, media, UI preferences — and
//! the direct-boot "Machine → New…" dialog around it.
//!
//! [`MachineForm`] holds the full editable draft and draws every row; it
//! never touches a machine or a file itself. Two hosts drive it: the
//! [`NewVmDialog`] window (direct boot: pick everything, then Create
//! cold-starts a fresh VM via `CocoApp::create_vm`), and the manager's
//! detail pane (`manager::draw_detail_ok`), which hosts the same form over
//! a saved machine definition and auto-saves each change. The `constrain`
//! rules below therefore live in exactly one place no matter who edits.

use std::path::PathBuf;

use coco_core::{
    MachineConfig, MachineVariant, MemorySize, MonitorType, VDGVariant, VideoStandard,
};
use eframe::egui;

/// RAM sizes selectable per machine — the same sets
/// [`MachineConfig::validate`] accepts (the configurations each machine
/// actually shipped in), so every config this dialog can produce validates.
const COCO1_RAM_CHOICES: &[MemorySize] = &[
    MemorySize::K4,
    MemorySize::K16,
    MemorySize::K32,
    MemorySize::K64,
];
const COCO2_RAM_CHOICES: &[MemorySize] = &[MemorySize::K16, MemorySize::K64];
const COCO3_RAM_CHOICES: &[MemorySize] = &[MemorySize::K128, MemorySize::K512, MemorySize::K2048];

/// Inner padding of the dialog body, matching the power-cycle confirmation
/// dialog in `main.rs`.
const DIALOG_MARGIN: i8 = 16;

/// Minimum size of the window's content area — roomy enough that revealing
/// the FD-502's Disk rows or the MPI's Slot rows (with nested Disk rows)
/// doesn't grow the window; the user may drag it larger. egui frames a
/// window around `last_content_size` alone (`Window::min_size` and
/// `default_size` only offer the content room, they never stretch the
/// frame), so the dialog claims this floor itself with `set_min_size`.
const DIALOG_MIN_SIZE: egui::Vec2 = egui::Vec2::new(380.0, 560.0);

/// The "New machine" shortcut, consumed by both the direct-boot Machine
/// menu ([`crate::CocoApp`]) and the manager's toolbar: ⌘N on macOS,
/// Ctrl+N on Windows/Linux ([`egui::Modifiers::COMMAND`] resolves to the
/// platform's primary modifier).
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

const fn ram_choices(variant: MachineVariant) -> &'static [MemorySize] {
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

/// Everything "Create" hands the caller besides the machine config: the
/// post-construction inventory (cartridge, its disks, the cassette) that
/// lives outside [`MachineConfig`] — see [`CartridgeChoice`].
#[derive(Debug, Clone)]
pub struct NewMachineSpec {
    pub config: MachineConfig,
    pub cartridge: CartridgeChoice,
    /// Only meaningful with [`CartridgeChoice::MPI`].
    pub mpi_slots: [SlotChoice; crate::MPI_SLOT_COUNT],
    /// Only meaningful when [`Self::has_drives`].
    pub disks: [MediaChoice; crate::UI_DRIVES],
    pub tape: MediaChoice,
    /// VHD hard-disk images. Always meaningful: the VHD is a bus device
    /// (`$FF80-$FF86`, `SystemBus::vhd`), not cartridge hardware, so the
    /// HD rows need no controller.
    pub vhds: [MediaChoice; crate::UI_DRIVES],
    /// The Display row: the created window's starting 4:3 aspect
    /// correction (F9 keeps toggling it live afterwards).
    pub aspect_correct: bool,
    /// The Keyboard row: the starting keyboard mode (F12 keeps toggling).
    pub kb_mode: crate::KbMode,
}

impl NewMachineSpec {
    /// Whether a disk controller is reachable: the bare FD-502, or one in
    /// an MPI slot.
    pub fn has_drives(&self) -> bool {
        match self.cartridge {
            CartridgeChoice::FD502 => true,
            CartridgeChoice::MPI => self.mpi_slots.contains(&SlotChoice::FD502),
            CartridgeChoice::None | CartridgeChoice::RomPak(_) | CartridgeChoice::RTC => false,
        }
    }
}

/// What the user clicked this frame, from [`NewVmDialog::show`].
#[must_use]
pub enum NewVmAction {
    None,
    /// "Create" was clicked; the caller should try to build this machine and
    /// either [`NewVmDialog::close`] the dialog or record the failure in
    /// [`NewVmDialog::error`].
    Create(Box<NewMachineSpec>),
}

/// Re-constrain a draft after a model change: snap RAM to the new family's
/// default when the current pick isn't valid for it, and force NTSC where
/// PAL isn't modeled ([`MachineConfig::validate`]'s rules). Free function
/// (rather than a `NewVmDialog` method) so [`config_form_rows`] can call it
/// too — the manager's detail pane edits a bare [`MachineConfig`], not a
/// dialog.
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

/// Shared hardware-config rows, all label + combo box: Machine, conditional
/// VDG (CoCo 2 only — see the inline comment below), RAM, Video (PAL only
/// for CoCo 3), conditional Monitor (CoCo 3 only). Must be called inside an
/// already-open two-column [`egui::Grid`]; `salt` distinguishes the
/// [`egui::ComboBox`]'s persistent id when this is drawn from more than one
/// call site in the same frame (the "New…" dialog *and* the manager's
/// detail pane can both be visible at once).
pub fn config_form_rows(ui: &mut egui::Ui, salt: &str, draft: &mut MachineConfig) {
    let font = ui.style().text_styles[&egui::TextStyle::Button].size;

    ui.label(egui::RichText::new("Machine").size(font));
    egui::ComboBox::from_id_salt((salt, "machine"))
        .selected_text(crate::machine_label(draft.variant))
        .show_ui(ui, |ui| {
            for variant in [
                MachineVariant::Coco1,
                MachineVariant::Coco2,
                MachineVariant::Coco3,
            ] {
                if ui
                    .selectable_value(&mut draft.variant, variant, crate::machine_label(variant))
                    .changed()
                {
                    constrain(draft);
                }
            }
        });
    ui.end_row();

    // The VDG choice only exists on the CoCo 2 (the CoCo 1 always shipped
    // the plain MC6847; the CoCo 3 has no VDG — the GIME does its own
    // character generation), so the row is only rendered for that model;
    // `constrain` re-seeds the family default for the others.
    if draft.variant == MachineVariant::Coco2 {
        ui.label(egui::RichText::new("VDG").size(font));
        let selected = draft.vdg.unwrap_or(VDGVariant::MC6847T1);
        egui::ComboBox::from_id_salt((salt, "vdg"))
            .selected_text(vdg_label(selected))
            .show_ui(ui, |ui| {
                for vdg in [VDGVariant::MC6847, VDGVariant::MC6847T1] {
                    ui.selectable_value(&mut draft.vdg, Some(vdg), vdg_label(vdg));
                }
            });
        ui.end_row();
    }

    ui.label(egui::RichText::new("RAM").size(font));
    egui::ComboBox::from_id_salt((salt, "ram"))
        .selected_text(ram_label(draft.memory))
        .show_ui(ui, |ui| {
            for &memory in ram_choices(draft.variant) {
                ui.selectable_value(&mut draft.memory, memory, ram_label(memory));
            }
        });
    ui.end_row();

    ui.label(egui::RichText::new("Video").size(font));
    // CoCo 1/2 PAL timing isn't modeled (`MachineConfig::validate`);
    // `constrain` already snapped the draft back to NTSC.
    let pal_possible = draft.variant == MachineVariant::Coco3;
    egui::ComboBox::from_id_salt((salt, "video"))
        .selected_text(video_label(draft.video))
        .show_ui(ui, |ui| {
            ui.selectable_value(
                &mut draft.video,
                VideoStandard::NTSC,
                video_label(VideoStandard::NTSC),
            );
            ui.add_enabled_ui(pal_possible, |ui| {
                ui.selectable_value(
                    &mut draft.video,
                    VideoStandard::PAL,
                    video_label(VideoStandard::PAL),
                )
                .on_disabled_hover_text("PAL is only supported on the CoCo 3");
            });
        });
    ui.end_row();

    // Monitor cable choice exists only on the CoCo 3 (RGB and composite
    // ports); a CoCo 1/2 outputs RF to a TV, full stop, and its config
    // carries `monitor: None` — see `constrain` and
    // [`MachineConfig::validate`].
    if draft.variant == MachineVariant::Coco3 {
        ui.label(egui::RichText::new("Monitor").size(font));
        let selected = draft.monitor.unwrap_or(MonitorType::RGB);
        egui::ComboBox::from_id_salt((salt, "monitor"))
            .selected_text(monitor_label(selected))
            .show_ui(ui, |ui| {
                for monitor in [MonitorType::RGB, MonitorType::Composite] {
                    ui.selectable_value(&mut draft.monitor, Some(monitor), monitor_label(monitor));
                }
            });
        ui.end_row();
    }
}

/// The full machine form both hosts draw: the hardware rows
/// ([`config_form_rows`]), then Cassette, Cartridge (with the nested
/// MPI-slot and Disk sub-rows), the HD rows, and the UI rows
/// (Display/Keyboard). The "New…" dialog ([`NewVmDialog`]) collects it into
/// a [`NewMachineSpec`] on Create; the manager's detail pane
/// (`manager::draw_detail_ok`) auto-saves it back into the machine's
/// definition on every change — but the rows, ordering, and constraint
/// rules live here exactly once.
pub struct MachineForm {
    /// Distinguishes the combos' persistent egui ids between hosts — the
    /// "New…" dialog and the manager's detail pane can be visible at once.
    salt: &'static str,
    /// Manager flow: a "Blank" media pick auto-places its file in the
    /// machine's artifact directory ([`MediaChoice::Blank`]`(None)`); the
    /// direct-boot dialog instead picks the backing file with a save dialog
    /// on the spot.
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
    /// The HD-row (VHD) picks. Always shown — see [`NewMachineSpec::vhds`].
    pub vhds: [MediaChoice; crate::UI_DRIVES],
    /// The Display row: 4:3 aspect correction (`[ui].aspect_correct`).
    pub aspect_correct: bool,
    /// The Keyboard row (`[ui].kb_mode`).
    pub kb_mode: crate::KbMode,
}

impl MachineForm {
    /// An all-defaults form. `salt` and `auto_place_blanks` are per-host
    /// constants — see the field docs.
    pub fn new(salt: &'static str, auto_place_blanks: bool) -> Self {
        Self {
            salt,
            auto_place_blanks,
            config: MachineConfig::default(),
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

    /// Reset every pick besides the hardware `config` and the UI rows to
    /// its default — the dialog reopens as "same machine again, nothing
    /// mounted".
    fn reset_inventory(&mut self) {
        self.cartridge = CartridgeChoice::None;
        self.mpi_slots = std::array::from_fn(|_| SlotChoice::Empty);
        self.disks = std::array::from_fn(|_| MediaChoice::None);
        self.tape = MediaChoice::None;
        self.vhds = std::array::from_fn(|_| MediaChoice::None);
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
        // cartridge required — see [`NewMachineSpec::vhds`].
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

    /// [`NewMachineSpec::has_drives`] over the form's own picks.
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

/// State of the direct-boot "New…" dialog: the [`MachineForm`] being
/// edited, plus the error from the last failed create attempt (e.g. a
/// missing ROM set), shown inline until the dialog closes or the next
/// attempt. The manager doesn't use this dialog at all — its "New…" creates
/// a default machine on the spot and edits it in the detail pane, which
/// hosts the same [`MachineForm`].
pub struct NewVmDialog {
    open: bool,
    pub error: Option<String>,
    pub form: MachineForm,
}

impl NewVmDialog {
    pub fn new() -> Self {
        Self {
            open: false,
            error: None,
            form: MachineForm::new("new_vm", false),
        }
    }

    /// Open the dialog with the form seeded from the running machine —
    /// config and UI preferences — so "New…" defaults to "same machine
    /// again", with nothing mounted.
    pub fn open_with(&mut self, current: MachineConfig, aspect_correct: bool, kb_mode: crate::KbMode) {
        self.form.reset_inventory();
        self.form.config = current;
        self.form.aspect_correct = aspect_correct;
        self.form.kb_mode = kb_mode;
        self.error = None;
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.error = None;
    }

    /// Draw the dialog if open. Returns [`NewVmAction::Create`] on the frame
    /// "Create" is clicked; the dialog stays open so a failure can be shown
    /// inline (the caller closes it on success).
    pub fn show(&mut self, ctx: &egui::Context) -> NewVmAction {
        if !self.open {
            return NewVmAction::None;
        }
        let mut action = NewVmAction::None;
        let font = ctx.style().text_styles[&egui::TextStyle::Button].size;
        let mut open = self.open;
        egui::Window::new(crate::window_title(ctx, "New Machine"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .min_size(DIALOG_MIN_SIZE)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // The frame only ever hugs the content (see
                // [`DIALOG_MIN_SIZE`]), so claim the floor — and any extra
                // room from a drag-resize — as the content's own size.
                ui.set_min_size(DIALOG_MIN_SIZE.max(ui.available_size()));
                egui::Frame::NONE.inner_margin(DIALOG_MARGIN).show(ui, |ui| {
                    egui::Grid::new("new_vm_grid")
                        .num_columns(2)
                        .spacing(FORM_GRID_SPACING)
                        .show(ui, |ui| {
                            self.form.rows(ui);
                        });

                    if let Some(error) = &self.error {
                        ui.add_space(DIALOG_MARGIN as f32 / 2.0);
                        ui.label(
                            egui::RichText::new(error)
                                .size(font)
                                .color(ui.visuals().error_fg_color),
                        );
                    }
                    ui.add_space(DIALOG_MARGIN as f32);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().button_padding = egui::vec2(12.0, 6.0);
                        if ui.button("Create").clicked() {
                            action = NewVmAction::Create(Box::new(NewMachineSpec {
                                config: self.form.config,
                                cartridge: self.form.cartridge.clone(),
                                mpi_slots: self.form.mpi_slots.clone(),
                                disks: self.form.disks.clone(),
                                tape: self.form.tape.clone(),
                                vhds: self.form.vhds.clone(),
                                aspect_correct: self.form.aspect_correct,
                                kb_mode: self.form.kb_mode,
                            }));
                        }
                        if ui.button("Cancel").clicked() {
                            self.close();
                        }
                    });
                });
            });
        // `open` only goes false via the title-bar close box; `self.close()`
        // inside the body must not be resurrected by writing `open` back.
        if !open {
            self.close();
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every config the dialog can produce must pass core validation — the
    /// choice lists and `constrain_draft` exist precisely to guarantee this.
    #[test]
    fn every_selectable_config_validates() {
        for variant in [
            MachineVariant::Coco1,
            MachineVariant::Coco2,
            MachineVariant::Coco3,
        ] {
            let videos: &[VideoStandard] = if variant == MachineVariant::Coco3 {
                &[VideoStandard::NTSC, VideoStandard::PAL]
            } else {
                &[VideoStandard::NTSC]
            };
            let vdgs: &[Option<VDGVariant>] = match variant {
                MachineVariant::Coco2 => {
                    &[Some(VDGVariant::MC6847), Some(VDGVariant::MC6847T1)]
                }
                MachineVariant::Coco1 => &[Some(VDGVariant::MC6847)],
                MachineVariant::Coco3 => &[None],
            };
            let monitors: &[Option<MonitorType>] = if variant == MachineVariant::Coco3 {
                &[Some(MonitorType::RGB), Some(MonitorType::Composite)]
            } else {
                &[None]
            };
            for &memory in ram_choices(variant) {
                for &video in videos {
                    for &monitor in monitors {
                        for &vdg in vdgs {
                            let config = MachineConfig {
                                variant,
                                video,
                                memory,
                                monitor,
                                vdg,
                            };
                            assert!(
                                config.validate().is_ok(),
                                "dialog offered invalid config: {config:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Switching model away from CoCo 3 must snap GIME-only RAM and PAL back
    /// to plain-SAM-valid values (and vice versa for RAM); switching models
    /// re-seeds the VDG family default (T1 on CoCo 2, plain MC6847 on
    /// CoCo 1, none on CoCo 3) and the monitor (a cable choice only where a
    /// monitor port exists — the CoCo 3).
    #[test]
    fn constrain_draft_snaps_family_specific_fields() {
        let mut dialog = NewVmDialog::new();
        dialog.open_with(
            MachineConfig {
                variant: MachineVariant::Coco3,
                video: VideoStandard::PAL,
                memory: MemorySize::K2048,
                monitor: Some(MonitorType::Composite),
                vdg: None,
            },
            true,
            crate::KbMode::Positional,
        );

        dialog.form.config.variant = MachineVariant::Coco2;
        constrain(&mut dialog.form.config);
        assert_eq!(dialog.form.config.memory, MemorySize::K64);
        assert_eq!(dialog.form.config.video, VideoStandard::NTSC);
        assert_eq!(
            dialog.form.config.vdg,
            Some(VDGVariant::MC6847T1),
            "CoCo 2 defaults to the T1 (CoCo 2B)"
        );
        assert_eq!(
            dialog.form.config.monitor, None,
            "a CoCo 2 has no monitor port to configure"
        );
        assert!(dialog.form.config.validate().is_ok());

        dialog.form.config.variant = MachineVariant::Coco3;
        constrain(&mut dialog.form.config);
        assert_eq!(dialog.form.config.memory, MemorySize::K512);
        assert_eq!(dialog.form.config.vdg, None);
        assert_eq!(
            dialog.form.config.monitor,
            Some(MonitorType::RGB),
            "returning to CoCo 3 re-seeds the default cable"
        );
        assert!(dialog.form.config.validate().is_ok());

        dialog.form.config.variant = MachineVariant::Coco1;
        constrain(&mut dialog.form.config);
        assert_eq!(dialog.form.config.vdg, Some(VDGVariant::MC6847));
        assert_eq!(dialog.form.config.monitor, None);
        assert!(dialog.form.config.validate().is_ok());
    }

    /// Re-opening seeds the draft from the running machine and clears any
    /// stale error from a previous failed attempt.
    #[test]
    fn open_with_seeds_draft_and_clears_error() {
        let mut dialog = NewVmDialog::new();
        dialog.error = Some("old failure".into());
        let current = MachineConfig {
            variant: MachineVariant::Coco1,
            video: VideoStandard::NTSC,
            memory: MemorySize::K16,
            monitor: None,
            vdg: Some(VDGVariant::MC6847),
        };
        dialog.open_with(current, false, crate::KbMode::Symbolic);
        assert!(dialog.open);
        assert!(dialog.error.is_none());
        assert_eq!(dialog.form.config.variant, MachineVariant::Coco1);
        assert_eq!(dialog.form.config.memory, MemorySize::K16);
    }
}
