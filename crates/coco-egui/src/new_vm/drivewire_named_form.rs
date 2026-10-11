//! The DriveWire tab's named-object rows: the share that serves DriveWire
//! named objects, and the objects a running VM has mounted.

use coco_core::drivewire::NamedObjectInfo;
use coco_core::drivewire::share::validate_share_name;
use eframe::egui;

use crate::machine_def::DriveWireDTO;

const SHARE_COMBO_WIDTH: f32 = 120.0;
const NO_SHARE_LABEL: &str = "None";
const ROW_LABEL: &str = "Named objects from";
const COMBO_LABEL: &str = "Named objects share";
const NAMED_HINT: &str = "Named objects let guest loaders open a file by name in this share \
    and read or write it by sector. Without a share, every request fails.";

/// The share picker, then a warning when the selection names no share.
pub(super) fn named_object_row(ui: &mut egui::Ui, drivewire: &mut DriveWireDTO) {
    let DriveWireDTO {
        shares,
        named_objects,
        ..
    } = drivewire;
    ui.horizontal(|ui| {
        ui.label(ROW_LABEL);
        let selected = named_objects
            .as_deref()
            .unwrap_or(NO_SHARE_LABEL)
            .to_string();
        let combo = egui::ComboBox::from_id_salt("named_objects")
            .width(SHARE_COMBO_WIDTH)
            .selected_text(selected)
            .show_ui(ui, |ui| {
                ui.selectable_value(named_objects, None, NO_SHARE_LABEL);
                for share in shares.iter() {
                    if validate_share_name(&share.name).is_ok() {
                        ui.selectable_value(
                            named_objects,
                            Some(share.name.clone()),
                            share.name.as_str(),
                        );
                    }
                }
            });
        combo.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, ui.is_enabled(), COMBO_LABEL)
        });
    });
    let defined = |name: &str| {
        shares
            .iter()
            .any(|share| share.name.eq_ignore_ascii_case(name))
    };
    if let Some(name) = named_objects.as_deref()
        && !defined(name)
    {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!("No share is named \"{name}\"; pick another one."),
        );
    }
    ui.small(NAMED_HINT);
}

/// Keeps the named-object selection on a share across a rename (`new` is
/// its name now) or a removal (`new` is `None`).
pub(super) fn follow_share(named_objects: &mut Option<String>, old: &str, new: Option<&str>) {
    if !named_objects
        .as_deref()
        .is_some_and(|name| name.eq_ignore_ascii_case(old))
    {
        return;
    }
    *named_objects = new.map(str::to_string);
}

/// One line per object a running VM has mounted.
pub(super) fn object_lines(ui: &mut egui::Ui, objects: &[NamedObjectInfo]) {
    for object in objects {
        ui.label(object_line(object));
    }
}

fn object_line(object: &NamedObjectInfo) -> String {
    let mut notes = Vec::new();
    if object.write_protected {
        notes.push("read-only");
    }
    if object.leased {
        notes.push("leased");
    }
    if !object.attached {
        notes.push("not reopened");
    }
    let notes = if notes.is_empty() {
        String::new()
    } else {
        format!(" ({})", notes.join(", "))
    };
    format!(
        "Named object DW{}: {}{notes}",
        object.drive, object.guest_path
    )
}

#[cfg(test)]
#[path = "drivewire_named_form_test.rs"]
mod tests;
