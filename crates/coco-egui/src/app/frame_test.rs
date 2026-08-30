use super::background_repaint_delay;
use crate::BACKGROUND_REPAINT_INTERVAL;

#[test]
fn no_viewports_never_throttles() {
    assert_eq!(background_repaint_delay([]), None);
}

#[test]
fn all_unfocused_throttles() {
    assert_eq!(
        background_repaint_delay([Some(false), Some(false)]),
        Some(BACKGROUND_REPAINT_INTERVAL)
    );
}

#[test]
fn any_focused_window_keeps_full_rate() {
    assert_eq!(background_repaint_delay([Some(false), Some(true)]), None);
}

#[test]
fn unknown_focus_counts_as_focused() {
    assert_eq!(background_repaint_delay([Some(false), None]), None);
}

/// Boots a default CoCo 3 from the installed `coco3.rom`.
fn boot() -> crate::CocoApp {
    let rom_path = crate::installed_roms_dir().join(crate::rom_load::COCO3_ROM_FILE);
    let rom = std::fs::read(&rom_path)
        .expect("installed coco3.rom is required (ensure_assets)")
        .into_boxed_slice();
    crate::CocoApp::new(
        crate::MachineConfig::default(),
        rom,
        crate::ROMSource::File(rom_path),
        crate::AppParams::default(),
    )
}

#[test]
fn a_breakpoint_hit_inside_the_cushion_run_is_not_overrun() {
    let mut app = boot();
    // Park a breakpoint on a PC the boot code is looping through, so the
    // cushion run trips it.
    app.run_fields(1);
    let bp = app.machine.cpu.pc;
    app.debugger.add_breakpoint(bp);
    app.last_update = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));

    app.run_emulation_fields(Some(BACKGROUND_REPAINT_INTERVAL));

    assert!(
        !app.running,
        "boot code must revisit {bp:#06X} within the cushion"
    );
    assert_eq!(
        app.machine.cpu.pc, bp,
        "the owed fields must not run past the breakpoint"
    );
}

#[test]
fn entering_the_throttle_runs_a_cushion_ahead_and_leaving_owes_it_back() {
    let mut app = boot();

    app.adjust_audio_cushion(Some(BACKGROUND_REPAINT_INTERVAL));
    let cushion = app.audio_cushion_fields;
    assert!(cushion > 0, "one interval of fields must run ahead");
    assert!(
        app.machine.take_audio().count() > 0,
        "cushion fields must execute and emit audio"
    );
    assert_eq!(
        app.field_debt, 0.0,
        "the cushion is not charged to the clock"
    );

    // Staying throttled adds nothing more.
    app.adjust_audio_cushion(Some(BACKGROUND_REPAINT_INTERVAL));
    assert_eq!(app.audio_cushion_fields, cushion);

    app.adjust_audio_cushion(None);
    assert_eq!(app.audio_cushion_fields, 0);
    assert_eq!(
        app.field_debt,
        -(cushion as f64),
        "leaving owes the cushion back"
    );
    assert_eq!(
        app.fields_due(),
        0,
        "the clock idles while the cushion plays out"
    );
}

#[test]
fn fields_run_increments_once_per_completed_field() {
    let mut app = boot();
    assert_eq!(app.fields_run, 0);
    app.run_fields(5);
    assert_eq!(app.fields_run, 5);
}

/// A field that trips a breakpoint didn't complete, so it must not be
/// counted. Polls one field at a time (rather than assuming how many boot
/// fields elapse before the breakpoint is revisited, like
/// `a_breakpoint_hit_inside_the_cushion_run_is_not_overrun` does) so the
/// assertion holds regardless of that timing.
#[test]
fn fields_run_does_not_count_a_field_that_breaks_on_a_breakpoint() {
    const SAFETY_CAP: usize = 100_000;

    let mut app = boot();
    app.run_fields(1);
    let bp = app.machine.cpu.pc;
    app.debugger.add_breakpoint(bp);

    let mut before = app.fields_run;
    for _ in 0..SAFETY_CAP {
        app.run_fields(1);
        if !app.running {
            assert_eq!(
                app.fields_run, before,
                "the field that broke on the breakpoint must not be counted"
            );
            return;
        }
        before = app.fields_run;
    }
    panic!("breakpoint at {bp:#06X} was never revisited within {SAFETY_CAP} fields");
}

#[test]
fn remote_type_ahead_advances_alongside_run_fields() {
    let mut app = boot();
    app.start_remote_typing("A").expect("running VM accepts");
    assert!(app.remote_type_ahead.is_active());

    while app.remote_type_ahead.is_active() {
        app.run_fields(1);
    }
    assert!(!app.remote_type_ahead.is_active());
}

#[test]
fn remote_held_releases_its_keys_after_exactly_fields_left_fields() {
    const ALL_COLUMNS_STROBED: u8 = 0x00;
    const NO_KEYS_DOWN: u8 = 0xFF;
    const HOLD_FIELDS: u32 = 3;

    let mut app = boot();
    app.start_remote_hold(&["A".to_string()], Some(HOLD_FIELDS))
        .expect("hold succeeds");
    assert_ne!(
        app.machine.bus.keyboard.sense(ALL_COLUMNS_STROBED),
        NO_KEYS_DOWN,
        "the key must be down immediately"
    );

    app.run_fields(HOLD_FIELDS as usize);
    assert!(
        app.remote_held.is_some(),
        "must still be held after HOLD_FIELDS fields (fields_left counts down to 0, released on the next)"
    );

    app.run_fields(1);
    assert!(app.remote_held.is_none(), "must be released and cleared");
    assert_eq!(
        app.machine.bus.keyboard.sense(ALL_COLUMNS_STROBED),
        NO_KEYS_DOWN,
        "the held key must be released on the matrix"
    );
}
