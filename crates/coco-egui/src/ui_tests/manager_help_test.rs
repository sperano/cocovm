//! The About window's two entry points: the toolbar's Help menu and the
//! macOS application menu's request.

use std::fs;

use egui_kittest::kittest::Queryable;

use crate::machine_def::tests::TempDir;
use crate::manager;
use crate::*;

use super::harness::*;

/// A line of the About window's text, to tell whether it is drawn.
const ABOUT_TEXT: &str = "A Tandy Color Computer emulator";

/// Help > About CoCoVM opens the About window; its close box closes it.
#[cfg(not(target_os = "macos"))]
#[test]
fn help_menu_opens_the_about_window() {
    let mut harness = egui_kittest::Harness::new_eframe(|_cc| {
        manager::ManagerApp::new(None, None, None, Vec::new(), None)
    });
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();
    assert!(harness.query_by_label(ABOUT_TEXT).is_none());

    click(&mut harness, "Help");
    click(&mut harness, crate::about::MENU_LABEL);
    assert!(harness.state().show_about);
    harness.get_by_label(ABOUT_TEXT);

    click(&mut harness, "Close window");
    assert!(!harness.state().show_about);
    assert!(harness.query_by_label(ABOUT_TEXT).is_none());
}

/// A raised request opens the About window once: closing it keeps it closed.
#[test]
fn about_request_opens_the_about_window() {
    let mut harness = egui_kittest::Harness::new_eframe(|_cc| {
        manager::ManagerApp::new(None, None, None, Vec::new(), None)
    });
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();
    assert!(harness.query_by_label(ABOUT_TEXT).is_none());

    harness.state().about_request.raise();
    harness.step();
    assert!(harness.state().show_about);
    harness.get_by_label(ABOUT_TEXT);

    click(&mut harness, "Close window");
    assert!(!harness.state().show_about);
    assert!(harness.query_by_label(ABOUT_TEXT).is_none());
}

/// The About window repeats the startup banner's inventory line, counted
/// from the manager's own ROM and cartridge directories and machine list.
/// Opened through the request, the one entry point present on every platform;
/// the Help menu counts through the same `open_about`.
#[test]
fn about_window_shows_the_asset_and_machine_inventory() {
    let roms = TempDir::new("about-roms");
    let cartridges = TempDir::new("about-cartridges");
    for name in ["coco3.rom", "disk11.rom", "._coco3.rom"] {
        fs::write(roms.path().join(name), []).expect("write ROM");
    }
    fs::write(cartridges.path().join("Atom.ccc"), []).expect("write cartridge");
    let (roms_dir, cartridges_dir) = (roms.path().to_owned(), cartridges.path().to_owned());
    let entries = vec![
        sample_entry("coco3", "CoCo 3"),
        sample_coco2_entry("coco2", "CoCo 2"),
    ];
    let mut harness = egui_kittest::Harness::new_eframe(move |_cc| {
        let mut app = manager::ManagerApp::new(None, None, None, entries, None);
        app.roms_dir = Some(roms_dir);
        app.cartridges_dir = Some(cartridges_dir);
        app
    });
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();

    harness.state().about_request.raise();
    harness.step();
    harness.get_by_label("2 ROMs, 1 cartridge and 2 machine configurations found.");
}
