//! Pure, egui-free rasterizer for the virtual fanfold "paper" window
//! (`docs/printer-plan.md` T5 visual spec). Turns a [`DotSource`] (the
//! DMP-105's abstract dot-matrix impressions, `coco_core::printer`/`dmp105`)
//! into an RGBA8 raster of period-correct tractor-feed stationery: tractor
//! strips with sprocket holes on both edges, a dotted perforation line
//! separating each strip from the printable body, horizontal page
//! perforations every 11", optional green-bar banding, and anti-aliased ink
//! dots.
//!
//! Every geometry/palette constant below is a **fixed decision** from the T5
//! visual spec, not a hardware fact — this is a rendering choice for a
//! fictional (if period-plausible) sheet of stock, not something to
//! re-derive or adjust. The two constants marked "rendering-scale choice"
//! ([`SPROCKET_RIM_PX`], [`PAGE_PERF_LINE_THICKNESS_PX`]) are this module's
//! own judgment calls for how thick a hairline reads at typical screen DPI —
//! still not spec-given, but distinct from the inch-space geometry the T5
//! spec did fix.
//!
//! Deliberately has no `egui`/`eframe` dependency so it's reusable by a
//! headless PNG-export path (`examples/paper_preview.rs`, and later T6) and
//! by the live `paper_view.rs` window alike.

use coco_core::printer::{X_UNITS_PER_INCH, Y_UNITS_PER_INCH};

// ---------------------------------------------------------------------
// Geometry constants (`docs/printer-plan.md` T5 visual spec — exact values).
// ---------------------------------------------------------------------

/// Overall sheet width, tractor strip to tractor strip.
pub const PAPER_WIDTH_IN: f32 = 9.5;
/// Fanfold page height (the classic 11" x 9.5" tractor-feed form).
pub const PAGE_HEIGHT_IN: f32 = 11.0;
/// Width of each tractor-feed strip (both edges).
pub const STRIP_WIDTH_IN: f32 = 0.5;
/// X position of the left strip's dotted separator perforation.
pub const PERF_LEFT_X_IN: f32 = STRIP_WIDTH_IN;
/// X position of the right strip's dotted separator perforation.
pub const PERF_RIGHT_X_IN: f32 = PAPER_WIDTH_IN - STRIP_WIDTH_IN;
/// Diameter of one dot in a dotted (vertical, strip-separator) perforation line.
pub const PERF_DOT_DIAMETER_IN: f32 = 0.02;
/// Center-to-center spacing of dots along a vertical perforation line.
pub const PERF_DOT_PITCH_IN: f32 = 0.10;
/// Sprocket (pin-feed) hole diameter (4mm).
pub const SPROCKET_HOLE_DIAMETER_IN: f32 = 0.157;
/// Sprocket hole center inset from the paper's outer edge (both strips).
pub const SPROCKET_HOLE_INSET_IN: f32 = 0.25;
/// Vertical center-to-center pitch of sprocket holes.
pub const SPROCKET_HOLE_PITCH_IN: f32 = 0.5;
/// Vertical offset of the first sprocket hole's center below each page-top
/// perforation.
pub const SPROCKET_HOLE_TOP_OFFSET_IN: f32 = 0.25;
/// "Ink" length of one dash in a horizontal page-perforation line.
pub const PAGE_PERF_DASH_ON_IN: f32 = 0.08;
/// Gap length between dashes in a horizontal page-perforation line.
pub const PAGE_PERF_DASH_OFF_IN: f32 = 0.05;
/// X position the paper model's `x = 0` (dot-matrix head's left margin) maps
/// to, i.e. where the printable body begins.
pub const PRINT_AREA_LEFT_IN: f32 = 0.75;
/// Width of the printable body (independent of the wider strip-to-strip
/// span used for green-bar banding).
pub const PRINT_AREA_WIDTH_IN: f32 = 8.0;
/// Diameter of one rendered ink dot: a 1/72" nominal dot-matrix pitch,
/// bled out 15% so adjacent dots visually overlap like real dot-matrix
/// impact print rather than leaving hairline gaps.
pub const DOT_DIAMETER_IN: f32 = (1.0 / 72.0) * 1.15;
/// Alpha of one ink dot's opaque "core" — dots are composited with
/// repeated source-over blending (never deduplicated), so overlapping
/// strikes darken naturally. ~224/255 as a `u8`, i.e. `(0.88 * 255.0).round()`.
pub const DOT_CORE_ALPHA: f32 = 0.88;
/// Height of one green-bar band (3 lines at 6 LPI = 3 * 1/6"); the first
/// band at each page top is non-green (see [`is_green_band`]).
pub const GREEN_BAR_BAND_HEIGHT_IN: f32 = 0.5;
/// Default/reference rasterization resolution.
pub const RASTER_DPI: f32 = 144.0;

