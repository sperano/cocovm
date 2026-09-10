//! The Settings dialog behind the toolbar's ⚙ tile (`manager/toolbar.rs`):
//! edits the global `config.toml` in place. Follows the delete
//! confirmation's `egui::Modal` pattern (`manager/delete.rs`); write-back
//! goes through [`config::save_file`], which preserves the file's comments
//! and any hand-added keys.

use std::num::NonZeroU32;
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

/// `welcome_image_cycle_secs`'s DragValue range: the config key is
/// `NonZeroU32`, so the draft can never hold a zero.
const WELCOME_IMAGE_CYCLE_SECS_RANGE: std::ops::RangeInclusive<u32> = 1..=u32::MAX;

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
    /// Plain `u32` so `DragValue` can edit it; [`WELCOME_IMAGE_CYCLE_SECS_RANGE`]
    /// keeps it nonzero.
    welcome_image_cycle_secs: u32,
    welcome_image_shuffle: bool,
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
        }
    }

    /// The modal's contents: the fields, any error from the last
    /// load/save, and the Save/Cancel row.
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
        ui.checkbox(&mut self.status_bar_icons_only, "Status bar icons only");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.welcome_image_cycle, "Change welcome image every");
            ui.add_enabled(
                self.welcome_image_cycle,
                egui::DragValue::new(&mut self.welcome_image_cycle_secs)
                    .range(WELCOME_IMAGE_CYCLE_SECS_RANGE)
                    .suffix(" s"),
            );
        });
        ui.add_enabled(
            self.welcome_image_cycle,
            egui::Checkbox::new(&mut self.welcome_image_shuffle, "Shuffle welcome images"),
        );

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

    /// Saves the draft and closes the dialog; applies the icons-only toggles,
    /// the welcome-image cycle, `log_level` and `control_port` live unless a
    /// CLI/env override wins. A failure shows in the dialog, which stays open
    /// with the old listener.
    fn commit_settings(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.settings else {
            return;
        };
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
}

impl LiveSettings {
    fn from_dialog(dialog: &SettingsDialog) -> Self {
        Self {
            toolbar_icons_only: dialog.toolbar_icons_only,
            status_bar_icons_only: dialog.status_bar_icons_only,
            welcome_image_cycle: dialog.welcome_image_cycle,
            welcome_image_cycle_secs: NonZeroU32::new(dialog.welcome_image_cycle_secs),
            welcome_image_shuffle: dialog.welcome_image_shuffle,
        }
    }
}

#[cfg(test)]
#[path = "settings_test.rs"]
mod tests;
