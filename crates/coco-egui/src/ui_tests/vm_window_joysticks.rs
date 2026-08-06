//! The VM window's status-bar joysticks entry/menu: the always-present
//! entry (reading "No joysticks" with both ports off), assigning a source
//! per port through the popped-up menu, and the entry's label tracking both
//! ports at once ("R: Keys · L: Mouse"). Split out of
//! [`super::vm_window_menus`] to keep that file under the project's
//! line-count guideline.

use egui_kittest::kittest::Queryable;

use crate::*;

use super::harness::*;

/// The joysticks entry is always present (unlike the old per-port readouts
/// it replaced, which appeared only once a port had a source): with both
/// ports at their default `JoySource::None` (`JoystickInputs::new`) it reads
/// "No joysticks" rather than disappearing, and carries neither port's "R:"/
/// "L:" label.
#[test]
fn status_bar_joystick_entry_shows_no_joysticks_at_boot() {
    let mut harness = boot_harness();
    harness.step();

    harness.get_by_label("No joysticks");
    assert!(
        harness.query_by_label_contains("R:").is_none(),
        "neither port has a source at boot, so no \"R:\" label should appear"
    );
    assert!(
        harness.query_by_label_contains("L:").is_none(),
        "neither port has a source at boot, so no \"L:\" label should appear"
    );
}

#[test]
fn joysticks_menu_assigns_a_source_to_the_right_stick() {
    let mut harness = boot_harness();

    // "No joysticks" is the entry's boot-time label and unambiguous — none
    // of the menu's own items (source names, stick headers) share it.
    click(&mut harness, "No joysticks");
    // Both sticks list the same four source labels; the right stick's list
    // is drawn first, so its "Keys" entry is the topmost one.
    topmost_by_label(&harness, "Keys").hover();
    harness.step();
    topmost_by_label(&harness, "Keys").click();
    harness.step();
    harness.step();

    let app = harness.state();
    assert_eq!(
        app.joysticks.sources[coco_core::joystick::RIGHT],
        joy::JoySource::Keys
    );
    assert_eq!(
        app.joysticks.sources[coco_core::joystick::LEFT],
        joy::JoySource::None
    );
    harness.get_by_label("R: Keys"); // the entry's label tracks the pick
}

/// The icon half of the click target (`CocoApp::joystick_status`'s
/// `widget_info` name, mirroring the keyboard/display icon tests): reopening
/// the menu through "Joysticks menu" rather than the entry's own label,
/// which by now reads "R: Keys" and isn't a menu item. Both sticks always
/// list all four source labels, so every pick disambiguates by draw order:
/// the right stick's list is drawn first (`topmost_by_label`), the left
/// stick's second (`lowest_by_label`). Ends by clearing both ports back to
/// `None`, restoring the "No joysticks" label.
#[test]
fn joystick_entry_label_tracks_both_ports() {
    let mut harness = boot_harness();

    click(&mut harness, "No joysticks");
    topmost_by_label(&harness, "Keys").hover();
    harness.step();
    topmost_by_label(&harness, "Keys").click();
    harness.step();
    harness.step();
    harness.get_by_label("R: Keys");

    click(&mut harness, "Joysticks menu");
    lowest_by_label(&harness, "Mouse").hover();
    harness.step();
    lowest_by_label(&harness, "Mouse").click();
    harness.step();
    harness.step();

    let app = harness.state();
    assert_eq!(
        app.joysticks.sources[coco_core::joystick::RIGHT],
        joy::JoySource::Keys
    );
    assert_eq!(
        app.joysticks.sources[coco_core::joystick::LEFT],
        joy::JoySource::Mouse
    );
    harness.get_by_label("R: Keys · L: Mouse"); // both ports in the label

    // Clear both back to None: two menu opens — the right stick's "None" is
    // the topmost copy (its list draws first), the left stick's the lowest.
    click(&mut harness, "R: Keys · L: Mouse");
    topmost_by_label(&harness, "None").hover();
    harness.step();
    topmost_by_label(&harness, "None").click();
    harness.step();
    harness.step();

    click(&mut harness, "Joysticks menu");
    lowest_by_label(&harness, "None").hover();
    harness.step();
    lowest_by_label(&harness, "None").click();
    harness.step();
    harness.step();

    harness.get_by_label("No joysticks");
}
