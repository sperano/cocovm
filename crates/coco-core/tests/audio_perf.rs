//! Deterministic performance-fixture validation. Timing lives in `perf_baseline`.
#[path = "../examples/perf/workloads_test.rs"]
mod tests;
#[allow(dead_code)]
#[path = "../examples/perf/workloads.rs"]
mod workloads;
