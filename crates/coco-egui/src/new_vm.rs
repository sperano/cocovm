//! "Machine → New…" dialog: pick a machine model and its parameters, then
//! cold-start a fresh VM from the resulting [`MachineConfig`]. The dialog
//! only *builds* the config — swapping the running machine (and writing back
//! dirty media first) is `CocoApp::create_vm`'s job, so this module stays a
//! pure view over a draft config.
//!
//! [`config_form_rows`] — the Model/VDG/RAM/Video/Monitor grid rows — is
//! shared with the manager's detail pane (`manager::draw_detail`), so the
//! `constrain` constraint behavior below lives in exactly one place no
//! matter which caller edits the draft.

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CartridgeChoice {
    #[default]
    None,
    /// FD-502 disk controller (Disk BASIC ROM + WD1773, empty drives).
    FD502,
}

const fn cartridge_label(cartridge: CartridgeChoice) -> &'static str {
    match cartridge {
        CartridgeChoice::None => "None",
        CartridgeChoice::FD502 => "FD-502",
    }
}

/// The Cartridge row's "Disk 0" companion, shown only with
/// [`CartridgeChoice::FD502`]: what to mount in drive 0 at create time.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DiskChoice {
    /// Empty drive.
    #[default]
    None,
    /// A fresh blank (0-track) image. Blank disks are file-backed (the
    /// drive writes back to the host file): direct boot picks the backing
    /// file with a save dialog when this is selected (`Some(path)`); the
    /// manager auto-places `disk0.dsk` in the machine's artifact directory
    /// (`None`).
    Blank(Option<PathBuf>),
    /// An existing image picked with the file dialog.
    File(PathBuf),
}

/// Closed-combo text: the choice name, or the chosen file's name.
fn disk_choice_text(disk: &DiskChoice) -> String {
    match disk {
        DiskChoice::None => "None".to_string(),
        DiskChoice::Blank(None) => "Blank".to_string(),
        DiskChoice::Blank(Some(path)) | DiskChoice::File(path) => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Disk".to_string()),
    }
}

/// The same filter the Machine-menu disk items use.
fn disk_file_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("Disk image", &["dsk", "jvc", "os9"])
}

/// What the user clicked this frame, from [`NewVmDialog::show`].
#[must_use]
pub enum NewVmAction {
    None,
    /// "Create" was clicked; the caller should try to build this machine and
    /// either [`NewVmDialog::close`] the dialog or record the failure in
    /// [`NewVmDialog::error`]. The [`DiskChoice`] is only meaningful with
    /// [`CartridgeChoice::FD502`].
    Create(MachineConfig, CartridgeChoice, DiskChoice),
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

/// State of the "New…" dialog: a draft [`MachineConfig`] being edited, plus
/// the error from the last failed create attempt (e.g. a missing ROM set),
/// shown inline until the dialog closes or the next attempt.
pub struct NewVmDialog {
    open: bool,
    draft: MachineConfig,
    pub error: Option<String>,
    /// Whether the "Name" row is drawn (the manager's flow needs a display
    /// name; `CocoApp`'s direct-boot flow doesn't — swapping the running
    /// machine doesn't rename anything).
    show_name_field: bool,
    /// The name-row draft, meaningful only when `show_name_field` is set.
    pub name: String,
    /// The Cartridge-row draft; reset to None on every open.
    pub cartridge: CartridgeChoice,
    /// The Disk 0 draft next to the Cartridge combo; reset to None on every
    /// open, and whenever the cartridge isn't the FD-502.
    pub disk0: DiskChoice,
}

impl NewVmDialog {
    pub fn new() -> Self {
        Self {
            open: false,
            draft: MachineConfig::default(),
            error: None,
            show_name_field: false,
            name: String::new(),
            cartridge: CartridgeChoice::None,
            disk0: DiskChoice::None,
        }
    }

    /// [`Self::new`] with the "Name" row enabled, for the manager's "New…"
    /// flow (`manager.rs`).
    pub fn new_for_manager() -> Self {
        Self {
            show_name_field: true,
            ..Self::new()
        }
    }

    /// Open the dialog with the draft seeded from `current` (the running
    /// machine's config), so "New…" defaults to "same machine again".
    pub fn open_with(&mut self, current: MachineConfig) {
        self.draft = current;
        self.error = None;
        self.cartridge = CartridgeChoice::None;
        self.disk0 = DiskChoice::None;
        self.open = true;
    }

