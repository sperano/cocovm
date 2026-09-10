//! Byte-to-glyph lookup. Code tables are verified from the manual
//! (`crate::dmp_charset`); dot patterns are artistic (`crate::dmp_symbols`).
use super::Charset;
use crate::dmp_charset::{self, BlockGlyph};
use crate::dmp_symbols::symbol_glyph;
use crate::dmp105_font::{self, Glyph};
const FIRST_ASCII: u8 = 0x20;
const LAST_ASCII: u8 = 0x7e;
pub(super) fn glyph(charset: Charset, country: u8, byte: u8) -> Glyph {
    if (FIRST_ASCII..=LAST_ASCII).contains(&byte) {
        return symbol_glyph(if charset == Charset::Tandy {
            dmp_charset::country_symbol(country, byte)
        } else {
            char::from(byte)
        });
    }
    if charset == Charset::Tandy {
        let symbol =
            dmp_charset::european_symbol(byte).or_else(|| dmp_charset::extended_symbol(byte));
        if let Some(symbol) = symbol {
            return symbol_glyph(symbol);
        }
    }
    dmp105_font::undefined_glyph()
}
/// Block graphics exist only in the Tandy character set (p. 34).
pub(super) fn block(charset: Charset, byte: u8) -> Option<BlockGlyph> {
    if charset == Charset::Tandy {
        dmp_charset::block_glyph(byte)
    } else {
        None
    }
}
