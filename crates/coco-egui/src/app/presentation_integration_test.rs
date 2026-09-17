use crate::CocoApp;
use crate::display::{Display, TV, TVSettings};
use eframe::egui;
use std::time::Instant;

const BOOT_FIELDS: usize = 120;
const STEP_LIMIT: usize = 100_000;
const SCREEN_BASE: u16 = 0x0400;
const INVERSE_GLYPH: u8 = 0x40;
const STATIC_TV: TVSettings = TVSettings {
    scanline_pct: 0,
    noise_pct: 0,
    overscan_pct: 0,
};

fn app() -> CocoApp {
    let path = test_assets::rom(test_assets::rom::COCO3);
    let rom = std::fs::read(&path)
        .expect("installed CoCo 3 ROM")
        .into_boxed_slice();
    CocoApp::new(
        crate::MachineConfig::default(),
        rom,
        crate::ROMSource::File(path),
        crate::AppParams::default(),
        crate::joy::SharedGamepad::without_backend(),
    )
}

fn present(app: &mut CocoApp, ctx: &egui::Context, now: Instant) -> Vec<egui::epaint::ImageDelta> {
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        app.present_framebuffer(ctx, now)
    });
    framebuffer_deltas(app, output)
}

fn framebuffer_deltas(app: &CocoApp, output: egui::FullOutput) -> Vec<egui::epaint::ImageDelta> {
    let id = app.texture.as_ref().expect("framebuffer texture").id();
    output
        .textures_delta
        .set
        .into_iter()
        .filter_map(|(texture, delta)| (texture == id).then_some(delta))
        .collect()
}

fn assert_upload_once(app: &mut CocoApp, ctx: &egui::Context, now: Instant) {
    assert_eq!(present(app, ctx, now).len(), 1);
    assert!(present(app, ctx, now).is_empty());
}

fn assert_uploaded_framebuffer(app: &mut CocoApp, ctx: &egui::Context, now: Instant) {
    let expected = egui::ColorImage::from_rgba_unmultiplied(
        [
            app.machine.fb_width as usize,
            app.machine.fb_height as usize,
        ],
        &app.machine.framebuffer,
    );
    let deltas = present(app, ctx, now);
    assert_eq!(deltas.len(), 1);
    let egui::ImageData::Color(actual) = &deltas[0].image;
    assert_eq!(actual.size, expected.size);
    assert_eq!(actual.pixels, expected.pixels);
    assert!(present(app, ctx, now).is_empty());
}

fn step_scanline(app: &mut CocoApp) {
    let line = app.machine.current_scanline();
    for _ in 0..STEP_LIMIT {
        let event = app.machine.step_instruction();
        if event.field_complete || app.machine.current_scanline() != line {
            return;
        }
    }
    panic!("scanline did not advance");
}

fn check_idle_presentations(display: Display) {
    let mut app = app();
    app.display = display;
    app.tv = TVSettings {
        noise_pct: 0,
        ..TVSettings::default()
    };
    let ctx = egui::Context::default();
    let now = Instant::now();
    assert_upload_once(&mut app, &ctx, now);
    app.running = false;
    let paused = ctx.run(egui::RawInput::default(), |ctx| {
        app.step_emulation(ctx, None)
    });
    assert!(framebuffer_deltas(&app, paused).is_empty());
    app.suspended = true;
    let suspended = ctx.run(egui::RawInput::default(), |ctx| app.window_ui(ctx, None));
    assert!(framebuffer_deltas(&app, suspended).is_empty());
    app.suspended = false;
    app.running = true;
    app.last_update = None;
    let resumed = ctx.run(egui::RawInput::default(), |ctx| {
        app.step_emulation(ctx, None)
    });
    assert_eq!(framebuffer_deltas(&app, resumed).len(), 1);
    const FOREGROUND_STARTUP_FIELDS: u64 = 2;
    assert_eq!(app.fields_run, FOREGROUND_STARTUP_FIELDS);
    assert!(present(&mut app, &ctx, now).is_empty());
    app.run_fields(BOOT_FIELDS);
    assert_upload_once(&mut app, &ctx, now);
}

#[test]
fn startup_cushion_upload_is_then_cached_while_paused_or_suspended() {
    for display in [
        Display::Monitor(coco_core::MonitorType::RGB),
        Display::TV(TV::Color),
        Display::TV(TV::BW),
    ] {
        check_idle_presentations(display);
    }
}

#[test]
fn paused_and_suspended_snow_only_uploads_at_animation_ticks() {
    use crate::app::presentation::NOISE_INTERVAL;
    for suspended in [false, true] {
        let mut app = app();
        let ctx = egui::Context::default();
        let now = Instant::now();
        app.running = false;
        app.suspended = suspended;
        app.display = Display::TV(TV::Color);
        assert_upload_once(&mut app, &ctx, now);
        assert!(present(&mut app, &ctx, now + NOISE_INTERVAL / 2).is_empty());
        assert_upload_once(&mut app, &ctx, now + NOISE_INTERVAL);
        assert_eq!(app.fields_run, 0);
    }
}

