//! Headless end-to-end drive of the full app through `egui_kittest`: real
//! `eframe::App::update` frames, with clicks and key presses dispatched
//! through the AccessKit tree — the closest a test gets to a user at the real
//! window. Like `coco-core`'s boot tests, these need the git-ignored local
//! `./roms`.
//!
//! Interaction conventions discovered the hard way:
//! - Clicks hover on one frame and press/release on the next: egui routes a
//!   press using the previous frame's hit-test data, so a press with no
//!   prior hover misses windows that were (re)anchored this frame.
//! - Menus close on *any* item click (egui's default menu close behavior),
//!   so every menu interaction reopens the menu from the bar.
//! - Submenu buttons expose their label with a trailing "⏵" arrow — match
//!   them with `_contains`, not exactly.

use egui_kittest::kittest::{NodeT, Queryable};

use crate::*;

type AppHarness = egui_kittest::Harness<'static, CocoApp>;

/// Boot a default (CoCo 3) machine into a kittest harness, exactly as
/// `main()` would with no CLI arguments.
fn boot_harness() -> AppHarness {
    let roms_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms");
    let rom = load_default_rom(MachineVariant::Coco3, &roms_dir)
        .expect("roms/coco3.rom is required (git-ignored, local-only)");
    let mut harness = egui_kittest::Harness::new_eframe(|cc| {
        CocoApp::new(
            cc,
            MachineConfig::default(),
            rom,
            None,
            [None, None],
            [None, None],
            false,
        )
    });
    // Room for the full Machine menu: egui only puts on-screen widgets in
    // the AccessKit tree, so a too-small viewport hides the lower items.
    harness.set_size(egui::vec2(1024.0, 768.0));
    harness.step();
    harness
}

/// Click the widget labelled exactly `label`: hover one frame (see module
/// docs), then press and release across the following two frames — egui
/// fires `clicked` on the release.
fn click(harness: &mut AppHarness, label: &str) {
    harness.get_by_label(label).hover();
    harness.step();
    harness.get_by_label(label).click();
    harness.step();
    harness.step();
}

/// [`click`] for submenu buttons, whose accessible label carries the
/// trailing "⏵" arrow (e.g. `"MultiPak Interface ⏵"`).
fn click_submenu(harness: &mut AppHarness, label: &str) {
    harness.get_by_label_contains(label).hover();
    harness.step();
    harness.get_by_label_contains(label).click();
    harness.step();
    harness.step();
}

/// Lowest-on-screen widget labelled `label` — the open-menu copy of a label
/// the toolbar shows too ("Pause", "Reset"): the menu popup hangs below the
/// toolbar row.
fn lowest_by_label<'t>(harness: &'t AppHarness, label: &'t str) -> egui_kittest::Node<'t> {
    harness
        .get_all_by_label(label)
        .max_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y))
        .unwrap_or_else(|| panic!("no node labelled {label:?}"))
}

/// Topmost widget labelled `label` — e.g. the right stick's copy of a source
/// label the Joysticks menu lists once per stick.
fn topmost_by_label<'t>(harness: &'t AppHarness, label: &'t str) -> egui_kittest::Node<'t> {
    harness
        .get_all_by_label(label)
        .min_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y))
        .unwrap_or_else(|| panic!("no node labelled {label:?}"))
}

/// [`click`] via [`lowest_by_label`].
fn click_in_menu(harness: &mut AppHarness, label: &str) {
    lowest_by_label(harness, label).hover();
    harness.step();
    lowest_by_label(harness, label).click();
    harness.step();
    harness.step();
}

