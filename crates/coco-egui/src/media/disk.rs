//! FD-502 floppies and virtual hard disks: mounting, formatting blanks,
//! ejecting, and writing modified images back to their files.

use crate::*;

impl CocoApp {
    /// Ensures the FD-502 controller is inserted, creating one (cold-resetting the machine —
    /// BASIC only probes for Disk BASIC at cold start) if needed. Swapping
    /// a floppy in an already-present controller does not reset. Refuses
    /// if an MPI is installed; use [`Self::mpi_insert_fd502`] instead.
    pub(crate) fn ensure_disk_controller(&mut self) -> Result<(), String> {
        if self.machine.bus.cart.as_disk_cart().is_some() {
            return Ok(());
        }
        if self.mpi.is_some() {
            return Err(
                "No FD-502 is installed in the MultiPak. Use Machine > MultiPak Interface > \
                 a slot > Insert FD-502 first."
                    .to_string(),
            );
        }
        let path = disk_basic_rom_path();
        let rom = std::fs::read(&path)
            .map_err(|e| format!("could not read Disk BASIC ROM {}: {e}", path.display()))?;
        report_rom_validation(&path, &rom);
        // No flush needed: past the early returns, no disk cart can exist here.
        self.machine
            .insert_cartridge(DiskCart::new(rom.into_boxed_slice()));
        // Power cycle, not warm reset: the DK probe that links Disk BASIC only runs on cold-start.
        self.power_cycle();
        self.cart_path = None;
        self.disk_paths = [None, None];
        self.rs232 = None;
        self.rs232_eprom_path = None;
        self.rtc_direct = false;
        Ok(())
    }

    /// Menu-path entry for Insert Disk: acts immediately if the FD-502 is already in the
    /// slot, otherwise parks behind the power-cycle confirmation dialog.
    pub(crate) fn request_insert_disk(&mut self, drive: usize, path: PathBuf) {
        if self.machine.bus.cart.as_disk_cart().is_some() {
            self.insert_disk(drive, path);
        } else {
            self.pending_disk_action = Some(PendingDiskAction::Insert { drive, path });
        }
    }

    /// Menu-path entry for New Blank Disk, gated like [`Self::request_insert_disk`].
    pub(crate) fn request_new_blank_disk(&mut self, drive: usize, path: PathBuf) {
        if self.machine.bus.cart.as_disk_cart().is_some() {
            self.new_blank_disk(drive, path);
        } else {
            self.pending_disk_action = Some(PendingDiskAction::NewBlank { drive, path });
        }
    }

