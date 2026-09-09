//! Run alone with `--ignored --test-threads=1 --nocapture`; allocation counters
//! cover the process, so concurrently running tests invalidate the measurement.
use super::pipeline_tests::{PreviousProducer, signal};
use super::*;
use serde_json::{Value, json};

const ITERATIONS: usize = 2_000;
const BATCH_FRAMES: usize = 1_048;
const DEVICE_RATE: f64 = 48_000.0;
const SOURCE_RATE: f64 = 62_866.0;
const CALLBACK_FRAMES: usize = 256;
const STEREO_CHANNELS: usize = 2;
const CHECKSUM_MULTIPLIER: u64 = 1_099_511_628_211;

fn checksum(frames: impl Iterator<Item = [f32; 2]>, mut value: u64) -> u64 {
    for frame in frames {
        for sample in frame {
            value = value.wrapping_mul(CHECKSUM_MULTIPLIER) ^ u64::from(sample.to_bits());
        }
    }
    value
}

fn measure(mut batch: impl FnMut(u64) -> u64) -> (Value, u64) {
    batch(0); // Warm filters, queue storage, and producer scratch before counting.
    crate::perf::reset();
    let mut result = 0;
    for _ in 0..ITERATIONS {
        result = batch(result);
    }
    (crate::perf::snapshot(), result)
}

fn measure_producers(samples: &[[f32; 2]]) -> (Value, Value) {
    let mut previous = PreviousProducer::new(DEVICE_RATE);
    let (before, before_checksum) = measure(|value| {
        previous.push(samples, SOURCE_RATE, DEFAULT_VOLUME, false);
        checksum(previous.queue.drain(..), value)
    });
    let mut output = AudioOutput::headless(DEVICE_RATE);
    let (after, after_checksum) = measure(|value| {
        output.push_samples(samples.iter().copied(), SOURCE_RATE);
        checksum(lock(&output.ring).drain(..), value)
    });
    assert_eq!(before_checksum, after_checksum);
    assert_eq!(after["allocations"]["count"], 0);
    (
        json!({"metrics": before, "checksum": before_checksum}),
        json!({"metrics": after, "checksum": after_checksum}),
    )
}

fn measure_callback(full: bool) -> Value {
    let ring = Mutex::new(VecDeque::with_capacity(CALLBACK_FRAMES));
    let mut callback = stream::Callback::new(STEREO_CHANNELS, DEVICE_RATE);
    let mut data = [0.0; CALLBACK_FRAMES * STEREO_CHANNELS];
    let (metrics, output_checksum) = measure(|value| {
        if full {
            lock(&ring).extend(std::iter::repeat_n([0.5, -0.25], CALLBACK_FRAMES));
        }
        callback.render(&ring, &mut data);
        checksum(
            data.chunks_exact(STEREO_CHANNELS).map(|s| [s[0], s[1]]),
            value,
        )
    });
    assert_eq!(metrics["allocations"]["count"], 0);
    json!({"metrics": metrics, "checksum": output_checksum})
}

#[test]
#[ignore = "isolated process-wide allocation measurement"]
fn audio_steady_state_allocation_measurement() {
    let samples = signal(BATCH_FRAMES);
    let (before, after) = measure_producers(&samples);
    let empty_callback = measure_callback(false);
    let full_callback = measure_callback(true);
    println!(
        "{}",
        json!({
            "iterations": ITERATIONS, "source_frames_per_batch": BATCH_FRAMES,
            "source_rate_hz": SOURCE_RATE, "device_rate_hz": DEVICE_RATE,
            "previous_producer": before, "reused_producer": after,
            "empty_callback": empty_callback, "full_callback": full_callback,
        })
    );
}
