use super::*;

#[test]
fn pot_from_unit_clamps_and_scales() {
    assert_eq!(pot_from_unit(0.0), AXIS_MIN);
    assert_eq!(pot_from_unit(1.0), AXIS_MAX);
    assert_eq!(pot_from_unit(-1.0), AXIS_MIN);
    assert_eq!(pot_from_unit(2.0), AXIS_MAX);
    assert_eq!(pot_from_unit(0.5), 32); // rounds to the nearest pot step
}

#[test]
fn pot_from_bipolar_maps_full_range() {
    assert_eq!(pot_from_bipolar(-1.0), AXIS_MIN);
    assert_eq!(pot_from_bipolar(1.0), AXIS_MAX);
    assert_eq!(pot_from_bipolar(0.0), 32);
}

#[test]
fn axis_from_keys_centers_on_conflict_or_no_input() {
    assert_eq!(axis_from_keys(false, false), AXIS_CENTER);
    assert_eq!(axis_from_keys(true, true), AXIS_CENTER);
    assert_eq!(axis_from_keys(true, false), AXIS_MIN);
    assert_eq!(axis_from_keys(false, true), AXIS_MAX);
}

#[test]
fn joy_source_default_is_none() {
    assert_eq!(JoySource::default(), JoySource::None);
}

#[test]
fn keys_active_reflects_either_port() {
    let mut inputs = JoystickInputs::new();
    assert!(!inputs.keys_active());
    inputs.sources[LEFT] = JoySource::Keys;
    assert!(inputs.keys_active());
}

#[test]
fn joystick_inputs_start_not_in_use() {
    assert_eq!(JoystickInputs::new().in_use, [false, false]);
}

#[test]
fn mouse_in_use_only_on_a_button() {
    assert!(!mouse_in_use(false, false));
    assert!(mouse_in_use(true, false));
    assert!(mouse_in_use(false, true));
}

/// The display rect shared by the `press_began_on_display` tests, with
/// room around it standing in for the chrome.
fn test_display_rect() -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(10.0, 10.0), egui::pos2(110.0, 90.0))
}

#[test]
fn press_began_on_display_gates_on_press_origin() {
    let display = test_display_rect();
    let bg = egui::LayerId::background();
    // No button down at all.
    assert!(!press_began_on_display(None, None, display, bg));
    // Press started on the bare display (no egui area at the origin).
    assert!(press_began_on_display(
        Some(egui::pos2(60.0, 50.0)),
        None,
        display,
        bg
    ));
    // Press started on the chrome (above the display, e.g. the menu bar) —
    // never fires, wherever the pointer is dragged afterwards.
    assert!(!press_began_on_display(
        Some(egui::pos2(60.0, 5.0)),
        None,
        display,
        bg
    ));
    // Press started below the display (e.g. the status bar).
    assert!(!press_began_on_display(
        Some(egui::pos2(60.0, 95.0)),
        None,
        display,
        bg
    ));
}

#[test]
fn press_began_on_display_respects_overlays() {
    let display = test_display_rect();
    let inside = Some(egui::pos2(60.0, 50.0));
    let overlay = egui::LayerId::new(egui::Order::Middle, egui::Id::new("menu-popup"));
    // Geometrically inside the display, but the press landed on a popup
    // floating above it — must not double as a fire-button press.
    assert!(!press_began_on_display(
        inside,
        Some(overlay),
        display,
        egui::LayerId::background()
    ));
    // Manager's embedded fallback: the display lives inside an
    // `egui::Window`, so the layer at the origin is the display's own.
    assert!(press_began_on_display(
        inside,
        Some(overlay),
        display,
        overlay
    ));
}

#[test]
fn press_began_on_display_never_true_before_first_frame() {
    // `display_rect` starts as `Rect::NOTHING` until `draw_display` runs;
    // a press must not fire against that placeholder.
    assert!(!press_began_on_display(
        Some(egui::pos2(0.0, 0.0)),
        None,
        egui::Rect::NOTHING,
        egui::LayerId::background()
    ));
}

#[test]
fn gamepad_in_use_on_button_or_deflection() {
    assert!(!gamepad_in_use([false, false], [0.0, 0.0]));
    assert!(gamepad_in_use([true, false], [0.0, 0.0]));
    assert!(gamepad_in_use([false, true], [0.0, 0.0]));
    // Below the deflection threshold: still idle.
    assert!(!gamepad_in_use([false, false], [PAD_DEFLECT - 0.01, 0.0]));
    // Past it, either sign, either axis.
    assert!(gamepad_in_use([false, false], [PAD_DEFLECT + 0.01, 0.0]));
    assert!(gamepad_in_use([false, false], [0.0, -(PAD_DEFLECT + 0.01)]));
}

#[test]
fn keys_in_use_on_any_mapped_key() {
    let idle = KeyState {
        left: false,
        right: false,
        up: false,
        down: false,
        button0: false,
        button1: false,
    };
    assert!(!keys_in_use(idle));
    assert!(keys_in_use(KeyState { left: true, ..idle }));
    assert!(keys_in_use(KeyState {
        button1: true,
        ..idle
    }));
}
