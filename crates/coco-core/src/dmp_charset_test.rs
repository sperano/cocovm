use super::*;

fn dots(glyph: BlockGlyph) -> u32 {
    glyph.iter().map(|bits| bits.count_ones()).sum()
}

#[test]
fn european_table_is_dense_and_bounded() {
    assert_eq!(european_symbol(0xA0), Some('´'));
    assert_eq!(european_symbol(0xA1), Some('à'));
    assert_eq!(european_symbol(0xB9), Some('ß'));
    assert_eq!(european_symbol(0xBF), Some('ƒ'));
    assert_eq!(european_symbol(0x9F), None);
    assert_eq!(european_symbol(0xC0), None);
    let unique: std::collections::HashSet<char> = EUROPEAN.iter().copied().collect();
    assert_eq!(unique.len(), EUROPEAN.len());
}

#[test]
fn extended_table_follows_the_printout() {
    assert_eq!(extended_symbol(0xC0), Some('â'));
    assert_eq!(extended_symbol(0xC8), Some('á'));
    assert_eq!(extended_symbol(0xD2), Some('Å'));
    assert_eq!(extended_symbol(0xD8), Some('Á'));
    assert_eq!(extended_symbol(0xDF), Some('Â'));
    assert_eq!(extended_symbol(0xBF), None);
    assert_eq!(extended_symbol(0xE0), None);
}

#[test]
fn block_range_has_thirty_defined_codes() {
    assert_eq!(block_glyph(0xE0), Some([0u8; BLOCK_SIZE]));
    assert!(block_glyph(0xFE).is_some());
    assert_eq!(block_glyph(0xFF), None);
    assert_eq!(block_glyph(0xDF), None);
    let glyphs: Vec<BlockGlyph> = (0xE1..=0xFE).map(|c| block_glyph(c).unwrap()).collect();
    let unique: std::collections::HashSet<BlockGlyph> = glyphs.iter().copied().collect();
    assert_eq!(unique.len(), 30);
    assert!(glyphs.iter().all(|g| dots(*g) > 0));
}

#[test]
fn quadrants_compose_by_bit() {
    let top_left = block_glyph(0xE1).unwrap();
    let top_right = block_glyph(0xE2).unwrap();
    let bottom_left = block_glyph(0xE3).unwrap();
    assert_eq!(dots(top_left), 9);
    assert_eq!(top_left, [TOP_ROWS, TOP_ROWS, TOP_ROWS, 0, 0, 0]);
    assert_eq!(
        bottom_left,
        [BOTTOM_ROWS, BOTTOM_ROWS, BOTTOM_ROWS, 0, 0, 0]
    );
    let top_row: BlockGlyph = std::array::from_fn(|i| top_left[i] | top_right[i]);
    assert_eq!(block_glyph(0xE7).unwrap(), top_row);
    let diagonal: BlockGlyph = std::array::from_fn(|i| top_right[i] | bottom_left[i]);
    assert_eq!(block_glyph(0xE6).unwrap(), diagonal);
    let left_column: BlockGlyph = std::array::from_fn(|i| top_left[i] | bottom_left[i]);
    assert_eq!(block_glyph(0xE9).unwrap(), left_column);
    let all_but_bottom_right: BlockGlyph = std::array::from_fn(|i| top_row[i] | bottom_left[i]);
    assert_eq!(block_glyph(0xEB).unwrap(), all_but_bottom_right);
    assert_eq!(block_glyph(0xEF).unwrap(), [ALL_ROWS; BLOCK_SIZE]);
}

#[test]
fn box_lines_reach_the_cell_edges_so_neighbours_join() {
    let horizontal = block_glyph(0xF1).unwrap();
    assert_eq!(horizontal, [BAR_ROW; BLOCK_SIZE]);
    let vertical = block_glyph(0xF5).unwrap();
    assert_eq!(vertical[STEM_COLUMN], ALL_ROWS);
    assert_eq!(dots(vertical), BLOCK_SIZE as u32);
    let top_left_corner = block_glyph(0xF0).unwrap();
    assert_eq!(top_left_corner[0], 0);
    assert_eq!(top_left_corner[BLOCK_SIZE - 1], BAR_ROW);
    assert_eq!(top_left_corner[STEM_COLUMN] & 1, 0);
    let cross = block_glyph(0xFA).unwrap();
    assert_eq!(cross[STEM_COLUMN], ALL_ROWS);
    assert_eq!(cross[0], BAR_ROW);
}

#[test]
fn triangles_fill_half_the_cell_with_the_right_angle_in_place() {
    let upper_left = block_glyph(0xFB).unwrap();
    let lower_right = block_glyph(0xFC).unwrap();
    let upper_right = block_glyph(0xFD).unwrap();
    let lower_left = block_glyph(0xFE).unwrap();
    for glyph in [upper_left, lower_right, upper_right, lower_left] {
        assert_eq!(dots(glyph), 21);
    }
    assert_eq!(upper_left[0], ALL_ROWS);
    assert_eq!(lower_right[BLOCK_SIZE - 1], ALL_ROWS);
    assert_eq!(upper_right[BLOCK_SIZE - 1], ALL_ROWS);
    assert_eq!(lower_left[0], ALL_ROWS);
    assert_eq!(upper_right[0], 1);
}

#[test]
fn country_table_matches_manual_rows() {
    const USA: u8 = 32;
    const GERMANY: u8 = 33;
    const FRANCE: u8 = 34;
    const SPAIN: u8 = 40;
    const BELGIUM: u8 = 42;
    assert_eq!(country_symbol(USA, b'#'), '#');
    assert_eq!(country_symbol(GERMANY, b'['), 'Ä');
    assert_eq!(country_symbol(GERMANY, b'~'), 'ß');
    assert_eq!(country_symbol(FRANCE, b'#'), '£');
    assert_eq!(country_symbol(FRANCE, b'|'), 'ù');
    assert_eq!(country_symbol(SPAIN, b'\\'), 'Ñ');
    assert_eq!(country_symbol(SPAIN, b']'), '¿');
    assert_eq!(country_symbol(BELGIUM, b'|'), 'ĳ');
    assert_eq!(country_symbol(GERMANY, b'A'), 'A');
    assert_eq!(country_symbol(43, b'['), '[');
    assert_eq!(country_symbol(0, b'['), '[');
}
