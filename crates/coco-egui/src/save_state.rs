//! Frontend save-state UX: the quick states (`quick.rs`) with their toolbar
//! controls and rebindable chords (`hotkeys.rs`), state files anywhere on
//! disk (`file.rs`), the machine-type prompt in front of user-requested
//! loads (`load_request.rs`), and the status-bar toast — all built on top
//! of the engine in [`coco_core::snapshot`], which this module is the only
//! caller of.
//!
//! [`CocoApp::save_state_to`]/[`CocoApp::load_state_from`] are the
//! unprompted entry points (suspend and resume); [`CocoApp::request_load`]
//! is the prompted one. Everything else here is either UI chrome around
//! them or the fiddly frontend-side re-injection
//! [`coco_core::snapshot::restore`] can't do itself (host-only resources,
//! path mirrors, pacing — see [`CocoApp::apply_restored_machine`]).

use crate::CocoApp;

#[cfg(test)]
mod fd502_test;
mod file;
mod load_request;
mod media_ref;
#[cfg(test)]
mod media_ref_test;
mod quick;
#[cfg(test)]
mod quick_test;
mod restore;
mod save;

pub(crate) use load_request::PendingLoad;
#[cfg(test)]
pub(crate) use load_request::{CANCEL, LOAD_ANYWAY, LoadSource};
pub(crate) use quick::{
    QUICK_SLOTS, StateFile, default_quick_state_dir, saved_time, slot_shortcuts_hint, state_name,
};
#[cfg(test)]
pub(crate) use save::DRIVEWIRE_HOST_BUSY;

/// How long a status-bar toast stays visible after [`CocoApp::set_toast`].
pub(crate) const TOAST_SECS: f64 = 4.0;

/// Disabled-hover text of a Load control whose state has no file yet.
pub(crate) fn empty_state_hover(slot: usize) -> String {
    format!("{} is empty. Save a state first.", state_name(slot))
}

/// `head`, then the restore `notes` (if any) after a colon — the shape of
/// every load toast.
fn with_notes(head: &str, notes: &[String]) -> String {
    if notes.is_empty() {
        head.to_string()
    } else {
        format!("{head}: {}", notes.join("; "))
    }
}

impl CocoApp {
    /// Show `msg` in the status bar for [`TOAST_SECS`] seconds.
    fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), std::time::Instant::now()));
    }

    /// The current toast text, if one is still within [`TOAST_SECS`] of
    /// [`Self::set_toast`] — clears itself once expired.
    pub(crate) fn toast_message(&mut self) -> Option<String> {
        let (msg, at) = self.toast.as_ref()?;
        if at.elapsed().as_secs_f64() > TOAST_SECS {
            self.toast = None;
            return None;
        }
        Some(msg.clone())
    }
}

#[cfg(test)]
#[path = "save_state_test.rs"]
pub(crate) mod tests;
