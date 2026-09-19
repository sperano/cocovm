//! Manager detail-pane peripheral/media auto-save tests: the Cartridge-row
//! and MPI-slot picks packed into `[peripherals].cartridge`/`.slots`, and
//! blank-media placement in the artifact directory.

use std::fs;

use crate::machine_def::tests::TempDir;
use crate::*;

use super::harness::*;

/// "New" creates the machine immediately; picking Cartridge = FD-502
/// auto-saves `[peripherals].cartridge` into the definition file.
#[test]
fn manager_edit_with_fd502_records_the_peripheral() {
    let dir = TempDir::new("create-fd502");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    click(&mut harness, "Devices");
    // Combos showing "None": Cassette, then Cartridge, then the VHDs — Cartridge is second.
    select_combo_at(&mut harness, "None", 1, "FD-502");

    assert_eq!(harness.state().entries.len(), 1);
    assert_eq!(
        harness.state().entries[0].def.peripherals.cartridge,
        machine_def::CartridgeDTO::FD502 {
            dos_rom: Default::default()
        }
    );
    let file = dir.path().join("coco-3.toml");
    let contents = fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    assert!(
        contents.contains("kind = \"fd502\""),
        "the TOML must record the peripheral:\n{contents}"
    );
}

/// Cartridge = MultiPak Interface in the pane records the MPI cartridge with
/// all four slots explicit and empty: `slots` is a required field of
/// `CartridgeDTO::MPI`, so an intentionally-empty loadout can't be omitted
/// from the file the way schema-1's booleans could.
#[test]
fn manager_edit_with_mpi_records_the_peripheral() {
    let dir = TempDir::new("create-mpi");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    click(&mut harness, "Devices");
    select_combo_at(&mut harness, "None", 1, "MultiPak Interface");

    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert_eq!(
        def.peripherals.cartridge,
        machine_def::CartridgeDTO::MPI {
            slots: std::array::from_fn(|_| machine_def::SlotDTO::Empty),
            switch: crate::DEFAULT_MPI_SWITCH_SLOT + 1,
        }
    );
    let contents = fs::read_to_string(dir.path().join("coco-3.toml")).unwrap();
    assert!(
        contents.contains("kind = \"mpi\""),
        "TOML must record the MPI:\n{contents}"
    );
    assert_eq!(
        contents.matches("kind = \"empty\"").count(),
        4,
        "an MPI with nothing in any slot must record four explicit empty slots:\n{contents}"
    );
}

/// A Disto RTC — in the port or in an MPI slot — records `[peripherals].cartridge`/`.slots`
/// accordingly.
#[test]
fn manager_edit_with_rtc_records_the_peripheral() {
    let dir = TempDir::new("create-rtc");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    click(&mut harness, "Devices");
    select_combo_at(&mut harness, "None", 1, "Disto RTC (4-N-1)");
    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert_eq!(def.peripherals.cartridge, machine_def::CartridgeDTO::RTC);
    let contents = fs::read_to_string(dir.path().join("coco-3.toml")).unwrap();
    assert!(
        contents.contains("kind = \"rtc\""),
        "TOML must record the RTC:\n{contents}"
    );

    // Slotted, on a second machine: MPI cartridge with slot 1 holding the RTC.
    click_containing(&mut harness, "New");
    click(&mut harness, "Devices");
    select_combo_at(&mut harness, "None", 1, "MultiPak Interface");
    select_combo_at(&mut harness, "Empty", 0, "Disto RTC (4-N-1)");
    assert_eq!(harness.state().entries.len(), 2);
    let def = &harness.state().entries[1].def;
    assert_eq!(
        def.peripherals.cartridge,
        machine_def::CartridgeDTO::MPI {
            slots: [
                machine_def::SlotDTO::RTC,
                machine_def::SlotDTO::Empty,
                machine_def::SlotDTO::Empty,
                machine_def::SlotDTO::Empty,
            ],
            switch: crate::DEFAULT_MPI_SWITCH_SLOT + 1,
        }
    );
}

