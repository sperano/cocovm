//! The hardware-config rows, split by the detail pane's sections:
//! [`machine_rows`] (Model) and [`display_rows`] (VDG) —
//! see the parent module doc. RAM is deliberately absent: the detail pane
//! draws it as its own titled radio-button group (`manager::detail`), not
//! a grid row.

use coco_core::{MachineConfig, MachineVariant, VDGVariant};
use eframe::egui;

use super::{constrain, vdg_label};

/// The Model row (which CoCo this machine is), label + combo box — the detail pane hosts
/// it inside its "Machine" titled group. Must be called inside an already-open two-column
/// [`egui::Grid`]; `salt` distinguishes the combo's persistent id across call sites.
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

/// The conditional VDG row (CoCo 2 only) — [`machine_rows`]'s sibling, hosted by the
/// detail pane's "Display" titled group. The monitor/TV row itself lives in
/// [`super::MachineForm::display_rows`]. There is no Video (NTSC/PAL) row: PAL isn't
/// modeled well enough to offer, so the form leaves `config.video` alone.
pub(super) fn display_rows(ui: &mut egui::Ui, salt: &str, draft: &mut MachineConfig) {
    let font = ui.style().text_styles[&egui::TextStyle::Button].size;

    // The VDG choice only exists on the CoCo 2 — CoCo 1 always shipped plain MC6847, CoCo 3's
    // GIME does its own character generation.
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
}
