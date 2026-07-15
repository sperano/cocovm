//! Audio output: drains speaker samples from `coco_core::Machine` each frame,
//! resamples them from the CoCo's horizontal rate (~15.7 kHz NTSC) to whatever
//! rate the host's default output device wants, and feeds a `cpal` stream.
//!
//! The device runs on its own high-priority thread and pulls samples out of a
//! `Mutex<VecDeque<f32>>` that `push_samples` (called once per `update()` on the
//! UI thread) fills. There is no synchronisation beyond that mutex — audio and
//! video are independently paced, exactly like a real CoCo's TV and speaker.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use eframe::egui;

/// Default output volume (of `menu_ui`'s 0.0..=1.0 slider) — audible but not
/// jarring the first time the app starts.
const DEFAULT_VOLUME: f32 = 0.5;
/// Fixed headroom applied ahead of the user volume control. The CoCo speaker
/// path (single-bit sound OR'd with the 6-bit DAC through the analog mux, see
/// `coco-core`'s `run_field` audio comment) can swing across most of the 0..1
/// sample range; without this the default volume plus a hot signal clips.
const MASTER_GAIN: f32 = 0.6;
/// One-pole DC-blocker feedback coefficient (`y[n] = x[n] - x[n-1] + R*y[n-1]`).
/// Close to 1.0 keeps the cutoff well below audible range (roughly
/// `(1-R) * sample_rate / (2*pi)` Hz) while still pulling the DAC's resting
/// offset (the CoCo's DAC parks at a nonzero level, not 0V) down to ~0 within
/// a few thousand samples.
const DC_BLOCKER_POLE: f32 = 0.995;
/// Ring buffer bound, in seconds of buffered audio, before `push_samples`
/// starts dropping the oldest queued samples. Bounds worst-case output latency
/// and keeps a paused/backgrounded UI from growing the buffer unboundedly.
const RING_BUFFER_SECS: f64 = 0.25;
/// How long a continuous underrun takes to fade the held-over output sample to
/// (effectively) silence, so a starved ring buffer decays smoothly instead of
/// clicking or parking at a stuck DC level.
const UNDERRUN_FADE_SECS: f64 = 0.05;
/// "Effectively silent" floor the underrun fade decays to by `UNDERRUN_FADE_SECS`,
/// expressed as a fraction of the held sample (-60 dB).
const UNDERRUN_FADE_FLOOR: f32 = 0.001;

/// Per-sample state for the one-pole DC-blocking highpass filter, factored out
/// of `AudioOutput` so the math can be unit-tested without a live stream.
#[derive(Default, Clone, Copy)]
struct DcBlocker {
    prev_in: f32,
    prev_out: f32,
}

impl DcBlocker {
    fn process(&mut self, x: f32) -> f32 {
        let y = x - self.prev_in + DC_BLOCKER_POLE * self.prev_out;
        self.prev_in = x;
        self.prev_out = y;
        y
    }
}

/// Streaming linear resampler state: a fractional position into the input
/// stream plus the last sample of the previous batch, carried across calls so
/// interpolation stays phase-accurate at a non-integer rate ratio (the same
/// "carry the remainder" pattern as `main.rs`'s `field_debt`).
#[derive(Default, Clone, Copy)]
struct Resampler {
    /// Fractional position, in input-sample units, of the next output sample
    /// relative to `prev`..`input[0]`.
    pos: f64,
    /// Last input sample consumed by the previous call (index -1 relative to
    /// the next call's `input`), so the first output sample of a new batch can
    /// still interpolate across the batch boundary.
    prev: f32,
}

impl Resampler {
    /// Resample `input` (at `step` input-samples-per-output-sample) into
    /// `out`, appending. `step = source_rate / device_rate`.
    fn process(&mut self, input: &[f32], step: f64, out: &mut Vec<f32>) {
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
            out.push(a + (b - a) * frac);
            self.pos += step;
        }
        self.prev = input[n - 1];
        self.pos -= n as f64;
    }
}

