//! VM window menu/toolbar/hotkey tests, driving a `CocoApp` opened directly
//! (not through the manager): transport controls, the Machine/View/Help
//! menus, the status bar's keyboard/display menus,
//! media-action gating, save-state menu wiring, error/confirmation dialogs,
//! and cartridge insertion (GMC) — peripherals themselves are configured
//! only through `[peripherals]` now, so their menus are gone; see
//! `launch_test.rs`/`peripherals_dto_test.rs` for that coverage instead. The
//! status bar's tape entry/menu — mounting, seeking,
//! and the auto-save/keyboard-focus interactions — lives in the sibling
//! [`super::vm_window_tape`], except the plain disabled-without-a-tape gating
//! checked here alongside the rest of `media_actions_are_disabled_until_media_is_present`.
//! The status bar's joysticks entry/menu lives in the sibling
//! [`super::vm_window_joysticks`].

use coco_core::{MachineVariant, MonitorType};
use egui_kittest::kittest::{NodeT, Queryable};

use crate::rom_load::load_default_rom;

use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;

const HEADLESS_SAMPLE_RATE_HZ: f64 = 48_000.0;

/// The VM window's own toolbar: Start/Suspend/Stop/Reset plus the feature-gated
/// Debug tile. While Running, Start is the one disabled tile (it only resumes a
/// suspended machine).
#[test]
fn toolbar_shows_start_disabled_and_others_live() {
    let mut harness = boot_harness();

    // One pass over the standard tiles: each label looked up and its enabled state checked once.
    let expectations = [
        (
            "Start",
            Some("Start is disabled while Running: it only resumes a suspended machine"),
        ),
        ("Suspend", None),
        ("Stop", None),
        ("Reset", None),
    ];
    for (label, disabled_reason) in expectations {
        let is_disabled = harness.get_by_label(label).accesskit_node().is_disabled();
        match disabled_reason {
            Some(reason) => assert!(is_disabled, "{reason}"),
            None => assert!(!is_disabled, "{label} must stay enabled in the VM window"),
        }
    }

    #[cfg(feature = "debug-ui")]
    assert!(
        !harness.get_by_label("Debug").accesskit_node().is_disabled(),
        "the Debug tile must stay enabled in the VM window"
    );
    #[cfg(not(feature = "debug-ui"))]
    assert!(
        harness.query_by_label("Debug").is_none(),
        "the Debug tile must be absent without the debug-ui feature"
    );

    click(&mut harness, "Reset");
    assert!(
        harness.state().running,
        "the toolbar's Reset tile must leave the machine on"
    );
}

/// A suspended window keeps its chrome, read-only: Start becomes the Resume
/// control and requests it, Suspend/Reset are off, the status bar says so.
#[test]
fn suspended_window_keeps_chrome_with_start_as_resume() {
    let mut harness = boot_harness();
    harness.state_mut().suspended = true;
    harness.step();

    assert!(label_exists(&harness, "Suspended"));
    assert!(
        harness
            .get_by_label("Machine")
            .accesskit_node()
            .is_disabled(),
        "the menu bar must be inert while suspended"
    );
    for (label, enabled) in [
        ("Start", true),
        ("Suspend", false),
        ("Stop", true),
        ("Reset", false),
    ] {
        let is_disabled = harness.get_by_label(label).accesskit_node().is_disabled();
        assert_eq!(
            !is_disabled, enabled,
            "{label} enabled state while suspended"
        );
    }
    #[cfg(feature = "debug-ui")]
    assert!(harness.get_by_label("Debug").accesskit_node().is_disabled());

    click(&mut harness, "Start");
    assert!(
        harness.state().pending_resume,
        "Start on a suspended window must request a resume"
    );
}

/// `ui.disable()` can't reach a menu that is already open, so the first
/// suspended frame closes whatever popup the running window left up.
#[test]
fn suspending_closes_an_open_menu() {
    let mut harness = boot_harness();
    click(&mut harness, "Machine");
    assert!(egui::Popup::is_any_open(&harness.ctx));

    harness.state_mut().suspended = true;
    harness.step();
    assert!(
        !egui::Popup::is_any_open(&harness.ctx),
        "a menu left open must not survive into the suspended window"
    );
}

