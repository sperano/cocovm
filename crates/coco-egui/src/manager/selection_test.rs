use coco_core::MachineConfig;

use super::*;
use crate::machine_def;

/// A minimal valid entry, just enough for [`Selection::snapshot`]/
/// [`Selection::restore`] to have a slug to key on.
fn entry(slug: &str) -> MachineEntry {
    MachineEntry::new(
        slug.to_string(),
        machine_def::MachineDef::from_config(slug.to_string(), None, &MachineConfig::default()),
    )
}

#[test]
fn toggle_adds_then_removes_and_always_becomes_the_anchor() {
    let mut sel = Selection::default();
    sel.toggle(2);
    assert!(sel.contains(2));
    assert_eq!(sel.anchor(), Some(2));

    sel.toggle(5);
    assert_eq!(sel.len(), 2);
    assert_eq!(
        sel.anchor(),
        Some(5),
        "the just-toggled row becomes the anchor even when added"
    );

    sel.toggle(2);
    assert!(!sel.contains(2));
    assert_eq!(sel.len(), 1);
    assert_eq!(sel.anchor(), Some(2), "…and even when removed");

    sel.toggle(5);
    assert!(
        sel.is_empty(),
        "toggling the last row off empties the selection"
    );
    assert_eq!(sel.anchor(), None, "an empty selection has no anchor");
}

#[test]
fn select_range_is_inclusive_in_either_direction_and_keeps_the_anchor() {
    let mut sel = Selection::default();
    sel.select_range(1, 4);
    assert_eq!(sel.iter().collect::<Vec<_>>(), vec![1, 2, 3, 4]);
    assert_eq!(sel.anchor(), Some(1));

    // Same anchor, extended downward past it — a second Shift-click must
    // measure from the same start.
    sel.select_range(1, 0);
    assert_eq!(sel.iter().collect::<Vec<_>>(), vec![0, 1]);
    assert_eq!(
        sel.anchor(),
        Some(1),
        "the anchor itself never moves for a range select"
    );
}

/// A range whose anchor sits past every index the caller cares about must
/// still compute a plain inclusive span — `Selection` has no notion of list
/// bounds to clamp against.
#[test]
fn select_range_with_anchor_beyond_the_clicked_row() {
    let mut sel = Selection::default();
    sel.select_range(10, 1);
    assert_eq!(sel.iter().collect::<Vec<_>>(), (1..=10).collect::<Vec<_>>());
    assert_eq!(sel.anchor(), Some(10));
}

#[test]
fn remove_index_shifts_later_rows_down_and_drops_the_removed_one() {
    let mut sel = Selection::default();
    sel.select_range(1, 5); // {1, 2, 3, 4, 5}, anchor 1
    sel.remove_index(3);
    assert_eq!(
        sel.iter().collect::<Vec<_>>(),
        vec![1, 2, 3, 4],
        "5 shifts down to 4, 3 drops out"
    );
    assert_eq!(
        sel.anchor(),
        Some(1),
        "anchor before the removed index is untouched"
    );
}

#[test]
fn remove_index_clears_the_anchor_when_the_anchor_row_is_removed() {
    let mut sel = Selection::default();
    sel.set_single(2);
    sel.remove_index(2);
    assert!(sel.is_empty());
    assert_eq!(sel.anchor(), None);
}

#[test]
fn remove_index_shifts_an_anchor_past_the_removed_row() {
    let mut sel = Selection::default();
    sel.toggle(4);
    assert_eq!(sel.anchor(), Some(4));
    sel.remove_index(1);
    assert!(sel.contains(3), "row 4 becomes row 3 once index 1 is gone");
    assert_eq!(sel.anchor(), Some(3));
}

