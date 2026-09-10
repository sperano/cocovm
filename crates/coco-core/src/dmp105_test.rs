use super::*;
use crate::dmp105_font;

/// Render `paper`'s dots in `[y0, y1)` as a compact debug string: one line
/// per row, `#`/`.` per dot column — a debugging aid, not a fixture format.
#[allow(dead_code)]
fn ascii_art(paper: &Paper, x0: u32, x1: u32, y0: u32, y1: u32, step: u32) -> String {
    let mut out = String::new();
    for y in (y0..y1).step_by(step as usize) {
        let dots = paper.dots_in_range(y, y);
        let mut x = x0;
        while x < x1 {
            out.push(if dots.iter().any(|&(dx, _)| dx == x) {
                '#'
            } else {
                '.'
            });
            x += step;
        }
        out.push('\n');
    }
    out
}

fn feed_str(dmp: &mut DMP105, bytes: &[u8]) {
    for &b in bytes {
        dmp.feed(b);
    }
}

/// A blank glyph advances `x` by the 12-dot cell without marking any dots.
fn normal_cell_width() -> u32 {
    CELL_DOTS * Pitch::Normal.dot_spacing()
}

/// Regression: `1C 1C 1C` must not stack-overflow — inside a repeat, `c` is
/// data, printed as the undefined-code `X` placeholder `n` times.
#[test]
fn repeating_the_repeat_introducer_terminates_and_prints_placeholders() {
    let mut dmp = DMP105::default();
    feed_str(&mut dmp, &[control::REPEAT, 3, control::REPEAT]);
    assert_eq!(dmp.x, 3 * normal_cell_width());
    assert!(dmp.paper.extent().dot_count > 0); // three X glyphs
    // The machine is idle again: a normal byte prints in cell 4.
    feed_str(&mut dmp, b"A");
    assert_eq!(dmp.x, 4 * normal_cell_width());
}

/// Repeating ESC is equally data inside a repeat: undefined CP code,
/// no escape sequence starts, no panic.
#[test]
fn repeating_the_escape_introducer_terminates() {
    let mut dmp = DMP105::default();
    feed_str(&mut dmp, &[control::REPEAT, 2, control::ESC]);
    assert_eq!(dmp.x, 2 * normal_cell_width());
    // Not left in Pending::Esc: a following 'Z' is a glyph, not a selector byte.
    feed_str(&mut dmp, b"Z");
    assert_eq!(dmp.x, 3 * normal_cell_width());
}

/// Head-position arithmetic saturates instead of overflowing, so the
/// interpreter survives arbitrary input volumes.
#[test]
fn head_position_saturates_at_extremes() {
    let mut dmp = DMP105 {
        y: u32::MAX - 2,
        ..Default::default()
    };
    feed_str(&mut dmp, &[control::LF, control::CR, control::LF]);
    assert_eq!(dmp.y, u32::MAX);
    dmp.x = u32::MAX - 2;
    feed_str(&mut dmp, b"A"); // glyph plot + cell advance, all saturating
    assert_eq!(dmp.x, u32::MAX);
}

/// Ink past the physical 8" print zone is dropped, so an unbounded CR-less
/// stream can't grow the paper model without limit.
#[test]
fn marks_beyond_print_width_are_dropped() {
    let mut dmp = DMP105 {
        x: PRINT_WIDTH_X_UNITS,
        ..Default::default()
    };
    feed_str(&mut dmp, b"A");
    assert_eq!(dmp.paper.extent().dot_count, 0);
}

#[test]
fn hello_cr_at_normal_pitch_produces_expected_glyph_columns_and_row() {
    let mut dmp = DMP105::new();
    feed_str(&mut dmp, b"HELLO\r");

    // 'H's left stroke is solid across all 7 body rows; check it lands at x=0, y=0.
    let cell = normal_cell_width();
    let col1_dots = dmp.paper.dots_in_range(0, DESCENDER_ROW);
    let h_col1: Vec<u32> = col1_dots
        .iter()
        .filter(|&&(x, _)| x == Pitch::Normal.dot_spacing())
        .map(|&(_, y)| y)
        .collect();
    assert_eq!(
        h_col1,
        (0..7).map(|row| row * DOT_ROW_UNITS).collect::<Vec<_>>(),
        "H's left stroke column"
    );

    // Five glyph cells wide (H, E, L, L, O); check ink within O's cell and
    // none immediately past it.
    let fifth_cell_dots = dmp.paper.dots_in_range(0, DESCENDER_ROW);
    assert!(
        fifth_cell_dots
            .iter()
            .any(|&(x, _)| (4 * cell..5 * cell).contains(&x)),
        "expected some ink within O's cell"
    );
    assert!(
        !fifth_cell_dots.iter().any(|&(x, _)| x == 5 * cell + 1),
        "no ink expected one dot past the fifth cell"
    );

    // CR (default NL mode CR+LF) returns x to 0 and feeds one line at the default 1/6" pitch.
    assert_eq!(dmp.x, 0);
    assert_eq!(dmp.y, LF_PITCH_1_6);
}

