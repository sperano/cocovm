//! DriveWire edits reach a Running machine as they are saved. A Suspended
//! machine is left alone: Resume restores its saved session.

use super::ManagerApp;

impl ManagerApp {
    /// Push `entries[index]`'s saved `[drivewire]` into its Running VM.
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
        entry.launch_error = vm.apply_drivewire_settings(settings).err();
    }
}

#[cfg(test)]
#[path = "live_drivewire_test.rs"]
mod tests;
