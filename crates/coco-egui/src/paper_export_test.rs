use super::*;
use crate::paper_render;

/// A scratch path under the OS temp dir, unique per test run (PID + a
/// per-call counter) so parallel `cargo test` runs of this file never
/// collide on the same file (`bitbanger.rs`'s `scratch_path` pattern).
fn scratch_path(name: &str) -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "cocovm-paper-export-test-{}-{n}-{name}",
        std::process::id()
    ))
}

struct EmptyDots;
impl paper_render::DotSource for EmptyDots {
    fn dots_in_range(&self, _y0: u32, _y1: u32) -> Vec<(u32, u32)> {
        Vec::new()
    }
}

#[test]
fn png_round_trips_through_the_image_crate() {
    let dpi = 50.0;
    let img = paper_render::rasterize(&EmptyDots, 0.0, 1.0, dpi, false);
    let path = scratch_path("roundtrip.png");

    save_png(&img, &path).expect("save_png");

    let bytes = std::fs::read(&path).expect("read back the written PNG");
    assert!(!bytes.is_empty(), "PNG file must not be empty");

    let reloaded = image::open(&path).expect("image crate must be able to reload it");
    assert_eq!(reloaded.width(), img.width);
    assert_eq!(reloaded.height(), img.height);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn pdf_starts_with_the_pdf_magic_and_has_a_trailer() {
    let dpi = 50.0;
    let page =
        paper_render::rasterize(&EmptyDots, 0.0, paper_render::PAGE_HEIGHT_IN, dpi, false);
    let mut buf = Vec::new();
    write_pdf(std::slice::from_ref(&page), dpi, &mut buf).expect("write_pdf");

    assert!(buf.starts_with(b"%PDF-"), "must start with the PDF magic");
    let text = String::from_utf8_lossy(&buf);
    assert!(text.contains("trailer"), "must have a trailer");
    assert!(text.trim_end().ends_with("%%EOF"), "must end with %%EOF");
}

#[test]
fn pdf_page_count_matches_the_pages_given() {
    let dpi = 30.0;
    let pages: Vec<RasterImage> = (0..3)
        .map(|_| {
            paper_render::rasterize(&EmptyDots, 0.0, paper_render::PAGE_HEIGHT_IN, dpi, false)
        })
        .collect();
    let mut buf = Vec::new();
    write_pdf(&pages, dpi, &mut buf).expect("write_pdf");

    let text = String::from_utf8_lossy(&buf);
    // "/Type /Page " (trailing space) matches only real Page objects,
    // not the "/Type /Pages" tree object (immediately followed by "s",
    // not a space).
    let page_object_count = text.matches("/Type /Page ").count();
    assert_eq!(page_object_count, pages.len());
    assert!(text.contains("/Count 3"));
}

#[test]
fn save_pdf_writes_a_non_empty_file() {
    let dpi = 30.0;
    let page =
        paper_render::rasterize(&EmptyDots, 0.0, paper_render::PAGE_HEIGHT_IN, dpi, false);
    let path = scratch_path("saved.pdf");

    save_pdf(std::slice::from_ref(&page), dpi, &path).expect("save_pdf");

    let bytes = std::fs::read(&path).expect("read back the written PDF");
    assert!(!bytes.is_empty());
    assert!(bytes.starts_with(b"%PDF-"));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn crop_to_trimmed_width_removes_both_tractor_strips() {
    let dpi = 100.0;
    let img = paper_render::rasterize(&EmptyDots, 0.0, 1.0, dpi, false);
    let cropped = crop_to_trimmed_width(&img, dpi);

    let expected_width_in = PAPER_WIDTH_IN - 2.0 * STRIP_WIDTH_IN;
    assert_eq!(
        expected_width_in, 8.5,
        "trimmed width must be exactly US Letter width"
    );
    assert_eq!(cropped.width, (expected_width_in * dpi).round() as u32);
    assert_eq!(cropped.height, img.height);
}
