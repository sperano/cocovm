//! `SettingsDialog::to_file_config`'s collapse rules: only values that
//! diverge from the built-in defaults reach `config.toml`.

use super::SettingsDialog;
use crate::cli::LogLevel;
use crate::config::FileConfig;

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