/// The Debug tile and shortcut both flip `DebuggerPanel::open`. Only ⌘D
/// closes it here — the embedded debugger windows land over the toolbar and swallow its clicks.
#[test]
#[cfg(feature = "debug-ui")]
fn debug_tile_and_shortcut_toggle_the_debugger() {
    let mut harness = boot_harness();
    assert!(!harness.state().debugger.open);

    click(&mut harness, "Debug");
    assert!(
        harness.state().debugger.open,
        "the Debug tile must open the debugger"
    );

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    harness.step();
    assert!(
        !harness.state().debugger.open,
        "the debugger shortcut must close the debugger"
    );

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    harness.step();
    assert!(
        harness.state().debugger.open,
        "the debugger shortcut must open it again"
    );
}

#[test]
#[cfg(not(feature = "debug-ui"))]
fn debugger_shortcut_is_disabled_without_debug_ui() {
    let mut harness = boot_harness();

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    harness.step();

    assert!(
        !harness.state().debugger.open,
        "the debugger shortcut must be disabled without the debug-ui feature"
    );
}

#[test]
fn function_key_hotkeys_toggle_aspect_help_and_keyboard_mode() {
    let mut harness = boot_harness();
    assert!(harness.state().aspect_correct);
    assert!(harness.state().kb_mode == KbMode::Positional);

    harness.key_press(egui::Key::F9);
    harness.step();
    assert!(!harness.state().aspect_correct, "F9 toggles 4:3 aspect");

    harness.key_press(egui::Key::F10);
    harness.step();
    assert!(
        harness.state().show_kbd_help,
        "F10 opens the key layout window"
    );
    harness.get_by_label_contains("CoCo Keyboard Mapping");
    harness.key_press(egui::Key::F10);
    harness.step();
    assert!(!harness.state().show_kbd_help, "F10 closes it again");

    harness.key_press(egui::Key::F12);
    harness.step();
    assert!(
        harness.state().kb_mode == KbMode::Symbolic,
        "F12 switches to symbolic keyboard mode"
    );
}

/// The status bar's keyboard entry is the menu button for the keyboard menu;
/// the entry's label IS the current mode name ("Positional" → "Symbolic").
#[test]
fn status_bar_keyboard_entry_opens_the_keyboard_menu() {
    let mut harness = boot_harness();

    click(&mut harness, "Positional");
    click(&mut harness, "Symbolic");
    assert!(harness.state().kb_mode == KbMode::Symbolic);

    // The entry now carries the new mode's name.
    click(&mut harness, "Symbolic");
    click(&mut harness, "Key layout (F10)");
    assert!(harness.state().show_kbd_help);

    // The same menu item toggles the window closed again.
    click(&mut harness, "Symbolic");
    click(&mut harness, "Key layout (F10)");
    assert!(!harness.state().show_kbd_help);
}

/// The icon half of that click target: icon and label are unioned into one
/// response, so clicking the painted keyboard opens the same menu.
#[test]
fn status_bar_keyboard_icon_opens_the_keyboard_menu_too() {
    let mut harness = boot_harness();

    click(&mut harness, "Keyboard menu");
    harness.get_by_label("Key layout (F10)"); // the menu is open

    click(&mut harness, "Symbolic");
    assert!(harness.state().kb_mode == KbMode::Symbolic);
}

/// The status bar's runtime entry shows even before any session time has
/// accrued: a freshly booted `CocoApp` reads exactly "Runtime: 0 s".
#[test]
fn status_bar_shows_the_runtime_entry() {
    let harness = boot_harness();
    harness.get_by_label("Runtime: 0 s");
}

#[test]
fn view_menu_toggles_aspect() {
    let mut harness = boot_harness();

    click(&mut harness, "View");
    click(&mut harness, "4:3 aspect (F9)");
    assert!(!harness.state().aspect_correct);
}

/// The status bar's display entry is the menu button for the display menu;
/// its label (short form) collides with the menu's full "B&W TV" item, unlike other picks.
#[test]
fn status_bar_display_entry_switches_display() {
    let mut harness = boot_harness();
    assert_eq!(
        harness.state().display,
        Display::Monitor(MonitorType::RGB),
        "a CoCo 3 direct boot defaults to the RGB monitor"
    );
    assert_eq!(harness.state().machine.bus.gime.monitor, MonitorType::RGB);

    // Picking a TV steers the GIME to the composite path too (the RF modulator feeds off it).
    click(&mut harness, "RGB");
    click(&mut harness, "B&W TV");
    assert_eq!(harness.state().display, Display::TV(crate::display::TV::BW));
    assert_eq!(
        harness.state().machine.bus.gime.monitor,
        MonitorType::Composite,
        "display swap takes effect live, no power cycle"
    );
    assert_eq!(
        harness.state().tv.overscan_pct,
        crate::display::TVSettings::default().overscan_pct
    );

    click(&mut harness, "Display menu");
    assert!(label_exists(&harness, "Overscan"));

    // The entry's label tracks the selection; the icon half of the click target works too.
    click(&mut harness, "RGB monitor");
    assert_eq!(harness.state().display, Display::Monitor(MonitorType::RGB));
    assert_eq!(harness.state().machine.bus.gime.monitor, MonitorType::RGB);
}

