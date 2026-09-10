//! The Settings dialog behind the toolbar's ⚙ tile (`manager/toolbar.rs`):
//! edits the global `config.toml` in place. Follows the delete
//! confirmation's `egui::Modal` pattern (`manager/delete.rs`); write-back
//! goes through [`config::save_file`], which preserves the file's comments
//! and any hand-added keys.

use std::path::Path;

use clap::ValueEnum;
use eframe::egui;

use crate::cli::LogLevel;
use crate::config::{self, FileConfig};

use super::{DETAIL_SECTION_GAP, ManagerApp, NO_CONFIG_DIR};

/// `control_port`'s DragValue range; `0` disables the control server
/// (`manager/control.rs`'s `bind_control`).
const CONTROL_PORT_RANGE: std::ops::RangeInclusive<u16> = 0..=u16::MAX;

/// Width of the `assets_url` text field.
const ASSETS_URL_WIDTH: f32 = 360.0;

/// Hint line under the fields: which changes are immediate and which need a
/// restart, and that CLI/env overrides still win.
const RESTART_HINT: &str = "Log level and assets URL take effect the next time cocovm starts. \
     A command-line flag or environment variable for any of these still overrides this file.";

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
            error,
        }
    }

    /// `Some(value)` only where `value` differs from the built-in default,
    /// so a field the user never touched keeps tracking future defaults
    /// instead of pinning today's value into the file.
    fn to_file_config(&self) -> FileConfig {
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
        }
    }

    /// The modal's contents: the four fields, the restart hint, any error
    /// from the last load/save, and the Save/Cancel row.
    fn draw(&mut self, ui: &mut egui::Ui) -> SettingsAction {
        ui.heading("Settings");
        ui.add_space(DETAIL_SECTION_GAP);

        egui::Grid::new("settings_fields")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Log level");
                egui::ComboBox::from_id_salt("settings_log_level")
                    .selected_text(config::log_level_name(self.log_level))
                    .show_ui(ui, |ui| {
                        for level in LogLevel::value_variants() {
                            ui.selectable_value(
                                &mut self.log_level,
                                *level,
                                config::log_level_name(*level),
                            );
                        }
                    });
                ui.end_row();

                ui.label("Control port (0 = off)");
                ui.add(egui::DragValue::new(&mut self.control_port).range(CONTROL_PORT_RANGE));
                ui.end_row();

                ui.label("Assets URL");
                ui.add(
                    egui::TextEdit::singleline(&mut self.assets_url)
                        .desired_width(ASSETS_URL_WIDTH),
                );
                ui.end_row();
            });

        ui.add_space(DETAIL_SECTION_GAP);
        ui.checkbox(&mut self.toolbar_icons_only, "Toolbar icons only");

        ui.add_space(DETAIL_SECTION_GAP);
        ui.label(RESTART_HINT);

        if let Some(err) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }

        ui.add_space(DETAIL_SECTION_GAP);
        let mut action = SettingsAction::None;
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                action = SettingsAction::Save;
            }
            if ui.button("Cancel").clicked() {
                action = SettingsAction::Cancel;
            }
        });
        action
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

    /// Saves the draft to [`ManagerApp::config_path`] and closes the dialog;
    /// applies `toolbar_icons_only` and `control_port` live unless a CLI/env
    /// override is active. A failure (write or bind) shows in the dialog and
    /// leaves it open.
    fn commit_settings(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.settings else {
            return;
        };
        let file = dialog.to_file_config();
        let toolbar_icons_only = dialog.toolbar_icons_only;
        let port_change = (dialog.control_port != dialog.opened_control_port
            && !self.control_port_overridden)
            .then_some(dialog.control_port);
        let result = match self.config_path.as_deref() {
            Some(path) => config::save_file(path, &file),
            None => Err(NO_CONFIG_DIR.to_string()),
        }
        .and_then(|()| {
            if !self.toolbar_icons_only_overridden {
                self.toolbar_icons_only = toolbar_icons_only;
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
}

#[cfg(test)]
#[path = "settings_test.rs"]
mod tests;
