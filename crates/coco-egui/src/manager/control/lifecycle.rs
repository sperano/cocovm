//! The lifecycle actions `start_vm`, `stop_vm`, and `suspend_vm`: the
//! manager's own Start/Resume, Stop, and Suspend (`manager::lifecycle`),
//! reporting the error each leaves in the entry's `launch_error`.

use crate::control::Reply;

use super::{ManagerApp, control_status};

impl ManagerApp {
    /// `start_vm`: resolve without requiring Running (a Suspended or Powered
    /// Off target is exactly what this brings up), no-op if already Running,
    /// otherwise resume or launch and report the outcome.
    pub(super) fn start_vm_action(&mut self, vm: &Option<String>) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, false)?;
        self.start_entry(idx)?;
        Ok(Reply::Done)
    }

    /// `stop_vm`: the manager's Stop ([`ManagerApp::stop_vm`]) on a Running
    /// or Suspended target, a no-op on a Powered Off one. Stop drops the VM
    /// even when writing dirty media back fails, so that failure comes back
    /// as an error naming the state the entry ended in.
    pub(super) fn stop_vm_action(&mut self, vm: &Option<String>) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, false)?;
        if !self.entries[idx].is_alive() {
            return Ok(Reply::Done);
        }
        self.stop_vm(idx);
        self.lifecycle_outcome(idx)
    }

    /// `suspend_vm`: the manager's Suspend ([`ManagerApp::suspend_vm`]) on a
    /// Running target, a no-op on a Suspended one. A failed media write-back
    /// or state save aborts it, leaving the VM Running.
    pub(super) fn suspend_vm_action(&mut self, vm: &Option<String>) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, false)?;
        let entry = &self.entries[idx];
        if entry.suspended {
            return Ok(Reply::Done);
        }
        if entry.vm.is_none() {
            return Err(format!(
                "VM '{}' is powered off; call start_vm first",
                entry.slug
            ));
        }
        self.suspend_vm(idx);
        self.lifecycle_outcome(idx)
    }

    /// The error a lifecycle action left in `launch_error`, with the status
    /// the entry ended in. Cloned rather than taken, so the manager row
    /// still shows a failed media write-back to whoever is at the screen.
    fn lifecycle_outcome(&self, idx: usize) -> Result<Reply, String> {
        let entry = &self.entries[idx];
        match &entry.launch_error {
            Some(error) => Err(format!(
                "{error}\nVM '{}' is now {}.",
                entry.slug,
                control_status(entry).as_str()
            )),
            None => Ok(Reply::Done),
        }
    }
}

#[cfg(test)]
#[path = "lifecycle_test.rs"]
mod tests;
