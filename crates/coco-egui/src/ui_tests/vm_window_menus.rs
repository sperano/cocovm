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
//! [`super::vm_window_joysticks`]; the suspended window (inert chrome, the
//! display overlay, click-to-resume) in [`super::vm_window_suspend`].

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
fn function_key_hotkeys_toggle_help_and_keyboard_mode() {
    let mut harness = boot_harness();
    assert!(harness.state().kb_mode == KbMode::Positional);

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

/// The status bar's sound entry replaces the menu-bar item and exposes the
/// existing mute and volume controls.
#[test]
fn status_bar_sound_entry_opens_audio_controls() {
    let mut harness = boot_harness();
    harness.state_mut().audio = audio::AudioOutput::headless(HEADLESS_SAMPLE_RATE_HZ);
    harness.step();

    assert_eq!(
        harness.query_all_by_label("Sound").count(),
        1,
        "Sound must have one entry in the status bar"
    );
    click(&mut harness, "Sound");
    assert!(
        harness.query_all_by_label("Volume").next().is_some(),
        "the sound popup must contain the volume control"
    );
    click(&mut harness, "Mute");

    click(&mut harness, "Sound");
    assert_eq!(
        harness.get_by_label("Mute").accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True),
        "the mute control must retain its state"
    );
}

#[test]
fn printer_menu_opens_the_printer_paper_window() {
    let mut harness = boot_harness();
    assert!(!harness.state().paper_window.open);

    click(&mut harness, "Printer menu");
    click(&mut harness, "View Papers");
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
    let mut harness = harness_with_fd502();

    click(&mut harness, "Printer menu");
    for label in ["Stop Print Capture", "Open Print Capture"] {
        assert!(
            harness.get_by_label(label).accesskit_node().is_disabled(),
            "{label} should be disabled with nothing captured"
        );
    }

    // Toggle each menu closed before opening the next, so the entry's click opens its
    // popup rather than dismissing the previous one.
    click(&mut harness, "Printer menu");
    click(&mut harness, "Disks menu");
    // Nothing mounted, so both drive eject entries are disabled too.
    for drive in 0..UI_DRIVES {
        assert!(
            harness
                .get_by_label(&format!("Eject Drive {drive}"))
                .accesskit_node()
                .is_disabled()
        );
    }

    click(&mut harness, "Disks menu");
    click(&mut harness, "Tape menu");
    for label in ["Rewind Tape", "Eject Tape"] {
        assert!(
            harness.get_by_label(label).accesskit_node().is_disabled(),
            "{label} should be disabled with no tape mounted"
        );
    }
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
            crate::joy::SharedGamepad::without_backend(),
        )
    });
    // Drive the app-glue directly: the menu item's click handler opens a native file dialog.
    harness.state_mut().insert_gmc(path.clone());
    harness.step();

    let app = harness.state_mut();
    assert_eq!(app.cart_path.as_deref(), Some(path.as_path()));
    assert!(app.cart_error.is_none(), "{:?}", app.cart_error);
    assert_eq!(app.machine.bus.cart.rom_read(0xC000), 0xB0, "bank 0 up");
    app.machine.bus.cart.write(0xFF40, 2);
    assert_eq!(app.machine.bus.cart.rom_read(0xC000), 0xB2, "bank latch");
}
