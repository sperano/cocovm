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
        crate::joy::SharedGamepad::without_backend(),
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
fn errors_when_the_target_vm_has_stopped() {
    let mut manager = manager(vec![off_entry("stopped")]);
    let (reply, rx) = reply_pair();

    manager.pending.push(PendingControl::new(
        reply,
        "stopped".to_string(),
        PendingCondition::WaitUntilField(1),
        NO_FIELDS,
        FIELD_RATE_HZ,
    ));
    manager.resolve_control_pending(&egui::Context::default());

    match rx.recv().expect("reply sent") {
        Response::Err(msg) => assert!(msg.contains("no longer running")),
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
    let now = Instant::now();
    pending.deadline = now - Duration::from_secs(1);
    manager.pending.push(pending);
    manager.resolve_control_pending_at(&egui::Context::default(), now);

    match rx.recv().expect("reply sent") {
        Response::Err(msg) => assert!(msg.contains("timed out")),
        other => panic!("expected an Err reply, got {other:?}"),
    }
    assert!(manager.pending.is_empty());
}

#[test]
fn type_text_past_its_deadline_lives_on_while_the_burst_still_drains() {
    let mut manager = manager(vec![running_entry("live")]);
    manager.entries[0]
        .vm
        .as_mut()
        .unwrap()
        .start_remote_typing("AB")
        .expect("typing succeeds");
    let (reply, rx) = reply_pair();

    let mut pending = PendingControl::new(
        reply,
        "live".to_string(),
        PendingCondition::TypeTextDrained,
        NO_FIELDS,
        FIELD_RATE_HZ,
    );
    let now = Instant::now();
    pending.deadline = now - Duration::from_secs(1);
    // Two taps queued: fewer than at the last check, so the burst progressed.
    pending.remaining_taps = 3;
    manager.pending.push(pending);
    manager.resolve_control_pending_at(&egui::Context::default(), now);
    assert_eq!(manager.pending.len(), 1, "progress renews the deadline");
    assert_eq!(manager.pending[0].remaining_taps, 2);
    assert!(manager.pending[0].deadline >= now + crate::manager::control::CONTROL_DEFER_MARGIN);

    // No further progress once the renewed deadline passes: it times out.
    let later = now + crate::manager::control::CONTROL_DEFER_MARGIN + Duration::from_secs(1);
    manager.resolve_control_pending_at(&egui::Context::default(), later);
    match rx.recv().expect("reply sent") {
        Response::Err(msg) => assert!(msg.contains("timed out")),
        other => panic!("expected an Err reply, got {other:?}"),
    }
    assert!(manager.pending.is_empty());
}

#[test]
fn unmet_condition_waits_for_its_deadline_without_an_immediate_poll() {
    const UNTIL_DEADLINE: Duration = Duration::from_secs(5);

    let mut manager = manager(vec![running_entry("live")]);
    let (reply, _rx) = reply_pair();
    let now = Instant::now();
    let mut pending = PendingControl::new(
        reply,
        "live".to_string(),
        PendingCondition::WaitUntilField(u64::MAX),
        NO_FIELDS,
        FIELD_RATE_HZ,
    );
    pending.deadline = now + UNTIL_DEADLINE;
    manager.pending.push(pending);

    let (repaint_tx, repaint_rx) = mpsc::channel();
    let ctx = egui::Context::default();
    ctx.set_request_repaint_callback(move |info| {
        repaint_tx.send(info.delay).expect("record repaint request");
    });
    manager.resolve_control_pending_at(&ctx, now);

    let delay = repaint_rx.try_recv().expect("deadline repaint requested");
    assert!(delay > Duration::ZERO);
    assert!(delay <= UNTIL_DEADLINE);
    assert_eq!(manager.pending.len(), 1);
}

/// Push a pending request on `manager`'s entry `slug` for `condition`, with
/// its deadline still [`CONTROL_DEFER_MARGIN`] away, resolve once, and return
/// the reply it got.
fn resolve_once(manager: &mut ManagerApp, slug: &str, condition: PendingCondition) -> Response {
    let (reply, rx) = reply_pair();
    manager.pending.push(PendingControl::new(
        reply,
        slug.to_string(),
        condition,
        NO_FIELDS,
        FIELD_RATE_HZ,
    ));
    manager.resolve_control_pending(&egui::Context::default());
    assert!(manager.pending.is_empty(), "must reply without waiting");
    rx.try_recv().expect("reply sent this frame")
}

#[test]
fn wait_errors_promptly_once_its_vm_is_paused() {
    let mut manager = manager(vec![running_entry("live")]);
    let vm = manager.entries[0].vm.as_mut().unwrap();
    let target = vm.fields_run + 1;
    vm.set_running(false);
    let condition = PendingCondition::WaitUntilField(target);

    match resolve_once(&mut manager, "live", condition) {
        Response::Err(msg) => {
            assert!(msg.contains("was paused"), "{msg}");
            assert!(msg.contains("call set_running"), "{msg}");
        }
        other => panic!("expected an Err reply, got {other:?}"),
    }
}

#[test]
fn wait_names_start_vm_once_its_vm_is_suspended() {
    let mut manager = manager(vec![running_entry("live")]);
    let vm = manager.entries[0].vm.as_mut().unwrap();
    let target = vm.fields_run + 1;
    // What `ManagerApp::suspend_vm` leaves behind for a still-open window.
    vm.set_running(false);
    manager.entries[0].suspended = true;
    let condition = PendingCondition::WaitUntilField(target);

    match resolve_once(&mut manager, "live", condition) {
        Response::Err(msg) => {
            assert!(msg.contains("was suspended"), "{msg}");
            assert!(msg.contains("call start_vm"), "{msg}");
        }
        other => panic!("expected an Err reply, got {other:?}"),
    }
}

#[test]
fn press_keys_errors_promptly_once_its_vm_is_paused() {
    let mut manager = manager(vec![running_entry("live")]);
    let vm = manager.entries[0].vm.as_mut().unwrap();
    vm.start_remote_hold(&["A".to_string()], Some(600))
        .expect("hold succeeds");
    vm.set_running(false);

    match resolve_once(&mut manager, "live", PendingCondition::KeysReleased) {
        Response::Err(msg) => assert!(msg.contains("held keys"), "{msg}"),
        other => panic!("expected an Err reply, got {other:?}"),
    }
}

#[test]
fn paused_vm_still_replies_done_when_the_condition_was_already_met() {
    let mut manager = manager(vec![running_entry("live")]);
    let vm = manager.entries[0].vm.as_mut().unwrap();
    let reached = vm.fields_run;
    vm.set_running(false);
    let condition = PendingCondition::WaitUntilField(reached);

    let reply = resolve_once(&mut manager, "live", condition);
    assert_eq!(reply, Response::Ok(Reply::Done));
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

#[test]
fn abandoned_pending_request_is_reclaimed_without_a_reply() {
    let mut manager = manager(vec![running_entry("live")]);
    let (reply, rx) = reply_pair();
    reply.abandon_for_test();
    manager.pending.push(PendingControl::new(
        reply,
        "live".to_string(),
        PendingCondition::WaitUntilField(u64::MAX),
        NO_FIELDS,
        FIELD_RATE_HZ,
    ));

    manager.resolve_control_pending(&egui::Context::default());

    assert!(manager.pending.is_empty());
    assert!(rx.try_recv().is_err());
}
