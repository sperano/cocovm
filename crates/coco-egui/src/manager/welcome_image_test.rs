//! `WelcomeImage`'s timer and crossfade: disarmed unless the cycle is on
//! and the pane is showing, one change per interval, in file-name order or
//! shuffled without repeats, and a fade that runs from 0 to 1 once.

use std::time::{Duration, Instant};

use super::*;
use crate::machine_def::tests::TempDir;

const INTERVAL: Duration = Duration::from_secs(10);

/// A 2×2 opaque photo titled `title`.
fn photo(title: &str) -> Photo {
    Photo {
        title: title.to_string(),
        pixels: egui::ColorImage::from_rgba_unmultiplied([2, 2], &[0xFF; 16]),
    }
}

/// A welcome image with the cycle on, a 10-second interval, and an images
/// dir holding `page-1.png` through `page-3.png`.
fn cycling(dir: &TempDir) -> WelcomeImage {
    for name in ["page-1.png", "page-2.png", "page-3.png"] {
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
            .save(dir.path().join(name))
            .unwrap();
    }
    let mut welcome = WelcomeImage::new(None);
    welcome.images_dir = Some(dir.path().to_path_buf());
    welcome.cycle = true;
    welcome.cycle_secs = NonZeroU32::new(10).unwrap();
    welcome
}

/// [`cycling`] with `title` already uploaded as the showing texture.
fn cycling_showing(dir: &TempDir, ctx: &egui::Context, title: &str) -> WelcomeImage {
    let mut welcome = cycling(dir);
    welcome.photo = Some(photo(title));
    welcome.upload(ctx, Instant::now());
    welcome
}

/// Runs one full interval from a fresh arm and returns the queued title.
fn title_after_one_interval(welcome: &mut WelcomeImage) -> Option<String> {
    let start = Instant::now();
    welcome.tick(start, true);
    welcome.tick(start + INTERVAL, true);
    welcome.photo.as_ref().map(|p| p.title.clone())
}

#[test]
fn cycle_off_never_arms_the_timer() {
    let dir = TempDir::new("welcome-cycle-off");
    let mut welcome = cycling(&dir);
    welcome.cycle = false;

    assert_eq!(welcome.tick(Instant::now(), true), None);
    assert!(welcome.due.is_none());
    assert!(welcome.photo.is_none());
}

#[test]
fn first_tick_arms_without_changing_and_the_change_lands_one_interval_later() {
    let dir = TempDir::new("welcome-cycle-arm");
    let mut welcome = cycling(&dir);
    let start = Instant::now();

    assert_eq!(welcome.tick(start, true), Some(start + INTERVAL));
    assert!(welcome.photo.is_none(), "arming must not change the image");

    let early = start + INTERVAL - Duration::from_millis(1);
    assert_eq!(welcome.tick(early, true), Some(start + INTERVAL));
    assert!(welcome.photo.is_none(), "no change before the deadline");

    let due = start + INTERVAL;
    assert_eq!(welcome.tick(due, true), Some(due + INTERVAL));
    assert!(welcome.photo.is_some(), "the deadline changes the image");
}

#[test]
fn in_order_picks_the_next_file_name_and_wraps_around() {
    let dir = TempDir::new("welcome-cycle-ordered");
    let ctx = egui::Context::default();

    let mut welcome = cycling_showing(&dir, &ctx, "page-2");
    assert_eq!(
        title_after_one_interval(&mut welcome).as_deref(),
        Some("page-3")
    );

    let mut welcome = cycling_showing(&dir, &ctx, "page-3");
    assert_eq!(
        title_after_one_interval(&mut welcome).as_deref(),
        Some("page-1")
    );
}

#[test]
fn in_order_starts_from_the_first_file_when_nothing_is_showing() {
    let dir = TempDir::new("welcome-cycle-ordered-blank");
    let mut welcome = cycling(&dir);
    assert_eq!(
        title_after_one_interval(&mut welcome).as_deref(),
        Some("page-1")
    );
}

#[test]
fn shuffled_never_repeats_the_image_showing() {
    let dir = TempDir::new("welcome-cycle-shuffled");
    let ctx = egui::Context::default();
    for _ in 0..8 {
        let mut welcome = cycling_showing(&dir, &ctx, "page-2");
        welcome.shuffle = true;
        let title = title_after_one_interval(&mut welcome).expect("a change lands");
        assert_ne!(title, "page-2");
    }
}

#[test]
fn hiding_the_pane_disarms_the_timer() {
    let dir = TempDir::new("welcome-cycle-hidden");
    let mut welcome = cycling(&dir);
    let start = Instant::now();
    welcome.tick(start, true);
    assert!(welcome.due.is_some());

    assert_eq!(welcome.tick(start + INTERVAL, false), None);
    assert!(welcome.due.is_none());
    assert!(
        welcome.photo.is_none(),
        "no change while the pane is hidden"
    );
}

#[test]
fn a_missing_images_dir_keeps_the_schedule_but_changes_nothing() {
    let dir = TempDir::new("welcome-cycle-no-dir");
    let mut welcome = cycling(&dir);
    welcome.images_dir = None;
    let start = Instant::now();
    welcome.tick(start, true);

    assert_eq!(
        welcome.tick(start + INTERVAL, true),
        Some(start + INTERVAL + INTERVAL)
    );
    assert!(welcome.photo.is_none());
}

#[test]
fn the_first_upload_shows_without_a_fade() {
    let dir = TempDir::new("welcome-fade-first");
    let ctx = egui::Context::default();
    let welcome = cycling_showing(&dir, &ctx, "page-1");

    assert_eq!(
        welcome.texture.as_ref().map(|t| t.name()),
        Some("page-1".to_string())
    );
    assert!(welcome.fade.is_none());
}

#[test]
fn a_change_crossfades_from_the_previous_image_then_drops_it() {
    let dir = TempDir::new("welcome-fade-change");
    let ctx = egui::Context::default();
    let start = Instant::now();
    let mut welcome = cycling_showing(&dir, &ctx, "page-1");
    welcome.photo = Some(photo("page-2"));
    welcome.upload(&ctx, start);

    let fade = welcome.fade.as_ref().expect("a change starts a fade");
    assert_eq!(fade.previous.name(), "page-1");
    assert_eq!(welcome.fade_progress(&ctx, start), 0.0);
    let midway = welcome.fade_progress(&ctx, start + FADE_DURATION / 2);
    assert!((midway - 0.5).abs() < 0.01, "midway progress {midway}");
    assert!(welcome.fade.is_some(), "still fading midway");

    assert_eq!(welcome.fade_progress(&ctx, start + FADE_DURATION), 1.0);
    assert!(welcome.fade.is_none(), "the fade ends on time");
    assert_eq!(welcome.fade_progress(&ctx, start + FADE_DURATION), 1.0);
}
