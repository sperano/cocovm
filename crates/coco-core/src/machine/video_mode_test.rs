use crate::gime::{init0, vmode};
use crate::{Machine, MachineConfig, TextCursor, basic_vars};

use crate::machine::text_cursor::{
    GIME_CURSOR_ROUTINE, GIME_CURSOR_ROUTINE_OFFSET, VDG_CURSOR_ROUTINE, VDG_CURSOR_ROUTINE_OFFSET,
};

const ROM_BYTES: usize = 32 * 1024;
const SPACE: u8 = 0x20;
const GIME_LPR_EIGHT_LINES: u8 = 3;
const GIME_HRES_40_COLUMNS: u8 = 1 << crate::gime::vres::HRES_SHIFT;

fn machine_with_rom(rom: Vec<u8>) -> Machine {
    let mut machine = Machine::new(MachineConfig::default(), rom.into_boxed_slice());
    machine.bus.gime.sam_page = (basic_vars::VDG_SCREEN_BASE / crate::gime::SAM_PAGE_UNIT) as u8;
    machine
}

fn machine() -> Machine {
    machine_with_rom(vec![0; ROM_BYTES])
}

fn stock_basic_machine() -> Machine {
    let mut rom = vec![0; ROM_BYTES];
    rom[VDG_CURSOR_ROUTINE_OFFSET..VDG_CURSOR_ROUTINE_OFFSET + VDG_CURSOR_ROUTINE.len()]
        .copy_from_slice(VDG_CURSOR_ROUTINE);
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
        basic_vars::CURPOS,
        (cursor_address >> 8) as u8,
    );
    write_workspace(&mut machine, basic_vars::CURPOS + 1, cursor_address as u8);
    write_workspace(&mut machine, cursor_address, BLINK_BYTE);

    let screen = machine.text_screen();

    assert_eq!(screen.cursor, Some(TextCursor { row: 2, col: 3 }));
    assert_eq!(screen.lines[2].as_bytes()[3], SPACE);
}

#[test]
fn legacy_cursor_outside_the_visible_buffer_is_ignored() {
    let mut machine = stock_basic_machine();
    machine.bus.gime.init0 = init0::COCO;
    let outside = machine.legacy_display_base() + crate::video::SCREEN_LEN as u16;
    write_workspace(&mut machine, basic_vars::CURPOS, (outside >> 8) as u8);
    write_workspace(&mut machine, basic_vars::CURPOS + 1, outside as u8);

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
    write_workspace(
        &mut machine,
        basic_vars::HRWIDTH,
        basic_vars::hrwidth::HIRES_40,
    );
    write_workspace(&mut machine, basic_vars::H_CURSX, 7);
    write_workspace(&mut machine, basic_vars::H_CURSY, 4);
    write_workspace(&mut machine, basic_vars::H_COLUMN, COLUMNS);
    write_workspace(&mut machine, basic_vars::H_ROW, ROWS);

    assert_eq!(
        machine.text_screen().cursor,
        Some(TextCursor { row: 4, col: 7 })
    );
}

#[test]
fn gime_cursor_requires_matching_dimensions_and_text_mode() {
    let mut machine = stock_basic_machine();
    machine.bus.gime.init0 = 0;
    machine.bus.gime.vmode = GIME_LPR_EIGHT_LINES;
    machine.bus.gime.vres = 0;
    machine.bus.gime.all_ram = true;
    write_workspace(
        &mut machine,
        basic_vars::HRWIDTH,
        basic_vars::hrwidth::HIRES_80,
    );
    write_workspace(&mut machine, basic_vars::H_COLUMN, 80);
    write_workspace(&mut machine, basic_vars::H_ROW, 24);
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
        basic_vars::CURPOS,
        (cursor_address >> 8) as u8,
    );
    write_workspace(&mut machine, basic_vars::CURPOS + 1, cursor_address as u8);
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
    write_workspace(
        &mut machine,
        basic_vars::HRWIDTH,
        basic_vars::hrwidth::HIRES_40,
    );
    write_workspace(&mut machine, basic_vars::H_CURSX, 7);
    write_workspace(&mut machine, basic_vars::H_CURSY, 4);
    write_workspace(&mut machine, basic_vars::H_COLUMN, COLUMNS);
    write_workspace(&mut machine, basic_vars::H_ROW, ROWS);

    assert_eq!(machine.text_screen().cursor, None);
}