/// Regression: an anchor can legally sit on a row that isn't itself
/// selected. If the last actually-selected row is removed, `remove_index`
/// must reconcile the anchor against the now-empty `rows`.
#[test]
fn remove_index_drops_a_stale_anchor_left_on_a_deselected_row() {
    let mut sel = Selection::default();
    sel.toggle(2); // rows={2}, anchor=2
    sel.toggle(5); // rows={2,5}, anchor=5
    sel.toggle(2); // rows={5}, anchor=2 (anchor stays on the just-deselected row)
    assert_eq!(sel.iter().collect::<Vec<_>>(), vec![5]);
    assert_eq!(sel.anchor(), Some(2));

    sel.remove_index(5);
    assert!(sel.is_empty(), "row 5 was the only selected row");
    assert_eq!(
        sel.anchor(),
        None,
        "the anchor must not outlive an emptied selection, even though removing index \
         5 never touched the anchor's own index (2) directly"
    );
}

/// [`restore`]'s own symmetry case: the restored selection can come back
/// empty while a stale anchor resolves to a real index. Same fix as
/// `remove_index`'s.
#[test]
fn restore_drops_a_stale_anchor_when_the_restored_selection_is_empty() {
    let before = vec![entry("alpha"), entry("bravo")];
    let mut sel = Selection::default();
    sel.toggle(0); // alpha selected
    sel.toggle(1); // bravo selected, becomes the anchor
    sel.toggle(1); // bravo deselected; anchor stays on it (rows={alpha} isn't empty)
    let snapshot = sel.snapshot(&before);

    let after = vec![entry("bravo")]; // alpha — the only selected row — is gone
    sel.restore(&after, &snapshot);
    assert!(sel.is_empty());
    assert_eq!(
        sel.anchor(),
        None,
        "the anchor must not survive a restore that empties the selection"
    );
}

#[test]
fn snapshot_and_restore_follow_a_reorder_by_slug() {
    let before = vec![entry("charlie"), entry("alpha"), entry("bravo")];
    let mut sel = Selection::default();
    sel.toggle(0); // charlie
    sel.toggle(2); // bravo — becomes the anchor
    let snapshot = sel.snapshot(&before);

    // Simulate the alphabetical re-sort a rename triggers.
    let mut after = before;
    after.sort_by(|a, b| a.slug.cmp(&b.slug));
    assert_eq!(
        after.iter().map(|e| e.slug.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "bravo", "charlie"]
    );

    sel.restore(&after, &snapshot);
    assert_eq!(
        sel.iter().collect::<Vec<_>>(),
        vec![1, 2],
        "bravo=1, charlie=2 in the new order"
    );
    assert_eq!(sel.anchor(), Some(1), "bravo (the anchor) is now index 1");
}

#[test]
fn restore_drops_slugs_no_longer_present() {
    let before = vec![entry("alpha"), entry("bravo")];
    let mut sel = Selection::default();
    sel.select_range(0, 1);
    let snapshot = sel.snapshot(&before);

    let after = vec![entry("bravo")]; // "alpha" gone
    sel.restore(&after, &snapshot);
    assert_eq!(
        sel.iter().collect::<Vec<_>>(),
        vec![0],
        "only bravo survives, now at index 0"
    );
}

#[test]
fn select_all_covers_every_row_and_anchors_the_last() {
    let mut sel = Selection::default();
    sel.select_all(3);
    assert_eq!(sel.iter().collect::<Vec<_>>(), vec![0, 1, 2]);
    assert_eq!(sel.anchor(), Some(2));

    sel.select_all(0);
    assert!(
        sel.is_empty(),
        "selecting all of an empty list selects nothing"
    );
    assert_eq!(sel.anchor(), None);
}

#[test]
fn single_is_some_only_for_exactly_one_row() {
    let mut sel = Selection::default();
    assert_eq!(sel.single(), None);
    sel.set_single(3);
    assert_eq!(sel.single(), Some(3));
    sel.toggle(4);
    assert_eq!(
        sel.single(),
        None,
        "two selected rows is not a single selection"
    );
}
