//! The manager toolbar's Help tile: its menu opens the About window.

use egui_kittest::kittest::Queryable;

use crate::manager;
use crate::*;

use super::harness::*;

/// A line of the About window's text, to tell whether it is drawn.
const ABOUT_TEXT: &str = "A Tandy Color Computer 3 emulator";

/// Help > About cocovm opens the About window; its close box closes it.
#[test]
fn help_menu_opens_the_about_window() {
    let mut harness = egui_kittest::Harness::new_eframe(|_cc| {
        manager::ManagerApp::new(None, None, None, Vec::new(), None)
    });
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();
    assert!(harness.query_by_label(ABOUT_TEXT).is_none());

    click(&mut harness, "Help");
    click(&mut harness, "About cocovm");
    assert!(harness.state().show_about);
    harness.get_by_label(ABOUT_TEXT);

    click(&mut harness, "Close window");
    assert!(!harness.state().show_about);
    assert!(harness.query_by_label(ABOUT_TEXT).is_none());
}
