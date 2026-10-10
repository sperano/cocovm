//! Manager-side save-state tool actions.

use crate::control::{Reply, StateTarget};

use super::ManagerApp;

impl ManagerApp {
    pub(super) fn save_state_action(
        &mut self,
        vm: &Option<String>,
        target: StateTarget,
    ) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, true)?;
        let app = self.entries[idx]
            .vm
            .as_mut()
            .expect("running target has a VM");
        match target {
            StateTarget::Path(path) => app.save_state_to(&path)?,
            StateTarget::Slot(slot) => app.save_state_to_slot(slot)?,
        }
        Ok(Reply::Done)
    }

    pub(super) fn load_state_action(
        &mut self,
        vm: &Option<String>,
        target: StateTarget,
    ) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, true)?;
        let app = self.entries[idx]
            .vm
            .as_mut()
            .expect("running target has a VM");
        match target {
            StateTarget::Path(path) => app.load_state_from(&path)?,
            StateTarget::Slot(slot) => app.load_state_from_slot(slot)?,
        }
        Ok(Reply::Done)
    }
}

#[cfg(test)]
#[path = "state_test.rs"]
mod tests;
