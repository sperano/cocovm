//! Headless sign-off example for the virtual fanfold-paper renderer
//!: feeds a canned byte stream
//! through a real [`DMP105Handle`], rasterizes the whole printed roll (plus
//! one trailing blank page, same "+2 pages" rule the live window uses), and
//! writes two PNGs so a human can eyeball the result without launching the
//! GUI.
//!
//! No `eframe`/`egui` dependency — just `coco_core` plus the egui-free
//! `paper_render` rasterizer (included directly via `#[path]`, since
//! `coco-egui` is a binary-only crate with no library target for an example
//! to depend on).
//!
//! Usage: `cargo run -p coco-egui --example paper_preview --release`

#[path = "../src/paper_render.rs"]
mod paper_render;

use coco_core::bitbanger::PrinterSink;
use coco_core::dmp105::DMP105Handle;
use coco_core::printer::Y_UNITS_PER_INCH;
use paper_render::{PAGE_HEIGHT_IN, RASTER_DPI};

/// Bare `\r`s fed after the styled lines, to land solidly on page 2. 66
/// lines/page at 6 LPI is the real "lines per page" convention
/// (`PAGE_HEIGHT_IN * Y_UNITS_PER_INCH / 12 == 66` exactly, checked below);
/// this is comfortably past that boundary.
const BLANK_LINES_TO_PAGE_2: usize = 75;

/// Display convenience for `full.png` — not a T5 spec value, just a
/// sensible width for a human skimming the whole roll in an image viewer.
const FULL_PREVIEW_WIDTH_PX: u32 = 900;

/// `detail.png` crop origin/size, in inches. Width (3") is spec-given; the
/// rest are this example's own choice of a region with real content: near
/// the roll's start so the left tractor strip's first sprocket hole
/// (0.25") and the plain text lines fed first (all within the first ~0.9")
/// both land inside the crop.
const DETAIL_X0_IN: f32 = 0.0;
const DETAIL_Y0_IN: f32 = 0.0;
const DETAIL_WIDTH_IN: f32 = 3.0;
const DETAIL_HEIGHT_IN: f32 = 1.0;

fn feed(printer: &mut DMP105Handle, bytes: &[u8]) {
    for &b in bytes {
        printer.write_byte(b);
    }
}

fn main() {
    // 66 lines/page @ 6 LPI: the real-hardware convention this module's
    // page-boundary byte count is chosen to cross.
    assert_eq!(
        (PAGE_HEIGHT_IN * Y_UNITS_PER_INCH as f32 / 12.0).round() as u32,
        66,
        "PAGE_HEIGHT_IN and Y_UNITS_PER_INCH must still agree with the 66-lines-per-page convention"
    );

    let mut printer = DMP105Handle::new();

    // Plain ASCII lines, bare-CR terminated (BASIC's own line ending).
    feed(&mut printer, b"REM FANFOLD PAPER TEST\r");
    feed(&mut printer, b"10 PRINT \"HELLO\"\r");
    feed(&mut printer, b"20 PRINT \"WORLD\"\r");
    feed(&mut printer, b"30 END\r");

    // Elongated line: ESC 0E ... ESC 0F.
    feed(&mut printer, b"\x1B\x0EELONGATED TEXT LINE\x1B\x0F\r");

    // Underlined line: raw START_UNDERLINE (0x0F, no ESC) ... raw END_UNDERLINE (0x0E).
    feed(&mut printer, b"\x0FUNDERLINED TEXT LINE\x0E\r");

    // Bold line: ESC 1F ... ESC 20.
    feed(&mut printer, b"\x1B\x1FBOLD TEXT LINE\x1B\x20\r");

    // Condensed-pitch line: ESC 14 (condensed) ... ESC 13 (restore Normal).
    feed(&mut printer, b"\x1B\x14CONDENSED PITCH LINE\x1B\x13\r");

    // Enough bare CRs (default mode is CR+LF, 1/6" pitch = 12 y-units each)
    // to cross a full page boundary onto page 2.
    for _ in 0..BLANK_LINES_TO_PAGE_2 {
        feed(&mut printer, b"\r");
    }
    feed(&mut printer, b"PAGE TWO LINE\r");

    // Short graphics-mode burst: SELECT_GRAPHICS, a handful of all-bits-set
    // data bytes, END_GRAPHICS.
    feed(&mut printer, b"\x12");
    feed(&mut printer, &[0xFFu8; 40]);
    feed(&mut printer, b"\x1E");

    // Rasterize the full printed region plus one trailing blank page, same
    // "+2 pages" rule the live paper window uses.
    let extent = printer.paper_extent();
    let last_content_page = if extent.dot_count == 0 {
        0
    } else {
        let max_y_in = extent.max_y as f32 / Y_UNITS_PER_INCH as f32;
        (max_y_in / PAGE_HEIGHT_IN).floor() as u32
    };
    let total_pages = last_content_page + 2;
    let total_height_in = total_pages as f32 * PAGE_HEIGHT_IN;

    let image = paper_render::rasterize(&printer, 0.0, total_height_in, RASTER_DPI, false);

    let out_dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/paper_preview");
    std::fs::create_dir_all(&out_dir).expect("create target/paper_preview");

    let full_rgba = image::RgbaImage::from_raw(image.width, image.height, image.pixels.clone())
        .expect("rasterized buffer matches its own declared dimensions");

    // full.png: the whole roll, downscaled to a sensible preview width if
    // it's wider than that (it always will be, at RASTER_DPI).
    let full_scale = FULL_PREVIEW_WIDTH_PX as f32 / image.width as f32;
    let full_preview_height = ((image.height as f32) * full_scale).round().max(1.0) as u32;
    let full_preview = image::imageops::resize(
        &full_rgba,
        FULL_PREVIEW_WIDTH_PX,
        full_preview_height,
        image::imageops::FilterType::Lanczos3,
    );
    let full_path = out_dir.join("full.png");
    full_preview.save(&full_path).expect("write full.png");

    // detail.png: a 100%-scale crop with real tractor-strip + perforation +
    // ink texture in view.
    let x0 = (DETAIL_X0_IN * RASTER_DPI).round() as u32;
    let y0 = (DETAIL_Y0_IN * RASTER_DPI).round() as u32;
    let w = (DETAIL_WIDTH_IN * RASTER_DPI).round() as u32;
    let h = (DETAIL_HEIGHT_IN * RASTER_DPI).round() as u32;
    let detail = image::imageops::crop_imm(&full_rgba, x0, y0, w, h).to_image();
    let detail_path = out_dir.join("detail.png");
    detail.save(&detail_path).expect("write detail.png");

    println!("wrote {}", full_path.display());
    println!("wrote {}", detail_path.display());
}
