//! The manager window's toolbar: [`ManagerApp::draw_toolbar`], built from the
//! shared icon-over-label tile widget in [`crate::widgets`] (also used by the
//! VM window's own toolbar, `chrome::toolbar`). Layout is New – Start –
//! Suspend – Stop – Reset – separator – Settings – separator – Help.
//! The four transport tiles act on the current selection through the same
//! [`super::bulk::BulkAction`]/[`ManagerApp::apply_bulk`] dispatch the bulk
//! context menu uses — one code path, three surfaces.

use eframe::egui;

use super::ManagerApp;
use super::bulk::BulkAction;
use crate::new_vm;
use crate::widgets::{
    BUTTON_GAP, PLAY_GLYPH, RESET_GLYPH, RESET_LABEL, START_LABEL, STOP_GLYPH, STOP_LABEL,
    SUSPEND_GLYPH, SUSPEND_HOVER, SUSPEND_LABEL, toolbar_button, toolbar_separator,
};

/// Toolbar glyphs, all verified present in egui's bundled emoji fonts
/// (NotoEmoji / emoji-icon-font — both monochrome, so they tint with the
/// widget text color like any label).
const NEW_ICON: &str = "➕";
const SETTINGS_ICON: &str = "⚙";
const HELP_ICON: &str = "❓";

/// Disabled-hover text shared by every transport tile when nothing at all
/// is selected — distinct from the flags-driven reason below it, so the
/// tile teaches *why* it's off from either starting point (selection-neutral,
/// since the tiles now act on a selection of any size rather than one fixed row).
const SELECT_A_MACHINE_HOVER: &str = "Select a machine first";
/// Disabled-hover text shared by Suspend and Reset — both gated on
/// [`super::bulk::BulkFlags::any_running`]. Reset's running-only rule
/// excludes a suspended machine deliberately: resetting a frozen machine's
/// live object without touching its `.ccstate` would silently desync the
/// two (same rationale as every other Reset control in the manager).
const NONE_RUNNING_HOVER: &str = "None of the selected machines are running";

const START_HOVER: &str = "Start or resume the selected machines that aren't already running";
const START_DISABLED_HOVER: &str = "The selected machines are already running";
const STOP_HOVER: &str = "Shut down the selected machines that are running or suspended";
const STOP_DISABLED_HOVER: &str = "None of the selected machines are running or suspended";
const RESET_HOVER: &str = "Press the reset button on the selected running machines";

impl ManagerApp {
    /// The manager actions row. "Settings"/"Help" are still inert
    /// scaffolding; the four transport tiles dispatch through
    /// [`ManagerApp::apply_bulk`] against every currently selected row
    /// (zero, one, or many — the same [`super::bulk::BulkFlags`] gating the
    /// bulk pane and bulk context menu use), so a single selected machine
    /// behaves exactly as the old per-row buttons did.
    pub(super) fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        let indices: Vec<usize> = self.selection.iter().collect();
        let flags = self.bulk_flags(&indices);
        let empty_selection = indices.is_empty();
        let disabled_hover = |ineligible: &'static str| -> &'static str {
            if empty_selection {
                SELECT_A_MACHINE_HOVER
            } else {
                ineligible
            }
        };

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = BUTTON_GAP;
            // The shortcut shows on hover (inline shortcut text is a
            // menu-row convention, not a toolbar one).
            if toolbar_button(ui, NEW_ICON, "New", true)
                .on_hover_text(ui.ctx().format_shortcut(&new_vm::NEW_MACHINE_SHORTCUT))
                .clicked()
            {
                self.create_machine_now();
            }

            if toolbar_button(ui, PLAY_GLYPH, START_LABEL, flags.any_startable)
                .on_hover_text(START_HOVER)
                .on_disabled_hover_text(disabled_hover(START_DISABLED_HOVER))
                .clicked()
            {
                self.apply_bulk(BulkAction::Play, &indices);
            }
            if toolbar_button(ui, SUSPEND_GLYPH, SUSPEND_LABEL, flags.any_running)
                .on_hover_text(SUSPEND_HOVER)
                .on_disabled_hover_text(disabled_hover(NONE_RUNNING_HOVER))
                .clicked()
            {
                self.apply_bulk(BulkAction::Suspend, &indices);
            }
            if toolbar_button(ui, STOP_GLYPH, STOP_LABEL, flags.any_alive)
                .on_hover_text(STOP_HOVER)
                .on_disabled_hover_text(disabled_hover(STOP_DISABLED_HOVER))
                .clicked()
            {
                self.apply_bulk(BulkAction::Stop, &indices);
            }
            if toolbar_button(ui, RESET_GLYPH, RESET_LABEL, flags.any_running)
                .on_hover_text(RESET_HOVER)
                .on_disabled_hover_text(disabled_hover(NONE_RUNNING_HOVER))
                .clicked()
            {
                self.apply_bulk(BulkAction::Reset, &indices);
            }

            toolbar_separator(ui);
            let _ = toolbar_button(ui, SETTINGS_ICON, "Settings", true);
            toolbar_separator(ui);
            let _ = toolbar_button(ui, HELP_ICON, "Help", true);
        });
    }
}
