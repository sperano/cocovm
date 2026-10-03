//! `SettingsDialog::to_file_config`'s collapse rules: only values that
//! diverge from the built-in defaults reach `config.toml`. Plus
//! `commit_settings`'s live control-port moves and log re-leveling, which
//! need the private dialog fields a `ui_tests` harness can't reach.

use std::net::{Ipv4Addr, TcpListener};
use std::sync::Arc;

use eframe::egui;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::{EnvFilter, Registry, reload};

use std::num::NonZeroU32;

use super::SettingsDialog;
use crate::cli::LogLevel;
use crate::config::FileConfig;
use crate::control::ControlServer;
use crate::hotkeys::{DEFAULT_HOTKEYS, Hotkey, HotkeyAction};
use crate::machine_def::tests::TempDir;
use crate::manager::ManagerApp;

#[test]
fn defaults_collapse_to_an_empty_file_config() {
    let dialog = SettingsDialog::from_file(FileConfig::default(), None);
    assert_eq!(dialog.to_file_config(), FileConfig::default());
}

#[test]
fn non_default_values_round_trip() {
    let file = FileConfig {
        log_level: Some(LogLevel::Trace),
        control_port: Some(7002),
        assets_url: Some("https://example.test/bundle.tgz".to_string()),
        toolbar_icons_only: Some(true),
        status_bar_icons_only: Some(true),
        welcome_image_cycle: Some(true),
        welcome_image_cycle_secs: NonZeroU32::new(7),
        welcome_image_shuffle: Some(true),
        hotkey_key_layout: hotkey("F9"),
        hotkey_keyboard_mode: hotkey("Shift+F12"),
        hotkey_new_machine: hotkey("Cmd+Shift+N"),
        hotkey_debugger: hotkey("F11"),
    };
    let dialog = SettingsDialog::from_file(
        FileConfig {
            log_level: file.log_level,
            control_port: file.control_port,
            assets_url: file.assets_url.clone(),
            toolbar_icons_only: file.toolbar_icons_only,
            status_bar_icons_only: file.status_bar_icons_only,
            welcome_image_cycle: file.welcome_image_cycle,
            welcome_image_cycle_secs: file.welcome_image_cycle_secs,
            welcome_image_shuffle: file.welcome_image_shuffle,
            hotkey_key_layout: file.hotkey_key_layout,
            hotkey_keyboard_mode: file.hotkey_keyboard_mode,
            hotkey_new_machine: file.hotkey_new_machine,
            hotkey_debugger: file.hotkey_debugger,
        },
        None,
    );
    assert_eq!(dialog.to_file_config(), file);
}

fn hotkey(text: &str) -> Option<Hotkey> {
    Some(text.parse().expect("valid hotkey"))
}

/// A hotkey set back to its default leaves the file, so it keeps tracking
/// future defaults.
#[test]
fn a_default_hotkey_collapses_to_none() {
    let file = FileConfig {
        hotkey_key_layout: hotkey("F9"),
        ..FileConfig::default()
    };
    let mut dialog = SettingsDialog::from_file(file, None);
    dialog
        .hotkey_editor
        .hotkeys
        .set(HotkeyAction::KeyLayout, DEFAULT_HOTKEYS.key_layout);
    assert_eq!(dialog.to_file_config().hotkey_key_layout, None);
}

/// Save applies the drafted hotkeys to the manager, which pushes them to
/// its VM windows.
#[test]
fn commit_applies_the_hotkeys() {
    let dir = TempDir::new("settings-hotkeys-apply");
    let mut manager = ManagerApp::new(None, None, None, Vec::new(), None);
    manager.config_path = Some(dir.path().join("config.toml"));
    let mut dialog = SettingsDialog::from_file(FileConfig::default(), None);
    let f9 = hotkey("F9").expect("parsed");
    dialog
        .hotkey_editor
        .hotkeys
        .set(HotkeyAction::KeyLayout, f9);
    manager.settings = Some(dialog);

    manager.commit_settings(&egui::Context::default());

    assert!(manager.settings.is_none(), "Save must close the dialog");
    assert_eq!(manager.hotkeys.key_layout, f9);
}

/// Reset can hand an action back a default another action has taken
/// meanwhile; Save refuses that draft instead of writing a file that
/// would fail to load.
#[test]
fn commit_refuses_two_actions_on_one_hotkey() {
    let dir = TempDir::new("settings-hotkeys-clash");
    let config_path = dir.path().join("config.toml");
    let mut manager = ManagerApp::new(None, None, None, Vec::new(), None);
    manager.config_path = Some(config_path.clone());
    let mut dialog = SettingsDialog::from_file(FileConfig::default(), None);
    dialog
        .hotkey_editor
        .hotkeys
        .set(HotkeyAction::KeyLayout, DEFAULT_HOTKEYS.keyboard_mode);
    manager.settings = Some(dialog);

    manager.commit_settings(&egui::Context::default());

    let dialog = manager.settings.as_ref().expect("the dialog stays open");
    let error = dialog.error.as_deref().expect("the clash is reported");
    assert!(error.contains("both F12"), "{error}");
    assert_eq!(manager.hotkeys, DEFAULT_HOTKEYS, "nothing was applied");
    assert!(!config_path.exists(), "nothing was written");
}

#[test]
fn a_zeroed_shuffle_interval_draft_collapses_to_the_default() {
    let mut dialog = SettingsDialog::from_file(FileConfig::default(), None);
    dialog.welcome_image_cycle_secs = 0;
    assert_eq!(dialog.to_file_config().welcome_image_cycle_secs, None);
}

#[test]
fn emptied_assets_url_means_reset_to_default() {
    let mut dialog = SettingsDialog::from_file(FileConfig::default(), None);
    dialog.assets_url = "   ".to_string();
    assert_eq!(dialog.to_file_config().assets_url, None);
}

