//! App exit choices, failure recovery, and root viewport interaction.

use std::fs;

use egui_kittest::kittest::{NodeT, Queryable};

use super::*;
use crate::machine_def::{MachineDef, tests::TempDir};
use crate::manager::{SUSPEND_STATE_FILE, suspend_state_path};
use crate::save_state::tests::{boot_app, is_dirty, mount_and_dirty, write_one_track_disk};

const TEST_WINDOW_SIZE: egui::Vec2 = egui::vec2(1080.0, 720.0);

fn manager(root: &TempDir, names: &[&str]) -> ManagerApp {
    let entries = names
        .iter()
        .map(|name| {
            let def = MachineDef::from_config(
                (*name).to_string(),
                None,
                &coco_core::MachineConfig::default(),
            );
            let mut entry = MachineEntry::new((*name).to_string(), def);
            entry.vm = Some(Box::new(boot_app()));
            entry
        })
        .collect();
    let mut app = ManagerApp::new(
        None,
        Some(root.path().join("machines")),
        Some(root.path().join("artifacts")),
        entries,
        None,
    );
    // Keep fixture indices in the requested order despite the manager's sort.
    app.entries
        .sort_by_key(|entry| names.iter().position(|name| *name == entry.slug));
    app
}

fn open_dialog(manager: &mut ManagerApp) {
    manager.exit.choices = Some(Vec::new());
    manager.exit.reconcile(&manager.entries);
}

fn close_input() -> egui::RawInput {
    let mut input = egui::RawInput::default();
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    input
}

fn root_commands(output: &egui::FullOutput) -> &[egui::ViewportCommand] {
    &output.viewport_output[&egui::ViewportId::ROOT].commands
}

#[test]
fn close_with_no_running_machines_exits_immediately() {
    let root = TempDir::new("exit-empty");
    let mut app = manager(&root, &[]);
    let ctx = egui::Context::default();
    let output = ctx.run(close_input(), |ctx| {
        assert!(app.handle_exit_request(ctx));
    });
    assert!(app.exit.approved);
    assert!(!app.exit.is_pending());
    assert!(!root_commands(&output).contains(&egui::ViewportCommand::CancelClose));
}

#[test]
fn only_suspended_machines_exit_without_prompt_and_keep_checkpoint() {
    let root = TempDir::new("exit-only-suspended");
    let mut app = manager(&root, &["saved"]);
    app.suspend_vm(0);
    let checkpoint = suspend_state_path(app.artifacts_root.as_ref().unwrap(), "saved");
    let bytes = fs::read(&checkpoint).unwrap();
    let ctx = egui::Context::default();
    let _ = ctx.run(close_input(), |ctx| {
        assert!(app.handle_exit_request(ctx));
    });
    assert!(app.exit.approved);
    assert!(!app.exit.is_pending());
    eframe::App::on_exit(&mut app, None);
    assert_eq!(fs::read(checkpoint).unwrap(), bytes);
}

#[test]
fn paused_vm_intercepts_close_and_repeated_requests_preserve_choices() {
    let root = TempDir::new("exit-paused");
    let mut app = manager(&root, &["paused"]);
    app.entries[0].vm.as_mut().unwrap().set_running(false);
    let ctx = egui::Context::default();
    for action in [ExitAction::Suspend, ExitAction::ShutDown] {
        let output = ctx.run(close_input(), |ctx| {
            assert!(!app.handle_exit_request(ctx));
            app.draw_exit_confirmation(ctx);
        });
        assert!(root_commands(&output).contains(&egui::ViewportCommand::CancelClose));
        let choice = &mut app.exit.choices.as_mut().unwrap()[0];
        assert_eq!(choice.action, action);
        choice.action = ExitAction::ShutDown;
    }
    assert!(app.entries[0].is_running());
    assert!(!app.exit.approved);
}

#[test]
fn mixed_choices_preserve_existing_checkpoints() {
    let root = TempDir::new("exit-mixed");
    let mut app = manager(&root, &["save", "stop", "already-suspended"]);
    app.suspend_vm(2);
    let existing = suspend_state_path(app.artifacts_root.as_ref().unwrap(), "already-suspended");
    let existing_bytes = fs::read(&existing).unwrap();
    open_dialog(&mut app);
    let choices = app.exit.choices.as_mut().unwrap();
    assert_eq!(choices.len(), 2);
    choices[1].action = ExitAction::ShutDown;
    app.commit_exit(&egui::Context::default());
    assert!(app.exit.approved);
    assert!(app.entries[0].suspended);
    assert!(app.entries[0].vm.is_none());
    assert!(!app.entries[1].is_alive());
    assert!(app.entries[2].suspended);
    assert_eq!(fs::read(existing).unwrap(), existing_bytes);
    assert!(suspend_state_path(app.artifacts_root.as_ref().unwrap(), "save").is_file());
    assert!(!suspend_state_path(app.artifacts_root.as_ref().unwrap(), "stop").exists());
}

