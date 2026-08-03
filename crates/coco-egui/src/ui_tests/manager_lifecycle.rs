//! Manager VM lifecycle tests: launching honors `[ui]` prefs, the
//! three-state model (Powered Off / Running / Suspended) end-to-end through
//! the toolbar's transport buttons — including suspend's screenshot +
//! `suspended.ccstate` persistence and resume-from-disk — two machines
//! running side by side, and a broken media reference reporting instead of
//! panicking.

use egui_kittest::kittest::{NodeT, Queryable};

use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;

/// `[ui]` preferences in a definition are the launched VM's *starting*
/// state (they stay live F9/F12 toggles afterwards) — regression coverage
/// for `launch_machine` ignoring the section entirely.
#[test]
fn launch_honors_ui_settings() {
    let mut def = machine_def::MachineDef::from_config(
        "UI Prefs".to_string(),
        None,
        &MachineConfig::default(),
    );
    def.ui.aspect_correct = false;
    def.ui.kb_mode = machine_def::KbModeDTO::Symbolic;
    def.hardware.display = Some(machine_def::DisplayDTO::TVBW);

    let vm = launch_machine(&def, "ui-prefs").expect("a default CoCo 3 definition launches");
    assert!(!vm.aspect_correct, "[ui].aspect_correct must reach the VM");
    assert!(
        vm.kb_mode == KbMode::Symbolic,
        "[ui].kb_mode must reach the VM"
    );
    assert_eq!(
        vm.display,
        Display::TV(crate::display::TV::BW),
        "[hardware].display must reach the VM"
    );
    assert_eq!(
        vm.machine.bus.gime.monitor,
        coco_core::MonitorType::Composite,
        "a TV decodes the composite signal path"
    );
}

/// The three-state round trip through the actual toolbar transport
/// buttons: Powered Off → (▶) Running → (⏸) Suspended — screenshot +
/// `suspended.ccstate` written, VM paused in place — → (▶) Running again
/// (frozen state discarded) → (⏹) Powered Off, with a (↻) Reset along the
/// way that must leave the machine Running, and Reset's Running-only
/// enable gate pinned at both off-states. Exercises
/// `manager::draw_running_vms`'s `ViewportClass::Embedded` fallback and the
/// `CocoApp::step_emulation`/`draw_display` split along the way — a
/// regression here would mean that split broke a running VM, not just the
/// manager's bookkeeping around it.
#[test]
fn transport_buttons_walk_the_three_states() {
    let artifacts = TempDir::new("suspend-transport");
    let entries = vec![sample_entry("dev-coco-3", "Dev CoCo 3")];
    let mut harness =
        manager_harness_with_artifacts(None, Some(artifacts.path().to_path_buf()), entries);
    let state_file = artifacts
        .path()
        .join("dev-coco-3")
        .join("suspended.ccstate");

    click(&mut harness, "Dev CoCo 3");
    assert!(harness.state().entries[0].vm.is_none());
    assert!(label_exists(&harness, "Powered Off"));
    assert!(
        harness.get_by_label("Reset").accesskit_node().is_disabled(),
        "Reset must be disabled on a powered-off machine"
    );

    click(&mut harness, "Start");
    assert!(
        harness.state().entries[0].vm.is_some(),
        "Play must launch the VM"
    );
    assert!(harness.state().entries[0].vm.as_ref().unwrap().is_running());
    assert!(label_exists(&harness, "Running"));
    // One more step so `step_emulation` (which uploads the framebuffer
    // texture) has run at least once — regression coverage for the row
    // thumbnail staying black: `draw_row_thumbnail` reads exactly this.
    harness.step();
    assert!(
        harness.state().entries[0]
            .vm
            .as_ref()
            .unwrap()
            .framebuffer_texture()
            .is_some(),
        "a running VM must have an uploaded framebuffer texture for the row thumbnail to show"
    );

    // Reset (↻) restarts the machine but leaves it on.
    click(&mut harness, "Reset");
    assert!(
        harness.state().entries[0].vm.as_ref().unwrap().is_running(),
        "Reset must leave the machine on"
    );

    click(&mut harness, "Suspend");
    {
        let entry = &harness.state().entries[0];
        assert!(entry.suspended, "Suspend must mark the entry");
        assert!(
            entry.vm.is_some(),
            "Suspend must NOT drop the VM (window stays open)"
        );
        assert!(
            !entry.vm.as_ref().unwrap().is_running(),
            "Suspend must pause emulation"
        );
    }
    assert!(
        state_file.is_file(),
        "Suspend must freeze the machine to disk"
    );
    assert!(
        artifacts
            .path()
            .join("dev-coco-3")
            .join("thumbnail.png")
            .exists(),
        "Suspend must capture the screen preview"
    );
    assert!(label_exists(&harness, "Suspended"));
    assert!(
        harness.get_by_label("Reset").accesskit_node().is_disabled(),
        "Reset must be disabled on a suspended machine"
    );

    click(&mut harness, "Start");
    {
        let entry = &harness.state().entries[0];
        assert!(!entry.suspended);
        assert!(
            entry.vm.as_ref().unwrap().is_running(),
            "Play must resume emulation"
        );
    }
    assert!(
        !state_file.exists(),
        "resuming must discard the frozen state"
    );
    assert!(label_exists(&harness, "Running"));

    click(&mut harness, "Stop");
    assert!(
        harness.state().entries[0].vm.is_none(),
        "Stop must drop the VM"
    );
    assert!(label_exists(&harness, "Powered Off"));
    harness.step();
    assert!(
        harness.state().entries[0].thumbnail.is_none(),
        "a powered-off row must not show a saved preview — it is black"
    );
}

