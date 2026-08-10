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
        Some((AXIS_MIN, 32))
    );
    // Center of the active area -> center on both axes.
    assert_eq!(
        pot_axes_from_pointer(egui::pos2(60.0, 50.0), ACTIVE),
        Some((32, 32))
    );
    // Bottom-right corner -> full deflection on both axes.
    assert_eq!(
        pot_axes_from_pointer(egui::pos2(100.0, 80.0), ACTIVE),
        Some((AXIS_MAX, AXIS_MAX))
    );
}

#[test]
fn pot_axes_from_pointer_clamps_a_pointer_in_the_border() {
    // Left of the active area (but still geometrically inside a larger
    // display rect, i.e. in the border) pins the X axis at full-left rather
    // than going negative.
    assert_eq!(
        pot_axes_from_pointer(egui::pos2(0.0, 50.0), ACTIVE),
        Some((AXIS_MIN, 32))
    );
    // Past the bottom-right corner pins both axes at full deflection rather
    // than overshooting.
    assert_eq!(
        pot_axes_from_pointer(egui::pos2(200.0, 200.0), ACTIVE),
        Some((AXIS_MAX, AXIS_MAX))
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
    // Press on the chrome (above the display, e.g. the menu bar) — never
    // fires, wherever the pointer is dragged afterwards.
    assert!(!press_began_on_display(
        egui::pos2(60.0, 5.0),
        None,
        DISPLAY,
        bg
    ));
    // Press below the display (e.g. the status bar).
    assert!(!press_began_on_display(
        egui::pos2(60.0, 95.0),
        None,
        DISPLAY,
        bg
    ));
    // `display_rect` starts as `Rect::NOTHING` until `draw_display` runs;
    // a press must not fire against that placeholder.
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
    // Geometrically inside the display, but the press landed on a popup
    // floating above it — must not double as a fire-button press.
    assert!(!press_began_on_display(
        inside,
        Some(overlay),
        DISPLAY,
        egui::LayerId::background()
    ));
    // Manager's embedded fallback: the display lives inside an
    // `egui::Window`, so the layer at the origin is the display's own.
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

/// One `update_mouse_fire` frame: run an egui pass on `ctx` with `events`
/// and update `inputs`' latches against [`DISPLAY`], with the given
/// held-button state. A bare context has no areas, so `layer_id_at` is
/// `None` everywhere and the gate reduces to its geometric half.
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
    let mut inputs = JoystickInputs::new();
    let inside = egui::pos2(60.0, 50.0);
    use egui::PointerButton::{Primary, Secondary};
    // Hold primary on the display…
    let ev = vec![button_event(inside, Primary, true)];
    mouse_fire_frame(&ctx, &mut inputs, ev, true, false);
    assert_eq!(inputs.mouse_fire, [true, false]);
    // …press secondary too, then release it: primary must stay latched.
    // (egui's shared `press_origin` is cleared by ANY release — the reason
    // the latches are driven from per-button events instead.)
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
    let mut inputs = JoystickInputs::new();
    use egui::PointerButton::{Primary, Secondary};
    // Primary pressed on the chrome above the display…
    let ev = vec![button_event(egui::pos2(60.0, 5.0), Primary, true)];
    mouse_fire_frame(&ctx, &mut inputs, ev, true, false);
    assert_eq!(inputs.mouse_fire, [false, false]);
    // …then secondary on the display while primary is still held: each
    // button keeps the verdict of its own press.
    let ev = vec![button_event(egui::pos2(60.0, 50.0), Secondary, true)];
    mouse_fire_frame(&ctx, &mut inputs, ev, true, true);
    assert_eq!(inputs.mouse_fire, [false, true]);
}

#[test]
fn mouse_fire_latch_clears_if_release_event_never_arrives() {
    let ctx = egui::Context::default();
    let mut inputs = JoystickInputs::new();
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
