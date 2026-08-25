//! The "it's alive" milestone (`DESIGN.md` §10 step 3): with the field-sync IRQ
//! wired through PIA0, the stock ROM breaks out of its idle loop, runs BASIC's
//! housekeeping, and paints the sign-on banner + `OK` prompt into the 32×16
//! VDG text screen at logical $0400. Verified against the real `roms/coco3.rom`.

use coco_core::{Machine, MachineConfig};
use mc6809::Bus;
use test_assets::rom::COCO3;

/// Logical base of the CoCo-compatible 32×16 text screen.
const TEXT_BASE: u16 = 0x0400;
const COLS: u16 = 32;
const ROWS: u16 = 16;

/// Translate a VDG alphanumeric display code to ASCII (letters/space only; the
/// inverse-video bit is masked off).
fn vdg_to_ascii(code: u8) -> char {
    match code & 0x3F {
        b @ 0x00..=0x1F => (b'@' + b) as char, // @, A–Z, [ \ ] ↑ ←
        b @ 0x20..=0x3F => (b' ' + (b - 0x20)) as char, // space, ! " # … ?
        _ => unreachable!(),
    }
}

fn boot_machine() -> Machine {
    let path = test_assets::rom(COCO3);
    let rom = std::fs::read(&path).unwrap().into_boxed_slice();
    Machine::new(MachineConfig::default(), rom)
}

/// Read one row of the text screen as a decoded ASCII string.
fn screen_row(m: &mut Machine, row: u16) -> String {
    (0..COLS)
        .map(|c| vdg_to_ascii(m.bus.read(TEXT_BASE + row * COLS + c)))
        .collect()
}

#[test]
fn rom_paints_signon_banner_and_prompt() {
    // Enough fields for the cold-start to size RAM and print the banner.
    const MAX_FIELDS: usize = 400;
    let mut m = boot_machine();

    // The `OK` prompt is the last thing the cold-start prints, so wait for it
    // rather than the banner (which appears a few fields earlier).
    let ok_prompt =
        |m: &mut Machine| (0..ROWS).any(|r| screen_row(m, r).trim_start().starts_with("OK"));

    let mut alive = false;
    for _ in 0..MAX_FIELDS {
        m.run_field();
        if ok_prompt(&mut m) {
            alive = true;
            break;
        }
    }

    assert!(
        alive,
        "OK prompt never appeared; row0 = {:?}",
        screen_row(&mut m, 0)
    );
    assert!(screen_row(&mut m, 0).contains("EXTENDED COLOR BASIC"));
    assert!(screen_row(&mut m, 1).contains("TANDY"));
    assert!(screen_row(&mut m, 2).contains("MICROSOFT"));
}

#[test]
fn field_sync_irq_breaks_the_idle_loop() {
    // Without interrupts the ROM spins on a single `BRA *`. Driving fields must
    // put it in a real main loop touching many addresses.
    let mut m = boot_machine();
    for _ in 0..200 {
        m.run_field();
    }
    let mut pcs = std::collections::HashSet::new();
    for _ in 0..2000 {
        pcs.insert(m.cpu.pc);
        m.step_cpu_raw();
    }
    assert!(
        pcs.len() > 3,
        "CPU still stuck (only {} distinct PCs) — IRQ not delivered",
        pcs.len()
    );
}
