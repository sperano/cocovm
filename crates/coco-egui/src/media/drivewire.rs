//! The DriveWire virtual serial link and the disk images it serves.

use crate::*;

impl CocoApp {
    /// Enable the Becker port ($FF41/$FF42) with a real wall clock, optionally
    /// in HDB-DOS sector addressing mode. Idempotent — if already enabled, does
    /// nothing. Failures land in [`Self::cart_error`].
    pub(crate) fn enable_drivewire(&mut self, hdbdos_mode: bool) {
        self.machine.bus.enable_drivewire();
        if let Some(ref mut dw) = self.machine.bus.drivewire {
            dw.set_hdbdos_mode(hdbdos_mode);
            // Inject real wall clock from the host.
            dw.set_clock(host_dw_clock());
        }
    }

    /// Disable the Becker port, ejecting all mounted DriveWire images and
    /// clearing the path tracking.
    pub(crate) fn disable_drivewire(&mut self) {
        self.machine.bus.drivewire = None;
        self.dw_paths = std::array::from_fn(|_| None);
    }

    /// Mount the DriveWire image at `path` in `drive`. Like VHD, writes hit the
    /// backing file directly. Failures land in [`Self::cart_error`].
    pub(crate) fn insert_dw_disk(&mut self, drive: usize, path: PathBuf) {
        let result = (|| -> Result<(), String> {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .map_err(|e| format!("could not open {}: {e}", path.display()))?;
            if let Some(ref mut dw) = self.machine.bus.drivewire {
                dw.mount(drive, DWImage::File(file));
                self.dw_paths[drive] = Some(path);
            } else {
                return Err("Becker port not enabled".to_string());
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.cart_error = Some(e);
        }
    }

    /// Eject the DriveWire image in `drive`. No write-back: writes already hit
    /// the backing file directly.
    pub(crate) fn eject_dw_disk(&mut self, drive: usize) {
        if let Some(ref mut dw) = self.machine.bus.drivewire {
            dw.eject(drive);
        }
        self.dw_paths[drive] = None;
    }
}
