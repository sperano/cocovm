//! Manager VM lifecycle tests: launching honors `[ui]` prefs, Start/Pause/
//! Resume/Stop end-to-end through the detail-pane transport buttons
//! (including thumbnail capture), two machines running side by side, and a
//! broken media reference reporting instead of panicking.

use egui_kittest::kittest::Queryable;

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

    let vm = launch_machine(&def, "ui-prefs").expect("a default CoCo 3 definition launches");
    assert!(!vm.aspect_correct, "[ui].aspect_correct must reach the VM");
    assert!(
        vm.kb_mode == KbMode::Symbolic,
        "[ui].kb_mode must reach the VM"
    );
}

/// Start opens the definition's own VM (`entries[0].vm` goes from `None` to
/// `Some`) and the row/detail status text follows: Stopped → Running on
/// Start, Running → Paused on Pause, back to Running on Resume, and to
/// Stopped (with `vm` dropped) on Stop. Exercises the actual detail-pane
/// buttons end-to-end, including `manager::draw_running_vms`'s
/// `ViewportClass::Embedded` fallback and the `CocoApp::step_emulation`/
/// `draw_display` split `main.rs`'s `window_ui` refactor introduced — a
/// regression here would mean that split broke a running VM, not just the
/// manager's bookkeeping around it.
#[test]
fn start_button_launches_and_stop_button_stops() {
    let artifacts = TempDir::new("thumbnails");
    let entries = vec![sample_entry("dev-coco-3", "Dev CoCo 3")];
    let mut harness =
        manager_harness_with_artifacts(None, Some(artifacts.path().to_path_buf()), entries);

    click(&mut harness, "Dev CoCo 3");
    assert!(harness.state().entries[0].vm.is_none());
    assert!(label_exists(&harness, "Stopped"));

    click(&mut harness, manager::PLAY_GLYPH);
    assert!(harness.state().entries[0].vm.is_some(), "Start must launch the VM");
    assert!(harness.state().entries[0].vm.as_ref().unwrap().is_running());
    assert!(label_exists(&harness, "Running"));
    // One more step so `step_emulation` (which uploads the framebuffer
    // texture) has run at least once — regression coverage for the row
    // thumbnail staying black: `draw_row_thumbnail` reads exactly this.
    harness.step();
    assert!(
        harness.state().entries[0].vm.as_ref().unwrap().framebuffer_texture().is_some(),
        "a running VM must have an uploaded framebuffer texture for the row thumbnail to show"
    );

    click(&mut harness, manager::PAUSE_GLYPH);
    assert!(!harness.state().entries[0].vm.as_ref().unwrap().is_running());
    assert!(label_exists(&harness, "Paused"));

    click(&mut harness, manager::PLAY_GLYPH);
    assert!(harness.state().entries[0].vm.as_ref().unwrap().is_running());
    assert!(label_exists(&harness, "Running"));

    // Console-side controls next to the transport: Reset restarts the
    // machine but leaves it running (the console button, not a deck
    // control); Suspend exists but is disabled until save-states land.
    click(&mut harness, "Reset");
    assert!(
        harness.state().entries[0].vm.as_ref().unwrap().is_running(),
        "Reset must leave the machine on"
    );
    harness.get_by_label("Suspend");

    click(&mut harness, manager::STOP_GLYPH);
    assert!(harness.state().entries[0].vm.is_none(), "Stop must drop the VM");
    assert!(label_exists(&harness, "Stopped"));

    // Stop captured the machine's last screen as thumbnail.png in its
    // artifact dir, and the next frame's row draw loads it back as the
    // stopped entry's cached preview texture.
    assert!(
        artifacts.path().join("dev-coco-3").join("thumbnail.png").exists(),
        "Stop must write the stopped machine's screen preview"
    );
    harness.step();
    assert!(
        harness.state().entries[0].thumbnail.is_some(),
        "the stopped row must reload the saved preview as its thumbnail"
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
    click(&mut harness, manager::PLAY_GLYPH);
    assert!(harness.state().entries[0].vm.is_some());

    click(&mut harness, "Dev CoCo 2");
    click(&mut harness, manager::PLAY_GLYPH);
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
    click(&mut harness, manager::PLAY_GLYPH);

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
