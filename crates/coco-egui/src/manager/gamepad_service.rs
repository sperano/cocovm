//! Manager-level service for the process gamepad event queue.

use std::time::Duration;

use eframe::egui;

use super::ManagerApp;

/// Maximum idle time between drains of gilrs' host-event channel. VM updates
/// can poll sooner, but the manager keeps servicing the process backend when
/// every VM is stopped or its window is closed.
const GAMEPAD_SERVICE_INTERVAL: Duration = Duration::from_millis(100);

impl ManagerApp {
    pub(super) fn service_gamepad(&self, ctx: &egui::Context) {
        if self.gamepad.service() {
            ctx.request_repaint_after(GAMEPAD_SERVICE_INTERVAL);
        }
    }
}

#[cfg(test)]
#[path = "gamepad_service_test.rs"]
mod tests;
