use std::cell::Cell;
use std::path::{Path, PathBuf};

use coco_core::printer::Paper;

use super::*;

fn scratch_path(name: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "cocovm-paper-export-test-{}-{sequence}-{name}",
        std::process::id()
    ))
}

fn request(target: &Path, page_count: u32, format: ExportFormat) -> ExportRequest {
    ExportRequest {
        paper: Paper::new(),
        page_count,
        green_bar: false,
        target: target.to_path_buf(),
        format,
    }
}

#[test]
fn synchronous_entry_point_exports_a_page_png() {
    let path = scratch_path("page.png");
    let mut progress = Vec::new();
    run(
        request(&path, 1, ExportFormat::PagePng { page: 0 }),
        || false,
        |completed| progress.push(completed),
    )
    .unwrap();

    let image = image::open(&path).unwrap();
    assert_eq!(image.width(), (PAPER_WIDTH_IN * RASTER_DPI).round() as u32);
    assert_eq!(progress, [1]);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn roll_png_streams_every_page_into_one_image() {
    let path = scratch_path("roll.png");
    let page_count = 2;
    run(
        request(&path, page_count, ExportFormat::RollPng),
        || false,
        |_| {},
    )
    .unwrap();

    let image = image::open(&path).unwrap();
    let (_, page_height) = page_dimensions();
    assert_eq!(image.height(), page_height * page_count);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn pdf_stream_has_one_page_object_per_page_and_valid_xref() {
    let path = scratch_path("streamed.pdf");
    let page_count = 2;
    run(
        request(&path, page_count, ExportFormat::Pdf { trimmed: false }),
        || false,
        |_| {},
    )
    .unwrap();

    let bytes = std::fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(bytes.starts_with(b"%PDF-"));
    assert_eq!(text.matches("/Type /Page ").count(), page_count as usize);
    assert!(text.contains("xref"));
    assert!(text.trim_end().ends_with("%%EOF"));
    assert_pdf_xref_offsets(
        &bytes,
        PDF_FIXED_OBJECT_COUNT + PDF_OBJECTS_PER_PAGE * page_count,
    );
    std::fs::remove_file(path).unwrap();
}

fn assert_pdf_xref_offsets(bytes: &[u8], object_count: u32) {
    const START_XREF: &[u8] = b"startxref\n";
    let marker = bytes
        .windows(START_XREF.len())
        .rposition(|window| window == START_XREF)
        .unwrap();
    let start = marker + START_XREF.len();
    let end = bytes[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap()
        + start;
    let xref_offset: usize = std::str::from_utf8(&bytes[start..end])
        .unwrap()
        .parse()
        .unwrap();
    assert!(bytes[xref_offset..].starts_with(b"xref\n"));
    let entries = bytes[xref_offset..].split(|byte| *byte == b'\n').skip(3);
    for (index, entry) in entries.take(object_count as usize).enumerate() {
        let offset: usize = std::str::from_utf8(&entry[..10]).unwrap().parse().unwrap();
        let header = format!("{} 0 obj\n", index + 1);
        assert!(bytes[offset..].starts_with(header.as_bytes()));
    }
}

#[test]
fn all_page_pngs_commit_as_one_directory() {
    let path = scratch_path("pages");
    run(request(&path, 2, ExportFormat::PagesPng), || false, |_| {}).unwrap();

    assert!(path.join("page-1.png").is_file());
    assert!(path.join("page-2.png").is_file());
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn cancellation_preserves_destination_and_removes_temporary_file() {
    let path = scratch_path("preserved.png");
    let original = b"existing destination";
    std::fs::write(&path, original).unwrap();
    let cancelled = Cell::new(false);

    let result = run(
        request(&path, 2, ExportFormat::RollPng),
        || cancelled.get(),
        |_| cancelled.set(true),
    );

    assert_eq!(result, Err(ExportError::Cancelled));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(
        !temporary_files_for(&path)
            .iter()
            .any(|entry| entry.is_file())
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn dense_page_observes_cancellation_during_rasterization() {
    const DENSE_DOT_COUNT: u32 = 10_000;
    const CANCEL_AFTER_CHECKS: usize = 8;

    let path = scratch_path("dense-cancel.png");
    let original = b"existing destination";
    std::fs::write(&path, original).unwrap();
    let mut paper = Paper::new();
    for _ in 0..DENSE_DOT_COUNT {
        paper.mark(0, 0);
    }
    let checks = Cell::new(0);
    let request = ExportRequest {
        paper,
        ..request(&path, 1, ExportFormat::PagePng { page: 0 })
    };

    let result = run(
        request,
        || {
            let count = checks.get() + 1;
            checks.set(count);
            count >= CANCEL_AFTER_CHECKS
        },
        |_| {},
    );

    assert_eq!(result, Err(ExportError::Cancelled));
    assert!(checks.get() >= CANCEL_AFTER_CHECKS);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(temporary_files_for(&path).is_empty());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn pdf_observes_cancellation_during_row_encoding() {
    const CANCEL_AFTER_CHECKS: usize = 10;

    let path = scratch_path("pdf-row-cancel.pdf");
    let original = b"existing destination";
    std::fs::write(&path, original).unwrap();
    let checks = Cell::new(0);

    let result = run(
        request(&path, 1, ExportFormat::Pdf { trimmed: false }),
        || {
            let count = checks.get() + 1;
            checks.set(count);
            count >= CANCEL_AFTER_CHECKS
        },
        |_| {},
    );

    assert_eq!(result, Err(ExportError::Cancelled));
    assert!(checks.get() >= CANCEL_AFTER_CHECKS);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(temporary_files_for(&path).is_empty());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn io_failure_is_actionable_and_preserves_existing_destination() {
    let path = scratch_path("existing-directory.png");
    std::fs::create_dir(&path).unwrap();
    let sentinel = path.join("keep.txt");
    std::fs::write(&sentinel, b"keep").unwrap();

    let error = run(
        request(&path, 1, ExportFormat::PagePng { page: 0 }),
        || false,
        |_| {},
    )
    .unwrap_err();

    let message = error.to_string();
    assert!(message.contains("could not replace printer export"));
    assert!(message.contains(path.to_str().unwrap()));
    assert_eq!(std::fs::read(sentinel).unwrap(), b"keep");
    assert!(temporary_files_for(&path).is_empty());
    std::fs::remove_dir_all(path).unwrap();
}

fn temporary_files_for(destination: &Path) -> Vec<PathBuf> {
    let parent = destination.parent().unwrap();
    let prefix = format!(".{}.", destination.file_name().unwrap().to_string_lossy());
    std::fs::read_dir(parent)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
        })
        .collect()
}

#[test]
fn fixed_dpi_page_respects_the_working_memory_budget() {
    let (width, height) = page_dimensions();
    let estimated = width as usize * height as usize * RGBA_BYTES_PER_PIXEL
        + width as usize * RGB_BYTES_PER_PIXEL
        + PNG_ENCODER_ALLOWANCE_BYTES;
    assert!(estimated <= MAX_EXPORT_WORKING_MEMORY_BYTES);
}

#[test]
fn pdf_object_table_is_included_in_the_working_memory_budget() {
    let path = scratch_path("too-many-pages.pdf");
    let base_request = request(
        &path,
        u32::MAX / PDF_OBJECTS_PER_PAGE,
        ExportFormat::RollPng,
    );
    let base_estimate = validate_request(&base_request);
    let pdf_request = ExportRequest {
        format: ExportFormat::Pdf { trimmed: false },
        ..base_request
    };

    assert!(base_estimate.is_ok());
    let error = validate_request(&pdf_request).unwrap_err().to_string();
    assert!(error.contains("exceeding") || error.contains("too many pages"));
}

#[test]
fn legacy_buffered_helpers_remain_available_for_benchmarks() {
    struct EmptyDots;
    impl paper_render::DotSource for EmptyDots {
        fn try_visit_dots_in_range(
            &self,
            _y0: u32,
            _y1: u32,
            _visit: &mut dyn FnMut(u32, u32) -> std::ops::ControlFlow<()>,
        ) -> std::ops::ControlFlow<()> {
            std::ops::ControlFlow::Continue(())
        }
    }
    let dpi = 30.0;
    let page = paper_render::rasterize(&EmptyDots, 0.0, PAGE_HEIGHT_IN, dpi, false);
    let cropped = crop_to_trimmed_width(&page, dpi);
    assert_eq!(cropped.width, (8.5 * dpi).round() as u32);
    let png_path = scratch_path("legacy.png");
    let pdf_path = scratch_path("legacy.pdf");
    save_png(&page, &png_path).unwrap();
    save_pdf(&[page], dpi, &pdf_path).unwrap();
    assert!(std::fs::read(&pdf_path).unwrap().starts_with(b"%PDF-"));
    std::fs::remove_file(png_path).unwrap();
    std::fs::remove_file(pdf_path).unwrap();
}
