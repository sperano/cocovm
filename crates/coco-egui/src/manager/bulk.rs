//! The bulk detail pane: shown in [`super::ManagerApp`]'s central panel in
//! place of a single machine's edit form whenever more than one row is
//! selected (`manager.rs`'s `update`) — just a summary line and a pointer to
//! the toolbar, which is where the transport buttons that used to live here
//! now sit (user decision 2026-07-29, `toolbar.rs`'s doc). There is no bulk
//! edit form: `EditState` only ever describes one machine
//! (`manager.rs`'s doc on the `edit` field), so a multi-selection can only
//! run transport actions, not edit definitions.
//!
//! [`BulkAction`] and [`ManagerApp::apply_bulk`] are the single dispatch
//! point shared by the toolbar (`super::toolbar::draw_toolbar`) and the bulk
//! context menu (`super::list::draw_bulk_row_context_menu`), so the three
//! UIs can't drift on what each action does or is gated by.

use eframe::egui;

use super::{DETAIL_SECTION_GAP, ManagerApp};

/// [`ManagerApp::draw_bulk_detail`]'s pointer to the toolbar.
const BULK_HINT: &str =
    "Use the toolbar's Start, Suspend, Stop, and Reset buttons to act on this selection.";

/// One of the four transport actions a bulk selection can run.
#[derive(Clone, Copy)]
pub(super) enum BulkAction {
    Play,
    Suspend,
    Stop,
    Reset,
}

/// The three aggregate eligibility flags every bulk UI (the toolbar's
/// transport tiles, the bulk context menu, and the delete-confirmation
/// modal's own "is anything running" check) gates its controls on —
/// computed once by [`ManagerApp::bulk_flags`] so the three surfaces can't
/// disagree about what "any row is eligible" means.
pub(super) struct BulkFlags {
    pub(super) any_startable: bool,
    pub(super) any_running: bool,
    pub(super) any_alive: bool,
}

impl ManagerApp {
    /// The bulk pane: a "N machines selected" heading and a pointer to the
    /// toolbar, which is where the actual transport controls now live.
    pub(super) fn draw_bulk_detail(&self, ui: &mut egui::Ui) {
        ui.heading(format!("{} machines selected", self.selection.len()));
        ui.add_space(DETAIL_SECTION_GAP);
        ui.small(BULK_HINT);
    }

    /// `indices`' three [`BulkFlags`] — "is at least one selected row
    /// eligible" for Play, Suspend/Reset, and Stop respectively.
    pub(super) fn bulk_flags(&self, indices: &[usize]) -> BulkFlags {
        BulkFlags {
            any_startable: indices.iter().any(|&i| self.entries[i].is_startable()),
            any_running: indices.iter().any(|&i| self.entries[i].is_running()),
            any_alive: indices.iter().any(|&i| self.entries[i].is_alive()),
        }
    }

    /// The single dispatch point for a bulk transport action — called from
    /// the toolbar (`super::toolbar::draw_toolbar`) and the bulk context
    /// menu (`super::list::draw_bulk_row_context_menu`).
    pub(super) fn apply_bulk(&mut self, action: BulkAction, indices: &[usize]) {
        match action {
            BulkAction::Play => self.bulk_play(indices),
            BulkAction::Suspend => self.bulk_suspend(indices),
            BulkAction::Stop => self.bulk_stop(indices),
            BulkAction::Reset => self.bulk_reset(indices),
        }
    }

    /// Resume each suspended row, start each powered-off one, skip anything
    /// already running — the same per-row rule the single-row context
    /// menu's Start/Resume item follows. Only the rows actually acted on
    /// are checked for a fresh [`super::MachineEntry::launch_error`]
    /// afterward — a *skipped* row's stale error from some earlier attempt
    /// must not steal focus from a fully successful bulk Play.
    fn bulk_play(&mut self, indices: &[usize]) {
        let mut acted = Vec::new();
        for &i in indices {
            let suspended = self.entries[i].suspended;
            if self.entries[i].is_running() {
                continue;
            }
            if suspended {
                self.resume_vm(i);
            } else {
                self.start_vm(i);
            }
            acted.push(i);
        }
        self.focus_first_failed_row(&acted);
    }

    /// Suspend every currently-running row.
    fn bulk_suspend(&mut self, indices: &[usize]) {
        let mut acted = Vec::new();
        for &i in indices {
            if self.entries[i].is_running() {
                self.suspend_vm(i);
                acted.push(i);
            }
        }
        self.focus_first_failed_row(&acted);
    }

    /// Stop every row that isn't already Powered Off. `stop_vm` has no
    /// failure path, so unlike Play/Suspend this never needs
    /// [`Self::focus_first_failed_row`].
    fn bulk_stop(&mut self, indices: &[usize]) {
        for &i in indices {
            if self.entries[i].is_alive() {
                self.stop_vm(i);
            }
        }
    }

    /// Reset every running row, never a suspended one: resetting a frozen
    /// machine's live object without touching its `.ccstate` would silently
    /// desync the two, exactly the divergence `resume_vm`'s own contract
    /// goes out of its way to avoid. `Machine::reset` cannot fail, so —
    /// like [`Self::bulk_stop`] — no error surfacing is needed.
    fn bulk_reset(&mut self, indices: &[usize]) {
        for &i in indices {
            if self.entries[i].is_running()
                && let Some(vm) = self.entries[i].vm.as_mut()
            {
                vm.machine.reset();
            }
        }
    }

    /// After a bulk Play/Suspend: if any row it actually *acted on* (not
    /// every selected row — a skipped one's stale error would otherwise
    /// misattribute the failure) recorded a
    /// [`super::MachineEntry::launch_error`], collapse the selection down
    /// to just that row so the detail pane — the error's only rendering
    /// surface — shows why. Mirrors `list::select_row_on_error`; a bulk
    /// action only ever surfaces the *first* failure this way, the
    /// documented tradeoff for a batch operation with no dialog of its own.
    fn focus_first_failed_row(&mut self, acted: &[usize]) {
        if let Some(&i) = acted
            .iter()
            .find(|&&i| self.entries[i].launch_error.is_some())
        {
            self.selection.set_single(i);
            self.save_error = None;
        }
    }
}
