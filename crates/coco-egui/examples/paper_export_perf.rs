//! Deterministic comparison of buffered and streamed printer export paths.
//!
//! Usage: `cargo run -p coco-egui --release --example paper_export_perf -- MODE`
//! where MODE is `legacy-pdf`, `legacy-roll-png`, `streamed-pdf`,
//! `streamed-roll-png`, `async-pdf`, `snapshot`, `dense-row-snapshot`, or
//! `dense-dot-snapshot`.

#[allow(dead_code)]
#[path = "../src/paper_export.rs"]
mod paper_export;
#[allow(dead_code)]
#[path = "../src/paper_render.rs"]
mod paper_render;

use coco_core::bitbanger::PrinterSink;
use coco_core::dmp::DmpHandle;
use coco_core::printer::Paper;
use paper_export::{ExportFormat, ExportRequest};
use paper_render::{PAGE_HEIGHT_IN, RASTER_DPI};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const EXPORT_PAGE_COUNT: u32 = 32;
const SNAPSHOT_PAGE_COUNT: u32 = 2_000;
const DENSE_ROW_COUNT: u32 = 110_000;
const DENSE_DOT_COUNT: u32 = 2_000_000;
const LINES_PER_PAGE: usize = 66;
const PERF_LINE: &[u8] = b"PERFORMANCE EXPORT 0123456789\r";
const MEBIBYTE: usize = 1024 * 1024;
const SNAPSHOT_LIMIT_BYTES: usize = 16 * MEBIBYTE;
const MILLISECONDS_PER_SECOND: f64 = 1_000.0;

fn printer(page_count: u32) -> DmpHandle {
    let mut printer = DmpHandle::new();
    for _ in 0..page_count {
        for &byte in PERF_LINE {
            printer.write_byte(byte);
        }
        for _ in 1..LINES_PER_PAGE {
            printer.write_byte(b'\r');
        }
    }
    printer
}

fn rasters(paper: &coco_core::printer::Paper) -> Vec<paper_render::RasterImage> {
    (0..EXPORT_PAGE_COUNT)
        .map(|page| {
            paper_render::rasterize(
                paper,
                page as f32 * PAGE_HEIGHT_IN,
                PAGE_HEIGHT_IN,
                RASTER_DPI,
                false,
            )
        })
        .collect()
}

fn request(
    paper: coco_core::printer::Paper,
    target: PathBuf,
    format: ExportFormat,
) -> ExportRequest {
    ExportRequest {
        paper,
        page_count: EXPORT_PAGE_COUNT,
        green_bar: false,
        target,
        format,
    }
}

fn run_legacy(mode: &str, paper: &coco_core::printer::Paper, target: &Path) {
    match mode {
        "legacy-pdf" => {
            paper_export::save_pdf(&rasters(paper), RASTER_DPI, target).expect("save PDF");
        }
        "legacy-roll-png" => {
            let image = paper_render::rasterize(
                paper,
                0.0,
                EXPORT_PAGE_COUNT as f32 * PAGE_HEIGHT_IN,
                RASTER_DPI,
                false,
            );
            paper_export::save_png(&image, target).expect("save PNG");
        }
        _ => unreachable!(),
    }
}

fn streamed_format(mode: &str) -> ExportFormat {
    match mode {
        "streamed-pdf" | "async-pdf" => ExportFormat::Pdf { trimmed: false },
        "streamed-roll-png" => ExportFormat::RollPng,
        _ => panic!("unknown mode {mode:?}"),
    }
}

fn run_async(request: ExportRequest) -> (Duration, Duration) {
    let started = Instant::now();
    let worker = std::thread::spawn(move || paper_export::run(request, || false, |_| {}));
    let submission = started.elapsed();
    worker
        .join()
        .expect("export worker panicked")
        .expect("export");
    (submission, started.elapsed())
}

fn dense_paper(mode: &str) -> Paper {
    let mut paper = Paper::new();
    let (count, distinct_rows) = match mode {
        "dense-row-snapshot" => (DENSE_ROW_COUNT, true),
        "dense-dot-snapshot" => (DENSE_DOT_COUNT, false),
        _ => unreachable!(),
    };
    for index in 0..count {
        paper.mark(index, if distinct_rows { index } else { 0 });
    }
    paper
}

fn run_dense_snapshot(mode: &str) {
    let paper = dense_paper(mode);
    let estimated_bytes = paper.estimated_owned_bytes();
    assert!(estimated_bytes <= SNAPSHOT_LIMIT_BYTES);
    let started = Instant::now();
    let snapshot = paper.clone();
    let elapsed = started.elapsed();
    println!(
        "mode={mode} dots={} estimated_bytes={estimated_bytes} snapshot_ms={:.3}",
        snapshot.extent().dot_count,
        elapsed.as_secs_f64() * MILLISECONDS_PER_SECOND,
    );
}

fn main() {
    let mode = std::env::args().nth(1).expect("export mode");
    if matches!(mode.as_str(), "dense-row-snapshot" | "dense-dot-snapshot") {
        run_dense_snapshot(&mode);
        return;
    }
    let page_count = if mode == "snapshot" {
        SNAPSHOT_PAGE_COUNT
    } else {
        EXPORT_PAGE_COUNT
    };
    let printer = printer(page_count);
    let snapshot_started = Instant::now();
    let paper = printer
        .paper_snapshot_with_limit(SNAPSHOT_LIMIT_BYTES)
        .expect("paper snapshot budget");
    let snapshot = snapshot_started.elapsed();
    let target = std::env::temp_dir().join(format!(
        "cocovm-paper-export-perf-{}-{mode}",
        std::process::id()
    ));
    let started = Instant::now();
    let (submission, elapsed) = match mode.as_str() {
        "legacy-pdf" | "legacy-roll-png" => {
            run_legacy(&mode, &paper, &target);
            (None, started.elapsed())
        }
        "snapshot" => (None, Duration::ZERO),
        _ => {
            let request = request(paper, target.clone(), streamed_format(&mode));
            if mode == "async-pdf" {
                let (submitted, total) = run_async(request);
                (Some(submitted), total)
            } else {
                paper_export::run(request, || false, |_| {}).expect("export");
                (None, started.elapsed())
            }
        }
    };
    let keep_output = std::env::var_os("COCOVM_KEEP_PERF_OUTPUT").is_some();
    if target.is_file() && !keep_output {
        std::fs::remove_file(&target).expect("remove output");
    }
    println!(
        "mode={mode} pages={page_count} dots={} snapshot_ms={:.3} submission_ms={} elapsed_ms={:.3}",
        printer.paper_extent().dot_count,
        snapshot.as_secs_f64() * MILLISECONDS_PER_SECOND,
        submission.map_or_else(
            || "n/a".to_string(),
            |value| format!("{:.3}", value.as_secs_f64() * MILLISECONDS_PER_SECOND)
        ),
        elapsed.as_secs_f64() * MILLISECONDS_PER_SECOND,
    );
    if keep_output {
        println!("output={}", target.display());
    }
}
