use super::*;

use crate::machine_def::tests::TempDir;
use coco_core::MachineConfig;

const TEST_TEXTURE_BYTES: usize = 2 * 2 * 4;

fn suspended_entry(slug: &str) -> MachineEntry {
    let def = crate::machine_def::MachineDef::from_config(
        slug.to_string(),
        None,
        &MachineConfig::default(),
    );
    let mut entry = MachineEntry::new(slug.to_string(), def);
    entry.suspended = true;
    entry
}

fn write_preview(root: &Path, slug: &str) {
    let pixels = [255; TEST_TEXTURE_BYTES];
    super::super::write_thumbnail_png(&root.join(slug), &pixels, 2, 2).unwrap();
}

fn test_manager(root: &Path, slugs: &[&str]) -> ManagerApp {
    let mut manager = ManagerApp::new(
        None,
        None,
        Some(root.to_path_buf()),
        slugs.iter().map(|slug| suspended_entry(slug)).collect(),
        None,
    );
    for entry in &mut manager.entries {
        entry.suspended = true;
    }
    manager
}

#[test]
fn cache_evicts_oldest_texture_and_reloads_it() {
    let root = TempDir::new("thumbnail-cache-eviction");
    let slugs = ["first", "second", "third"];
    for slug in slugs {
        write_preview(root.path(), slug);
    }
    let mut manager = test_manager(root.path(), &slugs);
    let ctx = egui::Context::default();

    let mut loads_remaining = 3;
    manager.load_thumbnail_range(&ctx, 0..3, 1, &mut loads_remaining);
    manager.enforce_thumbnail_cache_budget(TEST_TEXTURE_BYTES * 2);

    assert!(manager.entries[0].thumbnail.is_none());
    assert!(manager.entries[0].thumbnail_known_available);
    assert!(manager.entries[1].thumbnail.is_some());
    assert!(manager.entries[2].thumbnail.is_some());

    let mut loads_remaining = 1;
    manager.load_thumbnail_range(&ctx, 0..1, 2, &mut loads_remaining);
    manager.enforce_thumbnail_cache_budget(TEST_TEXTURE_BYTES * 2);

    assert!(
        manager.entries[0].thumbnail.is_some(),
        "evicted preview reloads"
    );
    assert!(
        manager.entries[1].thumbnail.is_none(),
        "oldest resident evicts"
    );
    assert!(manager.entries[2].thumbnail.is_some());
}

#[test]
fn missing_thumbnail_is_remembered_but_a_written_one_invalidates_it() {
    let root = TempDir::new("thumbnail-negative-cache");
    let mut manager = test_manager(root.path(), &["missing"]);
    let ctx = egui::Context::default();

    let mut loads_remaining = 1;
    manager.load_thumbnail_range(&ctx, 0..1, 1, &mut loads_remaining);
    assert!(manager.entries[0].thumbnail_load_attempted);
    assert!(!manager.entries[0].thumbnail_known_available);

    write_preview(root.path(), "missing");
    let mut loads_remaining = 1;
    manager.load_thumbnail_range(&ctx, 0..1, 2, &mut loads_remaining);
    assert!(
        manager.entries[0].thumbnail.is_none(),
        "negative cache prevents filesystem polling"
    );

    manager.entries[0].invalidate_thumbnail();
    let mut loads_remaining = 1;
    manager.load_thumbnail_range(&ctx, 0..1, 3, &mut loads_remaining);
    assert!(manager.entries[0].thumbnail.is_some());
}

#[test]
fn oversized_thumbnail_is_negative_cached() {
    let root = TempDir::new("thumbnail-size-limit");
    let slug = "oversized";
    let width = THUMBNAIL_MAX_WIDTH + 1;
    let image = image::RgbaImage::new(width, 1);
    let dir = root.path().join(slug);
    std::fs::create_dir(&dir).unwrap();
    image.save(dir.join(THUMBNAIL_FILE)).unwrap();
    let mut manager = test_manager(root.path(), &[slug]);

    let mut loads_remaining = 1;
    manager.load_thumbnail_range(&egui::Context::default(), 0..1, 1, &mut loads_remaining);

    assert!(manager.entries[0].thumbnail_load_attempted);
    assert!(!manager.entries[0].thumbnail_known_available);
    assert!(manager.entries[0].thumbnail.is_none());
}

#[test]
fn update_load_cap_prioritizes_visible_rows_before_near_rows() {
    let root = TempDir::new("thumbnail-update-cap");
    let slugs: Vec<_> = (0..12).map(|index| format!("entry-{index}")).collect();
    let slug_refs: Vec<_> = slugs.iter().map(String::as_str).collect();
    let mut manager = test_manager(root.path(), &slug_refs);

    manager.prepare_row_thumbnails(&egui::Context::default(), 0..12, 8..12);

    let attempted: Vec<_> = manager
        .entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.thumbnail_load_attempted.then_some(index))
        .collect();
    assert_eq!(attempted, vec![8, 9, 10, 11]);

    manager.prepare_detail_thumbnail(&egui::Context::default(), 0);
    assert!(
        !manager.entries[0].thumbnail_load_attempted,
        "detail and list share the per-update decode budget"
    );
    manager.thumbnail_loads_remaining = THUMBNAIL_LOADS_PER_UPDATE;
    manager.prepare_detail_thumbnail(&egui::Context::default(), 0);
    assert!(manager.entries[0].thumbnail_load_attempted);
}
