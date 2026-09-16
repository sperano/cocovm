use super::*;

use coco_core::{Machine, MachineConfig};
use test_assets::rom::COCO3;

/// Whether `pos` reads as pressed through the PIA sense path.
fn is_down(kb: &kbd::Keyboard, pos: Pos) -> bool {
    kb.sense(!(1 << pos.1)) & (1 << pos.0) == 0
}

/// One CPU pass over the whole matrix, column by column, as a scanning ROM does.
pub(crate) fn scan_matrix(kb: &mut kbd::Keyboard) {
    for col in 0..kbd::COLS as u8 {
        kb.note_read(!(1 << col));
    }
}

/// Advance a field under a target that scanned the keyboard continuously
/// during the previous field.
fn advance_scanned(ta: &mut TypeAhead, kb: &mut kbd::Keyboard) {
    for _ in 0..TAP_READS_TO_REGISTER {
        scan_matrix(kb);
    }
    ta.advance(kb);
}

#[test]
fn hold_survives_a_release_all_between_fields() {
    let mut ta = TypeAhead::default();
    let (pos, _) = kbd::char_key('a').unwrap();
    ta.queue.push_back((pos, false).into());
    let mut kb = kbd::Keyboard::new();

    ta.advance(&mut kb);
    assert!(is_down(&kb, pos));
    // A focus-loss frame between fields clears the matrix.
    kb.release_all();
    ta.advance(&mut kb);
    assert!(is_down(&kb, pos), "hold must be re-asserted");
    assert!(ta.is_active());
}

#[test]
fn one_tap_takes_fields_per_tap_advances_under_a_scanning_target() {
    let mut ta = TypeAhead::default();
    ta.queue.push_back(kbd::char_key('a').unwrap().into());
    let mut kb = kbd::Keyboard::new();
    for _ in 0..FIELDS_PER_TAP - 1 {
        advance_scanned(&mut ta, &mut kb);
        assert!(ta.is_active());
    }
    advance_scanned(&mut ta, &mut kb);
    assert!(!ta.is_active());
}

#[test]
fn hold_extends_until_the_target_reads_the_column_enough_times() {
    let mut ta = TypeAhead::default();
    let (pos, _) = kbd::char_key('3').unwrap();
    ta.queue.push_back((pos, false).into());
    let mut kb = kbd::Keyboard::new();

    // Minimum hold elapses with no scan at all: still held.
    for _ in 0..=TYPE_HOLD_FIELDS {
        ta.advance(&mut kb);
    }
    for _ in 0..5 {
        ta.advance(&mut kb);
        assert!(is_down(&kb, pos), "unscanned key stays held");
    }
    // Reads of other columns do not count.
    kb.note_read(!(1 << kbd::ENTER.1));
    ta.advance(&mut kb);
    assert!(is_down(&kb, pos));
    // One short of registering: still held.
    for _ in 1..TAP_READS_TO_REGISTER {
        kb.note_read(!(1 << pos.1));
    }
    ta.advance(&mut kb);
    assert!(is_down(&kb, pos));
    // The final read releases it on the next field.
    kb.note_read(!(1 << pos.1));
    ta.advance(&mut kb);
    assert!(!is_down(&kb, pos));
    assert!(matches!(ta.phase, TypePhase::Gap(0)));
}

#[test]
fn all_columns_strobed_counts_as_a_read_of_the_key_column() {
    let mut ta = TypeAhead::default();
    let (pos, _) = kbd::char_key('3').unwrap();
    ta.queue.push_back((pos, false).into());
    let mut kb = kbd::Keyboard::new();
    for _ in 0..=TYPE_HOLD_FIELDS {
        ta.advance(&mut kb);
    }
    const ALL_COLUMNS: u8 = 0x00;
    for _ in 0..TAP_READS_TO_REGISTER {
        kb.note_read(ALL_COLUMNS);
    }
    ta.advance(&mut kb);
    assert!(!is_down(&kb, pos));
}

#[test]
fn gap_extends_until_the_target_reads_the_release() {
    let mut ta = TypeAhead::default();
    let (pos, _) = kbd::char_key('0').unwrap();
    ta.queue.push_back((pos, false).into());
    ta.queue.push_back((pos, false).into());
    let mut kb = kbd::Keyboard::new();
    while !matches!(ta.phase, TypePhase::Gap(_)) {
        advance_scanned(&mut ta, &mut kb);
    }
    // Minimum gap elapses with no scan: the second tap waits.
    for _ in 0..TYPE_GAP_FIELDS + 5 {
        ta.advance(&mut kb);
        assert!(
            !is_down(&kb, pos),
            "second tap must wait for the release scan"
        );
    }
    kb.note_read(!(1 << pos.1));
    ta.advance(&mut kb);
    assert!(matches!(ta.phase, TypePhase::Idle));
    ta.advance(&mut kb);
    assert!(
        is_down(&kb, pos),
        "second tap starts once the release was scanned"
    );
}

#[test]
fn unscanned_tap_moves_on_at_the_scan_timeout() {
    let mut ta = TypeAhead::default();
    let (pos, _) = kbd::char_key('a').unwrap();
    ta.queue.push_back((pos, false).into());
    let mut kb = kbd::Keyboard::new();
    ta.advance(&mut kb);
    for _ in 0..=TYPE_SCAN_TIMEOUT_FIELDS {
        assert!(is_down(&kb, pos));
        ta.advance(&mut kb);
    }
    assert!(!is_down(&kb, pos), "hold gives up at the timeout");
    for _ in 0..=TYPE_SCAN_TIMEOUT_FIELDS {
        assert!(ta.is_active());
        ta.advance(&mut kb);
    }
    assert!(!ta.is_active(), "gap gives up at the timeout");
}

