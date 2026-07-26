use super::*;

/// Index of 'O' (@=0, A=1, ..., O=15) and 'K' (=11) in the shared
/// $00-$3F code space both tables use.
const GLYPH_O: usize = 15;
const GLYPH_K: usize = 11;

#[test]
fn plain_o_is_square() {
    assert_eq!(
        MC6847_FONT[GLYPH_O],
        [0x00, 0x00, 0x00, 0x3E, 0x22, 0x22, 0x22, 0x22, 0x22, 0x3E, 0x00, 0x00]
    );
}

#[test]
fn t1_o_is_rounded() {
    assert_eq!(
        MC6847T1_FONT[GLYPH_O],
        [0x00, 0x1C, 0x22, 0x22, 0x22, 0x22, 0x22, 0x1C, 0x00, 0x00, 0x00, 0x00]
    );
}

/// 'K' is shape-identical between the two chips — only row-shifted (the
/// plain chip's glyph occupies rows 3-10, the T1's rows 1-8).
#[test]
fn k_is_shape_identical_between_chips_modulo_row_offset() {
    let trim = |rows: &[u8; 12]| -> Vec<u8> {
        let start = rows.iter().position(|&r| r != 0).unwrap();
        let end = rows.iter().rposition(|&r| r != 0).unwrap();
        rows[start..=end].to_vec()
    };
    assert_eq!(trim(&MC6847_FONT[GLYPH_K]), trim(&MC6847T1_FONT[GLYPH_K]));
}
