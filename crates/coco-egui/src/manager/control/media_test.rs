//! The media actions on a real running VM, through the same `CocoApp`
//! methods the VM window and the launch path use.

use std::path::{Path, PathBuf};

use coco_core::cart::Cart;
use coco_core::cassette::test_support::{SPINUP_BURN_CYCLES, record_bytes_fsk, tape_block};

use super::*;
use crate::machine_def::tests::TempDir;
use crate::machine_def::{CartridgeDTO, MachineDef, RS232EndpointDTO};
use crate::manager::MachineEntry;
use crate::save_state::tests::{ReadOnly, boot_app, ensure_writable};

const SLUG: &str = "live";
/// A well-formed ROM Pak image ([`coco_core::cart::ROMPak::from_bytes`]
/// needs a valid size, not real code).
const ROM_PAK_SIZE: usize = 0x4000;
/// Tape bytes before the block a test records, so the head has somewhere
/// to move from.
const TAPE_FIXTURE: &[u8] = &[0x55; 64];
/// Leader byte that lets the cassette demodulator lock on before a block.
const LEADER: u8 = 0x55;
const LEADER_LEN: usize = 16;

fn vm() -> Option<String> {
    Some(SLUG.to_string())
}

fn manager() -> ManagerApp {
    let def = MachineDef::from_config(SLUG.to_string(), None, &crate::MachineConfig::default());
    let mut entry = MachineEntry::new(SLUG.to_string(), def);
    entry.vm = Some(Box::new(boot_app()));
    ManagerApp::new(None, None, None, vec![entry], None)
}

fn app(manager: &mut ManagerApp) -> &mut CocoApp {
    manager.entries[0].vm.as_mut().expect("running VM")
}

fn write_rom_pak(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, vec![0x11_u8; ROM_PAK_SIZE]).unwrap();
    path
}

fn write_file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// The cartridge `list_vms` reports for the VM.
fn listed_cartridge(manager: &ManagerApp) -> CartridgeDTO {
    manager.vm_infos().remove(0).cartridge
}

/// Record one framed block onto the mounted tape, leaving it dirty.
fn record_a_block(app: &mut CocoApp) {
    let mut block = vec![LEADER; LEADER_LEN];
    block.extend(tape_block(0x01, b"X"));
    let cassette = &mut app.machine.bus.cassette;
    cassette.tick(SPINUP_BURN_CYCLES, true);
    record_bytes_fsk(cassette, &block);
    cassette.finalize_recording();
    assert!(cassette.dirty(), "the recording must dirty the tape");
}

#[test]
fn tape_insert_rewind_and_eject() {
    let dir = TempDir::new("mcp-media-tape");
    let tape = write_file(dir.path(), "game.cas", TAPE_FIXTURE);
    let mut manager = manager();

    manager.insert_tape_action(&vm(), tape.clone()).unwrap();
    assert_eq!(app(&mut manager).tape_path.as_ref(), Some(&tape));
    app(&mut manager)
        .machine
        .bus
        .cassette
        .seek(TAPE_FIXTURE.len());

    manager.rewind_tape_action(&vm()).unwrap();
    assert_eq!(app(&mut manager).machine.bus.cassette.position().0, 0);

    manager.eject_tape_action(&vm()).unwrap();
    assert_eq!(app(&mut manager).tape_path, None);
    assert!(!app(&mut manager).machine.bus.cassette.has_tape());
}

#[test]
fn rewind_without_a_tape_fails() {
    let mut manager = manager();
    assert_eq!(manager.rewind_tape_action(&vm()), Err(NO_TAPE.to_string()));
}

#[test]
fn an_unreadable_tape_is_reported() {
    let mut manager = manager();
    let error = manager
        .insert_tape_action(&vm(), PathBuf::from("/no/such/tape.cas"))
        .unwrap_err();
    assert!(error.contains("/no/such/tape.cas"), "{error}");
}

