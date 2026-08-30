//! Applying one [`coco_control::Action`] to the manager: immediate actions
//! reply inline, others start work on the target `CocoApp` and defer their
//! reply via [`super::PendingControl`].

use std::path::PathBuf;

use coco_control::{Action, Incoming, Reply, ReplyHandle, Response};

use crate::{CocoApp, UI_DRIVES};

use super::{ManagerApp, PendingCondition, PendingControl};

fn check_drive(drive: usize) -> Result<(), String> {
    if drive >= UI_DRIVES {
        return Err(format!("drive must be 0..{UI_DRIVES}"));
    }
    Ok(())
}

/// Run a media action that reports failure through `CocoApp::cart_error`
/// and turn only the error *this* call raised into `Err`; a stale error
/// from an earlier UI action is set aside first, then put back.
fn with_cart_error(app: &mut CocoApp, action: impl FnOnce(&mut CocoApp)) -> Result<Reply, String> {
    let prior = app.cart_error.take();
    action(app);
    match app.cart_error.take() {
        Some(e) => Err(e),
        None => {
            app.cart_error = prior;
            Ok(Reply::Done)
        }
    }
}

/// `Ok`/`Err` into a [`Response`].
fn response(result: Result<Reply, String>) -> Response {
    match result {
        Ok(reply) => Response::Ok(reply),
        Err(e) => Response::Err(e),
    }
}

impl ManagerApp {
    /// Route one request to its handler. Every arm either replies through
    /// `incoming` before returning, or moves it into `self.pending`.
    pub(super) fn dispatch_control(&mut self, incoming: Incoming) {
        let (coco_control::protocol::Request { vm, action }, reply) = incoming.into_parts();
        match action {
            Action::ListVms => reply.reply(Response::Ok(Reply::Vms(self.vm_infos()))),
            Action::StartVm => reply.reply(response(self.start_vm_action(&vm))),
            Action::ScreenText => {
                let result = self
                    .resolve_alive(&vm)
                    .map(|idx| self.vm_mut(idx).screen_text());
                reply.reply(response(result));
            }
            Action::Screenshot => {
                let result = self
                    .resolve_alive(&vm)
                    .and_then(|idx| self.vm_ref(idx).screenshot());
                reply.reply(response(result));
            }
            Action::TypeText { text } => {
                self.start_deferred(reply, vm, |app| {
                    let fields = app.start_remote_typing(&text)?;
                    Ok((PendingCondition::TypeTextDrained, fields))
                });
            }
            Action::PressKeys { keys, hold_fields } => {
                self.start_deferred(reply, vm, move |app| {
                    let fields = app.start_remote_hold(&keys, hold_fields)?;
                    Ok((PendingCondition::KeysReleased, fields))
                });
            }
            Action::Joystick {
                stick,
                x,
                y,
                button1,
                button2,
                release,
            } => {
                let result = self.resolve_vm(&vm, true).map(|idx| {
                    self.vm_mut(idx)
                        .apply_remote_joystick(stick, x, y, button1, button2, release);
                    Reply::Done
                });
                reply.reply(response(result));
            }
            Action::InsertDisk { drive, path } => {
                reply.reply(response(self.insert_disk_action(&vm, drive, path)));
            }
            Action::EjectDisk { drive } => {
                reply.reply(response(self.eject_disk_action(&vm, drive)));
            }
            Action::Reset { hard } => {
                let result = self.resolve_vm(&vm, true).map(|idx| {
                    self.vm_mut(idx).remote_reset(hard);
                    Reply::Done
                });
                reply.reply(response(result));
            }
            Action::SetRunning { running } => {
                let result = self.resolve_vm(&vm, true).map(|idx| {
                    self.vm_mut(idx).set_running(running);
                    Reply::Done
                });
                reply.reply(response(result));
            }
            Action::Wait { fields } => self.start_wait(reply, vm, fields),
            Action::Peek { addr, len } => {
                let result = self
                    .resolve_alive(&vm)
                    .map(|idx| Reply::Bytes(self.vm_ref(idx).peek_bytes(addr, len)));
                reply.reply(response(result));
            }
            Action::Poke { addr, bytes } => {
                let result = self.resolve_vm(&vm, true).and_then(|idx| {
                    self.vm_mut(idx).poke_bytes(addr, &bytes)?;
                    Ok(Reply::Done)
                });
                reply.reply(response(result));
            }
        }
    }