#[test]
fn cr_only_mode_does_not_advance_y() {
    let mut dmp = DMP105::new();
    feed_str(&mut dmp, b"\x1B\x15"); // ESC 15: CR = CR only
    feed_str(&mut dmp, b"A\r");
    assert_eq!(dmp.x, 0);
    assert_eq!(dmp.y, 0, "CR-only mode must not feed a line");
}

#[test]
fn cr_lf_mode_advances_y_by_latched_pitch() {
    let mut dmp = DMP105::new();
    feed_str(&mut dmp, b"\x1B\x16"); // ESC 16: CR = CR+LF (also the default)
    feed_str(&mut dmp, b"A\r");
    assert_eq!(dmp.y, LF_PITCH_1_6);
}

#[test]
fn plain_lf_advances_y_without_touching_x() {
    let mut dmp = DMP105::new();
    feed_str(&mut dmp, b"AB\n");
    assert_eq!(dmp.y, LF_PITCH_1_6);
    assert_eq!(dmp.x, 2 * normal_cell_width(), "LF alone must not reset x");
}

#[test]
fn pitch_change_mid_line_changes_subsequent_cell_width() {
    let mut dmp = DMP105::new();
    feed_str(&mut dmp, b"A"); // at Normal pitch
    let after_a = dmp.x;
    assert_eq!(after_a, normal_cell_width());

    feed_str(&mut dmp, b"\x1B\x14"); // ESC 14: Condensed 16.7 CPI
    feed_str(&mut dmp, b"B");
    let condensed_width = CELL_DOTS * Pitch::Condensed.dot_spacing();
    assert_eq!(dmp.x, after_a + condensed_width);
    assert_ne!(
        condensed_width,
        normal_cell_width(),
        "condensed cell must differ from normal cell for this test to mean anything"
    );
}

#[test]
fn elongation_doubles_dot_spacing_and_cell_advance() {
    let mut normal = DMP105::new();
    feed_str(&mut normal, b"A");
    let normal_advance = normal.x;

    let mut elongated = DMP105::new();
    feed_str(&mut elongated, b"\x1B\x0E"); // ESC 0E: start elongation
    feed_str(&mut elongated, b"A");
    assert_eq!(elongated.x, normal_advance * 2);

    // Find the first row-0 dot column from the font data and check it
    // landed at twice the normal spacing.
    let dot = Pitch::Normal.dot_spacing();
    let glyph = dmp105_font::ascii_glyph(b'A').unwrap();
    let first_row0_col = glyph
        .iter()
        .position(|&bits| bits & 1 != 0)
        .expect("'A' must have at least one row-0 dot") as u32;
    let dots = elongated.paper.dots_in_range(0, 0);
    assert!(
        dots.iter().any(|&(x, _)| x == first_row0_col * dot * 2),
        "expected 'A's row-0 dot at twice the normal dot spacing when elongated"
    );
}

#[test]
fn underline_marks_full_cell_width_on_descender_row() {
    let mut dmp = DMP105::new();
    feed_str(&mut dmp, b"\x0F"); // start underline
    feed_str(&mut dmp, b"A");
    let dot = Pitch::Normal.dot_spacing();
    let width = CELL_DOTS * dot;
    let rule_dots: Vec<u32> = dmp
        .paper
        .dots_in_range(DESCENDER_ROW, DESCENDER_ROW)
        .into_iter()
        .map(|(x, _)| x)
        .collect();
    // A solid rule stepped at `dot` covers x=0 and the last in-range
    // multiple of `dot` before `width`.
    assert!(rule_dots.contains(&0));
    let last_step = ((width - 1) / dot) * dot;
    assert!(rule_dots.contains(&last_step));
    feed_str(&mut dmp, b"\x0E"); // end underline
    feed_str(&mut dmp, b"B");
    // 'B' cell must NOT get a descender-row rule now that underline is off.
    let second_cell_start = CELL_DOTS * dot;
    let second_cell_end = second_cell_start + CELL_DOTS * dot;
    let rule_in_b_cell = dmp
        .paper
        .dots_in_range(DESCENDER_ROW, DESCENDER_ROW)
        .into_iter()
        .filter(|&(x, _)| (second_cell_start..second_cell_end).contains(&x))
        .count();
    assert_eq!(rule_in_b_cell, 0);
}