#[test]
fn choices_follow_sessions_through_reordering_and_renaming() {
    let root = TempDir::new("exit-identity");
    let mut app = manager(&root, &["save", "stop"]);
    open_dialog(&mut app);
    app.exit.choices.as_mut().unwrap()[1].action = ExitAction::ShutDown;
    app.entries.swap(0, 1);
    app.entries[0].slug = "renamed".to_string();
    app.entries[0].def.name = "Renamed machine".to_string();
    app.commit_exit(&egui::Context::default());
    assert!(!app.entries[0].is_alive());
    assert!(app.entries[1].suspended);
}

#[test]
fn new_running_machine_requires_another_confirmation() {
    let root = TempDir::new("exit-new-running");
    let mut app = manager(&root, &["first", "later"]);
    app.suspend_vm(1);
    open_dialog(&mut app);
    app.resume_vm(1);
    app.commit_exit(&egui::Context::default());
    assert!(!app.exit.approved);
    assert!(app.entries.iter().all(MachineEntry::is_running));
    assert_eq!(app.exit.choices.as_ref().unwrap().len(), 2);
    app.commit_exit(&egui::Context::default());
    assert!(app.exit.approved);
    assert!(app.entries.iter().all(|entry| entry.suspended));
}

#[test]
fn external_stop_or_suspend_removes_obsolete_choices() {
    let root = TempDir::new("exit-external");
    let mut app = manager(&root, &["stopped", "suspended"]);
    open_dialog(&mut app);
    app.exit.choices.as_mut().unwrap()[1].action = ExitAction::ShutDown;
    app.stop_vm(0);
    app.suspend_vm(1);
    app.commit_exit(&egui::Context::default());
    assert!(app.exit.approved);
    assert!(app.entries[1].suspended);
    assert!(suspend_state_path(app.artifacts_root.as_ref().unwrap(), "suspended").exists());
}

#[test]
fn failed_suspend_keeps_live_vm_and_retries_without_repeating_successes() {
    let root = TempDir::new("exit-suspend-retry");
    let mut app = manager(&root, &["saved", "blocked"]);
    let blocked = app.artifacts_root.as_ref().unwrap().join("blocked");
    fs::create_dir_all(blocked.parent().unwrap()).unwrap();
    fs::write(&blocked, b"not a directory").unwrap();
    open_dialog(&mut app);
    app.commit_exit(&egui::Context::default());
    assert!(!app.exit.approved);
    assert!(app.entries[0].suspended);
    assert!(app.entries[0].vm.is_none());
    assert!(app.entries[1].is_running());
    let choices = app.exit.choices.as_ref().unwrap();
    assert_eq!(choices.len(), 1);
    assert!(choices[0].error.is_some());
    fs::remove_file(blocked).unwrap();
    app.commit_exit(&egui::Context::default());
    assert!(app.exit.approved);
    assert!(app.entries.iter().all(|entry| entry.suspended));
}

#[test]
fn failed_shutdown_flush_retains_dirty_media_for_retry() {
    let root = TempDir::new("exit-flush-retry");
    let mut app = manager(&root, &["dirty"]);
    let disk = root.path().join("disk.dsk");
    write_one_track_disk(&disk);
    mount_and_dirty(app.entries[0].vm.as_mut().unwrap(), 0, &disk);
    fs::remove_file(&disk).unwrap();
    fs::create_dir(&disk).unwrap();
    open_dialog(&mut app);
    app.exit.choices.as_mut().unwrap()[0].action = ExitAction::ShutDown;
    app.commit_exit(&egui::Context::default());
    assert!(!app.exit.approved);
    assert!(app.entries[0].is_running());
    assert!(is_dirty(app.entries[0].vm.as_mut().unwrap(), 0));
    assert!(app.exit.choices.as_ref().unwrap()[0].error.is_some());
    fs::remove_dir(&disk).unwrap();
    app.commit_exit(&egui::Context::default());
    assert!(app.exit.approved);
    assert!(!app.entries[0].is_alive());
    assert!(disk.is_file());
}

#[test]
fn shutdown_cleanup_error_stays_visible_after_vm_has_stopped() {
    let root = TempDir::new("exit-cleanup-retry");
    let mut app = manager(&root, &["blocked"]);
    let checkpoint = app
        .artifacts_root
        .as_ref()
        .unwrap()
        .join("blocked")
        .join(SUSPEND_STATE_FILE);
    fs::create_dir_all(&checkpoint).unwrap();
    open_dialog(&mut app);
    app.exit.choices.as_mut().unwrap()[0].action = ExitAction::ShutDown;
    app.commit_exit(&egui::Context::default());
    assert!(!app.exit.approved);
    assert!(app.entries[0].vm.is_none());
    app.exit.reconcile(&app.entries);
    let choice = &app.exit.choices.as_ref().unwrap()[0];
    assert!(choice.retry_shutdown);
    assert!(choice.error.is_some());
    fs::remove_dir(&checkpoint).unwrap();
    app.commit_exit(&egui::Context::default());
    assert!(app.exit.approved);
}

