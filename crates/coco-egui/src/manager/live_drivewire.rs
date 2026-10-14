//! The DriveWire tab. Its edits reach a Running machine as they are saved.
//! A Suspended machine is left alone: Resume restores its saved session.

use coco_core::drivewire;
use coco_core::drivewire::NamedObjectInfo;
use coco_core::drivewire::PrintDiagnostics;
use coco_core::drivewire::share::ShareStatus;
use eframe::egui;

use super::{DETAIL_SECTION_GAP, MachineEntry, ManagerApp};
use crate::{CocoApp, machine_def, new_vm, titled_group};

/// The switches sit bare — the tab's own label already says "DriveWire".
pub(super) fn draw_drivewire_tab(
    ui: &mut egui::Ui,
    form: &mut new_vm::MachineForm,
    slug: &str,
    entry: &MachineEntry,
) {
    let session = entry.share_status();
    let objects = entry.named_objects();
    let guest_drives = entry.guest_drive_notes();
    let printer = entry.print_diagnostics();
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
        form.drivewire_share_rows(ui, slug, session.as_ref(), &objects);
    });
    ui.add_space(DETAIL_SECTION_GAP);
    titled_group(ui, "Printer", |ui| {
        form.drivewire_printer_rows(ui, slug, printer.as_ref());
    });
}

impl MachineEntry {
    /// The running VM's share session, for the DriveWire tab.
    pub(super) fn share_status(&self) -> Option<ShareStatus> {
        let dw = self.vm.as_ref()?.machine.bus.drivewire.as_ref()?;
        Some(dw.share_session().status())
    }

    /// The running VM's print diagnostics, for the DriveWire tab's Printer
    /// status lines. `None` without a running server, like
    /// [`Self::share_status`].
    pub(super) fn print_diagnostics(&self) -> Option<PrintDiagnostics> {
        let dw = self.vm.as_ref()?.machine.bus.drivewire.as_ref()?;
        Some(dw.print_diagnostics())
    }

    /// The running VM's named objects, for the DriveWire tab.
    pub(super) fn named_objects(&self) -> Vec<NamedObjectInfo> {
        self.vm
            .as_ref()
            .and_then(|vm| vm.machine.bus.drivewire.as_ref())
            .map(|dw| dw.named_objects())
            .unwrap_or_default()
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
    /// Push `entries[index]`'s saved `[drivewire]`, shares and printer included, into its
    /// Running VM.
    /// Failures surface in `launch_error`; success clears it.
    pub(super) fn apply_live_drivewire(&mut self, index: usize) {
        let entry = &mut self.entries[index];
        if entry.suspended {
            return;
        }
        let Some(vm) = entry.vm.as_mut() else {
            return;
        };
        let port = drivewire::PrintOutput::PrinterPort;
        let was_port = vm.machine.bus.drivewire.is_some() && vm.dw_print_output == port;
        let settings = crate::launch::drivewire_settings(&entry.def, &entry.slug);
        let applied = vm.apply_drivewire_settings(settings);
        let output = entry.def.drivewire.printer.output(&entry.slug);
        let becomes_port = entry.def.drivewire.enabled && output == port && !was_port;
        let attached = becomes_port
            .then(|| attach_print_port_sink(vm, &entry.def, &entry.slug))
            .flatten();
        vm.set_drivewire_printer(output);
        entry.launch_error = match entry.def.drivewire.share_table(&entry.slug) {
            Ok(shares) => {
                vm.set_drivewire_shares(shares);
                applied.err().or(attached)
            }
            Err(error) => Some(error),
        };
    }
}

/// `[ports].serial` attaches its sink only at launch. Called when the
/// printer port becomes the active print output of a running VM (the
/// output changed to it, or DriveWire was turned on): if its bit-banger
/// feeds nothing, this attaches that sink now, so the jobs don't print into
/// nothing. Only on that change, so a later save never restarts a capture
/// the user stopped from the Printer menu (which would truncate its file).
/// Returns the attach failure; an earlier [`CocoApp::cart_error`] stays.
fn attach_print_port_sink(
    vm: &mut CocoApp,
    def: &machine_def::MachineDef,
    slug: &str,
) -> Option<String> {
    if vm.has_printer_sink() {
        return None;
    }
    let prior = vm.cart_error.take();
    crate::launch::mount_serial(vm, def.ports.serial, slug);
    std::mem::replace(&mut vm.cart_error, prior)
}

#[cfg(test)]
#[path = "live_drivewire_test.rs"]
mod tests;
