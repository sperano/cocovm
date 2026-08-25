//! Manager detail-pane peripheral/media auto-save tests: FD-502/MPI/RTC
//! peripheral flags, ROM Pak `[media].cart`, and blank-media placement in
//! the artifact directory.

use std::fs;

use egui_kittest::kittest::Queryable;

use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;

/// "New" creates the machine immediately; picking Cartridge = FD-502
/// auto-saves `[peripherals].fd502` into the definition file.
#[test]
fn manager_edit_with_fd502_records_the_peripheral() {
    let dir = TempDir::new("create-fd502");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    // Combos showing "None": Cassette, then Cartridge, then the VHDs — Cartridge is second.
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

    click_containing(&mut harness, "New");
    select_combo_at(&mut harness, "None", 1, "MultiPak Interface");

    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert!(def.peripherals.mpi);
    assert!(!def.peripherals.fd502);
    let contents = fs::read_to_string(dir.path().join("coco-3.toml")).unwrap();
    assert!(
        contents.contains("mpi = true"),
        "TOML must record the MPI:\n{contents}"
    );
}

/// A Disto RTC — in the port or slotted — records `peripherals.rtc` (the
/// schema keeps no slot layout; launch re-seats a slotted clock in its default slot).
#[test]
fn manager_edit_with_rtc_records_the_peripheral() {
    let dir = TempDir::new("create-rtc");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    select_combo_at(&mut harness, "None", 1, "Disto RTC");
    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert!(def.peripherals.rtc && !def.peripherals.mpi);
    let contents = fs::read_to_string(dir.path().join("coco-3.toml")).unwrap();
    assert!(
        contents.contains("rtc = true"),
        "TOML must record the RTC:\n{contents}"
    );

    // Slotted, on a second machine: rtc = true alongside mpi = true.
    click_containing(&mut harness, "New");
    select_combo_at(&mut harness, "None", 1, "MultiPak Interface");
    select_combo_at(&mut harness, "Empty", 0, "Disto RTC");
    assert_eq!(harness.state().entries.len(), 2);
    let def = &harness.state().entries[1].def;
    assert!(def.peripherals.rtc && def.peripherals.mpi);
}

/// A ROM Pak — in the port or in an MPI slot — records `[media].cart`.
/// Seeded directly on the edit form: the combo's own item opens a native file dialog.
#[test]
fn manager_edit_with_rom_pak_records_the_cart() {
    let pak = PathBuf::from("/paks/game.ccc");
    let dir = TempDir::new("create-rompak");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    harness
        .state_mut()
        .edit_form_mut()
        .expect("pane form seeded")
        .cartridge = new_vm::CartridgeChoice::ROMPak(pak.clone());
    harness.step();
    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert_eq!(def.media.cart.as_deref(), Some("/paks/game.ccc"));
    assert!(!def.peripherals.mpi && !def.peripherals.fd502);

    // Slotted, on a second machine: recorded the same way, alongside
    // mpi = true.
    click_containing(&mut harness, "New");
    {
        let form = harness
            .state_mut()
            .edit_form_mut()
            .expect("pane form seeded");
        form.cartridge = new_vm::CartridgeChoice::MPI;
        form.mpi_slots[1] = new_vm::SlotChoice::ROMPak(pak.clone());
    }
    harness.step();
    assert_eq!(harness.state().entries.len(), 2);
    let def = &harness.state().entries[1].def;
    assert_eq!(def.media.cart.as_deref(), Some("/paks/game.ccc"));
    assert!(def.peripherals.mpi);

    // Two slotted paks can't be represented in the schema: refused with an inline error.
    harness
        .state_mut()
        .edit_form_mut()
        .expect("pane form seeded")
        .mpi_slots[3] = new_vm::SlotChoice::ROMPak(pak);
    harness.step();
    harness.get_by_label_contains("a single ROM Pak");
    assert_eq!(
        harness.state().entries[1].def.media.cart.as_deref(),
        Some("/paks/game.ccc"),
        "the definition must keep the single recorded pak"
    );
}

/// Cartridge = "RS-232 Pak" in the pane auto-saves `[peripherals].rs232` —
/// same combo, same auto-save flow as the FD-502/MPI/RTC cases above.
#[test]
fn manager_edit_with_rs232_records_the_peripheral() {
    let dir = TempDir::new("create-rs232");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    select_combo_at(&mut harness, "None", 1, "RS-232 Pak");

    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert!(def.peripherals.rs232 && !def.peripherals.mpi);
    let contents = fs::read_to_string(dir.path().join("coco-3.toml")).unwrap();
    assert!(
        contents.contains("rs232 = true"),
        "the TOML must record the peripheral:\n{contents}"
    );
}

