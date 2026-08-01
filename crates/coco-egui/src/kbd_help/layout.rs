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
//!   the CoCo 3 (13 + 14 + 14 + 13 + 3). `layout_test.rs` asserts the totals
//!   and the per-row counts, so a typo in this table fails the build rather
//!   than silently drawing a keyboard Tandy never shipped.
//!
//! The **CoCo 1/2** plan is *not* the CoCo 3's minus four keys, and assuming
//! it was is how this table got the arrows wrong once already — the key
//! counts match either way, so counting cannot catch it. Read off a
//! photograph of a 64K Color Computer 2, the real arrangement is:
//!
//! - `↑` is the **first** key of row 2 and `↓` the **first** key of row 3 —
//!   the slots the CoCo 3 gives to ALT and CTRL.
//! - `←` and `→` close **row 2**, after `@`.
//! - ENTER and CLEAR sit together at the end of **row 3**, CLEAR outermost;
//!   the CoCo 3 splits them, CLEAR ending row 2 and ENTER row 3.
//! - There is no arrow diamond at all. That is a CoCo 3 innovation.
//!
//! Rows are 13 + 14 + 13 + 12 + 1 = 53, matching the *Color Computer 2 NTSC
//! Service Manual*'s "53-key" specification. Its Figure 4-10 is a matrix
//! diagram rather than a key plan, so it confirms the key *set* — the PA6 row
//! wires only ENTER, CLEAR, BREAK and SHIFT, leaving the columns the CoCo 3
//! fills with ALT/CTRL/F1/F2 unconnected — but says nothing about placement.
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
#[derive(Clone, Copy, Debug)]
pub(super) enum Legend {
    Text(&'static str),
    Arrow(Dir),
}

/// One key cap of the drawn keyboard.
#[derive(Clone, Copy, Debug)]
pub(super) struct Cap {
    /// The unshifted CoCo legend.
    pub(super) main: Legend,
    /// The legend printed above `main`, reached by holding SHIFT. `None` for
    /// caps that have none — every letter, plus the CoCo's `0` and `@`.
    pub(super) shift: Option<&'static str>,
    /// Host key to press for this CoCo key in positional mode.
    pub(super) host: &'static str,
    /// Cap width in key units (1.0 is one letter cap).
    pub(super) width: f32,
}

impl Cap {
    /// Whether symbolic mode still reaches this key by *position* rather than
    /// by the character typed. These are exactly the keys
    /// [`crate::keymap::control_key_pos`] routes — the ones that produce no
    /// text — so they are the only caps whose host key still means anything
    /// once the user switches modes, and the only ones that keep their host
    /// line there. Everything else in symbolic mode is reached by typing the
    /// character, which is what the mode is for.
    pub(super) fn routed_in_symbolic(&self) -> bool {
        match self.main {
            Legend::Arrow(_) => true,
            Legend::Text(text) => matches!(text, "ENTER" | "CLEAR" | "BREAK" | "F1" | "F2"),
        }
    }
}

/// One position along a row: either a cap or the empty space before the next
/// cluster.
#[derive(Clone, Copy, Debug)]
pub(super) enum Slot {
    Cap(Cap),
    Gap(f32),
}

impl Slot {
    /// How much of the row this slot consumes, in key units.
    pub(super) const fn units(self) -> f32 {
        match self {
            Slot::Cap(cap) => cap.width,
            Slot::Gap(units) => units,
        }
    }
}

/// A run of adjacent slots. Rows are built from these rather than written
/// flat so the character caps can be shared between the two layouts while
/// each row closes with its own variant-specific gap.
pub(super) type Segment = &'static [Slot];

/// One row of the keyboard, left to right.
pub(super) type Row = &'static [Segment];

// Cap widths, in key units. On both machines ALT/CTRL and the arrows are
// plain single-width caps; only SHIFT, ENTER, CLEAR, BREAK and the space bar
// are oversized, and by different amounts on the two keyboards.
/// ALT and CTRL: single width, like the arrows that take those slots on the
/// CoCo 1/2.
const W_MODIFIER: f32 = 1.0;
const W_SHIFT: f32 = 1.2;
const W_CLEAR: f32 = 1.15;
const W_ENTER: f32 = 1.2;
const W_BREAK: f32 = 1.15;
const W_SPACE: f32 = 9.0;
/// CoCo 1/2 proportions: its ENTER and CLEAR share the end of row 3, and its
/// SHIFTs are visibly wider than the CoCo 3's.
const W_COCO12_ENTER: f32 = 1.6;
const W_COCO12_CLEAR: f32 = 1.4;
const W_COCO12_SHIFT: f32 = 1.45;

// Row indents. Both keyboards are staggered like a typewriter rather than
// stacked flush, so each row starts a fraction of a cap right of the one
// above or below it — most visibly the number row, which sits about half a
// cap right of the row under it, and the letter columns, which step right as
// you go down (Q, then A, then Z). Measured off the same sources as the key
// plans; without them the drawn keyboard reads as a grid again, which is the
// complaint this window exists to answer.
const COCO3_ROW1_INDENT: f32 = 0.46;
const COCO3_ROW3_INDENT: f32 = 0.17;
const COCO3_ROW4_INDENT: f32 = 0.50;
const COCO12_ROW1_INDENT: f32 = 0.57;
const COCO12_ROW3_INDENT: f32 = 0.20;
const COCO12_ROW4_INDENT: f32 = 0.16;

// Gaps that place the arrow diamond: Up and Down share one column centred
// between Left and Right. Each row's gap is whatever puts its own arrow in
// that column, so the values differ per row and per variant rather than
// being one shared "cluster gap". `layout_test.rs` checks the resulting
// geometry instead of trusting these numbers.
const COCO3_BREAK_GAP: f32 = 1.04;
const COCO3_SIDE_ARROW_GAP: f32 = 0.28;
const COCO3_DOWN_GAP: f32 = 0.25;
const COCO3_SPACE_INDENT: f32 = 3.00;
const COCO3_FN_GAP: f32 = 0.65;
// The CoCo 1/2 has no diamond to place: its arrows sit at the row ends (see
// the layout table). These are plain alignment values — the number row is
// indented half a cap relative to the rows below it, as on the real machine,
// and BREAK follows the `-` cap after a narrow gap.
const COCO12_BREAK_GAP: f32 = 0.18;
const COCO12_SPACE_INDENT: f32 = 3.20;

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

/// A character cap. An empty `shift` is written as `""` to keep the tables
/// below readable; it is stored as `None`.
const fn key(main: &'static str, shift: &'static str, host: &'static str) -> Slot {
    Slot::Cap(Cap {
        main: Legend::Text(main),
        shift: if shift.is_empty() { None } else { Some(shift) },
        host,
        width: 1.0,
    })
}

/// A cap legended with a word rather than a character; none of them shift.
const fn word(main: &'static str, host: &'static str, width: f32) -> Slot {
    Slot::Cap(Cap {
        main: Legend::Text(main),
        shift: None,
        host,
        width,
    })
}

const fn arrow(dir: Dir, host: &'static str) -> Slot {
    Slot::Cap(Cap {
        main: Legend::Arrow(dir),
        shift: None,
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
const BREAK_CAP: Slot = word("BREAK", "Esc", W_BREAK);

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
/// The CoCo 1/2's ENTER, CLEAR and SHIFT are proportioned differently from
/// the CoCo 3's — its row 3 ends with ENTER *and* CLEAR side by side, and its
/// SHIFTs are visibly wider.
const COCO12_ENTER_CAP: Slot = word("ENTER", "Return", W_COCO12_ENTER);
const COCO12_CLEAR_CAP: Slot = word("CLEAR", CLEAR_HOST, W_COCO12_CLEAR);
const COCO12_SHIFT_CAP: Slot = word("SHIFT", "Shift", W_COCO12_SHIFT);

const COCO3_ROWS: &[Row] = &[
    &[
        &[Slot::Gap(COCO3_ROW1_INDENT)],
        DIGIT_CAPS,
        &[Slot::Gap(COCO3_BREAK_GAP), BREAK_CAP],
    ],
    &[
        &[word("ALT", ALT_HOST, W_MODIFIER)],
        QWERTY_CAPS,
        &[CLEAR_CAP, arrow(Dir::Up, "Up")],
    ],
    &[
        &[
            Slot::Gap(COCO3_ROW3_INDENT),
            word("CTRL", "Ctrl", W_MODIFIER),
        ],
        HOME_CAPS,
        &[
            ENTER_CAP,
            Slot::Gap(COCO3_SIDE_ARROW_GAP),
            arrow(Dir::Left, "Left"),
            arrow(Dir::Right, "Right"),
        ],
    ],
    &[
        &[Slot::Gap(COCO3_ROW4_INDENT), SHIFT_CAP],
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
    &[
        &[Slot::Gap(COCO12_ROW1_INDENT)],
        DIGIT_CAPS,
        &[Slot::Gap(COCO12_BREAK_GAP), BREAK_CAP],
    ],
    &[
        &[arrow(Dir::Up, "Up")],
        QWERTY_CAPS,
        &[arrow(Dir::Left, "Left"), arrow(Dir::Right, "Right")],
    ],
    &[
        &[Slot::Gap(COCO12_ROW3_INDENT), arrow(Dir::Down, "Down")],
        HOME_CAPS,
        &[COCO12_ENTER_CAP, COCO12_CLEAR_CAP],
    ],
    &[
        &[Slot::Gap(COCO12_ROW4_INDENT), COCO12_SHIFT_CAP],
        BOTTOM_CAPS,
        &[COCO12_SHIFT_CAP],
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
    row.iter().copied().flatten()
}

/// How wide `row` is, in key units.
pub(super) fn row_units(row: Row) -> f32 {
    slots(row).map(|slot| slot.units()).sum()
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
