use std::path::PathBuf;

use coco_core::{MachineVariant, MemorySize};

use super::*;
use crate::control::VmStatus;
use crate::machine_def::DosRom;
use crate::{AppParams, MachineConfig, ROMSource};

const SLUG: &str = "info";

fn off_entry() -> MachineEntry {
    let def =
        machine_def::MachineDef::from_config(SLUG.to_string(), None, &MachineConfig::default());
    MachineEntry::new(SLUG.to_string(), def)
}

/// A Running entry on the default CoCo 3 whose definition claims a 64K
/// CoCo 2, so a test can tell which of the two `vm_info` read.
fn running_entry_with_stale_definition() -> MachineEntry {
    let rom_path = crate::installed_roms_dir().join(crate::rom_load::COCO3_ROM_FILE);
    let rom = std::fs::read(&rom_path)
        .expect("installed coco3.rom is required (first-run asset download)")
        .into_boxed_slice();
    let vm = CocoApp::new(
        MachineConfig::default(),
        rom,
        ROMSource::File(rom_path),
        AppParams::default(),
        crate::joy::SharedGamepad::without_backend(),
    );
    let mut entry = off_entry();
    entry.def.hardware.variant = MachineVariant::Coco2.into();
    entry.def.hardware.ram = MemorySize::K64.into();
    entry.def.media.disk0 = Some("/definition/boot.dsk".to_string());
    entry.vm = Some(Box::new(vm));
    entry
}

#[test]
fn a_live_vm_reports_its_running_hardware_and_current_mounts() {
    let mut entry = running_entry_with_stale_definition();
    let vm = entry.vm.as_mut().unwrap();
    vm.disk_paths[1] = Some(PathBuf::from("/live/work.dsk"));
    vm.tape_path = Some(PathBuf::from("/live/tape.cas"));

    let info = vm_info(&entry);

    assert_eq!(info.status, VmStatus::Running);
    assert_eq!(info.model, MachineConfig::default().variant);
    assert_eq!(info.ram, MachineConfig::default().memory);
    assert_eq!(info.cpu, Cpu::MC6809);
    let media = info.media.expect("a live VM's media is known");
    assert_eq!(media.disks, [None, Some(PathBuf::from("/live/work.dsk"))]);
    assert_eq!(media.tape, Some(PathBuf::from("/live/tape.cas")));
}

#[test]
fn a_powered_off_vm_reports_its_definition() {
    let mut entry = off_entry();
    entry.def.name = "Info Test".to_string();
    entry.def.hardware.variant = MachineVariant::Coco2.into();
    entry.def.hardware.ram = MemorySize::K16.into();
    entry.def.peripherals.cartridge = CartridgeDTO::FD502 {
        dos_rom: DosRom::DiskBasic,
    };
    entry.def.media.disk0 = Some("/defs/boot.dsk".to_string());
    entry.def.media.vhd1 = Some("/defs/hard.vhd".to_string());
    entry.def.media.tape = Some("/defs/tape.cas".to_string());

    let info = vm_info(&entry);

    assert_eq!(info.slug, SLUG);
    assert_eq!(info.name, "Info Test");
    assert_eq!(info.status, VmStatus::PoweredOff);
    assert_eq!(info.model, MachineVariant::Coco2);
    assert_eq!(info.ram, MemorySize::K16);
    assert_eq!(info.cartridge, entry.def.peripherals.cartridge);
    assert_eq!(
        info.media,
        Some(VmMedia {
            disks: [Some(PathBuf::from("/defs/boot.dsk")), None],
            vhds: [None, Some(PathBuf::from("/defs/hard.vhd"))],
            tape: Some(PathBuf::from("/defs/tape.cas")),
            ..VmMedia::default()
        })
    );
}

#[test]
fn drivewire_disks_count_only_while_drivewire_is_enabled() {
    let mut entry = off_entry();
    entry.def.drivewire.disk2 = Some("/defs/dw.dsk".to_string());
    let drivewire = |entry: &MachineEntry| vm_info(entry).media.unwrap().drivewire;

    assert_eq!(drivewire(&entry), [None, None, None, None]);
    entry.def.drivewire.enabled = true;
    assert_eq!(
        drivewire(&entry),
        [None, None, Some(PathBuf::from("/defs/dw.dsk")), None]
    );
}

#[test]
fn a_suspended_vm_with_its_window_closed_has_unknown_media() {
    let mut entry = off_entry();
    entry.suspended = true;
    entry.def.media.disk0 = Some("/defs/boot.dsk".to_string());

    let info = vm_info(&entry);

    assert_eq!(info.status, VmStatus::Suspended);
    assert_eq!(info.media, None);
}

#[test]
fn cartridge_rom_paths_resolve_like_launch_does() {
    let resolved = |raw: &str| {
        machine_def::resolve_media_path(raw, SLUG)
            .display()
            .to_string()
    };
    let mut slots: [SlotDTO; crate::MPI_SLOT_COUNT] = Default::default();
    slots[0] = SlotDTO::GamesMaster {
        path: "gmc.rom".to_string(),
    };
    slots[1] = SlotDTO::ROMPak {
        path: "/abs/pak.rom".to_string(),
    };
    let mpi = CartridgeDTO::MPI { slots, switch: 1 };

    let CartridgeDTO::MPI { slots, switch } = resolved_cartridge(&mpi, SLUG) else {
        panic!("still a MultiPak");
    };

    assert_eq!(switch, 1);
    assert_eq!(
        slots[0],
        SlotDTO::GamesMaster {
            path: resolved("gmc.rom")
        }
    );
    assert_eq!(
        slots[1],
        SlotDTO::ROMPak {
            path: "/abs/pak.rom".to_string()
        }
    );
    let direct = CartridgeDTO::BankedROMPak {
        path: "banked.rom".to_string(),
    };
    assert_eq!(
        resolved_cartridge(&direct, SLUG),
        CartridgeDTO::BankedROMPak {
            path: resolved("banked.rom")
        }
    );
}
