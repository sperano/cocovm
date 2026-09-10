//! The detail pane's read-only "ROMs" group (`manager/roms.rs`): what the
//! selected definition will load, and whether each image is there.

use std::path::PathBuf;

use eframe::egui;
use egui_kittest::kittest::Queryable;

use crate::machine_def::tests::TempDir;
use crate::manager;

use super::harness::*;

/// A manager harness whose ROM lookups point at `roms_dir` — a temp dir,
/// never the user's installed ROMs.
fn roms_harness(roms_dir: PathBuf, entries: Vec<manager::MachineEntry>) -> ManagerHarness {
    let mut harness = egui_kittest::Harness::new_eframe(move |_cc| {
        let mut app = manager::ManagerApp::new(None, None, None, entries, None);
        app.roms_dir = Some(roms_dir.clone());
        app
    });
    harness.set_size(egui::vec2(1080.0, 1400.0));
    harness.step();
    harness
}

/// Selecting a machine lists its system ROM with a "missing" status when
/// the roms directory is empty; once the file appears, reselecting the
/// machine re-checks it.
#[test]
fn detail_pane_lists_the_system_rom_and_rechecks_it_on_reselect() {
    let dir = TempDir::new("ui-roms-listing");
    let mut harness = roms_harness(
        dir.path().to_path_buf(),
        vec![
            sample_entry("alpha", "Alpha CoCo 3"),
            sample_entry("beta", "Beta CoCo 3"),
        ],
    );

    click(&mut harness, "Alpha CoCo 3");
    harness.get_by_label("System ROM");
    harness.get_by_label("coco3.rom");
    harness.get_by_label("missing");

    std::fs::write(dir.path().join("coco3.rom"), [0u8; 16]).unwrap();
    click(&mut harness, "Beta CoCo 3");
    click(&mut harness, "Alpha CoCo 3");
    harness.get_by_label("differs from Super Extended Color BASIC 2.0 (CoCo 3 NTSC)");
    assert!(harness.query_by_label("missing").is_none());
}
