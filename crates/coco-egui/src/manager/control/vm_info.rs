//! One manager entry as `list_vms` reports it ([`crate::control::VmInfo`]):
//! lifecycle status plus model, RAM, CPU, cartridge, and mounted media.

use std::path::Path;

use coco_core::cart::Cart;

use crate::control::{Cpu, VmInfo, VmMedia};
use crate::machine_def::{self, CartridgeDTO, DosRom, MachineDef, RS232EndpointDTO, SlotDTO};
use crate::{CocoApp, MPISlot, RS232Endpoint};

use super::{MachineEntry, control_status};

/// `entry`'s [`VmInfo`]. Hardware, cartridge, and media come from the live
/// VM when one exists, since the cartridge and media can change while it
/// runs; otherwise from the definition, except a suspended entry's media,
/// which only its saved state records.
pub(super) fn vm_info(entry: &MachineEntry) -> VmInfo {
    let live = entry.vm.as_deref();
    let (model, ram) = match live {
        Some(vm) => (vm.machine.config.variant, vm.machine.config.memory),
        None => (
            entry.def.hardware.variant.into(),
            entry.def.hardware.ram.into(),
        ),
    };
    let media = match live {
        Some(vm) => Some(live_media(vm)),
        None if entry.suspended => None,
        None => Some(configured_media(&entry.def, &entry.slug)),
    };
    VmInfo {
        slug: entry.slug.clone(),
        name: entry.def.name.clone(),
        status: control_status(entry),
        model,
        ram,
        cpu: Cpu::MC6809,
        cartridge: match live {
            Some(vm) => live_cartridge(vm),
            None => resolved_cartridge(&entry.def.peripherals.cartridge, &entry.slug),
        },
        media,
    }
}

fn live_media(vm: &CocoApp) -> VmMedia {
    VmMedia {
        disks: vm.disk_paths.clone(),
        vhds: vm.vhd_paths.clone(),
        drivewire: vm.dw_paths.clone(),
        tape: vm.tape_path.clone(),
    }
}

/// The media launching `def` mounts, resolved the way `launch` resolves
/// `[media]` and `[drivewire]`.
fn configured_media(def: &MachineDef, slug: &str) -> VmMedia {
    let resolve = |raw: &Option<String>| {
        raw.as_deref()
            .map(|p| machine_def::resolve_media_path(p, slug))
    };
    VmMedia {
        disks: [resolve(&def.media.disk0), resolve(&def.media.disk1)],
        vhds: [resolve(&def.media.vhd0), resolve(&def.media.vhd1)],
        drivewire: crate::launch::drivewire_settings(def, slug)
            .map(|dw| dw.disk_paths)
            .unwrap_or_default(),
        tape: resolve(&def.media.tape),
    }
}

/// `cartridge` with every embedded ROM path resolved against `slug`'s
/// artifact directory, as `launch::resolve_cartridge` resolves it.
fn resolved_cartridge(cartridge: &CartridgeDTO, slug: &str) -> CartridgeDTO {
    let resolve = |path: &mut String| {
        *path = machine_def::resolve_media_path(path, slug)
            .display()
            .to_string();
    };
    let mut cartridge = cartridge.clone();
    match &mut cartridge {
        CartridgeDTO::ROMPak { path }
        | CartridgeDTO::BankedROMPak { path }
        | CartridgeDTO::GamesMaster { path } => resolve(path),
        CartridgeDTO::MPI { slots, .. } => {
            for slot in slots {
                match slot {
                    SlotDTO::ROMPak { path }
                    | SlotDTO::BankedROMPak { path }
                    | SlotDTO::GamesMaster { path } => resolve(path),
                    SlotDTO::Empty
                    | SlotDTO::FD502 { .. }
                    | SlotDTO::RTC
                    | SlotDTO::RS232 { .. }
                    | SlotDTO::Orch90
                    | SlotDTO::SoundSpeech
                    | SlotDTO::CoCoMax => {}
                }
            }
        }
        CartridgeDTO::None
        | CartridgeDTO::FD502 { .. }
        | CartridgeDTO::RTC
        | CartridgeDTO::RS232 { .. }
        | CartridgeDTO::Orch90
        | CartridgeDTO::SoundSpeech
        | CartridgeDTO::CoCoMax => {}
    }
    cartridge
}

