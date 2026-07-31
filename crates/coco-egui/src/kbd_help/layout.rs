//! Key-cap data for the on-screen keyboard help: where every key physically
//! sits on a real CoCo keyboard, per machine variant.
//!
//! Sourced rather than reconstructed:
//!
//! - The **CoCo 3** arrangement is read off the keyboard illustration in
//!   Tandy's *Introducing Your Color Computer 3*, "The Keyboard" (printed
//!   page 15): five rows, ALT and CTRL starting rows 2 and 3, BREAK alone at
//!   the top right, CLEAR and ENTER closing rows 2 and 3, two SHIFTs, F1/F2
//!   under the arrows to the right of the space bar, and the four arrows in
//!   the diamond the CoCo 3 introduced.
//! - The legends and shifted legends match the *Color Computer 3 Service
//!   Manual* Figure 5-9 ("Keyboard Array"), the same matrix
//!   [`coco_core::keyboard`]'s position constants encode.
//! - Key counts check against the manuals' specification pages: 57 keys for
//!   the CoCo 3 (13 + 14 + 14 + 13 + 3) and 53 for the CoCo 1/2
//!   (13 + 13 + 13 + 13 + 1). `layout_test.rs` asserts both totals, so a typo
//!   in this table fails the build rather than silently drawing a keyboard
//!   Tandy never shipped.
//!
//! The *Color Computer 2 NTSC Service Manual* has no keycap illustration —
//! its Figure 4-10 is a matrix diagram, not a key plan — so the **CoCo 1/2**
//! layout here is the CoCo 3's minus the four keys the CoCo 3 added: ALT,
//! CTRL, F1 and F2. That is exactly the 57 − 53 difference the two manuals'
//! specification pages state, and their matrix figures agree: the CoCo 2's
//! PA6 row wires only ENTER, CLEAR, BREAK and SHIFT, leaving the four
//! columns the CoCo 3 fills with ALT/CTRL/F1/F2 unconnected.
//!
//! Widths and gaps below are in *key units* — one unit is the pitch of a
//! letter cap, gap included — so a row's total is the exact sum of its
//! slots. The drawing code keeps that true by laying rows out with no
//! inter-item spacing of its own (`super::draw_row`); without that, rows with
//! different cap counts would drift apart and the arrow diamond would not
//! line up.

use coco_core::MachineVariant;

/// Which way an arrow cap points.
///
/// The CoCo's four arrow keys are painted as triangles rather than written as
/// text: the arrow codepoints (U+2190..U+2193) are absent from egui's bundled
/// fonts and render as empty tofu boxes, which is precisely what made the
/// previous version of this window unreadable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// The CoCo legend printed on a cap.
#[derive(Clone, Copy)]
pub(super) enum Legend {
    Text(&'static str),
    Arrow(Dir),
}

/// One key cap of the drawn keyboard.
#[derive(Clone, Copy)]
pub(super) struct Cap {
    /// The unshifted CoCo legend.
    pub(super) main: Legend,
    /// The legend printed above `main`, reached by holding SHIFT. Empty for
    /// caps that have none — every letter, plus the CoCo's `0` and `@`.
    pub(super) shift: &'static str,
    /// Host key to press for this CoCo key in positional mode.
    pub(super) host: &'static str,
    /// Cap width in key units (1.0 is one letter cap).
    pub(super) width: f32,
}

/// A row is a left-to-right run of caps and the gaps between clusters.
#[derive(Clone, Copy)]
pub(super) enum Slot {
    Cap(Cap),
    Gap(f32),
}

/// A row is a sequence of segments rather than one flat list, so the number
/// row's twelve character caps can be shared between the two layouts while
/// each closes with its own gap before BREAK.
pub(super) type Row = &'static [&'static [Slot]];