/// A manager with a live listener on an ephemeral port, its `config.toml`
/// under `dir`, and the Settings dialog open with `control_port` drafted.
fn manager_with_listener(dir: &TempDir, control_port: u16) -> ManagerApp {
    let server = ControlServer::bind(0, Arc::new(|| {})).expect("bind ephemeral port");
    let mut manager = ManagerApp::new(None, None, None, Vec::new(), Some(server));
    manager.config_path = Some(dir.path().join("config.toml"));
    let mut dialog = SettingsDialog::from_file(FileConfig::default(), None);
    dialog.control_port = control_port;
    manager.settings = Some(dialog);
    manager
}

/// A port nothing listens on right now: bound ephemerally, then released.
fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

#[test]
fn commit_moves_the_listener_to_the_new_port() {
    let dir = TempDir::new("settings-port-move");
    let target = free_port();
    let mut manager = manager_with_listener(&dir, target);
    let before = manager.control_port();
    assert_ne!(before, target);

    manager.commit_settings(&egui::Context::default());

    assert!(manager.settings.is_none(), "Save must close the dialog");
    assert_eq!(manager.control_port(), target);
}

#[test]
fn commit_with_port_zero_drops_the_listener() {
    let dir = TempDir::new("settings-port-zero");
    let mut manager = manager_with_listener(&dir, 0);

    manager.commit_settings(&egui::Context::default());

    assert!(manager.settings.is_none());
    assert!(
        manager.control.is_none(),
        "port 0 must disable the listener"
    );
}

#[test]
fn commit_keeps_the_listener_when_the_port_is_unchanged() {
    let dir = TempDir::new("settings-port-same");
    let mut manager = manager_with_listener(&dir, 0);
    let port = manager.control_port();
    manager.settings.as_mut().expect("dialog open").control_port = port;

    manager.commit_settings(&egui::Context::default());

    assert!(manager.settings.is_none());
    assert_eq!(manager.control_port(), port);
}

#[test]
fn commit_leaves_a_cli_env_port_override_alone() {
    let dir = TempDir::new("settings-port-override");
    let mut manager = manager_with_listener(&dir, 0);
    manager.control_port_overridden = true;
    let port = manager.control_port();

    manager.commit_settings(&egui::Context::default());

    assert!(manager.settings.is_none());
    assert_eq!(
        manager.control_port(),
        port,
        "an override must keep the listener"
    );
}

/// A port already in use: the file is saved, but the dialog stays open
/// naming the bind failure, and the old listener keeps serving.
#[test]
fn commit_reports_a_bind_failure_in_the_dialog() {
    let dir = TempDir::new("settings-port-taken");
    let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind ephemeral port");
    let port = taken.local_addr().expect("local addr").port();
    let mut manager = manager_with_listener(&dir, port);
    let before = manager.control_port();

    manager.commit_settings(&egui::Context::default());

    let dialog = manager.settings.as_ref().expect("dialog must stay open");
    let error = dialog
        .error
        .as_deref()
        .expect("dialog must show the bind error");
    assert!(error.contains(&port.to_string()), "{error}");
    assert_eq!(
        manager.control_port(),
        before,
        "the old listener must survive"
    );
    assert!(
        dir.path().join("config.toml").is_file(),
        "the file is saved before the rebind is attempted"
    );
}

/// A reload layer over a WARN filter that is never installed globally — the
/// handle swaps the filter whether or not a subscriber uses it, but only
/// weakly references the layer, so the caller must keep the layer alive.
/// Skipped under `RUST_LOG`: the env directives would change the hint.
fn log_reload_at_warn() -> Option<(
    reload::Layer<EnvFilter, Registry>,
    crate::startup::LogReload,
)> {
    if std::env::var_os("RUST_LOG").is_some() {
        eprintln!("skipping log re-level test: RUST_LOG is set");
        return None;
    }
    Some(reload::Layer::new(crate::startup::log_filter(
        LevelFilter::WARN,
    )))
}

fn current_level(handle: &crate::startup::LogReload) -> Option<LevelFilter> {
    handle
        .with_current(|filter| filter.max_level_hint())
        .expect("layer still alive")
}

#[test]
fn commit_relevels_the_log_subscriber() {
    let Some((_layer, handle)) = log_reload_at_warn() else {
        return;
    };
    let dir = TempDir::new("settings-log-level");
    let mut manager = ManagerApp::new(None, None, None, Vec::new(), None);
    manager.config_path = Some(dir.path().join("config.toml"));
    manager.log_reload = Some(handle.clone());
    let mut dialog = SettingsDialog::from_file(FileConfig::default(), None);
    dialog.log_level = LogLevel::Trace;
    manager.settings = Some(dialog);

    manager.commit_settings(&egui::Context::default());

    assert!(manager.settings.is_none());
    assert_eq!(current_level(&handle), Some(LevelFilter::TRACE));
}

#[test]
fn commit_leaves_a_cli_env_log_level_override_alone() {
    let Some((_layer, handle)) = log_reload_at_warn() else {
        return;
    };
    let dir = TempDir::new("settings-log-level-override");
    let mut manager = ManagerApp::new(None, None, None, Vec::new(), None);
    manager.config_path = Some(dir.path().join("config.toml"));
    manager.log_reload = Some(handle.clone());
    manager.log_level_overridden = true;
    let mut dialog = SettingsDialog::from_file(FileConfig::default(), None);
    dialog.log_level = LogLevel::Trace;
    manager.settings = Some(dialog);

    manager.commit_settings(&egui::Context::default());

    assert!(manager.settings.is_none());
    assert_eq!(current_level(&handle), Some(LevelFilter::WARN));
}
