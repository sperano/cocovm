//! The bulk detail pane: shown in [`super::ManagerApp`]'s central panel in
//! place of a single machine's edit form whenever more than one row is
//! selected (`manager.rs`'s `update`) — a summary line plus one transport
//! row that applies to every selected machine at once. There is no bulk
//! edit form: `EditState` only ever describes one machine
//! (`manager.rs`'s doc on the `edit` field), so a multi-selection can only
//! run transport actions, not edit definitions.
//!
//! [`BulkAction`] and [`ManagerApp::apply_bulk`] are the single dispatch
//! point shared with the bulk context menu (`super::list::
//! draw_bulk_row_context_menu`), so the two UIs can't drift on what each
//! action does or is gated by.

use eframe::egui;

use super::detail::transport_button;
use super::{
    DETAIL_SECTION_GAP, ManagerApp, PLAY_GLYPH, RESET_GLYPH, STOP_GLYPH, SUSPEND_GLYPH,
    SUSPEND_HOVER,
};

/// One of the four transport actions a bulk selection can run.
#[derive(Clone, Copy)]
pub(super) enum BulkAction {
    Play,
    Suspend,
    Stop,
    Reset,
}

/// The three aggregate eligibility flags every bulk UI (the pane's
/// transport row, the bulk context menu, and the delete-confirmation
/// modal's own "is anything running" check) gates its controls on —
/// computed once by [`ManagerApp::bulk_flags`] so the three surfaces can't
/// disagree about what "any row is eligible" means.
pub(super) struct BulkFlags {
    pub(super) any_startable: bool,
    pub(super) any_running: bool,
    pub(super) any_alive: bool,
}

impl ManagerApp {
    /// The bulk pane: a "N machines selected" heading over the shared
    /// transport row.
    pub(super) fn draw_bulk_detail(&mut self, ui: &mut egui::Ui) {
        ui.heading(format!("{} machines selected", self.selection.len()));
        ui.add_space(DETAIL_SECTION_GAP);
        self.draw_bulk_transport_row(ui);
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

    /// One transport row, [`transport_button`]-built like
    /// [`super::detail::draw_transport_row`]'s single-machine version, but
    /// each button now acts on every selected row via [`BulkAction`]/
    /// [`Self::apply_bulk`]. Reset is gated the same as every other Reset
    /// control in the manager (the detail pane's button, both context
    /// menus): *running* rows only, never a suspended one — resetting a
    /// frozen machine's live object without touching its `.ccstate` would
    /// silently desync the two, exactly the divergence Resume's own
    /// contract goes out of its way to avoid (`lifecycle::resume_vm`'s
    /// doc).
    fn draw_bulk_transport_row(&mut self, ui: &mut egui::Ui) {
        let indices: Vec<usize> = self.selection.iter().collect();
        let flags = self.bulk_flags(&indices);
        let mut picked = None;
        ui.horizontal(|ui| {
            if transport_button(ui, PLAY_GLYPH, "Play", flags.any_startable)
                .on_hover_text("Start or resume every selected machine that isn't already running")
                .clicked()
            {
                picked = Some(BulkAction::Play);
            }
            if transport_button(ui, SUSPEND_GLYPH, "Suspend", flags.any_running)
                .on_hover_text(SUSPEND_HOVER)
                .clicked()
            {
                picked = Some(BulkAction::Suspend);
            }
            if transport_button(ui, STOP_GLYPH, "Stop", flags.any_alive)
                .on_hover_text("Shut down every selected machine that is running or suspended")
                .clicked()
            {
                picked = Some(BulkAction::Stop);
            }
            if transport_button(ui, RESET_GLYPH, "Reset", flags.any_running)
                .on_hover_text("Press the reset button on every selected running machine")
                .clicked()
            {
                picked = Some(BulkAction::Reset);
            }
        });
        if let Some(action) = picked {
            self.apply_bulk(action, &indices);
        }
    }

    /// The single dispatch point for a bulk transport action — called from
    /// both the pane above and the bulk context menu
    /// (`super::list::draw_bulk_row_context_menu`).
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

    /// Reset every running row (see [`Self::draw_bulk_transport_row`]'s doc
    /// for why suspended rows are excluded). `Machine::reset` cannot fail,
    /// so — like [`Self::bulk_stop`] — no error surfacing is needed.
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
