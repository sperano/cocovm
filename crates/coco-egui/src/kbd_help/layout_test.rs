//! The key plans are transcribed from Tandy's own documentation (see the
//! module doc), so the tests that matter are the ones that catch a
//! transcription slip: the manuals state exact key counts, and the CoCo 3 is
//! the CoCo 1/2 plus exactly four keys.

use super::*;

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
    // "KEYBOARD: 57 keys" — Color Computer 3 Service Manual §2.1, broken down per row per the p.15 illustration.
    assert_eq!(row_counts(COCO3_ROWS), vec![13, 14, 14, 13, 3]);
    assert_eq!(caps(COCO3_ROWS).len(), 57);
}

#[test]
fn coco12_matches_the_service_manual_key_count() {
    // "Keyboard: 53-key microprocessor scanned matrix" — Color Computer 2 NTSC Service Manual §2.3.
    assert_eq!(row_counts(COCO12_ROWS), vec![13, 14, 13, 12, 1]);
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

/// The legend of the cap at `index` in `row`, for pinning a key to a slot.
fn cap_at(row: Row, index: usize) -> Cap {
    caps(&[row])[index]
}

fn is_arrow(cap: &Cap, dir: Dir) -> bool {
    matches!(cap.main, Legend::Arrow(d) if d == dir)
}

#[test]
fn the_coco3_arranges_its_arrows_in_a_diamond() {
    // Gap constants that place the diamond are fiddly, so assert the geometry rather than the numbers.
    let rows = rows(MachineVariant::Coco3);
    let up = arrow_center(rows[1], Dir::Up);
    let left = arrow_center(rows[2], Dir::Left);
    let right = arrow_center(rows[2], Dir::Right);
    let down = arrow_center(rows[3], Dir::Down);
    assert!(
        (up - down).abs() < UNIT_TOLERANCE,
        "up at {up} and down at {down} must share a column"
    );
    let midpoint = (left + right) / 2.0;
    assert!(
        (up - midpoint).abs() < UNIT_TOLERANCE,
        "up/down at {up} must sit between left ({left}) and right ({right})"
    );
}

#[test]
fn the_coco12_puts_its_arrows_at_the_row_ends_not_in_a_diamond() {
    // Placement the 53-key count alone can't pin down — the bug this test exists to catch.
    let rows = rows(MachineVariant::Coco2);
    assert!(is_arrow(&cap_at(rows[1], 0), Dir::Up), "Up opens row 2");
    assert!(is_arrow(&cap_at(rows[2], 0), Dir::Down), "Down opens row 3");

    let row2 = caps(&[rows[1]]);
    assert!(
        is_arrow(&row2[row2.len() - 2], Dir::Left) && is_arrow(&row2[row2.len() - 1], Dir::Right),
        "Left and Right close row 2, after @"
    );

    // ENTER/CLEAR share row 3's end, CLEAR outermost — CoCo 3 instead splits them across rows 2 and 3.
    let row3 = caps(&[rows[2]]);
    assert_eq!(legend(&row3[row3.len() - 2]), Some("ENTER"));
    assert_eq!(legend(&row3[row3.len() - 1]), Some("CLEAR"));

    // And no arrow anywhere near the CoCo 3's diamond column.
    for (index, row) in rows.iter().enumerate() {
        let arrows = caps(&[row])
            .iter()
            .filter(|cap| matches!(cap.main, Legend::Arrow(_)))
            .count();
        let expected = match index {
            1 => 3, // Up, Left, Right
            2 => 1, // Down
            _ => 0,
        };
        assert_eq!(arrows, expected, "row {index} arrow count");
    }
}

#[test]
fn the_keyboard_fits_the_window_it_is_drawn_in() {
    // The falsifiable claim: the widest keyboard must fit the VM window, FB_H * SCALE * TARGET_ASPECT wide.
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
    // A blank host line is the bug this window exists to avoid.
    for variant in MachineVariant::ALL {
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
    // Keys `control_key_pos` routes by position even in symbolic mode — getting this set wrong strands BREAK/CLEAR.
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
    // Spot-check pairs that differ from a US host keyboard (Service Manual Fig. 5-9).
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
    // Host key is the CoCo cap's position, not its character: CoCo ':' sits where the host's '-' is.
    assert_eq!(by_legend(":").host, "-");
    assert_eq!(by_legend("-").host, "=");
    assert_eq!(by_legend("@").host, "[");
}