#[test]
fn a_failed_cas_write_back_changes_nothing() {
    let dir = TempDir::new("mcp-media-tape-cas-failure");
    let old = write_file(dir.path(), "old.cas", TAPE_FIXTURE);
    ensure_writable(&old);
    let new = write_file(dir.path(), "new.cas", TAPE_FIXTURE);
    let mut manager = manager();
    manager.insert_tape_action(&vm(), old.clone()).unwrap();
    record_a_block(app(&mut manager));

    let _read_only = ReadOnly::new(&old);
    let error = manager.insert_tape_action(&vm(), new).unwrap_err();
    assert!(error.contains("old.cas"), "{error}");
    assert_eq!(app(&mut manager).tape_path.as_ref(), Some(&old));
    assert!(app(&mut manager).machine.bus.cassette.dirty());
    assert!(manager.eject_tape_action(&vm()).is_err());
    assert_eq!(app(&mut manager).tape_path.as_ref(), Some(&old));
}

#[test]
fn a_failed_wav_copy_is_an_error_but_the_tape_still_changes() {
    let dir = TempDir::new("mcp-media-tape-wav-failure");
    let old = write_file(dir.path(), "old.cas", TAPE_FIXTURE);
    let new = write_file(dir.path(), "new.cas", TAPE_FIXTURE);
    // A directory where the `.wav` copy should go makes only that write fail.
    std::fs::create_dir_all(dir.path().join("old.wav")).unwrap();
    let mut manager = manager();
    manager.insert_tape_action(&vm(), old).unwrap();
    app(&mut manager).save_tape_wav = true;
    record_a_block(app(&mut manager));

    let error = manager.insert_tape_action(&vm(), new.clone()).unwrap_err();
    assert!(error.contains("old.wav"), "{error}");
    assert!(error.contains("The tape was inserted"), "{error}");
    assert_eq!(app(&mut manager).tape_path.as_ref(), Some(&new));
    assert_eq!(app(&mut manager).cart_error, None);
}

#[test]
fn a_rom_pak_goes_in_and_comes_out_of_the_port() {
    let dir = TempDir::new("mcp-media-rompak");
    let rom = write_rom_pak(dir.path(), "game.ccc");
    let mut manager = manager();
    let cartridge = CartridgeDTO::ROMPak {
        path: rom.display().to_string(),
    };

    manager
        .insert_cartridge_action(&vm(), CartridgeTarget::Port(cartridge.clone()))
        .unwrap();
    assert_eq!(app(&mut manager).cart_path.as_ref(), Some(&rom));
    assert_eq!(listed_cartridge(&manager), cartridge);

    manager.remove_cartridge_action(&vm(), None).unwrap();
    assert!(matches!(app(&mut manager).machine.bus.cart, Cart::Empty(_)));
    assert_eq!(app(&mut manager).cart_path, None);
    assert_eq!(listed_cartridge(&manager), CartridgeDTO::None);
}

#[test]
fn a_failed_insert_leaves_the_port_empty_and_says_so() {
    let mut manager = manager();
    manager
        .insert_cartridge_action(&vm(), CartridgeTarget::Port(fd502()))
        .unwrap();

    let missing = CartridgeDTO::ROMPak {
        path: "/no/such/game.ccc".to_string(),
    };
    let error = manager
        .insert_cartridge_action(&vm(), CartridgeTarget::Port(missing))
        .unwrap_err();
    assert!(error.contains("/no/such/game.ccc"), "{error}");
    assert!(error.ends_with(OLD_CARTRIDGE_REMOVED), "{error}");
    assert_eq!(listed_cartridge(&manager), CartridgeDTO::None);
}

fn fd502() -> CartridgeDTO {
    CartridgeDTO::FD502 {
        dos_rom: DosRom::DiskBasic,
    }
}

