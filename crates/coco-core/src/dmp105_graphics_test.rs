use super::*;

fn feed(printer: &mut DMP105, bytes: &[u8]) {
    for &byte in bytes {
        printer.write_byte(byte);
    }
}

#[test]
fn graphics_spans_the_print_width_at_each_documented_density() {
    for (pitch, columns) in [
        (esc::PITCH_NORMAL, 480),
        (esc::PITCH_COMPRESSED, 576),
        (esc::PITCH_CONDENSED, 800),
    ] {
        let mut printer = DMP105::new();
        feed(
            &mut printer,
            &[control::ESC, pitch, control::SELECT_GRAPHICS],
        );
        for _ in 0..columns {
            printer.write_byte(0x81);
        }
        let dots = printer.paper().dots_in_range(0, 0);
        assert_eq!(dots.len(), columns);
        assert_eq!(printer.x, PRINT_WIDTH_X_UNITS);
        assert_eq!(
            dots.last().unwrap().0,
            PRINT_WIDTH_X_UNITS - PRINT_WIDTH_X_UNITS / columns as u32
        );
    }
}

#[test]
fn graphics_cr_uses_graphics_feed_and_respects_preselected_cr_only() {
    let mut printer = DMP105::new();
    feed(
        &mut printer,
        &[control::SELECT_GRAPHICS, 0x81, control::CR, 0xC0],
    );
    assert_eq!(
        printer.paper().dots_in_range(0, u32::MAX),
        vec![(0, 0), (0, GRAPHICS_LF_UNITS + 6 * DOT_ROW_UNITS)]
    );
    let mut cr_only = DMP105::new();
    feed(
        &mut cr_only,
        &[
            control::ESC,
            esc::NL_CR_ONLY,
            control::SELECT_GRAPHICS,
            0x81,
            control::CR,
            0xC0,
        ],
    );
    assert_eq!(cr_only.y, 0);
}

#[test]
fn ignored_graphics_escapes_do_not_change_later_text_settings() {
    let mut printer = DMP105::new();
    feed(
        &mut printer,
        &[
            control::SELECT_GRAPHICS,
            control::ESC,
            esc::PITCH_CONDENSED,
            control::ESC,
            esc::NL_CR_ONLY,
            control::ESC,
            esc::LF_PITCH_1_12,
            control::ESC,
            esc::BOLD_START,
            control::ESC,
            esc::DIRECTION,
            1,
            control::ESC,
            esc::FEED_LATCH,
            30,
            control::END_GRAPHICS,
        ],
    );
    assert_eq!(printer.pitch, Pitch::Normal);
    assert_eq!(printer.nl_mode, NlMode::CrLf);
    assert_eq!(printer.lf_pitch_units, LF_PITCH_1_6);
    assert_eq!(printer.direction, Direction::Bidirectional);
    assert!(!printer.bold);
}

#[test]
fn high_bit_newlines_are_controls_only_in_text() {
    let mut printer = DMP105::new();
    feed(&mut printer, &[control::LF_HIGH, control::CR_HIGH]);
    assert_eq!(printer.y, 2 * LF_PITCH_1_6);
    let y = printer.y;
    feed(
        &mut printer,
        &[control::SELECT_GRAPHICS, control::LF_HIGH, control::CR_HIGH],
    );
    assert_eq!(printer.y, y);
    assert_eq!(printer.paper().extent().dot_count, 5);
}

#[test]
fn manual_last_column_example_reaches_the_right_edge() {
    let mut printer = DMP105::new();
    feed(
        &mut printer,
        &[
            control::ESC,
            esc::PITCH_CONDENSED,
            control::SELECT_GRAPHICS,
            control::ESC,
            esc::POSITION,
            3,
            31,
            0x81,
        ],
    );
    assert_eq!(printer.paper().dots_in_range(0, 0), vec![(799 * 36, 0)]);
}

#[test]
fn repeating_a_mode_control_prints_invalid_glyphs_without_entering_graphics() {
    let mut printer = DMP105::new();
    feed(
        &mut printer,
        &[control::REPEAT, 2, control::SELECT_GRAPHICS],
    );
    assert_eq!(printer.mode, Mode::CharacterPrint);
    assert_eq!(printer.x, 2 * CELL_DOTS * Pitch::Normal.dot_spacing());
}

#[test]
fn manual_position_800_wraps_to_the_next_graphics_line() {
    let mut printer = DMP105::new();
    feed(
        &mut printer,
        &[
            control::ESC,
            esc::PITCH_CONDENSED,
            control::SELECT_GRAPHICS,
            control::ESC,
            esc::POSITION,
            3,
            32,
            0x81,
        ],
    );
    assert_eq!(
        printer.paper().dots_in_range(0, u32::MAX),
        vec![(0, GRAPHICS_LF_UNITS)]
    );
    let position = printer.x;
    feed(&mut printer, &[control::ESC, esc::POSITION, 0xFF, 0xFF]);
    assert_eq!(printer.x, position);
}
