use super::*;

const FIELD: Duration = Duration::from_millis(16);
const BACKGROUND: Duration = Duration::from_millis(240);

fn viewport(focused: Option<bool>, minimized: Option<bool>) -> egui::ViewportInfo {
    egui::ViewportInfo {
        focused,
        minimized,
        ..Default::default()
    }
}

#[test]
fn viewport_state_selects_its_own_background_policy() {
    assert_eq!(
        viewport_background_delay(&viewport(Some(true), Some(false))),
        None
    );
    assert_eq!(viewport_background_delay(&viewport(None, None)), None);
    assert_eq!(
        viewport_background_delay(&viewport(Some(false), Some(false))),
        Some(crate::BACKGROUND_REPAINT_INTERVAL)
    );
    assert_eq!(
        viewport_background_delay(&viewport(Some(true), Some(true))),
        Some(crate::BACKGROUND_REPAINT_INTERVAL)
    );
}

#[test]
fn incidental_high_refresh_repaints_do_not_move_the_deadline() {
    let start = Instant::now();
    let mut schedule = Schedule::default();
    assert_eq!(schedule.service_deadline(start, FIELD), start + FIELD);

    for millis in [2, 4, 6, 8, 10, 12, 14] {
        assert_eq!(
            schedule.service_deadline(start + Duration::from_millis(millis), FIELD),
            start + FIELD
        );
    }
    assert_eq!(
        schedule.service_deadline(start + FIELD, FIELD),
        start + 2 * FIELD
    );
}

#[test]
fn host_stall_skips_expired_deadlines_without_shifting_cadence() {
    let start = Instant::now();
    let mut schedule = Schedule::default();
    schedule.service_deadline(start, FIELD);

    let stalled = start + 5 * FIELD + Duration::from_millis(3);
    assert_eq!(schedule.service_deadline(stalled, FIELD), start + 6 * FIELD);
}

#[test]
fn foreground_background_and_unknown_transitions_restart_from_transition_time() {
    let start = Instant::now();
    let mut schedule = Schedule::default();
    assert_eq!(schedule.service_deadline(start, FIELD), start + FIELD);

    let backgrounded = start + Duration::from_millis(5);
    assert_eq!(
        schedule.service_deadline(backgrounded, BACKGROUND),
        backgrounded + BACKGROUND
    );
    let foregrounded = backgrounded + Duration::from_millis(20);
    assert_eq!(
        schedule.service_deadline(foregrounded, FIELD),
        foregrounded + FIELD
    );
}

#[test]
fn mixed_vm_schedules_advance_independently() {
    let start = Instant::now();
    let mut foreground = Schedule::default();
    let mut background = Schedule::default();

    assert_eq!(foreground.service_deadline(start, FIELD), start + FIELD);
    assert_eq!(
        background.service_deadline(start, BACKGROUND),
        start + BACKGROUND
    );
    assert_eq!(
        foreground.service_deadline(start + FIELD, FIELD),
        start + 2 * FIELD
    );
    assert_eq!(
        background.service_deadline(start + FIELD, BACKGROUND),
        start + BACKGROUND
    );
}

#[test]
fn stopping_and_resuming_drops_the_old_service_deadline() {
    let start = Instant::now();
    let resumed = start + Duration::from_secs(1);
    let mut schedule = Schedule::default();
    schedule.service_deadline(start, FIELD);
    assert!(schedule.presentation_due(start, FIELD));

    schedule.stop_service();

    assert_eq!(schedule.service_deadline(resumed, FIELD), resumed + FIELD);
}

#[test]
fn stopping_service_preserves_the_idle_presentation_cadence() {
    let start = Instant::now();
    let mut schedule = Schedule::default();
    assert!(schedule.presentation_due(start, BACKGROUND));

    schedule.stop_service();

    assert!(!schedule.presentation_due(start + FIELD, BACKGROUND));
    assert!(schedule.presentation_due(start + BACKGROUND, BACKGROUND));
}

#[test]
fn foreground_cushion_round_trips_to_two_fields_at_pal_and_ntsc_rates() {
    const PAL_FIELD_RATE: f64 = 50.0;
    const NTSC_FIELD_RATE: f64 = 59.923;
    const EXPECTED_FIELDS: usize = 2;

    for field_rate in [PAL_FIELD_RATE, NTSC_FIELD_RATE] {
        let fields = (foreground_cushion(field_rate).as_secs_f64() * field_rate).ceil() as usize;
        assert_eq!(fields, EXPECTED_FIELDS);
    }
}

#[test]
fn background_cushion_fits_the_audio_ring_and_service_keeps_one_field_in_reserve() {
    const PAL_FIELD_RATE: f64 = 50.0;
    const NTSC_FIELD_RATE: f64 = 59.923;

    for field_rate in [PAL_FIELD_RATE, NTSC_FIELD_RATE] {
        let cushion = cushion_fields(field_rate, crate::BACKGROUND_REPAINT_INTERVAL);
        let capacity = (crate::audio::RING_BUFFER_SECS * field_rate).floor() as usize;
        assert!(cushion + crate::MAX_FIELDS_PER_UPDATE <= capacity);

        let service = service_interval(field_rate, Some(crate::BACKGROUND_REPAINT_INTERVAL));
        let service_fields = (service.as_secs_f64() * field_rate).round() as usize;
        assert_eq!(service_fields + 1, cushion);
    }
}

#[test]
fn presentation_deadline_ignores_early_repaints_and_skips_stale_ticks() {
    let start = Instant::now();
    let mut schedule = Schedule::default();
    assert!(schedule.presentation_due(start, FIELD));
    assert!(!schedule.presentation_due(start + FIELD / 2, FIELD));
    assert!(schedule.presentation_due(start + 4 * FIELD + FIELD / 2, FIELD));
    assert!(schedule.presentation_due(start + 5 * FIELD, FIELD));
    assert!(!schedule.presentation_due(start + 5 * FIELD + FIELD / 2, FIELD));
}

#[test]
fn repaint_request_compensates_for_egui_predicted_frame_time() {
    const DEADLINE_DELAY: Duration = Duration::from_secs(1);
    const PREDICTED_DT: f32 = 0.25;
    const CLOCK_TOLERANCE: Duration = Duration::from_millis(100);
    let ctx = egui::Context::default();
    let _ = ctx.run(egui::RawInput::default(), |_| {});
    let input = egui::RawInput {
        predicted_dt: PREDICTED_DT,
        ..Default::default()
    };
    let output = ctx.run(input, |ctx| {
        request_repaint_at(ctx, Instant::now() + DEADLINE_DELAY);
    });

    assert!(
        output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
            >= DEADLINE_DELAY - CLOCK_TOLERANCE
    );
}
