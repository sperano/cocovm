//! FD-502 floppies and virtual hard disks: mounting, formatting blanks,
//! ejecting, and writing modified images back to their files.

use crate::*;

impl CocoApp {
    /// Make sure the inserted cartridge is the FD-502 disk controller,
    /// creating one (with `roms/disk11.rom`) if something else — or nothing —
    /// is in the slot. Creating it cold-resets the machine: BASIC only probes
    /// for Disk BASIC at cold start. Swapping a floppy in an already-present
    /// controller does NOT reset, like on real hardware.
    ///
    /// With a Multi-Pak Interface installed, the slot to plug the FD-502 into
    /// is a real choice a top-level "just ensure a controller exists" call
    /// can't make on the caller's behalf — so this refuses instead of
    /// silently replacing the MPI, and directs the caller to
    /// [`Self::mpi_insert_fd502`] via the MultiPak submenu.
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
        self.flush_dirty_disks_or_report();
        self.machine
            .insert_cartridge(DiskCart::new(rom.into_boxed_slice()));
        // Power cycle, not warm reset: the DK probe that links Disk BASIC
        // only runs on the ROM's cold-start path (a warm reset leaves the
        // DOS ROM unlinked and the drives dead).
        self.machine.power_cycle();
        self.cart_path = None;
        self.disk_paths = [None, None];
        self.rs232 = None;
        self.rs232_eprom_path = None;
        self.rtc_direct = false;
        Ok(())
    }

    /// Menu-path entry for Insert Disk: acts immediately when the FD-502 is
    /// already in the slot; otherwise parks the action behind the
    /// power-cycle confirmation dialog (see [`Self::pending_disk_action`]).
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

    /// Mount the floppy image at `path` in `drive`, inserting the FD-502
    /// controller first if needed. Failures land in [`Self::cart_error`].
    pub(crate) fn insert_disk(&mut self, drive: usize, path: PathBuf) {
        let result = (|| -> Result<(), String> {
            self.ensure_disk_controller()?;
            let bytes = std::fs::read(&path)
                .map_err(|e| format!("could not read {}: {e}", path.display()))?;
            let disk =
                JVCDisk::from_bytes(bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            // Whatever was in the drive first; a failed write-back reports
            // through `cart_error` but doesn't block mounting the new disk.
            if let Err(e) = self.write_back_disk(drive) {
                self.cart_error = Some(e);
            }
            let cart = self.machine.bus.cart.as_disk_cart().expect("just ensured");
            cart.insert_disk(drive, disk);
            self.disk_paths[drive] = Some(path);
            Ok(())
        })();
        if let Err(e) = result {
            self.cart_error = Some(e);
        }
    }

    /// Create a brand-new, blank (0-track) floppy image at `path` and mount it
    /// in `drive`, inserting the FD-502 controller first if needed. Refuses to
    /// overwrite an existing file. Failures land in [`Self::cart_error`].
    pub(crate) fn new_blank_disk(&mut self, drive: usize, path: PathBuf) {
        let result = (|| -> Result<(), String> {
            self.ensure_disk_controller()?;
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
            // Whatever was in the drive first; a failed write-back reports
            // through `cart_error` but doesn't block mounting the new disk.
            if let Err(e) = self.write_back_disk(drive) {
                self.cart_error = Some(e);
            }
            let cart = self.machine.bus.cart.as_disk_cart().expect("just ensured");
            cart.insert_disk(drive, disk);
            self.disk_paths[drive] = Some(path);
            Ok(())
        })();
        if let Err(e) = result {
            self.cart_error = Some(e);
        }
    }

    /// Eject the floppy in `drive`, writing a modified image back to its file
    /// first (like MAME/VCC, in-place). Failures land in [`Self::cart_error`].
    pub(crate) fn eject_disk(&mut self, drive: usize) {
        if let Err(e) = self.write_back_disk(drive) {
            self.cart_error = Some(e);
        }
        if let Some(cart) = self.machine.bus.cart.as_disk_cart() {
            cart.eject_disk(drive);
        }
        self.disk_paths[drive] = None;
    }

    /// If the floppy in `drive` was written to, save the image back to its
    /// source file and mark it saved ([`JVCDisk::mark_saved`]). On
    /// failure the in-memory disk is left mounted and still dirty, so a
    /// later retry can succeed; callers decide how to surface the error
    /// (most route it to [`Self::cart_error`] via
    /// [`Self::flush_dirty_disks_or_report`] or inline, but
    /// [`Self::flush_dirty_disks`] instead collects it to aggregate with
    /// its sibling drives' errors).
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

    /// Write every modified floppy back to its file (controller swap, exit).
    /// Tries every drive even after one fails — each drive's data is
    /// independent, so one bad write-back shouldn't leave a healthy drive's
    /// changes unsaved too — and joins every error message with `\n`.
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

    /// [`Self::flush_dirty_disks`], reporting a failure through
    /// [`Self::cart_error`] instead of propagating it — the shape every
    /// call site that isn't itself building a `Result` (cartridge/MPI swaps,
    /// controller setup) wants.
    pub(crate) fn flush_dirty_disks_or_report(&mut self) {
        if let Err(e) = self.flush_dirty_disks() {
            self.cart_error = Some(e);
        }
    }

    /// Mount the VHD image at `path` in `drive`. Unlike floppies, VHD is a
    /// bus-level device (`$FF80-$FF86`, `SystemBus::vhd`) independent of the
    /// cartridge slot: no controller to ensure, no machine reset, and no
    /// write-back on eject/replace (VHD command execution writes straight
    /// through to the backing file). Failures land in [`Self::cart_error`].
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
