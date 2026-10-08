//! The suspended VM window: inert chrome with Start as the Resume control,
//! the display overlay (scrim, Play glyph, centered "Suspended" marker), and
//! click-to-resume on the shrouded display. Split from the sibling
//! [`super::vm_window_menus`] to keep that file within bounds.

use egui_kittest::kittest::{NodeT, Queryable};

use crate::*;

use super::harness::*;

/// Pixel slop when checking an overlay node's horizontal centering.
const CENTER_TOLERANCE_PX: f32 = 1.0;

/// A suspended window keeps its chrome, read-only: Start becomes the Resume
/// control and requests it, Suspend/Reset are off, the status bar says so.
#[test]
fn suspended_window_keeps_chrome_with_start_as_resume() {
    let mut harness = boot_harness();
    harness.state_mut().suspended = true;
    harness.step();

    assert!(label_exists(&harness, "Suspended"));
    assert!(
        harness.get_by_label("View").accesskit_node().is_disabled(),
        "the menu bar must be inert while suspended"
    );
    for (label, enabled) in [
        ("Start", true),
        ("Suspend", false),
        ("Stop", true),
        ("Reset", false),
    ] {
        let is_disabled = harness.get_by_label(label).accesskit_node().is_disabled();
        assert_eq!(
            !is_disabled, enabled,
            "{label} enabled state while suspended"
        );
    }
    #[cfg(feature = "debug-ui")]
    assert!(harness.get_by_label("Debug").accesskit_node().is_disabled());

    click(&mut harness, "Start");
    assert!(
        harness.state().pending_resume,
        "Start on a suspended window must request a resume"
    );
}

/// The suspended display carries its own overlay beyond the status bar's
/// marker: a scrim over the whole frame, a centered Play glyph, and a
/// "Suspended" marker between the glyph and the bottom edge.
#[test]
fn suspended_display_shows_the_overlay() {
    let mut harness = boot_harness();
    assert_eq!(
        harness.query_all_by_label("Suspended").count(),
        0,
        "no Suspended marker anywhere while running"
    );

    harness.state_mut().suspended = true;
    harness.step();
    let display = harness.state().display_rect;
    let on_display = |label: &str| {
        harness
            .query_all_by_label(label)
            .filter(|node| {
                let center = node.rect().center();
                display.contains(center)
                    && (center.x - display.center().x).abs() < CENTER_TOLERANCE_PX
            })
            .count()
    };
    assert_eq!(on_display("Suspended"), 1, "one marker on the display");
    assert_eq!(
        on_display(PLAY_GLYPH),
        1,
        "one Play glyph centered on the display"
    );
    assert!(
        overlay_marker(&harness).rect().center().y > display.center().y,
        "the marker sits below the centered glyph"
    );
    assert!(
        scrim_covers(&harness, display),
        "the scrim must dim the whole display"
    );
}

/// Clicking anywhere on the shrouded display requests a resume, like Start.
#[test]
fn clicking_the_suspended_display_requests_a_resume() {
    let mut harness = boot_harness();
    harness.state_mut().suspended = true;
    harness.step();
    assert!(!harness.state().pending_resume);

    overlay_marker(&harness).hover();
    harness.step();
    overlay_marker(&harness).click();
    harness.step();
    harness.step();
    assert!(
        harness.state().pending_resume,
        "a click on the suspended display must request a resume"
    );
}

/// `ui.disable()` can't reach a menu that is already open, so the first
/// suspended frame closes whatever popup the running window left up.
#[test]
fn suspending_closes_an_open_menu() {
    let mut harness = boot_harness();
    click(&mut harness, "View");
    assert!(egui::Popup::is_any_open(&harness.ctx));

    harness.state_mut().suspended = true;
    harness.step();
    assert!(
        !egui::Popup::is_any_open(&harness.ctx),
        "a menu left open must not survive into the suspended window"
    );
}

/// The overlay's copy of the "Suspended" marker — the status bar shows
/// another, outside the display rect.
fn overlay_marker<'t>(harness: &'t AppHarness) -> egui_kittest::Node<'t> {
    let display = harness.state().display_rect;
    harness
        .query_all_by_label("Suspended")
        .find(|node| display.contains(node.rect().center()))
        .expect("suspended overlay marker on the display")
}

/// Whether the latest frame painted a rect with the scrim fill covering `display`.
fn scrim_covers(harness: &AppHarness, display: egui::Rect) -> bool {
    fn walk(shape: &egui::Shape, display: egui::Rect) -> bool {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| walk(shape, display)),
            egui::Shape::Rect(rect) => {
                rect.fill == app::SUSPENDED_SCRIM && rect.rect.contains_rect(display)
            }
            _ => false,
        }
    }
    harness
        .output()
        .shapes
        .iter()
        .any(|clipped| walk(&clipped.shape, display))
}
