use super::*;
fn send(printer: &mut DMP130, bytes: &[u8]) {
    for &byte in bytes {
        printer.write_byte(byte);
    }
}
fn dots(printer: &DMP130) -> Vec<(u32, u32)> {
    printer.paper().dots_in_range(0, u32::MAX)
}
#[test]
fn graphics_has_seven_pins_480_columns_and_exact_feed() {
    let mut printer = DMP130::new();
    send(&mut printer, &[0x12, 0x81, 0xc0, 0xff, 0x0a, 0x81]);
    let step = X_UNITS_PER_INCH / 60;
    let pin = Y_UNITS_PER_INCH / 72;
    let marks = dots(&printer);
    assert_eq!(marks.len(), 10);
    assert!(marks.contains(&(0, 0)));
    assert!(marks.contains(&(step, 6 * pin)));
    assert!(marks.contains(&(3 * step, 7 * pin)));
    send(&mut printer, &[0x1b, 0x10, 1, 224, 0x81]);
    assert!(dots(&printer).contains(&(0, 14 * pin)));
}
#[test]
fn graphics_restores_wp_and_style_and_ignores_backspace_operand() {
    let mut printer = DMP130::new();
    send(&mut printer, &[0x14, 0x0f, 0x12, 0x08, 0x81, 0x1e]);
    assert_eq!(printer.mode, Mode::WordProcessing);
    assert!(printer.style.underline);
    assert_eq!(dots(&printer), vec![(0, 0)]);
}
#[test]
fn wp_feeds_immediately_while_dp_latches_and_micro_halves_pitch() {
    let mut printer = DMP130::new();
    send(&mut printer, &[0x1b, 0x1c]);
    assert_eq!(printer.y, 0);
    send(&mut printer, &[0x0a]);
    assert_eq!(printer.y, FULL_FEED as u32 / 2);
    send(&mut printer, &[0x14, 0x1b, 0x1c, 0x1b, b'M', 0x0a]);
    assert_eq!(printer.y, FULL_FEED as u32 * 3 / 2);
}
#[test]
fn precise_feeds_do_not_return_carriage() {
    let mut printer = DMP130::new();
    send(&mut printer, b"A\x1b\x1a\x1b2\x1b3\x1b9\x1b@\x02");
    assert_eq!(printer.x, X_INCH / 10);
    assert_eq!(
        printer.y,
        Y_UNITS_PER_INCH / 48
            + Y_UNITS_PER_INCH / 72
            + Y_UNITS_PER_INCH / 216
            + 3 * Y_UNITS_PER_INCH / 144
    );
}
#[test]
fn ibm_graphics_payload_is_counted_and_all_bytes_are_data() {
    let mut printer = DMP130::new();
    send(&mut printer, b"\x1b!\x1bK\x04\x00\x80\x01\x1b\x0d");
    assert_eq!(printer.grammar, Grammar::Ibm);
    assert_eq!(printer.y, 0);
    assert!(matches!(printer.pending, Pending::None));
    assert!(dots(&printer).contains(&(0, 0)));
    assert!(dots(&printer).contains(&(X_UNITS_PER_INCH / 60, 7 * Y_UNITS_PER_INCH / 72)));
}
#[test]
fn cancel_discards_only_unprinted_ink() {
    let mut printer = DMP130::new();
    send(&mut printer, b"A\r\x1b!B\x18C\r");
    let mut expected = DMP130::new();
    send(&mut expected, b"A\r\x1b!C\r");
    assert_eq!(dots(&printer), dots(&expected));
}
#[test]
fn partial_graphics_survives_snapshot() {
    let mut printer = DMP130::new();
    send(&mut printer, b"\x1b!\x1bZ\x03\x00\x80");
    let json = serde_json::to_string(&printer).unwrap();
    let mut restored: DMP130 = serde_json::from_str(&json).unwrap();
    send(&mut printer, b"\x01\xff");
    send(&mut restored, b"\x01\xff");
    assert_eq!(dots(&printer), dots(&restored));
}
#[test]
fn reset_keeps_paper_and_clears_parser() {
    let mut printer = DMP130::new();
    send(&mut printer, b"A\r\x1b\x10\x01");
    let before = dots(&printer);
    printer.reset();
    assert_eq!(dots(&printer), before);
    assert!(matches!(printer.pending, Pending::None));
    assert_eq!(printer.x, 0);
}
#[test]
fn prefixed_cancel_and_high_bit_control_aliases_preserve_cancel() {
    let mut printer = DMP130::new();
    send(&mut printer, b"\x1b!A\x1b\x98B\x8d");
    let mut expected = DMP130::new();
    send(&mut expected, b"\x1b!B\r");
    assert_eq!(dots(&printer), dots(&expected));
}
#[test]
fn condensed_priority_suspends_tandy_bold_and_uses_all_137_cells() {
    let mut printer = DMP130::new();
    send(&mut printer, &[0x1b, 0x1f, 0x1b, 0x14]);
    for _ in 0..137 {
        printer.write_byte(b' ');
    }
    assert_eq!(printer.x, 0);
    assert_eq!(printer.y, FULL_FEED as u32);
    printer.write_byte(b'A');
    assert_eq!(printer.y, FULL_FEED as u32);
    assert_eq!(printer.style.pitch, Pitch::Condensed);
    assert!(printer.style.bold);
}
#[test]
fn physical_margins_survive_font_change_and_bound_wrap() {
    let mut printer = DMP130::new();
    send(&mut printer, b"\x1bQ\x0a\x1bR\x14\x1b\x17");
    assert_eq!((printer.left, printer.right), (X_INCH, 2 * X_INCH));
    for _ in 0..13 {
        printer.write_byte(b' ');
    }
    assert_eq!(printer.y, FULL_FEED as u32);
    assert_eq!(printer.x, X_INCH + X_INCH / 12);
}
#[test]
fn ibm_staged_feed_and_persistent_width_have_independent_lifetimes() {
    let mut printer = DMP130::new();
    send(&mut printer, b"\x1b!\x1bA\x03\n");
    assert_eq!(printer.y, FULL_FEED as u32);
    send(&mut printer, b"\x1b2\x1bW\x01\x0e\n");
    assert_eq!(printer.y, FULL_FEED as u32 + 3 * Y_UNITS_PER_INCH / 72);
    assert!(printer.style.wide);
    assert!(!printer.style.transient_wide);
}
#[test]
fn tabs_use_physical_positions_and_underline_includes_tab_span() {
    let mut printer = DMP130::new();
    send(&mut printer, b"\x1b!\x1bD\x04\x08\x00\x1b-\x01\t\r");
    let underline_y = 9 * Y_UNITS_PER_INCH / 72;
    let marks = printer.paper().dots_in_range(underline_y, underline_y);
    assert_eq!(marks.len(), 4 * 12);
    assert_eq!(marks.first(), Some(&(0, underline_y)));
}
#[test]
fn forms_skip_perforation_and_reverse_feed_stops_at_roll_start() {
    let mut printer = DMP130::new();
    send(&mut printer, b"\x1b!\x1bC\x04\x1bN\x01\n\n\n");
    assert_eq!(printer.y, 4 * FULL_FEED as u32);
    send(&mut printer, b"\x0c");
    assert_eq!(printer.y, 8 * FULL_FEED as u32);
    printer.reset();
    send(&mut printer, b"\x1b\x0a\n");
    assert_eq!(printer.y, 0);
}
#[test]
fn partial_repeat_operands_and_ink_survive_snapshot() {
    let mut printer = DMP130::new();
    send(&mut printer, b"A\x1c\x03");
    let mut restored: DMP130 =
        serde_json::from_str(&serde_json::to_string(&printer).unwrap()).unwrap();
    send(&mut printer, b"B\r");
    send(&mut restored, b"B\r");
    assert_eq!(dots(&printer), dots(&restored));
}
#[test]
fn repeat_function_codes_do_not_execute_commands() {
    let mut printer = DMP130::new();
    send(&mut printer, &[0x1c, 10, 0x0a, 0x12, 0x1c, 10, 0x1e, 0x81]);
    assert_eq!(printer.y, 0);
    assert_eq!(printer.mode, Mode::Graphics);
    assert_eq!(dots(&printer), vec![(0, 0)]);
}
#[test]
fn full_text_buffer_prints_immediately_and_respects_cr_only() {
    let mut printer = DMP130::new();
    send(&mut printer, b"\x1b\x15");
    for _ in 0..80 {
        printer.write_byte(b'A');
    }
    assert!(printer.paper().extent().dot_count > 0);
    assert_eq!((printer.x, printer.y), (0, 0));
    printer.write_byte(b'B');
    assert_eq!((printer.x, printer.y), (X_INCH / 10, 0));
}
#[test]
fn backspace_flushes_before_cancel_in_all_ibm_encodings() {
    for backspace in [
        &b"\x08"[..],
        &b"\x88"[..],
        &b"\x1b\x08"[..],
        &b"\x1b\x88"[..],
    ] {
        let mut printer = DMP130::new();
        send(&mut printer, b"\x1b!A");
        send(&mut printer, backspace);
        printer.write_byte(0x18);
        assert!(printer.paper().extent().dot_count > 0);
    }
}
#[test]
fn verified_extended_and_country_symbols_have_distinct_impressions() {
    let mut accented = DMP130::new();
    send(&mut accented, b"\xb6\r");
    let mut plain = DMP130::new();
    send(&mut plain, b"a\r");
    assert_ne!(dots(&accented), dots(&plain));
    let mut german = DMP130::new();
    send(&mut german, b"\x1bY\x21{\r");
    assert_eq!(dots(&accented), dots(&german));
    let mut spanish = DMP130::new();
    send(&mut spanish, b"\x1bY\x28\\\r");
    let mut extended_n_tilde = DMP130::new();
    send(&mut extended_n_tilde, b"\xd6\r");
    assert_eq!(dots(&spanish), dots(&extended_n_tilde));
    let mut acute = DMP130::new();
    send(&mut acute, b"\xc8\r");
    assert_ne!(dots(&acute), dots(&accented));
}
#[test]
fn block_graphics_join_across_cells() {
    const POSITIONS_PER_CELL: usize = 12;
    fn even_bar(printer: &DMP130, cells: usize) {
        let xs: std::collections::BTreeSet<u32> = dots(printer).iter().map(|&(x, _)| x).collect();
        assert_eq!(xs.len(), cells * POSITIONS_PER_CELL);
        let xs: Vec<u32> = xs.into_iter().collect();
        let gaps: Vec<u32> = xs.windows(2).map(|pair| pair[1] - pair[0]).collect();
        let (min, max) = (gaps.iter().min().unwrap(), gaps.iter().max().unwrap());
        assert!(max - min <= 1, "uneven block spacing: {gaps:?}");
        let ys: std::collections::BTreeSet<u32> = dots(printer).iter().map(|&(_, y)| y).collect();
        assert_eq!(ys.len(), 1);
    }
    let mut pica = DMP130::new();
    send(&mut pica, b"\xf1\xf1\r");
    even_bar(&pica, 2);
    const BAR_ROW: u32 = 2;
    assert_eq!(dots(&pica)[0], (0, BAR_ROW * Y_UNITS_PER_INCH / 72));
    // Condensed cells are 14 dots wide, not a multiple of 6.
    let mut condensed = DMP130::new();
    send(&mut condensed, b"\x1b\x14\xf1\xf1\r");
    even_bar(&condensed, 2);
    let mut wide = DMP130::new();
    send(&mut wide, b"\x1b\x0e\xf1\r");
    even_bar(&wide, 1);
    assert_eq!(dots(&wide).len(), dots(&pica).len() / 2);
    let mut ibm = DMP130::new();
    send(&mut ibm, b"\x1b:\xf1\r");
    assert_ne!(dots(&ibm), dots(&pica));
}
#[test]
fn proportional_metrics_use_manual_widths() {
    let mut printer = DMP130::new();
    send(&mut printer, b"\x1b\x11IM");
    const NLQ_I_DOTS: u64 = 18;
    const NLQ_M_DOTS: u64 = 24;
    assert_eq!(printer.x, (NLQ_I_DOTS + NLQ_M_DOTS) * X_INCH / 240);
}
#[test]
fn graphics_width_changes_preserve_text_styles_across_snapshot() {
    let mut printer = DMP130::new();
    send(&mut printer, b"\x0f\x1b\x1f\x1b\x0e\x12\x1b\x0f\xff");
    let mut restored: DMP130 =
        serde_json::from_str(&serde_json::to_string(&printer).unwrap()).unwrap();
    assert!(!restored.graphics_wide);
    send(&mut restored, b"\x1e");
    assert!(restored.style.wide);
    assert!(restored.style.bold);
    assert!(restored.style.underline);
    assert_eq!(restored.mode, Mode::DataProcessing);
}
#[test]
fn dot_spacing_overflow_begins_at_next_line() {
    let mut printer = DMP130::new();
    send(&mut printer, b"\x1b\x10\x01\xdf\x1b\x03");
    assert_eq!(printer.y, FULL_FEED as u32);
    assert_eq!(printer.x, 3 * X_INCH / 120);
}
