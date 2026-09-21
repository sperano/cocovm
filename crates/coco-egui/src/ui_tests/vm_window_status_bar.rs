//! The VM window's status bar under `status_bar_icons_only` (`config.rs`):
//! every iconed entry drops its readout, the icon-only click targets still
//! open their menus, and the icon-less entries are untouched.

use egui_kittest::kittest::Queryable;

use crate::machine_def::tests::TempDir;

use super::harness::*;

/// [`boot_harness`] with `status_bar_icons_only` set before the first frame.
fn icons_only_harness() -> AppHarness {
    let mut harness = boot_harness();
    harness.state_mut().status_bar_icons_only = true;
    harness.step();
    harness
}

/// The six menu entries stay reachable by their icons' accessible names,
/// while the readouts they used to draw next to them are gone.
#[test]
fn icons_only_keeps_menu_icons_and_drops_their_readouts() {
    let harness = icons_only_harness();

    for menu in [
        "Keyboard menu",
        "Display menu",
        "Sound menu",
        "Tape menu",
        "Joysticks menu",
        "Printer menu",
    ] {
        harness.get_by_label(menu);
    }
    for readout in ["Positional", "Sound", "No tape", "No joysticks", "Printer"] {
        assert!(
            harness.query_all_by_label(readout).next().is_none(),
            "{readout:?} readout must not be drawn in icons-only mode"
        );
    }
}

/// The icon alone opens its menu, and the menu works as usual.
#[test]
fn icons_only_tape_icon_opens_the_tape_menu() {
    let mut harness = icons_only_harness();

    click(&mut harness, "Tape menu");
    harness.get_by_label("Insert Tape…");
    click(&mut harness, "Also save tape audio (.wav)");
    assert!(harness.state().save_tape_wav);
}

/// A passive entry's readout moves onto its icon as the accessible name rather than
/// vanishing: the bare icon has none of its own (`status_icons::paint`).
#[test]
fn icons_only_passive_readout_becomes_the_icons_accessible_name() {
    let mut harness = icons_only_harness();
    let dir = TempDir::new("status-bar-icons-only-cart");
    let cart = dir.path().join("game.rom");
    std::fs::write(&cart, vec![0u8; 2048]).unwrap();
    harness.state_mut().insert_cartridge(cart);
    harness.step();

    harness.get_by_label("Cart: game.rom");
    assert!(
        harness
            .query_all_by_label("Cartridge ROM pak")
            .next()
            .is_none(),
        "the caller's hover text must not become the accessible name"
    );
}

/// The runtime readout has no icon, so icons-only mode leaves it drawn.
#[test]
fn icons_only_leaves_the_runtime_readout() {
    let harness = icons_only_harness();
    harness.get_by_label("Runtime: 0 s");
}
