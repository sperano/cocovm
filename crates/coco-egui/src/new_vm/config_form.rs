//! [`config_form_rows`]: the shared hardware-config rows (Machine, VDG, RAM,
//! Video, Monitor) drawn by both hosts — see the parent module doc.

use coco_core::{MachineConfig, MachineVariant, MonitorType, VDGVariant, VideoStandard};
use eframe::egui;

use super::{constrain, monitor_label, ram_choices, ram_label, vdg_label, video_label};

/// Shared hardware-config rows, all label + combo box: Machine, conditional
/// VDG (CoCo 2 only — see the inline comment below), RAM, Video (PAL only
/// for CoCo 3), conditional Monitor (CoCo 3 only). Must be called inside an
/// already-open two-column [`egui::Grid`]; `salt` distinguishes the
/// [`egui::ComboBox`]'s persistent id when this is drawn from more than one
/// call site in the same frame (the "New…" dialog *and* the manager's
/// detail pane can both be visible at once).
pub(super) fn config_form_rows(ui: &mut egui::Ui, salt: &str, draft: &mut MachineConfig) {
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