#[test]
fn bold_strikes_every_column_twice_one_dot_over() {
    let mut plain = DMP105::new();
    feed_str(&mut plain, b"H");
    let plain_dots = plain.paper.dots_in_range(0, DESCENDER_ROW).len();

    let mut bold = DMP105::new();
    feed_str(&mut bold, b"\x1B\x1F"); // ESC 1F: start bold
    feed_str(&mut bold, b"H");
    let bold_dots = bold.paper.dots_in_range(0, DESCENDER_ROW).len();

    assert_eq!(
        bold_dots,
        plain_dots * 2,
        "bold must double every dot via its one-column-over second pass"
    );

    // Cell advance itself is unaffected by bold (only elongation changes cell width).
    assert_eq!(bold.x, plain.x);
}

#[test]
fn repeat_code_repeats_a_character_n_times_in_cp_mode() {
    let mut dmp = DMP105::new();
    feed_str(&mut dmp, b"\x1C\x03A"); // repeat 'A' 3 times
    assert_eq!(dmp.x, 3 * normal_cell_width());
}

#[test]
fn repeat_code_in_graphics_mode_requires_msb_set_on_c() {
    let mut dmp = DMP105::new();
    dmp.feed(control::SELECT_GRAPHICS);

    // c without MSB set: per spec, not honored at all in Graphics mode.
    feed_str(&mut dmp, b"\x1C\x05\x41"); // n=5, c=0x41 (no MSB)
    assert_eq!(
        dmp.x, 0,
        "repeat with MSB-clear c must be a no-op in Graphics mode"
    );

    // c with MSB set: valid graphics data, repeated n times.
    feed_str(&mut dmp, b"\x1C\x03\xFF"); // n=3, c=0xFF (all 7 dots)
    let spacing = Pitch::Normal.addressable_spacing();
    assert_eq!(dmp.x, 3 * spacing);
    assert_eq!(dmp.paper.dots_in_range(0, 6 * DOT_ROW_UNITS).len(), 3 * 7);
}

#[test]
fn graphics_mode_enter_data_lf_and_exit_round_trip() {
    let mut dmp = DMP105::new();
    feed_str(&mut dmp, b"A"); // one CP char first, to prove entry doesn't reset x
    let x_before_graphics = dmp.x;

    dmp.feed(control::SELECT_GRAPHICS);
    assert_eq!(dmp.mode, Mode::Graphics);
    assert_eq!(
        dmp.x, x_before_graphics,
        "entering Graphics must not reset x"
    );

    dmp.feed(0xFF); // all 7 dots
    let spacing = Pitch::Normal.addressable_spacing();
    assert_eq!(dmp.x, x_before_graphics + spacing);
    let dots = dmp.paper.dots_in_range(0, 6 * DOT_ROW_UNITS);
    assert_eq!(
        dots.iter()
            .filter(|&&(x, _)| x == x_before_graphics)
            .count(),
        7,
        "0xFF must plot all 7 body rows"
    );

    let y_before_lf = dmp.y;
    dmp.feed(control::LF); // fixed 7/72" feed, does not touch x
    assert_eq!(dmp.y, y_before_lf + GRAPHICS_LF_UNITS);
    assert_eq!(
        dmp.x,
        x_before_graphics + spacing,
        "graphics LF must not touch x"
    );

    dmp.feed(control::END_GRAPHICS);
    assert_eq!(dmp.mode, Mode::CharacterPrint);
}

#[test]
fn graphics_lf_vs_text_lf_rounding_trap_is_not_reproducible_from_given_facts() {
    // See the module doc comment: the manual's "11 full-pitch LFs = 18
    // graphics LFs" identity does not hold under the individually-verified
    // unit values; this documents that rather than asserting a fabricated
    // resolution.
    let full_pitch_total = 11 * LF_PITCH_1_6;
    let graphics_total = 18 * GRAPHICS_LF_UNITS;
    assert_eq!(full_pitch_total, 132 * DOT_ROW_UNITS);
    assert_eq!(graphics_total, 126 * DOT_ROW_UNITS);
    assert_ne!(
        full_pitch_total, graphics_total,
        "if this ever holds, the manual's identity has become reproducible \
         from verified facts alone -- update the module doc comment"
    );
}

#[test]
fn head_positioning_sets_absolute_column_including_explicit_zero_band() {
    let mut dmp = DMP105::new();
    // n1=0 n2=0 must be accepted as a real 2-byte operand pair, not skipped as zero.
    feed_str(&mut dmp, b"\x1B\x10\x00\x00");
    assert_eq!(dmp.x, 0);

    // n1=1, n2=44 -> column 300, at Normal pitch (30 x-units/dot).
    feed_str(&mut dmp, b"\x1B\x10\x01\x2C");
    let expected = (256 + 44) * Pitch::Normal.addressable_spacing();
    assert_eq!(dmp.x, expected);
}

