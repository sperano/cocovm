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

#[test]
fn european_range_is_placeholder_only() {
    assert_eq!(european_glyph(0xA0), Some(undefined_glyph()));
    assert_eq!(european_glyph(0xBF), Some(undefined_glyph()));
    assert_eq!(european_glyph(0xC0), None);
}

#[test]
fn block_e0_is_blank_and_range_matches_spec_count() {
    assert_eq!(block_glyph(0xE0), Some([0u8; 9]));
    // 30 defined characters starting at $E0 -> last defined is $FD; $FE
    // is left undefined (module doc comment's reconciliation note).
    assert!(block_glyph(0xFD).is_some());
    assert_eq!(block_glyph(0xFE), None);
    assert_eq!(block_glyph(0x9F), None);
}

#[test]
fn block_glyphs_are_distinct_non_blank_patterns() {
    let patterns: Vec<Glyph> = (0xE1u8..=0xFDu8).map(|c| block_glyph(c).unwrap()).collect();
    for p in &patterns {
        assert_ne!(*p, [0u8; 9], "non-blank block index rendered blank");
    }
    let unique: std::collections::HashSet<Glyph> = patterns.iter().copied().collect();
    assert_eq!(
        unique.len(),
        patterns.len(),
        "block glyphs must be distinct"
    );
}
