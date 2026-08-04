//! PNG/PDF export for the virtual fanfold paper:
//! pure byte-producing functions consumed by `paper_view.rs`'s save-dialog
//! wiring. Kept separate from the egui-free `paper_render.rs` rasterizer
//! (T5) so that module stays free of an `image`/PDF dependency, per its own
//! doc comment ("reusable by a headless PNG-export path... and by the live
//! `paper_view.rs` window alike").
//!
//! Dependency choice: PNG export uses the `image` crate (MIT OR Apache-2.0;
//! already a dev-dependency for `examples/paper_preview.rs`, now promoted to
//! a regular one, `default-features = false, features = ["png"]`, to keep
//! the build light). For PDF, rather than pulling in `printpdf` (a full
//! PDF-object-model library with its own `lopdf`/`time`/etc. dependency
//! tree) this hand-rolls a minimal single-purpose PDF: one page per fanfold
//! page, each a `FlateDecode`-compressed `DeviceRGB` image XObject painted
//! over the whole media box — a well-understood, small format that doesn't
//! need a general parser/writer. `flate2` (MIT OR Apache-2.0) does the
//! deflate; it's already resolved in the workspace's dependency graph
//! transitively (`image`'s `png` feature -> `png` -> `flate2`), so promoting
//! it to a direct dependency here adds no new crate to the tree.

use std::io::{self, Write};
use std::path::Path;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::paper_render::{PAPER_WIDTH_IN, RasterImage, STRIP_WIDTH_IN};

/// PDF user-space units per inch: a PDF fact (ISO 32000-1 8.3.2.3 fixes 1
/// default user-space unit = 1/72"), not a T6 rendering choice.
const PDF_POINTS_PER_INCH: f32 = 72.0;

/// Save one raster as a PNG file.
pub fn save_png(img: &RasterImage, path: &Path) -> Result<(), String> {
    let rgba = image::RgbaImage::from_raw(img.width, img.height, img.pixels.clone())
        .ok_or_else(|| "internal error: raster buffer size mismatch".to_string())?;
    rgba.save_with_format(path, image::ImageFormat::Png)
        .map_err(|e| format!("could not save {}: {e}", path.display()))
}

/// Crop a rasterized page down to just the tractor-strip-to-tractor-strip
/// printable body, both strips removed: `PAPER_WIDTH_IN - 2 *
/// STRIP_WIDTH_IN == 8.5`in exactly (9.5" - 0.5" - 0.5"), matching US
/// Letter width — the "trimmed" PDF variant's whole reason for existing.
pub fn crop_to_trimmed_width(img: &RasterImage, dpi: f32) -> RasterImage {
    let x0 = (STRIP_WIDTH_IN * dpi).round() as u32;
    let x1 = ((PAPER_WIDTH_IN - STRIP_WIDTH_IN) * dpi).round() as u32;
    crop_columns(img, x0, x1)
}

/// Crop a raster to the column range `[x0_px, x1_px)`, full height.
fn crop_columns(img: &RasterImage, x0_px: u32, x1_px: u32) -> RasterImage {
    let width = x1_px.saturating_sub(x0_px);
    let mut pixels = Vec::with_capacity(width as usize * img.height as usize * 4);
    for y in 0..img.height {
        let row_start = (y as usize * img.width as usize + x0_px as usize) * 4;
        let row_end = row_start + width as usize * 4;
        pixels.extend_from_slice(&img.pixels[row_start..row_end]);
    }
    RasterImage {
        width,
        height: img.height,
        pixels,
    }
}

/// Save one PDF with one page per element of `pages`, each rendered at
/// `dpi`. See the module doc comment for the file format this hand-rolls.
pub fn save_pdf(pages: &[RasterImage], dpi: f32, path: &Path) -> Result<(), String> {
    let file = std::fs::File::create(path)
        .map_err(|e| format!("could not create {}: {e}", path.display()))?;
    let mut writer = io::BufWriter::new(file);
    write_pdf(pages, dpi, &mut writer)
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// Drop the alpha channel. [`RasterImage`] is always fully opaque — every
/// pixel-writing helper in `paper_render.rs` (`blank`, `composite`,
/// `set_opaque`) sets alpha to `0xFF` — so this loses no information.
fn rgba_to_rgb(img: &RasterImage) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(img.width as usize * img.height as usize * 3);
    for px in img.pixels.chunks_exact(4) {
        rgb.extend_from_slice(&px[..3]);
    }
    rgb
}

/// Zlib-wrap (RFC 1950 — the format PDF's `/Filter /FlateDecode` expects,
/// ISO 32000-1 7.4.4) a deflate-compressed copy of `data`.
fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(data)
        .expect("writing to an in-memory Vec<u8> never fails");
    encoder
        .finish()
        .expect("finishing an in-memory Vec<u8> encoder never fails")
}

/// Build the minimal PDF body: object numbering is fixed by construction
/// order — obj 1 Catalog, obj 2 Pages, then per page `i` (0-based) obj
/// `3+3*i` Page, `4+3*i` Contents, `5+3*i` Image XObject.
fn write_pdf<W: Write>(pages: &[RasterImage], dpi: f32, out: &mut W) -> io::Result<()> {
    // obj 1 (Catalog) and obj 2 (Pages) are filled in once every page's
    // object numbers are known.
    let mut objects: Vec<Vec<u8>> = vec![Vec::new(), Vec::new()];
    let mut page_obj_nums = Vec::with_capacity(pages.len());

    for page in pages {
        let page_num = objects.len() as u32 + 1;
        let contents_num = page_num + 1;
        let image_num = page_num + 2;
        page_obj_nums.push(page_num);

        let media_w_pt = (page.width as f32 / dpi) * PDF_POINTS_PER_INCH;
        let media_h_pt = (page.height as f32 / dpi) * PDF_POINTS_PER_INCH;

        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {media_w_pt:.3} {media_h_pt:.3}] \
                 /Resources << /XObject << /Im0 {image_num} 0 R >> >> \
                 /Contents {contents_num} 0 R >>"
            )
            .into_bytes(),
        );

        let content_text = format!("q {media_w_pt:.3} 0 0 {media_h_pt:.3} 0 0 cm /Im0 Do Q");
        let mut content_body =
            format!("<< /Length {} >>\nstream\n", content_text.len()).into_bytes();
        content_body.extend_from_slice(content_text.as_bytes());
        content_body.extend_from_slice(b"\nendstream");
        objects.push(content_body);

        let compressed = deflate(&rgba_to_rgb(page));
        let mut image_body = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB \
             /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            page.width,
            page.height,
            compressed.len()
        )
        .into_bytes();
        image_body.extend_from_slice(&compressed);
        image_body.extend_from_slice(b"\nendstream");
        objects.push(image_body);
    }

    let kids: String = page_obj_nums.iter().map(|n| format!("{n} 0 R ")).collect();
    objects[1] = format!("<< /Type /Pages /Kids [ {kids}] /Count {} >>", pages.len()).into_bytes();
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();

    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (idx, body) in objects.iter().enumerate() {
        offsets.push(buf.len());
        let num = idx + 1;
        buf.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        buf.extend_from_slice(body);
        buf.extend_from_slice(b"\nendobj\n");
    }
    let xref_offset = buf.len();
    buf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    buf.extend_from_slice(b"0000000000 65535 f\r\n");
    for off in &offsets {
        buf.extend_from_slice(format!("{off:010} 00000 n\r\n").as_bytes());
    }
    buf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF",
            objects.len() + 1
        )
        .as_bytes(),
    );

    out.write_all(&buf)
}

#[cfg(test)]
#[path = "paper_export_test.rs"]
mod tests;
