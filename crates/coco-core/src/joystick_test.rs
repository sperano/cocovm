use super::*;

#[test]
fn comparator_is_high_while_dac_at_or_below_pot() {
    let mut j = Joysticks::new();
    j.set_axis(RIGHT, AXIS_X, 40);
    assert!(j.compare(RIGHT, AXIS_X, 0));
    assert!(j.compare(RIGHT, AXIS_X, 40));
    assert!(!j.compare(RIGHT, AXIS_X, 41));
}

#[test]
fn axes_clamp_to_six_bits() {
    let mut j = Joysticks::new();
    j.set_axis(LEFT, AXIS_Y, 200);
    assert!(j.compare(LEFT, AXIS_Y, AXIS_MAX));
}

#[test]
fn button_rows_match_seb_wiring() {
    let mut j = Joysticks::new();
    j.set_button(RIGHT, 0, true);
    assert_eq!(j.button_rows(), 0x01);
    j.set_button(LEFT, 0, true);
    j.set_button(RIGHT, 1, true);
    j.set_button(LEFT, 1, true);
    assert_eq!(j.button_rows(), 0x0F);
}
