use crate::gime::{init0, vmode};
use crate::{Machine, MachineConfig, TextCursor};

use super::{
    GIME_CURSOR_COLUMN, GIME_CURSOR_ROUTINE, GIME_CURSOR_ROUTINE_OFFSET, GIME_CURSOR_ROW,
    GIME_TEXT_COLUMNS, GIME_TEXT_ROWS, LEGACY_CURSOR_POINTER, LEGACY_CURSOR_ROUTINE,
    LEGACY_CURSOR_ROUTINE_OFFSET,
};

const ROM_BYTES: usize = 32 * 1024;
const SPACE: u8 = 0x20;
const GIME_LPR_EIGHT_LINES: u8 = 3;
const GIME_HRES_40_COLUMNS: u8 = 1 << crate::gime::vres::HRES_SHIFT;

fn machine_with_rom(rom: Vec<u8>) -> Machine {
    Machine::new(MachineConfig::default(), rom.into_boxed_slice())
}

fn machine() -> Machine {
    machine_with_rom(vec![0; ROM_BYTES])
}

fn stock_basic_machine() -> Machine {
    let mut rom = vec![0; ROM_BYTES];
    rom[LEGACY_CURSOR_ROUTINE_OFFSET..LEGACY_CURSOR_ROUTINE_OFFSET + LEGACY_CURSOR_ROUTINE.len()]
        .copy_from_slice(LEGACY_CURSOR_ROUTINE);
    rom[GIME_CURSOR_ROUTINE_OFFSET..GIME_CURSOR_ROUTINE_OFFSET + GIME_CURSOR_ROUTINE.len()]
        .copy_from_slice(GIME_CURSOR_ROUTINE);
    machine_with_rom(rom)
}

fn write_workspace(machine: &mut Machine, address: u16, value: u8) {
    let physical = machine.bus.gime.translate(address);
    machine.bus.ram[physical] = value;
}

#[test]
fn legacy_text_reports_and_normalizes_a_valid_cursor_cell() {
    const CURSOR_OFFSET: u16 = 2 * crate::video::COLS as u16 + 3;
    const BLINK_BYTE: u8 = 0xFF;

    let mut machine = stock_basic_machine();
    machine.bus.gime.init0 = init0::COCO;
    let base = machine.legacy_display_base();
    let cursor_address = base + CURSOR_OFFSET;
    write_workspace(
        &mut machine,
        LEGACY_CURSOR_POINTER,
        (cursor_address >> 8) as u8,
    );
    write_workspace(
        &mut machine,
        LEGACY_CURSOR_POINTER + 1,
        cursor_address as u8,
    );
    write_workspace(&mut machine, cursor_address, BLINK_BYTE);

    let screen = machine.text_screen();

    assert_eq!(screen.cursor, Some(TextCursor { row: 2, column: 3 }));
    assert_eq!(screen.lines[2].as_bytes()[3], SPACE);
}

#[test]
fn legacy_cursor_outside_the_visible_buffer_is_ignored() {
    let mut machine = stock_basic_machine();
    machine.bus.gime.init0 = init0::COCO;
    let outside = machine.legacy_display_base() + crate::video::SCREEN_LEN as u16;
    write_workspace(&mut machine, LEGACY_CURSOR_POINTER, (outside >> 8) as u8);
    write_workspace(&mut machine, LEGACY_CURSOR_POINTER + 1, outside as u8);

    assert_eq!(machine.text_screen().cursor, None);
}

#[test]
fn gime_text_reports_valid_workspace_coordinates() {
    const COLUMNS: u8 = 40;
    const ROWS: u8 = 24;

    let mut machine = stock_basic_machine();
    machine.bus.gime.init0 = 0;
    machine.bus.gime.vmode = GIME_LPR_EIGHT_LINES;
    machine.bus.gime.vres = GIME_HRES_40_COLUMNS;
    machine.bus.gime.all_ram = true;
    write_workspace(&mut machine, GIME_CURSOR_COLUMN, 7);
    write_workspace(&mut machine, GIME_CURSOR_ROW, 4);
    write_workspace(&mut machine, GIME_TEXT_COLUMNS, COLUMNS);
    write_workspace(&mut machine, GIME_TEXT_ROWS, ROWS);

    assert_eq!(
        machine.text_screen().cursor,
        Some(TextCursor { row: 4, column: 7 })
    );
}

#[test]
fn gime_cursor_requires_matching_dimensions_and_text_mode() {
    let mut machine = stock_basic_machine();
    machine.bus.gime.init0 = 0;
    machine.bus.gime.vmode = GIME_LPR_EIGHT_LINES;
    machine.bus.gime.vres = 0;
    machine.bus.gime.all_ram = true;
    write_workspace(&mut machine, GIME_TEXT_COLUMNS, 80);
    write_workspace(&mut machine, GIME_TEXT_ROWS, 24);
    assert_eq!(machine.text_screen().cursor, None);

    machine.bus.gime.vmode = vmode::BP;
    assert_eq!(machine.text_screen().cursor, None);
}

#[test]
fn legacy_workspace_is_ignored_without_the_stock_basic_routine() {
    const CURSOR_OFFSET: u16 = 5;
    const CURSOR_BYTE: u8 = 0x01;

    let mut machine = machine();
    machine.bus.gime.init0 = init0::COCO;
    let base = machine.legacy_display_base();
    let cursor_address = base + CURSOR_OFFSET;
    write_workspace(
        &mut machine,
        LEGACY_CURSOR_POINTER,
        (cursor_address >> 8) as u8,
    );
    write_workspace(
        &mut machine,
        LEGACY_CURSOR_POINTER + 1,
        cursor_address as u8,
    );
    write_workspace(&mut machine, cursor_address, CURSOR_BYTE);

    let screen = machine.text_screen();

    assert_eq!(screen.cursor, None);
    assert_eq!(
        screen.lines[0].chars().nth(CURSOR_OFFSET as usize),
        Some(crate::video::decode_alpha_char(CURSOR_BYTE))
    );
}

#[test]
fn gime_workspace_is_ignored_without_the_stock_basic_routine() {
    const COLUMNS: u8 = 40;
    const ROWS: u8 = 24;

    let mut machine = machine();
    machine.bus.gime.init0 = 0;
    machine.bus.gime.vmode = GIME_LPR_EIGHT_LINES;
    machine.bus.gime.vres = GIME_HRES_40_COLUMNS;
    machine.bus.gime.all_ram = true;
    write_workspace(&mut machine, GIME_CURSOR_COLUMN, 7);
    write_workspace(&mut machine, GIME_CURSOR_ROW, 4);
    write_workspace(&mut machine, GIME_TEXT_COLUMNS, COLUMNS);
    write_workspace(&mut machine, GIME_TEXT_ROWS, ROWS);

    assert_eq!(machine.text_screen().cursor, None);
}
