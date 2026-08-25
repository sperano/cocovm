//! The hardware-config rows, split by the detail pane's sections:
//! [`machine_rows`] (Model) and [`display_rows`] (VDG, Video, Monitor) —
//! see the parent module doc. RAM is deliberately absent: the detail pane
//! draws it as its own titled radio-button group (`manager::detail`), not
//! a grid row.

use coco_core::{MachineConfig, MachineVariant, VDGVariant, VideoStandard};
use eframe::egui;

use super::{constrain, vdg_label, video_label};

/// The Model row (which CoCo this machine is), label + combo box — the detail pane hosts
/// it inside its "Machine" titled group. Must be called inside an already-open two-column [`egui::Grid`]; `salt` distinguishes the combo's persistent id across call sites.
pub(super) fn machine_rows(ui: &mut egui::Ui, salt: &str, draft: &mut MachineConfig) {
    let font = ui.style().text_styles[&egui::TextStyle::Button].size;

    ui.label(egui::RichText::new("Model").size(font));
    egui::ComboBox::from_id_salt((salt, "machine"))
        .selected_text(crate::machine_label(draft.variant))
        .show_ui(ui, |ui| {
            for variant in MachineVariant::ALL {
                if ui
                    .selectable_value(&mut draft.variant, variant, crate::machine_label(variant))
                    .changed()
                {
                    constrain(draft);
                }
            }
        });
    ui.end_row();
}

/// The conditional VDG row (CoCo 2 only) and the Video row — [`machine_rows`]'s sibling,
/// hosted by the detail pane's "Display" titled group. The monitor/TV row itself lives in [`super::MachineForm::display_rows`].
pub(super) fn display_rows(ui: &mut egui::Ui, salt: &str, draft: &mut MachineConfig) {
    let font = ui.style().text_styles[&egui::TextStyle::Button].size;

    // The VDG choice only exists on the CoCo 2 — CoCo 1 always shipped plain MC6847, CoCo 3's GIME does its own character generation.
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

    // Two-value choices are radio pairs, not combos — same as RAM and Keyboard.
    ui.label(egui::RichText::new("Video").size(font));
    ui.horizontal(|ui| {
        ui.radio_value(
            &mut draft.video,
            VideoStandard::NTSC,
            video_label(VideoStandard::NTSC),
        );
        // CoCo 1/2 PAL timing isn't modeled; `constrain` already snapped the draft back to NTSC.
        let pal_possible = draft.variant == MachineVariant::Coco3;
        ui.add_enabled_ui(pal_possible, |ui| {
            ui.radio_value(
                &mut draft.video,
                VideoStandard::PAL,
                video_label(VideoStandard::PAL),
            )
            .on_disabled_hover_text("PAL is only supported on the CoCo 3");
        });
    });
    ui.end_row();
}
