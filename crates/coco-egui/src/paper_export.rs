//! Streaming PNG and PDF export for immutable printer-paper snapshots.
//!
//! The worker renders at most one fanfold page at a time. PDF image data is
//! deflated directly into the output stream, and roll PNG rows flow through
//! `png`'s streaming writer. This bounds raster working storage independently
//! of roll length.

use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use coco_core::printer::Paper;
use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::paper_render::{
    self, PAGE_HEIGHT_IN, PAPER_WIDTH_IN, RASTER_DPI, RasterImage, STRIP_WIDTH_IN,
};

const PDF_POINTS_PER_INCH: f32 = 72.0;
const RGBA_BYTES_PER_PIXEL: usize = 4;
const RGB_BYTES_PER_PIXEL: usize = 3;
const MEBIBYTE: usize = 1024 * 1024;
const PNG_ENCODER_ALLOWANCE_BYTES: usize = MEBIBYTE;
const PDF_CATALOG_OBJECT: u32 = 1;
const PDF_PAGES_OBJECT: u32 = 2;
const PDF_FIXED_OBJECT_COUNT: u32 = PDF_PAGES_OBJECT;
const PDF_OBJECTS_PER_PAGE: u32 = 4;
const PDF_FIRST_PAGE_OBJECT: u32 = PDF_PAGES_OBJECT + 1;

/// Maximum estimated working storage for one export worker at the fixed DPI.
pub const MAX_EXPORT_WORKING_MEMORY_BYTES: usize = 32 * MEBIBYTE;

#[path = "paper_export/atomic.rs"]
mod atomic;
#[allow(dead_code)]
#[path = "paper_export/legacy.rs"]
mod legacy;

use atomic::{TemporaryDirectory, write_atomic_file, write_file};
#[allow(unused_imports)]
pub use legacy::{crop_to_trimmed_width, save_pdf, save_png};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    PagePng { page: u32 },
    PagesPng,
    RollPng,
    Pdf { trimmed: bool },
}

pub struct ExportRequest {
    pub paper: Paper,
    pub page_count: u32,
    pub green_bar: bool,
    pub target: PathBuf,
    pub format: ExportFormat,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExportError {
    Cancelled,
    Message(String),
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("printer export cancelled"),
            Self::Message(message) => f.write_str(message),
        }
    }
}

/// Runs an export without UI or file-dialog dependencies.
pub fn run(
    request: ExportRequest,
    is_cancelled: impl Fn() -> bool,
    mut report_progress: impl FnMut(u32),
) -> Result<(), ExportError> {
    validate_request(&request)?;
    match request.format {
        ExportFormat::PagePng { page } => {
            export_page_png(&request, page, &is_cancelled)?;
            report_progress(1);
        }
        ExportFormat::PagesPng => {
            export_page_pngs(&request, &is_cancelled, &mut report_progress)?;
        }
        ExportFormat::RollPng => {
            export_roll_png(&request, &is_cancelled, &mut report_progress)?;
        }
        ExportFormat::Pdf { trimmed } => {
            export_pdf(&request, trimmed, &is_cancelled, &mut report_progress)?;
        }
    }
    Ok(())
}

fn validate_request(request: &ExportRequest) -> Result<(), ExportError> {
    if request.page_count == 0 {
        return Err(message("printer export has no pages"));
    }
    if let ExportFormat::PagePng { page } = request.format
        && page >= request.page_count
    {
        return Err(message(format!(
            "printer page {} is outside the {}-page roll",
            page + 1,
            request.page_count
        )));
    }
    let (width, height) = page_dimensions();
    let raster_bytes = width as usize * height as usize * RGBA_BYTES_PER_PIXEL;
    let row_bytes = width as usize * RGB_BYTES_PER_PIXEL;
    let pdf_offsets_bytes = match request.format {
        ExportFormat::Pdf { .. } => (pdf_object_count(request.page_count)? as usize)
            .saturating_add(1)
            .saturating_mul(std::mem::size_of::<u64>()),
        _ => 0,
    };
    let estimate = raster_bytes
        .saturating_add(row_bytes)
        .saturating_add(PNG_ENCODER_ALLOWANCE_BYTES)
        .saturating_add(pdf_offsets_bytes);
    if estimate > MAX_EXPORT_WORKING_MEMORY_BYTES {
        return Err(message(format!(
            "printer export needs about {estimate} bytes per worker, exceeding the {}-byte limit",
            MAX_EXPORT_WORKING_MEMORY_BYTES
        )));
    }
    Ok(())
}

