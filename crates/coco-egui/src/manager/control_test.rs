use super::*;
use crate::machine_def;
use crate::{AppParams, CocoApp, MachineConfig, ROMSource};

/// A Powered Off entry: no VM, not suspended.
fn off_entry(slug: &str) -> MachineEntry {
    let def =
        machine_def::MachineDef::from_config(slug.to_string(), None, &MachineConfig::default());
    MachineEntry::new(slug.to_string(), def)
}

/// A Suspended entry with its window already closed (`vm: None`) — the
/// case with nothing live to read, which `resolve_alive` rejects.
fn suspended_closed_entry(slug: &str) -> MachineEntry {
    let mut entry = off_entry(slug);
    entry.suspended = true;
    entry
}

/// A real, Running entry — boots the installed `coco3.rom`, like other test
/// files' `boot()`/`test_app()` helpers.
fn running_entry(slug: &str) -> MachineEntry {
    let rom_path = crate::installed_roms_dir().join(crate::rom_load::COCO3_ROM_FILE);
    let rom = std::fs::read(&rom_path)
        .expect("installed coco3.rom is required (first-run asset download)")
        .into_boxed_slice();
    let vm = CocoApp::new(
        MachineConfig::default(),
        rom,
        ROMSource::File(rom_path),
        AppParams::default(),
        crate::joy::SharedGamepad::without_backend(),
    );
    let mut entry = off_entry(slug);
    entry.vm = Some(Box::new(vm));
    entry
}

fn manager(entries: Vec<MachineEntry>) -> ManagerApp {
    ManagerApp::new(None, None, None, entries, None)
}

#[test]
fn resolve_vm_errs_on_an_unknown_slug() {
    let app = manager(vec![off_entry("known")]);
    let err = app
        .resolve_vm(&Some("ghost".to_string()), false)
        .expect_err("unknown slug must be rejected");
    assert!(err.contains("ghost"));
}

#[test]
fn resolve_vm_errs_when_no_vm_is_running_and_slug_is_omitted() {
    let app = manager(vec![off_entry("a"), suspended_closed_entry("b")]);
    let err = app
        .resolve_vm(&None, false)
        .expect_err("no running VM must be rejected");
    assert!(err.contains("no VM is running"));
}

#[test]
fn resolve_vm_lists_every_running_slug_when_several_and_slug_is_omitted() {
    let app = manager(vec![running_entry("one"), running_entry("two")]);
    let err = app
        .resolve_vm(&None, false)
        .expect_err("ambiguous target must be rejected");
    assert!(err.contains("one"));
    assert!(err.contains("two"));
}

#[test]
fn resolve_vm_finds_the_sole_running_vm_when_slug_is_omitted() {
    let app = manager(vec![off_entry("off"), running_entry("running")]);
    let idx = app
        .resolve_vm(&None, false)
        .expect("sole running VM resolves");
    assert_eq!(app.entries[idx].slug, "running");
}

#[test]
fn resolve_vm_require_running_rejects_a_suspended_named_entry() {
    let app = manager(vec![suspended_closed_entry("frozen")]);
    let err = app
        .resolve_vm(&Some("frozen".to_string()), true)
        .expect_err("a Suspended target must be rejected for a mutating action");
    assert!(err.contains("not running"));
}

#[test]
fn resolve_vm_require_running_rejects_a_powered_off_named_entry() {
    let app = manager(vec![off_entry("cold")]);
    let err = app
        .resolve_vm(&Some("cold".to_string()), true)
        .expect_err("a Powered Off target must be rejected for a mutating action");
    assert!(err.contains("not running"));
}

#[test]
fn resolve_vm_without_require_running_accepts_a_suspended_named_entry() {
    // `start_vm` resolves this way — it must be able to name a Suspended
    // target to bring it up.
    let app = manager(vec![suspended_closed_entry("frozen")]);
    let idx = app
        .resolve_vm(&Some("frozen".to_string()), false)
        .expect("start_vm's own resolution must accept a Suspended target");
    assert_eq!(app.entries[idx].slug, "frozen");
}

#[test]
fn resolve_alive_rejects_a_suspended_but_window_closed_entry_by_slug() {
    // Suspended with the window closed still has `vm: None` — must still be
    // rejected, since there's nothing live to read from.
    let app = manager(vec![suspended_closed_entry("frozen")]);
    let err = app
        .resolve_alive(&Some("frozen".to_string()))
        .expect_err("a window-closed Suspended entry has no live VM to read");
    assert!(err.contains("not running"));
}

#[test]
fn resolve_alive_accepts_a_running_entry() {
    let app = manager(vec![running_entry("live")]);
    let idx = app
        .resolve_alive(&Some("live".to_string()))
        .expect("a Running entry is alive");
    assert_eq!(app.entries[idx].slug, "live");
}

#[test]
fn vm_infos_reports_each_entrys_wire_status() {
    let app = manager(vec![
        off_entry("off"),
        suspended_closed_entry("frozen"),
        running_entry("live"),
    ]);
    let infos = app.vm_infos();
    let status = |slug: &str| infos.iter().find(|i| i.slug == slug).map(|i| i.status);
    assert_eq!(status("off"), Some(crate::control::VmStatus::PoweredOff));
    assert_eq!(status("frozen"), Some(crate::control::VmStatus::Suspended));
    assert_eq!(status("live"), Some(crate::control::VmStatus::Running));
}

#[test]
fn bind_control_returns_none_when_the_port_is_zero() {
    let ctx = egui::Context::default();
    assert!(bind_control(0, &ctx).is_none());
}
