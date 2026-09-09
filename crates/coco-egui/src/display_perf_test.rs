//! Run alone with `--ignored --test-threads=1 --nocapture`; allocation counters
//! cover the process, so concurrently running tests invalidate the measurement.
use std::hint::black_box;
use std::time::Instant;

use coco_core::raster::{CANVAS_H, CANVAS_W};
use serde_json::{Value, json};

use super::processor_tests::{patterned_frame, previous_process};
use super::*;

const ITERATIONS: u32 = 120;
const FIXED_SEED: u32 = 7;
const MICROSECONDS_PER_SECOND: f64 = 1_000_000.0;

fn measure(mut render: impl FnMut()) -> Value {
    render(); // Warm lookup tables and retained storage before counting.
    crate::perf::reset();
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        render();
    }
    let elapsed = started.elapsed().as_secs_f64();
    let metrics = crate::perf::snapshot();
    json!({
        "elapsed_seconds": elapsed,
        "microseconds_per_frame": elapsed * MICROSECONDS_PER_SECOND / f64::from(ITERATIONS),
        "allocations": metrics["allocations"],
    })
}

fn measure_tv(tv: TV, scanline_pct: u8, src: &[u8]) -> Value {
    let settings = TVSettings {
        scanline_pct,
        ..TVSettings::default()
    };
    let before = measure(|| {
        black_box(previous_process(tv, settings, FIXED_SEED, CANVAS_W, src));
    });
    let mut processor = Processor::default();
    let after = measure(|| {
        black_box(processor.process(Display::TV(tv), settings, FIXED_SEED, CANVAS_W, src));
    });
    assert_eq!(after["allocations"]["count"], 0);
    assert_eq!(after["allocations"]["requested_bytes"], 0);
    let expected = previous_process(tv, settings, FIXED_SEED, CANVAS_W, src);
    let actual = processor.process(Display::TV(tv), settings, FIXED_SEED, CANVAS_W, src);
    assert_eq!(actual.pixels, expected);
    json!({
        "tv": format!("{tv:?}"), "scanline_pct": scanline_pct,
        "noise_pct": settings.noise_pct, "fixed_seed": FIXED_SEED,
        "previous": before, "reused": after,
    })
}

#[test]
#[ignore = "isolated process-wide allocation and timing measurement"]
fn tv_steady_state_allocation_measurement() {
    let src = patterned_frame(CANVAS_W, CANVAS_H);
    let mut cases = Vec::new();
    for tv in [TV::Color, TV::BW] {
        for scanline_pct in [0, DEFAULT_SCANLINE_PCT] {
            cases.push(measure_tv(tv, scanline_pct, &src));
        }
    }
    println!(
        "{}",
        json!({
            "iterations": ITERATIONS, "width": CANVAS_W, "height": CANVAS_H,
            "cases": cases,
        })
    );
}
