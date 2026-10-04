//! `Machine::basic_text_cursor` against the real CoCo 3 ROM: BASIC's cursor
//! variables (`coco_core::basic_vars`) on the 32-column VDG screen and the
//! `WIDTH 40`/`80` hi-res screens, and no cursor (or text) in PMODE graphics.

use coco_core::{Machine, MachineConfig, TextCursor, basic_vars, keyboard};
use mc6809::Bus;
use test_assets::rom::COCO3;

/// Enough fields for the cold start to print the banner and `OK`.
const BOOT_FIELDS: usize = 400;
/// Fields a key stays down, then up, per tap; BASIC's scan sees both.
const TAP_FIELDS: usize = 3;
/// Fields for a typed command (screen switch, program run) to finish.
const SETTLE_FIELDS: usize = 60;
/// Columns on BASIC's `WIDTH 40` and `WIDTH 80` screens.
const WIDTH_40_COLS: u8 = 40;
const WIDTH_80_COLS: u8 = 80;

fn boot_to_prompt() -> Machine {
    let rom = std::fs::read(test_assets::rom(COCO3))
        .expect("installed coco3.rom is required")
        .into_boxed_slice();
    let mut m = Machine::new(MachineConfig::default(), rom);
    for _ in 0..BOOT_FIELDS {
        m.run_field();
        if m.text_screen_lines().iter().any(|l| l.trim() == "OK") {
            // BASIC drops keys pressed before its prompt loop settles.
            run_fields(&mut m, SETTLE_FIELDS);
            return m;
        }
    }
    panic!("no OK prompt:\n{}", m.text_screen_lines().join("\n"));
}

fn run_fields(m: &mut Machine, n: usize) {
    for _ in 0..n {
        m.run_field();
    }
}

fn tap(m: &mut Machine, c: char) {
    let (pos, shift) = keyboard::char_key(c).unwrap_or_else(|| panic!("no key for {c:?}"));
    m.bus.keyboard.set(keyboard::SHIFT, shift);
    m.bus.keyboard.set(pos, true);
    run_fields(m, TAP_FIELDS);
    m.bus.keyboard.set(pos, false);
    m.bus.keyboard.set(keyboard::SHIFT, false);
    run_fields(m, TAP_FIELDS);
}

fn type_text(m: &mut Machine, text: &str) {
    text.chars().for_each(|c| tap(m, c));
}

fn type_line(m: &mut Machine, line: &str) {
    type_text(m, line);
    tap(m, '\r');
    run_fields(m, SETTLE_FIELDS);
}

fn cursor(m: &Machine) -> TextCursor {
    m.basic_text_cursor()
        .unwrap_or_else(|| panic!("no cursor; {}", m.video_mode_summary()))
}

/// The cursor sits at the start of the line under the last `OK`, then moves
/// one column per typed character.
fn assert_cursor_follows_prompt(m: &mut Machine, cols: usize) {
    let lines = m.text_screen_lines();
    assert!(lines.iter().all(|l| l.chars().count() == cols));
    let at = cursor(m);
    assert_eq!(at.col, 0);
    assert_eq!(
        lines[at.row - 1].trim(),
        "OK",
        "screen:\n{}",
        lines.join("\n")
    );

    const TYPED: &str = "AB";
    type_text(m, TYPED);
    let expected = TextCursor {
        row: at.row,
        col: TYPED.len(),
    };
    assert_eq!(cursor(m), expected);
}

#[test]
fn vdg_32_column_cursor_follows_the_prompt() {
    let mut m = boot_to_prompt();
    assert_cursor_follows_prompt(&mut m, coco_core::video::COLS);
}

#[test]
fn width_40_cursor_follows_the_prompt() {
    let mut m = boot_to_prompt();
    type_line(&mut m, "WIDTH 40");
    assert!(m.video_mode_summary().contains("GIME hi-res text"));
    assert_cursor_follows_prompt(&mut m, WIDTH_40_COLS.into());
}

#[test]
fn width_80_cursor_follows_the_prompt() {
    let mut m = boot_to_prompt();
    type_line(&mut m, "WIDTH 80");
    assert!(m.video_mode_summary().contains("GIME hi-res text"));
    assert_cursor_follows_prompt(&mut m, WIDTH_80_COLS.into());
}

/// `PMODE`/`SCREEN` hold only from a running program; BASIC's idle loop puts
/// the text screen back (see `coco2_boot/pokes.rs`).
#[test]
fn vdg_graphics_has_no_text_and_no_cursor() {
    let mut m = boot_to_prompt();
    type_line(&mut m, "10 PMODE 4,1:SCREEN 1,1");
    type_line(&mut m, "20 GOTO 20");
    type_line(&mut m, "RUN");

    assert!(m.video_mode_summary().contains("PMODE"));
    let lines = m.text_screen_lines();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("<no text buffer: VDG graphics"));
    assert_eq!(m.basic_text_cursor(), None);
}

#[test]
fn vdg_cursor_outside_basics_screen_is_rejected() {
    let mut m = boot_to_prompt();
    let past_screen = basic_vars::VDG_SCREEN_LAST + 1;
    let [hi, lo] = past_screen.to_be_bytes();
    m.bus.write(basic_vars::CURPOS, hi);
    m.bus.write(basic_vars::CURPOS + 1, lo);
    assert_eq!(m.basic_text_cursor(), None);
}

#[test]
fn vdg_cursor_needs_width_32() {
    let mut m = boot_to_prompt();
    m.bus
        .write(basic_vars::HRWIDTH, basic_vars::hrwidth::HIRES_40);
    assert_eq!(m.basic_text_cursor(), None);
}

#[test]
fn hires_cursor_needs_basics_screen_size_to_match_the_gime() {
    let mut m = boot_to_prompt();
    type_line(&mut m, "WIDTH 80");
    m.bus.write(basic_vars::H_COLUMN, WIDTH_40_COLS);
    assert_eq!(m.basic_text_cursor(), None);
}
