use super::*;

#[test]
fn percentiles_bound_ranked_samples_and_max_is_exact() {
    let histogram = Histogram::new();
    for nanos in 1..=100 {
        histogram.record(Duration::from_nanos(nanos));
    }
    let result = histogram.snapshot();
    assert_eq!(result["count"], 100);
    assert_eq!(result["p95_ns"], 95);
    assert_eq!(result["p99_ns"], 99);
    assert_eq!(result["max_ns"], 100);
    histogram.reset();
    assert_eq!(histogram.snapshot()["count"], 0);
    assert_eq!(histogram.snapshot()["p99_ns"], 0);
}

#[test]
fn largest_duration_saturates_bucket_without_overflow() {
    let histogram = Histogram::new();
    histogram.record(Duration::MAX);
    assert_eq!(histogram.snapshot()["p99_ns"], u64::MAX);
    assert_eq!(histogram.snapshot()["max_ns"], u64::MAX);
}
