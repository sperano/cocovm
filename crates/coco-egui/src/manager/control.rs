//! The built-in MCP server's request queue (`crate::control`): binding,
//! draining queued requests into per-VM actions (`control::dispatch`), and
//! resolving deferred ("pending") requests once the VM has finished the work
//! (`control::pending`).
//!
//! A request names its target VM by manager slug, or omits it to mean "the
//! only running VM" (`crate::control::protocol`'s own doc comment) —
//! [`ManagerApp::resolve_vm`] is the one place that convention is applied.

use std::time::{Duration, Instant};

use eframe::egui;

use super::{MachineEntry, ManagerApp};

mod dispatch;
mod pending;

/// Slack a deferred request (`type_text`, `press_keys`, `wait`) gets beyond
/// the wall-clock time its fields should take, before it times out.
const CONTROL_DEFER_MARGIN: Duration = Duration::from_secs(10);

/// A request whose reply is deferred until the VM finishes some work.
/// Targets the VM by slug, not entry index — an index can shift under a
/// pending request (a rename, another entry inserted/removed), a slug can't.
pub(super) struct PendingControl {
    reply: crate::control::ReplyHandle,
    slug: String,
    condition: PendingCondition,
    deadline: Instant,
}

impl PendingControl {
    pub(super) fn retarget(&mut self, old_slug: &str, new_slug: &str) {
        if self.slug == old_slug {
            self.slug = new_slug.to_string();
        }
    }

    /// `expected_fields` at `field_rate_hz` sets the deadline, plus
    /// [`CONTROL_DEFER_MARGIN`].
    fn new(
        reply: crate::control::ReplyHandle,
        slug: String,
        condition: PendingCondition,
        expected_fields: u64,
        field_rate_hz: f64,
    ) -> Self {
        let expected = Duration::from_secs_f64(expected_fields as f64 / field_rate_hz);
        Self {
            reply,
            slug,
            condition,
            deadline: Instant::now() + expected + CONTROL_DEFER_MARGIN,
        }
    }
}

/// What a [`PendingControl`] is waiting for, checked against its target
/// entry's live `CocoApp` each frame (`control::pending::check_pending`).
enum PendingCondition {
    /// `type_text`: `CocoApp::remote_type_ahead` has drained.
    TypeTextDrained,
    /// `press_keys`: `CocoApp::remote_held` has released and cleared.
    KeysReleased,
    /// `wait`: `CocoApp::fields_run` has reached the target count.
    WaitUntilField(u64),
}

impl PendingCondition {
    /// Named for the timeout error message.
    fn describe(&self) -> &'static str {
        match self {
            PendingCondition::TypeTextDrained => "typed text to drain",
            PendingCondition::KeysReleased => "held keys to release",
            PendingCondition::WaitUntilField(_) => "the requested fields to elapse",
        }
    }
}

/// The wire protocol's status for `entry`, distinct from
/// [`super::vm_status_label`]'s human-readable UI labels.
fn control_status(entry: &MachineEntry) -> crate::control::VmStatus {
    if entry.suspended {
        crate::control::VmStatus::Suspended
    } else if entry.vm.is_some() {
        crate::control::VmStatus::Running
    } else {
        crate::control::VmStatus::PoweredOff
    }
}

/// Bind the control listener on `port` at startup, waking `ctx` whenever a
/// request lands. `port == 0` disables the listener outright; a bind failure
/// is logged and also disables it — neither is fatal to the app.
pub(super) fn bind_control(
    port: u16,
    ctx: &egui::Context,
) -> Option<crate::control::ControlServer> {
    try_bind_control(port, ctx).unwrap_or_else(|e| {
        tracing::warn!("{e}");
        None
    })
}

/// [`bind_control`]'s fallible core: `Ok(None)` for `port == 0`, `Err` naming
/// the port a bind failed on.
fn try_bind_control(
    port: u16,
    ctx: &egui::Context,
) -> Result<Option<crate::control::ControlServer>, String> {
    if port == 0 {
        return Ok(None);
    }
    let ctx = ctx.clone();
    let wake: crate::control::Wake = std::sync::Arc::new(move || ctx.request_repaint());
    let server = crate::control::ControlServer::bind(port, wake)
        .map_err(|e| format!("control: could not bind 127.0.0.1:{port}: {e}"))?;
    tracing::info!("control: listening on 127.0.0.1:{}", server.port());
    Ok(Some(server))
}

