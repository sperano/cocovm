use super::*;

const Y_REG: u16 = COCOMAX_IO_BASE; // $FF90
const X_REG: u16 = COCOMAX_IO_BASE + 1; // $FF91
const LEFT_BUTTON_REG: u16 = COCOMAX_IO_BASE + 2; // $FF92
const RIGHT_BUTTON_REG: u16 = COCOMAX_IO_BASE + 3; // $FF93
const UNCONNECTED_REG: u16 = COCOMAX_IO_BASE + 4; // $FF94 (channel 4)

#[test]
fn power_on_axes_are_centered_and_result_is_zero() {
    let m = CoCoMaxModule::new();
    assert_eq!(m.x, 0x80);
    assert_eq!(m.y, 0x80);
    assert_eq!(m.upper_io_peek(Y_REG), 0);
}

#[test]
fn a_read_returns_the_previous_conversion_then_starts_a_new_one() {
    let mut m = CoCoMaxModule::new();
    m.set_position(0x11, 0x22);

    // First read after a change returns the stale (power-on) result...
    assert_eq!(m.upper_io_read(X_REG), 0);
    // ...and the second read sees the conversion the first read started.
    assert_eq!(m.upper_io_read(X_REG), 0x11);
}

#[test]
fn channel_select_reads_the_right_axis() {
    let mut m = CoCoMaxModule::new();
    m.set_position(0x11, 0x22);

    m.upper_io_read(X_REG); // starts a conversion on X
    assert_eq!(m.upper_io_read(X_REG), 0x11, "channel 1 is X");

    m.upper_io_read(Y_REG); // starts a conversion on Y
    assert_eq!(m.upper_io_read(Y_REG), 0x22, "channel 0 is Y");
}

#[test]
fn button_channels_report_active_low_polarity() {
    let mut m = CoCoMaxModule::new();
    m.set_buttons(true, false);

    m.upper_io_read(LEFT_BUTTON_REG);
    assert_eq!(m.upper_io_read(LEFT_BUTTON_REG), 0x00, "left pressed");

    m.upper_io_read(RIGHT_BUTTON_REG);
    assert_eq!(m.upper_io_read(RIGHT_BUTTON_REG), 0xFF, "right released");

    m.set_buttons(false, true);
    m.upper_io_read(LEFT_BUTTON_REG);
    assert_eq!(m.upper_io_read(LEFT_BUTTON_REG), 0xFF, "left released");
    m.upper_io_read(RIGHT_BUTTON_REG);
    assert_eq!(m.upper_io_read(RIGHT_BUTTON_REG), 0x00, "right pressed");
}

#[test]
fn not_connected_channels_leave_the_result_unchanged() {
    let mut m = CoCoMaxModule::new();
    m.set_position(0x11, 0x22);
    m.upper_io_read(X_REG); // latches 0x11 as the pending result
    assert_eq!(
        m.upper_io_read(UNCONNECTED_REG),
        0x11,
        "prior result reported"
    );
    assert_eq!(
        m.upper_io_read(UNCONNECTED_REG),
        0x11,
        "channel 4 started no conversion, so the same result reads back again"
    );
}

#[test]
fn writes_are_ignored() {
    let mut m = CoCoMaxModule::new();
    m.write(X_REG, 0xAA);
    assert_eq!(m.x, 0x80, "no write path into the axis state");
}

#[test]
fn peek_has_no_side_effects() {
    let mut m = CoCoMaxModule::new();
    m.set_position(0x11, 0x22);
    m.upper_io_read(X_REG); // starts a conversion, latches nothing yet

    let before = m.upper_io_peek(X_REG);
    let after = m.upper_io_peek(X_REG);
    assert_eq!(before, after, "peek does not start a conversion");
    // A real read still sees the conversion the earlier `upper_io_read` started.
    assert_eq!(m.upper_io_read(X_REG), 0x11);
}

#[test]
fn reset_clears_the_latched_result_but_not_the_pointer_state() {
    let mut m = CoCoMaxModule::new();
    m.set_position(0x11, 0x22);
    m.set_buttons(true, true);
    m.upper_io_read(X_REG);
    m.upper_io_read(X_REG); // result is now 0x11

    m.reset();
    assert_eq!(m.upper_io_peek(X_REG), 0, "result clears on reset");
    // Axes/buttons survive the reset, like a real pointer's position.
    assert_eq!((m.x, m.y), (0x11, 0x22));
    assert_eq!(m.buttons, [true, true]);
}
