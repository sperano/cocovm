use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Duration;

use serde_json::{Value, json};

const LINEAR_BITS: u32 = 4;
const SUBDIVISIONS: usize = 1 << LINEAR_BITS;
const BUCKET_COUNT: usize = (u64::BITS - LINEAR_BITS) as usize * SUBDIVISIONS + SUBDIVISIONS;
const PERCENT: u64 = 100;
const P95: u64 = 95;
const P99: u64 = 99;

pub(super) struct Histogram {
    buckets: [AtomicU64; BUCKET_COUNT],
    max: AtomicU64,
}

impl Histogram {
    pub(super) const fn new() -> Self {
        Self {
            buckets: [const { AtomicU64::new(0) }; BUCKET_COUNT],
            max: AtomicU64::new(0),
        }
    }

    pub(super) fn record(&self, elapsed: Duration) {
        let nanos = elapsed.as_nanos().min(u64::MAX.into()) as u64;
        let shift = nanos
            .checked_ilog2()
            .unwrap_or(0)
            .saturating_sub(LINEAR_BITS);
        let bucket = shift as usize * SUBDIVISIONS + (nanos >> shift) as usize;
        self.buckets[bucket].fetch_add(1, Relaxed);
        self.max.fetch_max(nanos, Relaxed);
    }

    pub(super) fn reset(&self) {
        for bucket in &self.buckets {
            bucket.store(0, Relaxed);
        }
        self.max.store(0, Relaxed);
    }

    pub(super) fn snapshot(&self) -> Value {
        let buckets = self.buckets.each_ref().map(|bucket| bucket.load(Relaxed));
        let count = buckets.iter().sum();
        json!({ "count": count, "p95_ns": percentile(&buckets, count, P95),
            "p99_ns": percentile(&buckets, count, P99), "max_ns": self.max.load(Relaxed) })
    }
}

fn percentile(buckets: &[u64; BUCKET_COUNT], count: u64, percent: u64) -> u64 {
    if count == 0 {
        return 0;
    }
    let rank = (u128::from(count) * u128::from(percent)).div_ceil(u128::from(PERCENT));
    let mut seen = 0_u128;
    for (index, bucket) in buckets.iter().enumerate() {
        seen += u128::from(*bucket);
        if seen >= rank {
            return upper_bound(index);
        }
    }
    u64::MAX
}

fn upper_bound(index: usize) -> u64 {
    if index < SUBDIVISIONS {
        return index as u64;
    }
    let shift = (index / SUBDIVISIONS - 1) as u32;
    let value = (index % SUBDIVISIONS + SUBDIVISIONS + 1) as u128;
    ((value << shift) - 1).min(u64::MAX.into()) as u64
}

#[cfg(test)]
#[path = "histogram_test.rs"]
mod tests;
