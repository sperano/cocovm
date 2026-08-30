use super::*;

#[test]
fn stick_index_maps_to_the_coco_core_joystick_ports() {
    assert_eq!(Stick::Left.index(), coco_core::joystick::LEFT);
    assert_eq!(Stick::Right.index(), coco_core::joystick::RIGHT);
}

#[test]
fn vm_status_as_str_is_the_snake_case_wire_spelling() {
    assert_eq!(VmStatus::Running.as_str(), "running");
    assert_eq!(VmStatus::Suspended.as_str(), "suspended");
    assert_eq!(VmStatus::PoweredOff.as_str(), "powered_off");
}
