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

/// [`click`] with the secondary button — opens the machine-list rows'
/// context menu.
fn right_click<S: 'static>(harness: &mut egui_kittest::Harness<'static, S>, label: &str) {
    harness.get_by_label(label).hover();
    harness.step();
    harness.get_by_label(label).click_secondary();
    harness.step();
    harness.step();
}

/// [`click`] matching by substring — for widgets whose accessible label
/// carries decoration beyond the visible caption: submenu buttons ("MultiPak
/// Interface ⏵") and menu rows with shortcut text ("New… ⌘N").
fn click_containing<S: 'static>(harness: &mut egui_kittest::Harness<'static, S>, label: &str) {
    harness.get_by_label_contains(label).hover();
    harness.step();
    harness.get_by_label_contains(label).click();
    harness.step();
    harness.step();
}

/// Select an item in a form combo box (Machine, Cartridge, …): click the
/// combo button to open the popup, then click the wanted item. The combo
/// button exposes the selected text as its accessibility *value* (egui sets
/// `WidgetInfo::current_text_value`, not a label), so it is addressed with
/// `get_by_value`; the popup items are plain selectables, addressed by label.
fn select_combo<S: 'static>(
    harness: &mut egui_kittest::Harness<'static, S>,
    current: &str,
    target: &str,
) {
    harness.get_by_value(current).hover();
    harness.step();
    harness.get_by_value(current).click();
    harness.step();
    harness.step();
    click(harness, target);
}

/// [`select_combo`] disambiguated by position: among all combo buttons
/// currently showing `current` as their value, open the `index`-th in
/// top-to-bottom (then left-to-right) screen order. Needed once more than
/// one drive combo shows "None" at the same time.
fn select_combo_at<S: 'static>(
    harness: &mut egui_kittest::Harness<'static, S>,
    current: &str,
    index: usize,
    target: &str,
) {
    fn nth<'t, S>(
        harness: &'t egui_kittest::Harness<'static, S>,
        value: &'t str,
        index: usize,
    ) -> egui_kittest::Node<'t> {
        let mut nodes: Vec<_> = harness.get_all_by_value(value).collect();
        nodes.sort_by(|a, b| {
            (a.rect().min.y.total_cmp(&b.rect().min.y))
                .then(a.rect().min.x.total_cmp(&b.rect().min.x))
        });
        nodes
            .into_iter()
            .nth(index)
            .unwrap_or_else(|| panic!("no {index}-th node with value {value:?}"))
    }
    nth(harness, current, index).hover();
    harness.step();
    nth(harness, current, index).click();
    harness.step();
    harness.step();
    click(harness, target);
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
    click_containing(&mut harness, "New…");
    select_combo(&mut harness, "CoCo 3", "CoCo 1");
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

