//! `SettingsDialog::to_file_config`'s collapse rules: only values that
//! diverge from the built-in defaults reach `config.toml`. Plus
//! `commit_settings`'s live control-port moves, which need the private
//! dialog fields a `ui_tests` harness can't reach.

use std::net::{Ipv4Addr, TcpListener};
use std::sync::Arc;

use eframe::egui;

use super::SettingsDialog;
use crate::cli::LogLevel;
use crate::config::FileConfig;
use crate::control::ControlServer;
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
    };
    let dialog = SettingsDialog::from_file(
        FileConfig {
            log_level: file.log_level,
            control_port: file.control_port,
            assets_url: file.assets_url.clone(),
            toolbar_icons_only: file.toolbar_icons_only,
        },
        None,
    );
    assert_eq!(dialog.to_file_config(), file);
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
/// naming the bind failure, and no listener is left behind.
#[test]
fn commit_reports_a_bind_failure_in_the_dialog() {
    let dir = TempDir::new("settings-port-taken");
    let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind ephemeral port");
    let port = taken.local_addr().expect("local addr").port();
    let mut manager = manager_with_listener(&dir, port);

    manager.commit_settings(&egui::Context::default());

    let dialog = manager.settings.as_ref().expect("dialog must stay open");
    let error = dialog
        .error
        .as_deref()
        .expect("dialog must show the bind error");
    assert!(error.contains(&port.to_string()), "{error}");
    assert!(manager.control.is_none());
    assert!(
        dir.path().join("config.toml").is_file(),
        "the file is saved before the rebind is attempted"
    );
}
