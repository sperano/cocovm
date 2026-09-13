use super::*;

/// A [`DotSource`] with a fixed set of dots, for tests.
struct FixedDots(Vec<(u32, u32)>);

impl DotSource for FixedDots {
    fn dots_in_range(&self, y0: u32, y1: u32) -> Vec<(u32, u32)> {
        self.0
            .iter()
            .copied()
            .filter(|&(_, y)| y >= y0 && y <= y1)
            .collect()
    }
}

fn empty() -> FixedDots {
    FixedDots(Vec::new())
}

/// Read one pixel's RGBA as an array, panicking if out of bounds (test
/// helper only).
fn pixel(img: &RasterImage, x: u32, y: u32) -> [u8; 4] {
    let off = (y as usize * img.width as usize + x as usize) * 4;
    [
        img.pixels[off],
        img.pixels[off + 1],
        img.pixels[off + 2],
        img.pixels[off + 3],
    ]
}

#[test]
fn output_dimensions_match_dpi_and_height() {
    let dpi = 100.0;
    let height_in = 2.0;
    let img = rasterize(&empty(), 0.0, height_in, dpi, false);
    assert_eq!(img.width, (PAPER_WIDTH_IN * dpi).round() as u32);
    assert_eq!(img.height, (height_in * dpi).round() as u32);
    assert_eq!(
        img.pixels.len(),
        img.width as usize * img.height as usize * 4
    );
}

/// Within the print-area x-range and a y-range confined to a single (non-green) band, blank
/// paper must render pure [`PAPER_COLOR`] everywhere — nothing else can paint ink there.
#[test]
fn blank_paper_in_print_area_is_pure_paper_color_no_ink() {
    let dpi = 100.0;
    // Comfortably inside band 0 (non-green), well clear of any page-perforation line.
    let height_in = 0.2;
    let img = rasterize(&empty(), 0.0, height_in, dpi, true);
    let x0 = (PRINT_AREA_LEFT_IN * dpi).round() as u32;
    let x1 = ((PRINT_AREA_LEFT_IN + PRINT_AREA_WIDTH_IN) * dpi).round() as u32;
    for y in 0..img.height {
        for x in x0..x1 {
            assert_eq!(
                pixel(&img, x, y),
                PAPER_COLOR,
                "unexpected non-paper pixel at ({x},{y})"
            );
        }
    }
}

/// A single marked dot produces an ink-colored pixel at the expected mapped position: pick
/// coordinates landing exactly on a pixel center so the AA coverage is fully saturated (1.0)
/// and reproducible.
#[test]
fn single_dot_marks_ink_at_the_mapped_pixel() {
    let dpi = RASTER_DPI;
    let x_units = 360; // 0.1" into the print area
    let y_units = Y_UNITS_PER_INCH; // 1.0" down the roll
    let dots = FixedDots(vec![(x_units, y_units)]);
    let img = rasterize(&dots, 0.0, 2.0, dpi, false);

    let x_in = PRINT_AREA_LEFT_IN + x_units as f32 / X_UNITS_PER_INCH as f32;
    let y_in = y_units_to_roll_in(y_units);
    let px = (x_in * dpi).floor() as u32;
    let py = (y_in * dpi).floor() as u32;

    // Full coverage at the dot's own center pixel: composite(paper, ink, DOT_CORE_ALPHA) exactly.
    let a = DOT_CORE_ALPHA;
    let expected: [u8; 3] = std::array::from_fn(|c| {
        (PAPER_COLOR[c] as f32 * (1.0 - a) + INK_COLOR[c] as f32 * a).round() as u8
    });
    let got = pixel(&img, px, py);
    assert_eq!([got[0], got[1], got[2]], expected);
}

/// The head's first row lands a top margin below the roll's leading edge, not on it: the
/// rows above `PRINT_AREA_TOP_IN` stay pure paper even with a dot at model `y = 0`.
#[test]
fn first_print_row_sits_below_the_top_margin() {
    let dpi = RASTER_DPI;
    let dots = FixedDots(vec![(0, 0)]);
    let img = rasterize(&dots, 0.0, 1.0, dpi, false);
    let x0 = (PRINT_AREA_LEFT_IN * dpi).round() as u32;
    let x1 = ((PRINT_AREA_LEFT_IN + PRINT_AREA_WIDTH_IN) * dpi).round() as u32;
    let margin_rows = ((PRINT_AREA_TOP_IN - DOT_DIAMETER_IN / 2.0) * dpi).floor() as u32;
    assert!(margin_rows > 0);
    for y in 0..margin_rows {
        for x in x0..x1 {
            assert_eq!(
                pixel(&img, x, y),
                PAPER_COLOR,
                "ink above the top margin at ({x},{y})"
            );
        }
    }
}

#[test]
fn page_of_units_counts_pages_from_the_offset_first_row() {
    assert_eq!(page_of_units(0), 0);
    let first_row_on_page_2 = roll_in_to_y_units(PAGE_HEIGHT_IN).ceil() as u32;
    assert_eq!(page_of_units(first_row_on_page_2 - 1), 0);
    assert_eq!(page_of_units(first_row_on_page_2), 1);
}

