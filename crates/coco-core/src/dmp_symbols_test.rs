use super::*;
use crate::dmp_charset::{COUNTRY_COUNT, COUNTRY_FIRST, EUROPEAN, EXTENDED, country_symbol};

fn blank() -> Glyph {
    [0; 9]
}

#[test]
fn every_table_symbol_has_its_own_glyph() {
    let mut symbols: Vec<char> = EUROPEAN.iter().chain(EXTENDED.iter()).copied().collect();
    for country in COUNTRY_FIRST..COUNTRY_FIRST + COUNTRY_COUNT as u8 {
        for byte in 0x20..=0x7E {
            symbols.push(country_symbol(country, byte));
        }
    }
    for symbol in symbols {
        let glyph = symbol_glyph(symbol);
        if symbol == ' ' {
            assert_eq!(glyph, blank());
            continue;
        }
        if symbol.is_ascii() {
            assert_eq!(glyph, ascii_glyph(symbol as u8).unwrap());
            continue;
        }
        assert_ne!(glyph, blank(), "{symbol:?} is blank");
        assert_ne!(glyph, undefined_glyph(), "{symbol:?} has no glyph");
    }
}

#[test]
fn ascii_symbols_use_the_ascii_font() {
    assert_eq!(symbol_glyph('A'), ascii_glyph(b'A').unwrap());
    assert_eq!(symbol_glyph('~'), ascii_glyph(b'~').unwrap());
}

#[test]
fn unknown_symbols_print_the_undefined_x() {
    assert_eq!(symbol_glyph('Ж'), undefined_glyph());
}

#[test]
fn accents_sit_above_a_letter_pushed_down_one_row() {
    let a = ascii_glyph(b'a').unwrap();
    let a_umlaut = symbol_glyph('ä');
    for column in 0..9 {
        let pushed_down = a[column] << 1;
        assert_eq!(a_umlaut[column] & pushed_down, pushed_down);
    }
    assert!(a_umlaut.iter().any(|bits| bits & DOT_ROW != 0));
    assert_ne!(symbol_glyph('ä'), symbol_glyph('á'));
    assert_ne!(symbol_glyph('à'), symbol_glyph('á'));
    assert_eq!(symbol_glyph('Å'), symbol_glyph('Å'));
}

#[test]
fn accented_i_drops_its_dot() {
    fn count(glyph: Glyph) -> u32 {
        glyph.iter().map(|bits| bits.count_ones()).sum()
    }
    const ACCENT_DOTS: u32 = 2;
    let i = ascii_glyph(b'i').unwrap();
    assert_eq!(count(symbol_glyph('í')), count(i) - 1 + ACCENT_DOTS);
    let a = ascii_glyph(b'a').unwrap();
    assert_eq!(count(symbol_glyph('á')), count(a) + ACCENT_DOTS);
}

#[test]
fn cedilla_hangs_from_the_descender_row() {
    let c = ascii_glyph(b'c').unwrap();
    let c_cedilla = symbol_glyph('ç');
    assert_eq!(c_cedilla[4] & 0x80, 0x80);
    for column in 0..9 {
        assert_eq!(c_cedilla[column] & 0x7F, c[column]);
    }
}