/// Rim thickness of a sprocket hole's inner deboss ring, in **pixel** space
/// (not inch space, unlike every geometry constant above) — a rendering-
/// scale choice for how a "1-px-ish" subtle rim reads at typical DPI,
/// distinct from the physical inch-space geometry the T5 spec fixed.
pub const SPROCKET_RIM_PX: f32 = 1.5;

/// Thickness of a horizontal page-perforation dash, in **pixel** space —
/// the same kind of rendering-scale judgment call as [`SPROCKET_RIM_PX`]
/// (the T5 spec fixes the dash on/off *lengths* along x but not a line
/// thickness).
pub const PAGE_PERF_LINE_THICKNESS_PX: f32 = 1.5;

// `PAGE_HEIGHT_IN / SPROCKET_HOLE_PITCH_IN == 22` exactly (11.0 / 0.5), so
// sprocket-hole centers land on the **same absolute-y phase every page**:
// `SPROCKET_HOLE_TOP_OFFSET_IN + k * SPROCKET_HOLE_PITCH_IN` for `k = 0..22`
// measured from each page's own top lands on exactly the same residues,
// continuously, as measuring `k` from the whole roll's y = 0 — i.e. hole
// centers occur at every absolute y with
// `y mod SPROCKET_HOLE_PITCH_IN == SPROCKET_HOLE_TOP_OFFSET_IN`, with no
// per-page phase reset needed (unlike the green-bar bands, below, which DO
// reset). See `twenty_two_sprocket_holes_per_page_same_phase_every_page`
// for the algebraic check.

// ---------------------------------------------------------------------
// Palette (fixed constants — a physical object, not UI chrome; do not
// theme these for dark/light mode).
// ---------------------------------------------------------------------

pub const PAPER_COLOR: [u8; 4] = [0xF4, 0xF1, 0xE4, 0xFF];
pub const PERF_COLOR: [u8; 4] = [0xC9, 0xC4, 0xB2, 0xFF];
pub const INK_COLOR: [u8; 3] = [0x26, 0x26, 0x2E];
pub const GREEN_BAR_COLOR: [u8; 4] = [0xDD, 0xEB, 0xDC, 0xFF];
pub const WINDOW_BG_COLOR: [u8; 4] = [0x3A, 0x3A, 0x40, 0xFF];

/// A padding, in [`Y_UNITS_PER_INCH`] units, applied on both sides of a
/// requested render range before querying [`DotSource::dots_in_range`]:
/// a dot's *center* can sit just outside `[y0, y1]` while its rendered
/// circle (radius [`DOT_DIAMETER_IN`] / 2) still bleeds into the visible
/// band. `y`-units are coarse (1/72") relative to the dot's sub-unit
/// diameter, so this is a generous fixed pad rather than a computed exact
/// radius — the judgment call the T5 spec leaves to this module.
///
/// `pub(crate)` because the paper window's dirty-page invalidation must
/// widen changed ranges by the same bleed before mapping them to page
/// textures: a dot near a page's top edge also renders into the bottom of
/// the previous page's texture.
pub(crate) const DOT_QUERY_PAD_Y_UNITS: u32 = 2;

/// A source of already-printed dot impressions, in the same `(x, y)` unit
/// system as `coco_core::printer::Paper`: `x` in [`X_UNITS_PER_INCH`]
/// units, `y` in [`Y_UNITS_PER_INCH`] units. A local trait over the two
/// foreign paper types (`Paper` itself, and `Dmp105Handle`'s live-printer
/// view of one) so `rasterize` doesn't care which it's drawing.
pub trait DotSource {
    /// Every dot in the inclusive row range `y0..=y1`, as `(x, y)` pairs.
    fn dots_in_range(&self, y0: u32, y1: u32) -> Vec<(u32, u32)>;
}

impl DotSource for coco_core::printer::Paper {
    fn dots_in_range(&self, y0: u32, y1: u32) -> Vec<(u32, u32)> {
        coco_core::printer::Paper::dots_in_range(self, y0, y1)
    }
}

impl DotSource for coco_core::dmp105::Dmp105Handle {
    fn dots_in_range(&self, y0: u32, y1: u32) -> Vec<(u32, u32)> {
        coco_core::dmp105::Dmp105Handle::dots_in_range(self, y0, y1)
    }
}