/// A ROM Pak — in the port or in an MPI slot — records `[peripherals].cartridge`/`.slots` with
/// its path. Seeded directly on the edit form: the combo's own item opens a native file dialog.
#[test]
fn manager_edit_with_rom_pak_records_the_cart() {
    let pak = PathBuf::from("/paks/game.ccc");
    let dir = TempDir::new("create-rompak");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    let form = harness
        .state_mut()
        .edit_form_mut()
        .expect("pane form seeded");
    form.cartridge = new_vm::CartridgeChoice::Image(cartridge_image_choice(
        pak.clone(),
        coco_core::rom_db::CartridgeHardware::RomPak,
    ));
    harness.step();
    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert_eq!(
        def.peripherals.cartridge,
        machine_def::CartridgeDTO::ROMPak {
            path: "/paks/game.ccc".to_string(),
            autostart: true,
        }
    );

    // Slotted, on a second machine — and unlike the retired boolean schema, two slotted paks
    // are both representable: each slot records its own path independently.
    click_containing(&mut harness, "New");
    {
        let form = harness
            .state_mut()
            .edit_form_mut()
            .expect("pane form seeded");
        form.cartridge = new_vm::CartridgeChoice::MPI;
        form.mpi_slots[1] = new_vm::SlotChoice::Image(cartridge_image_choice(
            pak.clone(),
            coco_core::rom_db::CartridgeHardware::RomPak,
        ));
        form.mpi_slots[3] = new_vm::SlotChoice::Image(cartridge_image_choice(
            PathBuf::from("/paks/other.ccc"),
            coco_core::rom_db::CartridgeHardware::RomPak,
        ));
    }
    harness.step();
    assert_eq!(harness.state().entries.len(), 2);
    let def = &harness.state().entries[1].def;
    assert_eq!(
        def.peripherals.cartridge,
        machine_def::CartridgeDTO::MPI {
            slots: [
                machine_def::SlotDTO::Empty,
                machine_def::SlotDTO::ROMPak {
                    path: "/paks/game.ccc".to_string(),
                    autostart: true,
                },
                machine_def::SlotDTO::Empty,
                machine_def::SlotDTO::ROMPak {
                    path: "/paks/other.ccc".to_string(),
                    autostart: true,
                },
            ],
            switch: crate::DEFAULT_MPI_SWITCH_SLOT + 1,
        }
    );
}

/// Cartridge = "RS-232 Pak" in the pane auto-saves `[peripherals].cartridge` —
/// same combo, same auto-save flow as the earlier FD-502/MPI/RTC cases.
#[test]
fn manager_edit_with_rs232_records_the_peripheral() {
    let dir = TempDir::new("create-rs232");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    click(&mut harness, "Devices");
    select_combo_at(&mut harness, "None", 1, "RS-232 Pak");

    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert_eq!(
        def.peripherals.cartridge,
        machine_def::CartridgeDTO::RS232 {
            endpoint: machine_def::RS232EndpointDTO::Loopback,
        }
    );
    let contents = fs::read_to_string(dir.path().join("coco-3.toml")).unwrap();
    assert!(
        contents.contains("kind = \"rs232\""),
        "the TOML must record the peripheral:\n{contents}"
    );
}

/// The RS-232 Pak in an MPI slot (not the bare port) records
/// `[peripherals].cartridge.slots` accordingly — the slot combo's own "RS-232 Pak" entry.
#[test]
fn manager_edit_with_slotted_rs232_records_the_peripheral() {
    let dir = TempDir::new("create-mpi-rs232-slot");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    click(&mut harness, "Devices");
    select_combo_at(&mut harness, "None", 1, "MultiPak Interface");
    // Slot 2 (index 1): the second "Empty" combo, 0-based over the y-sorted nodes; the
    // Switch combo has no "Empty" text so it never shifts the count.
    select_combo_at(&mut harness, "Empty", 1, "RS-232 Pak");

    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert_eq!(
        def.peripherals.cartridge,
        machine_def::CartridgeDTO::MPI {
            slots: [
                machine_def::SlotDTO::Empty,
                machine_def::SlotDTO::RS232 {
                    endpoint: machine_def::RS232EndpointDTO::Loopback,
                },
                machine_def::SlotDTO::Empty,
                machine_def::SlotDTO::Empty,
            ],
            switch: crate::DEFAULT_MPI_SWITCH_SLOT + 1,
        }
    );
    let contents = fs::read_to_string(dir.path().join("coco-3.toml")).unwrap();
    assert!(
        contents.contains("kind = \"rs232\""),
        "the TOML must record the slotted peripheral:\n{contents}"
    );
}