impl ManagerApp {
    /// The live listener's port, `0` when there is none.
    pub(super) fn control_port(&self) -> u16 {
        self.control.as_ref().map_or(0, |server| server.port())
    }

    /// Move the listener to `port` for a Settings save: a no-op when already
    /// there, otherwise drops the old listener (its open connections finish
    /// on their own) before binding the new one, so `Err` leaves no listener.
    pub(super) fn rebind_control(&mut self, port: u16, ctx: &egui::Context) -> Result<(), String> {
        if port == self.control_port() {
            return Ok(());
        }
        self.control = None;
        self.control = try_bind_control(port, ctx)?;
        Ok(())
    }

    /// Drain every request queued since last frame, dispatching each one —
    /// immediate actions reply right away; others join [`Self::pending`].
    /// Called once per `update()`, before [`Self::draw_running_vms`].
    pub(super) fn drain_control(&mut self) {
        while let Some(incoming) = self.control.as_ref().and_then(|s| s.try_recv()) {
            self.dispatch_control(incoming);
        }
    }

    /// Every machine the manager knows, with its wire-protocol status.
    fn vm_infos(&self) -> Vec<crate::control::VmInfo> {
        self.entries
            .iter()
            .map(|e| crate::control::VmInfo {
                slug: e.slug.clone(),
                name: e.def.name.clone(),
                status: control_status(e),
            })
            .collect()
    }

    /// Resolve a request's `vm` slug to an entry index. `None` selects the
    /// sole Running entry (`crate::control::protocol`'s convention), erroring
    /// out by name when there are zero or several. `require_running`
    /// additionally rejects a *named* entry that isn't Running — every
    /// mutating action but `start_vm` sets this (`start_vm` must be able to
    /// resolve a Suspended or Powered Off target, since bringing one up is
    /// the whole point).
    fn resolve_vm(&self, vm: &Option<String>, require_running: bool) -> Result<usize, String> {
        match vm {
            Some(slug) => {
                let idx = self
                    .entries
                    .iter()
                    .position(|e| &e.slug == slug)
                    .ok_or_else(|| format!("no VM named '{slug}'"))?;
                if require_running && !self.entries[idx].is_running() {
                    return Err(format!("VM '{slug}' is not running; call start_vm first"));
                }
                Ok(idx)
            }
            None => self.resolve_sole_running_vm(),
        }
    }

    /// [`Self::resolve_vm`]'s no-slug case: the one Running entry, or a
    /// message naming every candidate when there are zero or several.
    fn resolve_sole_running_vm(&self) -> Result<usize, String> {
        let running: Vec<usize> = (0..self.entries.len())
            .filter(|&i| self.entries[i].is_running())
            .collect();
        match running.as_slice() {
            [idx] => Ok(*idx),
            [] => Err("no VM is running; specify vm".to_string()),
            _ => {
                let slugs: Vec<&str> = running
                    .iter()
                    .map(|&i| self.entries[i].slug.as_str())
                    .collect();
                Err(format!(
                    "multiple VMs are running ({}); specify vm",
                    slugs.join(", ")
                ))
            }
        }
    }

    /// [`Self::resolve_vm`] for a read-only action: a *named* entry must
    /// merely be alive, not necessarily Running — a suspended-but-open
    /// window's frozen state can be inspected. The no-slug case still
    /// requires the sole Running entry.
    fn resolve_alive(&self, vm: &Option<String>) -> Result<usize, String> {
        let idx = self.resolve_vm(vm, false)?;
        if self.entries[idx].vm.is_none() {
            return Err(format!(
                "VM '{}' is not running; call start_vm first",
                self.entries[idx].slug
            ));
        }
        Ok(idx)
    }
}

#[cfg(test)]
#[path = "control_test.rs"]
mod tests;
