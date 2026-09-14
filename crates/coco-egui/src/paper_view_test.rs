use super::*;
use crate::paper_render::PAGE_HEIGHT_IN;
use coco_core::bitbanger::PrinterSink;
use coco_core::printer::Y_UNITS_PER_INCH;

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
    // A single dot mid-way down page 3 (0-indexed page 2): total_pages must be 2 + 2 = 4.
    let extent = PaperExtent {
        max_y: (2.4 * PAGE_HEIGHT_IN * Y_UNITS_PER_INCH as f32) as u32,
        dot_count: 1,
    };
    assert_eq!(PaperWindow::total_pages_for_extent(extent), 4);
}

/// Tearing off must discard the printed roll and reset every bit of this window's own view
/// state — a stale `current_page` or cached texture would otherwise point past the now-empty
/// roll. Tests state directly rather than driving a real GUI frame.
#[test]
fn tear_off_resets_paper_extent_and_view_state() {
    let mut window = PaperWindow::new();
    let mut handle = DmpHandle::new();
    // Print enough real ink (not line feeds, which mark no dots) to have state to reset
    // away from.
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
    // Torn off, still attached and open: the next frame must see the fresh-roll page count.
    assert_eq!(
        PaperWindow::total_pages_for_extent(handle.paper_extent()),
        2
    );
}

#[test]
fn detach_also_resets_current_page_and_pending_tear_off() {
    let mut window = PaperWindow::new();
    window.handle = Some(DmpHandle::new());
    window.open = true;
    window.current_page = 5;
    window.pending_tear_off = true;

    window.detach();

    assert!(window.handle.is_none());
    assert!(!window.open);
    assert_eq!(window.current_page, 0);
    assert!(!window.pending_tear_off);
}

/// The scroll area starts at the roll's top on the frame the window opens or reopens, and
/// only on that frame — later frames leave egui's stick-to-bottom in charge.
#[test]
fn window_asks_for_the_top_only_on_the_frame_it_opens() {
    let mut window = PaperWindow::new();
    assert!(!window.note_shown(false), "hidden: nothing opened");
    assert!(
        window.note_shown(true),
        "first shown frame is the opening frame"
    );
    assert!(!window.note_shown(true), "still open: no new edge");
    assert!(!window.note_shown(false), "closing is not an opening edge");
    assert!(
        window.note_shown(true),
        "reopening is an opening edge again"
    );
}