/// The Games Master cartridge — image-backed like the ROM Pak — records
/// `[peripherals].cartridge` with its path. Orchestra-90 in an MPI slot — fixed ROM, no
/// path — records like the Sound/Speech Cartridge.
#[test]
fn manager_edit_with_gmc_and_orch90_records_the_cart() {
    let gmc = PathBuf::from("/paks/gmc.ccc");
    let dir = TempDir::new("create-gmc-orch90");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    let form = harness
        .state_mut()
        .edit_form_mut()
        .expect("pane form seeded");
    form.cartridge = new_vm::CartridgeChoice::Image(cartridge_image_choice(
        gmc.clone(),
        coco_core::rom_db::CartridgeHardware::GamesMaster,
    ));
    harness.step();
    assert_eq!(
        harness.state().entries[0].def.peripherals.cartridge,
        machine_def::CartridgeDTO::GamesMaster {
            path: "/paks/gmc.ccc".to_string(),
            autostart: true,
        }
    );

    click_containing(&mut harness, "New");
    {
        let form = harness
            .state_mut()
            .edit_form_mut()
            .expect("pane form seeded");
        form.cartridge = new_vm::CartridgeChoice::MPI;
        form.mpi_slots[2] = new_vm::SlotChoice::Orch90;
    }
    harness.step();
    let def = &harness.state().entries[1].def;
    assert_eq!(
        def.peripherals.cartridge,
        machine_def::CartridgeDTO::MPI {
            slots: [
                machine_def::SlotDTO::Empty,
                machine_def::SlotDTO::Empty,
                machine_def::SlotDTO::Orch90,
                machine_def::SlotDTO::Empty,
            ],
            switch: crate::DEFAULT_MPI_SWITCH_SLOT + 1,
        }
    );
}

/// The Sound/Speech Cartridge — no path — records
/// `[peripherals].cartridge`/`.slots` through the same combo flow as the
/// FD-502/RTC (no file dialog to dodge).
#[test]
fn manager_edit_with_ssc_records_the_peripheral() {
    let dir = TempDir::new("create-ssc");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    click(&mut harness, "Devices");
    select_combo_at(&mut harness, "None", 1, "Sound/Speech Cartridge");
    assert_eq!(
        harness.state().entries[0].def.peripherals.cartridge,
        machine_def::CartridgeDTO::SoundSpeech
    );

    // Slotted, on a second machine — any number of slots may hold one, so two are recorded
    // independently.
    click_containing(&mut harness, "New");
    click(&mut harness, "Devices");
    select_combo_at(&mut harness, "None", 1, "MultiPak Interface");
    select_combo_at(&mut harness, "Empty", 0, "Sound/Speech Cartridge");
    select_combo_at(&mut harness, "Empty", 0, "Sound/Speech Cartridge");
    let def = &harness.state().entries[1].def;
    assert_eq!(
        def.peripherals.cartridge,
        machine_def::CartridgeDTO::MPI {
            slots: [
                machine_def::SlotDTO::SoundSpeech,
                machine_def::SlotDTO::SoundSpeech,
                machine_def::SlotDTO::Empty,
                machine_def::SlotDTO::Empty,
            ],
            switch: crate::DEFAULT_MPI_SWITCH_SLOT + 1,
        }
    );
}