/// The Cartridge row: default None leaves the port empty; picking FD-502
/// inserts a disk controller after the machine boots. The combo is driven
/// by accessibility value like the Machine one ([`select_combo`]'s doc).
#[test]
fn new_dialog_cartridge_row_inserts_fd502() {
    let mut harness = boot_harness();
    assert!(harness.state_mut().machine.bus.cart.as_disk_cart().is_none());

    // Default (None) leaves the port empty across a create. The HD (VHD)
    // rows are always visible — a bus device, not cartridge hardware.
    click(&mut harness, "Machine");
    click_containing(&mut harness, "New…");
    for drive in 0..UI_DRIVES {
        assert!(
            harness.query_by_label(&format!("HD {drive}")).is_some(),
            "HD {drive} row must be visible with Cartridge = None"
        );
    }
    click(&mut harness, "Create");
    assert!(harness.state_mut().machine.bus.cart.as_disk_cart().is_none());

    click(&mut harness, "Machine");
    click_containing(&mut harness, "New…");
    select_combo_at(&mut harness, "None", 1, "FD-502");
    // Selecting the FD-502 reveals the Disk 0 companion combo (default
    // None); its Blank/Select… items open native file dialogs, so a
    // headless test only exercises visibility.
    assert!(
        harness.query_by_label("Disk 0:").is_some(),
        "FD-502 selection must reveal the Disk 0 combo"
    );
    assert!(
        harness.query_by_label("Disk 1:").is_some(),
        "FD-502 selection must reveal the Disk 1 combo"
    );
    // The Disk rows are an indented sub-form under the Cartridge combo:
    // both "Disk N:" labels start at the same x, shifted right of the
    // combo's left edge.
    let disk0_x = harness.get_by_label("Disk 0:").rect().min.x;
    let disk1_x = harness.get_by_label("Disk 1:").rect().min.x;
    assert!(
        (disk0_x - disk1_x).abs() < 1.0,
        "Disk 1 ({disk1_x}) must line up under Disk 0 ({disk0_x})"
    );
    let cartridge_combo = harness.get_by_value("FD-502").rect();
    assert!(
        disk0_x > cartridge_combo.min.x,
        "Disk 0 ({disk0_x}) must indent past the Cartridge combo ({})",
        cartridge_combo.min.x
    );
    // The vertical gap above the Disk sub-form must stay the ordinary row
    // gap — `horizontal_top` in `sub_form_row` exists precisely because a
    // centering wrapper once pushed the block ~14px down.
    let disk0_combo_top = harness
        .get_all_by_value("None")
        .map(|n| n.rect().min.y)
        .filter(|y| *y > cartridge_combo.max.y)
        .fold(f32::INFINITY, f32::min);
    let gap = disk0_combo_top - cartridge_combo.max.y;
    assert!(
        gap <= new_vm::FORM_GRID_SPACING[1] + 1.0,
        "gap above Disk 0 ({gap}) must not exceed the form's row spacing"
    );
    click(&mut harness, "Create");
    assert!(
        harness.state_mut().machine.bus.cart.as_disk_cart().is_some(),
        "creating with Cartridge = FD-502 must insert the disk controller"
    );
    assert!(harness.state().cart_error.is_none());

    // MultiPak Interface: the four Slot rows appear, and the Disk rows
    // only once an FD-502 occupies a slot.
    click(&mut harness, "Machine");
    click_containing(&mut harness, "New…");
    select_combo_at(&mut harness, "None", 1, "MultiPak Interface");
    for slot in 1..=4 {
        assert!(
            harness.query_by_label(&format!("Slot {slot}:")).is_some(),
            "MPI selection must reveal Slot {slot}"
        );
    }
    assert!(
        harness.query_by_label("Disk 0:").is_none(),
        "no Disk rows until an FD-502 is slotted"
    );
    // All four slot combos read "Empty", stacked top to bottom, so index 1
    // is Slot 2.
    select_combo_at(&mut harness, "Empty", 1, "FD-502");
    assert!(
        harness.query_by_label("Disk 0:").is_some(),
        "a slotted FD-502 must reveal the Disk rows"
    );
    // The Disk rows nest under the slot that holds the FD-502: indented
    // past the Slot labels, between Slot 2 and Slot 3.
    let disk0 = harness.get_by_label("Disk 0:").rect();
    let slot2 = harness.get_by_label("Slot 2:").rect();
    let slot3 = harness.get_by_label("Slot 3:").rect();
    assert!(
        disk0.min.x > slot2.min.x,
        "Disk 0 ({}) must indent past the Slot labels ({})",
        disk0.min.x,
        slot2.min.x
    );
    assert!(
        disk0.min.y > slot2.min.y && disk0.min.y < slot3.min.y,
        "Disk 0 (y {}) must sit under its owning Slot 2 (y {}), above Slot 3 (y {})",
        disk0.min.y,
        slot2.min.y,
        slot3.min.y
    );
    click(&mut harness, "Create");
    assert!(harness.state().mpi.is_some(), "creating with MPI must insert one");
    assert!(
        harness.state_mut().machine.bus.cart.as_disk_cart().is_some(),
        "the slotted FD-502 must be reachable through the MPI"
    );

    // Back to Cartridge = None: the Disk 0 combo disappears.
    click(&mut harness, "Machine");
    click_containing(&mut harness, "New…");
    select_combo_at(&mut harness, "None", 1, "FD-502");
    select_combo(&mut harness, "FD-502", "None");
    assert!(
        harness.query_by_label("Disk 0:").is_none(),
        "the Disk 0 combo must vanish when the FD-502 is deselected"
    );
    click(&mut harness, "Cancel");
}

