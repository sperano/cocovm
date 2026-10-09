//! The VM toolbar's quick-state group, after Reset: a State selector, then
//! Save and Load tiles acting on the selected state (`save_state/quick.rs`).
//! The selector's list ends with Save to File… and Load from File…
//! (`save_state/file.rs`). When the row is too narrow it collapses to one
//! States tile opening a menu with the same controls, and when even that
//! does not fit it is left out (the state chords, `hotkeys.rs`, still
//! reach every state). The fit is measured from
//! the available width and the tile dimensions, never a fixed breakpoint,
//! and the toolbar never wraps: its height is part of the window-sizing math
//! (`crate::TOOLBAR_H`).

use crate::hotkeys::{Hotkey, HotkeyAction};
use crate::save_state::{QUICK_SLOTS, StateFile, empty_state_hover, saved_time, state_name};
use crate::*;

/// Save tile glyph — U+1F4E5 inbox tray (into the store); Load's is its
/// opposite, U+1F4E4 outbox tray. Both are in egui's bundled
/// NotoEmoji-Regular, like the Debug tile's beetle.
pub(super) const SAVE_GLYPH: &str = "📥";
pub(super) const LOAD_GLYPH: &str = "📤";
/// Collapsed group's glyph — U+1F4BE floppy disk, also in NotoEmoji-Regular.
pub(super) const STATES_GLYPH: &str = "💾";
const SAVE_LABEL: &str = "Save";
const LOAD_LABEL: &str = "Load";
/// Caption and accessible name of the collapsed group's tile.
pub(crate) const STATES_LABEL: &str = "States";
/// Hover text of [`STATES_LABEL`]'s tile.
const STATES_HOVER: &str = "Save or load a quick state";
/// Footnote under the state list: the states belong to the app, not to this
/// machine.
pub(crate) const SHARED_NOTE: &str = "Shared across VM windows";
/// Selector item that saves the machine state to a file the user picks.
pub(crate) const SAVE_TO_FILE: &str = "Save to File…";
/// Selector item that loads a state file the user picks.
pub(crate) const LOAD_FROM_FILE: &str = "Load from File…";
const SAVE_TO_FILE_HOVER: &str = "Save the machine state to a file of your choice.";
const LOAD_FROM_FILE_HOVER: &str =
    "Load a state from a file of your choice. Replaces the current machine state.";
/// Disabled-hover text of Save and Load while suspended: writing or
/// replacing the live machine would diverge it from its frozen `.ccstate`.
pub(crate) const SUSPENDED_HOVER: &str = "Resume the machine to save or load a state.";

/// How much of the group the toolbar row has room for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GroupFit {
    /// Selector plus Save and Load tiles.
    Full,
    /// One States tile with a menu.
    Collapsed,
    /// Nothing: only the state chords remain.
    Hidden,
}

/// Pick the widest layout of the group that fits `available` points while
/// leaving `reserve` for the tiles drawn after it.
pub(crate) fn group_fit(available: f32, full: f32, collapsed: f32, reserve: f32) -> GroupFit {
    let room = available - reserve;
    if room >= full {
        GroupFit::Full
    } else if room >= collapsed {
        GroupFit::Collapsed
    } else {
        GroupFit::Hidden
    }
}

/// The two quick actions the group offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QuickAction {
    Save,
    Load,
}

impl QuickAction {
    /// Accessible name of this action's control for `slot`: "Save to State
    /// 1", "Load State 1".
    pub(crate) fn name(self, slot: usize) -> String {
        match self {
            Self::Save => format!("Save to {}", state_name(slot)),
            Self::Load => format!("Load {}", state_name(slot)),
        }
    }

    /// The rebindable action behind this control for `slot` (`hotkeys.rs`).
    pub(crate) fn hotkey_action(self, slot: usize) -> HotkeyAction {
        match self {
            Self::Save => HotkeyAction::SaveState(slot),
            Self::Load => HotkeyAction::LoadState(slot),
        }
    }

