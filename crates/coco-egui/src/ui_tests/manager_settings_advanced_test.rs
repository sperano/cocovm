//! Advanced settings reset actions operate on the draft until Save.

use egui_kittest::kittest::{NodeT, Queryable};

use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;
use super::manager_settings::settings_harness;

const ASSETS_LABEL: &str = "Assets URL";
const EOU_LABEL: &str = "NitrOS-9 EOU URL";
const ASSETS_MIRROR: &str = "https://mirror.example.test/assets.tar.gz";
const EOU_MIRROR: &str = "https://mirror.example.test/eou.zip";
const LONG_URL_SEGMENTS: usize = 80;

fn open_advanced(harness: &mut ManagerHarness) {
    click(harness, "Settings");
    click(harness, "Advanced");
}

fn seed_urls(path: &std::path::Path, assets: &str, eou: &str) {
    config::save_file(
        path,
        &config::FileConfig {
            assets_url: Some(assets.to_string()),
            eou_url: Some(eou.to_string()),
            ..config::FileConfig::default()
        },
    )
    .expect("seed download sources");
}

fn assert_url(harness: &ManagerHarness, label: &str, expected: &str) {
    assert_eq!(
        harness
            .get_by_role_and_label(egui::accesskit::Role::TextInput, label)
            .accesskit_node()
            .value()
            .as_deref(),
        Some(expected)
    );
}

#[test]
fn default_actions_reset_independently_and_survive_tab_changes_until_save() {
    for (label, default, other_label, other_value) in [
        (
            ASSETS_LABEL,
            startup::DEFAULT_ASSETS_URL,
            EOU_LABEL,
            EOU_MIRROR,
        ),
        (EOU_LABEL, nitros9::EOU_URL, ASSETS_LABEL, ASSETS_MIRROR),
    ] {
        let dir = TempDir::new("settings-url-default-save");
        let path = dir.path().join("config.toml");
        seed_urls(&path, ASSETS_MIRROR, EOU_MIRROR);
        let mut harness = settings_harness(path.clone());
        open_advanced(&mut harness);
        click(&mut harness, &format!("Use default {label}"));
        assert_url(&harness, label, default);
        assert_url(&harness, other_label, other_value);
        click(&mut harness, "General");
        click(&mut harness, "Advanced");
        assert!(
            harness
                .get_by_label(&format!("Use default {label}"))
                .accesskit_node()
                .is_disabled()
        );
        click(&mut harness, "Save");
        assert!(harness.state().settings.is_none());
        let saved = config::load(Some(&path)).expect("saved config");
        if label == ASSETS_LABEL {
            assert_eq!(saved.assets_url, None);
            assert_eq!(saved.eou_url.as_deref(), Some(EOU_MIRROR));
        } else {
            assert_eq!(saved.eou_url, None);
            assert_eq!(saved.assets_url.as_deref(), Some(ASSETS_MIRROR));
            assert_eq!(harness.state().eou_url, nitros9::EOU_URL);
        }
    }
}

#[test]
fn cancelling_default_restoration_leaves_both_saved_urls_unchanged() {
    let dir = TempDir::new("settings-url-default-cancel");
    let path = dir.path().join("config.toml");
    seed_urls(&path, ASSETS_MIRROR, EOU_MIRROR);
    let before = std::fs::read(&path).expect("seeded config");
    let mut harness = settings_harness(path.clone());
    let live_eou = harness.state().eou_url.clone();
    open_advanced(&mut harness);
    for label in [ASSETS_LABEL, EOU_LABEL] {
        click(&mut harness, &format!("Use default {label}"));
    }
    assert_eq!(harness.state().eou_url, live_eou);
    click(&mut harness, "Cancel");
    assert_eq!(std::fs::read(&path).expect("unchanged config"), before);
    open_advanced(&mut harness);
    assert_url(&harness, ASSETS_LABEL, ASSETS_MIRROR);
    assert_url(&harness, EOU_LABEL, EOU_MIRROR);
}

#[test]
fn default_and_empty_urls_disable_their_default_actions() {
    for (assets, eou) in [(startup::DEFAULT_ASSETS_URL, nitros9::EOU_URL), ("", "   ")] {
        let dir = TempDir::new("settings-url-default-disabled");
        let path = dir.path().join("config.toml");
        seed_urls(&path, assets, eou);
        let mut harness = settings_harness(path.clone());
        open_advanced(&mut harness);
        for label in [ASSETS_LABEL, EOU_LABEL] {
            assert!(
                harness
                    .get_by_label(&format!("Use default {label}"))
                    .accesskit_node()
                    .is_disabled()
            );
        }
        click(&mut harness, "Save");
        let saved = config::load(Some(&path)).expect("saved config");
        assert_eq!(saved.assets_url, None);
        assert_eq!(saved.eou_url, None);
    }
}

#[test]
fn long_urls_stay_inside_the_dialog_in_light_and_dark_themes() {
    for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
        let dir = TempDir::new("settings-long-url");
        let path = dir.path().join("config.toml");
        let long_url = format!(
            "https://mirror.example.test/{}",
            "directory/".repeat(LONG_URL_SEGMENTS)
        );
        seed_urls(&path, &long_url, &long_url);
        let mut harness = settings_harness(path);
        harness.ctx.set_visuals(visuals);
        open_advanced(&mut harness);
        let dialog = harness
            .ctx
            .memory(|memory| memory.area_rect(egui::Id::new("settings_dialog")))
            .expect("dialog area");
        for label in [ASSETS_LABEL, EOU_LABEL] {
            assert_url(&harness, label, &long_url);
            let field = harness
                .get_by_role_and_label(egui::accesskit::Role::TextInput, label)
                .rect();
            assert!(dialog.contains_rect(field), "{label}: {field:?}");
            assert!(field.max.y < harness.get_by_label("Save").rect().min.y);
        }
    }
}

#[test]
fn focused_url_field_shows_the_mirror_archive_requirement() {
    let dir = TempDir::new("settings-url-keyboard-help");
    let mut harness = settings_harness(dir.path().join("config.toml"));
    open_advanced(&mut harness);
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, EOU_LABEL)
        .focus();
    harness.step();
    harness.step();
    harness.get_by_label_contains("A mirror must serve the same archive.");
}

#[test]
fn keyboard_can_restore_a_download_source_and_read_its_help() {
    let dir = TempDir::new("settings-url-keyboard-default");
    let path = dir.path().join("config.toml");
    seed_urls(&path, ASSETS_MIRROR, EOU_MIRROR);
    let mut harness = settings_harness(path);
    open_advanced(&mut harness);
    harness.get_by_label("Use default Assets URL").focus();
    harness.step();
    harness.step();
    harness.get_by_label_contains("Restore the default Assets URL:");
    harness.key_press(egui::Key::Space);
    harness.step();
    harness.step();
    assert_url(&harness, ASSETS_LABEL, startup::DEFAULT_ASSETS_URL);
    assert_url(&harness, EOU_LABEL, EOU_MIRROR);
}
