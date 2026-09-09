use super::*;

const DEVICE_RATES: [f64; 3] = [44_100.0, 48_000.0, 96_000.0];
const SOURCE_RATES: [f64; 4] = [62_866.0, 62_500.0, 32_000.0, 96_000.0];
const BATCH_LENGTHS: [usize; 8] = [0, 1, 2, 1048, 0, 7, 1250, 3];
const TEST_DEVICE_RATE: f64 = 48_000.0;
const TEST_SOURCE_RATE: f64 = 62_866.0;
const TEST_QUEUE_CAPACITY: usize = 4;

/// Allocation-heavy producer from before scratch reuse, retained as an output
/// oracle. Its resampling and append-then-truncate behavior are independent of
/// the production helpers.
pub(super) struct PreviousProducer {
    dc: [DCBlocker; 2],
    lowpass: Option<[LowPass; 2]>,
    source_rate: f64,
    device_rate: f64,
    pos: f64,
    prev: [f32; 2],
    pub(super) queue: VecDeque<[f32; 2]>,
}

impl PreviousProducer {
    pub(super) fn new(device_rate: f64) -> Self {
        Self {
            dc: [DCBlocker::default(); 2],
            lowpass: None,
            source_rate: 0.0,
            device_rate,
            pos: 0.0,
            prev: [0.0; 2],
            queue: VecDeque::new(),
        }
    }

    pub(super) fn push(&mut self, samples: &[[f32; 2]], rate: f64, volume: f32, muted: bool) {
        if rate != self.source_rate {
            self.source_rate = rate;
            self.lowpass = (rate > self.device_rate).then(|| {
                [LowPass::design(LOWPASS_CUTOFF_OF_DEVICE_RATE * self.device_rate, rate); 2]
            });
        }
        let gain = MASTER_GAIN * volume;
        let processed: Vec<_> = samples
            .iter()
            .map(|&[l, r]| {
                let mut l = self.dc[0].process(l) * gain;
                let mut r = self.dc[1].process(r) * gain;
                if let Some(lp) = self.lowpass.as_mut() {
                    l = lp[0].process(l);
                    r = lp[1].process(r);
                }
                [l, r]
            })
            .collect();
        let mut output = self.resample(&processed, rate);
        if muted {
            output.fill([0.0; 2]);
        }
        self.queue.extend(output);
        while self.queue.len() > ring_capacity(self.device_rate) {
            self.queue.pop_front();
        }
    }

    fn resample(&mut self, input: &[[f32; 2]], source_rate: f64) -> Vec<[f32; 2]> {
        let step = source_rate / self.device_rate;
        let mut output = Vec::with_capacity(
            (input.len() as f64 * self.device_rate / source_rate).ceil() as usize + 1,
        );
        if input.is_empty() {
            return output;
        }
        loop {
            let i = self.pos.floor() as usize;
            if i >= input.len() {
                break;
            }
            let frac = (self.pos - i as f64) as f32;
            let a = if i == 0 { self.prev } else { input[i - 1] };
            let b = input[i];
            output.push([a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]);
            self.pos += step;
        }
        self.prev = input[input.len() - 1];
        self.pos -= input.len() as f64;
        output
    }
}

pub(super) fn signal(length: usize) -> Vec<[f32; 2]> {
    const PERIOD: usize = 37;
    const RIGHT_PERIOD: usize = 13;
    (0..length)
        .map(|i| {
            [
                ((i % PERIOD) as f32 / PERIOD as f32) - 0.5,
                ((i % RIGHT_PERIOD) as f32 / RIGHT_PERIOD as f32) - 0.25,
            ]
        })
        .collect()
}

#[test]
fn producer_matches_previous_algorithm_across_batches_and_rate_changes() {
    for device_rate in DEVICE_RATES {
        let mut actual = AudioOutput::headless(device_rate);
        let mut previous = PreviousProducer::new(device_rate);
        for source_rate in SOURCE_RATES {
            for (batch, length) in BATCH_LENGTHS.into_iter().enumerate() {
                let samples = signal(length);
                actual.muted = batch % 3 == 1;
                actual.volume = if batch % 2 == 0 { DEFAULT_VOLUME } else { 0.8 };
                actual.push_samples(samples.iter().copied(), source_rate);
                previous.push(&samples, source_rate, actual.volume, actual.muted);
                assert_eq!(*lock(&actual.ring), previous.queue);
                assert_eq!(actual.resampler.pos, previous.pos);
                assert_eq!(actual.resampler.prev, previous.prev);
            }
        }
        let samples = signal(ring_capacity(device_rate) * 3);
        actual.push_samples(samples.iter().copied(), TEST_SOURCE_RATE);
        previous.push(&samples, TEST_SOURCE_RATE, actual.volume, actual.muted);
        assert_eq!(*lock(&actual.ring), previous.queue);
    }
}

