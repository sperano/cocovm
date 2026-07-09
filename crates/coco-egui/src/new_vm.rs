//! "Machine → New…" dialog: pick a machine model and its parameters, then
//! cold-start a fresh VM from the resulting [`MachineConfig`]. The dialog
//! only *builds* the config — swapping the running machine (and writing back
//! dirty media first) is `CocoApp::create_vm`'s job, so this module stays a
//! pure view over a draft config.

use coco_core::{MachineConfig, MachineVariant, MemorySize, MonitorType, VideoStandard};
use eframe::egui;

/// RAM sizes selectable per machine family — the same sets
/// [`MachineConfig::validate`] accepts (plain-SAM sizes for CoCo 1/2, GIME
/// MMU sizes for CoCo 3), so every config this dialog can produce validates.
const COCO12_RAM_CHOICES: &[MemorySize] = &[
    MemorySize::K4,
    MemorySize::K16,
    MemorySize::K32,
    MemorySize::K64,
];
const COCO3_RAM_CHOICES: &[MemorySize] = &[MemorySize::K128, MemorySize::K512, MemorySize::K2048];

/// Inner padding of the dialog body, matching the power-cycle confirmation
/// dialog in `main.rs`.
const DIALOG_MARGIN: i8 = 16;

const fn ram_choices(variant: MachineVariant) -> &'static [MemorySize] {
    match variant {
        MachineVariant::Coco1 | MachineVariant::Coco2 => COCO12_RAM_CHOICES,
        MachineVariant::Coco3 => COCO3_RAM_CHOICES,
    }
}

const fn ram_label(memory: MemorySize) -> &'static str {
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

/// What the user clicked this frame, from [`NewVmDialog::show`].
#[must_use]
pub enum NewVmAction {
    None,
    /// "Create" was clicked; the caller should try to build this machine and
    /// either [`NewVmDialog::close`] the dialog or record the failure in
    /// [`NewVmDialog::error`].
    Create(MachineConfig),
}

/// State of the "New…" dialog: a draft [`MachineConfig`] being edited, plus
/// the error from the last failed create attempt (e.g. a missing ROM set),
/// shown inline until the dialog closes or the next attempt.
pub struct NewVmDialog {
    open: bool,
    draft: MachineConfig,
    pub error: Option<String>,
}

impl NewVmDialog {
    pub fn new() -> Self {
        Self {
            open: false,
            draft: MachineConfig::default(),
            error: None,
        }
    }

    /// Open the dialog with the draft seeded from `current` (the running
    /// machine's config), so "New…" defaults to "same machine again".
    pub fn open_with(&mut self, current: MachineConfig) {
        self.draft = current;
        self.error = None;
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.error = None;
    }

    /// Re-constrain the draft after a model change: snap RAM to the new
    /// family's default when the current pick isn't valid for it, and force
    /// NTSC where PAL isn't modeled ([`MachineConfig::validate`]'s rules).
    fn constrain_draft(&mut self) {
        if !ram_choices(self.draft.variant).contains(&self.draft.memory) {
            self.draft.memory = match self.draft.variant {
                MachineVariant::Coco1 | MachineVariant::Coco2 => MemorySize::K64,
                MachineVariant::Coco3 => MemorySize::K512,
            };
        }
        if self.draft.variant != MachineVariant::Coco3 {
            self.draft.video = VideoStandard::Ntsc;
        }
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
                        .spacing([24.0, 10.0])
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new("Model").size(font));
                            ui.horizontal(|ui| {
                                for (variant, label) in [
                                    (MachineVariant::Coco1, "CoCo 1"),
                                    (MachineVariant::Coco2, "CoCo 2"),
                                    (MachineVariant::Coco3, "CoCo 3"),
                                ] {
                                    if ui
                                        .radio_value(&mut self.draft.variant, variant, label)
                                        .changed()
                                    {
                                        self.constrain_draft();
                                    }
                                }
                            });
                            ui.end_row();

                            ui.label(egui::RichText::new("RAM").size(font));
                            egui::ComboBox::from_id_salt("new_vm_ram")
                                .selected_text(ram_label(self.draft.memory))
                                .show_ui(ui, |ui| {
                                    for &memory in ram_choices(self.draft.variant) {
                                        ui.selectable_value(
                                            &mut self.draft.memory,
                                            memory,
                                            ram_label(memory),
                                        );
                                    }
                                });
                            ui.end_row();

                            ui.label(egui::RichText::new("Video").size(font));
                            // CoCo 1/2 PAL timing isn't modeled
                            // (`MachineConfig::validate`); constrain_draft
                            // already snapped the draft back to NTSC.
                            let pal_possible = self.draft.variant == MachineVariant::Coco3;
                            ui.horizontal(|ui| {
                                ui.radio_value(&mut self.draft.video, VideoStandard::Ntsc, "NTSC");
                                ui.add_enabled_ui(pal_possible, |ui| {
                                    ui.radio_value(
                                        &mut self.draft.video,
                                        VideoStandard::Pal,
                                        "PAL",
                                    )
                                    .on_disabled_hover_text(
                                        "PAL is only supported on the CoCo 3",
                                    );
                                });
                            });
                            ui.end_row();

                            ui.label(egui::RichText::new("Monitor").size(font));
                            ui.horizontal(|ui| {
                                ui.radio_value(&mut self.draft.monitor, MonitorType::Rgb, "RGB");
                                ui.radio_value(
                                    &mut self.draft.monitor,
                                    MonitorType::Composite,
                                    "Composite",
                                );
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
                            action = NewVmAction::Create(self.draft);
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
                &[VideoStandard::Ntsc, VideoStandard::Pal]
            } else {
                &[VideoStandard::Ntsc]
            };
            for &memory in ram_choices(variant) {
                for &video in videos {
                    for monitor in [MonitorType::Rgb, MonitorType::Composite] {
                        let config = MachineConfig {
                            variant,
                            video,
                            memory,
                            monitor,
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

    /// Switching model away from CoCo 3 must snap GIME-only RAM and PAL back
    /// to plain-SAM-valid values (and vice versa for RAM).
    #[test]
    fn constrain_draft_snaps_family_specific_fields() {
        let mut dialog = NewVmDialog::new();
        dialog.open_with(MachineConfig {
            variant: MachineVariant::Coco3,
            video: VideoStandard::Pal,
            memory: MemorySize::K2048,
            monitor: MonitorType::Rgb,
        });

        dialog.draft.variant = MachineVariant::Coco2;
        dialog.constrain_draft();
        assert_eq!(dialog.draft.memory, MemorySize::K64);
        assert_eq!(dialog.draft.video, VideoStandard::Ntsc);
        assert!(dialog.draft.validate().is_ok());

        dialog.draft.variant = MachineVariant::Coco3;
        dialog.constrain_draft();
        assert_eq!(dialog.draft.memory, MemorySize::K512);
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
            video: VideoStandard::Ntsc,
            memory: MemorySize::K16,
            monitor: MonitorType::Composite,
        };
        dialog.open_with(current);
        assert!(dialog.open);
        assert!(dialog.error.is_none());
        assert_eq!(dialog.draft.variant, MachineVariant::Coco1);
        assert_eq!(dialog.draft.memory, MemorySize::K16);
    }
}