/// Widths of the caps that carry a word rather than a character. ENTER is
/// wider than CLEAR on the real keyboard, and that difference is what makes
/// the arrow diamond line up: row 3 carries one letter fewer than row 2, so
/// without it the two rows' right edges would not fall where Tandy's
/// illustration puts them.
const W_MOD: f32 = 1.2;
const W_SHIFT: f32 = 1.2;
const W_CLEAR: f32 = 1.4;
const W_ENTER: f32 = 1.75;
const W_SPACE: f32 = 9.0;

/// Gaps that place the arrow diamond: Up and Down share one column centred
/// between Left and Right. Each row's gap is whatever puts its own arrow in
/// that column, so the values differ per row and per variant rather than
/// being one shared "cluster gap". `layout_test.rs` checks the resulting
/// geometry instead of trusting these numbers.
const COCO3_BREAK_GAP: f32 = 1.90;
const COCO3_SIDE_ARROW_GAP: f32 = 0.15;
const COCO3_DOWN_GAP: f32 = 1.20;
const COCO3_SPACE_INDENT: f32 = 2.90;
const COCO3_FN_GAP: f32 = 1.20;
const COCO12_BREAK_GAP: f32 = 0.70;
const COCO12_SIDE_ARROW_GAP: f32 = 0.15;
const COCO12_DOWN_GAP: f32 = 0.0;
const COCO12_SPACE_INDENT: f32 = 2.40;

/// macOS prints "option" on the key egui reports as Alt.
const ALT_HOST: &str = if cfg!(target_os = "macos") {
    "Option"
} else {
    "Alt"
};

/// CLEAR's host keys: Home, plus the backquote alias
/// ([`crate::keymap::key_to_pos`]) added because laptop keyboards — every
/// MacBook among them — have no Home key at all.
const CLEAR_HOST: &str = "Home  `";

const fn key(main: &'static str, shift: &'static str, host: &'static str) -> Slot {
    Slot::Cap(Cap {
        main: Legend::Text(main),
        shift,
        host,
        width: 1.0,
    })
}

const fn word(main: &'static str, host: &'static str, width: f32) -> Slot {
    Slot::Cap(Cap {
        main: Legend::Text(main),
        shift: "",
        host,
        width,
    })
}

const fn arrow(dir: Dir, host: &'static str) -> Slot {
    Slot::Cap(Cap {
        main: Legend::Arrow(dir),
        shift: "",
        host,
        width: 1.0,
    })
}

/// The character caps of the number row, identical on both machines. `0`
/// carries no shifted legend, and `:`/`-` shift to `*`/`=` — the CoCo's
/// punctuation is not the host keyboard's.
const DIGIT_CAPS: &[Slot] = &[
    key("1", "!", "1"),
    key("2", "\"", "2"),
    key("3", "#", "3"),
    key("4", "$", "4"),
    key("5", "%", "5"),
    key("6", "&", "6"),
    key("7", "'", "7"),
    key("8", "(", "8"),
    key("9", ")", "9"),
    key("0", "", "0"),
    key(":", "*", "-"),
    key("-", "=", "="),
];

/// BREAK sits alone at the far right of the number row.
const BREAK_CAP: Slot = word("BREAK", "Esc", W_MOD);

/// Q through @ — row 2's character caps, identical on both machines.
const QWERTY_CAPS: &[Slot] = &[
    key("Q", "", "Q"),
    key("W", "", "W"),
    key("E", "", "E"),
    key("R", "", "R"),
    key("T", "", "T"),
    key("Y", "", "Y"),
    key("U", "", "U"),
    key("I", "", "I"),
    key("O", "", "O"),
    key("P", "", "P"),
    key("@", "", "["),
];

/// A through `;` — row 3's character caps.
const HOME_CAPS: &[Slot] = &[
    key("A", "", "A"),
    key("S", "", "S"),
    key("D", "", "D"),
    key("F", "", "F"),
    key("G", "", "G"),
    key("H", "", "H"),
    key("J", "", "J"),
    key("K", "", "K"),
    key("L", "", "L"),
    key(";", "+", ";"),
];

