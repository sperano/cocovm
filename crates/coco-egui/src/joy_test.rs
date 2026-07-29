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
    assert!(keys_in_use(KeyState { button1: true, ..idle }));
}