/// At 6 LPI, 66 lines span exactly one 11" page, and the top-of-form offset phases them so
/// every page perforation falls centred in the gap between two lines, never through a row.
#[test]
fn page_perforation_is_centred_between_text_lines() {
    let line_units = (TEXT_LINE_PITCH_IN * Y_UNITS_PER_INCH as f32).round() as u32;
    let lines_per_page = (PAGE_HEIGHT_IN / TEXT_LINE_PITCH_IN).round() as u32;
    let last_line_on_page_1 = lines_per_page - 1 - TOP_MARGIN_LINES as u32;
    let last_row_in = y_units_to_roll_in(last_line_on_page_1 * line_units) + TEXT_CELL_HEIGHT_IN;
    let next_row_in = y_units_to_roll_in((last_line_on_page_1 + 1) * line_units);
    let clearance_above = PAGE_HEIGHT_IN - last_row_in;
    let clearance_below = next_row_in - PAGE_HEIGHT_IN;
    assert!(clearance_above > 0.0 && clearance_below > 0.0);
    assert!((clearance_above - clearance_below).abs() < 1e-5);
}

#[test]
fn twenty_two_sprocket_holes_per_page_same_phase_every_page() {
    // Algebraic claim from the module doc comment.
    assert_eq!(PAGE_HEIGHT_IN / SPROCKET_HOLE_PITCH_IN, 22.0);

    // Hole k=22 (page 2's first hole) must sit at exactly SPROCKET_HOLE_TOP_OFFSET_IN
    // past page 2's own top.
    let page2_top = PAGE_HEIGHT_IN;
    let hole_23_y = SPROCKET_HOLE_TOP_OFFSET_IN + 22.0 * SPROCKET_HOLE_PITCH_IN;
    assert_eq!(hole_23_y - page2_top, SPROCKET_HOLE_TOP_OFFSET_IN);
}

#[test]
fn sprocket_holes_render_at_expected_pixel_y() {
    let dpi = 100.0;
    let img = rasterize(&empty(), 0.0, 1.0, dpi, false);
    let cx = (SPROCKET_HOLE_INSET_IN * dpi).round() as u32;
    let cy = (SPROCKET_HOLE_TOP_OFFSET_IN * dpi).round() as u32;
    assert_eq!(
        pixel(&img, cx, cy),
        WINDOW_BG_COLOR,
        "expected the first sprocket hole's fill at its center pixel"
    );
}

#[test]
fn green_bar_band_alternates_and_resets_at_each_page_top() {
    assert!(!is_green_band(0.0), "band 0 (page top) must be non-green");
    assert!(
        !is_green_band(GREEN_BAR_BAND_HEIGHT_IN - 0.01),
        "still band 0"
    );
    assert!(
        is_green_band(GREEN_BAR_BAND_HEIGHT_IN + 0.01),
        "band 1 must be green"
    );
    assert!(
        is_green_band(2.0 * GREEN_BAR_BAND_HEIGHT_IN - 0.01),
        "still band 1"
    );
    assert!(
        !is_green_band(2.0 * GREEN_BAR_BAND_HEIGHT_IN + 0.01),
        "band 2 must be non-green again"
    );

    // Crossing into the next page resets the phase: the first band after the page-top boundary
    // must always be non-green.
    assert!(
        !is_green_band(PAGE_HEIGHT_IN + 0.01),
        "band 0 of the next page must be non-green"
    );
}

/// Perforation dots/dashes are anti-aliased, so a pixel at a rounded integer column/row won't
/// necessarily land on the exact sub-pixel center — checking "not pure paper color" isolates
/// "some perforation ink landed here" without being brittle to rounding.
#[test]
fn vertical_perforation_lines_land_at_mapped_columns() {
    let dpi = 100.0;
    let img = rasterize(&empty(), 0.0, 1.0, dpi, false);
    let left_x = (PERF_LEFT_X_IN * dpi).round() as u32;
    let right_x = (PERF_RIGHT_X_IN * dpi).round() as u32;
    let dot_y = (PERF_DOT_PITCH_IN * dpi).round() as u32; // second dot, k=1

    assert_ne!(
        pixel(&img, left_x, dot_y),
        PAPER_COLOR,
        "left perforation dot missing"
    );
    assert_ne!(
        pixel(&img, right_x, dot_y),
        PAPER_COLOR,
        "right perforation dot missing"
    );
}

#[test]
fn page_perforation_dash_appears_at_page_height_but_not_at_zero() {
    let dpi = 100.0;
    // A body x-range clear of the vertical perforation lines and sprocket holes, so only a
    // page-perforation dash can paint here.
    let x0 = (1.0f32 * dpi).round() as u32;
    let x1 = (8.5f32 * dpi).round() as u32;

    // Render straddling the first page boundary.
    let img = rasterize(&empty(), PAGE_HEIGHT_IN - 0.5, 1.0, dpi, false);
    let row_at_boundary = ((PAGE_HEIGHT_IN - (PAGE_HEIGHT_IN - 0.5)) * dpi).round() as u32;
    let has_dash_at_boundary = (x0..x1).any(|x| pixel(&img, x, row_at_boundary) != PAPER_COLOR);
    assert!(
        has_dash_at_boundary,
        "expected a page-perforation dash at y = PAGE_HEIGHT_IN"
    );

    // A render range covering only y=0 (the roll's start, not a perforation) must show no dash.
    let img0 = rasterize(&empty(), 0.0, 0.05, dpi, false);
    let has_dash_at_zero = (x0..x1).any(|x| pixel(&img0, x, 0) != PAPER_COLOR);
    assert!(
        !has_dash_at_zero,
        "y=0 is the roll start, not a page perforation"
    );
}
