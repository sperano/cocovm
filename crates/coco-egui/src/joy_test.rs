use super::*;

#[test]
fn pot_from_unit_clamps_and_scales() {
    assert_eq!(pot_from_unit(0.0), POT_MIN);
    assert_eq!(pot_from_unit(1.0), POT_MAX);
    assert_eq!(pot_from_unit(-1.0), POT_MIN);
    assert_eq!(pot_from_unit(2.0), POT_MAX);
    assert_eq!(pot_from_unit(0.5), 512); // rounds to the nearest pot step
}

#[test]
fn pot_from_bipolar_maps_full_range() {
    assert_eq!(pot_from_bipolar(-1.0), POT_MIN);
    assert_eq!(pot_from_bipolar(1.0), POT_MAX);
    assert_eq!(pot_from_bipolar(0.0), 512);
}

/// A display rect with a bordered active sub-rect inside it, for the mouse
/// axis-mapping tests: the active picture is inset from the
/// full (bordered) display, like `active_screen_rect` derives from
/// `Machine::active_rect`.
const ACTIVE: egui::Rect =
    egui::Rect::from_min_max(egui::pos2(20.0, 20.0), egui::pos2(100.0, 80.0));

#[test]
fn pot_axes_from_pointer_maps_over_the_active_rect() {
    // Left edge of the active area -> full-left X; vertical center -> center Y.
    assert_eq!(
        pot_axes_from_pointer(egui::pos2(20.0, 50.0), ACTIVE),
        Some((POT_MIN, 512))
    );
    // Center of the active area -> center on both axes.
    assert_eq!(
        pot_axes_from_pointer(egui::pos2(60.0, 50.0), ACTIVE),
        Some((512, 512))
    );
    // Bottom-right corner -> full deflection on both axes.
    assert_eq!(
        pot_axes_from_pointer(egui::pos2(100.0, 80.0), ACTIVE),
        Some((POT_MAX, POT_MAX))
    );
}

#[test]
fn pot_axes_from_pointer_clamps_a_pointer_in_the_border() {
    // Left of the active area but inside the border pins X at full-left rather than going negative.
    assert_eq!(
        pot_axes_from_pointer(egui::pos2(0.0, 50.0), ACTIVE),
        Some((POT_MIN, 512))
    );
    // Past the corner pins both axes at full deflection rather than overshooting.
    assert_eq!(
        pot_axes_from_pointer(egui::pos2(200.0, 200.0), ACTIVE),
        Some((POT_MAX, POT_MAX))
    );
}

#[test]
fn pot_axes_from_pointer_guards_a_zero_size_active_rect() {
    let degenerate = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(0.0, 0.0));
    assert_eq!(
        pot_axes_from_pointer(egui::pos2(5.0, 5.0), degenerate),
        None
    );
}

#[test]
fn axis_from_keys_centers_on_conflict_or_no_input() {
    assert_eq!(axis_from_keys(false, false), POT_CENTER);
    assert_eq!(axis_from_keys(true, true), POT_CENTER);
    assert_eq!(axis_from_keys(true, false), POT_MIN);
    assert_eq!(axis_from_keys(false, true), POT_MAX);
}

fn key_state_frame(ctx: &egui::Context, raw: egui::RawInput) -> KeyState {
    let mut state = KeyState::default();
    let _ = ctx.run(raw, |ctx| state = JoystickInputs::key_state(ctx));
    state
}

#[test]
fn key_state_centers_while_viewport_is_unfocused() {
    let ctx = egui::Context::default();
    let pressed = egui::RawInput {
        events: vec![egui::Event::Key {
            key: egui::Key::ArrowLeft,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }],
        ..Default::default()
    };
    assert!(key_state_frame(&ctx, pressed).left);

    // Keep egui's held-key state but remove focus, modeling a swallowed release event.
    let unfocused = egui::RawInput {
        focused: false,
        ..Default::default()
    };
    assert!(!keys_in_use(key_state_frame(&ctx, unfocused)));
}

#[test]
fn joy_source_default_is_none() {
    assert_eq!(JoySource::default(), JoySource::None);
}

#[test]
fn keys_active_reflects_either_port() {
    let mut inputs = JoystickInputs::new(SharedGamepad::without_backend());
    assert!(!inputs.keys_active());
    inputs.sources[LEFT] = JoySource::Keys;
    assert!(inputs.keys_active());
}

#[test]
fn joystick_inputs_start_not_in_use() {
    let inputs = JoystickInputs::new(SharedGamepad::without_backend());
    assert_eq!(inputs.in_use, [false, false]);
}

#[test]
fn mouse_in_use_only_on_a_button() {
    assert!(!mouse_in_use(false, false));
    assert!(mouse_in_use(true, false));
    assert!(mouse_in_use(false, true));
}

/// The display rect shared by the fire-button gating tests, with room
/// around it standing in for the chrome.
const DISPLAY: egui::Rect =
    egui::Rect::from_min_max(egui::pos2(10.0, 10.0), egui::pos2(110.0, 90.0));