    /// `entries[idx]`'s VM. Panics if absent — every call site resolves
    /// through [`Self::resolve_vm`]/[`Self::resolve_alive`] first, which
    /// guarantee it.
    fn vm_mut(&mut self, idx: usize) -> &mut CocoApp {
        self.entries[idx]
            .vm
            .as_mut()
            .expect("caller resolved this entry's VM as present")
    }

    fn vm_ref(&self, idx: usize) -> &CocoApp {
        self.entries[idx]
            .vm
            .as_ref()
            .expect("caller resolved this entry's VM as present")
    }

    /// `start_vm`: resolve without requiring Running (a Suspended or Powered
    /// Off target is exactly what this brings up), no-op if already Running,
    /// otherwise resume or launch and report the outcome.
    fn start_vm_action(&mut self, vm: &Option<String>) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, false)?;
        if self.entries[idx].is_running() {
            return Ok(Reply::Done);
        }
        if self.entries[idx].suspended {
            self.resume_vm(idx);
        } else {
            self.start_vm(idx);
        }
        match self.entries[idx].launch_error.take() {
            Some(e) => Err(e),
            None => Ok(Reply::Done),
        }
    }

    /// `insert_disk`: [`UI_DRIVES`], not `coco_core::fdc::DRIVE_COUNT` — the
    /// UI's own smaller exposed drive count, which is what `CocoApp::disk_paths`
    /// is actually sized to (see its field doc).
    fn insert_disk_action(
        &mut self,
        vm: &Option<String>,
        drive: usize,
        path: String,
    ) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, true)?;
        check_drive(drive)?;
        let app = self.vm_mut(idx);
        with_cart_error(app, |app| app.insert_disk(drive, PathBuf::from(path)))
    }

    /// `eject_disk`: [`Self::insert_disk_action`]'s twin.
    fn eject_disk_action(&mut self, vm: &Option<String>, drive: usize) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, true)?;
        check_drive(drive)?;
        let app = self.vm_mut(idx);
        with_cart_error(app, |app| app.eject_disk(drive))
    }

    /// `wait`: defer until `CocoApp::fields_run` reaches its current value
    /// plus `fields` (clamped to [`coco_control::MAX_WAIT_FIELDS`]).
    fn start_wait(&mut self, reply: ReplyHandle, vm: Option<String>, fields: u32) {
        let clamped = u64::from(fields.min(coco_control::MAX_WAIT_FIELDS));
        self.start_deferred(reply, vm, |app| {
            Ok((
                PendingCondition::WaitUntilField(app.fields_run + clamped),
                clamped,
            ))
        });
    }

    /// Shared shape of the deferred actions: resolve a Running target, start
    /// the work on it (`start` yields the completion condition and the fields
    /// it should take), then defer the reply — or send the error back
    /// immediately.
    fn start_deferred(
        &mut self,
        reply: ReplyHandle,
        vm: Option<String>,
        start: impl FnOnce(&mut CocoApp) -> Result<(PendingCondition, u64), String>,
    ) {
        let idx = match self.resolve_vm(&vm, true) {
            Ok(idx) => idx,
            Err(e) => return reply.reply(Response::Err(e)),
        };
        let slug = self.entries[idx].slug.clone();
        let app = self.vm_mut(idx);
        let field_rate_hz = app.machine.config.video.field_rate_hz();
        match start(app) {
            Ok((condition, expected_fields)) => self.pending.push(PendingControl::new(
                reply,
                slug,
                condition,
                expected_fields,
                field_rate_hz,
            )),
            Err(e) => reply.reply(Response::Err(e)),
        }
    }
}

#[cfg(test)]
#[path = "dispatch_test.rs"]
mod tests;
