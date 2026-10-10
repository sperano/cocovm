//! Applying one [`crate::control::Action`] to the manager: immediate actions
//! reply inline, others start work on the target `CocoApp` and defer their
//! reply via [`super::PendingControl`].

use std::path::PathBuf;

use crate::control::{
    Action, ControlError, Incoming, Reply, ReplyHandle, Response, ScreenSnapshot, TextMatcher,
};
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
        Err(e) => Response::Err(e.into()),
    }
}

impl ManagerApp {
    /// Route one request to its handler. Every arm either replies through
    /// `incoming` before returning, or moves it into `self.pending`.
    pub(super) fn dispatch_control(&mut self, incoming: Incoming) {
        let (crate::control::protocol::Request { vm, action }, reply) = incoming.into_parts();
        if reply.is_abandoned() {
            return;
        }
        match action {
            Action::ListVms => reply.reply(Response::Ok(Reply::Vms(self.vm_infos()))),
            Action::StartVm => reply.reply(response(self.start_vm_action(&vm))),
            Action::StopVm => reply.reply(response(self.stop_vm_action(&vm))),
            Action::SuspendVm => reply.reply(response(self.suspend_vm_action(&vm))),
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
                self.start_deferred(reply, vm, false, |app| {
                    let fields = app.start_remote_typing(&text)?;
                    Ok((PendingCondition::TypeTextDrained, fields))
                });
            }
            Action::PressKeys { keys, hold_fields } => {
                self.start_deferred(reply, vm, false, move |app| {
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
            Action::Wait {
                fields,
                fast_forward,
            } => self.start_wait(reply, vm, fields, fast_forward),
            Action::WaitForText {
                matcher,
                timeout_fields,
                fast_forward,
            } => self.start_wait_for_text(reply, vm, matcher, timeout_fields, fast_forward),
            Action::Peek { addr, len } => {
                let result = self
                    .resolve_alive(&vm)
                    .and_then(|idx| self.vm_ref(idx).peek_memory(addr, len))
                    .map(Reply::Bytes);
                reply.reply(response(result));
            }
            Action::Poke { addr, bytes } => {
                let result = self.resolve_vm(&vm, true).and_then(|idx| {
                    self.vm_mut(idx).poke_memory(addr, &bytes)?;
                    Ok(Reply::Done)
                });
                reply.reply(response(result));
            }
            Action::LoadBinary {
                segments,
                exec_address,
            } => {
                let result = self.resolve_vm(&vm, true).map(|idx| {
                    self.vm_mut(idx).load_binary(&segments, exec_address);
                    Reply::Done
                });
                reply.reply(response(result));
            }
            Action::SaveState { target } => {
                reply.reply(response(self.save_state_action(&vm, target)));
            }
            Action::LoadState { target } => {
                reply.reply(response(self.load_state_action(&vm, target)));
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
    /// plus `fields` (clamped to [`crate::control::MAX_WAIT_FIELDS`]). A
    /// paused VM is refused up front, as `type_text` and `press_keys` are:
    /// its field count would never move. With `fast_forward`, the VM runs
    /// those fields unthrottled (`CocoApp::start_fast_forward`); the run is
    /// cancelled when this request resolves.
    fn start_wait(
        &mut self,
        reply: ReplyHandle,
        vm: Option<String>,
        fields: u32,
        fast_forward: bool,
    ) {
        let clamped = u64::from(fields.min(crate::control::MAX_WAIT_FIELDS));
        self.start_deferred(reply, vm, fast_forward, |app| {
            if !app.running {
                return Err(crate::app::PAUSED_ERROR.to_string());
            }
            let target = app.fields_run + clamped;
            if fast_forward {
                app.start_fast_forward(target, None)?;
            }
            Ok((PendingCondition::WaitUntilField(target), clamped))
        });
    }

    /// `wait_for_text`: reply at once on a match, otherwise defer until the
    /// screen matches or `timeout_fields` (clamped) have passed. With
    /// `fast_forward`, the VM runs unthrottled until the match or the
    /// terminal field, whichever comes first.
    fn start_wait_for_text(
        &mut self,
        reply: ReplyHandle,
        vm: Option<String>,
        matcher: TextMatcher,
        timeout_fields: u32,
        fast_forward: bool,
    ) {
        if reply.is_abandoned() {
            return;
        }
        self.prune_abandoned_pending();
        if self.pending.len() >= super::MAX_PENDING_CONTROL_REQUESTS {
            return reply.reply(Response::Err(super::CONTROL_PENDING_OVERLOADED.into()));
        }
        let idx = match self.resolve_vm(&vm, true) {
            Ok(idx) => idx,
            Err(error) => return reply.reply(Response::Err(error.into())),
        };
        let slug = self.entries[idx].slug.clone();
        let app = self.vm_mut(idx);
        let timeout_fields = u64::from(timeout_fields.min(crate::control::MAX_WAIT_FIELDS));
        let snapshot = app.screen_snapshot();
        if matcher.is_match(&snapshot) {
            return reply.reply(Response::Ok(Reply::Screen(snapshot)));
        }
        if timeout_fields == 0 {
            return reply.reply(Response::Err(text_timeout(snapshot)));
        }
        let terminal_field = app.fields_run.saturating_add(timeout_fields);
        if fast_forward {
            // Refused while paused: a paused VM's screen can't change, so
            // the wait would only ever time out, as the generic check reports.
            if let Err(error) = app.start_fast_forward(terminal_field, Some(matcher.clone())) {
                return reply.reply(Response::Err(ControlError::with_screen(error, snapshot)));
            }
        }
        let condition = PendingCondition::WaitForText {
            matcher,
            terminal_field,
        };
        let field_rate_hz = app.machine.config.video.field_rate_hz();
        let pending = PendingControl::new(reply, slug, condition, timeout_fields, field_rate_hz);
        self.pending.push(if fast_forward {
            pending.with_fast_forward()
        } else {
            pending
        });
    }

    /// Shared shape of the deferred actions: resolve a Running target, start
    /// the work on it (`start` yields the completion condition and the fields
    /// it should take), then defer the reply — or send the error back
    /// immediately. `fast_forward` records that `start` began an
    /// unthrottled run this request owns.
    fn start_deferred(
        &mut self,
        reply: ReplyHandle,
        vm: Option<String>,
        fast_forward: bool,
        start: impl FnOnce(&mut CocoApp) -> Result<(PendingCondition, u64), String>,
    ) {
        if reply.is_abandoned() {
            return;
        }
        self.prune_abandoned_pending();
        if self.pending.len() >= super::MAX_PENDING_CONTROL_REQUESTS {
            return reply.reply(Response::Err(super::CONTROL_PENDING_OVERLOADED.into()));
        }
        let idx = match self.resolve_vm(&vm, true) {
            Ok(idx) => idx,
            Err(e) => return reply.reply(Response::Err(e.into())),
        };
        let slug = self.entries[idx].slug.clone();
        let app = self.vm_mut(idx);
        let field_rate_hz = app.machine.config.video.field_rate_hz();
        match start(app) {
            Ok((condition, expected_fields)) => {
                let pending =
                    PendingControl::new(reply, slug, condition, expected_fields, field_rate_hz);
                self.pending.push(if fast_forward {
                    pending.with_fast_forward()
                } else {
                    pending
                });
            }
            Err(e) => reply.reply(Response::Err(e.into())),
        }
    }
}

fn text_timeout(screen: ScreenSnapshot) -> ControlError {
    ControlError::with_screen("timed out waiting for screen text", screen)
}

#[cfg(test)]
#[path = "dispatch_test.rs"]
mod tests;
