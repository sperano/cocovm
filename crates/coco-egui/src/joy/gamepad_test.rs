use super::*;

const TEST_AXIS_X: f32 = 0.75;
const TEST_AXIS_Y: f32 = -0.5;

#[test]
fn cloned_handles_retain_one_shared_input_snapshot() {
    let first = SharedGamepad::without_backend();
    let second = first.clone();
    let expected = GamepadState {
        axes: [TEST_AXIS_X, TEST_AXIS_Y],
        buttons: [true, false],
    };

    first.set_test_state(expected);

    assert_eq!(second.poll(), expected);
}

#[test]
fn unavailable_backend_retains_centered_released_state() {
    let gamepad = SharedGamepad::without_backend();

    assert!(!gamepad.available());
    assert_eq!(gamepad.poll(), GamepadState::default());
}

#[test]
fn manager_drain_retains_input_for_later_vm_poll() {
    let manager = SharedGamepad::service_test_backend();
    let vm = manager.clone();
    let expected = GamepadState {
        axes: [TEST_AXIS_X, TEST_AXIS_Y],
        buttons: [false, true],
    };
    manager.queue_test_state(expected);

    assert!(manager.service());

    assert_eq!(vm.poll(), expected);
    assert_eq!(manager.service_count(), 2);
}
