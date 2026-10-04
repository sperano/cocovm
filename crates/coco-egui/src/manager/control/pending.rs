//! Resolving deferred requests: checking each [`PendingControl`]'s condition
//! against its (re-resolved, by slug) target entry every frame, replying
//! once it's met, gone, paused, or timed out.

use std::time::Instant;

use crate::control::{Reply, Response};
use eframe::egui;

use super::{ManagerApp, PendingCondition, PendingControl};

/// One [`PendingControl`]'s outcome this frame.
enum Outcome {
    Done,
    /// The target entry vanished because it stopped or its window closed.
    /// Rename transactions retarget pending requests before re-sorting.
    Gone(String),
    /// The target VM stopped advancing before the condition was met, so
    /// its fields would never come: paused by `set_running` or the debugger,
    /// or `suspended` from the manager.
    Paused {
        suspended: bool,
    },
    Waiting,
    /// Still waiting, but a `type_text` burst has drained to this many taps
    /// since the last check.
    Progressed(usize),
}

/// The error for a request whose VM stopped advancing mid-wait, naming the
/// tool that resumes it: a Suspended entry needs `start_vm` (`set_running`
/// refuses it), a paused one `set_running`. Any typed text or held keys stay
/// queued and finish once the VM resumes.
fn paused_message(pending: &PendingControl, suspended: bool) -> String {
    let (state, resume_with) = if suspended {
        ("suspended", "start_vm")
    } else {
        ("paused", "set_running")
    };
    format!(
        "VM '{}' was {state} while waiting for {}; call {resume_with} to resume",
        pending.slug,
        pending.condition.describe()
    )
}

impl ManagerApp {
    /// Reply to and drop every [`PendingControl`] whose condition is met,
    /// whose target vanished or paused, or whose deadline passed; keep the
    /// rest.
    /// Called once per `update()`, after [`Self::draw_running_vms`] has run
    /// this frame's fields. Running VM viewports wake the manager as fields
    /// advance; the earliest deadline supplies a bounded wake for stalled VMs.
    pub(in crate::manager) fn resolve_control_pending(&mut self, ctx: &egui::Context) {
        self.resolve_control_pending_at(ctx, Instant::now());
    }

    fn resolve_control_pending_at(&mut self, ctx: &egui::Context, now: Instant) {
        if self.pending.is_empty() {
            return;
        }
        let mut still_pending = Vec::new();
        // `mem::take` rather than `self.pending.drain(..)`: the loop body's
        // `self.check_pending` needs to borrow `self` immutably, which a
        // live draining borrow of `self.pending` would conflict with.
        for mut pending in std::mem::take(&mut self.pending) {
            if pending.is_abandoned() {
                continue;
            }
            match self.check_pending(&pending) {
                Outcome::Done => pending.reply.reply(Response::Ok(Reply::Done)),
                Outcome::Gone(msg) => pending.reply.reply(Response::Err(msg)),
                Outcome::Paused { suspended } => {
                    let msg = paused_message(&pending, suspended);
                    pending.reply.reply(Response::Err(msg));
                }
                Outcome::Waiting if now >= pending.deadline => {
                    let msg = format!("timed out waiting for {}", pending.condition.describe());
                    pending.reply.reply(Response::Err(msg));
                }
                Outcome::Waiting => still_pending.push(pending),
                Outcome::Progressed(remaining) => {
                    pending.note_progress(remaining, now);
                    still_pending.push(pending);
                }
            }
        }
        self.pending = still_pending;
        if let Some(deadline) = self.pending.iter().map(|pending| pending.deadline).min() {
            crate::app::scheduling::request_repaint_at(ctx, deadline);
        }
    }

    /// Re-resolve `pending`'s target by slug and check its condition against
    /// the live `CocoApp`.
    fn check_pending(&self, pending: &PendingControl) -> Outcome {
        let Some(idx) = self.entries.iter().position(|e| e.slug == pending.slug) else {
            return Outcome::Gone(format!("VM '{}' no longer exists", pending.slug));
        };
        let entry = &self.entries[idx];
        let Some(vm) = entry.vm.as_ref() else {
            return Outcome::Gone(format!("VM '{}' is no longer running", pending.slug));
        };
        let done = match pending.condition {
            PendingCondition::TypeTextDrained => !vm.remote_type_ahead.is_active(),
            PendingCondition::KeysReleased => vm.remote_held.is_none(),
            PendingCondition::WaitUntilField(target) => vm.fields_run >= target,
        };
        if done {
            return Outcome::Done;
        }
        if !vm.running {
            return Outcome::Paused {
                suspended: entry.suspended,
            };
        }
        let remaining = vm.remote_type_ahead.queue.len();
        let typing = matches!(pending.condition, PendingCondition::TypeTextDrained);
        if typing && remaining < pending.remaining_taps {
            Outcome::Progressed(remaining)
        } else {
            Outcome::Waiting
        }
    }
}

#[cfg(test)]
#[path = "pending_test.rs"]
mod tests;
