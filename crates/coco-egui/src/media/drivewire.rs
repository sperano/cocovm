//! The DriveWire virtual serial link and the disk images it serves.

use coco_core::drivewire::share::{AccessMode, AccessRegistry, ShareError, ShareTable};

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
            dw.set_shares(self.dw_shares.clone(), self.lease_owner);
        }
    }

    /// Record `table` as this VM's host shares and install it in a running
    /// server. An unchanged table keeps the guest's share session.
    pub(crate) fn set_drivewire_shares(&mut self, table: ShareTable) {
        self.dw_shares = table;
        self.install_drivewire_shares();
    }

    /// Install [`Self::dw_shares`], for example into a server a snapshot restored.
    pub(crate) fn install_drivewire_shares(&mut self) {
        if let Some(ref mut dw) = self.machine.bus.drivewire {
            dw.set_shares(self.dw_shares.clone(), self.lease_owner);
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
            self.dw_leases = Default::default();
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
            self.dw_leases[drive] = None;
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
        let lease = AccessRegistry::global()
            .acquire(&file, &path, self.lease_owner, AccessMode::Write)
            .map_err(|e| image_lease_error(&path, e))?;
        dw.mount(drive, DWImage::File(file));
        self.dw_paths[drive] = Some(path);
        self.dw_leases[drive] = Some(lease);
        Ok(())
    }

    /// Lease the images a snapshot restore mounted. An image another running
    /// VM holds is ejected and reported in `notes`, like a missing one.
    pub(crate) fn lease_restored_dw_images(&mut self, notes: &mut Vec<String>) {
        let registry = AccessRegistry::global();
        for drive in 0..drivewire::DRIVE_COUNT {
            let Some(path) = self.dw_paths[drive].clone() else {
                self.dw_leases[drive] = None;
                continue;
            };
            let lease = std::fs::File::open(&path)
                .map_err(ShareError::from)
                .and_then(|file| {
                    registry.acquire(&file, &path, self.lease_owner, AccessMode::Write)
                });
            match lease {
                Ok(lease) => self.dw_leases[drive] = Some(lease),
                Err(error) => {
                    if let Some(ref mut dw) = self.machine.bus.drivewire {
                        dw.eject(drive);
                    }
                    self.dw_paths[drive] = None;
                    self.dw_leases[drive] = None;
                    notes.push(format!("{} not mounted", image_lease_error(&path, error)));
                }
            }
        }
    }
}

fn image_lease_error(path: &Path, error: ShareError) -> String {
    match error {
        ShareError::Busy => format!(
            "DriveWire image {} is in use by another running VM",
            path.display()
        ),
        error => format!(
            "could not lease DriveWire image {}: {error}",
            path.display()
        ),
    }
}

#[cfg(test)]
#[path = "drivewire_test.rs"]
mod tests;
