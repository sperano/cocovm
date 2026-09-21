use super::*;

use crate::manager::MachineEntry;
use coco_core::MachineConfig;

const LARGE_LIBRARY_SIZE: usize = 500;

#[test]
fn performance_scroll_offset_includes_inter_row_spacing() {
    const TARGET_ROW: usize = 250;
    const ROW_HEIGHT: f32 = 64.0;
    const ROW_SPACING: f32 = 8.0;

    assert_eq!(
        machine_row_scroll_offset(TARGET_ROW, ROW_HEIGHT, ROW_SPACING),
        TARGET_ROW as f32 * (ROW_HEIGHT + ROW_SPACING)
    );
}

#[test]
fn thumbnail_near_range_expands_and_clamps_visible_rows() {
    const ENTRY_COUNT: usize = 100;

    assert_eq!(thumbnail_near_range(10..15, ENTRY_COUNT), 6..19);
    assert_eq!(thumbnail_near_range(0..3, ENTRY_COUNT), 0..7);
    assert_eq!(thumbnail_near_range(97..100, ENTRY_COUNT), 93..100);
}

#[test]
fn thumbnail_near_range_is_empty_for_an_empty_library() {
    assert_eq!(thumbnail_near_range(0..0, 0), 0..0);
}

/// A manager over [`LARGE_LIBRARY_SIZE`] suspended machines, so a row that
/// gets constructed records a preview attempt.
fn large_library_harness() -> egui_kittest::Harness<'static, ManagerApp> {
    let mut harness = egui_kittest::Harness::new_eframe(|_cc| {
        let mut entries: Vec<_> = (0..LARGE_LIBRARY_SIZE)
            .map(|index| {
                let slug = format!("machine-{index:04}");
                let def = crate::machine_def::MachineDef::from_config(
                    slug.clone(),
                    None,
                    &MachineConfig::default(),
                );
                MachineEntry::new(slug, def)
            })
            .collect();
        for entry in &mut entries {
            entry.suspended = true;
        }
        ManagerApp::new(None, None, None, entries, None)
    });
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness
}

#[test]
fn first_frame_attempts_only_near_visible_thumbnails() {
    let mut harness = large_library_harness();

    harness.step();

    let attempted = harness
        .state()
        .entries
        .iter()
        .filter(|entry| entry.thumbnail_load_attempted)
        .count();
    assert!(attempted > 0, "visible rows attempt their previews");
    assert!(
        attempted < LARGE_LIBRARY_SIZE / 10,
        "attempted {attempted} previews for {LARGE_LIBRARY_SIZE} rows"
    );
}

#[test]
fn machine_row_top_measures_from_the_first_constructed_row() {
    const FIRST_VISIBLE: usize = 10;
    const FIRST_VISIBLE_TOP: f32 = 100.0;
    const STRIDE: f32 = 72.0;

    assert_eq!(
        machine_row_top(12, FIRST_VISIBLE, FIRST_VISIBLE_TOP, STRIDE),
        FIRST_VISIBLE_TOP + 2.0 * STRIDE
    );
    assert_eq!(
        machine_row_top(7, FIRST_VISIBLE, FIRST_VISIBLE_TOP, STRIDE),
        FIRST_VISIBLE_TOP - 3.0 * STRIDE
    );
}

#[test]
fn arrow_keys_walk_the_selection() {
    let mut harness = large_library_harness();
    harness.step();

    harness.key_press(egui::Key::ArrowDown);
    harness.step();
    assert_eq!(harness.state().selection.single(), Some(0));

    harness.key_press(egui::Key::ArrowDown);
    harness.step();
    assert_eq!(harness.state().selection.single(), Some(1));

    harness.key_press(egui::Key::ArrowUp);
    harness.step();
    assert_eq!(harness.state().selection.single(), Some(0));
}

#[test]
fn arrow_keys_scroll_the_selected_row_into_view() {
    const OFFSCREEN_ROW: usize = 60;
    /// Frames for the scroll animation to land after the last key press.
    const SCROLL_SETTLE_FRAMES: usize = 30;

    let mut harness = large_library_harness();
    harness.step();
    assert!(!harness.state().entries[OFFSCREEN_ROW].thumbnail_load_attempted);

    for _ in 0..=OFFSCREEN_ROW {
        harness.key_press(egui::Key::ArrowDown);
        harness.step();
    }
    // Not `run()`: the gamepad service repaints forever.
    harness.run_steps(SCROLL_SETTLE_FRAMES);

    assert_eq!(harness.state().selection.single(), Some(OFFSCREEN_ROW));
    assert!(
        harness.state().entries[OFFSCREEN_ROW].thumbnail_load_attempted,
        "the selected row was constructed, so the list scrolled to it"
    );
}

#[test]
fn shifted_arrow_keys_keep_a_multi_selection() {
    const SELECTED_ROWS: usize = 5;

    let mut harness = large_library_harness();
    harness
        .state_mut()
        .selection
        .select_range(0, SELECTED_ROWS - 1);
    harness.step();

    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowDown);
    harness.step();

    assert_eq!(harness.state().selection.len(), SELECTED_ROWS);
}

#[test]
fn arrow_keys_leave_the_list_alone_under_a_modal() {
    let mut harness = large_library_harness();
    harness.state_mut().pending_delete = vec!["machine-0000".to_string()];
    harness.step();

    harness.key_press(egui::Key::ArrowDown);
    harness.step();

    assert!(harness.state().selection.is_empty());
}
