//! The hardware-config controls, split by the detail pane's sections:
//! [`machine_rows`] (model radios) and [`display_rows`] (VDG) — see the
//! parent module doc. RAM is deliberately absent: the detail pane draws it
//! as its own titled radio-button group (`manager::detail`).

use coco_core::{MachineConfig, MachineVariant, VDGVariant};
use eframe::egui;

use super::{constrain, vdg_label};

/// The model radios (which CoCo this machine is), hosted inside the detail
/// pane's "Machine" titled group.
pub(super) fn machine_rows(ui: &mut egui::Ui, draft: &mut MachineConfig) {
    ui.horizontal(|ui| {
        for variant in MachineVariant::ALL {
            if ui
                .radio_value(&mut draft.variant, variant, crate::machine_label(variant))
                .changed()
            {
                constrain(draft);
            }
        }
    });
}

/// The conditional VDG row (CoCo 2 only) — [`machine_rows`]'s sibling, hosted by the
/// detail pane's Display tab. The monitor/TV row itself lives in
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