#[test]
fn shifted_tap_holds_shift_and_releases_both() {
    let mut ta = TypeAhead::default();
    let (pos, shift) = kbd::char_key('!').unwrap();
    assert!(shift);
    ta.queue.push_back((pos, shift).into());
    let mut kb = kbd::Keyboard::new();

    for _ in 0..=TYPE_HOLD_FIELDS {
        advance_scanned(&mut ta, &mut kb);
        assert!(is_down(&kb, pos) && is_down(&kb, kbd::SHIFT));
    }
    advance_scanned(&mut ta, &mut kb);
    assert!(!is_down(&kb, pos) && !is_down(&kb, kbd::SHIFT));
}

#[test]
fn clicked_chord_releases_all_modifiers_before_the_next_tap() {
    let mut ta = TypeAhead::default();
    let (pos, _) = kbd::char_key('a').unwrap();
    ta.queue.push_back(KeyTap {
        pos,
        modifiers: KeyModifiers {
            shift: true,
            ctrl: true,
            alt: true,
        },
    });
    ta.queue.push_back((pos, false).into());
    let mut kb = kbd::Keyboard::new();
    for _ in 0..=TYPE_HOLD_FIELDS {
        advance_scanned(&mut ta, &mut kb);
        for key in [pos, kbd::SHIFT, kbd::CTRL, kbd::ALT] {
            assert!(is_down(&kb, key));
        }
    }
    for _ in TYPE_HOLD_FIELDS as u64 + 1..FIELDS_PER_TAP {
        advance_scanned(&mut ta, &mut kb);
        for key in [pos, kbd::SHIFT, kbd::CTRL, kbd::ALT] {
            assert!(!is_down(&kb, key));
        }
    }
    advance_scanned(&mut ta, &mut kb);
    assert!(is_down(&kb, pos));
    for modifier in [kbd::SHIFT, kbd::CTRL, kbd::ALT] {
        assert!(!is_down(&kb, modifier));
    }
}

// ---- against the real ROM ---------------------------------------------------

/// The artifact test program: its long lines keep BASIC busy storing them
/// for up to nine fields after ENTER, longer than the fixed hold used to be.
const PASTED_PROGRAM: &str = "10 PMODE 4,1:PCLS:SCREEN 1,1
20 FOR X=0 TO 126 STEP 2:LINE(X,0)-(X,95),PSET:NEXT
30 FOR X=129 TO 255 STEP 2:LINE(X,0)-(X,95),PSET:NEXT
40 LINE(0,100)-(127,191),PSET,BF
50 B=0
60 A$=INKEY$:IF A$=\"\" THEN 60
70 IF A$=\"S\" THEN B=32-B:POKE &HFF98,B
80 IF A$<>\"Q\" THEN 60
90 SCREEN 0,0:END
";
/// Fields allowed for the cold start to reach the `OK` prompt.
const BOOT_FIELDS: usize = 600;
/// Fields after `OK` first paints for the cold start to finish, which
/// discards any key still held from reset.
const PROMPT_SETTLE_FIELDS: usize = 10;
/// Fields for `LIST` to finish painting after its ENTER.
const LIST_FIELDS: usize = 60;

fn boot_to_prompt() -> Option<Machine> {
    let Ok(rom) = std::fs::read(test_assets::rom(COCO3)) else {
        eprintln!("skipping: {COCO3} not present");
        return None;
    };
    let mut m = Machine::new(MachineConfig::default(), rom.into_boxed_slice());
    for _ in 0..BOOT_FIELDS {
        m.run_field();
        if m.text_screen_lines().iter().any(|l| l.trim() == "OK") {
            for _ in 0..PROMPT_SETTLE_FIELDS {
                m.run_field();
            }
            return Some(m);
        }
    }
    panic!("OK prompt never appeared");
}

/// Drain `text` through the type-ahead the way `run_fields` does, returning
/// the fields it took.
fn paste(m: &mut Machine, text: &str) -> u64 {
    let mut ta = TypeAhead::default();
    ta.queue
        .extend(text.chars().filter_map(kbd::char_key).map(KeyTap::from));
    let mut fields = 0;
    while ta.is_active() {
        ta.advance(&mut m.bus.keyboard);
        m.run_field();
        fields += 1;
    }
    fields
}

#[test]
fn pasting_a_program_into_basic_loses_no_characters() {
    let Some(mut m) = boot_to_prompt() else {
        return;
    };
    let taps = PASTED_PROGRAM.chars().count() as u64;
    let fields = paste(&mut m, PASTED_PROGRAM);
    paste(&mut m, "CLS:LIST\r");
    for _ in 0..LIST_FIELDS {
        m.run_field();
    }

    // LIST wraps long lines across rows, so compare with all spaces removed.
    let screen = m.text_screen_lines();
    let listing = screen.concat().replace(' ', "");
    for line in PASTED_PROGRAM.lines() {
        assert!(
            listing.contains(&line.replace(' ', "")),
            "{line:?} not listed intact:\n{}",
            screen.join("\n")
        );
    }
    // At the prompt BASIC scans continuously, so the paste must run at the
    // nominal pace and only stretch where BASIC is busy, never to the timeout.
    assert!(
        fields < taps * FIELDS_PER_TAP * 2,
        "{fields} fields for {taps} taps: type-ahead is waiting on the timeout"
    );
}
