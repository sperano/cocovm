//! The DriveWire virtual serial link and the disk images it serves.

use crate::app::DriveWireLaunch;
use crate::*;

impl CocoApp {
    /// Enables the Becker port ($FF41/$FF42) with a real wall clock, optionally
    /// in HDB-DOS mode. Idempotent; failures land in [`Self::cart_error`].
    pub(crate) fn enable_drivewire(&mut self, hdbdos_mode: bool) {
        self.machine.bus.enable_drivewire();
        if let Some(ref mut dw) = self.machine.bus.drivewire {
            dw.set_hdbdos_mode(hdbdos_mode);
            dw.set_clock(host_dw_clock());
        }
    }

    /// Mount the DriveWire image at `path` in `drive`. Like VHD, writes hit the
    /// backing file directly. Failures land in [`Self::cart_error`].
    pub(crate) fn insert_dw_disk(&mut self, drive: usize, path: PathBuf) {
        if let Err(e) = self.mount_dw_disk(drive, path) {
            self.cart_error = Some(e);
        }
    }

    /// Reconcile the running Becker port with `settings` (`None` = disabled).
    /// A path that fails to open keeps that drive's current image.
    pub(crate) fn apply_drivewire_settings(
        &mut self,
        settings: Option<DriveWireLaunch>,
    ) -> Result<(), String> {
        let Some(settings) = settings else {
            self.machine.bus.disable_drivewire();
            self.dw_paths = Default::default();
            return Ok(());
        };
        if self.machine.bus.cart.contains_games_master() {
            return Err(machine_def::DRIVEWIRE_GMC_CONFLICT.to_string());
        }
        self.enable_drivewire(settings.hdbdos_mode);
        let errors: Vec<String> = settings
            .disk_paths
            .into_iter()
            .enumerate()
            .filter_map(|(drive, path)| self.set_dw_disk(drive, path).err())
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("\n"))
        }
    }

    /// Make `drive` serve `path`, leaving an already matching mount untouched.
    fn set_dw_disk(&mut self, drive: usize, path: Option<PathBuf>) -> Result<(), String> {
        if self.dw_paths[drive] == path {
            return Ok(());
        }
        let Some(path) = path else {
            if let Some(ref mut dw) = self.machine.bus.drivewire {
                dw.eject(drive);
            }
            self.dw_paths[drive] = None;
            return Ok(());
        };
        self.mount_dw_disk(drive, path)
    }

    fn mount_dw_disk(&mut self, drive: usize, path: PathBuf) -> Result<(), String> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("could not open {}: {e}", path.display()))?;
        let Some(ref mut dw) = self.machine.bus.drivewire else {
            return Err("Becker port not enabled".to_string());
        };
        dw.mount(drive, DWImage::File(file));
        self.dw_paths[drive] = Some(path);
        Ok(())
    }
}
