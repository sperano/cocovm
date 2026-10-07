//! The Settings dialog behind the toolbar's ⚙ tile (`manager/toolbar.rs`):
//! edits the global `config.toml` in place. Follows the delete
//! confirmation's `egui::Modal` pattern (`manager/delete.rs`); write-back
//! goes through [`config::save_file`], which preserves the file's comments
//! and any hand-added keys.

use std::num::NonZeroU32;
use std::path::Path;

use eframe::egui;

use crate::cli::LogLevel;
use crate::config::{self, FileConfig, ManagerSort};
use crate::hotkeys::{DEFAULT_HOTKEYS, Hotkey, Hotkeys};

use super::{ManagerApp, NO_CONFIG_DIR};

mod hotkeys;
mod layout;
use hotkeys::HotkeyEditor;

/// The dialog's edited draft, plus the error from the last failed load or
/// save (shown inline until the next attempt).
pub(crate) struct SettingsDialog {
    log_level: LogLevel,
    control_port: u16,
    /// `control_port` as opened; Save only moves the listener when the user
    /// changed it here, so an unrelated save never retries a failed bind.
    opened_control_port: u16,
    assets_url: String,
    toolbar_icons_only: bool,
    status_bar_icons_only: bool,
    welcome_image_cycle: bool,
    /// Plain `u32` so `DragValue` can edit it; the layout's nonzero range
    /// keeps it nonzero.
    welcome_image_cycle_secs: u32,
    welcome_image_shuffle: bool,
    hotkey_editor: HotkeyEditor,
    /// Preserved unchanged because the machine-list control owns this key.
    manager_sort: Option<ManagerSort>,
    error: Option<String>,
}

/// What [`SettingsDialog::draw`]'s button row asked for this frame.
enum SettingsAction {
    None,
    Save,
    Cancel,
}

impl SettingsDialog {
    /// Opens with `path`'s current file values, falling through to the
    /// built-in default for anything the file doesn't set. When `path`'s
    /// file can't be read or parsed, opens with the built-in defaults and
    /// shows the load error instead.
    fn open(path: Option<&Path>) -> Self {
        match config::load(path) {
            Ok(file) => Self::from_file(file, None),
            Err(e) => Self::from_file(FileConfig::default(), Some(e)),
        }
    }

    fn from_file(file: FileConfig, error: Option<String>) -> Self {
        let control_port = file.control_port.unwrap_or(crate::control::DEFAULT_PORT);
        let hotkeys = file.hotkeys();
        Self {
            log_level: file.log_level.unwrap_or(config::DEFAULT_LOG_LEVEL),
            control_port,
            opened_control_port: control_port,
            assets_url: file
                .assets_url
                .unwrap_or_else(|| crate::startup::DEFAULT_ASSETS_URL.to_string()),
            toolbar_icons_only: file
                .toolbar_icons_only
                .unwrap_or(config::DEFAULT_TOOLBAR_ICONS_ONLY),
            status_bar_icons_only: file
                .status_bar_icons_only
                .unwrap_or(config::DEFAULT_STATUS_BAR_ICONS_ONLY),
            welcome_image_cycle: file
                .welcome_image_cycle
                .unwrap_or(config::DEFAULT_WELCOME_IMAGE_CYCLE),
            welcome_image_cycle_secs: file
                .welcome_image_cycle_secs
                .unwrap_or(config::DEFAULT_WELCOME_IMAGE_CYCLE_SECS)
                .get(),
            welcome_image_shuffle: file
                .welcome_image_shuffle
                .unwrap_or(config::DEFAULT_WELCOME_IMAGE_SHUFFLE),
            hotkey_editor: HotkeyEditor::new(hotkeys),
            manager_sort: file.manager_sort,
            error,
        }
    }

    /// `Some(value)` only where `value` differs from the built-in default,
    /// so a field the user never touched keeps tracking future defaults
    /// instead of pinning today's value into the file.
    fn to_file_config(&self) -> FileConfig {
        let hotkeys = &self.hotkey_editor.hotkeys;
        let changed = |hotkey: Hotkey, default: Hotkey| (hotkey != default).then_some(hotkey);
        FileConfig {
            log_level: (self.log_level != config::DEFAULT_LOG_LEVEL).then_some(self.log_level),
            control_port: (self.control_port != crate::control::DEFAULT_PORT)
                .then_some(self.control_port),
            // An emptied field reads as "reset to default", not "set to ''".
            assets_url: (!self.assets_url.trim().is_empty()
                && self.assets_url != crate::startup::DEFAULT_ASSETS_URL)
                .then(|| self.assets_url.clone()),
            toolbar_icons_only: (self.toolbar_icons_only != config::DEFAULT_TOOLBAR_ICONS_ONLY)
                .then_some(self.toolbar_icons_only),
            status_bar_icons_only: (self.status_bar_icons_only
                != config::DEFAULT_STATUS_BAR_ICONS_ONLY)
                .then_some(self.status_bar_icons_only),
            welcome_image_cycle: (self.welcome_image_cycle != config::DEFAULT_WELCOME_IMAGE_CYCLE)
                .then_some(self.welcome_image_cycle),
            welcome_image_cycle_secs: NonZeroU32::new(self.welcome_image_cycle_secs)
                .filter(|secs| *secs != config::DEFAULT_WELCOME_IMAGE_CYCLE_SECS),
            welcome_image_shuffle: (self.welcome_image_shuffle
                != config::DEFAULT_WELCOME_IMAGE_SHUFFLE)
                .then_some(self.welcome_image_shuffle),
            hotkey_key_layout: changed(hotkeys.key_layout, DEFAULT_HOTKEYS.key_layout),
            hotkey_keyboard_mode: changed(hotkeys.keyboard_mode, DEFAULT_HOTKEYS.keyboard_mode),
            hotkey_new_machine: changed(hotkeys.new_machine, DEFAULT_HOTKEYS.new_machine),
            hotkey_debugger: changed(hotkeys.debugger, DEFAULT_HOTKEYS.debugger),
            manager_sort: self.manager_sort,
        }
    }
}

