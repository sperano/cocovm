//! The Settings dialog (`manager/settings.rs`): opening it from the
//! toolbar, editing a value, saving it into `config.toml`, and reopening
//! to see the saved value.

use egui_kittest::kittest::{NodeT, Queryable};

use crate::machine_def::tests::TempDir;
use crate::manager;
use crate::*;

use super::harness::*;

/// A manager harness pointed at a temp-dir `config.toml`
/// (`ManagerApp::config_path`) — never the user's real config directory.
fn settings_harness(config_path: std::path::PathBuf) -> ManagerHarness {
    settings_harness_with(config_path, |_| {})
}

/// [`settings_harness`] with extra app setup applied before the first frame.
fn settings_harness_with(
    config_path: std::path::PathBuf,
    configure: impl FnOnce(&mut manager::ManagerApp) + Send + 'static,
) -> ManagerHarness {
    let mut configure = Some(configure);
    let mut harness = egui_kittest::Harness::new_eframe(move |_cc| {
        let mut app = manager::ManagerApp::new(None, None, None, Vec::new(), None);
        app.config_path = Some(config_path.clone());
        (configure.take().expect("app is built once"))(&mut app);
        app
    });
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();
    harness
}

/// The toolbar's Settings tile opens the modal, seeded with defaults when
/// no `config.toml` exists yet.
#[test]
fn settings_button_opens_the_dialog() {
    let dir = TempDir::new("settings-open");
    let mut harness = settings_harness(dir.path().join("config.toml"));

    click(&mut harness, "Settings");

    harness.get_by_label("Save");
    harness.get_by_label("Cancel");
    assert!(harness.state().settings.is_some());
}

/// Toggling `toolbar_icons_only` and clicking Save writes the key to
/// `config.toml`, applies it to the manager's own toolbar immediately, and
/// closes the dialog; reopening shows the saved value rather than the
/// built-in default.
#[test]
fn save_writes_the_toggled_value_and_reopening_shows_it() {
    let dir = TempDir::new("settings-save");
    let config_path = dir.path().join("config.toml");
    let mut harness = settings_harness(config_path.clone());
    assert!(!harness.state().toolbar_icons_only);

    click(&mut harness, "Settings");
    click(&mut harness, "Toolbar icons only");
    click(&mut harness, "Save");

    assert!(
        harness.state().settings.is_none(),
        "Save must close the dialog"
    );
    assert!(
        harness.state().toolbar_icons_only,
        "Save must apply the toggle to the manager's own toolbar immediately"
    );
    let saved = std::fs::read_to_string(&config_path).expect("save_file must create the file");
    assert!(
        saved
            .lines()
            .any(|l| l.trim() == "toolbar_icons_only = true"),
        "config.toml must contain the saved key: {saved}"
    );

    click(&mut harness, "Settings");
    let toggled = harness
        .get_by_label("Toolbar icons only")
        .accesskit_node()
        .toggled();
    assert_eq!(
        toggled,
        Some(egui::accesskit::Toggled::True),
        "reopening the dialog must show the previously saved value"
    );
}

/// `status_bar_icons_only`'s Save path: the key lands in `config.toml` and
/// the manager's copy — what the next VM launch inherits
/// (`manager/lifecycle.rs`) — flips immediately, like the toolbar toggle.
#[test]
fn save_applies_the_status_bar_toggle_to_the_next_launch() {
    let dir = TempDir::new("settings-save-status-bar");
    let config_path = dir.path().join("config.toml");
    let mut harness = settings_harness(config_path.clone());
    assert!(!harness.state().status_bar_icons_only);

    click(&mut harness, "Settings");
    click(&mut harness, "Status bar icons only");
    click(&mut harness, "Save");

    assert!(harness.state().status_bar_icons_only);
    let saved = std::fs::read_to_string(&config_path).expect("save_file must create the file");
    assert!(
        saved
            .lines()
            .any(|l| l.trim() == "status_bar_icons_only = true"),
        "config.toml must contain the saved key: {saved}"
    );
}

/// When a CLI flag or env var supplied either icons-only toggle, Save
/// leaves the live value on the override; only the file changes.
#[test]
fn save_keeps_a_cli_env_icons_only_override_until_restart() {
    let dir = TempDir::new("settings-override");
    let config_path = dir.path().join("config.toml");
    let mut harness = settings_harness_with(config_path.clone(), |app| {
        app.toolbar_icons_only = true;
        app.toolbar_icons_only_overridden = true;
        app.status_bar_icons_only = true;
        app.status_bar_icons_only_overridden = true;
    });

    click(&mut harness, "Settings");
    click(&mut harness, "Save");

    assert!(
        harness.state().toolbar_icons_only,
        "Save must not clobber a CLI/env toolbar_icons_only override"
    );
    assert!(
        harness.state().status_bar_icons_only,
        "Save must not clobber a CLI/env status_bar_icons_only override"
    );
    let saved = std::fs::read_to_string(&config_path).expect("save_file must create the file");
    assert!(
        !saved
            .lines()
            .any(|l| l.trim().starts_with("toolbar_icons_only =")),
        "a draft left at the default must not pin the key into the file: {saved}"
    );
}

/// Cancel discards the draft: no file is written and the manager's own
/// toolbar setting is untouched.
#[test]
fn cancel_discards_the_draft() {
    let dir = TempDir::new("settings-cancel");
    let config_path = dir.path().join("config.toml");
    let mut harness = settings_harness(config_path.clone());

    click(&mut harness, "Settings");
    click(&mut harness, "Toolbar icons only");
    click(&mut harness, "Cancel");

    assert!(harness.state().settings.is_none());
    assert!(!harness.state().toolbar_icons_only);
    assert!(!config_path.is_file(), "Cancel must not write config.toml");
}
