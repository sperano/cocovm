use super::*;

use std::fs;

use crate::machine_def::tests::TempDir;

const RENAME_JOURNAL_FILE_NAME: &str = ".rename-journal.toml";

fn rename_fixture(name: &str) -> (TempDir, ManagerApp, EditState) {
    let machines_dir = TempDir::new("detail-name-commit");
    let def = machine_def::MachineDef::from_config(
        name.to_string(),
        None,
        &coco_core::MachineConfig::default(),
    );
    machine_def::save(machines_dir.path(), "alpha", &def).expect("seed definition");
    let manager = ManagerApp::new(
        None,
        Some(machines_dir.path().to_path_buf()),
        None,
        vec![crate::manager::MachineEntry::new(
            "alpha".to_string(),
            def.clone(),
        )],
        None,
    );
    let mut form = detail_map::seed_form(&def);
    let packed = manager
        .pack_def(&def, "alpha", &mut form)
        .expect("seeded form packs");
    let edit = EditState {
        slug: "alpha".to_string(),
        name: name.to_string(),
        form,
        roms: Vec::new(),
        packed,
    };
    (machines_dir, manager, edit)
}

#[test]
fn committing_an_unchanged_name_does_not_write_or_queue_a_rename() {
    let (machines_dir, mut manager, mut edit) = rename_fixture("Alpha");
    let config_path = machines_dir.path().join("alpha.toml");
    let mut original = fs::read_to_string(&config_path).expect("read definition");
    original.push_str("\n# unchanged-name sentinel\n");
    fs::write(&config_path, &original).expect("write sentinel");

    manager.commit_name(0, &mut edit);

    assert_eq!(fs::read_to_string(config_path).unwrap(), original);
    assert!(manager.pending_rename.is_none());
}

#[test]
fn committing_a_new_name_with_the_same_slug_only_saves_the_name() {
    let (machines_dir, mut manager, mut edit) = rename_fixture("Alpha");
    edit.name = "ALPHA!".to_string();

    manager.commit_name(0, &mut edit);

    assert_eq!(manager.entries[0].slug, "alpha");
    assert_eq!(manager.entries[0].def.name, "ALPHA!");
    assert!(manager.pending_rename.is_none());
    assert!(machines_dir.path().join("alpha.toml").is_file());
    assert!(!machines_dir.path().join(RENAME_JOURNAL_FILE_NAME).exists());
}

#[test]
fn collision_resolution_back_to_the_current_slug_does_not_rename() {
    let machines_dir = TempDir::new("detail-name-collision");
    let alpha = machine_def::MachineDef::from_config(
        "Alpha Existing".to_string(),
        None,
        &coco_core::MachineConfig::default(),
    );
    let alpha_two = machine_def::MachineDef::from_config(
        "Alpha Two".to_string(),
        None,
        &coco_core::MachineConfig::default(),
    );
    machine_def::save(machines_dir.path(), "alpha", &alpha).expect("seed alpha");
    machine_def::save(machines_dir.path(), "alpha-2", &alpha_two).expect("seed alpha-2");
    let mut manager = ManagerApp::new(
        None,
        Some(machines_dir.path().to_path_buf()),
        None,
        vec![
            crate::manager::MachineEntry::new("alpha".to_string(), alpha),
            crate::manager::MachineEntry::new("alpha-2".to_string(), alpha_two.clone()),
        ],
        None,
    );
    let mut form = detail_map::seed_form(&alpha_two);
    let packed = manager
        .pack_def(&alpha_two, "alpha-2", &mut form)
        .expect("seeded form packs");
    let mut edit = EditState {
        slug: "alpha-2".to_string(),
        name: "Alpha".to_string(),
        form,
        roms: Vec::new(),
        packed,
    };

    manager.commit_name(1, &mut edit);

    assert_eq!(manager.entries[1].slug, "alpha-2");
    assert_eq!(manager.entries[1].def.name, "Alpha");
    assert!(manager.pending_rename.is_none());
    assert!(machines_dir.path().join("alpha-2.toml").is_file());
    assert!(!machines_dir.path().join(RENAME_JOURNAL_FILE_NAME).exists());
}

#[test]
fn started_label_is_singular_for_one() {
    assert_eq!(started_label(1), "1 time");
}

#[test]
fn started_label_is_plural_otherwise() {
    assert_eq!(started_label(0), "0 times");
    assert_eq!(started_label(2), "2 times");
}

#[test]
fn displayed_runtime_secs_is_persisted_total_with_no_live_vm() {
    let def = machine_def::MachineDef::from_config(
        "Runtime Sum".to_string(),
        None,
        &coco_core::MachineConfig::default(),
    );
    let mut entry = crate::manager::MachineEntry::new("runtime-sum".to_string(), def);
    entry.def.stats.runtime_secs = 100;
    assert_eq!(displayed_runtime_secs(&entry), 100);
}

/// With a live VM, its own `total_runtime` is the source of truth — the
/// persisted `runtime_secs` stays at 0 while the display tracks the VM's
/// seeded-then-advanced total.
#[test]
fn displayed_runtime_secs_reads_the_live_vms_total_runtime() {
    let def = machine_def::MachineDef::from_config(
        "Runtime Sum Live".to_string(),
        None,
        &coco_core::MachineConfig::default(),
    );
    let mut entry = crate::manager::MachineEntry::new("runtime-sum-live".to_string(), def);
    let vm = crate::launch_machine(&entry.def, &entry.slug).expect("a default definition launches");
    entry.vm = Some(Box::new(vm));
    entry.vm.as_mut().unwrap().total_runtime = std::time::Duration::from_secs(130);
    assert_eq!(displayed_runtime_secs(&entry), 130);
}
