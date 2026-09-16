//! Keyboard matrix coverage: the row-sense logic, the symbolic char map, and an
//! end-to-end "type at the BASIC prompt and see it echo" test against the real ROM.

use coco_core::keyboard::{Keyboard, char_key};
use coco_core::{Machine, MachineConfig};
use mc6809::Bus;
use test_assets::rom::COCO3;

// ---- row sense --------------------------------------------------------------

#[test]
fn sense_pulls_row_low_only_for_strobed_column() {
    let mut kb = Keyboard::new();
    kb.set((0, 1), true); // 'A' at row 0, column 1

    // Strobe column 1 low (active low): row 0 must read low (0xFE), PA7 stays high.
    const ROW0_LOW: u8 = 0xFE;
    let strobe_col1 = !(1u8 << 1);
    assert_eq!(kb.sense(strobe_col1), ROW0_LOW);
    // Strobe a different column: no key sensed.
    let strobe_col2 = !(1u8 << 2);
    assert_eq!(kb.sense(strobe_col2), 0xFF);
    // Strobe all columns low: the key is still sensed.
    assert_eq!(kb.sense(0x00), ROW0_LOW);
}

#[test]
fn sense_reports_no_key_when_idle() {
    let kb = Keyboard::new();
    assert_eq!(kb.sense(0x00), 0xFF);
}

#[test]
fn note_read_counts_every_strobed_column() {
    let mut kb = Keyboard::new();
    kb.note_read(!(1u8 << 1)); // column 1 only
    kb.note_read(0x00); // every column
    kb.note_read(0xFF); // no column strobed: nothing read
    assert_eq!(kb.column_reads(1), 2);
    assert_eq!(kb.column_reads(2), 1);
    assert_eq!(kb.column_reads(8), 0, "out-of-range column reads as zero");
}

#[test]
fn cpu_reads_of_port_a_data_count_as_keyboard_scans() {
    let mut m = boot_to_prompt();
    const PIA0_PORT_A: u16 = 0xFF00;
    const PIA0_PORT_B: u16 = 0xFF02;
    const COLUMN_3_STROBE: u8 = !(1 << 3);
    m.bus.write(PIA0_PORT_B, COLUMN_3_STROBE);
    let before = m.bus.keyboard.column_reads(3);
    let other = m.bus.keyboard.column_reads(0);
    m.bus.read(PIA0_PORT_A);
    m.bus.read(PIA0_PORT_B); // port B is not a row sense
    assert_eq!(m.bus.keyboard.column_reads(3), before + 1);
    assert_eq!(m.bus.keyboard.column_reads(0), other);
}

// ---- symbolic char map ------------------------------------------------------

#[test]
fn char_key_maps_letters_and_symbols() {
    assert_eq!(char_key('A'), Some(((0, 1), true))); // uppercase needs shift
    assert_eq!(char_key('a'), Some(((0, 1), false)));
    assert_eq!(char_key('@'), Some(((0, 0), false)));
    assert_eq!(char_key('Z'), Some(((3, 2), true)));
    assert_eq!(char_key('8'), Some(((5, 0), false)));
    assert_eq!(char_key('('), Some(((5, 0), true))); // CoCo shift-8
    assert_eq!(char_key('*'), Some(((5, 2), true))); // CoCo shift-:
    assert_eq!(char_key(':'), Some(((5, 2), false)));
    assert_eq!(char_key('?'), Some(((5, 7), true)));
    assert_eq!(char_key(' '), Some(((3, 7), false)));
    assert_eq!(char_key('~'), None); // no CoCo key
}

// ---- end-to-end typing ------------------------------------------------------

fn boot_to_prompt() -> Machine {
    let path = test_assets::rom(COCO3);
    let rom = std::fs::read(&path).unwrap().into_boxed_slice();
    let mut m = Machine::new(MachineConfig::default(), rom);
    // Run until BASIC prints OK.
    for _ in 0..400 {
        m.run_field();
    }
    m
}

/// Tap a key: hold it down for a few fields, then release for a few (so the ROM's
/// keyboard scan registers exactly one press).
fn tap(m: &mut Machine, pos: (u8, u8)) {
    for _ in 0..3 {
        m.bus.keyboard.set(pos, true);
        m.run_field();
    }
    m.bus.keyboard.set(pos, false);
    for _ in 0..3 {
        m.run_field();
    }
}

/// Decode a text-screen row to ASCII (letters/digits only for the assert).
fn screen_row(m: &mut Machine, row: u16) -> String {
    (0..32)
        .map(|c| {
            let code = m.bus.read(0x0400 + row * 32 + c) & 0x3F;
            if code < 0x20 {
                (b'@' + code) as char
            } else {
                (b' ' + (code - 0x20)) as char
            }
        })
        .collect()
}

#[test]
fn typing_at_prompt_echoes_to_screen() {
    let mut m = boot_to_prompt();

    // Type 'A' (row 0, col 1). BASIC echoes it immediately after the "OK" line.
    tap(&mut m, (0, 1));

    // The echoed 'A' should now appear somewhere on the screen that was blank
    // before typing (rows following the banner).
    let found = (4..16).any(|r| screen_row(&mut m, r).contains('A'));
    assert!(found, "typed 'A' did not echo to the screen");
}

#[test]
fn typing_multiple_keys_with_irqs_active() {
    // Regression: keys pressed after the first used to derail BASIC because the
    // $FE00 interrupt-trampoline page was read from ROM. Type A, B, C with keys
    // held across several field-sync IRQs and confirm all three echo in order.
    let mut m = boot_to_prompt();
    tap(&mut m, (0, 1)); // A
    tap(&mut m, (0, 2)); // B
    tap(&mut m, (0, 3)); // C

    let typed = (4..16)
        .map(|r| screen_row(&mut m, r))
        .find(|row| row.trim_start().starts_with("ABC"))
        .unwrap_or_default();
    assert!(
        typed.trim_start().starts_with("ABC"),
        "expected 'ABC' echoed, screen had no such row"
    );
}