/// Picking Serial = "Printer (DMP-105)" auto-saves `[ports].serial =
/// "printer"`; DMP-130 and file capture persist their distinct values.
#[test]
fn manager_edit_with_serial_records_the_port() {
    let dir = TempDir::new("create-serial");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    click(&mut harness, "Devices");
    // "None" combo order: Cassette, Cartridge, VHD 0, VHD 1, then Serial (Ports renders below
    // Peripherals).
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

    select_combo_at(&mut harness, "Printer (DMP-105)", 0, "Printer (DMP-130)");
    assert_eq!(
        harness.state().entries[0].def.ports.serial,
        Some(machine_def::SerialDTO::Dmp130)
    );
    let contents = fs::read_to_string(&file).unwrap();
    assert!(contents.contains("serial = \"dmp130\""));

    select_combo_at(&mut harness, "Printer (DMP-130)", 0, "Print to file");
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
/// `joy_right`; both default "None" in the Input tab.
#[test]
fn manager_edit_with_joy_sources_records_them() {
    let dir = TempDir::new("create-joy-sources");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    click(&mut harness, "Input");
    select_combo_at(&mut harness, "None", 0, "Keys");
    // Left now reads "Keys", so Right is the remaining "None" combo.
    select_combo_at(&mut harness, "None", 0, "Mouse");

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
    click(&mut harness, "Devices");
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
    assert_eq!(
        def.peripherals.cartridge,
        machine_def::CartridgeDTO::FD502 {
            dos_rom: Default::default()
        }
    );
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

/// The MPI's Switch combo (shown while Cartridge = MultiPak Interface) records
/// `[peripherals].cartridge.switch` as the picked 1-based slot.
#[test]
fn manager_edit_with_mpi_switch_records_the_switch() {
    let dir = TempDir::new("create-mpi-switch");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    click(&mut harness, "Devices");
    select_combo_at(&mut harness, "None", 1, "MultiPak Interface");
    // The switch combo's own default text is "Slot 4" (`DEFAULT_MPI_SWITCH_SLOT`); picking
    // "Slot 2" is the 1-based front-panel switch the DTO records as `switch = 2`.
    select_combo_at(&mut harness, "Slot 4", 0, "Slot 2");

    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert_eq!(
        def.peripherals.cartridge,
        machine_def::CartridgeDTO::MPI {
            slots: std::array::from_fn(|_| machine_def::SlotDTO::Empty),
            switch: 2,
        }
    );
    let contents = fs::read_to_string(dir.path().join("coco-3.toml")).unwrap();
    assert!(
        contents.contains("switch = 2"),
        "TOML must record the switch pick:\n{contents}"
    );
}

/// Cartridge = "RS-232 Pak" with its Endpoint combo switched to TCP records
/// `[peripherals].cartridge.endpoint` with the default listen address the combo pick seeds.
#[test]
fn manager_edit_with_rs232_tcp_endpoint_records_the_endpoint() {
    let dir = TempDir::new("create-rs232-tcp");
    let mut harness = manager_harness(Some(dir.path().to_path_buf()), Vec::new());

    click_containing(&mut harness, "New");
    click(&mut harness, "Devices");
    select_combo_at(&mut harness, "None", 1, "RS-232 Pak");
    select_combo_at(&mut harness, "Loopback", 0, "TCP");

    assert_eq!(harness.state().entries.len(), 1);
    let def = &harness.state().entries[0].def;
    assert_eq!(
        def.peripherals.cartridge,
        machine_def::CartridgeDTO::RS232 {
            endpoint: machine_def::RS232EndpointDTO::TCP {
                listen: crate::RS232_TCP_DEFAULT_ADDR.to_string(),
            }
        }
    );
    let contents = fs::read_to_string(dir.path().join("coco-3.toml")).unwrap();
    assert!(
        contents.contains("kind = \"tcp\"")
            && contents.contains(&format!("listen = \"{}\"", crate::RS232_TCP_DEFAULT_ADDR)),
        "TOML must record the TCP endpoint and its listen address:\n{contents}"
    );
}