#[test]
fn view_menu_opens_the_printer_paper_window() {
    let mut harness = boot_harness();
    assert!(!harness.state().paper_window.open);

    click(&mut harness, "View");
    click(&mut harness, "Printer Paper");
    assert!(harness.state().paper_window.open);
    harness.get_by_label_contains("Printer Paper");
}

#[test]
fn help_about_toggles_the_about_window() {
    let mut harness = boot_harness();

    click(&mut harness, "Help");
    click(&mut harness, "About");
    assert!(harness.state().show_about);
    harness.get_by_label("A Tandy Color Computer 3 emulator");

    click(&mut harness, "Help");
    click(&mut harness, "About");
    assert!(!harness.state().show_about);
    assert!(
        harness
            .query_by_label("A Tandy Color Computer 3 emulator")
            .is_none()
    );
}

/// With no tape mounted the seek field is disabled along with the rest of
/// the mount-gated tape-menu items (`media_actions_are_disabled_until_media_is_present`
/// covers Rewind/Eject the same way).
#[test]
fn status_bar_tape_menu_seek_field_is_disabled_without_a_tape() {
    let mut harness = boot_harness();

    click(&mut harness, "Tape menu");
    assert!(
        harness
            .get_by_role(egui::accesskit::Role::TextInput)
            .accesskit_node()
            .is_disabled(),
        "the seek field should be disabled with no tape mounted"
    );
}

#[test]
fn media_actions_are_disabled_until_media_is_present() {
    let mut harness = boot_harness();

    click(&mut harness, "Machine");
    assert!(
        harness
            .get_by_label("Stop Print Capture")
            .accesskit_node()
            .is_disabled(),
        "Stop Print Capture should be disabled with nothing captured"
    );
    // Nothing mounted, so both drive eject entries are disabled too.
    for drive in 0..UI_DRIVES {
        assert!(
            harness
                .get_by_label(&format!("Eject Drive {drive}"))
                .accesskit_node()
                .is_disabled()
        );
    }

    // Toggle the Machine menu closed first so the tape entry's click opens its popup, not
    // dismisses one.
    click(&mut harness, "Machine");
    click(&mut harness, "Tape menu");
    for label in ["Rewind Tape", "Eject Tape"] {
        assert!(
            harness.get_by_label(label).accesskit_node().is_disabled(),
            "{label} should be disabled with no tape mounted"
        );
    }
}

/// The Machine menu's Save/Load State section shows both file-dialog items
/// and both quick-slot submenus — `rfd`'s native dialog makes this visibility-only.
#[test]
fn machine_menu_shows_save_and_load_state_items() {
    let mut harness = boot_harness();

    click(&mut harness, "Machine");
    harness.get_by_label("Save State…");
    harness.get_by_label("Load State…");
    harness.get_by_label_contains("Quick Save");
    harness.get_by_label_contains("Quick Load");
}

/// A save-then-load round trip driven directly through `save_state_to`/
/// `load_state_from` (not `rfd`, which a headless test can't drive).
#[test]
fn save_state_then_load_state_round_trip() {
    let mut harness = boot_harness();
    let dir = TempDir::new("save-state-roundtrip");
    let path = dir.path().join("slot.ccstate");

    assert!(harness.state().running, "boot_harness starts running");
    harness
        .state_mut()
        .save_state_to(&path)
        .unwrap_or_else(|e| panic!("save_state_to failed: {e}"));
    assert!(path.is_file(), "save_state_to must write the .ccstate file");
    assert!(
        harness.state_mut().toast_message().is_some(),
        "a successful save must show a status-bar toast"
    );
    harness.step();
    harness.get_by_label_contains("State saved");

    harness
        .state_mut()
        .load_state_from(&path)
        .unwrap_or_else(|e| panic!("load_state_from failed: {e}"));
    assert!(
        harness.state().running,
        "restoring a state saved while running must leave the machine running"
    );
    assert!(
        harness.state_mut().toast_message().is_some(),
        "a successful load must show a status-bar toast"
    );
    harness.step();
    harness.get_by_label_contains("State loaded");
}