    /// Mounts the floppy image at `path` in `drive`, inserting the FD-502 controller first
    /// if needed. A failed write-back of the drive's old disk aborts the
    /// mount, leaving it dirty and tracked for retry.
    pub(crate) fn insert_disk(&mut self, drive: usize, path: PathBuf) {
        let result = (|| -> Result<(), String> {
            self.ensure_disk_controller()?;
            let bytes = std::fs::read(&path)
                .map_err(|e| format!("could not read {}: {e}", path.display()))?;
            let disk =
                JVCDisk::from_bytes(bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            self.write_back_disk(drive)?;
            let cart = self.machine.bus.cart.as_disk_cart().expect("just ensured");
            cart.insert_disk(drive, disk);
            self.disk_paths[drive] = Some(path);
            Ok(())
        })();
        if let Err(e) = result {
            self.cart_error = Some(e);
        }
    }

    /// Creates a brand-new, blank (0-track) floppy image at `path` and mounts it in `drive`,
    /// inserting the FD-502 controller first if needed. Refuses to
    /// overwrite an existing file; a failed write-back of the drive's old
    /// disk aborts before the file is created.
    pub(crate) fn new_blank_disk(&mut self, drive: usize, path: PathBuf) {
        let result = (|| -> Result<(), String> {
            self.ensure_disk_controller()?;
            self.write_back_disk(drive)?;
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(format!(
                        "{} already exists; use Insert Disk to mount an existing image, or \
                         choose a different name",
                        path.display()
                    ));
                }
                Err(e) => return Err(format!("could not create {}: {e}", path.display())),
            }
            let disk =
                JVCDisk::from_bytes(Vec::new()).map_err(|e| format!("{}: {e}", path.display()))?;
            let cart = self.machine.bus.cart.as_disk_cart().expect("just ensured");
            cart.insert_disk(drive, disk);
            self.disk_paths[drive] = Some(path);
            Ok(())
        })();
        if let Err(e) = result {
            self.cart_error = Some(e);
        }
    }

    /// Ejects the floppy in `drive`, writing a modified image back to its file first
    /// (in-place, like MAME/VCC). A failed write-back aborts the eject,
    /// leaving the disk mounted and dirty for a later retry.
    pub(crate) fn eject_disk(&mut self, drive: usize) {
        if let Err(e) = self.write_back_disk(drive) {
            self.cart_error = Some(e);
            return;
        }
        if let Some(cart) = self.machine.bus.cart.as_disk_cart() {
            cart.eject_disk(drive);
        }
        self.disk_paths[drive] = None;
    }

    /// Saves the floppy in `drive` back to its source file if it was written to, marking it
    /// saved. On failure the disk stays mounted and dirty so a later retry can succeed.
    pub(crate) fn write_back_disk(&mut self, drive: usize) -> Result<(), String> {
        let Some(path) = self.disk_paths[drive].clone() else {
            return Ok(());
        };
        let Some(cart) = self.machine.bus.cart.as_disk_cart() else {
            return Ok(());
        };
        let Some(disk) = cart.disk_mut(drive) else {
            return Ok(());
        };
        if !disk.dirty() {
            return Ok(());
        }
        std::fs::write(&path, disk.bytes())
            .map_err(|e| format!("could not save {}: {e}", path.display()))?;
        disk.mark_saved();
        Ok(())
    }

    /// Writes every modified floppy back to its file. Tries every drive even after one
    /// fails, since each drive's data is independent; joins every error message with `\n`.
    pub(crate) fn flush_dirty_disks(&mut self) -> Result<(), String> {
        let errors: Vec<String> = (0..UI_DRIVES)
            .filter_map(|drive| self.write_back_disk(drive).err())
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("\n"))
        }
    }

    /// [`Self::flush_dirty_disks`], reporting failure via [`Self::cart_error`] instead of
    /// propagating it. Returns whether it's safe to proceed — every
    /// cartridge/MPI swap that could destroy a disk cart must check this
    /// first.
    #[must_use]
    pub(crate) fn flush_dirty_disks_or_report(&mut self) -> bool {
        match self.flush_dirty_disks() {
            Ok(()) => true,
            Err(e) => {
                self.cart_error = Some(e);
                false
            }
        }
    }

    /// Mounts the VHD image at `path` in `drive`. Unlike floppies, VHD is a bus-level device
    /// (`$FF80-$FF86`) independent of the cartridge slot — no controller,
    /// no reset, no write-back on eject (writes go straight through).
    pub(crate) fn insert_vhd(&mut self, drive: usize, path: PathBuf) {
        let result = (|| -> Result<(), String> {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .map_err(|e| format!("could not open {}: {e}", path.display()))?;
            self.machine.bus.vhd.insert(drive, VHDImage::File(file));
            self.vhd_paths[drive] = Some(path);
            Ok(())
        })();
        if let Err(e) = result {
            self.cart_error = Some(e);
        }
    }

    /// Eject the VHD image in `drive`. No write-back: VHD writes already hit
    /// the backing file directly.
    pub(crate) fn eject_vhd(&mut self, drive: usize) {
        self.machine.bus.vhd.eject(drive);
        self.vhd_paths[drive] = None;
    }
}

#[cfg(test)]
#[path = "disk_test.rs"]
mod tests;