impl ManagerApp {
    /// Opens the Settings dialog, seeded from [`ManagerApp::config_path`]'s
    /// current file (toolbar's ⚙ tile).
    pub(super) fn open_settings_dialog(&mut self) {
        self.settings = Some(SettingsDialog::open(self.config_path.as_deref()));
    }

    /// The Settings modal, drawn once per `update()` while
    /// [`ManagerApp::settings`] is `Some`. Esc, Cancel, and a click outside
    /// all dismiss without saving.
    pub(super) fn draw_settings_dialog(&mut self, ctx: &egui::Context) {
        if self.settings.is_none() {
            return;
        }
        let mut action = SettingsAction::None;
        let modal = egui::Modal::new(egui::Id::new("settings_dialog")).show(ctx, |ui| {
            if let Some(dialog) = &mut self.settings {
                action = dialog.draw(ui);
            }
        });
        match action {
            SettingsAction::None => {}
            SettingsAction::Cancel => self.settings = None,
            SettingsAction::Save => self.commit_settings(ctx),
        }
        if modal.should_close() {
            self.settings = None;
        }
    }

    /// Saves the draft and closes the dialog; applies the icons-only toggles,
    /// the welcome-image cycle, the hotkeys, `log_level` and `control_port`
    /// live unless a CLI/env override wins. A failure shows in the dialog,
    /// which stays open with the old listener.
    fn commit_settings(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.settings else {
            return;
        };
        // Reset can bring back a default another action has since taken.
        if let Err(e) = dialog.hotkey_editor.hotkeys.check_distinct() {
            dialog.error = Some(e);
            return;
        }
        let file = dialog.to_file_config();
        let live = LiveSettings::from_dialog(dialog);
        let log_level = (!self.log_level_overridden).then_some(dialog.log_level);
        let port_change = (dialog.control_port != dialog.opened_control_port
            && !self.control_port_overridden)
            .then_some(dialog.control_port);
        let result = match self.config_path.as_deref() {
            Some(path) => config::save_file(path, &file),
            None => Err(NO_CONFIG_DIR.to_string()),
        }
        .and_then(|()| {
            self.apply_live_settings(&live);
            if let Some(level) = log_level {
                self.relevel_logging(level)?;
            }
            match port_change {
                Some(port) => self.rebind_control(port, ctx),
                None => Ok(()),
            }
        });
        match result {
            Ok(()) => self.settings = None,
            Err(e) => {
                if let Some(dialog) = &mut self.settings {
                    dialog.error = Some(e);
                }
            }
        }
    }

    /// Swap the live log filter to `level` through [`ManagerApp::log_reload`];
    /// a no-op without one (tests).
    fn relevel_logging(&self, level: LogLevel) -> Result<(), String> {
        let Some(handle) = &self.log_reload else {
            return Ok(());
        };
        handle
            .reload(crate::startup::log_filter(level.into()))
            .map_err(|e| format!("could not change the log level: {e}"))
    }

    /// The settings that take effect without a restart, skipping any the
    /// CLI/env overrode. Re-arms the welcome-image timer so a new interval
    /// counts from now rather than from the old deadline.
    fn apply_live_settings(&mut self, live: &LiveSettings) {
        if !self.toolbar_icons_only_overridden {
            self.toolbar_icons_only = live.toolbar_icons_only;
        }
        if !self.status_bar_icons_only_overridden {
            self.status_bar_icons_only = live.status_bar_icons_only;
        }
        let welcome = &mut self.welcome_image;
        if !welcome.cycle_overridden {
            welcome.cycle = live.welcome_image_cycle;
        }
        if !welcome.cycle_secs_overridden
            && let Some(secs) = live.welcome_image_cycle_secs
        {
            welcome.cycle_secs = secs;
        }
        if !welcome.shuffle_overridden {
            welcome.shuffle = live.welcome_image_shuffle;
        }
        welcome.rearm();
        self.hotkeys = live.hotkeys;
    }
}

/// The draft values [`ManagerApp::apply_live_settings`] copies onto the
/// manager, read out of the dialog before the save borrows `self`.
struct LiveSettings {
    toolbar_icons_only: bool,
    status_bar_icons_only: bool,
    welcome_image_cycle: bool,
    /// `None` only for a zero draft, which the DragValue range never produces.
    welcome_image_cycle_secs: Option<NonZeroU32>,
    welcome_image_shuffle: bool,
    hotkeys: Hotkeys,
}

impl LiveSettings {
    fn from_dialog(dialog: &SettingsDialog) -> Self {
        Self {
            toolbar_icons_only: dialog.toolbar_icons_only,
            status_bar_icons_only: dialog.status_bar_icons_only,
            welcome_image_cycle: dialog.welcome_image_cycle,
            welcome_image_cycle_secs: NonZeroU32::new(dialog.welcome_image_cycle_secs),
            welcome_image_shuffle: dialog.welcome_image_shuffle,
            hotkeys: dialog.hotkey_editor.hotkeys,
        }
    }
}

#[cfg(test)]
#[path = "settings_test.rs"]
mod tests;