/// Owns the cpal output stream (if one could be opened) plus all producer-side
/// audio state: the DC blocker, resampler, and volume/mute controls.
pub struct AudioOutput {
    /// `None` when no output device/config/stream could be opened — every
    /// other method then degrades to a no-op instead of touching cpal.
    stream: Option<cpal::Stream>,
    /// Mono samples at the device's rate, shared with the stream's callback
    /// thread: `push_samples` (producer) pushes resampled frames; the cpal
    /// callback (consumer) pops one per output frame and fans it out to every
    /// channel.
    ring: Arc<Mutex<VecDeque<f32>>>,
    /// Bound on `ring`'s length, in frames (`RING_BUFFER_SECS` of device rate).
    ring_cap: usize,
    /// The open device's output rate, or 0.0 when disabled.
    device_rate: f64,
    muted: bool,
    volume: f32,
    dc: DcBlocker,
    resampler: Resampler,
}

impl AudioOutput {
    pub fn new() -> Self {
        let ring = Arc::new(Mutex::new(VecDeque::new()));
        match Self::try_build_stream(Arc::clone(&ring)) {
            Ok((stream, device_rate, ring_cap)) => Self {
                stream: Some(stream),
                ring,
                ring_cap,
                device_rate,
                muted: false,
                volume: DEFAULT_VOLUME,
                dc: DcBlocker::default(),
                resampler: Resampler::default(),
            },
            Err(e) => {
                tracing::warn!("audio output unavailable: {e}");
                Self {
                    stream: None,
                    ring,
                    ring_cap: 0,
                    device_rate: 0.0,
                    muted: false,
                    volume: DEFAULT_VOLUME,
                    dc: DcBlocker::default(),
                    resampler: Resampler::default(),
                }
            }
        }
    }

    /// Open the default output device at its default config and start playing
    /// a stream fed from `ring`. Every fallible step (no device, unsupported
    /// config, stream build/play failure) is folded into a single `Err` so
    /// `new()` has one place to log and degrade.
    fn try_build_stream(
        ring: Arc<Mutex<VecDeque<f32>>>,
    ) -> Result<(cpal::Stream, f64, usize), String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no output device".to_string())?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let channels = supported.channels() as usize;
        let device_rate = supported.sample_rate() as f64;
        let ring_cap = ((device_rate * RING_BUFFER_SECS) as usize).max(1);
        let stream_config: cpal::StreamConfig = supported.into();

        // Decay applied to the held-over sample on every underrun frame, derived
        // from the device's own rate so the fade always takes UNDERRUN_FADE_SECS
        // regardless of what that rate is (44.1 kHz, 48 kHz, ...).
        let fade_frames = (device_rate * UNDERRUN_FADE_SECS).max(1.0);
        let decay = UNDERRUN_FADE_FLOOR.powf(1.0 / fade_frames as f32);
        let mut held = 0.0f32;

        let stream = device
            .build_output_stream(
                stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let mut buf = lock(&ring);
                    for frame in data.chunks_mut(channels) {
                        let sample = match buf.pop_front() {
                            Some(s) => {
                                held = s;
                                s
                            }
                            None => {
                                held *= decay;
                                held
                            }
                        };
                        for out in frame {
                            *out = sample;
                        }
                    }
                },
                |err| tracing::error!("audio stream error: {err}"),
                None,
            )
            .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;

        Ok((stream, device_rate, ring_cap))
    }

    /// Resample and enqueue one batch of speaker samples (called once per
    /// `update()` with `machine.take_audio()` / `machine.audio_sample_rate()`).
    /// A no-op when no output device was found.
    pub fn push_samples(&mut self, samples: impl Iterator<Item = f32>, source_rate: f64) {
        if self.stream.is_none() {
            return;
        }

        // DC-block and apply gain/volume on every sample regardless of mute,
        // so the filter state and volume don't pop when unmuting mid-stream.
        let processed: Vec<f32> = samples
            .map(|s| self.dc.process(s) * MASTER_GAIN * self.volume)
            .collect();
        let step = source_rate / self.device_rate;
        let mut resampled = Vec::with_capacity(
            (processed.len() as f64 * self.device_rate / source_rate).ceil() as usize,
        );
        self.resampler.process(&processed, step, &mut resampled);

        if self.muted {
            resampled.iter_mut().for_each(|s| *s = 0.0);
        }

        let mut buf = lock(&self.ring);
        buf.extend(resampled);
        while buf.len() > self.ring_cap {
            buf.pop_front();
        }
    }

    /// "Sound" menu contents: a mute checkbox and volume slider, or a disabled
    /// label when no output device is available (mirrors
    /// `JoystickInputs::menu_ui`'s "Gamepad: unavailable" line).
    pub fn menu_ui(&mut self, ui: &mut egui::Ui) {
        if self.stream.is_none() {
            ui.add_enabled(false, egui::Label::new("No audio device"));
            return;
        }
        ui.checkbox(&mut self.muted, "Mute");
        ui.add(egui::Slider::new(&mut self.volume, 0.0..=1.0).text("Volume"));
    }
}

