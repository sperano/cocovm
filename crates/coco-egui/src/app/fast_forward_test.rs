use std::time::{Duration, Instant};

use super::*;

/// Boots a default CoCo 3 from the installed `coco3.rom`.
fn boot() -> CocoApp {
    let rom_path = crate::installed_roms_dir().join(crate::rom_load::COCO3_ROM_FILE);
    let rom = std::fs::read(&rom_path)
        .expect("installed coco3.rom is required (first-run asset download)")
        .into_boxed_slice();
    CocoApp::new(
        crate::MachineConfig::default(),
        rom,
        crate::ROMSource::File(rom_path),
        crate::AppParams::default(),
        crate::joy::SharedGamepad::without_backend(),
    )
}

/// A slice budget no test's field count can exhaust.
const UNLIMITED_SLICE: Duration = Duration::from_secs(600);
/// Text Super Extended Color BASIC prints once it reaches its prompt.
const BASIC_PROMPT: &str = "OK";
/// Fields that comfortably cover the boot to [`BASIC_PROMPT`].
const BOOT_FIELD_BUDGET: u64 = 3600;
/// A headless output rate for the audio-drop checks.
const DEVICE_RATE_HZ: f64 = 48_000.0;

fn basic_prompt_matcher() -> TextMatcher {
    TextMatcher::new(BASIC_PROMPT.to_string(), false).expect("literal pattern")
}

#[test]
fn start_fast_forward_refuses_a_paused_vm_and_a_second_run() {
    let mut app = boot();
    app.set_running(false);
    assert_eq!(
        app.start_fast_forward(10, None),
        Err(PAUSED_ERROR.to_string())
    );
    assert!(!app.is_fast_forwarding());

    app.set_running(true);
    app.start_fast_forward(10, None).expect("first run starts");
    assert_eq!(
        app.start_fast_forward(20, None),
        Err(FAST_FORWARD_BUSY_ERROR.to_string())
    );
}

#[test]
fn a_target_already_reached_starts_nothing() {
    let mut app = boot();
    app.run_fields(3);
    app.start_fast_forward(app.fields_run, None)
        .expect("accepted as already satisfied");
    assert!(!app.is_fast_forwarding());
}

#[test]
fn a_slice_runs_fields_back_to_back_and_ends_at_the_target() {
    const FIELDS: u64 = 20;
    let mut app = boot();
    app.last_update = Some(Instant::now() - Duration::from_secs(1));
    app.field_debt = 0.5;
    let target = app.fields_run + FIELDS;
    app.start_fast_forward(target, None).expect("run starts");

    let still_running = app.run_fast_forward_slice_until(Instant::now() + UNLIMITED_SLICE);

    assert!(!still_running);
    assert!(!app.is_fast_forwarding());
    assert_eq!(app.fields_run, target, "exactly the requested fields ran");
    assert_eq!(app.last_update, None, "the wall clock restarts afterwards");
    assert_eq!(app.field_debt, 0.0);
}

#[test]
fn a_slice_stops_at_its_budget_and_the_run_continues_next_time() {
    let mut app = boot();
    let start = app.fields_run;
    app.start_fast_forward(u64::MAX, None).expect("run starts");

    // A deadline already in the past: one field, then yield.
    let still_running = app.run_fast_forward_slice_until(Instant::now());

    assert!(still_running);
    assert!(app.is_fast_forwarding());
    assert_eq!(app.fields_run, start + 1);
}

#[test]
fn a_breakpoint_ends_the_run_and_pauses_the_vm() {
    let mut app = boot();
    app.run_fields(1);
    let bp = app.machine.cpu.pc;
    app.debugger.add_breakpoint(bp);
    app.start_fast_forward(u64::MAX, None).expect("run starts");

    let still_running = app.run_fast_forward_slice_until(Instant::now() + UNLIMITED_SLICE);

    assert!(!still_running);
    assert!(!app.running, "boot code must revisit {bp:#06X}");
    assert!(!app.is_fast_forwarding());
    assert_eq!(app.machine.cpu.pc, bp);
}

