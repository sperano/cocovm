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

use crate::machine_def::tests::TempDir;
use crate::*;

type AppHarness = egui_kittest::Harness<'static, CocoApp>;
type ManagerHarness = egui_kittest::Harness<'static, manager::ManagerApp>;

/// Boot a default (CoCo 3) machine into a kittest harness, exactly as
/// `main()` would with no CLI arguments.
fn boot_harness() -> AppHarness {
    let roms_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms");
    let rom = load_default_rom(MachineVariant::Coco3, &roms_dir)
        .expect("roms/coco3.rom is required (git-ignored, local-only)");
    let mut harness = egui_kittest::Harness::new_eframe(|_cc| {
        CocoApp::new(
            MachineConfig::default(),
            rom,
            None,
            [None, None],
            [None, None],
            std::array::from_fn(|_| None),
            false,
            false,
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
/// fires `clicked` on the release. Generic over the app type so the same
/// helper drives both `CocoApp` and `manager::ManagerApp` harnesses.
fn click<S: 'static>(harness: &mut egui_kittest::Harness<'static, S>, label: &str) {
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

/// The VDG radio row only exists on a CoCo 2 draft
/// (`MachineConfig::validate`: [`VDGVariant::MC6847T1`] is CoCo2-only) —
/// the row is absent with CoCo 1 or CoCo 3 selected, present and
/// selectable with CoCo 2 selected.
#[test]
fn new_dialog_vdg_row_only_visible_for_coco2() {
    let mut harness = boot_harness();

    click(&mut harness, "Machine");
    click(&mut harness, "New…");

    let t1_label = "MC6847T1 (CoCo 2B)";

    // Default draft is CoCo 3 (`MachineConfig::default`): row absent.
    assert!(
        harness.query_by_label(t1_label).is_none(),
        "VDG row must be absent for CoCo 3"
    );

    click(&mut harness, "CoCo 2");
    assert!(
        harness.query_by_label(t1_label).is_some(),
        "VDG row must be present for CoCo 2"
    );

    click(&mut harness, "CoCo 1");
    assert!(
        harness.query_by_label(t1_label).is_none(),
        "VDG row must be absent for CoCo 1"
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

/// The manager window scaffold: toolbar buttons present (and inert), the
/// machine-list panel and photo pane laid out without a photo injected.
#[test]
fn manager_window_shows_its_toolbar() {
    let mut harness =
        egui_kittest::Harness::new_eframe(|_cc| manager::ManagerApp::new(None, None, None, Vec::new()));
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();

    for label in ["New…", "Settings", "Help"] {
        harness.get_by_label(label);
    }
    // The buttons are scaffolding: clicking must be a no-op, not a panic.
    // (Same hover-then-click convention as `click`, which is typed for the
    // CocoApp harness.)
    harness.get_by_label("New…").hover();
    harness.step();
    harness.get_by_label("New…").click();
    harness.step();
    harness.step();
}

/// The divider between the machine list and the photo pane must be
/// draggable. Regression test for the empty-panel gotcha: a `SidePanel`
/// whose ui claims no space silently loses its resize drag
/// (`SidePanel::resizable` docs — hence `take_available_space` in
/// `ManagerApp::update`).
#[test]
fn manager_list_divider_is_draggable() {
    let mut harness =
        egui_kittest::Harness::new_eframe(|_cc| manager::ManagerApp::new(None, None, None, Vec::new()));
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();

    let panel_id = egui::Id::new("manager_machine_list");
    let width = |harness: &egui_kittest::Harness<'_, manager::ManagerApp>| {
        egui::containers::panel::PanelState::load(&harness.ctx, panel_id)
            .expect("machine-list panel state exists")
            .rect
            .width()
    };
    let before = width(&harness);

    // Grab the divider (the panel's right edge) at mid-height and drag it
    // 80 px right: hover, press, move while pressed, release.
    let grab = egui::pos2(before, 360.0);
    let target = egui::pos2(before + 80.0, 360.0);
    harness.hover_at(grab);
    harness.step();
    harness.drag_at(grab);
    harness.step();
    harness.hover_at(target);
    harness.step();
    harness.drop_at(target);
    harness.step();
    harness.step();

    let after = width(&harness);
    assert!(
        (after - before) > 60.0,
        "dragging the divider must widen the list panel (before {before}, after {after})"
    );
}

/// With a photo injected, the manager uploads it on the first frame and
/// keeps rendering (the image itself is not an accessible node — this
/// guards the upload path against panics/regressions).
#[test]
fn manager_window_renders_an_injected_photo() {
    let photo = photo_view::Photo {
        title: "test-photo".to_string(),
        pixels: egui::ColorImage::from_rgba_unmultiplied([8, 6], &[0x20; 8 * 6 * 4]),
    };
    let mut harness = egui_kittest::Harness::new_eframe(|_cc| {
        manager::ManagerApp::new(Some(photo), None, None, Vec::new())
    });
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();
    harness.step();
}

/// A minimal valid `Ok` entry: a CoCo 3 default config under `name`, built
/// through [`machine_def::MachineDef::from_config`] like the manager's own
/// "New…" flow does, so tests don't hand-roll a second copy of the DTO
/// shape.
fn sample_entry(slug: &str, name: &str) -> manager::MachineEntry {
    manager::MachineEntry::new(
        slug.to_string(),
        Ok(machine_def::MachineDef::from_config(
            name.to_string(),
            None,
            &MachineConfig::default(),
        )),
    )
}

/// Boot a manager harness with injected entries and (optionally) a real
/// machines directory for Create/Save to write into — never the user's real
/// config dir.
fn manager_harness(machines_dir: Option<PathBuf>, entries: Vec<manager::MachineEntry>) -> ManagerHarness {
    manager_harness_with_artifacts(machines_dir, None, entries)
}

/// [`manager_harness`] with the artifact root injected too — for tests that
/// exercise thumbnail persistence (always a temp dir, never the real
/// `data_dir()`).
fn manager_harness_with_artifacts(
    machines_dir: Option<PathBuf>,
    artifacts_root: Option<PathBuf>,
    entries: Vec<manager::MachineEntry>,
) -> ManagerHarness {
    let mut harness = egui_kittest::Harness::new_eframe(move |_cc| {
        manager::ManagerApp::new(None, machines_dir, artifacts_root, entries)
    });
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();
    harness
}

/// Selecting a row shows its detail pane, seeded from that entry's
/// definition — not whatever the previously-selected row left behind.
#[test]
fn manager_list_shows_entries_and_selecting_shows_detail() {
    let entries = vec![sample_entry("alpha", "Alpha CoCo 3"), sample_entry("beta", "Beta CoCo 3")];
    let mut harness = manager_harness(None, entries);

    harness.get_by_label("Alpha CoCo 3");
    harness.get_by_label("Beta CoCo 3");
    assert_eq!(harness.state().detail_name(), None, "nothing selected yet");

    click(&mut harness, "Beta CoCo 3");
    assert_eq!(harness.state().selected, Some(1));
    assert_eq!(harness.state().detail_name(), Some("Beta CoCo 3"));

    click(&mut harness, "Alpha CoCo 3");
    assert_eq!(harness.state().selected, Some(0));
    assert_eq!(
        harness.state().detail_name(),
        Some("Alpha CoCo 3"),
        "switching rows must reseed the draft, not keep editing the old one"
    );
}

/// "New…" opens the dialog; "Create" writes a `.toml` definition to the
/// injected machines dir and adds a list row — without booting anything (the
/// manager has no launch action at all yet, so there is nothing to assert
/// beyond "no machine-running side effect exists to trigger").
#[test]
fn manager_new_dialog_create_writes_a_definition_file() {
    let dir = TempDir::new("create");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());
    assert!(harness.state().entries.is_empty());

    click(&mut harness, "New…");
    harness.get_by_label("Create"); // dialog open

    click(&mut harness, "Create");

    assert_eq!(harness.state().entries.len(), 1, "Create must add a list row");
    let slug = harness.state().entries[0].slug.clone();
    assert_eq!(slug, "coco-3", "slugified from the default draft name");
    assert!(harness.state().entries[0].def.is_ok());

    let file = dir.path().join(format!("{slug}.toml"));
    let contents = fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    let parsed: machine_def::MachineDef =
        toml::from_str(&contents).expect("Create must write a parseable definition");
    assert_eq!(parsed.name, "CoCo 3");
    assert_eq!(harness.state().selected, Some(0), "Create must select the new row");
    // Not `get_by_label("CoCo 3")`: the now-visible detail pane's hardware
    // form has its own "CoCo 3" *model* radio button, so the name would be
    // ambiguous between that and the list row.
    assert_eq!(harness.state().detail_name(), Some("CoCo 3"));

    assert!(
        harness.query_by_label("Create").is_none(),
        "the dialog should close after a successful Create"
    );
}

/// Editing a hardware field and saving rewrites the definition file; Revert
/// discards the in-progress edit instead of writing it.
#[test]
fn manager_detail_save_rewrites_file_and_revert_discards_edit() {
    let dir = TempDir::new("save-revert");
    let entry = sample_entry("dev-coco-3", "Dev CoCo 3");
    let def = entry.def.clone().unwrap();
    machine_def::save(dir.path(), "dev-coco-3", &def).expect("seed the file the entry claims to be");
    assert!(!def.peripherals.mpi, "test assumes the sample starts without an MPI");

    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![entry]);
    click(&mut harness, "Dev CoCo 3");
    harness.get_by_label("Save"); // clean draft: no "*" yet
    assert!(harness.query_by_label("Save*").is_none());

    click(&mut harness, "MultiPak Interface"); // toggle a peripheral checkbox
    harness.get_by_label("Save*"); // now dirty

    click(&mut harness, "Save*");
    assert!(
        harness.query_by_label("Save*").is_none(),
        "a clean save must drop the dirty indicator"
    );
    let file = dir.path().join("dev-coco-3.toml");
    let saved: machine_def::MachineDef =
        toml::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert!(saved.peripherals.mpi, "Save must persist the toggled checkbox");

    // Toggle it off again without saving, then revert: the in-memory draft
    // must go back to the saved (mpi = true) state, and the file — which
    // Revert never touches — must be untouched too.
    click(&mut harness, "MultiPak Interface");
    harness.get_by_label("Save*");
    click(&mut harness, "Revert");
    assert!(
        harness.query_by_label("Save*").is_none(),
        "Revert must restore the clean (saved) draft"
    );
    let after_revert: machine_def::MachineDef =
        toml::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert!(after_revert.peripherals.mpi, "Revert must not touch the file");
}

/// A definition that failed to parse/validate shows its error instead of an
/// editable form, and selecting it doesn't panic (nothing to edit, but the
/// row must still be selectable like any other).
#[test]
fn manager_error_entry_shows_badge_and_is_selectable_without_panicking() {
    let entries = vec![manager::MachineEntry::new(
        "broken".to_string(),
        Err("hardware/schema-3: unsupported schema".to_string()),
    )];
    let mut harness = manager_harness(None, entries);

    harness.get_by_label("broken");
    harness.get_by_label_contains("⚠");

    click(&mut harness, "broken");
    assert_eq!(harness.state().selected, Some(0));
    assert_eq!(harness.state().detail_name(), None, "an Err entry has nothing to edit");
}

/// A minimal valid CoCo 2 `Ok` entry — `sample_entry`'s default is CoCo 3,
/// so pairing this with it gives two distinct machine families for
/// `starting_two_machines_runs_both` (`docs/plan-machine-persistence.md`
/// step 5's acceptance scenario: "a CoCo 3 and a newly created, launched
/// CoCo 2").
fn sample_coco2_entry(slug: &str, name: &str) -> manager::MachineEntry {
    manager::MachineEntry::new(
        slug.to_string(),
        Ok(machine_def::MachineDef::from_config(
            name.to_string(),
            None,
            &MachineConfig {
                variant: MachineVariant::Coco2,
                video: VideoStandard::NTSC,
                memory: MemorySize::K64,
                monitor: MonitorType::RGB,
                vdg: VDGVariant::MC6847T1,
            },
        )),
    )
}

/// Whether `label` matches at least one accessible node — unlike
/// `get_by_label`/`query_by_label` (which require *at most* one match), used
/// where a status word like "Running" is deliberately shown twice at once
/// (the list row's `weak()` copy and the detail pane header's `strong()`
/// copy, both driven by `manager::vm_status_label`).
fn label_exists<S: 'static>(harness: &egui_kittest::Harness<'static, S>, label: &str) -> bool {
    harness.get_all_by_label(label).next().is_some()
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
    let entries = vec![manager::MachineEntry::new("broken-media".to_string(), Ok(def))];
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
