//! `resolve_control_pending`/`check_pending` coverage. A [`PendingControl`]
//! can only be built from a real `coco_control::Incoming`, which has no
//! public constructor — so each test obtains one from an actual (ephemeral,
//! loopback) `ControlServer`/`ControlClient` pair, then drives the pending
//! machinery directly against it.

use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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

/// Deferred requests here need no field budget: the deadline is only
/// [`CONTROL_DEFER_MARGIN`] away.
const NO_FIELDS: u64 = 0;
const FIELD_RATE_HZ: f64 = 60.0;

fn manager(entries: Vec<MachineEntry>) -> ManagerApp {
    ManagerApp::new(None, None, None, entries, None)
}

/// Fire a throwaway request at `server` (its content doesn't matter — the
/// caller replaces it with a hand-built [`PendingControl`]) and hand back
/// the resulting `Incoming` plus the client thread waiting on its reply.
fn take_incoming(
    server: &ControlServer,
    port: u16,
) -> (
    coco_control::ReplyHandle,
    JoinHandle<Result<Reply, ControlError>>,
) {
    let handle = thread::spawn(move || {
        let mut client = ControlClient::connect(port).expect("connect to the control listener");
        client.call(&Request {
            vm: None,
            action: Action::ListVms,
        })
    });
    for _ in 0..2_000 {
        if let Some(incoming) = server.try_recv() {
            return (incoming.into_parts().1, handle);
        }
        thread::sleep(Duration::from_millis(2));
    }
    panic!("no incoming request within the poll budget");
}

#[test]
fn replies_done_once_the_condition_is_already_met() {
    let server = ControlServer::bind(0, Arc::new(|| {})).expect("bind ephemeral port");
    let port = server.port();
    let mut manager = manager(vec![running_entry("live")]);
    let (reply, handle) = take_incoming(&server, port);

    // A freshly booted VM's `remote_type_ahead` starts empty/inactive.
    manager.pending.push(PendingControl::new(
        reply,
        "live".to_string(),
        PendingCondition::TypeTextDrained,
        NO_FIELDS,
        FIELD_RATE_HZ,
    ));
    manager.resolve_control_pending(&egui::Context::default());

    assert_eq!(handle.join().unwrap().expect("done reply"), Reply::Done);
    assert!(manager.pending.is_empty());
}

#[test]
fn errors_when_the_target_vm_no_longer_exists() {
    let server = ControlServer::bind(0, Arc::new(|| {})).expect("bind ephemeral port");
    let port = server.port();
    let mut manager = manager(Vec::new());
    let (reply, handle) = take_incoming(&server, port);

    manager.pending.push(PendingControl::new(
        reply,
        "ghost".to_string(),
        PendingCondition::TypeTextDrained,
        NO_FIELDS,
        FIELD_RATE_HZ,
    ));
    manager.resolve_control_pending(&egui::Context::default());

    match handle.join().unwrap() {
        Err(ControlError::Remote(msg)) => assert!(msg.contains("no longer exists")),
        other => panic!("expected a Remote error, got {other:?}"),
    }
    assert!(manager.pending.is_empty());
}

#[test]
fn keeps_waiting_while_the_condition_is_unmet_and_the_deadline_has_not_passed() {
    let server = ControlServer::bind(0, Arc::new(|| {})).expect("bind ephemeral port");
    let port = server.port();
    let mut manager = manager(vec![running_entry("live")]);
    manager.entries[0]
        .vm
        .as_mut()
        .unwrap()
        .start_remote_hold(&["A".to_string()], Some(600))
        .expect("hold succeeds");
    let (reply, _handle) = take_incoming(&server, port);

    manager.pending.push(PendingControl::new(
        reply,
        "live".to_string(),
        PendingCondition::KeysReleased,
        NO_FIELDS,
        FIELD_RATE_HZ,
    ));
    manager.resolve_control_pending(&egui::Context::default());

    assert_eq!(manager.pending.len(), 1, "must still be waiting");
}

#[test]
fn times_out_once_the_deadline_has_passed() {
    let server = ControlServer::bind(0, Arc::new(|| {})).expect("bind ephemeral port");
    let port = server.port();
    let mut manager = manager(vec![running_entry("live")]);
    manager.entries[0]
        .vm
        .as_mut()
        .unwrap()
        .start_remote_hold(&["A".to_string()], Some(600))
        .expect("hold succeeds");
    let (reply, handle) = take_incoming(&server, port);

    let mut pending = PendingControl::new(
        reply,
        "live".to_string(),
        PendingCondition::KeysReleased,
        NO_FIELDS,
        FIELD_RATE_HZ,
    );
    pending.deadline = Instant::now() - Duration::from_secs(1);
    manager.pending.push(pending);
    manager.resolve_control_pending(&egui::Context::default());

    match handle.join().unwrap() {
        Err(ControlError::Remote(msg)) => assert!(msg.contains("timed out")),
        other => panic!("expected a Remote error, got {other:?}"),
    }
    assert!(manager.pending.is_empty());
}

#[test]
fn deadline_scales_with_the_expected_fields() {
    let server = ControlServer::bind(0, Arc::new(|| {})).expect("bind ephemeral port");
    let (reply, _handle) = take_incoming(&server, server.port());
    let before = Instant::now();
    let pending = PendingControl::new(
        reply,
        "live".to_string(),
        PendingCondition::WaitUntilField(0),
        coco_control::MAX_WAIT_FIELDS.into(),
        FIELD_RATE_HZ,
    );
    let expected =
        Duration::from_secs_f64(f64::from(coco_control::MAX_WAIT_FIELDS) / FIELD_RATE_HZ);
    assert!(pending.deadline >= before + expected + crate::manager::control::CONTROL_DEFER_MARGIN);
}
