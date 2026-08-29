//! SAVE side: [`CocoApp::save_state_to`] and the [`MediaRefs`] it builds from
//! every path this app already tracks.

use std::path::{Path, PathBuf};

use coco_core::drivewire;
use coco_core::fdc;
use coco_core::snapshot::{self, MediaRef, MediaRefs, SlotROMRef};
use coco_core::vhd;

use crate::{CocoApp, MPISlot, ROMSource, disk_basic_rom_path, rom_db_pseudo_path};

use super::media_ref::hash_media_ref;

impl CocoApp {
    /// Flush dirty media, build [`MediaRefs`], and write the encoded
    /// `.ccstate` using tmp-then-rename so a crash mid-write can't leave a
    /// truncated file. A flush failure aborts before anything is written.
    pub(crate) fn save_state_to(&mut self, path: &Path) -> Result<(), String> {
        self.flush_media()?;
        let media = self.build_media_refs()?;
        let bytes = snapshot::save(&self.machine, &media).map_err(|e| e.to_string())?;
        let tmp_path = path.with_extension("ccstate.tmp");
        std::fs::write(&tmp_path, bytes)
            .map_err(|e| format!("could not write {}: {e}", tmp_path.display()))?;
        std::fs::rename(&tmp_path, path)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        self.set_toast("State saved".to_string());
        Ok(())
    }

    /// Build this app's [`MediaRefs`] from the paths it already tracks,
    /// always hashing the referenced file (not in-memory bytes, which may differ).
    pub(super) fn build_media_refs(&mut self) -> Result<MediaRefs, String> {
        let system_rom = Some(self.system_rom_media_ref());
        let cart_roms = self.collect_cart_roms()?;

        let mut disks: Vec<Option<MediaRef>> = vec![None; fdc::DRIVE_COUNT];
        for (i, path) in self.disk_paths.iter().enumerate() {
            if let Some(path) = path {
                disks[i] = Some(hash_media_ref(path)?);
            }
        }
        let mut vhds: Vec<Option<MediaRef>> = vec![None; vhd::DRIVE_COUNT];
        for (i, path) in self.vhd_paths.iter().enumerate() {
            if let Some(path) = path {
                vhds[i] = Some(hash_media_ref(path)?);
            }
        }
        let mut drivewire: Vec<Option<MediaRef>> = vec![None; drivewire::DRIVE_COUNT];
        for (i, path) in self.dw_paths.iter().enumerate() {
            if let Some(path) = path {
                drivewire[i] = Some(hash_media_ref(path)?);
            }
        }
        let tape = match &self.tape_path {
            Some(path) => Some(hash_media_ref(path)?),
            None => None,
        };

        Ok(MediaRefs {
            system_rom,
            cart_roms,
            disks,
            vhds,
            drivewire,
            tape,
        })
    }

    /// [`MediaRefs::system_rom`]: hashes the boot-time bytes live from
    /// `self.machine.bus.rom`, not a re-read of the file, so a ROM edited
    /// mid-session produces a mismatch warning on restore, not a false pass.
    fn system_rom_media_ref(&self) -> MediaRef {
        let path = match &self.rom_source {
            ROMSource::File(path) => path.clone(),
            ROMSource::ComposedCoco12 => rom_db_pseudo_path(self.machine.config.variant),
        };
        MediaRef {
            path,
            sha256: snapshot::sha256_hex(&self.machine.bus.rom),
        }
    }

    /// [`MediaRefs::cart_roms`]: one entry per ROM-bearing cart the app
    /// tracks a path for — direct port/MPI slots, the FD-502's Disk BASIC
    /// ROM, and the RS-232 pak's optional EPROM.
    fn collect_cart_roms(&mut self) -> Result<Vec<SlotROMRef>, String> {
        let mut out = Vec::new();
        if let Some(mpi) = &self.mpi {
            // Snapshot paths first to end the immutable borrow of self.mpi before hashing needs
            // self again.
            let paths: Vec<(Option<u8>, PathBuf)> = mpi
                .slots
                .iter()
                .enumerate()
                .filter_map(|(i, slot)| {
                    let path = match slot {
                        MPISlot::ROMPak(p)
                        | MPISlot::GamesMasterCartridge(p)
                        | MPISlot::Orch90(p) => p.clone(),
                        MPISlot::FD502 => disk_basic_rom_path(),
                        MPISlot::DeluxeRS232(Some(p)) => p.clone(),
                        MPISlot::Empty
                        | MPISlot::DistoRTC
                        | MPISlot::DeluxeRS232(None)
                        | MPISlot::SoundSpeechCartridge => {
                            return None;
                        }
                    };
                    Some((Some(i as u8), path))
                })
                .collect();
            for (mpi_slot, path) in paths {
                out.push(SlotROMRef {
                    mpi_slot,
                    rom: hash_media_ref(&path)?,
                });
            }
        } else {
            if let Some(path) = &self.cart_path {
                out.push(SlotROMRef {
                    mpi_slot: None,
                    rom: hash_media_ref(path)?,
                });
            } else if self.machine.bus.cart.as_disk_cart().is_some() {
                out.push(SlotROMRef {
                    mpi_slot: None,
                    rom: hash_media_ref(&disk_basic_rom_path())?,
                });
            }
            if let Some(path) = &self.rs232_eprom_path {
                out.push(SlotROMRef {
                    mpi_slot: None,
                    rom: hash_media_ref(path)?,
                });
            }
        }
        Ok(out)
    }
}
