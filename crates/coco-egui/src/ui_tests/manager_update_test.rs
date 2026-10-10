//! The update check's surfaces: the welcome notice, the About window's
//! update line, the Help menu item, the application menu's request, and the
//! Settings checkbox. Checks run against a loopback server
//! (`update::tests::serve_once`), never GitHub.

use std::time::Instant;

use egui_kittest::kittest::Queryable;

use crate::about::{CHECK_FAILED_TEXT, UP_TO_DATE_TEXT};
use crate::machine_def::tests::TempDir;
use crate::manager;
use crate::manager::welcome::{DISMISS_LABEL, RELEASE_LINK_TEXT};
use crate::update::tests::{CHECK_DEADLINE, POLL_INTERVAL, release_json, serve_once};
use crate::update::{Release, Status, UpdateCheck};
use crate::*;

use super::harness::*;
use super::manager_settings::settings_harness;

/// A manager whose update check queries `url`.
fn update_harness(url: String) -> ManagerHarness {
    let mut harness = egui_kittest::Harness::new_eframe(move |_cc| {
        let mut app = manager::ManagerApp::new(None, None, None, Vec::new(), None);
        app.update_check = UpdateCheck::new(url.clone());
        app
    });
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();
    harness
}

/// Step frames until the running check has finished.
fn finish_check(harness: &mut ManagerHarness) {
    let deadline = Instant::now() + CHECK_DEADLINE;
    while matches!(harness.state().update_check.status(), Status::Checking(_)) {
        assert!(Instant::now() < deadline, "update check did not finish");
        std::thread::sleep(POLL_INTERVAL);
        harness.step();
    }
    harness.step();
}

fn newer_release() -> Release {
    let current = crate::update::current_version();
    let version = semver::Version::new(current.major, current.minor + 1, 0);
    Release {
        page_url: format!("https://github.com/sperano/cocovm/releases/tag/v{version}"),
        version,
    }
}

/// A newer release shows in the welcome panel with its link; Dismiss hides it.
#[test]
fn welcome_panel_announces_a_newer_release_until_dismissed() {
    let release = newer_release();
    let heading = format!("CoCoVM {} is available.", release.version);
    let mut harness = update_harness(String::new());
    harness
        .state_mut()
        .update_check
        .set_status(Status::Available(release));
    harness.step();
    harness.get_by_label(&heading);
    harness.get_by_label(RELEASE_LINK_TEXT);

    click(&mut harness, DISMISS_LABEL);
    assert!(harness.query_by_label(&heading).is_none());
    assert!(harness.query_by_label(RELEASE_LINK_TEXT).is_none());
}

/// The application menu's request starts a check and opens the About
/// window, which reports a newer release.
#[test]
fn update_request_checks_and_reports_a_newer_release_in_about() {
    let release = newer_release();
    let url = serve_once("200 OK", release_json(&format!("v{}", release.version)));
    let mut harness = update_harness(url);

    harness.state().update_request.raise();
    harness.step();
    assert!(harness.state().show_about);
    finish_check(&mut harness);
    harness.get_by_label(&format!("Version {} is available", release.version));
}

/// A check finding the built version reports "up to date" in About.
#[test]
fn about_reports_up_to_date() {
    let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
    let mut harness = update_harness(serve_once("200 OK", release_json(&tag)));

    harness.state().update_request.raise();
    harness.step();
    finish_check(&mut harness);
    harness.get_by_label(UP_TO_DATE_TEXT);
}

/// A failed startup check stays quiet; the same failure on request shows.
#[test]
fn about_shows_a_failure_only_for_a_requested_check() {
    let mut harness = update_harness(serve_once("500 Internal Server Error", String::new()));
    harness
        .state_mut()
        .update_check
        .start(&egui::Context::default(), false);
    finish_check(&mut harness);
    harness.state().about_request.raise();
    harness.step();
    assert!(harness.state().show_about);
    assert!(harness.query_by_label(CHECK_FAILED_TEXT).is_none());

    harness.state_mut().update_check =
        UpdateCheck::new(serve_once("500 Internal Server Error", String::new()));
    harness.state().update_request.raise();
    harness.step();
    finish_check(&mut harness);
    harness.get_by_label(CHECK_FAILED_TEXT);
}

/// Help > Check for Updates… starts a requested check and opens About.
#[cfg(not(target_os = "macos"))]
#[test]
fn help_menu_checks_for_updates() {
    let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
    let mut harness = update_harness(serve_once("200 OK", release_json(&tag)));

    click(&mut harness, "Help");
    click(&mut harness, crate::update::MENU_LABEL);
    assert!(harness.state().show_about);
    assert!(harness.state().update_check.requested());
    finish_check(&mut harness);
    harness.get_by_label(UP_TO_DATE_TEXT);
}

/// Clearing the General tab's checkbox saves `check_for_updates = false`.
#[test]
fn settings_checkbox_saves_check_for_updates() {
    let dir = TempDir::new("settings-check-for-updates");
    let config_path = dir.path().join("config.toml");
    let mut harness = settings_harness(config_path.clone());

    click(&mut harness, "Settings");
    click(&mut harness, "Check for updates at startup");
    click(&mut harness, "Save");

    let saved = std::fs::read_to_string(&config_path).expect("save_file must create the file");
    assert!(
        saved
            .lines()
            .any(|l| l.trim() == "check_for_updates = false"),
        "config.toml must contain the saved key: {saved}"
    );
}
