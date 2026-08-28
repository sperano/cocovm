//! DMP-105 dot font: 9-wide x 7-tall glyph cell plus a descender row
//! (`dmp105-protocol.md` §1 "Glyph matrix: 9 wide x 7 high dots"; §6
//! "Descenders/underline: one extra dot row below the 7-dot body (g p q y j;
//! ç µ § ß ƒ)").
//!
//! **ARTISTIC APPROXIMATION — not hardware-verified.** The manual gives cell
//! *geometry* (9x7 + descender row) but, obviously, not the ROM's actual
//! per-dot bitmaps; those are unobtainable from the source material this
//! project has. Every glyph bit pattern below is hand-authored to be a
//! plausible, legible dot-matrix rendering of the character at this cell
//! size — it is not a transcription of real DMP-105 ROM data and must never
//! be cited as a hardware fact.
//!
//! # Representation
//!
//! A [`Glyph`] is 9 columns (dot-matrix printers print column-by-column, and
//! this mirrors the graphics-mode data-byte layout for consistency —
//! `dmp105-protocol.md` §5): each column is one byte, bits 0-6 = that
//! column's 7 body dots top(bit0)-to-bottom(bit6), bit 7 = that column's
//! descender-row dot (only ever set for the specific characters the spec
//! lists as having one).
//!
//! # Character sets
//!
//! - `$20-$7E`: the full 94-char printable ASCII set (`ASCII_FONT`), 1:1 per
//!   `dmp105-protocol.md` §6.
//! - `$A0-$BF`: 32 European symbols. **TODO — not implemented per-code.**
//!   The spec document lists only example characters (à ç £ µ § ® © ¼ ¾ ½ ¶ ¥
//!   Å … ß ™), not the full ordered 32-entry code table, so there is no
//!   verified code-to-glyph mapping to author against. Rather than invent an
//!   assignment, every code in this range currently renders as the
//!   undefined-code placeholder (`undefined_glyph`). Revisit if the full
//!   Appendix C table is transcribed.
//! - `$E0-$FE`: 30 block-graphic characters, described only as "geometric,
//!   quadrant/sextant-style" blocks with no per-code bitmap given either.
//!   [`BLOCK_FONT`] is a **systematic placeholder**, not a verified mapping:
//!   a 2-wide x 3-tall grid of 6 sub-cells (63 non-blank combinations of 6
//!   bits, chosen because 30 fits comfortably and a 2x3 "sextant" grid is the
//!   documented shape family), enumerated in increasing bit order starting
//!   from `$E0` = blank (all 6 sub-cells off, matching the one verified
//!   detail: "$E0 = blank"). The `$E0-$FE` range is actually 31 codes but the
//!   spec claims exactly 30 defined characters (itself a known internal
//!   inconsistency the spec document already flags elsewhere for a different
//!   count); this implementation reconciles that by leaving the last code
//!   (`$FE`) undefined rather than guessing which one the real ROM omits.

/// One glyph: 9 columns, each byte's bits 0-6 = body dot rows top-to-bottom,
/// bit 7 = the descender-row dot for that column.
pub type Glyph = [u8; 9];

/// Bit position of the descender-row dot within a glyph column byte.
const DESCENDER_BIT: u8 = 0x80;

/// Author a glyph from row-major ASCII art (`#` = dot), transposed here into
/// the column-major [`Glyph`] representation described in the module doc comment.
const fn glyph(rows: [&str; 8]) -> Glyph {
    let mut cols: Glyph = [0u8; 9];
    let mut r = 0;
    while r < 8 {
        let bytes = rows[r].as_bytes();
        let bit = if r < 7 { 1u8 << r } else { DESCENDER_BIT };
        let mut c = 0;
        while c < 9 {
            if bytes[c] == b'#' {
                cols[c] |= bit;
            }
            c += 1;
        }
        r += 1;
    }
    cols
}

/// A row with no descender dots, for the 89 of 95 ASCII glyphs that don't
/// need one.
const BLANK_DESCENDER: &str = ".........";

