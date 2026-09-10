//! Host deadlines are independent of incidental UI events and monitor refresh.

use std::time::{Duration, Instant};

use eframe::egui;

/// Two fields protect foreground audio from a late host presentation/callback.
const FOREGROUND_CUSHION_FIELDS: f64 = 2.0;

/// Focus and minimization belong to this VM's viewport, not the manager. egui
/// does not expose occlusion/visibility here. Unknown state stays at foreground
/// cadence; a known minimized or unfocused window uses background cadence.
pub(crate) fn background_delay(ctx: &egui::Context) -> Option<Duration> {
    ctx.input(|input| viewport_background_delay(input.viewport()))
}

fn viewport_background_delay(viewport: &egui::ViewportInfo) -> Option<Duration> {
    (viewport.minimized == Some(true) || viewport.focused == Some(false))
        .then_some(crate::BACKGROUND_REPAINT_INTERVAL)
}

pub(super) fn foreground_cushion(field_rate: f64) -> Duration {
    // Truncate nanoseconds so converting back with ceil never adds a third field.
    let nanos = Duration::from_secs(1).as_nanos() as f64;
    Duration::from_nanos((FOREGROUND_CUSHION_FIELDS * nanos / field_rate) as u64)
}

pub(super) fn cushion_fields(field_rate: f64, duration: Duration) -> usize {
    let desired = (duration.as_secs_f64() * field_rate).ceil() as usize;
    let capacity = (crate::audio::RING_BUFFER_SECS * field_rate).floor() as usize;
    desired.min(capacity.saturating_sub(crate::MAX_FIELDS_PER_UPDATE))
}

pub(super) fn service_interval(field_rate: f64, background: Option<Duration>) -> Duration {
    let field = Duration::from_secs_f64(1.0 / field_rate);
    // Keep one field of safety in the existing background audio cushion. The
    // old egui delay implicitly subtracted predicted_dt; make this explicit.
    background.map_or(field, |delay| {
        let fields = cushion_fields(field_rate, delay).saturating_sub(1).max(1);
        Duration::from_secs_f64(fields as f64 / field_rate)
    })
}

/// egui 0.33 subtracts predicted_dt internally. Compensate so deadlines shorter
/// than one host frame don't become immediate repaint loops. Events can still
/// wake earlier; the native backend's render/swap can make delivery later.
pub(crate) fn request_repaint_at(ctx: &egui::Context, deadline: Instant) {
    let predicted =
        ctx.input(|input| Duration::try_from_secs_f32(input.predicted_dt).unwrap_or_default());
    let remaining = deadline.saturating_duration_since(Instant::now());
    ctx.request_repaint_after(remaining.saturating_add(predicted));
}

#[derive(Default)]
pub(crate) struct Schedule {
    service: Option<(Instant, Duration)>,
    presentation: Option<(Instant, Duration)>,
}

impl Schedule {
    /// Keep an absolute cadence through extra input/manager repaints. A host
    /// stall skips expired deadlines; the bounded field debt handles catch-up.
    pub(super) fn service_deadline(&mut self, now: Instant, interval: Duration) -> Instant {
        advance(&mut self.service, now, interval);
        self.service.expect("advance sets deadline").0
    }

    pub(super) fn presentation_due(&mut self, now: Instant, interval: Duration) -> bool {
        advance(&mut self.presentation, now, interval)
    }

    pub(super) fn presentation_deadline(&self) -> Instant {
        self.presentation.expect("presentation_due sets deadline").0
    }

    pub(super) fn stop_service(&mut self) {
        self.service = None;
    }
}

fn advance(timer: &mut Option<(Instant, Duration)>, now: Instant, interval: Duration) -> bool {
    if let Some((deadline, previous_interval)) = *timer
        && interval == previous_interval
    {
        if now < deadline {
            return false;
        }
        let late = now.saturating_duration_since(deadline).as_nanos();
        let remainder = Duration::from_nanos((late % interval.as_nanos()) as u64);
        *timer = Some((now + interval - remainder, interval));
    } else {
        *timer = Some((now + interval, interval));
    }
    true
}

#[cfg(test)]
#[path = "scheduling_test.rs"]
mod tests;