/// A rasterized slice of paper: RGBA8, row-major, top-left origin.
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl RasterImage {
    fn blank(width: u32, height: u32, fill: [u8; 4]) -> Self {
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..(width as usize * height as usize) {
            pixels.extend_from_slice(&fill);
        }
        Self {
            width,
            height,
            pixels,
        }
    }

    #[inline]
    fn pixel_offset(&self, x: i64, y: i64) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return None;
        }
        Some((y as usize * self.width as usize + x as usize) * 4)
    }

    /// Source-over composite of `rgb` at `alpha` (0.0-1.0, already including
    /// any anti-aliasing coverage) onto the pixel at (`x`, `y`). The canvas
    /// is always fully opaque paper, so the destination alpha channel stays
    /// 255 and only RGB is blended.
    fn composite(&mut self, x: i64, y: i64, rgb: [u8; 3], alpha: f32) {
        let Some(off) = self.pixel_offset(x, y) else {
            return;
        };
        let a = alpha.clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        for (c, &src) in rgb.iter().enumerate() {
            let dst = self.pixels[off + c] as f32;
            self.pixels[off + c] = (dst * (1.0 - a) + src as f32 * a).round() as u8;
        }
        self.pixels[off + 3] = 0xFF;
    }

    /// Hard-set (no blending) a pixel's opaque RGB — used for green-bar
    /// bands, which have no anti-aliased edge in the T5 spec (band
    /// boundaries are a per-scanline step function, unlike the circles/
    /// lines the spec does define a soft edge for).
    fn set_opaque(&mut self, x: i64, y: i64, rgb: [u8; 3]) {
        let Some(off) = self.pixel_offset(x, y) else {
            return;
        };
        self.pixels[off] = rgb[0];
        self.pixels[off + 1] = rgb[1];
        self.pixels[off + 2] = rgb[2];
        self.pixels[off + 3] = 0xFF;
    }

    /// Fill a circle of `radius_px` centered at (`cx_px`, `cy_px`) with
    /// `rgb` at `base_alpha`, anti-aliased over a ~1px edge band per the T5
    /// spec's formula: `coverage = clamp(radius_px + 0.5 - distance_px, 0, 1)`.
    fn fill_circle(
        &mut self,
        cx_px: f32,
        cy_px: f32,
        radius_px: f32,
        rgb: [u8; 3],
        base_alpha: f32,
    ) {
        let x_min = (cx_px - radius_px - 1.0).floor() as i64;
        let x_max = (cx_px + radius_px + 1.0).ceil() as i64;
        let y_min = (cy_px - radius_px - 1.0).floor() as i64;
        let y_max = (cy_px + radius_px + 1.0).ceil() as i64;
        for py in y_min..=y_max {
            for px in x_min..=x_max {
                let dx = px as f32 + 0.5 - cx_px;
                let dy = py as f32 + 0.5 - cy_px;
                let dist = (dx * dx + dy * dy).sqrt();
                let coverage = (radius_px + 0.5 - dist).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    self.composite(px, py, rgb, coverage * base_alpha);
                }
            }
        }
    }

    /// Draw a thin ring (a circle's rim only) at `radius_px` with `rim_px`
    /// thickness, anti-aliased the same way as [`Self::fill_circle`] but
    /// measuring distance from the ideal rim circle rather than from the
    /// center outward.
    fn draw_ring(
        &mut self,
        cx_px: f32,
        cy_px: f32,
        radius_px: f32,
        rim_px: f32,
        rgb: [u8; 3],
        base_alpha: f32,
    ) {
        let half = rim_px / 2.0;
        let x_min = (cx_px - radius_px - half - 1.0).floor() as i64;
        let x_max = (cx_px + radius_px + half + 1.0).ceil() as i64;
        let y_min = (cy_px - radius_px - half - 1.0).floor() as i64;
        let y_max = (cy_px + radius_px + half + 1.0).ceil() as i64;
        for py in y_min..=y_max {
            for px in x_min..=x_max {
                let dx = px as f32 + 0.5 - cx_px;
                let dy = py as f32 + 0.5 - cy_px;
                let dist = (dx * dx + dy * dy).sqrt();
                let coverage = (half + 0.5 - (dist - radius_px).abs()).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    self.composite(px, py, rgb, coverage * base_alpha);
                }
            }
        }
    }

    /// Draw a horizontal hairline of `thickness_px` at `cy_px`, anti-aliased
    /// vertically the same way as a ring's rim, for one pixel column.
    fn blend_hline_pixel(
        &mut self,
        x_px: i64,
        cy_px: f32,
        thickness_px: f32,
        rgb: [u8; 3],
        base_alpha: f32,
    ) {
        let half = thickness_px / 2.0;
        let y_min = (cy_px - half - 1.0).floor() as i64;
        let y_max = (cy_px + half + 1.0).ceil() as i64;
        for py in y_min..=y_max {
            let dy = (py as f32 + 0.5 - cy_px).abs();
            let coverage = (half + 0.5 - dy).clamp(0.0, 1.0);
            if coverage > 0.0 {
                self.composite(x_px, py, rgb, coverage * base_alpha);
            }
        }
    }
}

