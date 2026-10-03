use coco_core::MachineConfig;

use super::*;
use crate::machine_def;
use crate::machine_def::tests::TempDir;

fn entry(slug: &str, name: &str, created: Option<&str>) -> MachineEntry {
    let def = machine_def::MachineDef::from_config(
        name.to_string(),
        created.map(str::to_string),
        &MachineConfig::default(),
    );
    MachineEntry::new(slug.to_string(), def)
}

fn slugs(entries: &[MachineEntry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.slug.as_str()).collect()
}

#[test]
fn default_order_is_newest_created_first_with_deterministic_ties() {
    let entries = vec![
        entry("zulu", "Zulu", Some("2026-01-01")),
        entry("beta", "Same", Some("2026-02-01")),
        entry("alpha", "Same", Some("2026-02-01")),
        entry("bravo", "Bravo", Some("2026-02-01")),
    ];

    let manager = ManagerApp::new(None, None, None, entries, None);

    assert_eq!(slugs(&manager.entries), ["bravo", "alpha", "beta", "zulu"]);
}

#[test]
fn date_sort_supports_both_directions_and_keeps_bad_dates_last() {
    let mut entries = vec![
        entry("missing", "Missing", None),
        entry("new", "New", Some("2026-02-01")),
        entry("bad", "Bad", Some("February 1")),
        entry("old", "Old", Some("2026-01-01")),
    ];

    sort_entries(&mut entries, ManagerSort::CreatedAsc);
    assert_eq!(slugs(&entries), ["old", "new", "bad", "missing"]);

    sort_entries(&mut entries, ManagerSort::CreatedDesc);
    assert_eq!(slugs(&entries), ["new", "old", "bad", "missing"]);
}

#[test]
fn name_sort_uses_display_name_in_both_directions() {
    let mut entries = vec![
        entry("a-slug", "Zulu", None),
        entry("z-slug", "alpha", None),
        entry("middle", "Bravo", None),
    ];

    sort_entries(&mut entries, ManagerSort::NameAsc);
    assert_eq!(slugs(&entries), ["z-slug", "middle", "a-slug"]);

    sort_entries(&mut entries, ManagerSort::NameDesc);
    assert_eq!(slugs(&entries), ["a-slug", "middle", "z-slug"]);
}

#[test]
fn resort_preserves_selected_slugs_and_anchor() {
    let entries = vec![
        entry("alpha", "Charlie", Some("2026-03-01")),
        entry("bravo", "Alpha", Some("2026-02-01")),
        entry("charlie", "Bravo", Some("2026-01-01")),
    ];
    let mut manager = ManagerApp::new(None, None, None, entries, None);
    manager.selection.set_single(0);
    manager.selection.toggle(2);
    manager.selection.toggle(2);

    manager.apply_manager_sort(ManagerSort::NameAsc);

    let selected: Vec<_> = manager
        .selection
        .iter()
        .map(|index| manager.entries[index].slug.as_str())
        .collect();
    assert_eq!(selected, ["alpha"]);
    let anchor = manager.selection.anchor().expect("selection has an anchor");
    assert_eq!(manager.entries[anchor].slug, "charlie");
}

#[test]
fn create_places_and_selects_the_machine_in_the_active_order() {
    let dir = TempDir::new("manager-sort-create");
    let entries = vec![entry("zulu", "Zulu", None)];
    let mut manager = ManagerApp::new(None, Some(dir.path().to_path_buf()), None, entries, None);
    manager.apply_manager_sort(ManagerSort::NameAsc);

    manager.create_machine_now();

    assert_eq!(manager.entries[0].def.name, "CoCo 3");
    let selected = manager.selection.single().expect("new machine is selected");
    assert_eq!(manager.entries[selected].def.name, "CoCo 3");
}

#[test]
fn rename_repositions_and_keeps_the_machine_selected() {
    let dir = TempDir::new("manager-sort-rename");
    let entries = vec![entry("alpha", "Alpha", None), entry("bravo", "Bravo", None)];
    for entry in &entries {
        machine_def::save(dir.path(), &entry.slug, &entry.def).expect("save definition");
    }
    let mut manager = ManagerApp::new(None, Some(dir.path().to_path_buf()), None, entries, None);
    manager.apply_manager_sort(ManagerSort::NameAsc);
    manager.selection.set_single(0);

    manager.queue_rename("alpha".to_string(), "Zulu".to_string());
    manager.apply_pending_rename();

    assert_eq!(slugs(&manager.entries), ["bravo", "zulu"]);
    let selected = manager
        .selection
        .single()
        .expect("renamed machine is selected");
    assert_eq!(manager.entries[selected].slug, "zulu");
}

#[test]
fn display_name_only_rename_repositions_the_machine() {
    let dir = TempDir::new("manager-sort-name-only-rename");
    let entries = vec![entry("alpha", "Zulu", None), entry("bravo", "Bravo", None)];
    for entry in &entries {
        machine_def::save(dir.path(), &entry.slug, &entry.def).expect("save definition");
    }
    let mut manager = ManagerApp::new(None, Some(dir.path().to_path_buf()), None, entries, None);
    manager.apply_manager_sort(ManagerSort::NameAsc);
    manager.selection.set_single(1);

    manager.queue_rename("alpha".to_string(), "Alpha!".to_string());
    manager.apply_pending_rename();

    assert_eq!(slugs(&manager.entries), ["alpha", "bravo"]);
    let selected = manager
        .selection
        .single()
        .expect("renamed machine is selected");
    assert_eq!(manager.entries[selected].slug, "alpha");
}
