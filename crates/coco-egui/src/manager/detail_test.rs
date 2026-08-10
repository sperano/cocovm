use super::*;

#[test]
fn zero_seconds() {
    assert_eq!(humanize_runtime(0), "0 s");
}

#[test]
fn under_a_minute() {
    assert_eq!(humanize_runtime(42), "42 s");
}

#[test]
fn exactly_one_minute() {
    assert_eq!(humanize_runtime(60), "1 m 0 s");
}

#[test]
fn minutes_and_seconds() {
    assert_eq!(humanize_runtime(12 * 60 + 5), "12 m 5 s");
}

#[test]
fn exactly_one_hour() {
    assert_eq!(humanize_runtime(3600), "1 h 0 m");
}

#[test]
fn hours_and_minutes() {
    assert_eq!(humanize_runtime(3 * 3600 + 12 * 60), "3 h 12 m");
}

#[test]
fn exactly_one_day() {
    assert_eq!(humanize_runtime(86_400), "1 d 0 h");
}

#[test]
fn days_and_hours() {
    assert_eq!(humanize_runtime(2 * 86_400 + 3 * 3600), "2 d 3 h");
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

/// Boots a real machine via `crate::launch_machine`, same as
/// `lifecycle_test.rs` — reads the real `roms/coco3.rom` (git-ignored,
/// local-only).
#[test]
fn displayed_runtime_secs_adds_the_live_vms_session_runtime() {
    let def = machine_def::MachineDef::from_config(
        "Runtime Sum Live".to_string(),
        None,
        &coco_core::MachineConfig::default(),
    );
    let mut entry = crate::manager::MachineEntry::new("runtime-sum-live".to_string(), def);
    entry.def.stats.runtime_secs = 100;
    let vm = crate::launch_machine(&entry.def, &entry.slug).expect("a default definition launches");
    entry.vm = Some(Box::new(vm));
    entry.vm.as_mut().unwrap().session_runtime = std::time::Duration::from_secs(30);
    assert_eq!(displayed_runtime_secs(&entry), 130);
}
