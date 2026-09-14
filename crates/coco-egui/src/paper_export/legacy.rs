//! Original whole-export buffering retained for before-and-after benchmarks.

use std::io::{self, Write};
use std::path::Path;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::paper_render::{PAPER_WIDTH_IN, RasterImage, STRIP_WIDTH_IN};

const PDF_POINTS_PER_INCH: f32 = 72.0;
const RGBA_BYTES_PER_PIXEL: usize = 4;

pub fn save_png(image: &RasterImage, path: &Path) -> Result<(), String> {
    let rgba = image::RgbaImage::from_raw(image.width, image.height, image.pixels.clone())
        .ok_or_else(|| "internal error: raster buffer size mismatch".to_string())?;
    rgba.save_with_format(path, image::ImageFormat::Png)
        .map_err(|error| format!("could not save {}: {error}", path.display()))
}

pub fn crop_to_trimmed_width(image: &RasterImage, dpi: f32) -> RasterImage {
    let x0 = (STRIP_WIDTH_IN * dpi).round() as usize;
    let x1 = ((PAPER_WIDTH_IN - STRIP_WIDTH_IN) * dpi).round() as usize;
    let width = x1 - x0;
    let mut pixels = Vec::with_capacity(width * image.height as usize * RGBA_BYTES_PER_PIXEL);
    for row in image
        .pixels
        .chunks_exact(image.width as usize * RGBA_BYTES_PER_PIXEL)
    {
        pixels.extend_from_slice(&row[x0 * RGBA_BYTES_PER_PIXEL..x1 * RGBA_BYTES_PER_PIXEL]);
    }
    RasterImage {
        width: width as u32,
        height: image.height,
        pixels,
    }
}

pub fn save_pdf(pages: &[RasterImage], dpi: f32, path: &Path) -> Result<(), String> {
    let file = std::fs::File::create(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    let mut writer = io::BufWriter::new(file);
    write_pdf(pages, dpi, &mut writer)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

pub(super) fn write_pdf<W: Write>(pages: &[RasterImage], dpi: f32, out: &mut W) -> io::Result<()> {
    let mut objects: Vec<Vec<u8>> = vec![Vec::new(), Vec::new()];
    let mut page_numbers = Vec::with_capacity(pages.len());
    for page in pages {
        push_page_objects(&mut objects, &mut page_numbers, page, dpi);
    }
    let kids: String = page_numbers
        .iter()
        .map(|number| format!("{number} 0 R "))
        .collect();
    objects[1] = format!("<< /Type /Pages /Kids [ {kids}] /Count {} >>", pages.len()).into();
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
    write_objects(&objects, out)
}

fn push_page_objects(
    objects: &mut Vec<Vec<u8>>,
    page_numbers: &mut Vec<u32>,
    page: &RasterImage,
    dpi: f32,
) {
    let page_num = objects.len() as u32 + 1;
    let content_num = page_num + 1;
    let image_num = page_num + 2;
    page_numbers.push(page_num);
    let width = page.width as f32 / dpi * PDF_POINTS_PER_INCH;
    let height = page.height as f32 / dpi * PDF_POINTS_PER_INCH;
    objects.push(
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width:.3} {height:.3}] /Resources << /XObject << /Im0 {image_num} 0 R >> >> /Contents {content_num} 0 R >>"
        )
        .into(),
    );
    let content = format!("q {width:.3} 0 0 {height:.3} 0 0 cm /Im0 Do Q");
    objects.push(
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        )
        .into(),
    );
    objects.push(image_object(page));
}

fn image_object(image: &RasterImage) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(image.width as usize * image.height as usize * 3);
    for pixel in image.pixels.chunks_exact(RGBA_BYTES_PER_PIXEL) {
        rgb.extend_from_slice(&pixel[..3]);
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&rgb).expect("Vec writes cannot fail");
    let compressed = encoder.finish().expect("Vec writes cannot fail");
    let mut body = format!(
        "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
        image.width,
        image.height,
        compressed.len()
    )
    .into_bytes();
    body.extend_from_slice(&compressed);
    body.extend_from_slice(b"\nendstream");
    body
}

fn write_objects(objects: &[Vec<u8>], out: &mut impl Write) -> io::Result<()> {
    let mut buffer = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, body) in objects.iter().enumerate() {
        offsets.push(buffer.len());
        buffer.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        buffer.extend_from_slice(body);
        buffer.extend_from_slice(b"\nendobj\n");
    }
    let xref = buffer.len();
    buffer.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    buffer.extend_from_slice(b"0000000000 65535 f\r\n");
    for offset in offsets {
        buffer.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
    }
    buffer.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out.write_all(&buffer)
}
