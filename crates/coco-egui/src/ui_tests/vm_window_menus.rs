//! VM window menu/toolbar/hotkey tests, driving a `CocoApp` opened directly
//! (not through the manager): transport controls, the Machine/View/Help
//! menus, the status bar's keyboard/display menus,
//! media-action gating, the MultiPak install/slot/switch flow, save-state
//! menu wiring, error/confirmation dialogs, the RS-232 pak, and cartridge
//! insertion (GMC). The status bar's tape entry/menu — mounting, seeking,
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

#[test]
fn machine_menu_reset_keeps_the_ui_alive() {
    let mut harness = boot_harness();

    click(&mut harness, "Machine");
    click_in_menu(&mut harness, "Reset");
    assert!(harness.state().running, "Reset leaves the machine on");
    assert_eq!(
        harness.state().machine.config.variant,
        MachineVariant::Coco3
    );
}

/// The VM window's own toolbar: Start/Suspend/Stop/Reset plus the VM-only
/// Debug tile. Start stays permanently disabled (a chrome-bearing window only exists while
/// Running).
#[test]
fn toolbar_shows_start_disabled_and_others_live() {
    let mut harness = boot_harness();

    // One pass over all five tiles: each label looked up and its enabled state checked once.
    let expectations = [
        (
            "Start",
            Some(
                "Start is always disabled in the VM window: a chrome-bearing window only exists while Running",
            ),
        ),
        ("Suspend", None),
        ("Stop", None),
        ("Reset", None),
        ("Debug", None),
    ];
    for (label, disabled_reason) in expectations {
        let is_disabled = harness.get_by_label(label).accesskit_node().is_disabled();
        match disabled_reason {
            Some(reason) => assert!(is_disabled, "{reason}"),
            None => assert!(!is_disabled, "{label} must stay enabled in the VM window"),
        }
    }

    click(&mut harness, "Reset");
    assert!(
        harness.state().running,
        "the toolbar's Reset tile must leave the machine on"
    );
}

/// The Debug tile and shortcut both flip `DebuggerPanel::open`. Only ⌘D
/// closes it here — the embedded debugger windows land over the toolbar and swallow its clicks.
#[test]
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
    for label in ["Eject Cartridge", "Stop Print Capture"] {
        assert!(
            harness.get_by_label(label).accesskit_node().is_disabled(),
            "{label} should be disabled with nothing inserted"
        );
    }
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

#[test]
fn multipak_install_slot_and_switch_flow() {
    let mut harness = boot_harness();

    click(&mut harness, "Machine");
    click_containing(&mut harness, "MultiPak Interface");
    click(&mut harness, "Insert MultiPak");
    {
        let app = harness.state();
        let mpi = app.mpi.as_ref().expect("MPI should be installed");
        assert_eq!(mpi.switch, DEFAULT_MPI_SWITCH_SLOT);
        assert!(mpi.slots.iter().all(|s| matches!(s, MPISlot::Empty)));
    }
    harness.get_by_label("MPI [S1:- S2:- S3:- S4:-]"); // status bar

    // Plug the FD-502 into slot 1 through the nested slot submenu.
    click(&mut harness, "Machine");
    click_containing(&mut harness, "MultiPak Interface");
    click_containing(&mut harness, "Slot 1");
    click(&mut harness, "Insert FD-502");
    assert!(matches!(
        harness.state().mpi.as_ref().unwrap().slots[0],
        MPISlot::FD502
    ));
    assert!(
        harness
            .state_mut()
            .machine
            .bus
            .cart
            .as_disk_cart()
            .is_some(),
        "the FD-502 in an MPI slot must be reachable through the cart chain"
    );

    // Move the front-panel switch to slot 2; the parent menu's own entry is "Slot 2 ⏵", not
    // "Slot 2".
    click(&mut harness, "Machine");
    click_containing(&mut harness, "MultiPak Interface");
    click_containing(&mut harness, "Switch");
    click(&mut harness, "Slot 2");
    assert_eq!(harness.state().mpi.as_ref().unwrap().switch, 1);

    click(&mut harness, "Machine");
    click_containing(&mut harness, "MultiPak Interface");
    click(&mut harness, "Remove MultiPak");
    let app = harness.state();
    assert!(app.mpi.is_none());
    assert!(app.disk_paths.iter().all(Option::is_none));
}

#[test]
fn machine_menu_checkbox_toggles_cartridge_autostart() {
    let mut harness = boot_harness();
    assert!(harness.state().autostart_cart);

    click(&mut harness, "Machine");
    click(&mut harness, "Auto-start cartridge");
    assert!(!harness.state().autostart_cart);
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
    harness.state_mut().audio = audio::AudioOutput::headless(48_000.0);
    for _ in 0..8 {
        harness.step();
    }
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
/// actually present, and enable once the MultiPak Interface flow installs one.
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

    // Plug an FD-502 into an MPI slot, the only way to get one at runtime.
    click_containing(&mut harness, "MultiPak Interface");
    click(&mut harness, "Insert MultiPak");
    click(&mut harness, "Machine");
    click_containing(&mut harness, "MultiPak Interface");
    click_containing(&mut harness, "Slot 1");
    click(&mut harness, "Insert FD-502");

    click(&mut harness, "Machine");
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

/// Machine ▸ Deluxe RS-232 Pak ▸ Insert plugs the pak in on the loopback
/// endpoint and the status bar reports it; Remove restores the empty slot.
#[test]
fn rs232_menu_inserts_and_removes_the_pak() {
    let mut harness = boot_harness();

    click(&mut harness, "Machine");
    // Not click_submenu: its substring match would also hit the Insert/Remove items once open.
    click(&mut harness, "Deluxe RS-232 Pak ⏵");
    click(&mut harness, "Insert Deluxe RS-232 Pak");
    {
        let app = harness.state_mut();
        assert!(matches!(app.rs232, Some(RS232Endpoint::Loopback)));
        assert!(
            app.machine.bus.cart.as_deluxe_rs232().is_some(),
            "the pak must be reachable behind the trait object"
        );
    }
    harness.step();
    assert!(
        harness.query_by_label("RS-232 [loopback] ↑0 ↓0").is_some(),
        "status bar should describe the pak and its endpoint"
    );

    click(&mut harness, "Machine");
    // Not click_submenu: its substring match would also hit the Insert/Remove items once open.
    click(&mut harness, "Deluxe RS-232 Pak ⏵");
    click(&mut harness, "Remove Deluxe RS-232 Pak");
    let app = harness.state_mut();
    assert!(app.rs232.is_none());
    assert!(app.machine.bus.cart.as_deluxe_rs232().is_none());
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
    harness.state_mut().insert_gmc(path.clone());
    harness.step();

    let app = harness.state_mut();
    assert_eq!(app.cart_path.as_deref(), Some(path.as_path()));
    assert!(app.cart_error.is_none(), "{:?}", app.cart_error);
    assert_eq!(app.machine.bus.cart.rom_read(0xC000), 0xB0, "bank 0 up");
    app.machine.bus.cart.write(0xFF40, 2);
    assert_eq!(app.machine.bus.cart.rom_read(0xC000), 0xB2, "bank latch");
}