fn export_page_png(
    request: &ExportRequest,
    page: u32,
    is_cancelled: &impl Fn() -> bool,
) -> Result<(), ExportError> {
    check_cancelled(is_cancelled)?;
    let image = rasterize_page(request, page, is_cancelled).ok_or(ExportError::Cancelled)?;
    check_cancelled(is_cancelled)?;
    write_atomic_file(&request.target, |out| write_png(&image, out))
}

fn export_page_pngs(
    request: &ExportRequest,
    is_cancelled: &impl Fn() -> bool,
    report_progress: &mut impl FnMut(u32),
) -> Result<(), ExportError> {
    let staging = TemporaryDirectory::create(&request.target)?;
    for page in 0..request.page_count {
        check_cancelled(is_cancelled)?;
        let image = rasterize_page(request, page, is_cancelled).ok_or(ExportError::Cancelled)?;
        check_cancelled(is_cancelled)?;
        let path = staging.path().join(format!("page-{}.png", page + 1));
        write_file(&path, |out| write_png(&image, out))?;
        report_progress(page + 1);
    }
    check_cancelled(is_cancelled)?;
    staging.commit(&request.target)
}

fn export_roll_png(
    request: &ExportRequest,
    is_cancelled: &impl Fn() -> bool,
    report_progress: &mut impl FnMut(u32),
) -> Result<(), ExportError> {
    let (width, page_height) = page_dimensions();
    let height = page_height
        .checked_mul(request.page_count)
        .ok_or_else(|| message("printer roll PNG height exceeds the PNG format limit"))?;
    write_atomic_file(&request.target, |out| {
        let mut encoder = png::Encoder::new(out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(png_error)?;
        let mut stream = writer.stream_writer().map_err(png_error)?;
        for page in 0..request.page_count {
            check_cancelled_io(is_cancelled)?;
            let image = rasterize_page(request, page, is_cancelled).ok_or_else(cancelled_io)?;
            check_cancelled_io(is_cancelled)?;
            stream.write_all(&image.pixels)?;
            report_progress(page + 1);
        }
        check_cancelled_io(is_cancelled)?;
        stream.finish().map_err(png_error)
    })
}

fn export_pdf(
    request: &ExportRequest,
    trimmed: bool,
    is_cancelled: &impl Fn() -> bool,
    report_progress: &mut impl FnMut(u32),
) -> Result<(), ExportError> {
    write_atomic_file(&request.target, |out| {
        write_pdf(request, trimmed, out, is_cancelled, report_progress)
    })
}

fn rasterize_page(
    request: &ExportRequest,
    page: u32,
    is_cancelled: &impl Fn() -> bool,
) -> Option<RasterImage> {
    paper_render::rasterize_cancellable(
        &request.paper,
        page as f32 * PAGE_HEIGHT_IN,
        PAGE_HEIGHT_IN,
        RASTER_DPI,
        request.green_bar,
        is_cancelled,
    )
}

fn page_dimensions() -> (u32, u32) {
    (
        (PAPER_WIDTH_IN * RASTER_DPI).round() as u32,
        (PAGE_HEIGHT_IN * RASTER_DPI).round() as u32,
    )
}

fn write_png(image: &RasterImage, out: &mut impl Write) -> io::Result<()> {
    let mut encoder = png::Encoder::new(out, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(png_error)?;
    let mut stream = writer.stream_writer().map_err(png_error)?;
    stream.write_all(&image.pixels)?;
    stream.finish().map_err(png_error)
}

fn write_pdf(
    request: &ExportRequest,
    trimmed: bool,
    out: &mut impl Write,
    is_cancelled: &impl Fn() -> bool,
    report_progress: &mut impl FnMut(u32),
) -> io::Result<()> {
    let object_count = PDF_FIXED_OBJECT_COUNT + PDF_OBJECTS_PER_PAGE * request.page_count;
    let mut out = CountingWriter::new(out);
    let mut offsets = vec![0_u64; object_count as usize + 1];
    out.write_all(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n")?;
    write_catalog_and_pages(&mut out, request.page_count, &mut offsets)?;
    for page in 0..request.page_count {
        check_cancelled_io(is_cancelled)?;
        let image = rasterize_page(request, page, is_cancelled).ok_or_else(cancelled_io)?;
        check_cancelled_io(is_cancelled)?;
        write_pdf_page(&mut out, &image, page, trimmed, &mut offsets, is_cancelled)?;
        report_progress(page + 1);
    }
    check_cancelled_io(is_cancelled)?;
    write_xref_and_trailer(&mut out, &offsets)
}

fn pdf_object_count(page_count: u32) -> Result<u32, ExportError> {
    PDF_OBJECTS_PER_PAGE
        .checked_mul(page_count)
        .and_then(|pages| PDF_FIXED_OBJECT_COUNT.checked_add(pages))
        .ok_or_else(|| message("printer PDF has too many pages for the PDF object table"))
}

fn write_catalog_and_pages(
    out: &mut CountingWriter<impl Write>,
    page_count: u32,
    offsets: &mut [u64],
) -> io::Result<()> {
    start_object(out, offsets, PDF_CATALOG_OBJECT)?;
    writeln!(
        out,
        "<< /Type /Catalog /Pages {PDF_PAGES_OBJECT} 0 R >>\nendobj"
    )?;
    start_object(out, offsets, PDF_PAGES_OBJECT)?;
    write!(out, "<< /Type /Pages /Kids [ ")?;
    for page in 0..page_count {
        write!(out, "{} 0 R ", page_object_number(page))?;
    }
    writeln!(out, "] /Count {page_count} >>\nendobj")
}

fn write_pdf_page(
    out: &mut CountingWriter<impl Write>,
    image: &RasterImage,
    page: u32,
    trimmed: bool,
    offsets: &mut [u64],
    is_cancelled: &impl Fn() -> bool,
) -> io::Result<()> {
    let page_num = page_object_number(page);
    let content_num = page_num + 1;
    let image_num = page_num + 2;
    let length_num = page_num + 3;
    let (x0, width) = pdf_columns(image, trimmed);
    let media_w = width as f32 / RASTER_DPI * PDF_POINTS_PER_INCH;
    let media_h = image.height as f32 / RASTER_DPI * PDF_POINTS_PER_INCH;

    start_object(out, offsets, page_num)?;
    writeln!(
        out,
        "<< /Type /Page /Parent {PDF_PAGES_OBJECT} 0 R /MediaBox [0 0 {media_w:.3} {media_h:.3}] /Resources << /XObject << /Im0 {image_num} 0 R >> >> /Contents {content_num} 0 R >>\nendobj"
    )?;
    write_pdf_content(out, offsets, content_num, media_w, media_h)?;
    let object = PdfImageObject {
        image_num,
        length_num,
        x0,
        width,
    };
    write_pdf_image(out, offsets, image, object, is_cancelled)
}

fn write_pdf_content(
    out: &mut CountingWriter<impl Write>,
    offsets: &mut [u64],
    number: u32,
    width: f32,
    height: f32,
) -> io::Result<()> {
    let content = format!("q {width:.3} 0 0 {height:.3} 0 0 cm /Im0 Do Q");
    start_object(out, offsets, number)?;
    write!(
        out,
        "<< /Length {} >>\nstream\n{content}\nendstream\nendobj\n",
        content.len()
    )
}

#[derive(Clone, Copy)]
struct PdfImageObject {
    image_num: u32,
    length_num: u32,
    x0: usize,
    width: usize,
}

fn write_pdf_image(
    out: &mut CountingWriter<impl Write>,
    offsets: &mut [u64],
    image: &RasterImage,
    object: PdfImageObject,
    is_cancelled: &impl Fn() -> bool,
) -> io::Result<()> {
    start_object(out, offsets, object.image_num)?;
    write!(
        out,
        "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} 0 R >>\nstream\n",
        object.width, image.height, object.length_num
    )?;
    let compressed_start = out.bytes_written();
    write_deflated_rgb(out, image, object.x0, object.width, is_cancelled)?;
    let compressed_len = out.bytes_written() - compressed_start;
    out.write_all(b"\nendstream\nendobj\n")?;
    start_object(out, offsets, object.length_num)?;
    writeln!(out, "{compressed_len}\nendobj")
}

fn write_deflated_rgb(
    out: &mut CountingWriter<impl Write>,
    image: &RasterImage,
    x0: usize,
    width: usize,
    is_cancelled: &impl Fn() -> bool,
) -> io::Result<()> {
    let mut encoder = ZlibEncoder::new(out, Compression::default());
    let mut row = Vec::with_capacity(width * RGB_BYTES_PER_PIXEL);
    for source in image
        .pixels
        .chunks_exact(image.width as usize * RGBA_BYTES_PER_PIXEL)
    {
        check_cancelled_io(is_cancelled)?;
        row.clear();
        let columns = &source[x0 * RGBA_BYTES_PER_PIXEL..(x0 + width) * RGBA_BYTES_PER_PIXEL];
        for pixel in columns.as_chunks::<RGBA_BYTES_PER_PIXEL>().0 {
            row.extend_from_slice(&pixel[..RGB_BYTES_PER_PIXEL]);
        }
        encoder.write_all(&row)?;
    }
    check_cancelled_io(is_cancelled)?;
    encoder.finish().map(|_| ())
}

fn pdf_columns(image: &RasterImage, trimmed: bool) -> (usize, usize) {
    if !trimmed {
        return (0, image.width as usize);
    }
    let x0 = (STRIP_WIDTH_IN * RASTER_DPI).round() as usize;
    let x1 = ((PAPER_WIDTH_IN - STRIP_WIDTH_IN) * RASTER_DPI).round() as usize;
    (x0, x1 - x0)
}

fn page_object_number(page: u32) -> u32 {
    PDF_FIRST_PAGE_OBJECT + PDF_OBJECTS_PER_PAGE * page
}

fn start_object(
    out: &mut CountingWriter<impl Write>,
    offsets: &mut [u64],
    number: u32,
) -> io::Result<()> {
    offsets[number as usize] = out.bytes_written();
    writeln!(out, "{number} 0 obj")
}

fn write_xref_and_trailer(out: &mut CountingWriter<impl Write>, offsets: &[u64]) -> io::Result<()> {
    let xref_offset = out.bytes_written();
    writeln!(out, "xref\n0 {}", offsets.len())?;
    out.write_all(b"0000000000 65535 f\r\n")?;
    for offset in &offsets[1..] {
        writeln!(out, "{offset:010} 00000 n\r")?;
    }
    write!(
        out,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF",
        offsets.len()
    )
}

struct CountingWriter<W> {
    inner: W,
    bytes_written: u64,
}

impl<W> CountingWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            bytes_written: 0,
        }
    }

    fn bytes_written(&self) -> u64 {
        self.bytes_written
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.bytes_written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn check_cancelled(is_cancelled: &impl Fn() -> bool) -> Result<(), ExportError> {
    if is_cancelled() {
        Err(ExportError::Cancelled)
    } else {
        Ok(())
    }
}

fn check_cancelled_io(is_cancelled: &impl Fn() -> bool) -> io::Result<()> {
    if is_cancelled() {
        Err(cancelled_io())
    } else {
        Ok(())
    }
}

fn cancelled_io() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "printer export cancelled")
}

fn png_error(error: impl fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

fn path_error(operation: &str, path: &Path, error: io::Error) -> ExportError {
    if error.kind() == io::ErrorKind::Interrupted {
        ExportError::Cancelled
    } else {
        message(format!(
            "could not {operation} printer export {}: {error}",
            path.display()
        ))
    }
}

fn message(message: impl Into<String>) -> ExportError {
    ExportError::Message(message.into())
}

#[cfg(test)]
#[path = "paper_export_test.rs"]
mod tests;
