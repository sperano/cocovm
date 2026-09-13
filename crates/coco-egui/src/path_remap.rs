//! Path migration for a machine artifact-directory rename.

use std::path::{Path, PathBuf};

use coco_core::snapshot::{MediaRef, MediaRefs};

use crate::machine_def::{CartridgeDTO, MachineDef, SlotDTO};
use crate::{CocoApp, MPISlot, ROMSource};

fn remapped(path: &Path, old_dir: &Path, new_dir: &Path) -> Option<PathBuf> {
    path.strip_prefix(old_dir)
        .ok()
        .map(|suffix| new_dir.join(suffix))
}

fn remap_path(path: &mut PathBuf, old_dir: &Path, new_dir: &Path) {
    if let Some(new_path) = remapped(path, old_dir, new_dir) {
        *path = new_path;
    }
}

fn remap_optional_path(path: &mut Option<PathBuf>, old_dir: &Path, new_dir: &Path) {
    if let Some(path) = path {
        remap_path(path, old_dir, new_dir);
    }
}

fn remap_string(path: &mut String, old_dir: &Path, new_dir: &Path) {
    let parsed = Path::new(path);
    if parsed.is_absolute()
        && let Some(new_path) = remapped(parsed, old_dir, new_dir)
    {
        *path = new_path.to_string_lossy().into_owned();
    }
}

fn remap_optional_string(path: &mut Option<String>, old_dir: &Path, new_dir: &Path) {
    if let Some(path) = path {
        remap_string(path, old_dir, new_dir);
    }
}

fn remap_slot_dto(slot: &mut SlotDTO, old_dir: &Path, new_dir: &Path) {
    match slot {
        SlotDTO::ROMPak { path, .. }
        | SlotDTO::GamesMaster { path, .. }
        | SlotDTO::Orch90 { path } => remap_string(path, old_dir, new_dir),
        SlotDTO::Empty
        | SlotDTO::FD502
        | SlotDTO::RTC
        | SlotDTO::RS232 { .. }
        | SlotDTO::SoundSpeech => {}
    }
}

fn remap_cartridge_dto(cartridge: &mut CartridgeDTO, old_dir: &Path, new_dir: &Path) {
    match cartridge {
        CartridgeDTO::ROMPak { path, .. }
        | CartridgeDTO::GamesMaster { path, .. }
        | CartridgeDTO::Orch90 { path } => remap_string(path, old_dir, new_dir),
        CartridgeDTO::MPI { slots, .. } => {
            for slot in slots {
                remap_slot_dto(slot, old_dir, new_dir);
            }
        }
        CartridgeDTO::None
        | CartridgeDTO::FD502
        | CartridgeDTO::RTC
        | CartridgeDTO::RS232 { .. }
        | CartridgeDTO::SoundSpeech => {}
    }
}

pub(crate) fn remap_definition_paths(def: &mut MachineDef, old_dir: &Path, new_dir: &Path) {
    remap_optional_string(&mut def.hardware.rom, old_dir, new_dir);
    remap_optional_string(&mut def.media.disk0, old_dir, new_dir);
    remap_optional_string(&mut def.media.disk1, old_dir, new_dir);
    remap_optional_string(&mut def.media.vhd0, old_dir, new_dir);
    remap_optional_string(&mut def.media.vhd1, old_dir, new_dir);
    remap_optional_string(&mut def.media.tape, old_dir, new_dir);
    remap_cartridge_dto(&mut def.peripherals.cartridge, old_dir, new_dir);
}

fn remap_mpi_slot(slot: &mut MPISlot, old_dir: &Path, new_dir: &Path) {
    match slot {
        MPISlot::ROMPak(path) | MPISlot::GamesMasterCartridge(path) | MPISlot::Orch90(path) => {
            remap_path(path, old_dir, new_dir)
        }
        MPISlot::DeluxeRS232(Some(path)) => remap_path(path, old_dir, new_dir),
        MPISlot::Empty
        | MPISlot::FD502
        | MPISlot::DistoRTC
        | MPISlot::DeluxeRS232(None)
        | MPISlot::SoundSpeechCartridge => {}
    }
}

impl CocoApp {
    pub(crate) fn remap_managed_paths(&mut self, old_dir: &Path, new_dir: &Path) {
        remap_optional_path(&mut self.cart_path, old_dir, new_dir);
        for path in &mut self.disk_paths {
            remap_optional_path(path, old_dir, new_dir);
        }
        for path in &mut self.vhd_paths {
            remap_optional_path(path, old_dir, new_dir);
        }
        for path in &mut self.dw_paths {
            remap_optional_path(path, old_dir, new_dir);
        }
        remap_optional_path(&mut self.tape_path, old_dir, new_dir);
        remap_optional_path(&mut self.print_capture_path, old_dir, new_dir);
        remap_optional_path(&mut self.rs232_eprom_path, old_dir, new_dir);
        if let Some(mpi) = &mut self.mpi {
            for slot in &mut mpi.slots {
                remap_mpi_slot(slot, old_dir, new_dir);
            }
        }
        if let ROMSource::File(path) = &mut self.rom_source {
            remap_path(path, old_dir, new_dir);
        }
    }
}

fn remap_media_ref(media_ref: &mut Option<MediaRef>, old_dir: &Path, new_dir: &Path) {
    if let Some(media_ref) = media_ref {
        remap_path(&mut media_ref.path, old_dir, new_dir);
    }
}

pub(crate) fn remap_media_refs(media: &mut MediaRefs, old_dir: &Path, new_dir: &Path) {
    remap_media_ref(&mut media.system_rom, old_dir, new_dir);
    for cart in &mut media.cart_roms {
        remap_path(&mut cart.rom.path, old_dir, new_dir);
    }
    for media_ref in &mut media.disks {
        remap_media_ref(media_ref, old_dir, new_dir);
    }
    for media_ref in &mut media.vhds {
        remap_media_ref(media_ref, old_dir, new_dir);
    }
    for media_ref in &mut media.drivewire {
        remap_media_ref(media_ref, old_dir, new_dir);
    }
    remap_media_ref(&mut media.tape, old_dir, new_dir);
}

#[cfg(test)]
#[path = "path_remap_test.rs"]
mod tests;
