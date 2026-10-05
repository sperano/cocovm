//! `dispatch_control` coverage. Most of its logic (VM resolution, disk/reset/
//! joystick effects) is exercised through the private per-action helpers
//! directly — no networking needed. `list_vms_dispatches_through_dispatch_control`
//! drives `dispatch_control` itself through a hand-built `Incoming`
//! (`crate::control::Incoming::new`), proving that wiring works; the real
//! HTTP transport in front of it is covered end to end in
//! `crate::control::control_test`.

use std::sync::mpsc;

use crate::control::{Action, Incoming, Reply, ReplyHandle, Request, Response};

use super::*;
use crate::machine_def;
use crate::manager::MachineEntry;
use crate::{AppParams, CocoApp, MachineConfig, ROMSource};

fn off_entry(slug: &str) -> MachineEntry {
    let def =
        machine_def::MachineDef::from_config(slug.to_string(), None, &MachineConfig::default());
    MachineEntry::new(slug.to_string(), def)
}

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

/// Prove `list_vms` round-trips through `dispatch_control` itself, using a
/// hand-built `Incoming` in place of a real HTTP connection.
#[test]
fn list_vms_dispatches_through_dispatch_control() {
    let mut manager = manager(vec![off_entry("solo")]);
    let (tx, rx) = mpsc::channel();
    let incoming = Incoming::new(
        Request {
            vm: None,
            action: Action::ListVms,
        },
        ReplyHandle::new(tx),
    );

    manager.dispatch_control(incoming);

    let Response::Ok(Reply::Vms(infos)) = rx.recv().expect("reply sent") else {
        panic!("expected Ok(Reply::Vms)");
    };
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].slug, "solo");
}

#[test]
fn start_vm_action_is_a_no_op_when_already_running() {
    let mut app = manager(vec![running_entry("live")]);
    let reply = app
        .start_vm_action(&Some("live".to_string()))
        .expect("already-running start_vm succeeds");
    assert_eq!(reply, Reply::Done);
}

#[test]
fn start_vm_action_powers_on_a_stopped_machine() {
    let mut app = manager(vec![off_entry("cold")]);
    let reply = app
        .start_vm_action(&Some("cold".to_string()))
        .expect("start_vm should launch a Powered Off machine");
    assert_eq!(reply, Reply::Done);
    assert!(app.entries[0].vm.is_some(), "the VM must now be running");
}

#[test]
fn insert_disk_action_rejects_a_drive_beyond_ui_drives() {
    let mut app = manager(vec![running_entry("live")]);
    let err = app
        .insert_disk_action(
            &Some("live".to_string()),
            crate::UI_DRIVES,
            "x.dsk".to_string(),
        )
        .expect_err("an out-of-range drive must be rejected");
    assert!(err.contains("drive"));
}

#[test]
fn insert_disk_action_reports_a_missing_fd502() {
    let mut app = manager(vec![running_entry("live")]);
    // No FD-502 was mounted at launch (bare `AppParams::default()`), so
    // insertion must fail through the same `cart_error` path the UI uses.
    let err = app
        .insert_disk_action(
            &Some("live".to_string()),
            0,
            "/no/such/disk.dsk".to_string(),
        )
        .expect_err("insert without a controller must fail");
    assert!(!err.is_empty());
}

#[test]
fn insert_disk_action_reports_a_new_failure_despite_a_stale_cart_error() {
    const STALE: &str = "stale error from an earlier UI action";
    let mut app = manager(vec![running_entry("live")]);
    app.entries[0].vm.as_mut().unwrap().cart_error = Some(STALE.to_string());
    let err = app
        .insert_disk_action(
            &Some("live".to_string()),
            0,
            "/no/such/disk.dsk".to_string(),
        )
        .expect_err("a fresh failure must surface even with a stale error present");
    assert_ne!(err, STALE);
}

#[test]
fn eject_disk_action_keeps_a_stale_cart_error_when_it_succeeds() {
    const STALE: &str = "stale error from an earlier UI action";
    let mut app = manager(vec![running_entry("live")]);
    app.entries[0].vm.as_mut().unwrap().cart_error = Some(STALE.to_string());
    // Ejecting an empty drive with no controller is a no-op, not an error.
    app.eject_disk_action(&Some("live".to_string()), 0)
        .expect("ejecting an empty drive succeeds");
    assert_eq!(
        app.entries[0].vm.as_ref().unwrap().cart_error.as_deref(),
        Some(STALE)
    );
}