/// 95 glyphs for `$20` (space) through `$7E` (`~`), index 0 = `$20`.
pub const ASCII_FONT: [Glyph; 95] = [
    // $20 ' '
    glyph([
        ".........",
        ".........",
        ".........",
        ".........",
        ".........",
        ".........",
        ".........",
        BLANK_DESCENDER,
    ]),
    // $21 '!'
    glyph([
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        ".........",
        "....#....",
        BLANK_DESCENDER,
    ]),
    // $22 '"'
    glyph([
        "...#.#...",
        "...#.#...",
        ".........",
        ".........",
        ".........",
        ".........",
        ".........",
        BLANK_DESCENDER,
    ]),
    // $23 '#'
    glyph([
        "..#...#..",
        "..#...#..",
        "#########",
        "..#...#..",
        "#########",
        "..#...#..",
        "..#...#..",
        BLANK_DESCENDER,
    ]),
    // $24 '$'
    glyph([
        "....#....",
        "..#####..",
        ".#.#.....",
        "..####...",
        "....#.#..",
        ".#####...",
        "....#....",
        BLANK_DESCENDER,
    ]),
    // $25 '%'
    glyph([
        "##....#..",
        "##...#...",
        ".....#...",
        "....#....",
        "...#.....",
        "..#...##.",
        ".#....##.",
        BLANK_DESCENDER,
    ]),
    // $26 '&'
    glyph([
        "..##.....",
        ".#..#....",
        ".#..#....",
        "..##.....",
        ".#..#.#..",
        "#....##..",
        ".####.#..",
        BLANK_DESCENDER,
    ]),
    // $27 '\''
    glyph([
        "....#....",
        "...#.....",
        ".........",
        ".........",
        ".........",
        ".........",
        ".........",
        BLANK_DESCENDER,
    ]),
    // $28 '('
    glyph([
        ".....#...",
        "....#....",
        "...#.....",
        "...#.....",
        "...#.....",
        "....#....",
        ".....#...",
        BLANK_DESCENDER,
    ]),
    // $29 ')'
    glyph([
        "...#.....",
        "....#....",
        ".....#...",
        ".....#...",
        ".....#...",
        "....#....",
        "...#.....",
        BLANK_DESCENDER,
    ]),
    // $2A '*'
    glyph([
        ".........",
        "..#.#.#..",
        "...###...",
        "..#####..",
        "...###...",
        "..#.#.#..",
        ".........",
        BLANK_DESCENDER,
    ]),
    // $2B '+'
    glyph([
        ".........",
        "....#....",
        "....#....",
        "..#####..",
        "....#....",
        "....#....",
        ".........",
        BLANK_DESCENDER,
    ]),
    // $2C ','
    glyph([
        ".........",
        ".........",
        ".........",
        ".........",
        ".........",
        "....##...",
        "...##....",
        BLANK_DESCENDER,
    ]),
    // $2D '-'
    glyph([
        ".........",
        ".........",
        ".........",
        "..#####..",
        ".........",
        ".........",
        ".........",
        BLANK_DESCENDER,
    ]),
    // $2E '.'
    glyph([
        ".........",
        ".........",
        ".........",
        ".........",
        ".........",
        ".........",
        "....##...",
        BLANK_DESCENDER,
    ]),
    // $2F '/'
    glyph([
        "......#..",
        ".....#...",
        ".....#...",
        "....#....",
        "...#.....",
        "...#.....",
        "..#......",
        BLANK_DESCENDER,
    ]),
    // $30 '0'
    glyph([
        "..#####..",
        ".#.....#.",
        ".#....##.",
        ".#...#.#.",
        ".##..#.#.",
        ".#.....#.",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $31 '1'
    glyph([
        "....#....",
        "...##....",
        "..#.#....",
        "....#....",
        "....#....",
        "....#....",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $32 '2'
    glyph([
        "..#####..",
        ".#.....#.",
        "......#..",
        "....##...",
        "..##.....",
        ".#.......",
        ".#######.",
        BLANK_DESCENDER,
    ]),
    // $33 '3'
    glyph([
        "..#####..",
        ".#.....#.",
        "......#..",
        "...####..",
        "......#..",
        ".#.....#.",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $34 '4'
    glyph([
        ".....#...",
        "....##...",
        "...#.#...",
        "..#..#...",
        ".#######.",
        ".....#...",
        ".....#...",
        BLANK_DESCENDER,
    ]),
    // $35 '5'
    glyph([
        ".#######.",
        ".#.......",
        ".######..",
        "......#..",
        "......#..",
        ".#.....#.",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $36 '6'
    glyph([
        "...####..",
        "..#......",
        ".#.......",
        ".######..",
        ".#.....#.",
        ".#.....#.",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $37 '7'
    glyph([
        ".#######.",
        "......#..",
        ".....#...",
        "....#....",
        "...#.....",
        "...#.....",
        "...#.....",
        BLANK_DESCENDER,
    ]),
    // $38 '8'
    glyph([
        "..#####..",
        ".#.....#.",
        ".#.....#.",
        "..#####..",
        ".#.....#.",
        ".#.....#.",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $39 '9'
    glyph([
        "..#####..",
        ".#.....#.",
        ".#.....#.",
        "..######.",
        ".......#.",
        "......#..",
        "..####...",
        BLANK_DESCENDER,
    ]),
    // $3A ':'
    glyph([
        ".........",
        ".........",
        "....##...",
        ".........",
        ".........",
        "....##...",
        ".........",
        BLANK_DESCENDER,
    ]),
    // $3B ';'
    glyph([
        ".........",
        ".........",
        "....##...",
        ".........",
        ".........",
        "....##...",
        "...##....",
        BLANK_DESCENDER,
    ]),
    // $3C '<'
    glyph([
        "......#..",
        ".....#...",
        "....#....",
        "...#.....",
        "....#....",
        ".....#...",
        "......#..",
        BLANK_DESCENDER,
    ]),
    // $3D '='
    glyph([
        ".........",
        ".........",
        "..#####..",
        ".........",
        "..#####..",
        ".........",
        ".........",
        BLANK_DESCENDER,
    ]),
    // $3E '>'
    glyph([
        "..#......",
        "...#.....",
        "....#....",
        ".....#...",
        "....#....",
        "...#.....",
        "..#......",
        BLANK_DESCENDER,
    ]),
    // $3F '?'
    glyph([
        "..#####..",
        ".#.....#.",
        "......#..",
        ".....#...",
        "....#....",
        ".........",
        "....#....",
        BLANK_DESCENDER,
    ]),
    // $40 '@'
    glyph([
        "..#####..",
        ".#.....#.",
        ".#.##..#.",
        ".#.#.#.#.",
        ".#.##.#..",
        ".#.......",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $41 'A'
    glyph([
        "...#.....",
        "..#.#....",
        ".#...#...",
        ".#####...",
        ".#...#...",
        ".#...#...",
        ".#...#...",
        BLANK_DESCENDER,
    ]),
    // $42 'B'
    glyph([
        ".######..",
        ".#.....#.",
        ".#.....#.",
        ".######..",
        ".#.....#.",
        ".#.....#.",
        ".######..",
        BLANK_DESCENDER,
    ]),
    // $43 'C'
    glyph([
        "..#####..",
        ".#.....#.",
        ".#.......",
        ".#.......",
        ".#.......",
        ".#.....#.",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $44 'D'
    glyph([
        ".#####...",
        ".#....#..",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        ".#....#..",
        ".#####...",
        BLANK_DESCENDER,
    ]),
    // $45 'E'
    glyph([
        ".#######.",
        ".#.......",
        ".#.......",
        ".#####...",
        ".#.......",
        ".#.......",
        ".#######.",
        BLANK_DESCENDER,
    ]),
    // $46 'F'
    glyph([
        ".#######.",
        ".#.......",
        ".#.......",
        ".#####...",
        ".#.......",
        ".#.......",
        ".#.......",
        BLANK_DESCENDER,
    ]),
    // $47 'G'
    glyph([
        "..#####..",
        ".#.....#.",
        ".#.......",
        ".#..####.",
        ".#.....#.",
        ".#.....#.",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $48 'H'
    glyph([
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        ".#######.",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        BLANK_DESCENDER,
    ]),
    // $49 'I'
    glyph([
        "..#####..",
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $4A 'J' (no descender: only lowercase 'j' is on the spec's descender
    // list, so the tail must stay within the 7-row body)
    glyph([
        "......#..",
        "......#..",
        "......#..",
        "......#..",
        ".#....#..",
        ".#....#..",
        "..####...",
        BLANK_DESCENDER,
    ]),
    // $4B 'K'
    glyph([
        ".#....#..",
        ".#...#...",
        ".#..#....",
        ".###.....",
        ".#..#....",
        ".#...#...",
        ".#....#..",
        BLANK_DESCENDER,
    ]),
    // $4C 'L'
    glyph([
        ".#.......",
        ".#.......",
        ".#.......",
        ".#.......",
        ".#.......",
        ".#.......",
        ".#######.",
        BLANK_DESCENDER,
    ]),
    // $4D 'M'
    glyph([
        ".#.....#.",
        ".##...##.",
        ".#.#.#.#.",
        ".#..#..#.",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        BLANK_DESCENDER,
    ]),
    // $4E 'N'
    glyph([
        ".#.....#.",
        ".##....#.",
        ".#.#...#.",
        ".#..#..#.",
        ".#...#.#.",
        ".#....##.",
        ".#.....#.",
        BLANK_DESCENDER,
    ]),
    // $4F 'O'
    glyph([
        "..#####..",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $50 'P'
    glyph([
        ".######..",
        ".#.....#.",
        ".#.....#.",
        ".######..",
        ".#.......",
        ".#.......",
        ".#.......",
        BLANK_DESCENDER,
    ]),
    // $51 'Q'
    glyph([
        "..#####..",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        ".#..#..#.",
        ".#...#...",
        "..####.#.",
        BLANK_DESCENDER,
    ]),
    // $52 'R'
    glyph([
        ".######..",
        ".#.....#.",
        ".#.....#.",
        ".######..",
        ".#..#....",
        ".#...#...",
        ".#....#..",
        BLANK_DESCENDER,
    ]),
    // $53 'S'
    glyph([
        "..#####..",
        ".#.....#.",
        ".#.......",
        "..#####..",
        "......#..",
        ".#.....#.",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $54 'T'
    glyph([
        ".#######.",
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        BLANK_DESCENDER,
    ]),
    // $55 'U'
    glyph([
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $56 'V'
    glyph([
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        "..#...#..",
        "..#...#..",
        "...#.#...",
        "....#....",
        BLANK_DESCENDER,
    ]),
    // $57 'W'
    glyph([
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        ".#..#..#.",
        ".#.#.#.#.",
        ".##...##.",
        ".#.....#.",
        BLANK_DESCENDER,
    ]),
    // $58 'X'
    glyph([
        ".#.....#.",
        "..#...#..",
        "...#.#...",
        "....#....",
        "...#.#...",
        "..#...#..",
        ".#.....#.",
        BLANK_DESCENDER,
    ]),
    // $59 'Y'
    glyph([
        ".#.....#.",
        "..#...#..",
        "...#.#...",
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        BLANK_DESCENDER,
    ]),
    // $5A 'Z'
    glyph([
        ".#######.",
        "......#..",
        ".....#...",
        "....#....",
        "...#.....",
        "..#......",
        ".#######.",
        BLANK_DESCENDER,
    ]),
    // $5B '['
    glyph([
        "...##....",
        "...#.....",
        "...#.....",
        "...#.....",
        "...#.....",
        "...#.....",
        "...##....",
        BLANK_DESCENDER,
    ]),
    // $5C '\\'
    glyph([
        "..#......",
        "...#.....",
        "...#.....",
        "....#....",
        ".....#...",
        ".....#...",
        "......#..",
        BLANK_DESCENDER,
    ]),
    // $5D ']'
    glyph([
        "....##...",
        ".....#...",
        ".....#...",
        ".....#...",
        ".....#...",
        ".....#...",
        "....##...",
        BLANK_DESCENDER,
    ]),
    // $5E '^'
    glyph([
        "....#....",
        "...#.#...",
        "..#...#..",
        ".........",
        ".........",
        ".........",
        ".........",
        BLANK_DESCENDER,
    ]),
    // $5F '_'
    glyph([
        ".........",
        ".........",
        ".........",
        ".........",
        ".........",
        ".........",
        ".#######.",
        BLANK_DESCENDER,
    ]),
    // $60 '`'
    glyph([
        "...#.....",
        "....#....",
        ".........",
        ".........",
        ".........",
        ".........",
        ".........",
        BLANK_DESCENDER,
    ]),
    // $61 'a'
    glyph([
        ".........",
        ".........",
        "..####...",
        "......#..",
        "..#####..",
        ".#....#..",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $62 'b'
    glyph([
        ".#.......",
        ".#.......",
        ".######..",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        ".######..",
        BLANK_DESCENDER,
    ]),
    // $63 'c'
    glyph([
        ".........",
        ".........",
        "..#####..",
        ".#.......",
        ".#.......",
        ".#.......",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $64 'd'
    glyph([
        "......#..",
        "......#..",
        "..#####..",
        ".#....#..",
        ".#....#..",
        ".#....#..",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $65 'e'
    glyph([
        ".........",
        ".........",
        "..#####..",
        ".#.....#.",
        ".#######.",
        ".#.......",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $66 'f'
    glyph([
        "...###...",
        "..#......",
        ".#######.",
        "..#......",
        "..#......",
        "..#......",
        "..#......",
        BLANK_DESCENDER,
    ]),
    // $67 'g'
    glyph([
        ".........",
        "..#####..",
        ".#.....#.",
        ".#.....#.",
        "..#####..",
        "......#..",
        "..#####..",
        ".#####...",
    ]),
    // $68 'h'
    glyph([
        ".#.......",
        ".#.......",
        ".######..",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        BLANK_DESCENDER,
    ]),
    // $69 'i'
    glyph([
        "....#....",
        ".........",
        "...##....",
        "....#....",
        "....#....",
        "....#....",
        "...###...",
        BLANK_DESCENDER,
    ]),
    // $6A 'j'
    glyph([
        ".....#...",
        ".........",
        "....##...",
        ".....#...",
        ".....#...",
        ".....#...",
        ".....#...",
        "..###....",
    ]),
    // $6B 'k'
    glyph([
        ".#.......",
        ".#.......",
        ".#....#..",
        ".#...#...",
        ".####....",
        ".#...#...",
        ".#....#..",
        BLANK_DESCENDER,
    ]),
    // $6C 'l'
    glyph([
        "...##....",
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        "...###...",
        BLANK_DESCENDER,
    ]),
    // $6D 'm'
    glyph([
        ".........",
        ".........",
        ".##.#.##.",
        ".#.#.#.#.",
        ".#.#.#.#.",
        ".#.#.#.#.",
        ".#.#.#.#.",
        BLANK_DESCENDER,
    ]),
    // $6E 'n'
    glyph([
        ".........",
        ".........",
        ".######..",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        BLANK_DESCENDER,
    ]),
    // $6F 'o'
    glyph([
        ".........",
        ".........",
        "..#####..",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $70 'p'
    glyph([
        ".........",
        ".........",
        ".######..",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        ".######..",
        ".#.......",
    ]),
    // $71 'q'
    glyph([
        ".........",
        ".........",
        "..#####..",
        ".#....#..",
        ".#....#..",
        ".#....#..",
        "..#####..",
        "......#..",
    ]),
    // $72 'r'
    glyph([
        ".........",
        ".........",
        ".#.####..",
        ".##......",
        ".#.......",
        ".#.......",
        ".#.......",
        BLANK_DESCENDER,
    ]),
    // $73 's'
    glyph([
        ".........",
        ".........",
        "..#####..",
        ".#.......",
        "..#####..",
        "......#..",
        "..#####..",
        BLANK_DESCENDER,
    ]),
    // $74 't'
    glyph([
        "..#......",
        ".#######.",
        "..#......",
        "..#......",
        "..#......",
        "..#......",
        "...####..",
        BLANK_DESCENDER,
    ]),
    // $75 'u'
    glyph([
        ".........",
        ".........",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        ".#....##.",
        "..######.",
        BLANK_DESCENDER,
    ]),
    // $76 'v'
    glyph([
        ".........",
        ".........",
        ".#.....#.",
        ".#.....#.",
        "..#...#..",
        "..#...#..",
        "...#.#...",
        BLANK_DESCENDER,
    ]),
    // $77 'w'
    glyph([
        ".........",
        ".........",
        ".#.....#.",
        ".#.#.#.#.",
        ".#.#.#.#.",
        ".##...##.",
        ".#.....#.",
        BLANK_DESCENDER,
    ]),
    // $78 'x'
    glyph([
        ".........",
        ".........",
        ".#.....#.",
        "..#...#..",
        "...#.#...",
        "..#...#..",
        ".#.....#.",
        BLANK_DESCENDER,
    ]),
    // $79 'y'
    glyph([
        ".........",
        ".........",
        ".#.....#.",
        ".#.....#.",
        ".#.....#.",
        "..#####..",
        ".......#.",
        "..#####..",
    ]),
    // $7A 'z'
    glyph([
        ".........",
        ".........",
        ".#######.",
        ".....#...",
        "....#....",
        "...#.....",
        ".#######.",
        BLANK_DESCENDER,
    ]),
    // $7B '{'
    glyph([
        "....##...",
        "...#.....",
        "...#.....",
        "..#......",
        "...#.....",
        "...#.....",
        "....##...",
        BLANK_DESCENDER,
    ]),
    // $7C '|'
    glyph([
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        "....#....",
        BLANK_DESCENDER,
    ]),
    // $7D '}'
    glyph([
        "..##.....",
        ".....#...",
        ".....#...",
        "......#..",
        ".....#...",
        ".....#...",
        "..##.....",
        BLANK_DESCENDER,
    ]),
    // $7E '~'
    glyph([
        ".........",
        "..##...#.",
        ".#..###..",
        ".........",
        ".........",
        ".........",
        ".........",
        BLANK_DESCENDER,
    ]),
];

/// First code point covered by [`ASCII_FONT`] (`dmp105-protocol.md` §6:
/// "$20-$7E: standard 94-char ASCII, 1:1").
const ASCII_FIRST: u8 = 0x20;
const ASCII_LAST: u8 = 0x7E;

/// First/last codes of the European set (`dmp105-protocol.md` §6). See the
/// module doc comment: TODO, no per-code mapping available, so this whole
/// range falls back to [`undefined_glyph`].
const EUROPEAN_FIRST: u8 = 0xA0;
const EUROPEAN_LAST: u8 = 0xBF;

/// First/last codes of the block-graphics set (`dmp105-protocol.md` §6).
const BLOCK_FIRST: u8 = 0xE0;
const BLOCK_LAST: u8 = 0xFE;

/// Number of block-graphics characters the spec claims (`dmp105-protocol.md`
/// §6: "30 block-graphic chars"), one fewer than the 31 codes the `$E0-$FE`
/// range actually spans — see the module doc comment's reconciliation note.
const BLOCK_COUNT: u32 = 30;

/// Look up a glyph for a printable ASCII code (`$20-$7E`).
pub fn ascii_glyph(code: u8) -> Option<Glyph> {
    if (ASCII_FIRST..=ASCII_LAST).contains(&code) {
        Some(ASCII_FONT[(code - ASCII_FIRST) as usize])
    } else {
        None
    }
}

/// The literal `X` glyph printed for every undefined/unimplemented code (`dmp105-protocol.md` §3).
pub fn undefined_glyph() -> Glyph {
    ascii_glyph(b'X').expect("'X' is always present in ASCII_FONT")
}

/// European symbol glyph (`$A0-$BF`) — see the module doc comment: not
/// implemented per-code (no verified mapping), always the placeholder.
pub fn european_glyph(code: u8) -> Option<Glyph> {
    if (EUROPEAN_FIRST..=EUROPEAN_LAST).contains(&code) {
        Some(undefined_glyph())
    } else {
        None
    }
}

/// Sextant-style block-graphics glyph (`$E0-$FE`) — an unverified systematic
/// placeholder, not a transcription of the real ROM table (see module doc comment).
pub fn block_glyph(code: u8) -> Option<Glyph> {
    if !(BLOCK_FIRST..=BLOCK_LAST).contains(&code) {
        return None;
    }
    let index = u32::from(code - BLOCK_FIRST);
    if index >= BLOCK_COUNT {
        return None;
    }
    Some(sextant_glyph(index))
}

/// Build glyph `index` (0..=62, 6 bits) as a 2x3 grid of sub-blocks: bit 0 =
/// top-left, bit 1 = top-right, ... bit 5 = bottom-right.
fn sextant_glyph(index: u32) -> Glyph {
    const LEFT_COLS: std::ops::Range<usize> = 1..5;
    const RIGHT_COLS: std::ops::Range<usize> = 5..9;
    const TOP_ROWS: std::ops::Range<usize> = 0..2;
    const MID_ROWS: std::ops::Range<usize> = 2..5;
    const BOTTOM_ROWS: std::ops::Range<usize> = 5..7;

    let sub_blocks: [(bool, std::ops::Range<usize>, std::ops::Range<usize>); 6] = [
        (index & 0x01 != 0, LEFT_COLS, TOP_ROWS.clone()),
        (index & 0x02 != 0, RIGHT_COLS, TOP_ROWS),
        (index & 0x04 != 0, LEFT_COLS, MID_ROWS.clone()),
        (index & 0x08 != 0, RIGHT_COLS, MID_ROWS),
        (index & 0x10 != 0, LEFT_COLS, BOTTOM_ROWS.clone()),
        (index & 0x20 != 0, RIGHT_COLS, BOTTOM_ROWS),
    ];

    let mut cols: Glyph = [0u8; 9];
    for (on, col_range, row_range) in sub_blocks {
        if !on {
            continue;
        }
        for c in col_range {
            for r in row_range.clone() {
                cols[c] |= 1u8 << r;
            }
        }
    }
    cols
}

#[cfg(test)]
#[path = "dmp105_font_test.rs"]
mod tests;
