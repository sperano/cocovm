//! Coverage for `scale_active_rect` — the framebuffer→screen mapping behind
//! `CocoApp::active_screen_rect`. Pure geometry, so no `CocoApp` needed.

use coco_core::ActiveRect;
use eframe::egui;

use crate::rom_load::load_default_rom;
use crate::{AppParams, CocoApp, MachineConfig};

use super::scale_active_rect;

/// The CoCo 3 canvas dims (`raster::CANVAS_W`/`CANVAS_H`) the fixtures scale
/// from.
const FB_W: u32 = 640;
const FB_H: u32 = 240;
/// A non-wide, LPF-192 active rect: 512×192 at (64, 25).
const ACTIVE: ActiveRect = ActiveRect {
    x: 64,
    y: 25,
    width: 512,
    height: 192,
};

fn full_uv() -> egui::Rect {
    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0))
}

#[test]
fn scales_offsets_and_size_into_the_display_rect() {
    // Origin off (0,0) so a forgotten `left_top()` offset would show.
    let display = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(1280.0, 480.0));
    let r = scale_active_rect(ACTIVE, FB_W, FB_H, display, full_uv());
    assert_eq!(r.left_top(), egui::pos2(100.0 + 128.0, 50.0 + 50.0));
    assert_eq!(r.size(), egui::vec2(1024.0, 384.0));
}

#[test]
fn per_axis_scales_are_independent() {
    // 1× horizontally but 2× vertically — a mixed-up `sx`/`sy` fails here.
    let display = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 480.0));
    let r = scale_active_rect(ACTIVE, FB_W, FB_H, display, full_uv());
    assert_eq!(r.left_top(), egui::pos2(64.0, 50.0));
    assert_eq!(r.size(), egui::vec2(512.0, 384.0));
}

#[test]
fn zero_size_framebuffer_falls_back_to_the_display_rect() {
    let display = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 480.0));
    assert_eq!(
        scale_active_rect(ACTIVE, 0, FB_H, display, full_uv()),
        display
    );
    assert_eq!(
        scale_active_rect(ACTIVE, FB_W, 0, display, full_uv()),
        display
    );
}

#[test]
fn crop_maps_only_the_visible_source_span_onto_the_display() {
    const CROP: f32 = 0.05;
    const EPSILON: f32 = 0.001;
    const EXPECTED_LEFT: f32 = 171.111_11;
    const EXPECTED_TOP: f32 = 78.888_885;
    const EXPECTED_WIDTH: f32 = 1_137.777_8;
    const EXPECTED_HEIGHT: f32 = 426.666_66;

    let display = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(1280.0, 480.0));
    let uv = egui::Rect::from_min_max(egui::pos2(CROP, CROP), egui::pos2(1.0 - CROP, 1.0 - CROP));
    let r = scale_active_rect(ACTIVE, FB_W, FB_H, display, uv);

    assert!((r.left() - EXPECTED_LEFT).abs() < EPSILON);
    assert!((r.top() - EXPECTED_TOP).abs() < EPSILON);
    assert!((r.width() - EXPECTED_WIDTH).abs() < EPSILON);
    assert!((r.height() - EXPECTED_HEIGHT).abs() < EPSILON);
}

fn test_app() -> CocoApp {
    let roms_dir = test_assets::roms_dir();
    let (rom, source) = load_default_rom(coco_core::MachineVariant::Coco3, &roms_dir)
        .expect("coco3.rom is required in the cocovm XDG data directory");
    CocoApp::new(MachineConfig::default(), rom, source, AppParams::default())
}

fn run_input_frame(ctx: &egui::Context, app: &mut CocoApp, raw: egui::RawInput) {
    let _ = ctx.run(raw, |ctx| app.handle_input(ctx));
}

fn key_event(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::default(),
    }
}

/// A lost-focus frame must release a positional key without receiving key-up,
/// cancel typeahead, and keep a stale modifier released until focus returns.
#[test]
fn focus_loss_releases_keyboard_state_without_key_up() {
    const ALL_COLUMNS_STROBED: u8 = 0x00;
    const NO_KEYS_DOWN: u8 = 0xFF;

    let ctx = egui::Context::default();
    let mut app = test_app();
    let pressed = egui::RawInput {
        events: vec![key_event(egui::Key::A, true)],
        ..Default::default()
    };
    run_input_frame(&ctx, &mut app, pressed);
    assert_ne!(
        app.machine.bus.keyboard.sense(ALL_COLUMNS_STROBED),
        NO_KEYS_DOWN
    );

    app.enqueue_text("B");
    let stale_modifiers = egui::Modifiers {
        shift: true,
        ..Default::default()
    };
    let lost_focus = egui::RawInput {
        focused: false,
        modifiers: stale_modifiers,
        events: vec![egui::Event::WindowFocused(false)],
        ..Default::default()
    };
    run_input_frame(&ctx, &mut app, lost_focus);
    assert_eq!(
        app.machine.bus.keyboard.sense(ALL_COLUMNS_STROBED),
        NO_KEYS_DOWN
    );
    assert!(!app.type_ahead.is_active());

    let still_unfocused = egui::RawInput {
        focused: false,
        modifiers: stale_modifiers,
        ..Default::default()
    };
    run_input_frame(&ctx, &mut app, still_unfocused);
    assert_eq!(
        app.machine.bus.keyboard.sense(ALL_COLUMNS_STROBED),
        NO_KEYS_DOWN
    );
}

/// A remote `type_text` session survives a focus-loss frame — unlike the
/// host's own `type_ahead`, which `release_keyboard_state` clears — since
/// it's driven from `run_fields`, not from focus-gated input handling.
#[test]
fn remote_type_ahead_survives_a_focus_loss_frame() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.start_remote_typing("B").expect("running VM accepts");
    assert!(app.remote_type_ahead.is_active());

    let lost_focus = egui::RawInput {
        focused: false,
        events: vec![egui::Event::WindowFocused(false)],
        ..Default::default()
    };
    run_input_frame(&ctx, &mut app, lost_focus);

    assert!(
        app.remote_type_ahead.is_active(),
        "remote typing must not be cleared by a host focus-loss frame"
    );
    assert!(
        !app.type_ahead.is_active(),
        "the host's own type_ahead is unaffected here (nothing was queued into it)"
    );
}

/// A `joystick` override wins over the host source, even a port set to
/// `JoySource::None` which recenters every frame. `Joysticks` has no axis/
/// button getters, so the override is checked through `compare` (the
/// comparator the hardware itself reads) and `button_rows`.
#[test]
fn remote_joystick_override_wins_over_the_host_source() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    let right = coco_core::joystick::RIGHT;
    app.apply_remote_joystick(
        crate::control::Stick::Right,
        Some(63),
        Some(0),
        Some(true),
        None,
        false,
    );

    let _ = ctx.run(egui::RawInput::default(), |ctx| app.drive_joysticks(ctx));

    let joy = &app.machine.bus.joysticks;
    assert!(
        joy.compare(right, coco_core::joystick::AXIS_X, 63),
        "x must be at AXIS_MAX (63), not JoySource::None's recentered 32"
    );
    assert!(
        !joy.compare(right, coco_core::joystick::AXIS_Y, 1),
        "y must be at 0, not JoySource::None's recentered 32"
    );
    const RIGHT_BUTTON1_BIT: u8 = 0x01;
    assert_ne!(
        joy.button_rows() & RIGHT_BUTTON1_BIT,
        0,
        "button1 must be held"
    );
}
