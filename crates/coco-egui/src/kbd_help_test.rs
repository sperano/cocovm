use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::collections::BTreeSet;

const HARNESS_SIZE: egui::Vec2 = egui::vec2(1024.0, 768.0);
const SHIFT_CAP_COUNT: usize = 2;

struct KeyboardState {
    variant: MachineVariant,
    symbolic: bool,
    enabled: bool,
    modifiers: KeyModifiers,
    taps: Vec<KeyTap>,
    text: String,
    wants_keyboard: bool,
}

impl KeyboardState {
    fn new(variant: MachineVariant, symbolic: bool) -> Self {
        Self {
            variant,
            symbolic,
            enabled: true,
            modifiers: KeyModifiers::default(),
            taps: Vec::new(),
            text: String::new(),
            wants_keyboard: false,
        }
    }
}

fn harness(variant: MachineVariant, symbolic: bool) -> Harness<'static, KeyboardState> {
    let mut harness = Harness::new_ui_state(
        |ui, state: &mut KeyboardState| {
            ui.text_edit_singleline(&mut state.text)
                .labelled_by(ui.label("Text field").id);
            ui.add_enabled_ui(state.enabled, |ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                for row in layout::rows(state.variant) {
                    draw_row(
                        ui,
                        row,
                        state.symbolic,
                        &mut state.modifiers,
                        &mut state.taps,
                    );
                }
            });
            state.wants_keyboard = ui.ctx().wants_keyboard_input();
        },
        KeyboardState::new(variant, symbolic),
    );
    harness.set_size(HARNESS_SIZE);
    harness.run();
    harness
}

fn click(harness: &mut Harness<'_, KeyboardState>, label: &str, index: usize) {
    harness.get_all_by_label(label).nth(index).unwrap().hover();
    harness.step();
    harness.get_all_by_label(label).nth(index).unwrap().click();
    harness.run();
}

#[test]
fn every_nonmodifier_cap_clicks_its_matrix_key_in_both_modes() {
    for variant in [
        MachineVariant::Coco1,
        MachineVariant::Coco2,
        MachineVariant::Coco3,
    ] {
        for symbolic in [false, true] {
            let mut harness = harness(variant, symbolic);
            let mut actual = BTreeSet::new();
            for slot in layout::rows(variant)
                .iter()
                .copied()
                .flat_map(layout::slots)
            {
                let Slot::Cap(cap) = slot else { continue };
                if matches!(cap.label(), "SHIFT" | "CTRL" | "ALT") {
                    continue;
                }
                click(&mut harness, cap.label(), 0);
                let tap = harness
                    .state_mut()
                    .taps
                    .pop()
                    .expect("one click emits a tap");
                assert!(harness.state().taps.is_empty());
                assert_eq!(tap.pos, cap.pos(), "{}", cap.label());
                assert_eq!(tap.modifiers, KeyModifiers::default(), "{}", cap.label());
                assert!(actual.insert(tap.pos), "duplicate key {}", cap.label());
            }
            let expected = expected_nonmodifiers(variant);
            assert_eq!(actual, expected, "{variant:?}, symbolic={symbolic}");
        }
    }
}

fn expected_nonmodifiers(variant: MachineVariant) -> BTreeSet<keyboard::Pos> {
    (0..keyboard::ROWS as u8)
        .flat_map(|row| (0..keyboard::COLS as u8).map(move |column| (row, column)))
        .filter(|pos| !matches!(*pos, keyboard::SHIFT | keyboard::CTRL | keyboard::ALT))
        .filter(|pos| {
            variant == MachineVariant::Coco3 || !matches!(*pos, keyboard::F1 | keyboard::F2)
        })
        .collect()
}

#[test]
fn both_shift_caps_toggle_the_same_latch_independently() {
    let mut harness = harness(MachineVariant::Coco3, false);
    let ids: BTreeSet<_> = harness
        .get_all_by_label("SHIFT")
        .map(|node| node.accesskit_node().id())
        .collect();
    assert_eq!(ids.len(), SHIFT_CAP_COUNT);
    click(&mut harness, "SHIFT", 0);
    assert!(harness.state().modifiers.shift);
    assert!(
        harness.get_all_by_label("SHIFT").all(|node| {
            node.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
        })
    );
    click(&mut harness, "SHIFT", 1);
    assert!(!harness.state().modifiers.shift);
    assert!(harness.state().taps.is_empty());
}

#[test]
fn modifiers_latch_across_clicks_and_taps_keep_their_snapshot() {
    let mut harness = harness(MachineVariant::Coco3, true);
    for label in ["SHIFT", "CTRL", "ALT", "A", "B"] {
        click(&mut harness, label, 0);
    }
    let chord = KeyModifiers {
        shift: true,
        ctrl: true,
        alt: true,
    };
    assert_eq!(harness.state().modifiers, chord);
    for label in ["SHIFT", "CTRL", "ALT", "C"] {
        click(&mut harness, label, 0);
    }
    let taps = &harness.state().taps;
    assert_eq!(taps.len(), 3);
    assert_eq!(taps[0].modifiers, chord);
    assert_eq!(taps[1].modifiers, chord);
    assert_eq!(taps[2].modifiers, KeyModifiers::default());
}

#[test]
fn cap_click_releases_text_field_and_keycap_keyboard_focus() {
    let mut harness = harness(MachineVariant::Coco3, false);
    click(&mut harness, "Text field", 0);
    assert!(harness.state().wants_keyboard);
    click(&mut harness, "A", 0);
    assert!(!harness.state().wants_keyboard);
    click(&mut harness, "Text field", 0);
    assert!(harness.state().wants_keyboard);
    click(&mut harness, "CTRL", 0);
    assert!(!harness.state().wants_keyboard);
}

#[test]
fn disabled_keyboard_emits_no_taps_or_modifier_changes() {
    let mut harness = harness(MachineVariant::Coco3, false);
    harness.state_mut().enabled = false;
    harness.run();
    click(&mut harness, "A", 0);
    click(&mut harness, "CTRL", 0);
    assert!(harness.state().taps.is_empty());
    assert_eq!(harness.state().modifiers, KeyModifiers::default());
}

#[test]
fn closing_window_clears_latched_modifiers() {
    let state = (
        true,
        KeyModifiers {
            shift: true,
            ctrl: true,
            alt: true,
        },
    );
    let mut harness = Harness::new_state(
        |ctx, (open, modifiers)| {
            window(ctx, open, false, MachineVariant::Coco3, true, modifiers);
        },
        state,
    );
    harness.state_mut().0 = false;
    harness.run();
    assert_eq!(harness.state().1, KeyModifiers::default());
}
