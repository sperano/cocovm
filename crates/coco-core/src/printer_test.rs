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
fn visit_dots_in_range_matches_the_collecting_adapter() {
    let mut paper = Paper::new();
    paper.mark(1, 0);
    paper.mark(2, 10);
    paper.mark(3, 20);
    paper.mark(4, 30);
    let expected = paper.dots_in_range(10, 20);
    let mut visited = Vec::new();

    paper.visit_dots_in_range(10, 20, |x, y| visited.push((x, y)));

    assert_eq!(visited, expected);
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

#[test]
fn owned_byte_estimate_accounts_for_rows_and_dot_capacity() {
    let mut paper = Paper::new();
    let empty_bytes = paper.estimated_owned_bytes();
    paper.mark(1, 10);
    let one_row_bytes = paper.estimated_owned_bytes();
    paper.mark(2, 20);
    let two_row_bytes = paper.estimated_owned_bytes();

    assert!(one_row_bytes > empty_bytes);
    assert!(two_row_bytes > one_row_bytes);
    assert!(
        two_row_bytes - one_row_bytes >= BTREE_ROW_ALLOCATION_OVERHEAD_ESTIMATE,
        "each allocated row must include the conservative B-tree allowance"
    );
}

#[test]
fn printer_capture_preserves_order_normalizes_cr_and_decodes_lossily() {
    let mut capture = PrinterCapture::default();
    for byte in [b'A', b'\r', 0xFF, b'B'] {
        capture.push(byte);
    }

    assert_eq!(
        capture.snapshot(),
        PrinterCaptureSnapshot {
            text: "A\n\u{fffd}B".to_string(),
            truncated: false,
        }
    );
}

#[test]
fn printer_capture_retains_newest_bytes_and_reports_truncation() {
    let mut capture = PrinterCapture::default();
    for byte in 0..=PRINTER_CAPTURE_CAPACITY {
        capture.push((byte % 251) as u8);
    }

    let expected: Vec<u8> = (1..=PRINTER_CAPTURE_CAPACITY)
        .map(|byte| (byte % 251) as u8)
        .map(|byte| if byte == b'\r' { b'\n' } else { byte })
        .collect();
    let snapshot = capture.snapshot();
    assert_eq!(snapshot.text, String::from_utf8_lossy(&expected));
    assert!(snapshot.truncated);
}

#[test]
fn clearing_printer_capture_resets_text_and_truncation() {
    let mut capture = PrinterCapture::default();
    for _ in 0..=PRINTER_CAPTURE_CAPACITY {
        capture.push(b'X');
    }
    capture.clear();

    assert_eq!(
        capture.snapshot(),
        PrinterCaptureSnapshot {
            text: String::new(),
            truncated: false,
        }
    );
}
