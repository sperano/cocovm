use super::*;

#[test]
fn empty_paper_has_zero_extent() {
    let paper = Paper::new();
    assert_eq!(paper.extent(), PaperExtent::default());
}

#[test]
fn mark_updates_extent_and_dot_count() {
    let mut paper = Paper::new();
    paper.mark(10, 5);
    paper.mark(20, 5);
    paper.mark(15, 50);
    let extent = paper.extent();
    assert_eq!(extent.max_y, 50);
    assert_eq!(extent.dot_count, 3);
}

#[test]
fn dots_in_range_only_returns_the_requested_band() {
    let mut paper = Paper::new();
    paper.mark(1, 0);
    paper.mark(2, 10);
    paper.mark(3, 20);
    paper.mark(4, 30);
    let mut dots = paper.dots_in_range(10, 20);
    dots.sort();
    assert_eq!(dots, vec![(2, 10), (3, 20)]);
}

#[test]
fn dirty_range_reported_then_cleared_on_take() {
    let mut paper = Paper::new();
    assert_eq!(paper.take_dirty(), None);
    paper.mark(0, 5);
    paper.mark(0, 15);
    assert_eq!(paper.take_dirty(), Some((5, 15)));
    // Second call sees nothing new until another mark happens.
    assert_eq!(paper.take_dirty(), None);
    paper.mark(0, 100);
    assert_eq!(paper.take_dirty(), Some((100, 100)));
}

#[test]
fn clear_empties_dots_but_does_not_touch_future_absolute_y() {
    let mut paper = Paper::new();
    paper.mark(0, 5);
    paper.clear();
    assert_eq!(paper.extent(), PaperExtent::default());
    assert!(paper.dots_in_range(0, 1000).is_empty());
    // A mark at a large absolute y after clear lands exactly there.
    paper.mark(0, 9_000);
    assert_eq!(paper.extent().max_y, 9_000);
}