#[test]
fn press_began_on_display_gates_on_position() {
    let bg = egui::LayerId::background();
    // Press on the bare display (no egui area at the origin).
    assert!(press_began_on_display(
        egui::pos2(60.0, 50.0),
        None,
        DISPLAY,
        bg
    ));
    // Presses on the chrome (for example, the toolbar) never fire, wherever dragged afterward.
    assert!(!press_began_on_display(
        egui::pos2(60.0, 5.0),
        None,
        DISPLAY,
        bg
    ));
    // Presses below the display (for example, on the status bar) never fire.
    assert!(!press_began_on_display(
        egui::pos2(60.0, 95.0),
        None,
        DISPLAY,
        bg
    ));
    // `display_rect` starts as `Rect::NOTHING` until `draw_display` runs; must not fire against it.
    assert!(!press_began_on_display(
        egui::pos2(0.0, 0.0),
        None,
        egui::Rect::NOTHING,
        bg
    ));
}

#[test]
fn press_began_on_display_respects_overlays() {
    let inside = egui::pos2(60.0, 50.0);
    let overlay = egui::LayerId::new(egui::Order::Middle, egui::Id::new("menu-popup"));
    // Inside the display geometrically, but under a floating popup — must not fire.
    assert!(!press_began_on_display(
        inside,
        Some(overlay),
        DISPLAY,
        egui::LayerId::background()
    ));
    // Manager's embedded display lives inside its own `egui::Window` layer.
    assert!(press_began_on_display(
        inside,
        Some(overlay),
        DISPLAY,
        overlay
    ));
}

/// A pointer-button event for the latch tests.
fn button_event(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// One `update_mouse_fire` frame: run an egui pass with `events` and update
/// `inputs`' latches. A bare context has no areas, so the gate reduces to
/// its geometric half.
fn mouse_fire_frame(
    ctx: &egui::Context,
    inputs: &mut JoystickInputs,
    events: Vec<egui::Event>,
    primary_down: bool,
    secondary_down: bool,
) {
    let raw = egui::RawInput {
        events,
        ..Default::default()
    };
    // The pass's `FullOutput` (shapes, platform commands) is irrelevant here.
    let _ = ctx.run(raw, |ctx| {
        inputs.update_mouse_fire(
            ctx,
            DISPLAY,
            egui::LayerId::background(),
            primary_down,
            secondary_down,
        );
    });
}

#[test]
fn releasing_one_mouse_button_keeps_the_other_latched() {
    let ctx = egui::Context::default();
    let mut inputs = JoystickInputs::new(SharedGamepad::without_backend());
    let inside = egui::pos2(60.0, 50.0);
    use egui::PointerButton::{Primary, Secondary};
    // Hold primary on the display…
    let ev = vec![button_event(inside, Primary, true)];
    mouse_fire_frame(&ctx, &mut inputs, ev, true, false);
    assert_eq!(inputs.mouse_fire, [true, false]);
    // Press secondary too, then release it: primary must stay latched (egui's shared
    // press_origin isn't used).
    let ev = vec![button_event(inside, Secondary, true)];
    mouse_fire_frame(&ctx, &mut inputs, ev, true, true);
    assert_eq!(inputs.mouse_fire, [true, true]);
    let ev = vec![button_event(inside, Secondary, false)];
    mouse_fire_frame(&ctx, &mut inputs, ev, true, false);
    assert_eq!(inputs.mouse_fire, [true, false]);
}

#[test]
fn chrome_press_is_not_regated_by_a_later_display_press() {
    let ctx = egui::Context::default();
    let mut inputs = JoystickInputs::new(SharedGamepad::without_backend());
    use egui::PointerButton::{Primary, Secondary};
    // Primary pressed on the chrome above the display…
    let ev = vec![button_event(egui::pos2(60.0, 5.0), Primary, true)];
    mouse_fire_frame(&ctx, &mut inputs, ev, true, false);
    assert_eq!(inputs.mouse_fire, [false, false]);
    // Then secondary on the display while primary is held: each button keeps its own verdict.
    let ev = vec![button_event(egui::pos2(60.0, 50.0), Secondary, true)];
    mouse_fire_frame(&ctx, &mut inputs, ev, true, true);
    assert_eq!(inputs.mouse_fire, [false, true]);
}

#[test]
fn mouse_fire_latch_clears_if_release_event_never_arrives() {
    let ctx = egui::Context::default();
    let mut inputs = JoystickInputs::new(SharedGamepad::without_backend());
    let ev = vec![button_event(
        egui::pos2(60.0, 50.0),
        egui::PointerButton::Primary,
        true,
    )];
    mouse_fire_frame(&ctx, &mut inputs, ev, true, false);
    assert_eq!(inputs.mouse_fire, [true, false]);
    // Focus loss: the button state says "up" but no release event came.
    mouse_fire_frame(&ctx, &mut inputs, vec![], false, false);
    assert_eq!(inputs.mouse_fire, [false, false]);
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
