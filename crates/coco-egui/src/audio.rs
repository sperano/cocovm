//! Audio output: drains stereo speaker samples from `coco_core::Machine` each
//! frame, resamples them from the core's oversampled grid rate (~62.9 kHz
//! NTSC — `coco_core::audio::OVERSAMPLE` × the horizontal rate) to whatever
//! rate the host's default output device wants, and feeds a `cpal` stream.
//!
//! The device runs on its own high-priority thread and pulls frames out of a
//! `Mutex<VecDeque<[f32; 2]>>` that `push_samples` (called once per `update()`
//! on the UI thread) fills. There is no synchronisation beyond that mutex —
//! audio and video are independently paced, exactly like a real CoCo's TV and
//! speaker.
//!
//! Because the grid rate exceeds typical device rates (44.1/48 kHz), the
//! producer side low-passes before decimating ([`LowPass`]): plain linear
//! decimation would fold the >Nyquist half of the spectrum straight back
//! into the audible band — re-aliasing exactly what the oversampled grid
//! exists to prevent.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use eframe::egui;

mod stream;

/// Default output volume (of `menu_ui`'s 0.0..=1.0 slider) — audible but not
/// jarring the first time the app starts.
const DEFAULT_VOLUME: f32 = 0.5;
/// Fixed headroom applied ahead of the user volume control. The CoCo speaker
/// path (single-bit sound OR'd with the 6-bit DAC through the analog mux, see
/// `coco-core`'s `audio` module) can swing across most of the 0..1 sample
/// range; without this the default volume plus a hot signal clips.
const MASTER_GAIN: f32 = 0.6;
/// One-pole DC-blocker feedback coefficient (`y[n] = x[n] - x[n-1] + R*y[n-1]`).
/// Close to 1.0 keeps the cutoff well below audible range (roughly
/// `(1-R) * sample_rate / (2*pi)` Hz) while still pulling the DAC's resting
/// offset (the CoCo's DAC parks at a nonzero level, not 0V) down to ~0 within
/// a few thousand samples.
const DC_BLOCKER_POLE: f32 = 0.995;
/// Anti-alias low-pass cutoff, as a fraction of the DEVICE rate — just under
/// Nyquist, per the plan ("a 2-pole IIR at ~0.45·device-rate is enough").
const LOWPASS_CUTOFF_OF_DEVICE_RATE: f64 = 0.45;
/// Butterworth Q for the 2-pole low-pass (maximally flat passband).
const LOWPASS_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;
/// Ring buffer bound, in seconds of buffered audio, before `push_samples`
/// starts dropping the oldest queued frames. Bounds worst-case output latency
/// and keeps a paused/backgrounded UI from growing the buffer unboundedly.
const RING_BUFFER_SECS: f64 = 0.25;
/// Initial producer scratch space, allowing several fields per UI update.
/// Unusually large batches can grow this storage; subsequent batches reuse it.
const INITIAL_PROCESSED_CAPACITY: usize = 16_384;
/// Rounding headroom for the resampler's carried fractional position.
const RESAMPLER_CAPACITY_HEADROOM: usize = 1;
/// How long a continuous underrun takes to fade the held-over output frame to
/// (effectively) silence, so a starved ring buffer decays smoothly instead of
/// clicking or parking at a stuck DC level.
const UNDERRUN_FADE_SECS: f64 = 0.05;
/// "Effectively silent" floor the underrun fade decays to by `UNDERRUN_FADE_SECS`,
/// expressed as a fraction of the held frame (-60 dB).
const UNDERRUN_FADE_FLOOR: f32 = 0.001;

/// Per-sample state for the one-pole DC-blocking highpass filter, factored out
/// of `AudioOutput` so the math can be unit-tested without a live stream.
#[derive(Default, Clone, Copy)]
struct DCBlocker {
    prev_in: f32,
    prev_out: f32,
}

impl DCBlocker {
    fn process(&mut self, x: f32) -> f32 {
        let y = x - self.prev_in + DC_BLOCKER_POLE * self.prev_out;
        self.prev_in = x;
        self.prev_out = y;
        y
    }
}

