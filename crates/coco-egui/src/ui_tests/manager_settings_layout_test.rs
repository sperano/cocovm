//! Settings layout regressions at normal and constrained viewport sizes.

use egui_kittest::kittest::Queryable;

use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;
use super::manager_settings::settings_harness;

const SMALL_VIEWPORT: egui::Vec2 = egui::vec2(480.0, 280.0);
const SCROLL_TO_END: f32 = -2000.0;
const SETTLE_FRAMES: usize = 30;
const MIN_URL_WIDTH: f32 = 480.0;

#[test]
fn settings_tabs_show_their_controls_and_a_full_width_url() {
    let dir = TempDir::new("settings-sections");
    let mut harness = settings_harness(dir.path().join("config.toml"));
    click(&mut harness, "Settings");

    for title in [
        "General",
        "Appearance",
        "Welcome images",
        "Hotkeys",
        "MCP server",
        "Advanced",
    ] {
        harness.get_by_label(title);
    }
    click(&mut harness, "Advanced");
    let url = harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Assets URL")
        .rect();
    assert!(
        url.width() >= MIN_URL_WIDTH,
        "URL field uses the dialog width"
    );
    assert!(url.max.y < harness.get_by_label("Save").rect().min.y);
}

#[test]
fn small_window_keeps_actions_visible_while_scrolling_and_saves_edits() {
    let dir = TempDir::new("settings-small-save");
    let config_path = dir.path().join("config.toml");
    let mut harness = settings_harness(config_path.clone());
    click(&mut harness, "Settings");
    harness.set_size(SMALL_VIEWPORT);
    for _ in 0..SETTLE_FRAMES {
        harness.step();
    }
    click(&mut harness, "Toolbar icons only");
    let save_before = harness.get_by_label("Save").rect();
    assert_actions_visible(&harness);

    scroll_to_end(&mut harness);
    click(&mut harness, "Advanced");

    let url = harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Assets URL")
        .rect();
    let save_after = harness.get_by_label("Save").rect();
    assert_eq!(
        save_before, save_after,
        "scrolling must not move the footer"
    );
    assert!(url.min.x >= 0.0 && url.max.x <= SMALL_VIEWPORT.x);
    assert!(
        url.max.y < save_after.min.y,
        "last field is above the footer"
    );
    assert_actions_visible(&harness);
    click(&mut harness, "Save");
    assert!(harness.state().settings.is_none());
    assert!(harness.state().toolbar_icons_only);
    let saved = config::load(Some(&config_path)).expect("saved config");
    assert_eq!(saved.toolbar_icons_only, Some(true));
}

#[test]
fn small_window_cancel_after_scrolling_discards_edits() {
    let dir = TempDir::new("settings-small-cancel");
    let config_path = dir.path().join("config.toml");
    let mut harness = settings_harness(config_path.clone());
    click(&mut harness, "Settings");
    click(&mut harness, "Toolbar icons only");
    harness.set_size(SMALL_VIEWPORT);
    for _ in 0..SETTLE_FRAMES {
        harness.step();
    }
    scroll_to_end(&mut harness);
    click(&mut harness, "MCP server");
    click(&mut harness, "Enable MCP server");
    click(&mut harness, "Cancel");
    assert!(harness.state().settings.is_none());
    assert!(!harness.state().toolbar_icons_only);
    assert!(!config_path.exists());
}

#[test]
fn small_window_save_error_stays_visible_with_the_actions() {
    let dir = TempDir::new("settings-small-error");
    let parent = dir.path().join("blocked");
    let config_path = parent.join("config.toml");
    let mut harness = settings_harness(config_path.clone());
    click(&mut harness, "Settings");
    harness.set_size(SMALL_VIEWPORT);
    for _ in 0..SETTLE_FRAMES {
        harness.step();
    }
    std::fs::write(&parent, "not a directory").expect("block the save path");
    click(&mut harness, "Save");
    for _ in 0..SETTLE_FRAMES {
        harness.step();
    }
    assert!(harness.state().settings.is_some());
    click(&mut harness, "Advanced");
    let error = harness
        .get_by_label_contains(parent.to_str().expect("UTF-8 temp path"))
        .rect();
    assert!(error.min.y >= 0.0 && error.max.y < harness.get_by_label("Save").rect().min.y);
    assert_actions_visible(&harness);
    click(&mut harness, "Cancel");
    assert!(harness.state().settings.is_none());
    assert!(!config_path.exists());
}

fn assert_actions_visible(harness: &ManagerHarness) {
    for label in ["Save", "Cancel"] {
        let rect = harness.get_by_label(label).rect();
        assert!(rect.min.x >= 0.0 && rect.max.x <= SMALL_VIEWPORT.x);
        assert!(rect.min.y >= 0.0 && rect.max.y <= SMALL_VIEWPORT.y);
    }
}

fn scroll_to_end(harness: &mut ManagerHarness) {
    harness.get_by_label("Toolbar icons only").hover();
    harness.step();
    harness.input_mut().events.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, SCROLL_TO_END),
        modifiers: egui::Modifiers::NONE,
    });
    for _ in 0..SETTLE_FRAMES {
        harness.step();
    }
}

#[test]
fn changing_tabs_keeps_the_footer_in_place() {
    let dir = TempDir::new("settings-tab-geometry");
    let mut harness = settings_harness(dir.path().join("config.toml"));
    click(&mut harness, "Settings");
    harness.set_size(SMALL_VIEWPORT);
    for _ in 0..SETTLE_FRAMES {
        harness.step();
    }
    let original = harness.get_by_label("Save").rect();
    for tab in ["Hotkeys", "MCP server", "Advanced", "General"] {
        click(&mut harness, tab);
        assert_eq!(harness.get_by_label("Save").rect(), original, "{tab}");
        assert_actions_visible(&harness);
    }
}
