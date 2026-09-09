use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, Once, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::Stage;

mod allocator;
mod histogram;

use histogram::Histogram;

const STAGE_COUNT: usize = Stage::SnapshotRestore as usize + 1;
const STAGE_NAMES: [&str; STAGE_COUNT] = [
    "manager_update",
    "vm_ui_update",
    "field_execution",
    "display_conversion",
    "texture_enqueue_cpu",
    "audio_push",
    "audio_callback_lock_wait",
    "audio_callback_lock_hold",
    "host_operation",
    "snapshot_save",
    "snapshot_restore",
];
const REPORT_INTERVAL: Duration = Duration::from_secs(1);
const MAX_STANDALONE_REPORTS: usize = 3600;
pub(super) static ENABLED: AtomicBool = AtomicBool::new(false);
static STAGES: [Histogram; STAGE_NAMES.len()] = [const { Histogram::new() }; STAGE_NAMES.len()];
static START: OnceLock<Mutex<Instant>> = OnceLock::new();
static INITIALIZE: Once = Once::new();
static ENQUEUES: AtomicU64 = AtomicU64::new(0);
static ENQUEUE_BYTES: AtomicU64 = AtomicU64::new(0);
static CALLBACKS: AtomicU64 = AtomicU64::new(0);
static CALLBACK_FRAMES: AtomicU64 = AtomicU64::new(0);
static UNDERRUNS: AtomicU64 = AtomicU64::new(0);
static MISSING: AtomicU64 = AtomicU64::new(0);
static OVERFLOW: AtomicU64 = AtomicU64::new(0);
static OVERFLOW_BATCHES: AtomicU64 = AtomicU64::new(0);
static QUEUE_SAMPLES: AtomicU64 = AtomicU64::new(0);
static QUEUE_MIN: AtomicU64 = AtomicU64::new(u64::MAX);
static QUEUE_MAX: AtomicU64 = AtomicU64::new(0);
static AUDIO_RATE: AtomicU64 = AtomicU64::new(0);
static AUDIO_CHANNELS: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Span {
    stage: Stage,
    started: Option<Instant>,
}

pub(crate) fn span(stage: Stage) -> Span {
    Span {
        stage,
        started: ENABLED.load(Relaxed).then(Instant::now),
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            STAGES[self.stage as usize].record(started.elapsed());
        }
    }
}

/// Enables measurement. Call between workloads, with producers quiescent.
/// Live callbacks can straddle the boundary; snapshots are not atomic transactions.
pub(crate) fn reset() {
    ENABLED.store(false, Relaxed);
    for histogram in &STAGES {
        histogram.reset();
    }
    for counter in [
        &ENQUEUES,
        &ENQUEUE_BYTES,
        &CALLBACKS,
        &CALLBACK_FRAMES,
        &UNDERRUNS,
        &MISSING,
        &OVERFLOW,
        &OVERFLOW_BATCHES,
        &QUEUE_SAMPLES,
        &QUEUE_MAX,
    ] {
        counter.store(0, Relaxed);
    }
    QUEUE_MIN.store(u64::MAX, Relaxed);
    *START
        .get_or_init(|| Mutex::new(Instant::now()))
        .lock()
        .unwrap() = Instant::now();
    allocator::reset();
    ENABLED.store(
        std::env::var("COCOVM_PERF_DISABLE_METRICS").as_deref() != Ok("1"),
        Relaxed,
    );
}