/// 2-pole (biquad) low-pass, RBJ-cookbook coefficients, run at the SOURCE
/// rate before decimation. One instance per channel.
#[derive(Default, Clone, Copy)]
struct LowPass {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl LowPass {
    /// Coefficients for cutoff `fc` Hz at sample rate `fs` Hz (fc < fs/2).
    fn design(fc: f64, fs: f64) -> Self {
        let w0 = std::f64::consts::TAU * fc / fs;
        let alpha = w0.sin() / (2.0 * LOWPASS_Q);
        let cos_w0 = w0.cos();
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 - cos_w0) / 2.0 / a0) as f32,
            b1: ((1.0 - cos_w0) / a0) as f32,
            b2: ((1.0 - cos_w0) / 2.0 / a0) as f32,
            a1: (-2.0 * cos_w0 / a0) as f32,
            a2: ((1.0 - alpha) / a0) as f32,
            ..Self::default()
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Streaming linear resampler state over stereo frames: a fractional position
/// into the input stream plus the last frame of the previous batch, carried
/// across calls so interpolation stays phase-accurate at a non-integer rate
/// ratio (the same "carry the remainder" pattern as `main.rs`'s `field_debt`).
#[derive(Default, Clone, Copy)]
struct Resampler {
    /// Fractional position, in input-frame units, of the next output frame
    /// relative to `prev`..`input[0]`.
    pos: f64,
    /// Last input frame consumed by the previous call (index -1 relative to
    /// the next call's `input`), so the first output frame of a new batch can
    /// still interpolate across the batch boundary.
    prev: [f32; 2],
}

impl Resampler {
    /// Resample `input` (at `step` input-frames-per-output-frame) into `out`,
    /// appending. `step = source_rate / device_rate`.
    fn process(&mut self, input: &[[f32; 2]], step: f64, out: &mut Vec<[f32; 2]>) {
        let n = input.len();
        if n == 0 {
            return;
        }
        loop {
            let i = self.pos.floor() as usize;
            if i >= n {
                break;
            }
            let frac = (self.pos - i as f64) as f32;
            let a = if i == 0 { self.prev } else { input[i - 1] };
            let b = input[i];
            out.push([a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]);
            self.pos += step;
        }
        self.prev = input[n - 1];
        self.pos -= n as f64;
    }
}

/// Owns the cpal output stream (if one could be opened) plus all producer-side
/// audio state: the DC blockers, anti-alias filters, resampler, and
/// volume/mute controls.
pub struct AudioOutput {
    /// The live cpal stream, held only so it keeps playing for as long as
    /// this `AudioOutput` does (dropping it stops the device). `None` when no
    /// output device/config/stream could be opened — see [`Self::is_enabled`]
    /// for the gate every producer-side method degrades through.
    _stream: Option<cpal::Stream>,
    /// Stereo frames at the device's rate, shared with the stream's callback
    /// thread: `push_samples` (producer) pushes resampled frames; the cpal
    /// callback (consumer) pops one per output frame and maps L/R onto the
    /// device's channels.
    ring: Arc<Mutex<VecDeque<[f32; 2]>>>,
    /// Bound on `ring`'s length, in frames (`RING_BUFFER_SECS` of device rate).
    ring_cap: usize,
    /// The open device's output rate, or 0.0 when disabled.
    device_rate: f64,
    muted: bool,
    volume: f32,
    dc: [DCBlocker; 2],
    /// Anti-alias low-pass per channel, designed lazily for the source rate
    /// seen on the first `push_samples` call (`None` until then, or when
    /// upsampling makes it unnecessary).
    lowpass: Option<[LowPass; 2]>,
    /// The source rate `lowpass` was designed for — redesign on change.
    lowpass_rate: f64,
    resampler: Resampler,
    processed: Vec<[f32; 2]>,
    resampled: Vec<[f32; 2]>,
}

impl AudioOutput {
    pub fn new() -> Self {
        let ring = Arc::new(Mutex::new(VecDeque::new()));
        match Self::try_build_stream(Arc::clone(&ring)) {
            Ok((stream, device_rate, ring_cap)) => {
                Self::from_parts(Some(stream), ring, device_rate, ring_cap)
            }
            Err(e) => {
                tracing::warn!("audio output unavailable: {e}");
                Self::from_parts(None, ring, 0.0, 0)
            }
        }
    }

    /// Runs the full producer path (filters, resampler, ring) at `device_rate`
    /// without a real device, so tests can exercise it. Nothing drains the ring.
    #[cfg(test)]
    pub(crate) fn headless(device_rate: f64) -> Self {
        let ring_cap = ring_capacity(device_rate);
        Self::from_parts(
            None,
            Arc::new(Mutex::new(VecDeque::with_capacity(ring_cap))),
            device_rate,
            ring_cap,
        )
    }

    fn from_parts(
        stream: Option<cpal::Stream>,
        ring: Arc<Mutex<VecDeque<[f32; 2]>>>,
        device_rate: f64,
        ring_cap: usize,
    ) -> Self {
        Self {
            _stream: stream,
            ring,
            ring_cap,
            device_rate,
            muted: false,
            volume: DEFAULT_VOLUME,
            dc: [DCBlocker::default(); 2],
            lowpass: None,
            lowpass_rate: 0.0,
            resampler: Resampler::default(),
            processed: Vec::with_capacity(if device_rate > 0.0 {
                INITIAL_PROCESSED_CAPACITY
            } else {
                0
            }),
            resampled: Vec::with_capacity(if device_rate > 0.0 {
                ring_cap + RESAMPLER_CAPACITY_HEADROOM
            } else {
                0
            }),
        }
    }