/// Cartridge = ROM Pak: creating inserts the pak in the port (direct), or
/// in its chosen MPI slot (slotted). The "ROM Pak…" combo item opens a
/// native file dialog a headless harness cannot drive, so the picked path
/// is seeded on the dialog directly (the reason `new_vm` is reachable).
#[test]
fn new_dialog_rom_pak_choice_inserts_the_pak() {
    let pak = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/disk11.rom");
    assert!(pak.is_file(), "roms/disk11.rom is required (git-ignored, local-only)");

    // Straight in the port.
    let mut harness = boot_harness();
    click(&mut harness, "Machine");
    click_containing(&mut harness, "New…");
    harness.state_mut().new_vm.form.cartridge = new_vm::CartridgeChoice::RomPak(pak.clone());
    click(&mut harness, "Create");
    assert_eq!(harness.state().cart_error, None);
    assert_eq!(harness.state().cart_path.as_deref(), Some(pak.as_path()));
    assert!(harness.state().mpi.is_none());

    // Slotted in the MPI.
    click(&mut harness, "Machine");
    click_containing(&mut harness, "New…");
    select_combo_at(&mut harness, "None", 1, "MultiPak Interface");
    harness.state_mut().new_vm.form.mpi_slots[2] = new_vm::SlotChoice::RomPak(pak.clone());
    click(&mut harness, "Create");
    assert_eq!(harness.state().cart_error, None);
    assert!(harness.state().mpi.is_some(), "creating with MPI must insert one");
    assert!(
        matches!(
            &harness.state().mpi.as_ref().unwrap().slots[2],
            MPISlot::ROMPak(p) if p == &pak
        ),
        "the pak must land in the chosen slot"
    );
}

/// The dialog opens at [`new_vm::DIALOG_MIN_SIZE`] and must never grow:
/// revealing the FD-502's Disk rows or the MPI's full Slot+Disk block has
/// to fit inside the minimum. The window node (labelled by its title)
/// spans the whole frame, so its rect pins both size and position.
#[test]
fn new_dialog_window_never_resizes_when_rows_appear() {
    let mut harness = boot_harness();
    click(&mut harness, "Machine");
    click_containing(&mut harness, "New…");
    let baseline = harness.get_by_label("New Machine").rect();

    select_combo_at(&mut harness, "None", 1, "FD-502");
    assert_eq!(
        harness.get_by_label("New Machine").rect(),
        baseline,
        "revealing the Disk rows must not resize the window"
    );

    select_combo(&mut harness, "FD-502", "MultiPak Interface");
    select_combo_at(&mut harness, "Empty", 0, "FD-502");
    assert!(harness.query_by_label("Disk 0:").is_some());
    assert_eq!(
        harness.get_by_label("New Machine").rect(),
        baseline,
        "the full MPI Slot+Disk block must fit inside the dialog's minimum size"
    );
    click(&mut harness, "Cancel");
}

#[test]
fn new_dialog_cancel_leaves_the_machine_untouched() {
    let mut harness = boot_harness();

    click(&mut harness, "Machine");
    click_containing(&mut harness, "New…");
    // Editing the draft must not leak into the running machine.
    select_combo(&mut harness, "CoCo 3", "CoCo 2");
    click(&mut harness, "Cancel");

    let app = harness.state();
    assert_eq!(app.machine.config.variant, MachineVariant::Coco3);
    assert!(app.running, "cancelling New… must not pause the machine");
    assert!(
        harness.query_by_label("Create").is_none(),
        "the New Machine dialog should close on Cancel"
    );
}

/// The VDG combo row only exists on a CoCo 2 draft
/// (`MachineConfig::validate`: [`VDGVariant::MC6847T1`] is CoCo2-only) —
/// the row is absent with CoCo 1 or CoCo 3 selected, present with CoCo 2
/// selected. The row is probed through the combo button's accessibility
/// *value*: a CoCo 2 draft defaults to the T1 (`default_vdg`), so its
/// closed combo shows the T1 text.
#[test]
fn new_dialog_vdg_row_only_visible_for_coco2() {
    let mut harness = boot_harness();

    click(&mut harness, "Machine");
    click_containing(&mut harness, "New…");

    let t1_label = "MC6847T1 (CoCo 2B)";

    // Default draft is CoCo 3 (`MachineConfig::default`): row absent.
    assert!(
        harness.query_by_value(t1_label).is_none(),
        "VDG row must be absent for CoCo 3"
    );

    select_combo(&mut harness, "CoCo 3", "CoCo 2");
    assert!(
        harness.query_by_value(t1_label).is_some(),
        "VDG row must be present for CoCo 2"
    );

    select_combo(&mut harness, "CoCo 2", "CoCo 1");
    assert!(
        harness.query_by_value(t1_label).is_none(),
        "VDG row must be absent for CoCo 1"
    );
}