    /// Enabled-hover text of this action's control for `slot`, whose file is
    /// `file` and whose chord is `hotkey`: the name, what it replaces, and
    /// the chord.
    fn hover(self, ctx: &egui::Context, slot: usize, file: StateFile, hotkey: Hotkey) -> String {
        let name = self.name(slot);
        let detail = match (self, file) {
            (Self::Save, StateFile::Empty) => String::new(),
            (Self::Save, StateFile::Saved(_)) => " Replaces the saved state.".to_string(),
            (Self::Load, StateFile::Saved(Some(t))) => format!(
                " Saved {}. Replaces the current machine state.",
                saved_time(t, chrono::Local::now())
            ),
            (Self::Load, _) => " Replaces the current machine state.".to_string(),
        };
        let shortcut = ctx.format_shortcut(&hotkey.shortcut());
        if detail.is_empty() {
            format!("{name} ({shortcut})")
        } else {
            format!("{name}.{detail} ({shortcut})")
        }
    }
}

impl CocoApp {
    /// The quick-state group, laid out to fit what's left of the row after
    /// keeping `reserve` points for the tiles that follow it.
    pub(super) fn quick_state_group(&mut self, ui: &mut egui::Ui, icons_only: bool, reserve: f32) {
        let separator = toolbar_separator_width(ui);
        let tile = toolbar_button_width(ui, icons_only);
        let selector = selector_width(ui);
        let full = separator + selector + ui.spacing().item_spacing.x + 2.0 * tile;
        let collapsed = separator + tile;
        match group_fit(ui.available_width(), full, collapsed, reserve) {
            GroupFit::Full => {
                toolbar_separator(ui);
                self.quick_state_selector(ui, selector);
                self.quick_state_tiles(ui, icons_only);
            }
            GroupFit::Collapsed => {
                toolbar_separator(ui);
                self.quick_states_menu_tile(ui, icons_only);
            }
            GroupFit::Hidden => {}
        }
    }

    /// The "State N ▾" combo: picking a row only changes the target.
    fn quick_state_selector(&mut self, ui: &mut egui::Ui, width: f32) {
        egui::ComboBox::from_id_salt("quick_state_selector")
            .selected_text(state_name(self.selected_quick_state))
            .width(width)
            // Tall enough for all five rows, the note, and the file items without a scroll bar.
            .height(ui.ctx().content_rect().height())
            .wrap_mode(egui::TextWrapMode::Extend)
            .show_ui(ui, |ui| self.quick_state_rows(ui));
    }

    /// Every state's row (selectable even when empty, so Save can target
    /// it), then [`SHARED_NOTE`], then the state-file items.
    fn quick_state_rows(&mut self, ui: &mut egui::Ui) {
        for slot in 0..QUICK_SLOTS {
            let label = self.quick_state_label(slot);
            ui.selectable_value(&mut self.selected_quick_state, slot, label);
        }
        ui.separator();
        ui.label(egui::RichText::new(SHARED_NOTE).weak());
        ui.separator();
        self.state_file_items(ui);
    }

    /// [`SAVE_TO_FILE`] and [`LOAD_FROM_FILE`], gated like the tiles. Each
    /// closes the popup before its file dialog opens.
    fn state_file_items(&mut self, ui: &mut egui::Ui) {
        let enabled = !self.suspended;
        let save = ui
            .add_enabled(enabled, egui::Button::new(SAVE_TO_FILE))
            .on_hover_text(SAVE_TO_FILE_HOVER)
            .on_disabled_hover_text(SUSPENDED_HOVER);
        if save.clicked() {
            ui.close();
            self.save_state_file_dialog();
        }
        let load = ui
            .add_enabled(enabled, egui::Button::new(LOAD_FROM_FILE))
            .on_hover_text(LOAD_FROM_FILE_HOVER)
            .on_disabled_hover_text(SUSPENDED_HOVER);
        if load.clicked() {
            ui.close();
            self.load_state_file_dialog(ui.ctx());
        }
    }

