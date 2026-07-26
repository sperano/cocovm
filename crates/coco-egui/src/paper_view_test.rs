use super::*;
use coco_core::bitbanger::PrinterSink;

#[test]
fn total_pages_for_extent_is_one_blank_page_when_empty() {
    assert_eq!(
        PaperWindow::total_pages_for_extent(PaperExtent::default()),
        1 + 1,
        "one page shown, plus one trailing blank page"
    );
}

#[test]
fn total_pages_for_extent_always_counts_one_trailing_blank_page() {
    // A single dot near the top of page 3 (0-indexed page 2): content
    // reaches page index 2, so total_pages must be 2 + 2 = 4 (pages
    // 0..=2 have/may-have content, page 3 is the trailing blank one).
    let extent = PaperExtent {
        max_y: (2.5 * PAGE_HEIGHT_IN * Y_UNITS_PER_INCH as f32) as u32,
        dot_count: 1,
    };
    assert_eq!(PaperWindow::total_pages_for_extent(extent), 4);
}

/// Tearing off must both discard the printed roll (the underlying
/// [`Dmp105Handle`]'s extent resets to empty) and reset every bit of
/// this window's own view state that referred to the old roll's
/// content — a stale `current_page` or cached texture would otherwise
/// point past the now-empty roll on the very next `ui()` frame. Tests
/// the state directly rather than driving a real `egui::Context`/GUI
/// frame (T6 acceptance: "unit-test the state, not the GUI").
#[test]
fn tear_off_resets_paper_extent_and_view_state() {
    let mut window = PaperWindow::new();
    let mut handle = Dmp105Handle::new();
    // Print enough real ink (not just bare line feeds, which move the
    // head but mark no dots) to have non-default extent/cache state to
    // reset away from.
    for _ in 0..80 {
        for &b in b"HELLO WORLD\r" {
            handle.write_byte(b);
        }
    }
    assert!(handle.paper_extent().dot_count > 0);

    window.handle = Some(handle.clone());
    window.open = true;
    window.current_page = 3;
    window.pending_tear_off = true;

    window.perform_tear_off();

    assert_eq!(handle.paper_extent(), PaperExtent::default());
    assert_eq!(window.current_page, 0);
    assert!(!window.pending_tear_off);
    assert!(window.pages.is_empty());
    // Torn off, still attached and open: the next `ui()` frame must see
    // exactly the fresh-roll page count (1 blank page + 1 trailing).
    assert_eq!(
        PaperWindow::total_pages_for_extent(handle.paper_extent()),
        2
    );
}

#[test]
fn detach_also_resets_current_page_and_pending_tear_off() {
    let mut window = PaperWindow::new();
    window.handle = Some(Dmp105Handle::new());
    window.open = true;
    window.current_page = 5;
    window.pending_tear_off = true;

    window.detach();

    assert!(window.handle.is_none());
    assert!(!window.open);
    assert_eq!(window.current_page, 0);
    assert!(!window.pending_tear_off);
}