/// ⌘N / Ctrl+N opens the New-machine dialog without touching the menu, in
/// both apps. `Modifiers::COMMAND` in the harness matches what
/// `consume_shortcut` looks for on every platform, so this exercises the
/// mac/Windows/Linux binding in one test.
#[test]
fn cmd_n_triggers_new_machine_in_both_flows() {
    let mut harness = boot_harness();
    assert!(harness.query_by_label("Create").is_none());

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::N);
    harness.step();
    harness.step();
    assert!(
        harness.query_by_label("Create").is_some(),
        "Cmd/Ctrl+N must open the New Machine dialog"
    );

    // The manager has no dialog: Cmd/Ctrl+N creates a machine on the spot
    // (same instant-create as the toolbar's "New…").
    let dir = TempDir::new("cmd-n-manager");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());
    assert!(harness.state().entries.is_empty());

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::N);
    harness.step();
    harness.step();
    assert_eq!(
        harness.state().entries.len(),
        1,
        "Cmd/Ctrl+N must create a machine immediately in the manager"
    );
    assert_eq!(harness.state().selected, Some(0));
    assert!(dir.path().join("coco-3.toml").is_file());
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
    click_containing(&mut harness,"MultiPak Interface");
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
    click_containing(&mut harness,"MultiPak Interface");
    click_containing(&mut harness,"Slot 1");
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
    click_containing(&mut harness,"MultiPak Interface");
    click_containing(&mut harness,"Switch");
    click(&mut harness, "Slot 2");
    assert_eq!(harness.state().mpi.as_ref().unwrap().switch, 1);

    click(&mut harness, "Machine");
    click_containing(&mut harness,"MultiPak Interface");
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