#[test]
fn incidental_paused_repaints_do_not_upload_background_snow_early() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.running = false;
    app.display = Display::TV(TV::Color);
    let mut input = egui::RawInput::default();
    input.viewports.insert(
        egui::ViewportId::ROOT,
        egui::ViewportInfo {
            focused: Some(false),
            minimized: Some(false),
            ..Default::default()
        },
    );

    use crate::app::presentation::NOISE_INTERVAL;
    let now = Instant::now();
    let initial = ctx.run(input.clone(), |ctx| {
        app.upload_framebuffer_texture_at(ctx, now)
    });
    assert_eq!(framebuffer_deltas(&app, initial).len(), 1);
    let incidental = ctx.run(input, |ctx| {
        app.upload_framebuffer_texture_at(ctx, now + NOISE_INTERVAL / 2)
    });
    assert!(framebuffer_deltas(&app, incidental).is_empty());
}

#[test]
fn instruction_step_presents_partial_render_without_a_completed_field() {
    let mut app = app();
    let ctx = egui::Context::default();
    let now = Instant::now();
    app.running = false;
    assert_upload_once(&mut app, &ctx, now);
    let initial = app.machine.framebuffer.clone();
    for _ in 0..STEP_LIMIT {
        let event = app.machine.step_instruction();
        if app.machine.framebuffer != initial {
            assert!(!event.field_complete);
            assert_eq!(app.fields_run, 0);
            assert_upload_once(&mut app, &ctx, now);
            return;
        }
    }
    panic!("instruction stepping never produced pixels");
}

#[test]
fn scanline_step_presents_partial_render_without_running_a_field() {
    let mut app = app();
    let ctx = egui::Context::default();
    let now = Instant::now();
    app.running = false;
    assert_upload_once(&mut app, &ctx, now);
    let initial = app.machine.framebuffer.clone();
    for _ in 0..STEP_LIMIT {
        step_scanline(&mut app);
        if app.machine.framebuffer != initial {
            assert_eq!(app.fields_run, 0);
            assert_upload_once(&mut app, &ctx, now);
            return;
        }
    }
    panic!("scanline stepping never produced pixels");
}

#[test]
fn screen_poke_waits_for_render_before_uploading() {
    let mut app = app();
    let ctx = egui::Context::default();
    let now = Instant::now();
    app.run_fields(BOOT_FIELDS);
    app.running = false;
    assert_upload_once(&mut app, &ctx, now);
    let before = app.machine.framebuffer.clone();
    // The same BASIC text screen and inverse bit used by core render tests.
    let glyph = app.peek_bytes(SCREEN_BASE, 1)[0] ^ INVERSE_GLYPH;
    app.poke_bytes(SCREEN_BASE, &[glyph]).unwrap();
    assert_eq!(app.machine.framebuffer, before);
    assert!(present(&mut app, &ctx, now).is_empty());
    app.machine.run_field();
    assert_ne!(app.machine.framebuffer, before);
    assert_upload_once(&mut app, &ctx, now);
}

#[test]
fn reset_and_power_cycle_upload_when_the_renderer_changes_pixels() {
    let mut app = app();
    let ctx = egui::Context::default();
    let now = Instant::now();
    app.run_fields(BOOT_FIELDS);
    assert_upload_once(&mut app, &ctx, now);
    for hard in [false, true] {
        app.remote_reset(hard);
        assert!(present(&mut app, &ctx, now).is_empty());
        let before = app.machine.framebuffer.clone();
        for _ in 0..BOOT_FIELDS {
            app.machine.run_field();
            if app.machine.framebuffer != before {
                break;
            }
        }
        assert_ne!(
            app.machine.framebuffer, before,
            "reset must eventually render"
        );
        assert_upload_once(&mut app, &ctx, now);
    }
}

#[test]
fn snapshot_load_uploads_rebuilt_framebuffer_then_newly_rendered_pixels() {
    let mut app = app();
    let ctx = egui::Context::default();
    let now = Instant::now();
    app.run_fields(BOOT_FIELDS);
    assert_upload_once(&mut app, &ctx, now);
    let path = std::env::temp_dir().join(format!(
        "cocovm-presentation-{}.ccstate",
        std::process::id()
    ));
    app.save_state_to(&path).expect("save state");
    app.power_cycle();
    app.machine.run_field();
    assert_upload_once(&mut app, &ctx, now);
    let before_load = app.machine.framebuffer.clone();
    app.load_state_from(&path).expect("restore state");
    std::fs::remove_file(path).expect("remove test state");
    // Machine::after_restore rebuilds the skipped framebuffer with zero bytes.
    assert!(app.machine.framebuffer.iter().all(|byte| *byte == 0));
    assert_ne!(app.machine.framebuffer, before_load);
    assert_uploaded_framebuffer(&mut app, &ctx, now);
    let restored = app.machine.framebuffer.clone();
    app.machine.run_field();
    assert_ne!(app.machine.framebuffer, restored);
    assert_uploaded_framebuffer(&mut app, &ctx, now);
}

