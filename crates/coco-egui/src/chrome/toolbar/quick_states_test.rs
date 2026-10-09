use eframe::egui;

use super::*;

/// The full group wins when it fits after the reserve, then the collapsed
/// tile, then nothing.
#[test]
fn group_fit_prefers_the_widest_layout_that_fits() {
    const FULL: f32 = 200.0;
    const COLLAPSED: f32 = 80.0;
    const RESERVE: f32 = 90.0;
    assert_eq!(
        group_fit(FULL + RESERVE, FULL, COLLAPSED, RESERVE),
        GroupFit::Full
    );
    assert_eq!(
        group_fit(FULL + RESERVE - 1.0, FULL, COLLAPSED, RESERVE),
        GroupFit::Collapsed
    );
    assert_eq!(
        group_fit(COLLAPSED + RESERVE, FULL, COLLAPSED, RESERVE),
        GroupFit::Collapsed
    );
    assert_eq!(
        group_fit(COLLAPSED + RESERVE - 1.0, FULL, COLLAPSED, RESERVE),
        GroupFit::Hidden
    );
    assert_eq!(group_fit(FULL, FULL, COLLAPSED, 0.0), GroupFit::Full);
}

/// Accessible names carry both the action and the state.
#[test]
fn action_names_identify_the_action_and_the_state() {
    assert_eq!(QuickAction::Save.name(0), "Save to State 1");
    assert_eq!(QuickAction::Load.name(4), "Load State 5");
}

/// The group's glyphs are in egui's bundled fonts, so no tile draws a
/// missing-glyph box.
#[test]
fn group_glyphs_are_in_the_bundled_fonts() {
    let ctx = egui::Context::default();
    let _ = ctx.run(egui::RawInput::default(), |_| {});
    let font = egui::FontId::proportional(egui::TextStyle::Body.resolve(&ctx.style()).size);
    for glyph in [SAVE_GLYPH, LOAD_GLYPH, STATES_GLYPH] {
        assert!(
            ctx.fonts_mut(|f| f.has_glyphs(&font, glyph)),
            "{glyph} missing from the bundled fonts"
        );
    }
}
