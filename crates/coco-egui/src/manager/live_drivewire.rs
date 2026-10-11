//! The DriveWire tab. Its edits reach a Running machine as they are saved.
//! A Suspended machine is left alone: Resume restores its saved session.

use coco_core::drivewire;
use coco_core::drivewire::share::ShareStatus;
use eframe::egui;

use super::{DETAIL_SECTION_GAP, MachineEntry, ManagerApp};
use crate::{new_vm, titled_group};

/// The switches sit bare — the tab's own label already says "DriveWire".
pub(super) fn draw_drivewire_tab(
    ui: &mut egui::Ui,
    form: &mut new_vm::MachineForm,
    slug: &str,
    entry: &MachineEntry,
) {
    let session = entry.share_status();
    let guest_drives = entry.guest_drive_notes();
    form.drivewire_rows(ui);
    ui.add_space(DETAIL_SECTION_GAP);
    titled_group(ui, "Disk images", |ui| {
        form.drivewire_disk_rows(ui);
        for note in &guest_drives {
            ui.small(note);
        }
    });
    ui.add_space(DETAIL_SECTION_GAP);
    titled_group(ui, "Host shares", |ui| {
        form.drivewire_share_rows(ui, slug, session.as_ref());
    });
}

impl MachineEntry {
    /// The running VM's share session, for the DriveWire tab.
    pub(super) fn share_status(&self) -> Option<ShareStatus> {
        let dw = self.vm.as_ref()?.machine.bus.drivewire.as_ref()?;
        Some(dw.share_session().status())
    }

    /// One note per drive whose running image differs from these settings
    /// because the guest ran `dw disk insert` or `dw disk eject`.
    pub(super) fn guest_drive_notes(&self) -> Vec<String> {
        let Some(vm) = self.vm.as_ref() else {
            return Vec::new();
        };
        let Some(dw) = vm.machine.bus.drivewire.as_ref() else {
            return Vec::new();
        };
        (0..drivewire::DRIVE_COUNT)
            .filter(|&drive| dw.guest_changed(drive))
            .map(|drive| match dw.drive_media(drive) {
                Some(media) => format!(
                    "DW{drive} now holds {}{}, inserted by the guest until the VM restarts.",
                    media.name.as_deref().unwrap_or("an image"),
                    if media.write_protected {
                        " (read-only)"
                    } else {
                        ""
                    },
                ),
                None => format!(
                    "DW{drive} stays empty until the VM restarts: the guest ejected it, \
                     or its guest-inserted image was missing on restore."
                ),
            })
            .collect()
    }
}

impl ManagerApp {
    /// Push `entries[index]`'s saved `[drivewire]`, shares included, into its Running VM.
    /// Failures surface in `launch_error`; success clears it.
    pub(super) fn apply_live_drivewire(&mut self, index: usize) {
        let entry = &mut self.entries[index];
        if entry.suspended {
            return;
        }
        let Some(vm) = entry.vm.as_mut() else {
            return;
        };
        let settings = crate::launch::drivewire_settings(&entry.def, &entry.slug);
        let applied = vm.apply_drivewire_settings(settings);
        entry.launch_error = match entry.def.drivewire.share_table(&entry.slug) {
            Ok(shares) => {
                vm.set_drivewire_shares(shares);
                applied.err()
            }
            Err(error) => Some(error),
        };
    }
}

#[cfg(test)]
#[path = "live_drivewire_test.rs"]
mod tests;
