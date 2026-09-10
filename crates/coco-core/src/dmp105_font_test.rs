use super::*;

#[test]
fn ascii_font_covers_every_printable_code() {
    for code in ASCII_FIRST..=ASCII_LAST {
        assert!(ascii_glyph(code).is_some(), "missing glyph for {code:#04x}");
    }
    assert!(ascii_glyph(0x1F).is_none());
    assert!(ascii_glyph(0x7F).is_none());
}

#[test]
fn space_glyph_is_blank() {
    assert_eq!(ascii_glyph(b' ').unwrap(), [0u8; 9]);
}

#[test]
fn undefined_glyph_matches_capital_x() {
    assert_eq!(undefined_glyph(), ascii_glyph(b'X').unwrap());
}

#[test]
fn only_the_documented_five_ascii_letters_use_the_descender_row() {
    let descenders: Vec<u8> = (ASCII_FIRST..=ASCII_LAST)
        .filter(|&c| {
            ascii_glyph(c)
                .unwrap()
                .iter()
                .any(|&col| col & DESCENDER_BIT != 0)
        })
        .collect();
    assert_eq!(descenders, vec![b'g', b'j', b'p', b'q', b'y']);
}
