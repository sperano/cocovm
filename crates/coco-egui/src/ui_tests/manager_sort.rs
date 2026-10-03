//! Machine-list sort controls and preference persistence.

use egui_kittest::kittest::{NodeT, Queryable};

use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;

fn sort_harness(config_path: std::path::PathBuf) -> ManagerHarness {
    let mut alpha = sample_entry("alpha", "Alpha");
    alpha.def.created = Some("2026-01-01".to_string());
    let mut zulu = sample_entry("zulu", "Zulu");
    zulu.def.created = Some("2026-02-01".to_string());
    let entries = vec![alpha, zulu];
    let mut harness = egui_kittest::Harness::new_eframe(move |_cc| {
        let mut app = manager::ManagerApp::new(None, None, None, entries, None);
        app.config_path = Some(config_path);
        app
    });
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();
    harness
}

#[test]
fn controls_reorder_immediately_and_persist_each_choice() {
    let dir = TempDir::new("manager-sort-controls");
    let config_path = dir.path().join("config.toml");
    let mut harness = sort_harness(config_path.clone());
    assert_eq!(harness.state().entries[0].slug, "zulu");

    select_combo_at(&mut harness, "Date created", 0, "Name");

    assert_eq!(harness.state().entries[0].slug, "zulu");
    assert!(
        std::fs::read_to_string(&config_path)
            .expect("sort change creates config.toml")
            .contains("manager_sort = \"name-desc\"")
    );

    let direction_button = harness.get_by_label("Sort ascending");
    assert_eq!(
        direction_button.accesskit_node().role(),
        egui::accesskit::Role::Button
    );
    click(&mut harness, "Sort ascending");

    assert_eq!(harness.state().entries[0].slug, "alpha");
    harness.get_by_label("Sort descending");
    assert!(
        std::fs::read_to_string(config_path)
            .expect("direction change updates config.toml")
            .contains("manager_sort = \"name-asc\"")
    );
}