/// The manager window scaffold: toolbar buttons present, the machine-list
/// panel and photo pane laid out without a photo injected. This harness has
/// no machines dir (no home), so "New…" must report that instead of
/// creating or panicking.
#[test]
fn manager_window_shows_its_toolbar() {
    let mut harness =
        egui_kittest::Harness::new_eframe(|_cc| manager::ManagerApp::new(None, None, None, Vec::new()));
    harness.set_size(egui::vec2(1080.0, 720.0));
    harness.step();

    for label in ["New…", "Settings", "Help"] {
        harness.get_by_label(label);
    }
    harness.get_by_label("New…").hover();
    harness.step();
    harness.get_by_label("New…").click();
    harness.step();
    harness.step();
    assert!(
        harness.state().entries.is_empty(),
        "no config dir: New… must fail gracefully, not add a row"
    );
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

/// A minimal valid entry: a CoCo 3 default config under `name`, built
/// through [`machine_def::MachineDef::from_config`] like the manager's own
/// "New…" flow does, so tests don't hand-roll a second copy of the DTO
/// shape.
fn sample_entry(slug: &str, name: &str) -> manager::MachineEntry {
    manager::MachineEntry::new(
        slug.to_string(),
        machine_def::MachineDef::from_config(name.to_string(), None, &MachineConfig::default()),
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

/// Right-clicking a list row opens its context menu *without* moving the
/// visual selection — the menu's items act on the row under the cursor, not
/// on `selected` (user decision 2026-07-23). The one exception is "Show
/// config", whose whole job is to select; it (like every pick) also closes
/// the menu.
#[test]
fn manager_row_right_click_opens_context_menu_without_selecting() {
    let entries = vec![sample_entry("alpha", "Alpha CoCo 3"), sample_entry("beta", "Beta CoCo 3")];
    let mut harness = manager_harness(None, entries);
    assert!(harness.query_by_label("Show config").is_none(), "menu must start closed");

    click(&mut harness, "Alpha CoCo 3");
    assert_eq!(harness.state().selected, Some(0));

    right_click(&mut harness, "Beta CoCo 3");
    assert_eq!(
        harness.state().selected,
        Some(0),
        "right-click must leave the selection cue where it was"
    );

    click(&mut harness, "Show config");
    assert_eq!(
        harness.state().selected,
        Some(1),
        "Show config selects the right-clicked row, not the old selection"
    );
    assert!(harness.query_by_label("Show config").is_none(), "picking an item closes the menu");
}

/// The context menu's "Delete…" asks for confirmation first: Cancel keeps
/// the machine untouched; Delete removes the list row and its `<slug>.toml`,
/// and the selection follows the surviving row as indices shift.
#[test]
fn manager_row_context_menu_delete_confirms_and_removes() {
    let dir = TempDir::new("ctx-delete");
    let entries = vec![sample_entry("alpha", "Alpha CoCo 3"), sample_entry("beta", "Beta CoCo 3")];
    for entry in &entries {
        machine_def::save(dir.path(), &entry.slug, &entry.def).expect("seed definition files");
    }
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), entries);

    click(&mut harness, "Beta CoCo 3");
    assert_eq!(harness.state().selected, Some(1));

    right_click(&mut harness, "Alpha CoCo 3");
    click(&mut harness, "Delete…");
    click(&mut harness, "Cancel");
    assert_eq!(harness.state().entries.len(), 2, "Cancel must keep the machine");
    assert!(dir.path().join("alpha.toml").exists(), "Cancel must keep the definition file");

    right_click(&mut harness, "Alpha CoCo 3");
    click(&mut harness, "Delete…");
    click(&mut harness, "Delete");
    assert_eq!(harness.state().entries.len(), 1);
    assert!(!dir.path().join("alpha.toml").exists(), "the definition file must be removed");
    assert!(dir.path().join("beta.toml").exists(), "only the confirmed machine is deleted");
    assert_eq!(
        harness.state().detail_name(),
        Some("Beta CoCo 3"),
        "the selection must follow the surviving row as indices shift"
    );
}

/// "New…" creates the machine immediately; picking Cartridge = FD-502 in
/// the detail pane's form auto-saves `[peripherals].fd502` into the
/// definition file — no Save button involved.
#[test]
fn manager_edit_with_fd502_records_the_peripheral() {
    let dir = TempDir::new("create-fd502");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New…");
    // Pane combos showing "None": Cassette (its row sits above Cartridge),
    // then Cartridge, then the HDs — Cartridge is second.
    select_combo_at(&mut harness, "None", 1, "FD-502");

    assert_eq!(harness.state().entries.len(), 1);
    assert!(harness.state().entries[0].def.peripherals.fd502);
    let file = dir.path().join("coco-3.toml");
    let contents = fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    assert!(
        contents.contains("fd502 = true"),
        "the TOML must record the peripheral:\n{contents}"
    );
}

/// Cartridge = MultiPak Interface in the pane records the mpi peripheral
/// (no slotted FD-502 → no fd502 flag).
#[test]
fn manager_edit_with_mpi_records_the_peripheral() {
    let dir = TempDir::new("create-mpi");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New…");
    select_combo_at(&mut harness, "None", 1, "MultiPak Interface");

    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert!(def.peripherals.mpi);
    assert!(!def.peripherals.fd502);
    let contents = fs::read_to_string(dir.path().join("coco-3.toml")).unwrap();
    assert!(contents.contains("mpi = true"), "TOML must record the MPI:\n{contents}");
}

/// Cartridge = Disto RTC: creating plugs the clock into the port (direct)
/// or the chosen MPI slot — fully UI-driven, no file dialog involved.
#[test]
fn new_dialog_rtc_choice_inserts_the_clock() {
    let mut harness = boot_harness();

    // Straight in the port.
    click(&mut harness, "Machine");
    click_containing(&mut harness, "New…");
    select_combo_at(&mut harness, "None", 1, "Disto RTC");
    click(&mut harness, "Create");
    assert_eq!(harness.state().cart_error, None);
    assert!(harness.state().rtc_direct, "the RTC must sit in the port");
    assert!(harness.state_mut().machine.bus.cart.as_disto_rtc().is_some());

    // Slotted in the MPI: Slot 3 (index 2 among the "Empty" combos).
    click(&mut harness, "Machine");
    click_containing(&mut harness, "New…");
    select_combo_at(&mut harness, "None", 1, "MultiPak Interface");
    select_combo_at(&mut harness, "Empty", 2, "Disto RTC");
    click(&mut harness, "Create");
    assert_eq!(harness.state().cart_error, None);
    assert!(harness.state().mpi.is_some(), "creating with MPI must insert one");
    assert!(
        matches!(harness.state().mpi.as_ref().unwrap().slots[2], MPISlot::DistoRTC),
        "the clock must land in the chosen slot"
    );
    assert!(harness.state_mut().machine.bus.cart.as_disto_rtc().is_some());
}

/// A Disto RTC — in the port or slotted — records `peripherals.rtc` (the
/// schema keeps no slot layout; launch re-seats a slotted clock in its
/// default slot). Fully UI-driven through the pane's combos.
#[test]
fn manager_edit_with_rtc_records_the_peripheral() {
    let dir = TempDir::new("create-rtc");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New…");
    select_combo_at(&mut harness, "None", 1, "Disto RTC");
    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert!(def.peripherals.rtc && !def.peripherals.mpi);
    let contents = fs::read_to_string(dir.path().join("coco-3.toml")).unwrap();
    assert!(contents.contains("rtc = true"), "TOML must record the RTC:\n{contents}");

    // Slotted, on a second machine: rtc = true alongside mpi = true.
    click_containing(&mut harness, "New…");
    select_combo_at(&mut harness, "None", 1, "MultiPak Interface");
    select_combo_at(&mut harness, "Empty", 0, "Disto RTC");
    assert_eq!(harness.state().entries.len(), 2);
    let def = &harness.state().entries[1].def;
    assert!(def.peripherals.rtc && def.peripherals.mpi);
}

/// A ROM Pak — in the port or in an MPI slot — records `[media].cart`; two
/// slotted paks exceed what the schema can represent, so that change is
/// refused with an inline error and nothing is saved. Picks are seeded on
/// the edit form directly (native file dialogs, see
/// `new_dialog_rom_pak_choice_inserts_the_pak`).
#[test]
fn manager_edit_with_rom_pak_records_the_cart() {
    let pak = PathBuf::from("/paks/game.ccc");
    let dir = TempDir::new("create-rompak");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New…");
    harness.state_mut().edit_form_mut().expect("pane form seeded").cartridge =
        new_vm::CartridgeChoice::RomPak(pak.clone());
    harness.step();
    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert_eq!(def.media.cart.as_deref(), Some("/paks/game.ccc"));
    assert!(!def.peripherals.mpi && !def.peripherals.fd502);

    // Slotted, on a second machine: recorded the same way, alongside
    // mpi = true.
    click_containing(&mut harness, "New…");
    {
        let form = harness.state_mut().edit_form_mut().expect("pane form seeded");
        form.cartridge = new_vm::CartridgeChoice::MPI;
        form.mpi_slots[1] = new_vm::SlotChoice::RomPak(pak.clone());
    }
    harness.step();
    assert_eq!(harness.state().entries.len(), 2);
    let def = &harness.state().entries[1].def;
    assert_eq!(def.media.cart.as_deref(), Some("/paks/game.ccc"));
    assert!(def.peripherals.mpi);

    // Two slotted paks cannot be represented in the schema: the change is
    // refused with an inline error instead of silently dropping one, and
    // the definition keeps the single recorded pak.
    harness.state_mut().edit_form_mut().expect("pane form seeded").mpi_slots[3] =
        new_vm::SlotChoice::RomPak(pak);
    harness.step();
    harness.get_by_label_contains("a single ROM Pak");
    assert_eq!(
        harness.state().entries[1].def.media.cart.as_deref(),
        Some("/paks/game.ccc"),
        "the definition must keep the single recorded pak"
    );
}

/// Disk 0 = Blank in the pane: a 0-byte blank image lands in the machine's
/// artifact dir the moment it's picked, and `[media].disk0` records it by
/// relative path. (The manager flow's Blank is the picker-free one, so this
/// drives the whole path headlessly — direct boot's Blank opens a native
/// save dialog.)
#[test]
fn manager_edit_with_blank_disk0_places_it_in_the_artifact_dir() {
    let machines = TempDir::new("create-blank-machines");
    let artifacts = TempDir::new("create-blank-artifacts");
    let mut harness = manager_harness_with_artifacts(
        Some(machines.path().to_path_buf()),
        Some(artifacts.path().to_path_buf()),
        Vec::new(),
    );

    click_containing(&mut harness, "New…");
    select_combo_at(&mut harness, "None", 1, "FD-502");
    // Screen order of the "None"-valued combos: Cassette (its row sits
    // above Cartridge), then Disk 0, then Disk 1, then HD 0, then HD 1.
    // A picked Blank is auto-placed on the spot and its combo then shows
    // the placed file's name, so it leaves the "None" pool immediately.
    select_combo_at(&mut harness, "None", 1, "Blank");
    // Disk 0 now reads "disk0.dsk"; remaining "None"s: Cassette, Disk 1, HDs.
    select_combo_at(&mut harness, "None", 1, "Blank");
    // The cassette (topmost remaining "None").
    select_combo_at(&mut harness, "None", 0, "Blank");
    // And HD 0 (now the topmost remaining "None", above HD 1).
    select_combo_at(&mut harness, "None", 0, "Blank");

    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert!(def.peripherals.fd502);
    assert_eq!(def.media.disk0.as_deref(), Some("disk0.dsk"));
    assert_eq!(def.media.disk1.as_deref(), Some("disk1.dsk"));
    assert_eq!(def.media.tape.as_deref(), Some("tape.cas"));
    assert_eq!(def.media.vhd0.as_deref(), Some("hd0.vhd"));
    assert_eq!(def.media.vhd1, None, "HD 1 stayed None");
    for file in ["disk0.dsk", "disk1.dsk", "tape.cas", "hd0.vhd"] {
        let blank = artifacts.path().join("coco-3").join(file);
        assert!(blank.is_file(), "blank image must exist at {}", blank.display());
        assert_eq!(
            fs::metadata(&blank).unwrap().len(),
            0,
            "fresh blank media is a 0-byte file"
        );
    }
}

/// "New…" creates a definition file *immediately* — default name under a
/// uniquified slug, saved, selected, no dialog and no Create button (macOS
/// System-Settings-style, user decision 2026-07-24). A second "New…"
/// uniquifies against the first.
#[test]
fn manager_new_creates_a_definition_file_immediately() {
    let dir = TempDir::new("create");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());
    assert!(harness.state().entries.is_empty());

    click_containing(&mut harness, "New…");

    assert_eq!(harness.state().entries.len(), 1, "New… must add a list row on the spot");
    assert!(harness.query_by_label("Create").is_none(), "no dialog is involved");
    let slug = harness.state().entries[0].slug.clone();
    assert_eq!(slug, "coco-3", "slugified from the default name");
    assert_eq!(harness.state().entries[0].def.name, "CoCo 3");

    let file = dir.path().join(format!("{slug}.toml"));
    let contents = fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    let parsed: machine_def::MachineDef =
        toml::from_str(&contents).expect("New… must write a parseable definition");
    assert_eq!(parsed.name, "CoCo 3");
    assert_eq!(harness.state().selected, Some(0), "New… must select the new row");
    // Not `get_by_label("CoCo 3")`: the now-visible detail pane's hardware
    // form has its own "CoCo 3" Machine combo button, so the name would be
    // ambiguous between that and the list row.
    assert_eq!(harness.state().detail_name(), Some("CoCo 3"));

    click_containing(&mut harness, "New…");
    assert_eq!(harness.state().entries.len(), 2);
    assert_eq!(harness.state().entries[1].slug, "coco-3-2", "second default uniquifies");
    assert!(dir.path().join("coco-3-2.toml").is_file());
}