/// The VM window's own Suspend tile (`chrome::toolbar`) sets
/// `CocoApp::pending_suspend`, but the tile itself is unreachable under
/// kittest — a VM viewport degrades to the embedded display-only fallback
/// in headless tests (`draw_running_vms`'s `ViewportClass::Embedded`
/// branch), which draws no chrome at all. So this drives the plumbing
/// directly: setting the flag and stepping one frame must produce exactly
/// the suspend `transport_buttons_walk_the_three_states` gets from clicking
/// the manager toolbar's own Suspend tile.
#[test]
fn vm_window_pending_suspend_flag_suspends_through_the_manager() {
    let artifacts = TempDir::new("suspend-pending-flag");
    let entries = vec![sample_entry("dev-coco-3", "Dev CoCo 3")];
    let mut harness =
        manager_harness_with_artifacts(None, Some(artifacts.path().to_path_buf()), entries);
    let state_file = artifacts
        .path()
        .join("dev-coco-3")
        .join("suspended.ccstate");

    click(&mut harness, "Dev CoCo 3");
    click(&mut harness, "Start");

    harness.state_mut().entries[0]
        .vm
        .as_mut()
        .unwrap()
        .pending_suspend = true;
    harness.step();

    let entry = &harness.state().entries[0];
    assert!(
        entry.suspended,
        "the pending_suspend flag must suspend the entry"
    );
    assert!(
        state_file.is_file(),
        "suspend must freeze the machine to disk"
    );
}

/// Suspend → close the VM window → resume: the frozen state survives the VM
/// object being dropped and restores byte-for-byte from `suspended.ccstate`.
/// The marker is a byte poked into physical RAM the booted machine never
/// maps (a 128K CoCo 3's default MMU task only reaches the upper 64K), so
/// the machine can't disturb it between the poke and the suspend.
#[test]
fn resume_after_window_close_restores_the_frozen_state() {
    const MARKER_ADDR: usize = 0x1234;
    const MARKER: u8 = 0xAB;
    let artifacts = TempDir::new("suspend-resume-disk");
    let entries = vec![sample_entry("dev-coco-3", "Dev CoCo 3")];
    let mut harness =
        manager_harness_with_artifacts(None, Some(artifacts.path().to_path_buf()), entries);
    let state_file = artifacts
        .path()
        .join("dev-coco-3")
        .join("suspended.ccstate");

    click(&mut harness, "Dev CoCo 3");
    click(&mut harness, "Start");
    harness.state_mut().entries[0]
        .vm
        .as_mut()
        .unwrap()
        .machine
        .bus
        .ram[MARKER_ADDR] = MARKER;

    click(&mut harness, "Suspend");
    assert!(state_file.is_file());

    // The window-close path for a suspended machine — the exact production
    // branch (`vm_windows::close_vm_window`): the VM object is dropped, the
    // frozen state stays.
    harness.state_mut().close_vm_window(0);
    assert!(harness.state().entries[0].vm.is_none());
    assert!(
        state_file.is_file(),
        "closing a suspended window must keep the frozen state"
    );
    harness.step();
    assert!(label_exists(&harness, "Suspended"));
    assert!(
        harness.state().entries[0].thumbnail.is_some(),
        "a window-closed suspended row must show its suspend-time screenshot"
    );

    click(&mut harness, "Start");
    let entry = &harness.state().entries[0];
    assert!(!entry.suspended);
    let vm = entry.vm.as_ref().expect("resume must relaunch the VM");
    assert!(vm.is_running());
    assert_eq!(
        vm.machine.bus.ram[MARKER_ADDR], MARKER,
        "resume must restore the frozen machine, not boot a fresh one"
    );
    assert!(
        !state_file.exists(),
        "resuming must discard the frozen state"
    );
}