/// Fields a freshly booted VM runs, one at a time, before its screen
/// matches `matcher`.
fn fields_until_match(matcher: &TextMatcher) -> u64 {
    let mut app = boot();
    while !matcher.is_match(&app.screen_snapshot()) {
        app.run_fields(1);
        assert!(
            app.fields_run < BOOT_FIELD_BUDGET,
            "{BASIC_PROMPT:?} must appear within the boot budget"
        );
    }
    app.fields_run
}

#[test]
fn a_text_match_ends_the_run_on_the_first_matching_field() {
    let matcher = basic_prompt_matcher();
    let expected = fields_until_match(&matcher);
    let mut app = boot();
    app.start_fast_forward(BOOT_FIELD_BUDGET, Some(matcher.clone()))
        .expect("run starts");

    let still_running = app.run_fast_forward_slice_until(Instant::now() + UNLIMITED_SLICE);

    assert!(!still_running);
    assert!(!app.is_fast_forwarding());
    assert!(matcher.is_match(&app.screen_snapshot()));
    assert_eq!(app.fields_run, expected, "not a field past the match");
}

#[test]
fn a_run_credits_emulated_time_to_the_runtime_total() {
    const FIELDS: u64 = 30;
    let mut app = boot();
    let target = app.fields_run + FIELDS;
    app.start_fast_forward(target, None).expect("run starts");

    app.run_fast_forward_slice_until(Instant::now() + UNLIMITED_SLICE);

    let field_rate_hz = app.machine.config.video.field_rate_hz();
    let expected = Duration::from_secs_f64(FIELDS as f64 / field_rate_hz);
    assert_eq!(app.total_runtime, expected);
}

#[test]
fn stop_fast_forward_restarts_the_emulation_clock() {
    let mut app = boot();
    app.start_fast_forward(u64::MAX, None).expect("run starts");
    app.last_update = Some(Instant::now() - Duration::from_secs(1));
    app.field_debt = 0.75;
    app.audio_cushion_fields = 2;

    app.stop_fast_forward();

    assert!(!app.is_fast_forwarding());
    assert_eq!(app.last_update, None);
    assert_eq!(app.field_debt, 0.0);
    assert_eq!(app.audio_cushion_fields, 0);
}

#[test]
fn step_emulation_drops_the_audio_of_a_fast_forward_slice() {
    const FIELDS: u64 = 5;
    let mut app = boot();
    app.audio = crate::audio::AudioOutput::headless(DEVICE_RATE_HZ);
    let target = app.fields_run + FIELDS;
    app.start_fast_forward(target, None).expect("run starts");
    let ctx = eframe::egui::Context::default();

    let _ = ctx.run(eframe::egui::RawInput::default(), |ctx| {
        app.step_emulation(ctx, None);
    });

    assert_eq!(app.fields_run, target);
    assert_eq!(app.audio.queued_frames(), 0, "nothing reached the ring");
    assert_eq!(
        app.machine.take_audio().count(),
        0,
        "the slice's samples were drained, not left to pile up"
    );
}

#[test]
fn step_emulation_asks_for_the_next_slice_right_away() {
    let mut app = boot();
    app.start_fast_forward(u64::MAX, None).expect("run starts");
    let (repaint_tx, repaint_rx) = std::sync::mpsc::channel();
    let ctx = eframe::egui::Context::default();
    ctx.set_request_repaint_callback(move |info| {
        let _ = repaint_tx.send(info.delay);
    });

    let _ = ctx.run(eframe::egui::RawInput::default(), |ctx| {
        app.step_emulation(ctx, None);
    });

    assert!(app.is_fast_forwarding(), "one slice can't finish this run");
    let delays: Vec<Duration> = repaint_rx.try_iter().collect();
    assert!(
        delays.contains(&Duration::ZERO),
        "an immediate repaint was requested: {delays:?}"
    );
    app.stop_fast_forward();
}