/// Editing in the detail pane saves immediately — there are no Save/Revert
/// buttons anymore (auto-save, user decision 2026-07-24) — while merely
/// selecting a row must not rewrite its file.
#[test]
fn manager_detail_edits_save_immediately() {
    let dir = TempDir::new("auto-save");
    let entry = sample_entry("dev-coco-3", "Dev CoCo 3");
    machine_def::save(dir.path(), "dev-coco-3", &entry.def)
        .expect("seed the file the entry claims to be");
    assert!(entry.def.ui.aspect_correct, "test assumes the sample starts aspect-corrected");

    let mut harness = manager_harness(Some(dir.path().to_path_buf()), vec![entry]);
    let file = dir.path().join("dev-coco-3.toml");
    let before = fs::read_to_string(&file).unwrap();

    click(&mut harness, "Dev CoCo 3");
    harness.step();
    assert!(harness.query_by_label("Save").is_none(), "auto-save: no Save button");
    assert!(harness.query_by_label("Revert").is_none(), "auto-save: no Revert button");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        before,
        "selecting a row must not rewrite its definition"
    );

    click(&mut harness, "4:3 aspect correction");
    let saved: machine_def::MachineDef =
        toml::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert!(
        !saved.ui.aspect_correct,
        "the toggle must reach the file without any Save click"
    );
    assert!(!harness.state().entries[0].def.ui.aspect_correct);
}

