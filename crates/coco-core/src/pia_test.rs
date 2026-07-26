use super::*;

#[test]
fn c2_output_follows_set_reset_level() {
    let mut pia = MC6821::new();
    // The ROM's standard idle control value: C2 set/reset output, level 0.
    pia.write(1, 0x34);
    assert!(!pia.a.c2_output());
    // Level bit raised (e.g. selecting the other joystick mux input).
    pia.write(1, 0x3C);
    assert!(pia.a.c2_output());
}

#[test]
fn c2_flags_are_not_writable() {
    let mut pia = MC6821::new();
    pia.write(1, 0xFF);
    assert_eq!(pia.a.control & (cr::C1_FLAG | cr::C2_FLAG), 0);
}

#[test]
fn falling_edge_selected_flags_only_on_high_to_low() {
    let mut port = PiaPort::default(); // idle high, control=0 -> falling edge selected
    assert_eq!(port.control & cr::C1_EDGE_HIGH, 0);
    port.set_c1(true); // still high: no transition
    assert_eq!(port.control & cr::C1_FLAG, 0);
    port.set_c1(false); // high -> low: matches the selected edge
    assert_ne!(port.control & cr::C1_FLAG, 0);
}

#[test]
fn rising_edge_selected_flags_only_on_low_to_high() {
    let mut port = PiaPort::default();
    port.control |= cr::C1_EDGE_HIGH; // select low->high
    port.set_c1(false); // high -> low: not the selected edge
    assert_eq!(port.control & cr::C1_FLAG, 0);
    port.set_c1(false); // still low: no transition
    assert_eq!(port.control & cr::C1_FLAG, 0);
    port.set_c1(true); // low -> high: matches the selected edge
    assert_ne!(port.control & cr::C1_FLAG, 0);
}

#[test]
fn repeated_level_never_flags() {
    let mut port = PiaPort::default();
    port.control |= cr::C1_EDGE_HIGH; // rising edge selected
    port.set_c1(true); // still high: no transition, no flag even though level matches
    assert_eq!(port.control & cr::C1_FLAG, 0);
}
