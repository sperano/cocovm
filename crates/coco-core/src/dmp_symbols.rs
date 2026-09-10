//! Glyphs for the non-ASCII symbols the Tandy DMP family prints: the
//! European set, the DMP-130 extended set, and country substitutions.
//! Which symbol each code prints is verified from the manuals
//! (`crate::dmp_charset`); these dot patterns are artistic approximations,
//! since no ROM dump exists (`docs/dmp-font-sources.md`).
use crate::dmp105_font::{
    BLANK_DESCENDER, DESCENDER_BIT, Glyph, ascii_glyph, glyph, undefined_glyph,
};

/// Glyph for `symbol`: ASCII maps 1:1 to the ASCII font, accented letters
/// are composed from their base letter, and anything unknown prints the
/// undefined-code `X`.
pub fn symbol_glyph(symbol: char) -> Glyph {
    if let Ok(code) = u8::try_from(symbol)
        && let Some(glyph) = ascii_glyph(code)
    {
        return glyph;
    }
    if let Some((_, glyph)) = DRAWN.iter().find(|(drawn, _)| *drawn == symbol) {
        return *glyph;
    }
    match decompose(symbol) {
        Some((base, accent)) => accented(base, accent),
        None => undefined_glyph(),
    }
}

#[derive(Clone, Copy)]
enum Accent {
    Acute,
    Grave,
    Circumflex,
    Diaeresis,
    Tilde,
    Ring,
    Cedilla,
}

fn decompose(symbol: char) -> Option<(u8, Accent)> {
    use Accent::*;
    Some(match symbol {
        'á' => (b'a', Acute),
        'é' => (b'e', Acute),
        'í' => (b'i', Acute),
        'ó' => (b'o', Acute),
        'ú' => (b'u', Acute),
        'Á' => (b'A', Acute),
        'É' => (b'E', Acute),
        'Í' => (b'I', Acute),
        'Ó' => (b'O', Acute),
        'Ú' => (b'U', Acute),
        'à' => (b'a', Grave),
        'è' => (b'e', Grave),
        'ì' => (b'i', Grave),
        'ò' => (b'o', Grave),
        'ù' => (b'u', Grave),
        'À' => (b'A', Grave),
        'È' => (b'E', Grave),
        'Ù' => (b'U', Grave),
        'â' => (b'a', Circumflex),
        'ê' => (b'e', Circumflex),
        'î' => (b'i', Circumflex),
        'ô' => (b'o', Circumflex),
        'û' => (b'u', Circumflex),
        'Â' => (b'A', Circumflex),
        'ä' => (b'a', Diaeresis),
        'ë' => (b'e', Diaeresis),
        'ï' => (b'i', Diaeresis),
        'ö' => (b'o', Diaeresis),
        'ü' => (b'u', Diaeresis),
        'Ä' => (b'A', Diaeresis),
        'Ö' => (b'O', Diaeresis),
        'Ü' => (b'U', Diaeresis),
        'ã' => (b'a', Tilde),
        'õ' => (b'o', Tilde),
        'ñ' => (b'n', Tilde),
        'Ñ' => (b'N', Tilde),
        'å' => (b'a', Ring),
        'Å' => (b'A', Ring),
        'ç' => (b'c', Cedilla),
        _ => return None,
    })
}

/// Row 0 of the ASCII `i` is the dot an accent replaces.
const DOT_ROW: u8 = 1;
/// Column the cedilla hangs from, under the bowl of `c`.
const CEDILLA_COLUMN: usize = 4;

const NO_ROW: &str = ".........";
const ACUTE_MARK: Glyph = glyph([
    ".....#...",
    "....#....",
    NO_ROW,
    NO_ROW,
    NO_ROW,
    NO_ROW,
    NO_ROW,
    BLANK_DESCENDER,
]);
const GRAVE_MARK: Glyph = glyph([
    "...#.....",
    "....#....",
    NO_ROW,
    NO_ROW,
    NO_ROW,
    NO_ROW,
    NO_ROW,
    BLANK_DESCENDER,
]);
const CIRCUMFLEX_MARK: Glyph = glyph([
    "....#....",
    "...#.#...",
    NO_ROW,
    NO_ROW,
    NO_ROW,
    NO_ROW,
    NO_ROW,
    BLANK_DESCENDER,
]);
const DIAERESIS_MARK: Glyph = glyph([
    "..#...#..",
    NO_ROW,
    NO_ROW,
    NO_ROW,
    NO_ROW,
    NO_ROW,
    NO_ROW,
    BLANK_DESCENDER,
]);
const TILDE_MARK: Glyph = glyph([
    "...#.#...",
    "....#....",
    NO_ROW,
    NO_ROW,
    NO_ROW,
    NO_ROW,
    NO_ROW,
    BLANK_DESCENDER,
]);
const RING_MARK: Glyph = glyph([
    "....#....",
    "...#.#...",
    "....#....",
    NO_ROW,
    NO_ROW,
    NO_ROW,
    NO_ROW,
    BLANK_DESCENDER,
]);

