//! One manager entry as `list_vms` reports it ([`crate::control::VmInfo`]):
//! lifecycle status plus model, RAM, CPU, cartridge, and mounted media.

use crate::CocoApp;
use crate::control::{Cpu, VmInfo, VmMedia};
use crate::machine_def::{self, CartridgeDTO, MachineDef, SlotDTO};

use super::{MachineEntry, control_status};

/// `entry`'s [`VmInfo`]. Hardware and media come from the live VM when one
/// exists, since media can change while it runs; otherwise from the
/// definition, except a suspended entry's media, which only its saved state
/// records.
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
        cartridge: resolved_cartridge(&entry.def.peripherals.cartridge, &entry.slug),
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

#[cfg(test)]
#[path = "vm_info_test.rs"]
mod tests;