/// A `suspended.ccstate` already on disk at startup seeds the entry as
/// Suspended ([`manager::ManagerApp::new`]) — the state survives quitting
/// the manager. A file that then fails to restore (garbage here) reports in
/// the detail pane and leaves the machine Suspended, its frozen copy
/// untouched.
#[test]
fn startup_seeds_suspended_from_disk_and_failed_resume_keeps_it() {
    let artifacts = TempDir::new("suspend-startup");
    let machine_dir = artifacts.path().join("dev-coco-3");
    std::fs::create_dir_all(&machine_dir).unwrap();
    let state_file = machine_dir.join("suspended.ccstate");
    std::fs::write(&state_file, b"not a ccstate").unwrap();

    let entries = vec![sample_entry("dev-coco-3", "Dev CoCo 3")];
    let mut harness =
        manager_harness_with_artifacts(None, Some(artifacts.path().to_path_buf()), entries);
    assert!(harness.state().entries[0].suspended);

    click(&mut harness, "Dev CoCo 3");
    assert!(label_exists(&harness, "Suspended"));

    click(&mut harness, "Start");
    let entry = &harness.state().entries[0];
    assert!(
        entry.suspended,
        "a failed restore must keep the machine Suspended"
    );
    assert!(
        entry.vm.is_none(),
        "a failed restore must not leave a half-launched VM"
    );
    // Specifically the snapshot decoder's own error — proving the relaunch
    // succeeded and it was the *restore* that failed, not `start_vm` (which
    // would record a ROM/media error instead and pass these asserts for
    // the wrong reason).
    assert!(
        entry
            .launch_error
            .as_deref()
            .is_some_and(|e| e.contains("not a CoCo save state")),
        "the restore failure must reach the detail pane; got {:?}",
        entry.launch_error
    );
    assert!(
        state_file.is_file(),
        "the frozen state must survive a failed resume"
    );
}

/// A suspended VM's window is display-only all the way down to *input*:
/// `draw_running_vms` runs only the framebuffer-texture upload for it,
/// never `step_emulation`, whose `handle_input` would leave app shortcuts
/// (quick-load ⌘1 replacing the frozen machine wholesale!) and keyboard/
/// joystick writes live on a machine whose on-disk frozen copy they'd
/// silently diverge from. F9 (the aspect toggle) is the cheapest
/// observable of that whole class: it must bounce off a suspended VM and
/// work again once resumed.
#[test]
fn suspended_vm_window_ignores_input() {
    let artifacts = TempDir::new("suspend-input-gate");
    let entries = vec![sample_entry("dev-coco-3", "Dev CoCo 3")];
    let mut harness =
        manager_harness_with_artifacts(None, Some(artifacts.path().to_path_buf()), entries);

    click(&mut harness, "Dev CoCo 3");
    click(&mut harness, "Start");
    click(&mut harness, "Suspend");
    let before = harness.state().entries[0]
        .vm
        .as_ref()
        .unwrap()
        .aspect_correct;

    harness.key_press(egui::Key::F9);
    harness.step();
    assert_eq!(
        harness.state().entries[0]
            .vm
            .as_ref()
            .unwrap()
            .aspect_correct,
        before,
        "a suspended VM's window must not process app shortcuts"
    );

    click(&mut harness, "Start");
    harness.key_press(egui::Key::F9);
    harness.step();
    assert_eq!(
        harness.state().entries[0]
            .vm
            .as_ref()
            .unwrap()
            .aspect_correct,
        !before,
        "a resumed VM's window processes shortcuts again"
    );
}

