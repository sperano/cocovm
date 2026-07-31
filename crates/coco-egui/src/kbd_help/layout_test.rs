//! The key plans are transcribed from Tandy's own documentation (see the
//! module doc), so the tests that matter are the ones that catch a
//! transcription slip: the manuals state exact key counts, and the CoCo 3 is
//! the CoCo 1/2 plus exactly four keys.

use super::*;

const ALL_VARIANTS: [MachineVariant; 3] = [
    MachineVariant::Coco1,
    MachineVariant::Coco2,
    MachineVariant::Coco3,
];

/// Tolerance for comparing positions in key units. Deliberately absolute
/// rather than an ULP count: the sums being compared accumulate through
/// different sequences of the same constants, and anything below a
/// hundredth of a key unit is far under one screen pixel anyway.
const UNIT_TOLERANCE: f32 = 1e-3;

/// Every cap of a layout, in row order.
fn caps(rows: &[Row]) -> Vec<Cap> {
    rows.iter()
        .copied()
        .flat_map(slots)
        .filter_map(|slot| match slot {
            Slot::Cap(cap) => Some(*cap),
            Slot::Gap(_) => None,
        })
        .collect()
}

/// Where `row` centres the arrow pointing `dir`, in key units from the row's
/// left edge.
fn arrow_center(row: Row, dir: Dir) -> f32 {
    let mut x = 0.0;
    for slot in slots(row) {
        if let Slot::Cap(cap) = slot
            && matches!(cap.main, Legend::Arrow(d) if d == dir)
        {
            return x + slot.units() / 2.0;
        }
        x += slot.units();
    }
    panic!("row has no {dir:?} arrow");
}

/// The word printed on a cap, or `None` for the painted arrows.
fn legend(cap: &Cap) -> Option<&'static str> {
    match cap.main {
        Legend::Text(text) => Some(text),
        Legend::Arrow(_) => None,
    }
}

fn legends(rows: &[Row]) -> Vec<&'static str> {
    caps(rows).iter().filter_map(legend).collect()
}

/// Per-row cap counts, which is what pins each key to the right row rather
/// than merely to the layout.
fn row_counts(rows: &[Row]) -> Vec<usize> {
    rows.iter().map(|row| caps(&[*row]).len()).collect()
}

#[test]
fn coco3_matches_the_service_manual_key_count() {
    // "KEYBOARD: 57 keys" — Color Computer 3 Service Manual, §2.1
    // Specifications, and the illustration in Introducing Your Color
    // Computer 3 p.15 breaks those 57 down per row exactly like this.
    assert_eq!(row_counts(COCO3_ROWS), vec![13, 14, 14, 13, 3]);
    assert_eq!(caps(COCO3_ROWS).len(), 57);
}

#[test]
fn coco12_matches_the_service_manual_key_count() {
    // "Keyboard: 53-key microprocessor scanned matrix" — Color Computer 2
    // NTSC Service Manual, §2.3 Technical.
    assert_eq!(row_counts(COCO12_ROWS), vec![13, 13, 13, 13, 1]);
    assert_eq!(caps(COCO12_ROWS).len(), 53);
}

#[test]
fn the_coco3_added_exactly_alt_ctrl_and_the_function_keys() {
    let mut added = legends(COCO3_ROWS);
    for older in legends(COCO12_ROWS) {
        let at = added
            .iter()
            .position(|&newer| newer == older)
            .expect("every CoCo 1/2 key survives on the CoCo 3");
        added.remove(at);
    }
    added.sort_unstable();
    assert_eq!(added, vec!["ALT", "CTRL", "F1", "F2"]);
}

#[test]
fn both_layouts_keep_the_arrow_diamond() {
    // Up closes row 2, left and right close row 3, down closes row 4 — the
    // diamond the CoCo 3 introduced and which the CoCo 1/2 also arranges
    // down the right-hand edge.
    for rows in [COCO3_ROWS, COCO12_ROWS] {
        let arrows: Vec<Dir> = caps(rows)
            .iter()
            .filter_map(|cap| match cap.main {
                Legend::Arrow(dir) => Some(dir),
                Legend::Text(_) => None,
            })
            .collect();
        assert_eq!(arrows, vec![Dir::Up, Dir::Left, Dir::Right, Dir::Down]);
    }
}