/// `vm`'s cartridge-port occupant as a definition names it, read from the
/// frontend's record of what each insert installed (`cart_path`, `mpi`,
/// `rs232`, `disk_rom_path`).
fn live_cartridge(vm: &CocoApp) -> CartridgeDTO {
    if let Some(mpi) = &vm.mpi {
        return CartridgeDTO::MPI {
            slots: std::array::from_fn(|i| live_slot(vm, &mpi.slots[i])),
            switch: mpi.switch + 1,
        };
    }
    let path = || path_text(vm.cart_path.as_deref());
    match &vm.machine.bus.cart {
        Cart::ROMPak(_) => CartridgeDTO::ROMPak { path: path() },
        Cart::BankedROMPak(_) => CartridgeDTO::BankedROMPak { path: path() },
        Cart::GamesMasterCartridge(_) => CartridgeDTO::GamesMaster { path: path() },
        Cart::DiskCart(_) => CartridgeDTO::FD502 {
            dos_rom: live_dos_rom(vm),
        },
        Cart::DistoRTC(_) => CartridgeDTO::RTC,
        Cart::DeluxeRS232(_) => CartridgeDTO::RS232 {
            endpoint: live_endpoint(vm),
        },
        Cart::Orch90(_) => CartridgeDTO::Orch90,
        Cart::SoundSpeechCartridge(_) => CartridgeDTO::SoundSpeech,
        Cart::CoCoMaxModule(_) => CartridgeDTO::CoCoMax,
        // An empty port; a MultiPak, which always comes with its `mpi`
        // record (an insert or a restore sets it); or a test double, which
        // has no definition form.
        _ => CartridgeDTO::None,
    }
}

fn live_slot(vm: &CocoApp, slot: &MPISlot) -> SlotDTO {
    let path = |path: &Path| path_text(Some(path));
    match slot {
        MPISlot::Empty => SlotDTO::Empty,
        MPISlot::ROMPak(p) => SlotDTO::ROMPak { path: path(p) },
        MPISlot::BankedROMPak(p) => SlotDTO::BankedROMPak { path: path(p) },
        MPISlot::FD502 => SlotDTO::FD502 {
            dos_rom: live_dos_rom(vm),
        },
        MPISlot::DistoRTC => SlotDTO::RTC,
        MPISlot::DeluxeRS232(_) => SlotDTO::RS232 {
            endpoint: live_endpoint(vm),
        },
        MPISlot::GamesMasterCartridge(p) => SlotDTO::GamesMaster { path: path(p) },
        MPISlot::Orch90 => SlotDTO::Orch90,
        MPISlot::SoundSpeechCartridge => SlotDTO::SoundSpeech,
        MPISlot::CoCoMax => SlotDTO::CoCoMax,
    }
}

fn path_text(path: Option<&Path>) -> String {
    path.map(|p| p.display().to_string()).unwrap_or_default()
}

/// The FD-502's DOS, told apart by the installed ROM's file name
/// ([`DosRom::filename`]).
fn live_dos_rom(vm: &CocoApp) -> DosRom {
    let file_name = vm.disk_rom_path.as_deref().and_then(Path::file_name);
    [DosRom::DiskBasic, DosRom::HdbDosDw3]
        .into_iter()
        .find(|rom| file_name.is_some_and(|name| name == rom.filename()))
        .unwrap_or_default()
}

fn live_endpoint(vm: &CocoApp) -> RS232EndpointDTO {
    match &vm.rs232 {
        None | Some(RS232Endpoint::Loopback) => RS232EndpointDTO::Loopback,
        Some(RS232Endpoint::TCP(listen)) => RS232EndpointDTO::TCP {
            listen: listen.clone(),
        },
        #[cfg(unix)]
        Some(RS232Endpoint::PTY(_)) => RS232EndpointDTO::PTY,
    }
}

#[cfg(test)]
#[path = "vm_info_test.rs"]
mod tests;
