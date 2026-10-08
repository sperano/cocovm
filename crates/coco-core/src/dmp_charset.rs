//! Tandy DMP-family code tables: the 32 European symbols (`$A0-$BF`), the
//! DMP-130's 32 extended symbols (`$C0-$DF`), the 30 block graphics
//! (`$E0-$FE`), and the DMP-130 country substitutions.
//!
//! Sources: DMP-105 Operation Manual Appendix C pp. 47-49 (typeset tables);
//! DMP-130 Operation Manual p. 57 Table 26 and p. 81 (grid plus a real
//! printout of every code). Cells the typeset tables leave ambiguous are
//! taken from the printout and marked INFERRED. See wiki `cocovm/dmp-font-sources`.

/// First code of the European symbol set.
pub const EUROPEAN_FIRST: u8 = 0xA0;

/// European symbols in code order from `$A0`. INFERRED from the DMP-130
/// printout where the DMP-105 table is unreadable: `$A4` grave, `$B5`
/// overline, `$BE` diaeresis.
pub const EUROPEAN: [char; 32] = [
    '´', 'à', 'ç', '£', '`', 'µ', '°', '▼', '†', '§', '®', '©', '¼', '¾', '½', '¶', // $A0-$AF
    '¥', 'Ä', 'Ö', 'Ü', '¢', '‾', 'ä', 'ö', 'ü', 'ß', '™', 'é', 'ù', 'è', '¨', 'ƒ', // $B0-$BF
];

/// First code of the DMP-130 extended symbol set.
pub const EXTENDED_FIRST: u8 = 0xC0;

/// DMP-130 extended symbols in code order from `$C0` (p. 81 grid and printout).
pub const EXTENDED: [char; 32] = [
    'â', 'ê', 'î', 'ô', 'û', '^', 'ë', 'ï', 'á', 'í', 'ó', 'ú', '¡', 'ñ', 'ã', 'õ', // $C0-$CF
    'Æ', 'æ', 'Å', 'å', 'Ø', 'ø', 'Ñ', 'É', 'Á', 'Í', 'Ó', 'Ú', '¿', 'Ù', 'È', 'Â', // $D0-$DF
];

pub fn european_symbol(code: u8) -> Option<char> {
    table_symbol(&EUROPEAN, EUROPEAN_FIRST, code)
}

pub fn extended_symbol(code: u8) -> Option<char> {
    table_symbol(&EXTENDED, EXTENDED_FIRST, code)
}

fn table_symbol(table: &[char; 32], first: u8, code: u8) -> Option<char> {
    let index = usize::from(code.checked_sub(first)?);
    table.get(index).copied()
}

/// First and last block-graphic codes; `$FF` is not in the table.
pub const BLOCK_FIRST: u8 = 0xE0;
pub const BLOCK_LAST: u8 = 0xFE;

/// Block graphics are a 6x6 dot matrix (DMP-105 p. 22, p. 49 note).
pub const BLOCK_SIZE: usize = 6;

/// One block glyph: 6 columns, bits 0-5 = rows top to bottom.
pub type BlockGlyph = [u8; BLOCK_SIZE];

const HALF: usize = BLOCK_SIZE / 2;
const ALL_ROWS: u8 = (1 << BLOCK_SIZE) - 1;
const TOP_ROWS: u8 = (1 << HALF) - 1;
const BOTTOM_ROWS: u8 = ALL_ROWS & !TOP_ROWS;
/// Box-drawing pieces share one stem column and one bar row.
const STEM_COLUMN: usize = 2;
const BAR_ROW: u8 = 1 << 2;
const STEM_UP: u8 = TOP_ROWS;
const STEM_DOWN: u8 = BOTTOM_ROWS | BAR_ROW;