    /// The Save and Load tiles for the selected state.
    fn quick_state_tiles(&mut self, ui: &mut egui::Ui, icons_only: bool) {
        let tiles = [
            (QuickAction::Save, SAVE_GLYPH, SAVE_LABEL),
            (QuickAction::Load, LOAD_GLYPH, LOAD_LABEL),
        ];
        for (action, glyph, caption) in tiles {
            let slot = self.selected_quick_state;
            let file = self.quick_state_file(slot);
            let enabled = self.quick_action_enabled(action, file);
            let name = action.name(slot);
            let tile = toolbar_tile(ui, glyph, caption, &name, enabled, icons_only);
            self.quick_action_response(ui.ctx(), tile, action, slot, file);
        }
    }

    /// `action`'s control for the selected state as a plain button, for the
    /// collapsed group's menu.
    fn quick_action_button(&mut self, ui: &mut egui::Ui, action: QuickAction) {
        let slot = self.selected_quick_state;
        let file = self.quick_state_file(slot);
        let enabled = self.quick_action_enabled(action, file);
        let button = ui.add_enabled(enabled, egui::Button::new(action.name(slot)));
        self.quick_action_response(ui.ctx(), button, action, slot, file);
    }

    /// Attach `action`'s hover texts to its control's `response`, and run
    /// the action on `slot` when it was clicked.
    fn quick_action_response(
        &mut self,
        ctx: &egui::Context,
        response: egui::Response,
        action: QuickAction,
        slot: usize,
        file: StateFile,
    ) {
        let disabled = if self.suspended {
            SUSPENDED_HOVER.to_string()
        } else {
            empty_state_hover(slot)
        };
        let hotkey = self.hotkeys.get(action.hotkey_action(slot));
        let clicked = response
            .on_hover_text(action.hover(ctx, slot, file, hotkey))
            .on_disabled_hover_text(disabled)
            .clicked();
        if clicked {
            match action {
                QuickAction::Save => {
                    self.quick_save(slot);
                }
                QuickAction::Load => self.quick_load(slot, ctx),
            }
        }
    }

    /// Whether `action` is enabled for a selected state whose file is
    /// `file`: neither while suspended (a debugger pause is fine), and Load
    /// only while the state has a file.
    pub(crate) fn quick_action_enabled(&self, action: QuickAction, file: StateFile) -> bool {
        !self.suspended && (action == QuickAction::Save || !file.is_empty())
    }

    /// The collapsed group: a States tile whose menu holds Save and Load
    /// for the selected state, then the state rows.
    fn quick_states_menu_tile(&mut self, ui: &mut egui::Ui, icons_only: bool) {
        let response = toolbar_button(ui, STATES_GLYPH, STATES_LABEL, true, icons_only)
            .on_hover_text(STATES_HOVER);
        egui::Popup::menu(&response).show(|ui| {
            self.quick_action_button(ui, QuickAction::Save);
            self.quick_action_button(ui, QuickAction::Load);
            ui.separator();
            self.quick_state_rows(ui);
        });
    }
}

/// Outer width that fits the selector's widest selected text ("State 5")
/// next to its dropdown icon. Passed to `ComboBox::width`, which then sizes
/// the button to exactly this, so the fit test and the drawn width agree.
fn selector_width(ui: &egui::Ui) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let widest = (0..QUICK_SLOTS)
        .map(|slot| {
            ui.fonts_mut(|f| {
                f.layout_no_wrap(state_name(slot), font.clone(), egui::Color32::PLACEHOLDER)
                    .size()
                    .x
            })
        })
        .fold(0.0, f32::max);
    let spacing = ui.spacing();
    widest + spacing.icon_spacing + spacing.icon_width + 2.0 * spacing.button_padding.x
}

#[cfg(test)]
#[path = "quick_states_test.rs"]
mod tests;
