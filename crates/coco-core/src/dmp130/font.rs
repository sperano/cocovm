//! Artistic glyphs with verified byte mappings where the manual is legible.
//! Exact DMP-130 ROM bitmaps are unavailable. Unknown mappings use an explicit
//! undefined-character glyph, rather than assigning a guessed Unicode symbol.
use super::Charset;
use crate::dmp105_font::{self, Glyph};
const GERMANY: u8 = 33;
const FRANCE: u8 = 34;
const FIRST_ASCII: u8 = 0x20;
const LAST_ASCII: u8 = 0x7e;
pub(super) fn glyph(charset: Charset, country: u8, byte: u8) -> Glyph {
    if (FIRST_ASCII..=LAST_ASCII).contains(&byte) {
        return symbol(if charset == Charset::Tandy {
            country_symbol(country, byte)
        } else {
            char::from(byte)
        });
    }
    if charset == Charset::Tandy {
        if let Some(character) = extended(byte) {
            return symbol(character);
        }
        if let Some(glyph) = dmp105_font::block_glyph(byte) {
            return glyph;
        }
    }
    dmp105_font::undefined_glyph()
}
fn country_symbol(country: u8, byte: u8) -> char {
    // DMP-130 Operation Manual p. 57, Table 26. Other country rows remain
    // unverified; their substitutions are explicitly listed as unsupported.
    match (country, byte) {
        (GERMANY, b'@') => '§',
        (GERMANY, b'[') => 'Ä',
        (GERMANY, b'\\') => 'Ö',
        (GERMANY, b']') => 'Ü',
        (GERMANY, b'{') => 'ä',
        (GERMANY, b'|') => 'ö',
        (GERMANY, b'}') => 'ü',
        (GERMANY, b'~') => 'ß',
        (FRANCE, b'#') => '£',
        (FRANCE, b'@') => 'à',
        (FRANCE, b'[') => '°',
        (FRANCE, b'\\') => 'ç',
        (FRANCE, b']') => '§',
        (FRANCE, b'{') => 'é',
        (FRANCE, b'|') => 'ù',
        (FRANCE, b'}') => 'è',
        _ => char::from(byte),
    }
}
fn extended(byte: u8) -> Option<char> {
    // Legible entries of Appendix A p. 81, visually transcribed.
    Some(match byte {
        0xc0 => 'â',
        0xc1 => 'ê',
        0xc2 => 'î',
        0xc3 => 'ô',
        0xc4 => 'û',
        0xc8 => 'ä',
        0xc9 => 'ö',
        0xca => 'ü',
        0xcb => 'ù',
        0xcc => 'ì',
        0xcd => 'ñ',
        0xce => 'É',
        0xd0 => 'Æ',
        0xd1 => 'æ',
        0xd4 => 'Ø',
        0xd5 => 'ø',
        0xd6 => 'Ñ',
        0xd8 => 'Ä',
        0xda => 'Ö',
        0xdb => 'Ü',
        0xdc => '¿',
        0xdf => 'Å',
        _ => return None,
    })
}
fn symbol(character: char) -> Glyph {
    if character.is_ascii() {
        return dmp105_font::ascii_glyph(character as u8).unwrap_or([0; 9]);
    }
    let (base, accent) = match character {
        'â' => (b'a', '^'),
        'ê' => (b'e', '^'),
        'î' => (b'i', '^'),
        'ô' => (b'o', '^'),
        'û' => (b'u', '^'),
        'ä' => (b'a', ':'),
        'ö' => (b'o', ':'),
        'ü' => (b'u', ':'),
        'Ä' => (b'A', ':'),
        'Ö' => (b'O', ':'),
        'Ü' => (b'U', ':'),
        'ù' => (b'u', '`'),
        'ì' => (b'i', '`'),
        'à' => (b'a', '`'),
        'è' => (b'e', '`'),
        'é' => (b'e', '\''),
        'É' => (b'E', '\''),
        'ñ' => (b'n', '~'),
        'Ñ' => (b'N', '~'),
        'Å' => (b'A', 'o'),
        'ç' => (b'c', ','),
        'Ø' => (b'O', '/'),
        'ø' => (b'o', '/'),
        'Æ' => (b'A', 'E'),
        'æ' => (b'a', 'e'),
        '¿' => (b'?', '!'),
        '§' => (b'S', '|'),
        'ß' => (b'B', '|'),
        '£' => (b'L', '-'),
        '°' => return [0, 0, 2, 5, 5, 2, 0, 0, 0],
        _ => return dmp105_font::undefined_glyph(),
    };
    accented(base, accent)
}
fn accented(base: u8, accent: char) -> Glyph {
    let mut glyph = dmp105_font::ascii_glyph(base).unwrap_or([0; 9]);
    match accent {
        ':' | '^' | '`' | '\'' | '~' | 'o' => {
            for bits in &mut glyph {
                *bits <<= 1;
            }
            match accent {
                ':' => {
                    glyph[2] |= 1;
                    glyph[6] |= 1;
                }
                '^' => {
                    glyph[3] |= 2;
                    glyph[4] |= 1;
                    glyph[5] |= 2;
                }
                '`' => {
                    glyph[3] |= 1;
                    glyph[4] |= 2;
                }
                '\'' => {
                    glyph[4] |= 2;
                    glyph[5] |= 1;
                }
                '~' => {
                    glyph[3] |= 1;
                    glyph[4] |= 2;
                    glyph[5] |= 1;
                }
                _ => {
                    glyph[3] |= 2;
                    glyph[4] |= 5;
                    glyph[5] |= 2;
                }
            }
        }
        ',' => glyph[4] |= 0x80,
        '/' => {
            for (column, bits) in glyph.iter_mut().enumerate().take(8) {
                *bits |= 0x80 >> column;
            }
        }
        'E' | 'e' => {
            glyph[5] |= 0x7f;
            glyph[6] |= 0x49;
            glyph[7] |= 0x49;
        }
        '!' => {
            for bits in &mut glyph {
                *bits = bits.reverse_bits();
            }
        }
        '|' => glyph[3] |= 0x7f,
        '-' => {
            for bits in &mut glyph[1..7] {
                *bits |= 0x10;
            }
        }
        _ => {}
    }
    glyph
}
