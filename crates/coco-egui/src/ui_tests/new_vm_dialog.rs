//! Direct-boot "Machine → New…" dialog tests: model/cartridge/media rows,
//! layout stability, and the ⌘N/Ctrl+N shortcut.

use egui_kittest::kittest::Queryable;

use coco_core::{MachineVariant, MemorySize};

use crate::rom_load::load_default_rom;

use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;

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
