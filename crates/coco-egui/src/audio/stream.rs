use super::*;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

impl AudioOutput {
    /// Opens the default output device and starts a stream fed from `ring`.
    /// Every fallible step is folded into one `Err` so `new()` has a single
    /// place to log and degrade.
    pub(super) fn try_build_stream(
        ring: Arc<Mutex<VecDeque<[f32; 2]>>>,
    ) -> Result<(cpal::Stream, f64, usize), String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| "no output device".to_string())?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let channels = supported.channels() as usize;
        let device_rate = supported.sample_rate() as f64;
        let ring_cap = ring_capacity(device_rate);
        // Allocate before starting the callback; enqueue never exceeds this limit.
        lock(&ring).reserve(ring_cap);
        let mut callback = Callback::new(channels, device_rate);
        let stream = device
            .build_output_stream(
                supported.into(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    callback.render(&ring, data);
                },
                |err| tracing::error!("audio stream error: {err}"),
                None,
            )
            .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        #[cfg(feature = "perf")]
        crate::perf::audio_config(device_rate as u32, channels as u16);
        Ok((stream, device_rate, ring_cap))
    }
}

pub(super) struct Callback {
    channels: usize,
    decay: f32,
    held: [f32; 2],
}

impl Callback {
    pub(super) fn new(channels: usize, device_rate: f64) -> Self {
        let fade_frames = (device_rate * UNDERRUN_FADE_SECS).max(1.0);
        Self {
            channels,
            decay: UNDERRUN_FADE_FLOOR.powf(1.0 / fade_frames as f32),
            held: [0.0; 2],
        }
    }

    pub(super) fn render(&mut self, ring: &Mutex<VecDeque<[f32; 2]>>, data: &mut [f32]) {
        #[cfg(feature = "perf")]
        let wait = crate::perf::span(crate::perf::Stage::AudioCallbackLockWait);
        let mut buf = lock(ring);
        #[cfg(feature = "perf")]
        drop(wait);
        #[cfg(feature = "perf")]
        let hold = crate::perf::span(crate::perf::Stage::AudioCallbackLockHold);
        #[cfg(feature = "perf")]
        let queue_before = buf.len();
        #[cfg(feature = "perf")]
        let mut missing = 0;
        for frame in data.chunks_mut(self.channels) {
            if let Some(sample) = buf.pop_front() {
                self.held = sample;
            } else {
                #[cfg(feature = "perf")]
                {
                    missing += 1;
                }
                self.held[0] *= self.decay;
                self.held[1] *= self.decay;
            }
            self.write_frame(frame);
        }
        #[cfg(feature = "perf")]
        let queue_after = buf.len();
        drop(buf);
        #[cfg(feature = "perf")]
        {
            drop(hold);
            crate::perf::audio_callback(missing, queue_before, queue_after);
        }
    }

    fn write_frame(&self, frame: &mut [f32]) {
        let [l, r] = self.held;
        // L→even channels, R→odd; a mono device gets the mix.
        if frame.len() == 1 {
            frame[0] = (l + r) * 0.5;
        } else {
            for (i, out) in frame.iter_mut().enumerate() {
                *out = if i % 2 == 0 { l } else { r };
            }
        }
    }
}

#[cfg(test)]
#[path = "stream_test.rs"]
mod tests;
