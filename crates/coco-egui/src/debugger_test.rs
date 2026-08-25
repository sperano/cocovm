use super::*;
use test_assets::rom::COCO3;

fn load_rom() -> Box<[u8]> {
    let path = test_assets::rom(COCO3);
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .into_boxed_slice()
}

fn boot_machine() -> Machine {
    Machine::new(coco_core::MachineConfig::default(), load_rom())
}

#[test]
fn parse_addr_accepts_dollar_0x_and_bare_hex() {
    assert_eq!(parse_addr("$C000"), Some(0xC000));
    assert_eq!(parse_addr("0xC000"), Some(0xC000));
    assert_eq!(parse_addr("C000"), Some(0xC000));
    assert_eq!(parse_addr(" c000 "), Some(0xC000));
    assert_eq!(parse_addr("not hex"), None);
}

#[test]
fn ascii_char_dots_non_printable() {
    assert_eq!(ascii_char(b'A'), 'A');
    assert_eq!(ascii_char(0x00), '.');
    assert_eq!(ascii_char(0x7F), '.');
}

/// `run_field` with no breakpoints/watchpoints must behave exactly like
/// `Machine::run_field` — the zero-overhead common case the whole
/// per-frame running loop depends on.
#[test]
fn run_field_with_no_breakpoints_matches_plain_run_field() {
    let mut via_panel = boot_machine();
    let mut via_plain = boot_machine();
    let mut panel = DebuggerPanel::new();

    for _ in 0..3 {
        assert!(
            panel.run_field(&mut via_panel),
            "no breakpoints set: must always continue"
        );
        via_plain.run_field();
    }

    assert_eq!(via_panel.cpu.pc, via_plain.cpu.pc);
    assert_eq!(via_panel.cpu.cycles, via_plain.cpu.cycles);
}

/// An enabled breakpoint stops `run_field` early (returns `false`) and
/// parks the CPU exactly at the breakpoint address.
#[test]
fn run_field_stops_at_breakpoint() {
    const PROBE_STEPS: usize = 40;
    let target = {
        let mut probe = boot_machine();
        for _ in 0..PROBE_STEPS {
            probe.step_instruction();
        }
        probe.cpu.pc
    };

    let mut machine = boot_machine();
    let mut panel = DebuggerPanel::new();
    panel.core.add_breakpoint(target);

    let mut stopped = false;
    for _ in 0..3 {
        if !panel.run_field(&mut machine) {
            stopped = true;
            break;
        }
    }
    assert!(stopped, "breakpoint should have stopped a run_field call");
    assert_eq!(machine.cpu.pc, target);
}

/// Step In always retires exactly one real instruction, never stopping
/// mid-HALT-burn.
#[test]
fn step_in_advances_pc() {
    let mut machine = boot_machine();
    let pc0 = machine.cpu.pc;
    DebuggerPanel::step_in(&mut machine);
    assert_ne!(machine.cpu.pc, pc0);
}

/// Step Over on a non-call instruction falls back to Step In (advances
/// exactly one instruction, same as `step_in_advances_pc`).
#[test]
fn step_over_falls_back_to_step_in_for_non_call() {
    // Cold start's first instruction isn't a JSR/BSR/LBSR, so step_over must advance like step_in.
    let mut via_over = boot_machine();
    let mut via_in = boot_machine();
    let mut panel = DebuggerPanel::new();
    panel.step_over(&mut via_over);
    DebuggerPanel::step_in(&mut via_in);
    assert_eq!(via_over.cpu.pc, via_in.cpu.pc);
}

/// Step Scanline advances the scanline counter (or wraps the field) and
/// makes forward progress.
#[test]
fn step_scanline_advances_scanline_or_field() {
    let mut machine = boot_machine();
    let start_line = machine.current_scanline();
    let pc0 = machine.cpu.pc;
    DebuggerPanel::step_scanline(&mut machine);
    assert!(machine.current_scanline() != start_line || machine.cpu.pc != pc0);
}