#[test]
fn undefined_codes_print_the_x_glyph() {
    let mut undefined = DMP105::new();
    undefined.feed(0x02); // undefined low control code
    let mut x_glyph = DMP105::new();
    feed_str(&mut x_glyph, b"X");
    assert_eq!(
        undefined.paper.dots_in_range(0, DESCENDER_ROW),
        x_glyph.paper.dots_in_range(0, DESCENDER_ROW)
    );

    let mut undefined_high = DMP105::new();
    undefined_high.feed(0x85); // undefined in $80-$9F
    assert_eq!(
        undefined_high.paper.dots_in_range(0, DESCENDER_ROW),
        x_glyph.paper.dots_in_range(0, DESCENDER_ROW)
    );
}

#[test]
fn esc_5a_feeds_immediately_esc_5b_only_latches() {
    let mut immediate = DMP105::new();
    feed_str(&mut immediate, b"\x1B\x5A\x0A"); // ESC 5A 10: feed 10/72" now
    assert_eq!(immediate.y, 10 * DOT_ROW_UNITS);
    // Latched pitch is untouched: a later plain LF still uses the default 1/6" pitch.
    immediate.feed(control::LF);
    assert_eq!(immediate.y, 10 * DOT_ROW_UNITS + LF_PITCH_1_6);

    let mut latched = DMP105::new();
    feed_str(&mut latched, b"\x1B\x5B\x0A"); // ESC 5B 10: latch only, no feed
    assert_eq!(latched.y, 0, "5B must not feed immediately");
    latched.feed(control::LF);
    assert_eq!(
        latched.y,
        10 * DOT_ROW_UNITS,
        "a later plain LF must use the newly latched pitch"
    );
}

#[test]
fn reset_restores_power_on_defaults_but_leaves_paper_alone() {
    let mut dmp = DMP105::new();
    feed_str(&mut dmp, b"\x1B\x0EHELLO"); // elongated, some ink on the paper
    assert!(dmp.x > 0);
    assert!(!dmp.paper.dots_in_range(0, DESCENDER_ROW).is_empty());

    dmp.reset();
    assert_eq!(dmp.x, 0);
    assert_eq!(dmp.y, 0);
    assert_eq!(dmp.pitch, Pitch::Normal);
    assert_eq!(dmp.lf_pitch_units, LF_PITCH_1_6);
    assert!(!dmp.elongation);
    assert!(
        !dmp.paper.dots_in_range(0, DESCENDER_ROW).is_empty(),
        "reset must not erase already-printed paper"
    );
}

#[test]
fn handle_exposes_extent_dirty_range_and_tear_off() {
    let handle = DMP105Handle::new();
    let mut sink: Box<dyn PrinterSink> = Box::new(handle.clone());
    for &b in b"HI\r" {
        sink.write_byte(b);
    }
    let extent = handle.paper_extent();
    assert!(extent.dot_count > 0);
    assert!(handle.take_dirty().is_some());
    assert!(handle.take_dirty().is_none());

    handle.tear_off();
    assert_eq!(handle.paper_extent().dot_count, 0);
}

fn body_dots(bytes: &[u8]) -> Vec<(u32, u32)> {
    let mut dmp = DMP105::new();
    feed_str(&mut dmp, bytes);
    dmp.paper.dots_in_range(0, DESCENDER_ROW)
}

#[test]
fn european_codes_print_symbols_and_extended_codes_stay_undefined() {
    let placeholder = body_dots(b"X");
    let a_grave = body_dots(b"\xA1");
    assert!(!a_grave.is_empty());
    assert_ne!(a_grave, placeholder);
    assert_ne!(body_dots(b"\xBF"), placeholder);
    assert_eq!(body_dots(b"\xC0"), placeholder);
    assert_eq!(body_dots(b"\xFF"), placeholder);
}

#[test]
fn block_graphics_fill_the_cell_and_join_across_cells() {
    let dot = Pitch::Normal.dot_spacing();
    let bar = body_dots(b"\xF1\xF1");
    let xs: std::collections::BTreeSet<u32> = bar.iter().map(|&(x, _)| x).collect();
    let expected: std::collections::BTreeSet<u32> =
        (0..2 * CELL_DOTS).map(|position| position * dot).collect();
    assert_eq!(xs, expected);
    let ys: std::collections::BTreeSet<u32> = bar.iter().map(|&(_, y)| y).collect();
    assert_eq!(ys.len(), 1);
    assert!(body_dots(b"\xE0").is_empty());
    let mut blank = DMP105::new();
    feed_str(&mut blank, b"\xE0");
    assert_eq!(blank.x, normal_cell_width());
    let full = body_dots(b"\xEF");
    assert_eq!(full.len(), CELL_DOTS as usize * BLOCK_SIZE);
    let stem = body_dots(b"\xF5");
    let stem_xs: std::collections::BTreeSet<u32> = stem.iter().map(|&(x, _)| x).collect();
    assert_eq!(stem_xs.len(), BLOCK_DOT_STEP);
}