/// Whether the green-bar band containing `y_in` (absolute roll-inches) is
/// green: band index is computed relative to that y's *own page top*, so
/// every page starts fresh with a non-green band (band 0), per the T5 spec.
fn is_green_band(y_in: f32) -> bool {
    let page_top_in = (y_in / PAGE_HEIGHT_IN).floor() * PAGE_HEIGHT_IN;
    let band_index = ((y_in - page_top_in) / GREEN_BAR_BAND_HEIGHT_IN).floor() as i64;
    band_index.rem_euclid(2) == 1
}

/// Rasterize a `height_in`-tall slice of the paper roll starting at absolute
/// roll-y `top_in` (inches), at `dpi`, optionally with green-bar banding.
/// See the module doc comment for the draw order and every constant used.
pub fn rasterize<D: DotSource>(
    dots: &D,
    top_in: f32,
    height_in: f32,
    dpi: f32,
    green_bar: bool,
) -> RasterImage {
    // Sanity check on the T5 spec's own geometry constants: the printable
    // body must fit within the tractor-strip-to-tractor-strip span. Every
    // operand is itself a fixed constant, so clippy sees this as always
    // true and would otherwise flag it as foldable — it's still worth
    // stating explicitly as a guard against a future edit to one constant
    // silently breaking that relationship.
    #[allow(clippy::assertions_on_constants)]
    {
        debug_assert!(PRINT_AREA_LEFT_IN + PRINT_AREA_WIDTH_IN <= PAPER_WIDTH_IN - STRIP_WIDTH_IN);
    }

    let width_px = (PAPER_WIDTH_IN * dpi).round() as u32;
    let height_px = (height_in * dpi).round() as u32;
    let mut image = RasterImage::blank(width_px, height_px, PAPER_COLOR);

    // 1. Paper base color: already the blank fill above.

    // 2. Green-bar bands, clipped to the tractor-strip-to-tractor-strip body
    // (not the narrower print line) — a hard-edged per-scanline step, no AA.
    if green_bar {
        let strip_x0 = (STRIP_WIDTH_IN * dpi).round() as i64;
        let strip_x1 = ((PAPER_WIDTH_IN - STRIP_WIDTH_IN) * dpi).round() as i64;
        for py in 0..height_px as i64 {
            let y_in = top_in + (py as f32 + 0.5) / dpi;
            if is_green_band(y_in) {
                for px in strip_x0..strip_x1 {
                    image.set_opaque(
                        px,
                        py,
                        [GREEN_BAR_COLOR[0], GREEN_BAR_COLOR[1], GREEN_BAR_COLOR[2]],
                    );
                }
            }
        }
    }

    // 3. Vertical dotted perforation lines (both x positions, full height),
    // phased continuously from the roll's y = 0 (not reset per page).
    let perf_radius_in = PERF_DOT_DIAMETER_IN / 2.0;
    let perf_radius_px = perf_radius_in * dpi;
    for &x_in in &[PERF_LEFT_X_IN, PERF_RIGHT_X_IN] {
        let cx_px = x_in * dpi;
        let k_min = ((top_in - perf_radius_in) / PERF_DOT_PITCH_IN)
            .floor()
            .max(0.0) as i64;
        let k_max = ((top_in + height_in + perf_radius_in) / PERF_DOT_PITCH_IN).ceil() as i64;
        for k in k_min..=k_max {
            let y_in = k as f32 * PERF_DOT_PITCH_IN;
            let cy_px = (y_in - top_in) * dpi;
            let perf_rgb = [PERF_COLOR[0], PERF_COLOR[1], PERF_COLOR[2]];
            image.fill_circle(cx_px, cy_px, perf_radius_px, perf_rgb, 1.0);
        }
    }

    // 4. Horizontal dashed page-perforation lines, full width, at every
    // POSITIVE integer multiple of PAGE_HEIGHT_IN (not at y=0 — that's the
    // roll's start, not a perforation between two pages).
    {
        let line_half_in = (PAGE_PERF_LINE_THICKNESS_PX / dpi) / 2.0;
        let n_min = (((top_in - line_half_in) / PAGE_HEIGHT_IN).floor() as i64).max(1);
        let n_max = ((top_in + height_in + line_half_in) / PAGE_HEIGHT_IN).ceil() as i64;
        let dash_period_in = PAGE_PERF_DASH_ON_IN + PAGE_PERF_DASH_OFF_IN;
        let perf_rgb = [PERF_COLOR[0], PERF_COLOR[1], PERF_COLOR[2]];
        for n in n_min..=n_max {
            let y_in = n as f32 * PAGE_HEIGHT_IN;
            let cy_px = (y_in - top_in) * dpi;
            if cy_px < -PAGE_PERF_LINE_THICKNESS_PX
                || cy_px > height_px as f32 + PAGE_PERF_LINE_THICKNESS_PX
            {
                continue;
            }
            for px in 0..width_px as i64 {
                let x_in = (px as f32 + 0.5) / dpi;
                let phase = x_in.rem_euclid(dash_period_in);
                if phase < PAGE_PERF_DASH_ON_IN {
                    image.blend_hline_pixel(px, cy_px, PAGE_PERF_LINE_THICKNESS_PX, perf_rgb, 1.0);
                }
            }
        }
    }

    // 5. Sprocket holes (both strips): fill WINDOW_BG_COLOR, then the
    // PERF_COLOR rim. Hole phase is continuous from the roll's y = 0 (see
    // the `SPROCKET_HOLES_PER_PAGE` doc comment above).
    {
        let hole_radius_in = SPROCKET_HOLE_DIAMETER_IN / 2.0;
        let hole_radius_px = hole_radius_in * dpi;
        let bg_rgb = [WINDOW_BG_COLOR[0], WINDOW_BG_COLOR[1], WINDOW_BG_COLOR[2]];
        let perf_rgb = [PERF_COLOR[0], PERF_COLOR[1], PERF_COLOR[2]];
        for &x_in in &[
            SPROCKET_HOLE_INSET_IN,
            PAPER_WIDTH_IN - SPROCKET_HOLE_INSET_IN,
        ] {
            let cx_px = x_in * dpi;
            let k_min = (((top_in - hole_radius_in - SPROCKET_HOLE_TOP_OFFSET_IN)
                / SPROCKET_HOLE_PITCH_IN)
                .floor()
                .max(0.0)) as i64;
            let k_max = ((top_in + height_in + hole_radius_in - SPROCKET_HOLE_TOP_OFFSET_IN)
                / SPROCKET_HOLE_PITCH_IN)
                .ceil() as i64;
            for k in k_min..=k_max {
                let y_in = SPROCKET_HOLE_TOP_OFFSET_IN + k as f32 * SPROCKET_HOLE_PITCH_IN;
                if y_in < 0.0 {
                    continue;
                }
                let cy_px = (y_in - top_in) * dpi;
                image.fill_circle(cx_px, cy_px, hole_radius_px, bg_rgb, 1.0);
                image.draw_ring(cx_px, cy_px, hole_radius_px, SPROCKET_RIM_PX, perf_rgb, 1.0);
            }
        }
    }

    // 6. Ink dots (topmost): alpha-composited via repeated source-over
    // blending, never manually deduplicated, so overlapping strikes darken
    // naturally.
    {
        let dot_radius_in = DOT_DIAMETER_IN / 2.0;
        let dot_radius_px = dot_radius_in * dpi;
        let y0_in = top_in;
        let y1_in = top_in + height_in;
        // Pad the y-unit query range: a dot's center can sit just outside
        // [y0, y1] while its rendered circle still bleeds into view (see
        // DOT_QUERY_PAD_Y_UNITS's doc comment).
        let y0_units = ((y0_in * Y_UNITS_PER_INCH as f32).floor() as i64
            - DOT_QUERY_PAD_Y_UNITS as i64)
            .max(0) as u32;
        let y1_units = (y1_in * Y_UNITS_PER_INCH as f32).ceil() as u32 + DOT_QUERY_PAD_Y_UNITS;
        for (x_units, y_units) in dots.dots_in_range(y0_units, y1_units) {
            let x_in = PRINT_AREA_LEFT_IN + x_units as f32 / X_UNITS_PER_INCH as f32;
            let y_in = y_units as f32 / Y_UNITS_PER_INCH as f32;
            let cx_px = x_in * dpi;
            let cy_px = (y_in - top_in) * dpi;
            image.fill_circle(cx_px, cy_px, dot_radius_px, INK_COLOR, DOT_CORE_ALPHA);
        }
    }

    image
}

#[cfg(test)]
#[path = "paper_render_test.rs"]
mod tests;
