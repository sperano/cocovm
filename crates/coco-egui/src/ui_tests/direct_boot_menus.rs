//! Direct-boot `CocoApp` menu/toolbar/hotkey tests: transport controls,
//! Machine/Keyboard/View/Help/Joysticks menus, media-action gating, the
//! MultiPak install/slot/switch flow, save-state menu wiring, error/
//! confirmation dialogs, the RS-232 pak, and cartridge insertion (GMC).

use coco_core::MachineVariant;
use egui_kittest::kittest::{NodeT, Queryable};

use crate::rom_load::load_default_rom;

use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;

/// The user-facing Run/Pause toggle is gone everywhere (three-state model,
/// user decision 2026-07-27): no toolbar button, no Machine-menu item, no
/// "Running"/"Paused" status-bar label — regression coverage against any of
/// them creeping back.
#[test]
fn pause_controls_are_gone_from_the_chrome() {
    let mut harness = boot_harness();
    assert!(harness.state().running);
    assert!(harness.query_by_label("Pause").is_none(), "no toolbar Pause");
    assert!(harness.query_by_label("Running").is_none(), "no status-bar run state");

    click(&mut harness, "Machine");
    assert!(harness.query_by_label("Pause").is_none(), "no menu Pause");
    // The console Reset survives — in the toolbar and the open menu alike.
    assert!(label_exists(&harness, "Reset"));
}

#[test]
fn machine_menu_reset_keeps_the_ui_alive() {
    let mut harness = boot_harness();

    click(&mut harness, "Machine");
    click_in_menu(&mut harness, "Reset");
    assert!(harness.state().running, "Reset leaves the machine on");
    assert_eq!(harness.state().machine.config.variant, MachineVariant::Coco3);
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
    assert!(harness.state().show_kbd_help, "F10 opens the key layout window");
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
    harness.get_by_label("Keyboard: Symbolic (F12)"); // status bar follows
}

#[test]
fn keyboard_menu_selects_mode_and_opens_key_layout() {
    let mut harness = boot_harness();

    click(&mut harness, "Keyboard");
    click(&mut harness, "Symbolic");
    assert!(harness.state().kb_mode == KbMode::Symbolic);

    click(&mut harness, "Keyboard");
    click(&mut harness, "Key layout (F10)");
    assert!(harness.state().show_kbd_help);

    // The toolbar shortcut toggles the same window closed.
    click(&mut harness, "⌨ Keys (F10)");
    assert!(!harness.state().show_kbd_help);
}

#[test]
fn view_menu_toggles_aspect_and_switches_monitor_type() {
    let mut harness = boot_harness();
    assert_eq!(harness.state().machine.bus.gime.monitor, MonitorType::RGB);

    click(&mut harness, "View");
    click(&mut harness, "4:3 aspect (F9)");
    assert!(!harness.state().aspect_correct);

    click(&mut harness, "View");
    click(&mut harness, "Composite monitor");
    assert_eq!(
        harness.state().machine.bus.gime.monitor,
        MonitorType::Composite,
        "monitor swap takes effect live, no power cycle"
    );

    click(&mut harness, "View");
    click(&mut harness, "RGB monitor");
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
    assert!(harness
        .query_by_label("A Tandy Color Computer 3 emulator")
        .is_none());
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
    for label in ["Eject Cartridge", "Eject Tape", "Rewind Tape", "Stop Print Capture"] {
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
        harness.state_mut().machine.bus.cart.as_disk_cart().is_some(),
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

    // Cloned out first (cheap — `egui::Context` is `Arc`-backed): `state_mut()`
    // borrows all of `harness` mutably for the call below, which would
    // conflict with a `&harness.ctx` argument evaluated in the same
    // expression.
    let ctx = harness.ctx.clone();
    harness
        .state_mut()
        .load_state_from(&path, &ctx)
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
        assert!(matches!(app.rs232, Some(Rs232Endpoint::Loopback)));
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

    let rom_source = RomSource::File(roms_dir.join("coco3.rom"));
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