#[test]
fn new_dialog_creates_a_coco1_machine_without_panicking() {
    let roms_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms");
    load_default_rom(MachineVariant::Coco1, &roms_dir)
        .expect("a roms/bas1x.rom Color BASIC dump is required (git-ignored, local-only)");

    let mut harness = boot_harness();
    assert_eq!(harness.state().machine.config.variant, MachineVariant::Coco3);

    click(&mut harness, "Machine");
    click(&mut harness, "New…");
    click(&mut harness, "CoCo 1");
    // The frame that processes Create draws the CentralPanel *after*
    // swapping the machine — the exact path that used to panic on the
    // framebuffer texture.
    click(&mut harness, "Create");

    let app = harness.state();
    assert_eq!(app.machine.config.variant, MachineVariant::Coco1);
    assert_eq!(
        app.machine.config.memory,
        MemorySize::K64,
        "RAM should snap to the CoCo 1/2 default when the model changes"
    );
    assert!(app.running, "a new VM boots running, like startup");
    assert!(app.cart_path.is_none() && app.mpi.is_none());
    assert!(
        harness.query_by_label("Create").is_none(),
        "the New Machine dialog should close after a successful create"
    );
}

#[test]
fn new_dialog_cancel_leaves_the_machine_untouched() {
    let mut harness = boot_harness();

    click(&mut harness, "Machine");
    click(&mut harness, "New…");
    // Editing the draft must not leak into the running machine.
    click(&mut harness, "CoCo 2");
    click(&mut harness, "Cancel");

    let app = harness.state();
    assert_eq!(app.machine.config.variant, MachineVariant::Coco3);
    assert!(app.running, "cancelling New… must not pause the machine");
    assert!(
        harness.query_by_label("Create").is_none(),
        "the New Machine dialog should close on Cancel"
    );
}

#[test]
fn toolbar_pause_and_resume_update_the_status_bar() {
    let mut harness = boot_harness();
    assert!(harness.state().running);
    harness.get_by_label("Running"); // status bar

    // With no menu open, the toolbar's copy of the button is the only one.
    click(&mut harness, "Pause");
    assert!(!harness.state().running);
    harness.get_by_label("Paused");

    click(&mut harness, "Run");
    assert!(harness.state().running);
    harness.get_by_label("Running");
}

#[test]
fn machine_menu_pause_and_reset_keep_the_ui_alive() {
    let mut harness = boot_harness();

    click(&mut harness, "Machine");
    click_in_menu(&mut harness, "Pause");
    assert!(!harness.state().running);

    // Reset while paused: the machine restarts but stays paused — the two
    // controls are independent, like the reset button on the real case.
    click(&mut harness, "Machine");
    click_in_menu(&mut harness, "Reset");
    assert!(!harness.state().running);
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
    assert_eq!(harness.state().machine.bus.gime.monitor, MonitorType::Rgb);

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
    assert_eq!(harness.state().machine.bus.gime.monitor, MonitorType::Rgb);
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
    click_submenu(&mut harness, "MultiPak Interface");
    click(&mut harness, "Insert MultiPak");
    {
        let app = harness.state();
        let mpi = app.mpi.as_ref().expect("MPI should be installed");
        assert_eq!(mpi.switch, DEFAULT_MPI_SWITCH_SLOT);
        assert!(mpi.slots.iter().all(|s| matches!(s, MpiSlot::Empty)));
    }
    harness.get_by_label("MPI [S1:- S2:- S3:- S4:-]"); // status bar

    // Plug the FD-502 into slot 1 through the nested slot submenu.
    click(&mut harness, "Machine");
    click_submenu(&mut harness, "MultiPak Interface");
    click_submenu(&mut harness, "Slot 1");
    click(&mut harness, "Insert FD-502");
    assert!(matches!(
        harness.state().mpi.as_ref().unwrap().slots[0],
        MpiSlot::Fd502
    ));
    assert!(
        harness.state_mut().machine.bus.cart.as_disk_cart().is_some(),
        "the FD-502 in an MPI slot must be reachable through the cart chain"
    );

    // Move the front-panel switch to slot 2. The Switch popup's exact
    // "Slot 2" is unique — the parent menu's slot entry is a submenu button
    // labelled "Slot 2 ⏵".
    click(&mut harness, "Machine");
    click_submenu(&mut harness, "MultiPak Interface");
    click_submenu(&mut harness, "Switch");
    click(&mut harness, "Slot 2");
    assert_eq!(harness.state().mpi.as_ref().unwrap().switch, 1);

    click(&mut harness, "Machine");
    click_submenu(&mut harness, "MultiPak Interface");
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
