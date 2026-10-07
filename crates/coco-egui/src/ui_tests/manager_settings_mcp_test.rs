//! MCP controls preserve their draft across tab changes and checkbox toggles.

use egui_kittest::kittest::{NodeT, Queryable};

use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;
use super::manager_settings::settings_harness_with;

const CUSTOM_PORT: u16 = 7002;

#[test]
fn mcp_toggle_retains_custom_port_and_tabs_retain_drafts() {
    let dir = TempDir::new("settings-mcp-draft");
    let config_path = dir.path().join("config.toml");
    let mut harness = settings_harness_with(config_path.clone(), |app| {
        app.control_port_overridden = true;
    });
    click(&mut harness, "Settings");
    click(&mut harness, "Toolbar icons only");
    click(&mut harness, "MCP server");
    assert_port_enabled(&harness, true);
    set_port(&mut harness, CUSTOM_PORT);
    click(&mut harness, "Enable MCP server");
    assert_port_enabled(&harness, false);
    click(&mut harness, "Advanced");
    click(&mut harness, "MCP server");
    assert_port_enabled(&harness, false);
    click(&mut harness, "Enable MCP server");
    assert_port_enabled(&harness, true);
    click(&mut harness, "General");
    assert_eq!(
        harness
            .get_by_label("Toolbar icons only")
            .accesskit_node()
            .toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    click(&mut harness, "Save");

    let saved = config::load(Some(&config_path)).expect("saved config");
    assert_eq!(saved.control_port, Some(CUSTOM_PORT));
    assert_eq!(saved.toolbar_icons_only, Some(true));
}

#[test]
fn disabling_mcp_saves_zero_and_reopens_with_a_disabled_positive_port() {
    let dir = TempDir::new("settings-mcp-disabled");
    let config_path = dir.path().join("config.toml");
    let mut harness = settings_harness_with(config_path.clone(), |app| {
        app.control_port_overridden = true;
    });
    click(&mut harness, "Settings");
    click(&mut harness, "MCP server");
    click(&mut harness, "Enable MCP server");
    click(&mut harness, "Save");
    let saved = config::load(Some(&config_path)).expect("saved config");
    assert_eq!(saved.control_port, Some(0));

    click(&mut harness, "Settings");
    click(&mut harness, "MCP server");
    assert_port_enabled(&harness, false);
    assert_eq!(
        harness
            .get_by_label("Enable MCP server")
            .accesskit_node()
            .toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    click(&mut harness, "Enable MCP server");
    assert_port_enabled(&harness, true);
    click(&mut harness, "Save");
    let saved = config::load(Some(&config_path)).expect("saved config");
    assert_eq!(saved.control_port, None, "the default port is not pinned");
}

#[test]
fn an_enabled_mcp_port_cannot_be_zero() {
    let dir = TempDir::new("settings-mcp-positive-port");
    let config_path = dir.path().join("config.toml");
    let mut harness = settings_harness_with(config_path.clone(), |app| {
        app.control_port_overridden = true;
    });
    click(&mut harness, "Settings");
    click(&mut harness, "MCP server");
    set_port(&mut harness, 0);
    assert_port_enabled(&harness, true);
    click(&mut harness, "Save");

    let saved = config::load(Some(&config_path)).expect("saved config");
    assert_eq!(
        saved.control_port,
        Some(1),
        "zero is clamped to the minimum"
    );
}

fn assert_port_enabled(harness: &ManagerHarness, enabled: bool) {
    let port = harness.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Port");
    assert_eq!(port.accesskit_node().is_disabled(), !enabled);
    assert!(port.accesskit_node().numeric_value().expect("numeric port") > 0.0);
}

fn set_port(harness: &mut ManagerHarness, port: u16) {
    let target = harness
        .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Port")
        .accesskit_node()
        .id();
    harness
        .input_mut()
        .events
        .push(egui::Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                target,
                action: egui::accesskit::Action::SetValue,
                data: Some(egui::accesskit::ActionData::NumericValue(f64::from(port))),
            },
        ));
    harness.step();
}