#[test]
fn restarting_vm_with_pending_cleanup_requires_fresh_choices() {
    let root = TempDir::new("exit-cleanup-restart");
    let mut app = manager(&root, &["restarted"]);
    open_dialog(&mut app);
    let choice = &mut app.exit.choices.as_mut().unwrap()[0];
    choice.action = ExitAction::ShutDown;
    choice.retry_shutdown = true;
    choice.error = Some("Previous cleanup failed".to_string());
    app.commit_exit(&egui::Context::default());
    assert!(!app.exit.approved);
    assert!(app.entries[0].is_running());
    let choice = &app.exit.choices.as_ref().unwrap()[0];
    assert_eq!(choice.action, ExitAction::Suspend);
    assert!(!choice.retry_shutdown);
    assert!(choice.error.is_none());
}

fn harness(app: ManagerApp) -> egui_kittest::Harness<'static, ManagerApp> {
    let mut harness = egui_kittest::Harness::new_eframe(move |_| app);
    harness.set_size(TEST_WINDOW_SIZE);
    harness.step();
    harness
}

fn request_close(harness: &mut egui_kittest::Harness<'static, ManagerApp>) {
    harness
        .input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    harness.step();
    harness.step();
}

fn click(harness: &mut egui_kittest::Harness<'static, ManagerApp>, label: &str) {
    harness.get_by_label(label).hover();
    harness.step();
    harness.get_by_label(label).click();
    harness.step();
    harness.step();
}

#[test]
fn cancel_and_escape_leave_running_state_and_checkpoints_unchanged() {
    let root = TempDir::new("exit-ui-cancel");
    let mut harness = harness(manager(&root, &["running"]));
    request_close(&mut harness);
    harness.get_by_label("Quit CoCoVM?");
    click(&mut harness, "Shut down");
    click(&mut harness, "Cancel");
    assert!(!harness.state().exit.is_pending());
    assert!(harness.state().entries[0].is_running());
    assert!(!harness.state().exit.approved);
    request_close(&mut harness);
    assert_eq!(
        harness.state().exit.choices.as_ref().unwrap()[0].action,
        ExitAction::Suspend
    );
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.step();
    assert!(!harness.state().exit.is_pending());
    assert!(harness.state().entries[0].is_running());
    assert!(!root.path().join("artifacts").exists());
}

#[test]
fn quit_button_suspends_and_approved_frames_cannot_create_machines() {
    let root = TempDir::new("exit-ui-quit");
    let mut harness = harness(manager(&root, &["running"]));
    request_close(&mut harness);
    click(&mut harness, "Quit");
    assert!(harness.state().exit.approved);
    assert!(harness.state().entries[0].suspended);
    assert!(harness.state().entries[0].vm.is_none());
    assert!(root_commands(harness.output()).contains(&egui::ViewportCommand::Close));
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::N);
    harness.step();
    assert_eq!(harness.state().entries.len(), 1);
    request_close(&mut harness);
    assert!(!harness.state().exit.is_pending());
}

#[test]
fn suspend_failure_is_visible_in_modal_and_quit_retries() {
    let root = TempDir::new("exit-ui-retry");
    let mut app = manager(&root, &["running"]);
    let artifacts = app.artifacts_root.take();
    let mut harness = harness(app);
    request_close(&mut harness);
    click(&mut harness, "Quit");
    harness.get_by_label(super::super::NO_DATA_DIR);
    assert!(!harness.state().exit.approved);
    assert!(harness.state().entries[0].is_running());
    harness.state_mut().artifacts_root = artifacts;
    click(&mut harness, "Quit");
    assert!(harness.state().exit.approved);
    assert!(harness.state().entries[0].suspended);
}

#[test]
fn newly_running_machine_is_shown_before_quit_becomes_enabled() {
    let root = TempDir::new("exit-ui-new-running");
    let mut app = manager(&root, &["running", "later"]);
    app.entries[1].vm = None;
    let mut harness = harness(app);
    request_close(&mut harness);
    assert!(!harness.get_by_label("Quit").accesskit_node().is_disabled());
    harness.state_mut().entries[1].vm = Some(Box::new(boot_app()));
    harness.step();
    assert!(harness.get_by_label("Quit").accesskit_node().is_disabled());
    assert_eq!(harness.state().exit.choices.as_ref().unwrap().len(), 2);
    harness.step();
    assert!(!harness.get_by_label("Quit").accesskit_node().is_disabled());
}

#[test]
fn same_frame_native_close_and_confirmation_close_again_next_frame() {
    let root = TempDir::new("exit-same-frame");
    let mut app = manager(&root, &["running"]);
    open_dialog(&mut app);
    let ctx = egui::Context::default();
    let output = ctx.run(close_input(), |ctx| {
        assert!(!app.handle_exit_request(ctx));
        app.commit_exit(ctx);
    });
    assert!(root_commands(&output).contains(&egui::ViewportCommand::CancelClose));
    assert!(root_commands(&output).contains(&egui::ViewportCommand::Close));
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        assert!(app.handle_exit_request(ctx));
    });
    assert_eq!(root_commands(&output), &[egui::ViewportCommand::Close]);
}