#[test]
fn deferred_capacity_is_reserved_before_vm_mutation() {
    let mut app = manager(vec![running_entry("live")]);
    for _ in 0..super::super::MAX_PENDING_CONTROL_REQUESTS {
        let (tx, _rx) = mpsc::channel();
        app.pending.push(PendingControl::new(
            ReplyHandle::new(tx),
            "live".to_string(),
            PendingCondition::TypeTextDrained,
            0,
            60.0,
        ));
    }
    let (tx, rx) = mpsc::channel();

    app.start_deferred(ReplyHandle::new(tx), Some("live".to_string()), |vm| {
        let fields = vm.start_remote_typing("A")?;
        Ok((PendingCondition::TypeTextDrained, fields))
    });

    let Response::Err(message) = rx.recv().expect("overload reply") else {
        panic!("expected overload error");
    };
    assert!(message.message.contains("too many deferred"));
    assert!(
        !app.entries[0]
            .vm
            .as_ref()
            .unwrap()
            .remote_type_ahead
            .is_active()
    );
}

/// Dispatch `action` on `vm` through `dispatch_control` and return the
/// receiver its reply lands on.
fn dispatch(manager: &mut ManagerApp, vm: &str, action: Action) -> mpsc::Receiver<Response> {
    let (tx, rx) = mpsc::channel();
    let request = Request {
        vm: Some(vm.to_string()),
        action,
    };
    manager.dispatch_control(Incoming::new(request, ReplyHandle::new(tx)));
    rx
}

#[test]
fn wait_rejects_a_paused_vm_up_front() {
    let mut app = manager(vec![running_entry("live")]);
    app.entries[0].vm.as_mut().unwrap().set_running(false);

    let rx = dispatch(&mut app, "live", Action::Wait { fields: 1 });

    assert_eq!(
        rx.try_recv().expect("immediate reply"),
        Response::Err(crate::app::PAUSED_ERROR.into())
    );
    assert!(app.pending.is_empty());
}

#[test]
fn pending_wait_fails_in_the_update_that_pauses_its_vm() {
    let mut app = manager(vec![running_entry("live")]);
    let wait = dispatch(&mut app, "live", Action::Wait { fields: 1 });
    assert_eq!(app.pending.len(), 1, "a running VM defers the wait");

    let pause = dispatch(&mut app, "live", Action::SetRunning { running: false });
    assert_eq!(
        pause.try_recv().expect("immediate reply"),
        Response::Ok(Reply::Done)
    );
    app.resolve_control_pending(&eframe::egui::Context::default());

    match wait.try_recv().expect("wait replied without timing out") {
        Response::Err(error) => assert!(error.message.contains("was paused"), "{error}"),
        other => panic!("expected an Err reply, got {other:?}"),
    }
    assert!(app.pending.is_empty());
}

#[test]
fn wait_for_text_matches_immediately_on_a_paused_live_vm() {
    let mut app = manager(vec![running_entry("live")]);
    let pattern = app.entries[0].vm.as_mut().unwrap().screen_snapshot().lines[0].clone();
    app.entries[0].vm.as_mut().unwrap().running = false;
    let matcher = crate::control::TextMatcher::new(pattern, false).unwrap();
    let (tx, rx) = mpsc::channel();

    app.start_wait_for_text(ReplyHandle::new(tx), Some("live".to_string()), matcher, 60);

    let Response::Ok(Reply::Screen(screen)) = rx.recv().expect("immediate screen reply") else {
        panic!("expected matching screen");
    };
    assert!(!screen.lines.is_empty());
    assert!(app.pending.is_empty());
}

#[test]
fn wait_for_text_clamps_its_terminal_field_and_defers_a_miss() {
    let mut app = manager(vec![running_entry("live")]);
    let start_field = app.entries[0].vm.as_ref().unwrap().fields_run;
    let matcher = crate::control::TextMatcher::new("NEVER PRESENT".to_string(), false).unwrap();
    let (tx, _rx) = mpsc::channel();

    app.start_wait_for_text(
        ReplyHandle::new(tx),
        Some("live".to_string()),
        matcher,
        crate::control::MAX_WAIT_FIELDS + 1,
    );

    let PendingCondition::WaitForText { terminal_field, .. } = app.pending[0].condition else {
        panic!("expected text wait");
    };
    assert_eq!(
        terminal_field,
        start_field + u64::from(crate::control::MAX_WAIT_FIELDS)
    );
}
