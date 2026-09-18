use super::*;
use crate::hires_joystick::HiResInterface;

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
fn set_axis_round_trips_through_the_ten_bit_pot() {
    // Every 6-bit value must survive `set_axis` -> `pot() >> 4` unchanged, so a hi-res-free
    // port behaves exactly as it did before pots widened to 10 bits.
    let mut j = Joysticks::new();
    for v in 0..=AXIS_MAX {
        j.set_axis(RIGHT, AXIS_X, v);
        assert_eq!((j.pot(RIGHT, AXIS_X) >> 4) as u8, v);
    }
}

#[test]
fn compare_is_byte_identical_to_the_old_six_bit_formula() {
    // `compare` must agree with `dac <= v` for every (dac, pot) pair a 6-bit sweep can produce,
    // exactly like before pots widened to 10 bits (no hi-res interface installed).
    let mut j = Joysticks::new();
    for v in 0..=AXIS_MAX {
        j.set_axis(RIGHT, AXIS_X, v);
        for dac in 0..=AXIS_MAX {
            assert_eq!(j.compare(RIGHT, AXIS_X, dac), dac <= v, "v={v} dac={dac}");
        }
    }
}

#[test]
fn set_pot_clamps_to_ten_bits() {
    let mut j = Joysticks::new();
    j.set_pot(LEFT, AXIS_Y, u16::MAX);
    assert_eq!(j.pot(LEFT, AXIS_Y), POT_MAX);
    j.set_pot(LEFT, AXIS_Y, 0);
    assert_eq!(j.pot(LEFT, AXIS_Y), 0);
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

#[test]
fn set_hires_is_exclusive_to_one_port() {
    let mut j = Joysticks::new();
    j.set_hires(RIGHT, HiResInterface::Tandy);
    j.set_hires(LEFT, HiResInterface::Tandy);
    assert_eq!(
        j.hires(RIGHT),
        HiResInterface::None,
        "installing on LEFT clears RIGHT"
    );
    assert_eq!(j.hires(LEFT), HiResInterface::Tandy);
}

#[test]
fn fast_clock_credits_half_weight_cycles_with_carry() {
    let mut j = Joysticks::new();
    j.set_hires(RIGHT, HiResInterface::Tandy);
    j.observe_dac(0, RIGHT, AXIS_X); // was_low starts false: arms the one-shot.

    let duration = crate::hires_joystick::duration_cycles(HiResInterface::Tandy, POT_CENTER);
    // Two 1-raw-cycle fast ticks must credit exactly one slow-clock cycle (via the carry), not
    // zero (dropped every call) or two (rounded up every call). `duration - 1` credited pairs
    // land one slow cycle short of saturation, matching the exact boundary
    // `hires_joystick_test.rs`'s `expiry_boundary_is_exact` checks at the `HiResPort` level.
    for _ in 0..duration - 1 {
        j.tick(1, true);
        j.tick(1, true);
    }
    assert!(
        !j.compare(RIGHT, AXIS_X, 0),
        "must not saturate a cycle early"
    );
    j.tick(1, true);
    j.tick(1, true);
    assert!(
        j.compare(RIGHT, AXIS_X, 0),
        "must saturate exactly at the duration boundary"
    );
}

#[test]
fn observe_dac_and_compare_do_nothing_without_a_hires_interface() {
    let mut j = Joysticks::new();
    j.set_pot(RIGHT, AXIS_X, 1023);
    j.observe_dac(0, RIGHT, AXIS_X);
    j.tick(1_000_000, false);
    // Falls through to the stock formula: dac=0 <= pot>>4=63, so this is just the stock case.
    assert!(j.compare(RIGHT, AXIS_X, 0));
}
