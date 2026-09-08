//! `resolve_control_pending`/`check_pending` coverage. A [`PendingControl`]
//! needs only a [`crate::control::ReplyHandle`], which `ReplyHandle::new`
//! builds directly from an `mpsc` sender — no real connection needed.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::control::{Reply, ReplyHandle, Response};

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

/// A [`ReplyHandle`] with the receiving end kept, so a test can read back
/// what `resolve_control_pending` sent it.
fn reply_pair() -> (ReplyHandle, mpsc::Receiver<Response>) {
    let (tx, rx) = mpsc::channel();
    (ReplyHandle::new(tx), rx)
}

#[test]
fn retarget_changes_only_the_matching_slug() {
    let (reply, _receiver) = reply_pair();
    let mut pending = PendingControl::new(
        reply,
        "old".to_string(),
        PendingCondition::TypeTextDrained,
        NO_FIELDS,
        FIELD_RATE_HZ,
    );

    pending.retarget("other", "ignored");
    assert_eq!(pending.slug, "old");
    pending.retarget("old", "new");
    assert_eq!(pending.slug, "new");
}

#[test]
fn replies_done_once_the_condition_is_already_met() {
    let mut manager = manager(vec![running_entry("live")]);
    let (reply, rx) = reply_pair();

    // A freshly booted VM's `remote_type_ahead` starts empty/inactive.
    manager.pending.push(PendingControl::new(
        reply,
        "live".to_string(),
        PendingCondition::TypeTextDrained,
        NO_FIELDS,
        FIELD_RATE_HZ,
    ));
    manager.resolve_control_pending(&egui::Context::default());

    assert_eq!(rx.recv().expect("reply sent"), Response::Ok(Reply::Done));
    assert!(manager.pending.is_empty());
}

#[test]
fn errors_when_the_target_vm_no_longer_exists() {
    let mut manager = manager(Vec::new());
    let (reply, rx) = reply_pair();

    manager.pending.push(PendingControl::new(
        reply,
        "ghost".to_string(),
        PendingCondition::TypeTextDrained,
        NO_FIELDS,
        FIELD_RATE_HZ,
    ));
    manager.resolve_control_pending(&egui::Context::default());

    match rx.recv().expect("reply sent") {
        Response::Err(msg) => assert!(msg.contains("no longer exists")),
        other => panic!("expected an Err reply, got {other:?}"),
    }
    assert!(manager.pending.is_empty());
}

#[test]
fn keeps_waiting_while_the_condition_is_unmet_and_the_deadline_has_not_passed() {
    let mut manager = manager(vec![running_entry("live")]);
    manager.entries[0]
        .vm
        .as_mut()
        .unwrap()
        .start_remote_hold(&["A".to_string()], Some(600))
        .expect("hold succeeds");
    let (reply, _rx) = reply_pair();

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
    let mut manager = manager(vec![running_entry("live")]);
    manager.entries[0]
        .vm
        .as_mut()
        .unwrap()
        .start_remote_hold(&["A".to_string()], Some(600))
        .expect("hold succeeds");
    let (reply, rx) = reply_pair();

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

    match rx.recv().expect("reply sent") {
        Response::Err(msg) => assert!(msg.contains("timed out")),
        other => panic!("expected an Err reply, got {other:?}"),
    }
    assert!(manager.pending.is_empty());
}

#[test]
fn deadline_scales_with_the_expected_fields() {
    let (reply, _rx) = reply_pair();
    let before = Instant::now();
    let pending = PendingControl::new(
        reply,
        "live".to_string(),
        PendingCondition::WaitUntilField(0),
        crate::control::MAX_WAIT_FIELDS.into(),
        FIELD_RATE_HZ,
    );
    let expected =
        Duration::from_secs_f64(f64::from(crate::control::MAX_WAIT_FIELDS) / FIELD_RATE_HZ);
    assert!(pending.deadline >= before + expected + crate::manager::control::CONTROL_DEFER_MARGIN);
}
