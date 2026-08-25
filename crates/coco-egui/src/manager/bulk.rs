//! The bulk detail pane: shown in [`super::ManagerApp`]'s central panel in
//! place of a single machine's edit form whenever more than one row is
//! selected (`manager.rs`'s `update`) — just a summary line and a pointer to
//! the toolbar. There is no bulk
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

    /// The single dispatch point for a bulk transport action.
    pub(super) fn apply_bulk(&mut self, action: BulkAction, indices: &[usize]) {
        match action {
            BulkAction::Play => self.bulk_play(indices),
            BulkAction::Suspend => self.bulk_suspend(indices),
            BulkAction::Stop => self.bulk_stop(indices),
            BulkAction::Reset => self.bulk_reset(indices),
        }
    }

    /// Resume each suspended row, start each powered-off one, skip anything
    /// already running. Only acted-on rows are checked for a fresh
    /// [`super::MachineEntry::launch_error`], so a skipped row's stale error
    /// can't steal focus from a fully successful bulk Play.
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

    /// Stop every row that isn't already Powered Off. `stop_vm` can fail (a
    /// flush error, or an undischarged suspend checkpoint), so acted-on rows
    /// are checked with [`Self::focus_first_failed_row`].
    fn bulk_stop(&mut self, indices: &[usize]) {
        let mut acted = Vec::new();
        for &i in indices {
            if self.entries[i].is_alive() {
                self.stop_vm(i);
                acted.push(i);
            }
        }
        self.focus_first_failed_row(&acted);
    }

    /// Reset every running row, never a suspended one: resetting a frozen
    /// machine's live object without touching its `.ccstate` would desync
    /// the two. `Machine::reset` cannot fail, so no error surfacing is needed.
    fn bulk_reset(&mut self, indices: &[usize]) {
        for &i in indices {
            if self.entries[i].is_running()
                && let Some(vm) = self.entries[i].vm.as_mut()
            {
                vm.machine.reset();
            }
        }
    }

    /// If any acted-on row recorded a [`super::MachineEntry::launch_error`],
    /// collapse the selection to just that row so the detail pane shows why.
    /// Surfaces only the first failure — the documented tradeoff for a batch
    /// action with no dialog of its own.
    pub(super) fn focus_first_failed_row(&mut self, acted: &[usize]) {
        if let Some(&i) = acted
            .iter()
            .find(|&&i| self.entries[i].launch_error.is_some())
        {
            self.selection.set_single(i);
            self.save_error = None;
        }
    }
}