#[test]
fn the_arrows_form_a_diamond() {
    // Tandy's illustration puts Up and Down in one column centred between
    // Left and Right. The gap constants that achieve that are fiddly and
    // per-variant, so assert the geometry they produce rather than trusting
    // the numbers.
    for variant in [MachineVariant::Coco2, MachineVariant::Coco3] {
        let rows = rows(variant);
        // Row 2 carries Up, row 3 Left and Right, row 4 Down.
        let up = arrow_center(rows[1], Dir::Up);
        let left = arrow_center(rows[2], Dir::Left);
        let right = arrow_center(rows[2], Dir::Right);
        let down = arrow_center(rows[3], Dir::Down);
        assert!(
            (up - down).abs() < UNIT_TOLERANCE,
            "{variant:?}: up at {up} and down at {down} must share a column"
        );
        let midpoint = (left + right) / 2.0;
        assert!(
            (up - midpoint).abs() < UNIT_TOLERANCE,
            "{variant:?}: up/down at {up} must sit between left ({left}) and right ({right})"
        );
    }
}

#[test]
fn the_keyboard_fits_the_window_it_is_drawn_in() {
    // `width_units` is the max over the rows, so comparing rows against it
    // proves nothing. The falsifiable claim is the absolute one: the widest
    // keyboard has to fit the VM window it is drawn over, whose content is
    // FB_H * SCALE * TARGET_ASPECT wide (`manager::vm_windows`).
    let window_w = coco_core::video::FB_H as f32 * crate::SCALE * crate::TARGET_ASPECT;
    let widest = width_units(MachineVariant::Coco3) * super::super::UNIT_W;
    assert!(
        widest < window_w,
        "the CoCo 3 keyboard is {widest} px wide but the window is only {window_w} px"
    );
    // And the CoCo 1/2's keyboard, having fewer keys, must be the narrower.
    assert!(width_units(MachineVariant::Coco1) < width_units(MachineVariant::Coco3));
}

#[test]
fn every_cap_names_a_host_key() {
    // A blank host line is the bug this window exists to avoid: the user has
    // to be able to read what to press for every single key.
    for variant in ALL_VARIANTS {
        for cap in caps(rows(variant)) {
            assert!(
                !cap.host.trim().is_empty(),
                "{variant:?} has a cap with no host key"
            );
        }
    }
}

#[test]
fn symbolic_mode_keeps_the_host_key_on_exactly_the_positional_keys() {
    // These are the keys `keymap::control_key_pos` routes by position even in
    // symbolic mode, because they produce no text — and so the only ones
    // whose host key is still worth showing there. Getting this set wrong is
    // what would leave a user unable to find BREAK or CLEAR again.
    let mut routed: Vec<&str> = caps(COCO3_ROWS)
        .iter()
        .filter(|cap| cap.routed_in_symbolic())
        .map(|cap| match cap.main {
            Legend::Text(text) => text,
            Legend::Arrow(_) => "arrow",
        })
        .collect();
    routed.sort_unstable();
    assert_eq!(
        routed,
        vec![
            "BREAK", "CLEAR", "ENTER", "F1", "F2", "arrow", "arrow", "arrow", "arrow"
        ]
    );
}

#[test]
fn shifted_legends_follow_the_coco_not_the_host_keyboard() {
    // Spot-check the pairs that differ from a US host keyboard — these are
    // exactly the ones a reader would otherwise assume (Service Manual
    // Figure 5-9): the CoCo shifts 2 to a double quote, : to *, - to =, and
    // ; to +, and its 0 has no shifted legend at all.
    let coco3 = caps(COCO3_ROWS);
    let by_legend = |want: &str| -> Cap {
        *coco3
            .iter()
            .find(|cap| legend(cap) == Some(want))
            .expect("cap present")
    };
    assert_eq!(by_legend("2").shift, Some("\""));
    assert_eq!(by_legend(":").shift, Some("*"));
    assert_eq!(by_legend("-").shift, Some("="));
    assert_eq!(by_legend(";").shift, Some("+"));
    assert_eq!(by_legend("0").shift, None);
    // And the host key for a CoCo cap is its *position*, not its character:
    // CoCo ':' sits where the host's '-' is.
    assert_eq!(by_legend(":").host, "-");
    assert_eq!(by_legend("-").host, "=");
    assert_eq!(by_legend("@").host, "[");
}
