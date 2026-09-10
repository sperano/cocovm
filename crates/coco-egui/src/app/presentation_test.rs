use super::{NOISE_INTERVAL, Presentation};
use crate::display::{Display, MAX_PCT, TV, TVSettings};
use coco_core::MonitorType;
use std::time::{Duration, Instant};

const WIDTH: usize = 4;
const HEIGHT: usize = 3;
const PIXEL: [u8; 4] = [80, 120, 160, 255];
const RGB: Display = Display::Monitor(MonitorType::RGB);
const COLOR_TV: Display = Display::TV(TV::Color);
const STATIC: TVSettings = TVSettings {
    scanline_pct: 0,
    noise_pct: 0,
    overscan_pct: 0,
};
const SNOW: TVSettings = TVSettings {
    noise_pct: 25,
    ..STATIC
};

fn source() -> Vec<u8> {
    PIXEL.repeat(WIDTH * HEIGHT)
}

#[test]
fn identical_pixels_hit_even_when_source_allocation_changes() {
    let mut presentation = Presentation::default();
    let now = Instant::now();
    let src = source();
    assert!(
        presentation
            .prepare(RGB, STATIC, WIDTH, &src, now)
            .is_some()
    );
    assert!(
        presentation
            .prepare(RGB, STATIC, WIDTH, &src.clone(), now)
            .is_none()
    );
    assert!(
        presentation
            .prepare(RGB, STATIC, WIDTH, &src, now + Duration::from_secs(1))
            .is_none()
    );
}

#[test]
fn changed_pixels_upload_once_and_reverting_pixels_uploads_again() {
    let mut presentation = Presentation::default();
    let now = Instant::now();
    let src = source();
    let original = presentation.prepare(RGB, STATIC, WIDTH, &src, now).unwrap();
    let mut changed = src.clone();
    changed[0] += 1;
    let image = presentation
        .prepare(RGB, STATIC, WIDTH, &changed, now)
        .unwrap();
    assert_ne!(image.pixels, original.pixels);
    assert!(
        presentation
            .prepare(RGB, STATIC, WIDTH, &changed, now)
            .is_none()
    );
    assert_eq!(
        presentation
            .prepare(RGB, STATIC, WIDTH, &src, now)
            .unwrap()
            .pixels,
        original.pixels
    );
}

#[test]
fn width_and_height_changes_invalidate() {
    const NARROW_WIDTH: usize = WIDTH / 2;
    let mut presentation = Presentation::default();
    let now = Instant::now();
    let mut src = source();
    assert_eq!(
        presentation
            .prepare(RGB, STATIC, WIDTH, &src, now)
            .unwrap()
            .size,
        [WIDTH, HEIGHT]
    );
    assert_eq!(
        presentation
            .prepare(RGB, STATIC, NARROW_WIDTH, &src, now)
            .unwrap()
            .size,
        [NARROW_WIDTH, HEIGHT * 2]
    );
    src.extend_from_slice(&PIXEL.repeat(NARROW_WIDTH));
    assert_eq!(
        presentation
            .prepare(RGB, STATIC, NARROW_WIDTH, &src, now)
            .unwrap()
            .size,
        [NARROW_WIDTH, HEIGHT * 2 + 1]
    );
    assert!(
        presentation
            .prepare(RGB, STATIC, NARROW_WIDTH, &src, now)
            .is_none()
    );
}

#[test]
fn display_and_tv_pixel_settings_invalidate_once() {
    let mut presentation = Presentation::default();
    let now = Instant::now();
    let src = source();
    let scanlines = TVSettings {
        scanline_pct: 35,
        ..STATIC
    };
    for (display, settings) in [
        (RGB, STATIC),
        (Display::Monitor(MonitorType::Composite), STATIC),
        (COLOR_TV, STATIC),
        (Display::TV(TV::BW), STATIC),
        (Display::TV(TV::BW), scanlines),
        (Display::TV(TV::BW), SNOW),
    ] {
        assert!(
            presentation
                .prepare(display, settings, WIDTH, &src, now)
                .is_some()
        );
        assert!(
            presentation
                .prepare(display, settings, WIDTH, &src, now)
                .is_none()
        );
    }
}

#[test]
fn monitor_ignores_tv_knobs_and_tv_overscan_only_changes_uv() {
    let mut presentation = Presentation::default();
    let now = Instant::now();
    let src = source();
    presentation.prepare(RGB, STATIC, WIDTH, &src, now).unwrap();
    assert!(
        presentation
            .prepare(RGB, TVSettings::default(), WIDTH, &src, now)
            .is_none()
    );
    presentation
        .prepare(COLOR_TV, STATIC, WIDTH, &src, now)
        .unwrap();
    let cropped = TVSettings {
        overscan_pct: 5,
        ..STATIC
    };
    assert_ne!(
        crate::display::texture_uv(COLOR_TV, STATIC),
        crate::display::texture_uv(COLOR_TV, cropped)
    );
    assert!(
        presentation
            .prepare(COLOR_TV, cropped, WIDTH, &src, now)
            .is_none()
    );
}