#[test]
fn framebuffer_dimensions_and_display_settings_reach_texture_metadata() {
    const NATIVE_WIDTH: u32 = 16;
    const NATIVE_HEIGHT: u32 = 12;
    const PIXEL: [u8; 4] = [80, 120, 160, 255];
    let mut app = app();
    let ctx = egui::Context::default();
    let now = Instant::now();
    assert_upload_once(&mut app, &ctx, now);
    app.machine.fb_width = NATIVE_WIDTH;
    app.machine.fb_height = NATIVE_HEIGHT;
    app.machine.framebuffer = PIXEL.repeat((NATIVE_WIDTH * NATIVE_HEIGHT) as usize);
    let resized = present(&mut app, &ctx, now);
    assert_eq!(resized.len(), 1);
    assert_eq!(
        resized[0].image.size(),
        [NATIVE_WIDTH as usize, NATIVE_HEIGHT as usize]
    );
    assert_eq!(resized[0].options, egui::TextureOptions::NEAREST);
    app.display = Display::TV(TV::Color);
    app.tv = STATIC_TV;
    let tv = present(&mut app, &ctx, now);
    assert_eq!(tv.len(), 1);
    assert_eq!(tv[0].options, egui::TextureOptions::LINEAR);
    app.tv.scanline_pct = TVSettings::default().scanline_pct;
    let scanlines = present(&mut app, &ctx, now);
    assert_eq!(scanlines.len(), 1);
    assert_eq!(
        scanlines[0].image.size(),
        [NATIVE_WIDTH as usize, NATIVE_HEIGHT as usize * 2]
    );
    assert!(present(&mut app, &ctx, now).is_empty());
    app.tv.noise_pct = TVSettings::default().noise_pct;
    assert_upload_once(&mut app, &ctx, now);
    app.display = Display::TV(TV::BW);
    assert_upload_once(&mut app, &ctx, now);
}

#[test]
fn window_resize_aspect_and_overscan_reuse_texture() {
    const WINDOW_SIZE: egui::Vec2 = egui::vec2(900.0, 600.0);
    let mut app = app();
    let ctx = egui::Context::default();
    let now = Instant::now();
    app.display = Display::TV(TV::Color);
    app.tv = STATIC_TV;
    assert_upload_once(&mut app, &ctx, now);
    app.aspect_correct = !app.aspect_correct;
    app.tv.overscan_pct = TVSettings::default().overscan_pct;
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, WINDOW_SIZE)),
        ..Default::default()
    };
    let output = ctx.run(input, |ctx| {
        app.present_framebuffer(ctx, now);
        egui::CentralPanel::default().show(ctx, |ui| app.draw_display(ui));
    });
    assert!(framebuffer_deltas(&app, output).is_empty());
}

#[test]
fn dropped_texture_is_recreated_even_with_cached_pixels() {
    let mut app = app();
    let ctx = egui::Context::default();
    let now = Instant::now();
    assert_upload_once(&mut app, &ctx, now);
    app.texture = None;
    assert_upload_once(&mut app, &ctx, now);
}

#[cfg(feature = "perf")]
#[test]
#[ignore = "run alone: process-wide frontend counters"]
fn first_upload_enqueues_once_and_cache_hits_do_not_convert_or_enqueue() {
    const CACHE_HITS: usize = 5;
    let mut app = app();
    let ctx = egui::Context::default();
    let now = Instant::now();
    crate::perf::reset();
    assert_eq!(present(&mut app, &ctx, now).len(), 1);
    let first = crate::perf::snapshot();
    assert_eq!(first["texture_enqueue_cpu"]["count"], 1);
    assert_eq!(
        first["texture_enqueue_cpu"]["bytes"],
        app.machine.framebuffer.len()
    );
    assert_eq!(first["stages"]["display_conversion"]["count"], 1);
    for _ in 0..CACHE_HITS {
        assert!(present(&mut app, &ctx, now).is_empty());
    }
    let cached = crate::perf::snapshot();
    assert_eq!(cached["texture_enqueue_cpu"], first["texture_enqueue_cpu"]);
    assert_eq!(cached["stages"]["display_conversion"]["count"], 1);
}
