//! `SettingsDialog::to_file_config`'s collapse rules: only values that
//! diverge from the built-in defaults reach `config.toml`.

use coco_core::MachineConfig;

use super::SettingsDialog;
use crate::cli::LogLevel;
use crate::config::FileConfig;
use crate::machine_def::{self, tests::TempDir};
use crate::manager::{MachineEntry, ManagerApp};

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
    };
    let dialog = SettingsDialog::from_file(
        FileConfig {
            log_level: file.log_level,
            control_port: file.control_port,
            assets_url: file.assets_url.clone(),
            toolbar_icons_only: file.toolbar_icons_only,
            status_bar_icons_only: file.status_bar_icons_only,
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

/// Save pushes both icons-only toggles into every running VM, not just the
/// next launch. Boots a real machine like `lifecycle_test.rs` does.
#[test]
fn commit_pushes_icons_only_toggles_into_running_vms() {
    let machines_dir = TempDir::new("settings-commit-machines");
    let artifacts_root = TempDir::new("settings-commit-artifacts");
    let config_dir = TempDir::new("settings-commit-config");
    let def = machine_def::MachineDef::from_config(
        "Settings Test".to_string(),
        None,
        &MachineConfig::default(),
    );
    let mut manager = ManagerApp::new(
        None,
        Some(machines_dir.path().to_path_buf()),
        Some(artifacts_root.path().to_path_buf()),
        vec![MachineEntry::new("settings-test".to_string(), def)],
        None,
    );
    manager.config_path = Some(config_dir.path().join("config.toml"));
    manager.start_vm(0);
    let vm = manager.entries[0]
        .vm
        .as_ref()
        .expect("launch should succeed");
    assert!(!vm.toolbar_icons_only && !vm.status_bar_icons_only);

    let mut dialog = SettingsDialog::from_file(FileConfig::default(), None);
    dialog.toolbar_icons_only = true;
    dialog.status_bar_icons_only = true;
    manager.settings = Some(dialog);
    manager.commit_settings();

    assert!(manager.settings.is_none(), "Save must close the dialog");
    let vm = manager.entries[0].vm.as_ref().expect("VM still running");
    assert!(
        vm.toolbar_icons_only,
        "toolbar toggle must reach the running VM"
    );
    assert!(
        vm.status_bar_icons_only,
        "status bar toggle must reach the running VM"
    );
}