/// Quadrant mask bits.
const TOP_LEFT: u8 = 1;
const TOP_RIGHT: u8 = 2;
const BOTTOM_LEFT: u8 = 4;
const BOTTOM_RIGHT: u8 = 8;
/// Quadrant blocks `$E1-$EF` in the manual's order: the four singles, two
/// diagonals, top and bottom rows, left and right columns, the four
/// three-quarter blocks, then the full block.
const QUADRANT_MASKS: [u8; 15] = [
    TOP_LEFT,
    TOP_RIGHT,
    BOTTOM_LEFT,
    BOTTOM_RIGHT,
    TOP_LEFT | BOTTOM_RIGHT,
    TOP_RIGHT | BOTTOM_LEFT,
    TOP_LEFT | TOP_RIGHT,
    BOTTOM_LEFT | BOTTOM_RIGHT,
    TOP_LEFT | BOTTOM_LEFT,
    TOP_RIGHT | BOTTOM_RIGHT,
    TOP_LEFT | TOP_RIGHT | BOTTOM_LEFT,
    TOP_LEFT | TOP_RIGHT | BOTTOM_RIGHT,
    TOP_LEFT | BOTTOM_LEFT | BOTTOM_RIGHT,
    TOP_RIGHT | BOTTOM_LEFT | BOTTOM_RIGHT,
    TOP_LEFT | TOP_RIGHT | BOTTOM_LEFT | BOTTOM_RIGHT,
];
/// Box-drawing pieces `$F0-$FA`: stem rows, then bar start and end columns.
const BOX_PIECES: [(u8, usize, usize); 11] = [
    (STEM_DOWN, STEM_COLUMN, BLOCK_SIZE), // $F0 ┌
    (0, 0, BLOCK_SIZE),                   // $F1 ─
    (STEM_DOWN, 0, STEM_COLUMN + 1),      // $F2 ┐
    (STEM_DOWN, 0, BLOCK_SIZE),           // $F3 ┬
    (ALL_ROWS, STEM_COLUMN, BLOCK_SIZE),  // $F4 ├
    (ALL_ROWS, 0, 0),                     // $F5 │
    (STEM_UP, STEM_COLUMN, BLOCK_SIZE),   // $F6 └
    (STEM_UP, 0, STEM_COLUMN + 1),        // $F7 ┘
    (STEM_UP, 0, BLOCK_SIZE),             // $F8 ┴
    (ALL_ROWS, 0, STEM_COLUMN + 1),       // $F9 ┤
    (ALL_ROWS, 0, BLOCK_SIZE),            // $FA ┼
];
const QUADRANT_LAST: u8 = BLOCK_FIRST + QUADRANT_MASKS.len() as u8;
const BOX_FIRST: u8 = QUADRANT_LAST + 1;
const TRIANGLE_FIRST: u8 = BOX_FIRST + BOX_PIECES.len() as u8;

/// Block glyph for `code`, or `None` outside `$E0-$FE`.
pub fn block_glyph(code: u8) -> Option<BlockGlyph> {
    if !(BLOCK_FIRST..=BLOCK_LAST).contains(&code) {
        return None;
    }
    Some(if code == BLOCK_FIRST {
        [0; BLOCK_SIZE]
    } else if code <= QUADRANT_LAST {
        quadrants(QUADRANT_MASKS[usize::from(code - BLOCK_FIRST - 1)])
    } else if code < TRIANGLE_FIRST {
        box_piece(BOX_PIECES[usize::from(code - BOX_FIRST)])
    } else {
        triangle(code - TRIANGLE_FIRST)
    })
}

fn quadrants(mask: u8) -> BlockGlyph {
    let mut glyph = [0u8; BLOCK_SIZE];
    for (column, bits) in glyph.iter_mut().enumerate() {
        let (top, bottom) = if column < HALF {
            (TOP_LEFT, BOTTOM_LEFT)
        } else {
            (TOP_RIGHT, BOTTOM_RIGHT)
        };
        if mask & top != 0 {
            *bits |= TOP_ROWS;
        }
        if mask & bottom != 0 {
            *bits |= BOTTOM_ROWS;
        }
    }
    glyph
}