/// The VM window's close box, driven through the real UI (the embedded
/// fallback's `egui::Window` close button): on a Running machine it is the
/// power switch; on a Suspended one it only drops the VM object, keeping
/// the frozen state on disk. Guards `draw_running_vms`'s close-request
/// wiring end to end — the other suspend tests call `close_vm_window`
/// directly.
#[test]
fn window_close_powers_off_running_but_preserves_suspended() {
    let artifacts = TempDir::new("suspend-window-close");
    let entries = vec![sample_entry("dev-coco-3", "Dev CoCo 3")];
    let mut harness =
        manager_harness_with_artifacts(None, Some(artifacts.path().to_path_buf()), entries);
    let state_file = artifacts
        .path()
        .join("dev-coco-3")
        .join("suspended.ccstate");

    click(&mut harness, "Dev CoCo 3");
    click(&mut harness, "Start");
    click(&mut harness, "Close window");
    assert!(
        harness.state().entries[0].vm.is_none(),
        "closing a running VM's window is the power switch"
    );
    assert!(label_exists(&harness, "Powered Off"));

    click(&mut harness, "Start");
    click(&mut harness, "Suspend");
    assert!(state_file.is_file());
    click(&mut harness, "Close window");
    let entry = &harness.state().entries[0];
    assert!(
        entry.vm.is_none(),
        "closing a suspended VM's window drops the VM object"
    );
    assert!(entry.suspended, "…but the machine stays Suspended");
    assert!(state_file.is_file(), "…and the frozen state survives");
}

/// Stop on a suspended, window-closed machine is the power switch: the
/// frozen state and the row preview are both discarded.
#[test]
fn stop_on_suspended_machine_discards_the_frozen_state() {
    let artifacts = TempDir::new("suspend-stop");
    let entries = vec![sample_entry("dev-coco-3", "Dev CoCo 3")];
    let mut harness =
        manager_harness_with_artifacts(None, Some(artifacts.path().to_path_buf()), entries);
    let state_file = artifacts
        .path()
        .join("dev-coco-3")
        .join("suspended.ccstate");

    click(&mut harness, "Dev CoCo 3");
    click(&mut harness, "Start");
    click(&mut harness, "Suspend");
    harness.state_mut().close_vm_window(0);
    harness.step();

    click(&mut harness, "Stop");
    let entry = &harness.state().entries[0];
    assert!(!entry.suspended);
    assert!(entry.vm.is_none());
    assert!(!state_file.exists(), "Stop must discard the frozen state");
    assert!(label_exists(&harness, "Powered Off"));
    harness.step();
    assert!(
        harness.state().entries[0].thumbnail.is_none(),
        "a powered-off row must not keep the stale screenshot"
    );
}

/// Two machines — a CoCo 3 and a CoCo 2 — start independently in the same
/// manager and both keep stepping across further frames without panicking:
/// the "DECIDED: in-process, one native window per running VM" acceptance
/// scenario (`docs/plan-machine-persistence.md`), minus the pacing/audio
/// independence a headless harness has no way to observe.
#[test]
fn starting_two_machines_runs_both() {
    let entries = vec![
        sample_entry("dev-coco-3", "Dev CoCo 3"),
        sample_coco2_entry("dev-coco-2", "Dev CoCo 2"),
    ];
    let mut harness = manager_harness(None, entries);

    click(&mut harness, "Dev CoCo 3");
    click(&mut harness, "Start");
    assert!(harness.state().entries[0].vm.is_some());

    click(&mut harness, "Dev CoCo 2");
    click(&mut harness, "Start");
    assert!(harness.state().entries[1].vm.is_some());

    assert!(harness.state().entries[0].vm.as_ref().unwrap().is_running());
    assert!(harness.state().entries[1].vm.as_ref().unwrap().is_running());

    // Both VMs keep emulating side by side for a few more frames without
    // panicking (each has its own `field_debt`/audio stream, so neither
    // stepping the other is expected — just that co-existing doesn't break).
    for _ in 0..5 {
        harness.step();
    }
    assert!(harness.state().entries[0].vm.is_some());
    assert!(harness.state().entries[1].vm.is_some());
}

/// A definition whose media references a file that doesn't exist reports the
/// failure in the detail pane instead of panicking or leaving a partially
/// mounted VM behind (`crate::launch_machine`'s contract: any mount failure
/// is a returned `Err`, never a partial `CocoApp`).
#[test]
fn launch_error_is_reported_not_fatal() {
    let mut def = machine_def::MachineDef::from_config(
        "Broken Media".to_string(),
        None,
        &MachineConfig::default(),
    );
    def.media.disk0 = Some("/definitely/does/not/exist.dsk".to_string());
    let entries = vec![manager::MachineEntry::new("broken-media".to_string(), def)];
    let mut harness = manager_harness(None, entries);

    click(&mut harness, "Broken Media");
    click(&mut harness, "Start");

    assert!(
        harness.state().entries[0].vm.is_none(),
        "a failed launch must not leave a partial VM running"
    );
    assert!(
        harness.state().entries[0].launch_error.is_some(),
        "the failure must be recorded for the detail pane"
    );
    harness.get_by_label_contains("could not read");
}