#[test]
fn a_multipak_is_installed_switched_and_edited_slot_by_slot() {
    let dir = TempDir::new("mcp-media-mpi");
    let rom = write_rom_pak(dir.path(), "game.ccc");
    let rom_slot = SlotDTO::ROMPak {
        path: rom.display().to_string(),
    };
    let fd502_slot = SlotDTO::FD502 {
        dos_rom: DosRom::DiskBasic,
    };
    let mpi = CartridgeDTO::MPI {
        slots: [
            rom_slot.clone(),
            SlotDTO::Empty,
            SlotDTO::Empty,
            fd502_slot.clone(),
        ],
        switch: 1,
    };
    let mut manager = manager();

    manager
        .insert_cartridge_action(&vm(), CartridgeTarget::Port(mpi.clone()))
        .unwrap();
    assert_eq!(listed_cartridge(&manager), mpi);
    assert!(app(&mut manager).machine.bus.cart.as_disk_cart().is_some());

    manager.set_mpi_switch_action(&vm(), 3).unwrap();
    let multipak = app(&mut manager).machine.bus.cart.as_multipak().unwrap();
    assert_eq!(multipak.switch_slot(), 3);

    manager
        .insert_cartridge_action(
            &vm(),
            CartridgeTarget::Slot {
                slot: 1,
                occupant: SlotDTO::RS232 {
                    endpoint: RS232EndpointDTO::Loopback,
                },
            },
        )
        .unwrap();
    manager.remove_cartridge_action(&vm(), Some(0)).unwrap();
    assert_eq!(
        listed_cartridge(&manager),
        CartridgeDTO::MPI {
            slots: [
                SlotDTO::Empty,
                SlotDTO::RS232 {
                    endpoint: RS232EndpointDTO::Loopback
                },
                SlotDTO::Empty,
                fd502_slot,
            ],
            switch: 4,
        }
    );

    manager.remove_cartridge_action(&vm(), Some(1)).unwrap();
    assert!(app(&mut manager).rs232.is_none());
}

#[test]
fn replacing_the_fd502_in_its_own_slot_keeps_one_controller() {
    let mut manager = manager();
    let mpi = CartridgeDTO::MPI {
        slots: std::array::from_fn(|_| SlotDTO::Empty),
        switch: 4,
    };
    manager
        .insert_cartridge_action(&vm(), CartridgeTarget::Port(mpi))
        .unwrap();
    let fd502_slot = || CartridgeTarget::Slot {
        slot: 3,
        occupant: SlotDTO::FD502 {
            dos_rom: DosRom::DiskBasic,
        },
    };
    manager
        .insert_cartridge_action(&vm(), fd502_slot())
        .unwrap();
    manager
        .insert_cartridge_action(&vm(), fd502_slot())
        .unwrap();
    assert!(app(&mut manager).machine.bus.cart.as_disk_cart().is_some());
}

#[test]
fn slot_actions_need_a_multipak() {
    let mut manager = manager();
    let slot_insert = CartridgeTarget::Slot {
        slot: 0,
        occupant: SlotDTO::Orch90,
    };
    for result in [
        manager.insert_cartridge_action(&vm(), slot_insert),
        manager.remove_cartridge_action(&vm(), Some(0)),
        manager.set_mpi_switch_action(&vm(), 0),
    ] {
        assert_eq!(result, Err(NO_MULTIPAK.to_string()));
    }
}

#[test]
fn a_games_master_is_refused_while_drivewire_is_on() {
    let dir = TempDir::new("mcp-media-gmc-drivewire");
    let rom = write_rom_pak(dir.path(), "gmc.ccc");
    let mut manager = manager();
    app(&mut manager).enable_drivewire(false);
    manager
        .insert_cartridge_action(&vm(), CartridgeTarget::Port(fd502()))
        .unwrap();

    let gmc = CartridgeDTO::GamesMaster {
        path: rom.display().to_string(),
    };
    let error = manager
        .insert_cartridge_action(&vm(), CartridgeTarget::Port(gmc))
        .unwrap_err();
    assert_eq!(error, machine_def::DRIVEWIRE_GMC_CONFLICT);
    assert_eq!(listed_cartridge(&manager), fd502());
}