/// Committing a new name (focus leaves the Name field) saves it and
/// migrates the slug: `<slug>.toml` and the artifact directory follow the
/// display name. Nothing else persists the slug — relative `[media]`
/// entries name files *inside* the artifact dir — so a rename is exactly
/// those two filesystem moves.
#[test]
fn manager_rename_migrates_definition_file_and_artifact_dir() {
    let machines = TempDir::new("rename-machines");
    let artifacts = TempDir::new("rename-artifacts");
    let entry = sample_entry("alpha", "Alpha");
    machine_def::save(machines.path(), "alpha", &entry.def).expect("seed the definition");
    fs::create_dir_all(artifacts.path().join("alpha")).unwrap();
    fs::write(artifacts.path().join("alpha").join("disk0.dsk"), b"").unwrap();

    let mut harness = manager_harness_with_artifacts(
        Some(machines.path().to_path_buf()),
        Some(artifacts.path().to_path_buf()),
        vec![entry],
    );
    click(&mut harness, "Alpha");

    // Type into the Name field — the pane's only text input; by-value
    // lookup would be ambiguous with the list row's own "Alpha" label —
    // and commit with Enter: the TextEdit surrenders focus, which is the
    // commit signal.
    let name_field = || harness.get_by_role(egui::accesskit::Role::TextInput);
    name_field().focus();
    harness.step();
    harness.get_by_role(egui::accesskit::Role::TextInput).type_text(" Two");
    harness.step();
    assert_eq!(harness.state().detail_name(), Some("Alpha Two"));
    harness.key_press(egui::Key::Enter);
    harness.step();
    harness.step(); // commit frame, then the deferred migration frame
    harness.step();

    assert_eq!(harness.state().entries[0].slug, "alpha-two");
    assert_eq!(harness.state().entries[0].def.name, "Alpha Two");
    assert!(machines.path().join("alpha-two.toml").is_file());
    assert!(!machines.path().join("alpha.toml").exists());
    assert!(
        artifacts.path().join("alpha-two").join("disk0.dsk").is_file(),
        "the artifact dir must follow the slug"
    );
    assert!(!artifacts.path().join("alpha").exists());
    assert_eq!(harness.state().selected, Some(0), "selection follows the renamed row");
    assert_eq!(harness.state().detail_name(), Some("Alpha Two"));
}

/// A minimal valid CoCo 2 `Ok` entry — `sample_entry`'s default is CoCo 3,
/// so pairing this with it gives two distinct machine families for
/// `starting_two_machines_runs_both` (`docs/plan-machine-persistence.md`
/// step 5's acceptance scenario: "a CoCo 3 and a newly created, launched
/// CoCo 2").
fn sample_coco2_entry(slug: &str, name: &str) -> manager::MachineEntry {
    manager::MachineEntry::new(
        slug.to_string(),
        machine_def::MachineDef::from_config(
            name.to_string(),
            None,
            &MachineConfig {
                variant: MachineVariant::Coco2,
                video: VideoStandard::NTSC,
                memory: MemorySize::K64,
                monitor: None,
                vdg: Some(VDGVariant::MC6847T1),
            },
        ),
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