fn box_piece((stem, bar_start, bar_end): (u8, usize, usize)) -> BlockGlyph {
    let mut glyph = [0u8; BLOCK_SIZE];
    glyph[STEM_COLUMN] = stem;
    for bits in &mut glyph[bar_start..bar_end] {
        *bits |= BAR_ROW;
    }
    glyph
}

/// `$FB` ◤, `$FC` ◢, `$FD` ◥, `$FE` ◣: filled right triangles named by
/// where the right angle sits.
fn triangle(index: u8) -> BlockGlyph {
    let last = BLOCK_SIZE - 1;
    let mut glyph = [0u8; BLOCK_SIZE];
    for (column, bits) in glyph.iter_mut().enumerate() {
        for row in 0..BLOCK_SIZE {
            let filled = match index {
                0 => column + row <= last,
                1 => column + row >= last,
                2 => column >= row,
                _ => column <= row,
            };
            if filled {
                *bits |= 1 << row;
            }
        }
    }
    glyph
}

/// `ESC Y n` country codes (DMP-130 p. 57): 32 USA through 42 Belgium.
pub const COUNTRY_FIRST: u8 = 32;
pub const COUNTRY_COUNT: usize = 11;

/// ASCII codes Table 26 substitutes.
const COUNTRY_ROW_COUNT: usize = 12;

/// Table 26 by ASCII code, columns USA through Belgium; inferred cells are
/// listed in wiki `cocovm/dmp-font-sources`.
const COUNTRY_ROWS: [(u8, [char; COUNTRY_COUNT]); COUNTRY_ROW_COUNT] = [
    (
        b'#',
        ['#', '#', '£', '#', '#', '#', '#', '£', '£', '£', '#'],
    ),
    (
        b'$',
        ['$', '$', '$', '¤', '¤', '$', '¤', '$', '$', '$', '$'],
    ),
    (
        b'@',
        ['@', '§', 'à', 'Ü', 'É', 'É', '@', '§', '§', '@', 'à'],
    ),
    (
        b'[',
        ['[', 'Ä', '°', 'Æ', 'Ä', 'Æ', 'Ä', '°', '¡', '[', '°'],
    ),
    (
        b'\\',
        ['\\', 'Ö', 'ç', 'Ø', 'Ö', 'Ø', 'Ö', 'ç', 'Ñ', '\\', 'ç'],
    ),
    (
        b']',
        [']', 'Ü', '§', 'Å', 'Å', 'Å', 'Å', 'é', '¿', ']', '§'],
    ),
    (
        b'^',
        ['^', '^', '^', 'Ä', 'Ü', 'Ü', '`', '^', '^', '^', '^'],
    ),
    (
        b'`',
        ['`', '`', '`', 'ü', 'é', 'é', '´', 'ù', '`', '`', '`'],
    ),
    (
        b'{',
        ['{', 'ä', 'é', 'æ', 'ä', 'æ', 'ä', 'à', '°', '{', 'é'],
    ),
    (
        b'|',
        ['|', 'ö', 'ù', 'ø', 'ö', 'ø', 'ö', 'ò', 'ñ', '|', 'ĳ'],
    ),
    (
        b'}',
        ['}', 'ü', 'è', 'å', 'å', 'å', 'å', 'è', 'ç', '}', 'è'],
    ),
    (
        b'~',
        ['~', 'ß', ' ', 'ä', 'ü', 'ü', '‾', 'ì', '~', '‾', '‾'],
    ),
];

/// The symbol `byte` prints under `country`; bytes outside Table 26 and
/// unknown countries print the ASCII symbol.
pub fn country_symbol(country: u8, byte: u8) -> char {
    let Some(column) = country
        .checked_sub(COUNTRY_FIRST)
        .map(usize::from)
        .filter(|column| *column < COUNTRY_COUNT)
    else {
        return char::from(byte);
    };
    COUNTRY_ROWS
        .iter()
        .find(|(code, _)| *code == byte)
        .map_or(char::from(byte), |(_, row)| row[column])
}

#[cfg(test)]
#[path = "dmp_charset_test.rs"]
mod tests;