#[test]
fn clamped_settings_share_the_same_key() {
    let mut presentation = Presentation::default();
    let now = Instant::now();
    let src = source();
    let maximum = TVSettings {
        scanline_pct: MAX_PCT,
        noise_pct: MAX_PCT,
        ..STATIC
    };
    let excessive = TVSettings {
        scanline_pct: u8::MAX,
        noise_pct: u8::MAX,
        ..STATIC
    };
    let expected = presentation
        .prepare(COLOR_TV, maximum, WIDTH, &src, now)
        .unwrap();
    assert!(
        presentation
            .prepare(COLOR_TV, excessive, WIDTH, &src, now)
            .is_none()
    );
    let actual = Presentation::default()
        .prepare(COLOR_TV, excessive, WIDTH, &src, now)
        .unwrap();
    assert_eq!(expected.pixels, actual.pixels);
}

#[test]
fn snow_changes_at_tick_boundaries_and_skips_missed_ticks() {
    const MISSED_TICKS: u32 = 17;
    let mut presentation = Presentation::default();
    let now = Instant::now();
    let src = source();
    let first = presentation
        .prepare(COLOR_TV, SNOW, WIDTH, &src, now)
        .unwrap();
    let before_tick = now + NOISE_INTERVAL - Duration::from_nanos(1);
    assert!(
        presentation
            .prepare(COLOR_TV, SNOW, WIDTH, &src, before_tick)
            .is_none()
    );
    let second = presentation
        .prepare(COLOR_TV, SNOW, WIDTH, &src, now + NOISE_INTERVAL)
        .unwrap();
    assert_ne!(first.pixels, second.pixels);
    let after_stall = now + NOISE_INTERVAL * MISSED_TICKS;
    let stalled = presentation
        .prepare(COLOR_TV, SNOW, WIDTH, &src, after_stall)
        .unwrap();
    let mut regular = Presentation::default();
    for tick in 0..MISSED_TICKS {
        regular.prepare(COLOR_TV, SNOW, WIDTH, &src, now + NOISE_INTERVAL * tick);
    }
    let expected = regular
        .prepare(COLOR_TV, SNOW, WIDTH, &src, after_stall)
        .unwrap();
    assert_eq!(stalled.pixels, expected.pixels);
}

#[test]
fn incidental_repaints_do_not_advance_snow() {
    const EXTRA_REPAINTS: u32 = 20;
    let mut busy = Presentation::default();
    let mut quiet = Presentation::default();
    let now = Instant::now();
    let src = source();
    busy.prepare(COLOR_TV, SNOW, WIDTH, &src, now).unwrap();
    quiet.prepare(COLOR_TV, SNOW, WIDTH, &src, now).unwrap();
    for repaint in 1..EXTRA_REPAINTS {
        let time = now + NOISE_INTERVAL * repaint / EXTRA_REPAINTS;
        assert!(busy.prepare(COLOR_TV, SNOW, WIDTH, &src, time).is_none());
    }
    let busy_frame = busy
        .prepare(COLOR_TV, SNOW, WIDTH, &src, now + NOISE_INTERVAL)
        .unwrap();
    let quiet_frame = quiet
        .prepare(COLOR_TV, SNOW, WIDTH, &src, now + NOISE_INTERVAL)
        .unwrap();
    assert_eq!(busy_frame.pixels, quiet_frame.pixels);
}

#[test]
fn invalidation_rebuilds_unchanged_texture_once() {
    let mut presentation = Presentation::default();
    let now = Instant::now();
    let src = source();
    presentation.prepare(RGB, STATIC, WIDTH, &src, now).unwrap();
    presentation.invalidate();
    assert!(
        presentation
            .prepare(RGB, STATIC, WIDTH, &src, now)
            .is_some()
    );
    assert!(
        presentation
            .prepare(RGB, STATIC, WIDTH, &src, now)
            .is_none()
    );
}

#[test]
fn thumbnail_processing_does_not_mark_pixels_as_presented() {
    let mut presentation = Presentation::default();
    let now = Instant::now();
    let src = source();
    presentation
        .prepare(COLOR_TV, STATIC, WIDTH, &src, now)
        .unwrap();
    let changed = [120, 80, 160, 255].repeat(WIDTH * HEIGHT);
    let _ = presentation.snapshot(COLOR_TV, STATIC, WIDTH, &changed);
    assert!(
        presentation
            .prepare(COLOR_TV, STATIC, WIDTH, &changed, now)
            .is_some()
    );
    let _ = presentation.snapshot(Display::TV(TV::BW), STATIC, WIDTH, &src);
    assert!(
        presentation
            .prepare(COLOR_TV, STATIC, WIDTH, &changed, now)
            .is_none()
    );
}