/// Picking Serial = "Printer (DMP-105)" auto-saves `[ports].serial =
/// "printer"`; switching to "Print to file" updates it to `"file"`.
#[test]
fn manager_edit_with_serial_records_the_port() {
    let dir = TempDir::new("create-serial");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    // "None" combo order: Cassette, Cartridge, VHD 0, VHD 1, then Serial (Ports renders below Peripherals).
    select_combo_at(&mut harness, "None", 4, "Printer (DMP-105)");

    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert_eq!(def.ports.serial, Some(machine_def::SerialDTO::Printer));
    let file = dir.path().join("coco-3.toml");
    let contents = fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    assert!(
        contents.contains("serial = \"printer\""),
        "the TOML must record the serial sink:\n{contents}"
    );

    select_combo_at(&mut harness, "Printer (DMP-105)", 0, "Print to file");
    assert_eq!(
        harness.state().entries[0].def.ports.serial,
        Some(machine_def::SerialDTO::File)
    );
    let contents = fs::read_to_string(&file).unwrap();
    assert!(
        contents.contains("serial = \"file\""),
        "the TOML must record the switched sink:\n{contents}"
    );
}

/// Picking Left = "Keys" and Right = "Mouse" auto-saves `[ui].joy_left`/
/// `joy_right`; both default "None", landing at combo indexes 5 and 6.
#[test]
fn manager_edit_with_joy_sources_records_them() {
    let dir = TempDir::new("create-joy-sources");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    select_combo_at(&mut harness, "None", 5, "Keys");
    // Left now reads "Keys", so Right is the new index-5 "None".
    select_combo_at(&mut harness, "None", 5, "Mouse");

    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert_eq!(def.ui.joy_left, machine_def::JoySourceDTO::Keys);
    assert_eq!(def.ui.joy_right, machine_def::JoySourceDTO::Mouse);
    let file = dir.path().join("coco-3.toml");
    let contents = fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    assert!(
        contents.contains("joy_left = \"keys\"") && contents.contains("joy_right = \"mouse\""),
        "the TOML must record both joystick sources:\n{contents}"
    );
}

/// Disk 0 = Blank: a 0-byte blank image lands in the machine's artifact dir
/// the moment it's picked, and `[media].disk0` records it by relative path.
#[test]
fn manager_edit_with_blank_disk0_places_it_in_the_artifact_dir() {
    let machines = TempDir::new("create-blank-machines");
    let artifacts = TempDir::new("create-blank-artifacts");
    let mut harness = manager_harness_with_artifacts(
        Some(machines.path().to_path_buf()),
        Some(artifacts.path().to_path_buf()),
        Vec::new(),
    );

    click_containing(&mut harness, "New");
    select_combo_at(&mut harness, "None", 1, "FD-502");
    // "None" order: Cassette, Disk 0, Disk 1, VHD 0, VHD 1; each pick leaves the pool immediately.
    select_combo_at(&mut harness, "None", 1, "Blank");
    // Disk 0 now reads "disk0.dsk"; remaining "None"s: Cassette, Disk 1, VHDs.
    select_combo_at(&mut harness, "None", 1, "Blank");
    // The cassette (topmost remaining "None").
    select_combo_at(&mut harness, "None", 0, "Blank");
    // And VHD 0 (now the topmost remaining "None", above VHD 1).
    select_combo_at(&mut harness, "None", 0, "Blank");

    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert!(def.peripherals.fd502);
    assert_eq!(def.media.disk0.as_deref(), Some("disk0.dsk"));
    assert_eq!(def.media.disk1.as_deref(), Some("disk1.dsk"));
    assert_eq!(def.media.tape.as_deref(), Some("tape.cas"));
    assert_eq!(def.media.vhd0.as_deref(), Some("hd0.vhd"));
    assert_eq!(def.media.vhd1, None, "VHD 1 stayed None");
    for file in ["disk0.dsk", "disk1.dsk", "tape.cas", "hd0.vhd"] {
        let blank = artifacts.path().join("coco-3").join(file);
        assert!(
            blank.is_file(),
            "blank image must exist at {}",
            blank.display()
        );
        assert_eq!(
            fs::metadata(&blank).unwrap().len(),
            0,
            "fresh blank media is a 0-byte file"
        );
    }
}