    /// [`Self::open_with`] that also seeds the "New Machine" title row —
    /// the manager's "New…" flow, which has no "running machine" to default
    /// from, so both the config and the display name are given explicitly.
    pub fn open_new(&mut self, config: MachineConfig, name: impl Into<String>) {
        self.name = name.into();
        self.open_with(config);
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
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Frame::NONE.inner_margin(DIALOG_MARGIN).show(ui, |ui| {
                    egui::Grid::new("new_vm_grid")
                        .num_columns(2)
                        .spacing(FORM_GRID_SPACING)
                        .show(ui, |ui| {
                            if self.show_name_field {
                                let name_label =
                                    ui.label(egui::RichText::new("Name").size(font));
                                ui.text_edit_singleline(&mut self.name)
                                    .labelled_by(name_label.id);
                                ui.end_row();
                            }
                            config_form_rows(ui, "new_vm", &mut self.draft);

                            // Dialog-only row (not `config_form_rows`): the
                            // cartridge isn't part of `MachineConfig` — see
                            // [`CartridgeChoice`] — and the manager's detail
                            // pane edits peripherals through its own
                            // checkboxes.
                            if self.cartridge != CartridgeChoice::FD502 {
                                self.disk0 = DiskChoice::None;
                            }
                            ui.label(egui::RichText::new("Cartridge").size(font));
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt(("new_vm", "cartridge"))
                                    .selected_text(cartridge_label(self.cartridge))
                                    .show_ui(ui, |ui| {
                                        for choice in
                                            [CartridgeChoice::None, CartridgeChoice::FD502]
                                        {
                                            ui.selectable_value(
                                                &mut self.cartridge,
                                                choice,
                                                cartridge_label(choice),
                                            );
                                        }
                                    });
                                if self.cartridge == CartridgeChoice::FD502 {
                                    self.disk0_combo(ui, font);
                                }
                            });
                            ui.end_row();
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
                    ui.label(
                        egui::RichText::new(
                            "Creating a new machine replaces the current one — any \
                             unsaved work in memory will be lost.",
                        )
                        .size(font)
                        .weak(),
                    );
                    ui.add_space(DIALOG_MARGIN as f32);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().button_padding = egui::vec2(12.0, 6.0);
                        if ui.button("Create").clicked() {
                            action = NewVmAction::Create(
                                self.draft,
                                self.cartridge,
                                self.disk0.clone(),
                            );
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

    /// The "Disk 0:" label + combo drawn to the right of the Cartridge
    /// combo while the FD-502 is selected. "Blank" and "Select…" open
    /// native file dialogs on the spot (save-file and open-file
    /// respectively) — except the manager flow's "Blank"
    /// (`show_name_field`), which is auto-placed in the machine's artifact
    /// directory at create time and needs no path here. A cancelled dialog
    /// falls back to None rather than keeping a pathless choice.
    fn disk0_combo(&mut self, ui: &mut egui::Ui, font: f32) {
        ui.label(egui::RichText::new("Disk 0:").size(font));
        egui::ComboBox::from_id_salt(("new_vm", "disk0"))
            .selected_text(disk_choice_text(&self.disk0))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(self.disk0 == DiskChoice::None, "None")
                    .clicked()
                {
                    self.disk0 = DiskChoice::None;
                }
                if ui
                    .selectable_label(matches!(self.disk0, DiskChoice::Blank(_)), "Blank")
                    .clicked()
                {
                    self.disk0 = if self.show_name_field {
                        DiskChoice::Blank(None)
                    } else {
                        match disk_file_dialog().set_file_name("blank.dsk").save_file() {
                            Some(path) => DiskChoice::Blank(Some(path)),
                            None => DiskChoice::None,
                        }
                    };
                }
                if ui
                    .selectable_label(matches!(self.disk0, DiskChoice::File(_)), "Select…")
                    .clicked()
                {
                    if let Some(path) = disk_file_dialog().pick_file() {
                        self.disk0 = DiskChoice::File(path);
                    }
                }
            });
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
        dialog.open_with(MachineConfig {
            variant: MachineVariant::Coco3,
            video: VideoStandard::PAL,
            memory: MemorySize::K2048,
            monitor: Some(MonitorType::Composite),
            vdg: None,
        });

        dialog.draft.variant = MachineVariant::Coco2;
        constrain(&mut dialog.draft);
        assert_eq!(dialog.draft.memory, MemorySize::K64);
        assert_eq!(dialog.draft.video, VideoStandard::NTSC);
        assert_eq!(
            dialog.draft.vdg,
            Some(VDGVariant::MC6847T1),
            "CoCo 2 defaults to the T1 (CoCo 2B)"
        );
        assert_eq!(
            dialog.draft.monitor, None,
            "a CoCo 2 has no monitor port to configure"
        );
        assert!(dialog.draft.validate().is_ok());

        dialog.draft.variant = MachineVariant::Coco3;
        constrain(&mut dialog.draft);
        assert_eq!(dialog.draft.memory, MemorySize::K512);
        assert_eq!(dialog.draft.vdg, None);
        assert_eq!(
            dialog.draft.monitor,
            Some(MonitorType::RGB),
            "returning to CoCo 3 re-seeds the default cable"
        );
        assert!(dialog.draft.validate().is_ok());

        dialog.draft.variant = MachineVariant::Coco1;
        constrain(&mut dialog.draft);
        assert_eq!(dialog.draft.vdg, Some(VDGVariant::MC6847));
        assert_eq!(dialog.draft.monitor, None);
        assert!(dialog.draft.validate().is_ok());
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
        dialog.open_with(current);
        assert!(dialog.open);
        assert!(dialog.error.is_none());
        assert_eq!(dialog.draft.variant, MachineVariant::Coco1);
        assert_eq!(dialog.draft.memory, MemorySize::K16);
    }
}
