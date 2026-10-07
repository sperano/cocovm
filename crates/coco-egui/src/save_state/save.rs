//! SAVE side: [`CocoApp::save_state_to`] and the [`MediaRefs`] it builds from
//! every path this app already tracks.

use std::path::{Path, PathBuf};

use coco_core::drivewire;
use coco_core::fdc;
use coco_core::snapshot::{self, CartROMRole, MediaRef, MediaRefs, SlotROMRef};
use coco_core::vhd;

use crate::{CocoApp, MPISlot, ROMSource, orch90_rom_path, rom_db_pseudo_path};

use super::media_ref::hash_media_ref;

pub(crate) const DRIVEWIRE_HOST_BUSY: &str = "DriveWire host I/O is still pending; try again";

impl CocoApp {
    /// [`Self::write_state_to`], then the "State saved" toast.
    pub(crate) fn save_state_to(&mut self, path: &Path) -> Result<(), String> {
        self.write_state_to(path)?;
        self.set_toast("State saved");
        Ok(())
    }

    /// Flush dirty media, build [`MediaRefs`], and write the encoded
    /// `.ccstate` using tmp-then-rename so a crash mid-write can't leave a
    /// truncated file. A flush failure aborts before anything is written.
    /// Shows no toast: callers word their own confirmation.
    pub(super) fn write_state_to(&mut self, path: &Path) -> Result<(), String> {
        if self
            .machine
            .bus
            .drivewire
            .as_ref()
            .is_some_and(|dw| !dw.host_is_idle())
        {
            return Err(DRIVEWIRE_HOST_BUSY.to_string());
        }
        self.flush_media()?;
        let media = self.build_media_refs()?;
        let bytes = snapshot::save(&self.machine, &media).map_err(|e| e.to_string())?;
        let tmp_path = path.with_extension("ccstate.tmp");
        std::fs::write(&tmp_path, bytes)
            .map_err(|e| format!("could not write {}: {e}", tmp_path.display()))?;
        std::fs::rename(&tmp_path, path)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
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

    fn fd502_rom_path(&self) -> Result<PathBuf, String> {
        self.disk_rom_path
            .clone()
            .ok_or_else(|| "FD-502 DOS ROM source is missing".to_string())
    }

    /// [`MediaRefs::cart_roms`]: one entry per ROM image the app tracks a
    /// path for — direct port/MPI slots, the FD-502's Disk BASIC ROM, the
    /// SSC's two ROMs, and the RS-232 pak's optional EPROM.
    fn collect_cart_roms(&mut self) -> Result<Vec<SlotROMRef>, String> {
        let mut paths: Vec<(Option<u8>, CartROMRole, PathBuf)> = Vec::new();
        if let Some(mpi) = &self.mpi {
            for (i, slot) in mpi.slots.iter().enumerate() {
                let mpi_slot = Some(i as u8);
                match slot {
                    MPISlot::ROMPak(p)
                    | MPISlot::BankedROMPak(p)
                    | MPISlot::GamesMasterCartridge(p) => {
                        paths.push((mpi_slot, CartROMRole::Primary, p.clone()));
                    }
                    MPISlot::FD502 => {
                        paths.push((mpi_slot, CartROMRole::Primary, self.fd502_rom_path()?));
                    }
                    MPISlot::Orch90 => {
                        paths.push((mpi_slot, CartROMRole::Primary, orch90_rom_path()));
                    }
                    MPISlot::SoundSpeechCartridge => paths.extend(ssc_rom_paths(mpi_slot)),
                    MPISlot::DeluxeRS232(Some(p)) => {
                        paths.push((mpi_slot, CartROMRole::Primary, p.clone()));
                    }
                    MPISlot::Empty
                    | MPISlot::DistoRTC
                    | MPISlot::DeluxeRS232(None)
                    | MPISlot::CoCoMax => {}
                }
            }
        } else {
            if let Some(path) = &self.cart_path {
                paths.push((None, CartROMRole::Primary, path.clone()));
            } else if self.machine.bus.cart.as_disk_cart().is_some() {
                paths.push((None, CartROMRole::Primary, self.fd502_rom_path()?));
            } else if self.machine.bus.cart.as_ssc().is_some() {
                paths.extend(ssc_rom_paths(None));
            } else if self.machine.bus.cart.as_orch90().is_some() {
                paths.push((None, CartROMRole::Primary, orch90_rom_path()));
            }
            if let Some(path) = &self.rs232_eprom_path {
                paths.push((None, CartROMRole::Primary, path.clone()));
            }
        }
        paths
            .into_iter()
            .map(|(mpi_slot, role, path)| {
                Ok(SlotROMRef {
                    mpi_slot,
                    role,
                    rom: hash_media_ref(&path)?,
                })
            })
            .collect()
    }
}

/// The Sound/Speech Cartridge's two images at `mpi_slot`.
fn ssc_rom_paths(mpi_slot: Option<u8>) -> [(Option<u8>, CartROMRole, PathBuf); 2] {
    [
        (
            mpi_slot,
            CartROMRole::Primary,
            crate::rom_load::sp0256_rom_path(),
        ),
        (
            mpi_slot,
            CartROMRole::SSCFirmware,
            crate::rom_load::ssc_firmware_rom_path(),
        ),
    ]
}
