//! CoCo 3 keyboard matrix (`DESIGN.md` §7).
//!
//! The keyboard is a 7-row × 8-column matrix. The CPU strobes columns by writing
//! PIA0 port B ($FF02, active low) and senses rows by reading PIA0 port A ($FF00,
//! active low; PA7 is the joystick comparator, not a key). Layout and the CoCo
//! shift semantics are the authentic matrix (cross-checked against MAME's
//! `coco3_keyboard`):
//!
//! ```text
//!         PB0    PB1    PB2    PB3   PB4    PB5   PB6    PB7
//! PA0:    @      A      B      C     D      E     F      G
//! PA1:    H      I      J      K     L      M     N      O
//! PA2:    P      Q      R      S     T      U     V      W
//! PA3:    X      Y      Z      up    down   left  right  space
//! PA4:    0      1      2      3     4      5     6      7
//! PA5:    8      9      :(*)   ;(+)  ,(<)   -(=)  .(>)   /(?)
//! PA6:    ENTER  CLEAR  BREAK  ALT   CTRL   F1    F2     SHIFT
//! ```

use serde::{Deserialize, Serialize};

pub const ROWS: usize = 7;
pub const COLS: usize = 8;

/// A CoCo key position as `(row, column)` in the matrix.
pub type Pos = (u8, u8);

// Named positions for the non-alphanumeric keys.
pub const UP: Pos = (3, 3);
pub const DOWN: Pos = (3, 4);
pub const LEFT: Pos = (3, 5);
pub const RIGHT: Pos = (3, 6);
pub const SPACE: Pos = (3, 7);
pub const ENTER: Pos = (6, 0);
pub const CLEAR: Pos = (6, 1);
pub const BREAK: Pos = (6, 2);
pub const ALT: Pos = (6, 3);
pub const CTRL: Pos = (6, 4);
pub const F1: Pos = (6, 5);
pub const F2: Pos = (6, 6);
pub const SHIFT: Pos = (6, 7);
pub const AT: Pos = (0, 0);

/// The live matrix state: `rows[r]` has bit `c` set when key `(r, c)` is held.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Keyboard {
    rows: [u8; ROWS],
}

impl Keyboard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Press or release the key at `(row, col)`.
    pub fn set(&mut self, pos: Pos, down: bool) {
        let (row, col) = (pos.0 as usize, pos.1);
        if row >= ROWS || col as usize >= COLS {
            return;
        }
        if down {
            self.rows[row] |= 1 << col;
        } else {
            self.rows[row] &= !(1 << col);
        }
    }

    /// Release every key.
    pub fn release_all(&mut self) {
        self.rows = [0; ROWS];
    }

    /// Compute the PIA0 port-A row sense for a given port-B column strobe.
    ///
    /// Both are active low: a column is selected when its `strobe` bit is 0, and a
    /// sensed row reads 0 when a held key connects it to a selected column. PA7 is
    /// left high (joystick comparator).
    pub fn sense(&self, strobe: u8) -> u8 {
        let selected = !strobe; // 1 = column currently strobed low
        // Rows 0..6 sense keys; PA7 (bit 7) stays high (joystick comparator).
        let mut pa = 0xFF;
        for (r, &pressed) in self.rows.iter().enumerate() {
            if pressed & selected != 0 {
                pa &= !(1 << r);
            }
        }
        pa
    }
}

/// Map a character to the CoCo key that produces it, plus whether the CoCo SHIFT
/// key must be held. Used by symbolic ("natural") keyboard mode. Returns `None`
/// for characters with no CoCo key.
pub fn char_key(c: char) -> Option<(Pos, bool)> {
    // Letters: @ A..Z live linearly from (0,0); uppercase needs CoCo shift.
    if c.is_ascii_alphabetic() {
        let upper = c.to_ascii_uppercase();
        let p = 1 + (upper as u8 - b'A'); // '@' is position 0, 'A' is 1
        let pos = (p / COLS as u8, p % COLS as u8);
        return Some((pos, c.is_ascii_uppercase()));
    }
    let (pos, shift) = match c {
        '@' => (AT, false),
        '0'..='7' => ((4, c as u8 - b'0'), false),
        '8' => ((5, 0), false),
        '9' => ((5, 1), false),
        '!' => ((4, 1), true),
        '"' => ((4, 2), true),
        '#' => ((4, 3), true),
        '$' => ((4, 4), true),
        '%' => ((4, 5), true),
        '&' => ((4, 6), true),
        '\'' => ((4, 7), true),
        '(' => ((5, 0), true),
        ')' => ((5, 1), true),
        ':' => ((5, 2), false),
        '*' => ((5, 2), true),
        ';' => ((5, 3), false),
        '+' => ((5, 3), true),
        ',' => ((5, 4), false),
        '<' => ((5, 4), true),
        '-' => ((5, 5), false),
        '=' => ((5, 5), true),
        '.' => ((5, 6), false),
        '>' => ((5, 6), true),
        '/' => ((5, 7), false),
        '?' => ((5, 7), true),
        ' ' => (SPACE, false),
        '\n' | '\r' => (ENTER, false),
        _ => return None,
    };
    Some((pos, shift))
}