/// Stack `accent` above `base`, pushing the letter down one row (its bottom
/// row lands on the descender row, which is how the manual lists ç and ß).
fn accented(base: u8, accent: Accent) -> Glyph {
    let mut columns = ascii_glyph(base).unwrap_or([0; 9]);
    let mark = match accent {
        Accent::Cedilla => {
            columns[CEDILLA_COLUMN] |= DESCENDER_BIT;
            return columns;
        }
        Accent::Acute => ACUTE_MARK,
        Accent::Grave => GRAVE_MARK,
        Accent::Circumflex => CIRCUMFLEX_MARK,
        Accent::Diaeresis => DIAERESIS_MARK,
        Accent::Tilde => TILDE_MARK,
        Accent::Ring => RING_MARK,
    };
    for (bits, mark) in columns.iter_mut().zip(mark) {
        if base == b'i' {
            *bits &= !DOT_ROW;
        }
        *bits = (*bits << 1) | mark;
    }
    columns
}

/// Symbols with no ASCII base, drawn by hand on the 9x7 grid plus descender row.
const DRAWN: [(char, Glyph); 28] = [
    (
        '´',
        glyph([
            ".....#...",
            "....#....",
            NO_ROW,
            NO_ROW,
            NO_ROW,
            NO_ROW,
            NO_ROW,
            BLANK_DESCENDER,
        ]),
    ),
    (
        '¨',
        glyph([
            "...#.#...",
            NO_ROW,
            NO_ROW,
            NO_ROW,
            NO_ROW,
            NO_ROW,
            NO_ROW,
            BLANK_DESCENDER,
        ]),
    ),
    (
        '‾',
        glyph([
            "..#####..",
            NO_ROW,
            NO_ROW,
            NO_ROW,
            NO_ROW,
            NO_ROW,
            NO_ROW,
            BLANK_DESCENDER,
        ]),
    ),
    (
        '°',
        glyph([
            "...###...",
            "..#...#..",
            "...###...",
            NO_ROW,
            NO_ROW,
            NO_ROW,
            NO_ROW,
            BLANK_DESCENDER,
        ]),
    ),
    (
        '£',
        glyph([
            "...###...",
            "..#...#..",
            "..#......",
            ".#####...",
            "..#......",
            "..#......",
            ".#######.",
            BLANK_DESCENDER,
        ]),
    ),
    (
        '¥',
        glyph([
            "#.......#",
            ".#.....#.",
            "..#...#..",
            ".#######.",
            "....#....",
            ".#######.",
            "....#....",
            BLANK_DESCENDER,
        ]),
    ),
    (
        '¢',
        glyph([
            "....#....",
            "..#####..",
            ".#..#..#.",
            "#...#....",
            "#...#....",
            ".#..#..#.",
            "..#####..",
            "....#....",
        ]),
    ),
    (
        '¤',
        glyph([
            "#.......#",
            ".#.###.#.",
            "..#...#..",
            "..#...#..",
            "..#...#..",
            ".#.###.#.",
            "#.......#",
            BLANK_DESCENDER,
        ]),
    ),
    (
        'µ',
        glyph([
            NO_ROW,
            NO_ROW,
            ".#.....#.",
            ".#.....#.",
            ".#.....#.",
            ".#....##.",
            ".######.#",
            ".#.......",
        ]),
    ),
    (
        '§',
        glyph([
            "..#####..",
            ".#.......",
            "..###....",
            ".#...#...",
            "..###....",
            ".....#...",
            "..#####..",
            "....#....",
        ]),
    ),
    (
        '¶',
        glyph([
            ".#######.",
            "#.#..#...",
            "#.#..#...",
            ".##..#...",
            "..#..#...",
            "..#..#...",
            "..#..#...",
            BLANK_DESCENDER,
        ]),
    ),
    (
        '†',
        glyph([
            "....#....",
            "....#....",
            ".#######.",
            "....#....",
            "....#....",
            "....#....",
            "....#....",
            BLANK_DESCENDER,
        ]),
    ),
    (
        '▼',
        glyph([
            "#########",
            ".#######.",
            "..#####..",
            "...###...",
            "....#....",
            NO_ROW,
            NO_ROW,
            BLANK_DESCENDER,
        ]),
    ),
    (
        '®',
        glyph([
            "..#####..",
            ".#.....#.",
            "#.####..#",
            "#.#...#.#",
            "#.####..#",
            "#.#..#..#",
            ".#.....#.",
            "..#####..",
        ]),
    ),
    (
        '©',
        glyph([
            "..#####..",
            ".#.....#.",
            "#..####.#",
            "#.#.....#",
            "#.#.....#",
            "#..####.#",
            ".#.....#.",
            "..#####..",
        ]),
    ),
    (
        '™',
        glyph([
            "###.#...#",
            ".#..##.##",
            ".#..#.#.#",
            ".#..#...#",
            ".#..#...#",
            NO_ROW,
            NO_ROW,
            BLANK_DESCENDER,
        ]),
    ),
    (
        '¼',
        glyph([
            ".#......#",
            "##.....#.",
            ".#....#..",
            ".#...#...",
            "....#.#.#",
            "...#..###",
            "..#.....#",
            BLANK_DESCENDER,
        ]),
    ),
    (
        '½',
        glyph([
            ".#......#",
            "##.....#.",
            ".#....#..",
            ".#...#.##",
            "....#...#",
            "...#...#.",
            "..#...###",
            BLANK_DESCENDER,
        ]),
    ),
    (
        '¾',
        glyph([
            "###.....#",
            "..#....#.",
            ".##...#..",
            "..#..#...",
            "###.#.#.#",
            "...#..###",
            "..#.....#",
            BLANK_DESCENDER,
        ]),
    ),
    (
        'ß',
        glyph([
            "..###....",
            ".#...#...",
            ".#...#...",
            ".#.##....",
            ".#...#...",
            ".#...#...",
            ".#.###...",
            ".#.......",
        ]),
    ),
    (
        'ƒ',
        glyph([
            ".....##..",
            "....#..#.",
            "....#....",
            "..#####..",
            "....#....",
            "....#....",
            "#...#....",
            ".###.....",
        ]),
    ),
    (
        '¡',
        glyph([
            "....#....",
            NO_ROW,
            "....#....",
            "....#....",
            "....#....",
            "....#....",
            "....#....",
            BLANK_DESCENDER,
        ]),
    ),
    (
        '¿',
        glyph([
            "....#....",
            NO_ROW,
            "....#....",
            "...#.....",
            "..#......",
            "..#...#..",
            "...###...",
            BLANK_DESCENDER,
        ]),
    ),
    (
        'Æ',
        glyph([
            "...#####.",
            "..#.#....",
            ".#..#....",
            ".#..###..",
            ".####....",
            ".#..#....",
            ".#..####.",
            BLANK_DESCENDER,
        ]),
    ),
    (
        'æ',
        glyph([
            NO_ROW,
            NO_ROW,
            "..##.##..",
            "....#..#.",
            "..#####..",
            ".#..#....",
            "..##.##..",
            BLANK_DESCENDER,
        ]),
    ),
    (
        'Ø',
        glyph([
            "..####.#.",
            ".#....#..",
            ".#...#.#.",
            ".#..#..#.",
            ".#.#...#.",
            "..#....#.",
            ".#.####..",
            BLANK_DESCENDER,
        ]),
    ),
    (
        'ø',
        glyph([
            NO_ROW,
            NO_ROW,
            "..####.#.",
            ".#...##..",
            ".#..#.#..",
            ".##....#.",
            ".#.####..",
            BLANK_DESCENDER,
        ]),
    ),
    (
        'ĳ',
        glyph([
            "..#...#..",
            NO_ROW,
            "..#...#..",
            "..#...#..",
            "..#...#..",
            "..#...#..",
            "..#...#..",
            ".....##..",
        ]),
    ),
];

#[cfg(test)]
#[path = "dmp_symbols_test.rs"]
mod tests;