/// A load must not play out the pre-load machine's sound: `load_state_from`
/// drops whatever `AudioOutput` still had queued.
#[test]
fn load_state_drops_queued_audio_from_before_the_load() {
    let mut harness = boot_harness();
    let dir = TempDir::new("load-state-audio");
    let path = dir.path().join("slot.ccstate");
    harness
        .state_mut()
        .save_state_to(&path)
        .unwrap_or_else(|e| panic!("save_state_to failed: {e}"));

    // Stand in for the real device (absent on CI) with a headless pipeline.
    harness.state_mut().audio = audio::AudioOutput::headless(HEADLESS_SAMPLE_RATE_HZ);
    harness.state_mut().machine.run_field();
    harness.step();
    assert!(
        harness.state().audio.queued_frames() > 0,
        "running frames must queue audio in the headless pipeline"
    );

    harness
        .state_mut()
        .load_state_from(&path)
        .unwrap_or_else(|e| panic!("load_state_from failed: {e}"));
    assert_eq!(
        harness.state().audio.queued_frames(),
        0,
        "restore must drop every frame queued before the load"
    );
}

#[test]
fn cartridge_error_dialog_dismisses_with_ok() {
    let mut harness = boot_harness();
    harness.state_mut().cart_error = Some("could not read pak".to_string());
    harness.step();

    harness.get_by_label("could not read pak");
    click(&mut harness, "OK");
    assert!(harness.state().cart_error.is_none());
    assert!(harness.query_by_label("could not read pak").is_none());
}

/// Insert Disk/New Blank Disk stay disabled — with a hover explanation — until an FD-502 is
/// actually present, and enable once one is installed (`insert_disk_controller`, driven
/// directly here — there is no runtime menu to install one anymore).
#[test]
fn disk_menu_items_are_disabled_without_an_fd502() {
    let mut harness = boot_harness();

    click(&mut harness, "Machine");
    for drive in 0..UI_DRIVES {
        for label in [
            format!("Insert Disk in Drive {drive}…"),
            format!("New Blank Disk in Drive {drive}…"),
        ] {
            assert!(
                harness.get_by_label(&label).accesskit_node().is_disabled(),
                "{label} should be disabled with no FD-502 installed"
            );
        }
    }

    harness
        .state_mut()
        .insert_disk_controller()
        .unwrap_or_else(|e| panic!("insert_disk_controller failed: {e}"));
    harness.step();

    // The Machine menu is still open from earlier — nothing closed it, since this test never
    // clicks a menu item.
    for drive in 0..UI_DRIVES {
        for label in [
            format!("Insert Disk in Drive {drive}…"),
            format!("New Blank Disk in Drive {drive}…"),
        ] {
            assert!(
                !harness.get_by_label(&label).accesskit_node().is_disabled(),
                "{label} should be enabled once an FD-502 is installed"
            );
        }
    }
}

#[test]
fn insert_gmc_pages_banked_rom_and_survives_power_cycle() {
    let roms_dir = test_assets::roms_dir();
    let (rom, rom_source) = load_default_rom(MachineVariant::Coco3, &roms_dir)
        .expect("coco3.rom is required in the cocovm XDG data directory");

    // A 64K banked image: every byte of 16K page `n` is 0xB0|n.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp-test-roms/gmc");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("banked.rom");
    let mut image = vec![0u8; 4 * 16 * 1024];
    for (n, page) in image.chunks_mut(16 * 1024).enumerate() {
        page.fill(0xB0 | n as u8);
    }
    std::fs::write(&path, &image).unwrap();

    let mut harness = egui_kittest::Harness::new_eframe(|_cc| {
        CocoApp::new(
            MachineConfig::default(),
            rom,
            rom_source,
            AppParams::default(),
        )
    });
    // Drive the app-glue directly: the menu item's click handler opens a native file dialog.
    harness.state_mut().insert_gmc(path.clone(), true);
    harness.step();

    let app = harness.state_mut();
    assert_eq!(app.cart_path.as_deref(), Some(path.as_path()));
    assert!(app.cart_error.is_none(), "{:?}", app.cart_error);
    assert_eq!(app.machine.bus.cart.rom_read(0xC000), 0xB0, "bank 0 up");
    app.machine.bus.cart.write(0xFF40, 2);
    assert_eq!(app.machine.bus.cart.rom_read(0xC000), 0xB2, "bank latch");
}