/// Z through `/` — row 4's character caps.
const BOTTOM_CAPS: &[Slot] = &[
    key("Z", "", "Z"),
    key("X", "", "X"),
    key("C", "", "C"),
    key("V", "", "V"),
    key("B", "", "B"),
    key("N", "", "N"),
    key("M", "", "M"),
    key(",", "<", ","),
    key(".", ">", "."),
    key("/", "?", "/"),
];

const SHIFT_CAP: Slot = word("SHIFT", "Shift", W_SHIFT);
const CLEAR_CAP: Slot = word("CLEAR", CLEAR_HOST, W_CLEAR);
const ENTER_CAP: Slot = word("ENTER", "Return", W_ENTER);
const SPACE_CAP: Slot = word("SPACE", "Space", W_SPACE);

const COCO3_ROWS: &[Row] = &[
    &[DIGIT_CAPS, &[Slot::Gap(COCO3_BREAK_GAP), BREAK_CAP]],
    &[
        &[word("ALT", ALT_HOST, W_MOD)],
        QWERTY_CAPS,
        &[CLEAR_CAP, arrow(Dir::Up, "Up")],
    ],
    &[
        &[word("CTRL", "Ctrl", W_MOD)],
        HOME_CAPS,
        &[
            ENTER_CAP,
            Slot::Gap(COCO3_SIDE_ARROW_GAP),
            arrow(Dir::Left, "Left"),
            arrow(Dir::Right, "Right"),
        ],
    ],
    &[
        &[SHIFT_CAP],
        BOTTOM_CAPS,
        &[
            SHIFT_CAP,
            Slot::Gap(COCO3_DOWN_GAP),
            arrow(Dir::Down, "Down"),
        ],
    ],
    &[&[
        Slot::Gap(COCO3_SPACE_INDENT),
        SPACE_CAP,
        Slot::Gap(COCO3_FN_GAP),
        key("F1", "", "F1"),
        key("F2", "", "F2"),
    ]],
];

const COCO12_ROWS: &[Row] = &[
    &[DIGIT_CAPS, &[Slot::Gap(COCO12_BREAK_GAP), BREAK_CAP]],
    &[QWERTY_CAPS, &[CLEAR_CAP, arrow(Dir::Up, "Up")]],
    &[
        HOME_CAPS,
        &[
            ENTER_CAP,
            Slot::Gap(COCO12_SIDE_ARROW_GAP),
            arrow(Dir::Left, "Left"),
            arrow(Dir::Right, "Right"),
        ],
    ],
    &[
        &[SHIFT_CAP],
        BOTTOM_CAPS,
        &[
            SHIFT_CAP,
            Slot::Gap(COCO12_DOWN_GAP),
            arrow(Dir::Down, "Down"),
        ],
    ],
    &[&[Slot::Gap(COCO12_SPACE_INDENT), SPACE_CAP]],
];

/// The keyboard `variant` shipped with. The CoCo 1 and CoCo 2 share a key
/// plan (they differ in key *feel* — chiclet versus full-travel — which a
/// mapping legend has no way to show and no reason to).
pub(super) fn rows(variant: MachineVariant) -> &'static [Row] {
    match variant {
        MachineVariant::Coco3 => COCO3_ROWS,
        MachineVariant::Coco1 | MachineVariant::Coco2 => COCO12_ROWS,
    }
}

/// Every slot of a row, across its segments.
pub(super) fn slots(row: Row) -> impl Iterator<Item = &'static Slot> {
    row.iter().flat_map(|segment| segment.iter())
}

/// How wide `row` is, in key units.
pub(super) fn row_units(row: Row) -> f32 {
    slots(row)
        .map(|slot| match slot {
            Slot::Cap(cap) => cap.width,
            Slot::Gap(units) => *units,
        })
        .sum()
}

/// Width of the widest row, which is what the window sizes itself from.
pub(super) fn width_units(variant: MachineVariant) -> f32 {
    rows(variant)
        .iter()
        .copied()
        .map(row_units)
        .fold(0.0, f32::max)
}

#[cfg(test)]
#[path = "layout_test.rs"]
mod tests;