pub(crate) fn snapshot() -> Value {
    // Read allocation totals before constructing the JSON representation.
    let allocations = allocator::snapshot();
    let duration = START.get().map(|start| start.lock().unwrap().elapsed());
    let stages: serde_json::Map<String, Value> = STAGE_NAMES
        .iter()
        .zip(&STAGES)
        .map(|(name, histogram)| ((*name).into(), histogram.snapshot()))
        .collect();
    json!({
        "enabled": ENABLED.load(Relaxed),
        "scope": "process-wide aggregates across all VMs; sampled concurrently",
        "measurement_duration_seconds": duration.unwrap_or_default().as_secs_f64(),
        "histogram": "fixed nanosecond buckets, 16 subdivisions per power of two; percentiles are upper bounds (at most 6.25% bucket width)",
        "stages": stages,
        "allocations": allocations,
        "texture_enqueue_cpu": {"count": ENQUEUES.load(Relaxed), "bytes": ENQUEUE_BYTES.load(Relaxed)},
        "audio": {
            "config_scope": "last successfully opened output stream",
            "device_rate_hz": AUDIO_RATE.load(Relaxed), "device_channels": AUDIO_CHANNELS.load(Relaxed),
            "callbacks": CALLBACKS.load(Relaxed), "underrun_callbacks": UNDERRUNS.load(Relaxed),
            "requested_frames": CALLBACK_FRAMES.load(Relaxed),
            "missing_frames": MISSING.load(Relaxed), "overflow_frames": OVERFLOW.load(Relaxed),
            "overflow_batches": OVERFLOW_BATCHES.load(Relaxed),
            "queue_samples": QUEUE_SAMPLES.load(Relaxed),
            "queue_min_frames": QUEUE_MIN.load(Relaxed).min(if QUEUE_SAMPLES.load(Relaxed) == 0 {0} else {u64::MAX}),
            "queue_max_frames": QUEUE_MAX.load(Relaxed)
        }
    })
}

pub(crate) fn texture_enqueue(bytes: usize) {
    if ENABLED.load(Relaxed) {
        ENQUEUES.fetch_add(1, Relaxed);
        ENQUEUE_BYTES.fetch_add(bytes as u64, Relaxed);
    }
}

pub(crate) fn audio_queue(depth: usize, dropped: usize) {
    if ENABLED.load(Relaxed) {
        queue_depth(depth);
        OVERFLOW.fetch_add(dropped as u64, Relaxed);
        OVERFLOW_BATCHES.fetch_add(u64::from(dropped > 0), Relaxed);
    }
}

fn queue_depth(depth: usize) {
    QUEUE_SAMPLES.fetch_add(1, Relaxed);
    QUEUE_MIN.fetch_min(depth as u64, Relaxed);
    QUEUE_MAX.fetch_max(depth as u64, Relaxed);
}

/// Called once per callback, after releasing the audio queue mutex.
pub(crate) fn audio_callback(missing: usize, before: usize, after: usize) {
    if ENABLED.load(Relaxed) {
        CALLBACKS.fetch_add(1, Relaxed);
        CALLBACK_FRAMES.fetch_add((before.saturating_sub(after) + missing) as u64, Relaxed);
        UNDERRUNS.fetch_add(u64::from(missing > 0), Relaxed);
        MISSING.fetch_add(missing as u64, Relaxed);
        queue_depth(before);
        queue_depth(after);
    }
}

pub(crate) fn audio_config(rate: u32, channels: u16) {
    AUDIO_RATE.store(rate.into(), Relaxed);
    AUDIO_CHANNELS.store(channels.into(), Relaxed);
}

/// Open the optional output once. A reporter thread writes cumulative snapshots;
/// it holds no audio locks, and does not retain earlier reports in memory.
pub(crate) fn initialize() {
    INITIALIZE.call_once(|| {
        // The timed scenario driver owns its final output file and reset boundary.
        if std::env::var_os("COCOVM_PERF_SCENARIO").is_some() {
            return;
        }
        let Some(path) = std::env::var_os("COCOVM_PERF_OUTPUT") else {
            return;
        };
        let output = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path);
        let Ok(mut output) = output else {
            tracing::warn!("could not open COCOVM_PERF_OUTPUT");
            return;
        };
        reset();
        std::thread::spawn(move || {
            use std::io::Write;
            for _ in 0..MAX_STANDALONE_REPORTS {
                std::thread::sleep(REPORT_INTERVAL);
                if serde_json::to_writer(&mut output, &snapshot()).is_err()
                    || output.write_all(b"\n").is_err()
                {
                    tracing::warn!("could not write COCOVM_PERF_OUTPUT");
                    break;
                }
            }
            ENABLED.store(false, Relaxed);
        });
    });
}
