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

#[test]
fn first_frame_attempts_only_near_visible_thumbnails() {
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