#[test]
fn queue_overflow_discards_oldest_without_growing_storage() {
    const OLD_LENGTHS: std::ops::RangeInclusive<usize> = 0..=TEST_QUEUE_CAPACITY;
    const NEW_LENGTHS: std::ops::RangeInclusive<usize> = 0..=10;
    for old_len in OLD_LENGTHS {
        for new_len in NEW_LENGTHS {
            let mut queue = VecDeque::with_capacity(TEST_QUEUE_CAPACITY);
            queue.extend(signal(old_len));
            let capacity = queue.capacity();
            let mut expected = queue.clone();
            let samples = signal(new_len);
            expected.extend(samples.iter().copied());
            let dropped = expected.len().saturating_sub(TEST_QUEUE_CAPACITY);
            expected.drain(..dropped);
            assert_eq!(enqueue(&mut queue, &samples, TEST_QUEUE_CAPACITY), dropped);
            assert_eq!(queue, expected);
            assert_eq!(queue.capacity(), capacity);
        }
    }
}

#[test]
fn scratch_storage_is_reused_after_warmup_and_reset() {
    let mut output = AudioOutput::headless(TEST_DEVICE_RATE);
    let samples = signal(INITIAL_PROCESSED_CAPACITY * 2);
    output.push_samples(samples.iter().copied(), TEST_SOURCE_RATE);
    let storage = (output.processed.as_ptr(), output.resampled.as_ptr());
    let capacities = (output.processed.capacity(), output.resampled.capacity());
    let queue_capacity = lock(&output.ring).capacity();
    for length in [samples.len(), 1, 0, samples.len()] {
        output.push_samples(samples[..length].iter().copied(), TEST_SOURCE_RATE);
        assert_eq!(
            (output.processed.as_ptr(), output.resampled.as_ptr()),
            storage
        );
        assert_eq!(
            (output.processed.capacity(), output.resampled.capacity()),
            capacities
        );
        assert_eq!(lock(&output.ring).capacity(), queue_capacity);
    }
    output.reset();
    assert!(output.processed.is_empty());
    assert!(output.resampled.is_empty());
    assert_eq!(
        (output.processed.as_ptr(), output.resampled.as_ptr()),
        storage
    );
}

#[test]
fn mute_keeps_filter_and_resampler_history_and_reset_preserves_controls() {
    let samples = signal(BATCH_LENGTHS[3]);
    let mut muted = AudioOutput::headless(TEST_DEVICE_RATE);
    let mut audible = AudioOutput::headless(TEST_DEVICE_RATE);
    muted.muted = true;
    muted.push_samples(samples.iter().copied(), TEST_SOURCE_RATE);
    audible.push_samples(samples.iter().copied(), TEST_SOURCE_RATE);
    assert!(lock(&muted.ring).iter().all(|&s| s == [0.0; 2]));
    lock(&muted.ring).clear();
    lock(&audible.ring).clear();
    muted.muted = false;
    muted.push_samples(samples.iter().copied(), TEST_SOURCE_RATE);
    audible.push_samples(samples.iter().copied(), TEST_SOURCE_RATE);
    assert_eq!(*lock(&muted.ring), *lock(&audible.ring));

    muted.muted = true;
    muted.volume = 0.8;
    muted.reset();
    assert!(muted.muted);
    assert_eq!(muted.volume, 0.8);
    muted.muted = false;
    let mut fresh = AudioOutput::headless(TEST_DEVICE_RATE);
    fresh.volume = muted.volume;
    muted.push_samples(samples.iter().copied(), TEST_SOURCE_RATE);
    fresh.push_samples(samples.iter().copied(), TEST_SOURCE_RATE);
    assert_eq!(*lock(&muted.ring), *lock(&fresh.ring));
}

#[test]
fn disabled_output_does_not_consume_input_or_allocate_scratch() {
    let mut output = AudioOutput::from_parts(None, Arc::new(Mutex::new(VecDeque::new())), 0.0, 0);
    output.push_samples(
        std::iter::from_fn(|| panic!("disabled producer consumed input")),
        TEST_SOURCE_RATE,
    );
    assert_eq!(output.processed.capacity(), 0);
    assert_eq!(output.resampled.capacity(), 0);
    assert_eq!(output.queued_frames(), 0);
    assert_eq!(output.lowpass_rate, 0.0);
}
