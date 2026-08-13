use super::*;

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

/// With a live VM, its own `total_runtime` is the one source of truth —
/// note this def's persisted `runtime_secs` is left at 0 (the default),
/// distinct from the VM's seeded-then-advanced total, and the display must
/// track the latter. Boots a real machine via `crate::launch_machine`, same
/// as `lifecycle_test.rs` — reads the real `roms/coco3.rom` (git-ignored,
/// local-only).
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
