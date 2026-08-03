//! VM window menu/toolbar/hotkey tests, driving a `CocoApp` opened directly
//! (not through the manager): transport controls, Machine/View/Help/
//! Joysticks menus, the status bar's keyboard menu, media-action gating, the
//! MultiPak install/slot/switch flow, save-state menu wiring, error/
//! confirmation dialogs, the RS-232 pak, and cartridge insertion (GMC).

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

/// The VM window's own toolbar: the same four transport tiles
/// (Start/Suspend/Stop/Reset) the manager toolbar draws, via the shared
/// `toolbar_button` widget. A direct boot is never manager-owned
/// (`CocoApp::managed` stays `false` unless `launch_machine` sets it), and a
/// chrome-bearing VM window only ever exists while Running, so Start and
/// Suspend are permanently disabled here; Stop and Reset stay live. With no
/// menu open, "Reset" can only be the toolbar tile — the Machine menu's own
/// "Reset" item only joins the accessibility tree while that menu is open
/// (`harness.rs`'s `lowest_by_label` doc covers the collision once it is).
#[test]
fn toolbar_shows_transport_tiles_disabled_for_direct_boot() {
    let mut harness = boot_harness();

    // One pass over all four tiles: each label is looked up (asserting it
    // exists) exactly once, and its enabled/disabled state checked in the
    // same step rather than re-querying "Start"/"Suspend" a second time.
    let expectations = [
        (
            "Start",
            Some(
                "Start is always disabled in the VM window: a chrome-bearing window only exists while Running",
            ),
        ),
        (
            "Suspend",
            Some("Suspend needs the manager (CocoApp::managed); a direct boot has none"),
        ),
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

    click(&mut harness, "Reset");
    assert!(
        harness.state().running,
        "the toolbar's Reset tile must leave the machine on"
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

/// The status bar's keyboard entry is the menu button for the keyboard menu
/// (`CocoApp::keyboard_menu_ui`), so the mode is changed from the same entry
/// that names the device. Its items are only in the accessibility tree while
/// the popup is open, so an exact "Symbolic"/"Key layout (F10)" match is
/// unambiguous — and "Keyboard" is the entry alone, the menu bar having no
/// Keyboard menu of its own.
#[test]
fn status_bar_keyboard_entry_opens_the_keyboard_menu() {
    let mut harness = boot_harness();

    click(&mut harness, "Keyboard");
    click(&mut harness, "Symbolic");
    assert!(harness.state().kb_mode == KbMode::Symbolic);

    click(&mut harness, "Keyboard");
    click(&mut harness, "Key layout (F10)");
    assert!(harness.state().show_kbd_help);

    // The same menu item toggles the window closed again.
    click(&mut harness, "Keyboard");
    click(&mut harness, "Key layout (F10)");
    assert!(!harness.state().show_kbd_help);
}

/// The icon half of that click target: icon and label are unioned into
/// one response, so clicking the painted keyboard opens the same menu. The
/// icon is painted rather than built from a widget, so what puts it in the
/// accessibility tree under "Keyboard menu" is `keyboard_status`'s own
/// `widget_info` call.
#[test]
fn status_bar_keyboard_icon_opens_the_keyboard_menu_too() {
    let mut harness = boot_harness();

    click(&mut harness, "Keyboard menu");
    harness.get_by_label("Key layout (F10)"); // the menu is open

    click(&mut harness, "Symbolic");
    assert!(harness.state().kb_mode == KbMode::Symbolic);
}

#[test]
fn view_menu_toggles_aspect_and_switches_display() {
    let mut harness = boot_harness();
    assert_eq!(
        harness.state().display,
        Display::Monitor(MonitorType::RGB),
        "a CoCo 3 direct boot defaults to the RGB monitor"
    );
    assert_eq!(harness.state().machine.bus.gime.monitor, MonitorType::RGB);

    click(&mut harness, "View");
    click(&mut harness, "4:3 aspect (F9)");
    assert!(!harness.state().aspect_correct);

    // Picking a TV steers the GIME to the composite path too: the TV hangs
    // off the RF modulator, which is fed the composite signal.
    click(&mut harness, "View");
    click(&mut harness, "B&W TV");
    assert_eq!(harness.state().display, Display::TV(crate::display::TV::BW));
    assert_eq!(
        harness.state().machine.bus.gime.monitor,
        MonitorType::Composite,
        "display swap takes effect live, no power cycle"
    );

    click(&mut harness, "View");
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

#[test]
fn status_bar_shows_no_joystick_entries_by_default() {
    let mut harness = boot_harness();
    harness.step();
    // Default config: both sticks off (`JoySource::None`) — see
    // `JoystickInputs::new`. `joystick_status` only ever emits a "JR"/"JL"
    // entry for a port whose source isn't `JoySource::None`, so neither
    // should appear until the Joysticks menu assigns one.
    assert!(
        harness.query_by_label_contains("JR:").is_none(),
        "the right stick's source is None by default, so it shouldn't get a status-bar entry"
    );
    assert!(
        harness.query_by_label_contains("JL:").is_none(),
        "the left stick's source is None by default, so it shouldn't get a status-bar entry"
    );
}

#[test]
fn joysticks_menu_assigns_a_source_to_the_right_stick() {
    let mut harness = boot_harness();

    click(&mut harness, "Joysticks");
    // Both sticks list the same four source labels; the right stick's list
    // is drawn first, so its "Keys" entry is the topmost one.
    topmost_by_label(&harness, "Keys").hover();
    harness.step();
    topmost_by_label(&harness, "Keys").click();
    harness.step();
    harness.step();

    let app = harness.state();
    assert!(app.joysticks.sources[coco_core::joystick::RIGHT] == joy::JoySource::Keys);
    assert!(app.joysticks.sources[coco_core::joystick::LEFT] == joy::JoySource::None);
}

#[test]
fn media_actions_are_disabled_until_media_is_present() {
    let mut harness = boot_harness();

    click(&mut harness, "Machine");
    for label in [
        "Eject Cartridge",
        "Eject Tape",
        "Rewind Tape",
        "Stop Print Capture",
    ] {
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

    // Move the front-panel switch to slot 2. The Switch popup's exact
    // "Slot 2" is unique — the parent menu's slot entry is a submenu button
    // labelled "Slot 2 ⏵".
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

/// The Machine menu's Save/Load State section ([`crate::save_state`]) shows
/// both file-dialog items and both quick-slot submenus — the section itself
/// is mostly untestable headlessly (`rfd` opens a native dialog), so this
/// just covers visibility/wiring; the actual save/load round trip is
/// exercised directly through `save_state_to`/`load_state_from` below.
#[test]
fn machine_menu_shows_save_and_load_state_items() {
    let mut harness = boot_harness();

    click(&mut harness, "Machine");
    harness.get_by_label("Save State…");
    harness.get_by_label("Load State…");
    harness.get_by_label_contains("Quick Save");
    harness.get_by_label_contains("Quick Load");
}

/// A save-then-load round trip driven directly through
/// `save_state_to`/`load_state_from` against a temp file (not `rfd`, which a
/// headless test can't drive): the machine keeps running across the load,
/// and both the save and the load show a status-bar toast.
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
fn disk_controller_confirmation_can_be_cancelled() {
    let mut harness = boot_harness();
    // The state a menu Insert Disk lands in when no FD-502 is installed
    // (reached directly — the menu path itself opens a native file dialog).
    harness.state_mut().pending_disk_action = Some(PendingDiskAction::Insert {
        drive: 0,
        path: PathBuf::from("nonexistent.dsk"),
    });
    harness.step();

    harness.get_by_label_contains("Insert disk controller?");
    click(&mut harness, "Cancel");
    let app = harness.state();
    assert!(app.pending_disk_action.is_none());
    assert!(
        app.disk_paths[0].is_none(),
        "cancelling must not mount the disk"
    );
}

/// Machine ▸ Deluxe RS-232 Pak ▸ Insert plugs the pak in on the loopback
/// endpoint, reachable behind the trait object, and the status bar reports
/// it; Remove restores the empty slot.
#[test]
fn rs232_menu_inserts_and_removes_the_pak() {
    let mut harness = boot_harness();

    click(&mut harness, "Machine");
    // Not `click_submenu`: its substring match would also hit the
    // "Insert/Remove Deluxe RS-232 Pak" items once hovering opens the
    // submenu, so match the arrow-suffixed label exactly.
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
    // Not `click_submenu`: its substring match would also hit the
    // "Insert/Remove Deluxe RS-232 Pak" items once hovering opens the
    // submenu, so match the arrow-suffixed label exactly.
    click(&mut harness, "Deluxe RS-232 Pak ⏵");
    click(&mut harness, "Remove Deluxe RS-232 Pak");
    let app = harness.state_mut();
    assert!(app.rs232.is_none());
    assert!(app.machine.bus.cart.as_deluxe_rs232().is_none());
}

#[test]
fn insert_gmc_pages_banked_rom_and_survives_power_cycle() {
    let roms_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms");
    let rom = load_default_rom(MachineVariant::Coco3, &roms_dir)
        .expect("roms/coco3.rom is required (git-ignored, local-only)");

    // A 64K banked image: every byte of 16K page `n` is 0xB0|n.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp-test-roms/gmc");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("banked.rom");
    let mut image = vec![0u8; 4 * 16 * 1024];
    for (n, page) in image.chunks_mut(16 * 1024).enumerate() {
        page.fill(0xB0 | n as u8);
    }
    std::fs::write(&path, &image).unwrap();

    let rom_source = ROMSource::File(roms_dir.join("coco3.rom"));
    let mut harness = egui_kittest::Harness::new_eframe(|_cc| {
        CocoApp::new(
            MachineConfig::default(),
            rom,
            rom_source,
            None,
            [None, None],
            [None, None],
            std::array::from_fn(|_| None),
            false,
            false,
            false,
        )
    });
    // Drive the app-glue directly (the menu item's click handler opens a
    // native file dialog, which a headless test can't answer).
    harness.state_mut().insert_gmc(path.clone());
    harness.step();

    let app = harness.state_mut();
    assert_eq!(app.cart_path.as_deref(), Some(path.as_path()));
    assert!(app.cart_error.is_none(), "{:?}", app.cart_error);
    assert_eq!(app.machine.bus.cart.rom_read(0xC000), 0xB0, "bank 0 up");
    app.machine.bus.cart.write(0xFF40, 2);
    assert_eq!(app.machine.bus.cart.rom_read(0xC000), 0xB2, "bank latch");
}
