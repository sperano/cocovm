//! `dispatch_control` coverage. Most of its logic (VM resolution, disk/reset/
//! joystick effects) is exercised through the private per-action helpers
//! directly — no networking needed. One end-to-end test drives the whole
//! stack through a real loopback connection, proving the wiring in
//! `ManagerApp::drain_control`/`bind_control` actually works.

use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use coco_control::{Action, ControlClient, ControlError, ControlServer, Reply, Request};
use eframe::egui;

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
        .expect("installed coco3.rom is required (ensure_assets)")
        .into_boxed_slice();
    let vm = CocoApp::new(
        MachineConfig::default(),
        rom,
        ROMSource::File(rom_path),
        AppParams::default(),
    );
    let mut entry = off_entry(slug);
    entry.vm = Some(Box::new(vm));
    entry
}

fn manager(entries: Vec<MachineEntry>) -> ManagerApp {
    ManagerApp::new(None, None, None, entries, None)
}

/// Send `request` on a background thread and poll `drain_control`/
/// `resolve_control_pending` until its reply lands.
fn call(manager: &mut ManagerApp, port: u16, request: Request) -> Result<Reply, ControlError> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut client = ControlClient::connect(port).expect("connect to the control listener");
        let _ = tx.send(client.call(&request));
    });
    let ctx = egui::Context::default();
    for _ in 0..2_000 {
        manager.drain_control();
        manager.resolve_control_pending(&ctx);
        if let Ok(result) = rx.try_recv() {
            return result;
        }
        thread::sleep(Duration::from_millis(2));
    }
    panic!("no reply within the poll budget");
}

/// End-to-end: bind a real listener, connect a real client, and prove
/// `list_vms` round-trips through `bind_control`/`drain_control`.
#[test]
fn list_vms_round_trips_over_a_real_loopback_connection() {
    let server = ControlServer::bind(0, Arc::new(|| {})).expect("bind ephemeral port");
    let port = server.port();
    let mut manager = manager(vec![off_entry("solo")]);
    manager.control = Some(server);

    let reply = call(
        &mut manager,
        port,
        Request {
            vm: None,
            action: Action::ListVms,
        },
    )
    .expect("list_vms succeeds");

    let Reply::Vms(infos) = reply else {
        panic!("expected Reply::Vms");
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