    /// Whether there is anywhere for samples to go: a live cpal stream, or a
    /// test-only headless pipeline (`device_rate` set, no stream).
    fn is_enabled(&self) -> bool {
        self.device_rate > 0.0
    }

    /// Clears queued samples and filter state after a discontinuity (state
    /// restore, power cycle), so audio/filter history from the old machine
    /// doesn't bleed into the new stream. Volume/mute persist.
    pub fn reset(&mut self) {
        lock(&self.ring).clear();
        self.dc = [DCBlocker::default(); 2];
        self.lowpass = None;
        self.lowpass_rate = 0.0;
        self.resampler = Resampler::default();
        self.processed.clear();
        self.resampled.clear();
    }

    /// Frames waiting in the ring buffer for the device to consume.
    #[cfg(test)]
    pub(crate) fn queued_frames(&self) -> usize {
        lock(&self.ring).len()
    }

    /// Resamples and enqueues one batch of stereo speaker frames. A no-op
    /// when no output device was found.
    pub fn push_samples(&mut self, samples: impl Iterator<Item = [f32; 2]>, source_rate: f64) {
        let _perf = crate::perf::span(crate::perf::Stage::AudioPush);
        if !self.is_enabled() {
            return;
        }
        self.process_samples(samples, source_rate);
        self.resampled.clear();
        self.resampler.process(
            &self.processed,
            source_rate / self.device_rate,
            &mut self.resampled,
        );
        if self.muted {
            self.resampled.fill([0.0; 2]);
        }

        let mut buf = lock(&self.ring);
        let dropped = enqueue(&mut buf, &self.resampled, self.ring_cap);
        crate::perf::audio_queue(buf.len(), dropped);
    }

    fn process_samples(&mut self, samples: impl Iterator<Item = [f32; 2]>, source_rate: f64) {
        // Redesign anti-alias filters when the source rate changes; only needed when decimating.
        if source_rate != self.lowpass_rate {
            self.lowpass_rate = source_rate;
            self.lowpass = (source_rate > self.device_rate).then(|| {
                let fc = LOWPASS_CUTOFF_OF_DEVICE_RATE * self.device_rate;
                [LowPass::design(fc, source_rate); 2]
            });
        }

        // Applied regardless of mute so filter state doesn't pop when unmuting mid-stream.
        let gain = MASTER_GAIN * self.volume;
        self.processed.clear();
        self.processed.extend(samples.map(|[l, r]| {
            let mut l = self.dc[0].process(l) * gain;
            let mut r = self.dc[1].process(r) * gain;
            if let Some(lp) = self.lowpass.as_mut() {
                l = lp[0].process(l);
                r = lp[1].process(r);
            }
            [l, r]
        }));
    }

    /// Mute checkbox and volume slider, or a disabled label when no output
    /// device is available.
    pub fn menu_ui(&mut self, ui: &mut egui::Ui) {
        if !self.is_enabled() {
            ui.add_enabled(false, egui::Label::new("No audio device"));
            return;
        }
        ui.checkbox(&mut self.muted, "Mute");
        ui.add(egui::Slider::new(&mut self.volume, 0.0..=1.0).text("Volume"));
    }
}

/// Discard the oldest frames before appending, so the deque never grows beyond
/// its preallocated logical limit, even when a single batch exceeds that limit.
fn enqueue(buf: &mut VecDeque<[f32; 2]>, samples: &[[f32; 2]], capacity: usize) -> usize {
    let dropped = buf
        .len()
        .saturating_add(samples.len())
        .saturating_sub(capacity);
    let old_dropped = dropped.min(buf.len());
    buf.drain(..old_dropped);
    let new_dropped = samples.len().saturating_sub(capacity);
    buf.extend(samples[new_dropped..].iter().copied());
    dropped
}

/// [`AudioOutput::ring_cap`] for a device at `device_rate` Hz:
/// `RING_BUFFER_SECS` worth of frames, never zero.
fn ring_capacity(device_rate: f64) -> usize {
    ((device_rate * RING_BUFFER_SECS) as usize).max(1)
}

/// Locks `ring`, recovering from poisoning instead of propagating a panic
/// across threads — losing a frame beats tearing down the app.
fn lock(ring: &Mutex<VecDeque<[f32; 2]>>) -> MutexGuard<'_, VecDeque<[f32; 2]>> {
    ring.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
#[path = "audio_test.rs"]
mod tests;

#[cfg(test)]
#[path = "audio/pipeline_test.rs"]
mod pipeline_tests;

#[cfg(all(test, feature = "perf"))]
#[path = "audio_perf_test.rs"]
mod perf_tests;