/// Lock `ring`, recovering from mutex poisoning instead of propagating a panic
/// from one thread (UI or audio callback) into the other — a lost frame or two
/// of audio is far preferable to tearing down the whole app.
fn lock(ring: &Mutex<VecDeque<f32>>) -> MutexGuard<'_, VecDeque<f32>> {
    ring.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_upsamples_2x_with_linear_interpolation() {
        // step = 0.5 means the device rate is double the source rate: every
        // input sample should produce two output samples, the second being
        // the midpoint to the next input sample.
        let mut r = Resampler::default();
        let mut out = Vec::new();
        r.process(&[0.0, 10.0], 0.5, &mut out);
        // prev starts at 0.0 (no prior batch), so:
        // pos=0.0 -> a=prev=0.0, b=input[0]=0.0, frac=0.0 -> 0.0
        // pos=0.5 -> a=prev=0.0, b=input[0]=0.0, frac=0.5 -> 0.0
        // pos=1.0 -> a=input[0]=0.0, b=input[1]=10.0, frac=0.0 -> 0.0
        // pos=1.5 -> a=input[0]=0.0, b=input[1]=10.0, frac=0.5 -> 5.0
        assert_eq!(out, vec![0.0, 0.0, 0.0, 5.0]);
    }

    #[test]
    fn resampler_carries_fractional_position_and_prev_sample_across_calls() {
        // Same math as above, but split across two process() calls to prove
        // the carried `pos`/`prev` state reproduces one continuous stream.
        let mut r = Resampler::default();
        let mut out = Vec::new();
        r.process(&[0.0], 0.5, &mut out);
        r.process(&[10.0], 0.5, &mut out);
        assert_eq!(out, vec![0.0, 0.0, 0.0, 5.0]);
    }

    #[test]
    fn resampler_downsamples_when_step_exceeds_one() {
        // step = 2.0: device rate is half the source rate, so every other
        // input sample is emitted. The very first output still interpolates
        // against `prev`'s initial 0.0 (no prior batch), which is why it
        // isn't simply input[0].
        let mut r = Resampler::default();
        let mut out = Vec::new();
        r.process(&[1.0, 2.0, 3.0, 4.0], 2.0, &mut out);
        assert_eq!(out, vec![0.0, 2.0]);
    }

    #[test]
    fn dc_blocker_converges_toward_zero_on_constant_input() {
        let mut dc = DcBlocker::default();
        let mut last = 1.0;
        for _ in 0..2000 {
            last = dc.process(1.0);
        }
        assert!(last.abs() < 1e-3, "expected near-zero, got {last}");
    }

    #[test]
    fn dc_blocker_passes_already_centered_signal_without_blowing_up() {
        let mut dc = DcBlocker::default();
        let mut max_abs = 0.0f32;
        for i in 0..1000 {
            let x = if i % 2 == 0 { 1.0 } else { -1.0 };
            max_abs = f32::max(max_abs, dc.process(x).abs());
        }
        // A signal already centered at 0 should stay bounded near its own
        // amplitude, not grow — a highpass shouldn't amplify AC content.
        assert!(max_abs < 2.5, "expected bounded output, got {max_abs}");
    }

    #[test]
    fn underrun_decay_reaches_floor_within_fade_window() {
        let device_rate = 48_000.0;
        let fade_frames = (device_rate * UNDERRUN_FADE_SECS) as u32;
        let decay = UNDERRUN_FADE_FLOOR.powf(1.0 / fade_frames as f32);
        let mut held = 1.0f32;
        for _ in 0..fade_frames {
            held *= decay;
        }
        assert!(held <= UNDERRUN_FADE_FLOOR * 1.01);
    }
}
