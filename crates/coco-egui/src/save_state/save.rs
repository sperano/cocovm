//! SAVE side: [`CocoApp::save_state_to`] and the [`MediaRefs`] it builds from
//! every path this app already tracks.

use std::path::{Path, PathBuf};

use coco_core::drivewire;
use coco_core::fdc;
use coco_core::snapshot::{self, MediaRef, MediaRefs, SlotRomRef};
use coco_core::vhd;

use crate::{CocoApp, MPISlot, RomSource, disk_basic_rom_path, rom_db_pseudo_path};

use super::media_ref::hash_media_ref;

impl CocoApp {
    /// Flush dirty media first (so path+hash refs describe the on-disk
    /// truth — [`snapshot::save`]'s own contract), build [`MediaRefs`] from
    /// the paths this app already tracks, and write the encoded `.ccstate`
    /// bytes to `path`.
    pub(crate) fn save_state_to(&mut self, path: &Path) -> Result<(), String> {
        self.flush_media();
        let media = self.build_media_refs()?;
        let bytes = snapshot::save(&self.machine, &media).map_err(|e| e.to_string())?;
        std::fs::write(path, bytes)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        self.set_toast("State saved".to_string());
        Ok(())
    }

    /// Build this app's [`MediaRefs`] from the paths it already tracks
    /// (`cart_path`/`mpi`/`rs232_eprom_path`/`disk_paths`/`vhd_paths`/
    /// `dw_paths`/`tape_path`), hashing each referenced FILE — pak images
    /// are mirror-filled in memory, so in-memory bytes never equal file
    /// bytes, hence always [`snapshot::sha256_file`] on the source.
    ///
    /// `pub(super)`, not private: `save_state_test.rs` (`super::tests`, a
    /// sibling of this module under `save_state`) calls it directly to
    /// check the refs it builds against the mounted fixtures' real hashes.
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

        Ok(MediaRefs { system_rom, cart_roms, disks, vhds, drivewire, tape })
    }

    /// [`MediaRefs::system_rom`]: the frontend resolved/composed the boot
    /// ROM at boot time ([`RomSource`]) — a real file records its path but
    /// hashes the boot-time bytes live from `self.machine.bus.rom` rather
    /// than re-reading the file (a ROM file modified mid-session should
    /// produce a mismatch WARNING on restore, not a silently-passing wrong
    /// hash computed from bytes the running machine never actually used); a
    /// CoCo 1/2 `rom_db`-composed image has no single file, so this records
    /// a pseudo-path ([`rom_db_pseudo_path`]) and hashes the composed bytes
    /// the same way — this arm already worked this way before this
    /// function's `File` arm was brought in line with it.
    fn system_rom_media_ref(&self) -> MediaRef {
        let path = match &self.rom_source {
            RomSource::File(path) => path.clone(),
            RomSource::ComposedCoco12 => rom_db_pseudo_path(self.machine.config.variant),
        };
        MediaRef { path, sha256: snapshot::sha256_hex(&self.machine.bus.rom) }
    }

    /// [`MediaRefs::cart_roms`]: one entry per ROM-bearing cart the app
    /// tracks a path for — `cart_path`/`mpi` (direct port / MultiPak slots)
    /// plus the FD-502's Disk BASIC ROM ([`disk_basic_rom_path`]) and the
    /// Deluxe RS-232 pak's optional EPROM (`rs232_eprom_path`, only present
    /// when one was actually mounted — the pak also runs ROM-less).
    fn collect_cart_roms(&mut self) -> Result<Vec<SlotRomRef>, String> {
        let mut out = Vec::new();
        if let Some(mpi) = &self.mpi {
            // Snapshot the slot paths first — this ends the immutable
            // borrow of `self.mpi` before the hashing loop below needs
            // `self` again for `hash_media_ref`'s error path.
            let paths: Vec<(Option<u8>, PathBuf)> = mpi
                .slots
                .iter()
                .enumerate()
                .filter_map(|(i, slot)| {
                    let path = match slot {
                        MPISlot::ROMPak(p) | MPISlot::Gmc(p) | MPISlot::Orch90(p) => p.clone(),
                        MPISlot::FD502 => disk_basic_rom_path(),
                        MPISlot::Empty | MPISlot::DistoRTC | MPISlot::Ssc => return None,
                    };
                    Some((Some(i as u8), path))
                })
                .collect();
            for (mpi_slot, path) in paths {
                out.push(SlotRomRef { mpi_slot, rom: hash_media_ref(&path)? });
            }
        } else {
            if let Some(path) = &self.cart_path {
                out.push(SlotRomRef { mpi_slot: None, rom: hash_media_ref(path)? });
            } else if self.machine.bus.cart.as_disk_cart().is_some() {
                out.push(SlotRomRef {
                    mpi_slot: None,
                    rom: hash_media_ref(&disk_basic_rom_path())?,
                });
            }
            if let Some(path) = &self.rs232_eprom_path {
                out.push(SlotRomRef { mpi_slot: None, rom: hash_media_ref(path)? });
            }
        }
        Ok(out)
    }
}