#[test]
fn hdb_dos_needs_a_coco_3() {
    let mut app = boot_app();
    let needs = slot_needs(&SlotDTO::FD502 {
        dos_rom: DosRom::HdbDosDw3,
    });
    assert_eq!(check_fits(&app, &needs), Ok(()));
    app.machine.config.variant = MachineVariant::Coco2;
    assert_eq!(
        check_fits(&app, &needs),
        Err(machine_def::HDBDOS_NEEDS_COCO3.to_string())
    );
}

#[test]
fn a_vhd_is_inserted_and_ejected() {
    let dir = TempDir::new("mcp-media-vhd");
    let image = write_file(dir.path(), "os9.vhd", &[0; ROM_PAK_SIZE]);
    let mut manager = manager();

    manager.insert_vhd_action(&vm(), 1, image.clone()).unwrap();
    assert_eq!(app(&mut manager).vhd_paths[1].as_ref(), Some(&image));
    assert!(app(&mut manager).machine.bus.vhd.is_mounted(1));

    manager.eject_vhd_action(&vm(), 1).unwrap();
    assert_eq!(app(&mut manager).vhd_paths[1], None);
    assert!(!app(&mut manager).machine.bus.vhd.is_mounted(1));

    assert!(manager.eject_vhd_action(&vm(), crate::UI_DRIVES).is_err());
    let error = manager
        .insert_vhd_action(&vm(), 0, PathBuf::from("/no/such/os9.vhd"))
        .unwrap_err();
    assert!(error.contains("/no/such/os9.vhd"), "{error}");
}

#[test]
fn drivewire_disks_need_drivewire_on() {
    let mut manager = manager();
    let error = manager
        .set_drivewire_disk_action(&vm(), 0, None)
        .unwrap_err();
    assert!(error.contains("DriveWire is off"), "{error}");
}

#[test]
fn a_drivewire_disk_outlives_an_unrelated_settings_change() {
    let dir = TempDir::new("mcp-media-drivewire");
    let image = write_file(
        dir.path(),
        "nos9.dsk",
        &[0; coco_core::drivewire::SECTOR_SIZE],
    );
    let mut manager = manager();
    let settings = || {
        Some(crate::app::DriveWireLaunch {
            hdbdos_mode: false,
            disk_paths: Default::default(),
        })
    };
    app(&mut manager)
        .apply_drivewire_settings(settings())
        .unwrap();

    manager
        .set_drivewire_disk_action(&vm(), 2, Some(image.clone()))
        .unwrap();
    assert_eq!(app(&mut manager).dw_paths[2].as_ref(), Some(&image));
    assert_eq!(app(&mut manager).dw_startup[2], None);

    app(&mut manager)
        .apply_drivewire_settings(settings())
        .unwrap();
    assert_eq!(app(&mut manager).dw_paths[2].as_ref(), Some(&image));

    manager.set_drivewire_disk_action(&vm(), 2, None).unwrap();
    assert_eq!(app(&mut manager).dw_paths[2], None);
    assert!(
        manager
            .set_drivewire_disk_action(&vm(), coco_core::drivewire::DRIVE_COUNT, None)
            .is_err()
    );
}

#[test]
fn media_actions_need_a_running_vm() {
    let def = MachineDef::from_config(SLUG.to_string(), None, &crate::MachineConfig::default());
    let entry = MachineEntry::new(SLUG.to_string(), def);
    let mut manager = ManagerApp::new(None, None, None, vec![entry], None);
    let error = manager.eject_tape_action(&vm()).unwrap_err();
    assert!(error.contains("call start_vm first"), "{error}");
}
