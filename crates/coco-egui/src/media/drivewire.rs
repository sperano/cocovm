//! The DriveWire virtual serial link and the disk images it serves.

use coco_core::drivewire::MediaOrigin;
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
    /// Records `path` as the drive's applied startup path (see [`Self::dw_startup`]).
    pub(crate) fn insert_dw_disk(&mut self, drive: usize, path: PathBuf) {
        match self.mount_dw_disk(drive, path.clone()) {
            Ok(()) => self.dw_startup[drive] = Some(path),
            Err(e) => self.cart_error = Some(e),
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
            self.dw_startup = Default::default();
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

    /// Make `drive` serve the startup `path` when it differs from the one last
    /// applied, so an unchanged setting leaves a guest's `dw disk` mount alone.
    fn set_dw_disk(&mut self, drive: usize, path: Option<PathBuf>) -> Result<(), String> {
        if self.dw_startup[drive] == path {
            return Ok(());
        }
        let result = match path.clone() {
            Some(path) => self.mount_dw_disk(drive, path),
            None => {
                if let Some(ref mut dw) = self.machine.bus.drivewire {
                    dw.eject(drive);
                }
                self.dw_paths[drive] = None;
                self.dw_leases[drive] = None;
                Ok(())
            }
        };
        if result.is_ok() {
            self.dw_startup[drive] = path;
        }
        result
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
        if let Some(name) = path.file_name() {
            dw.set_drive_name(drive, name.to_string_lossy());
        }
        self.dw_paths[drive] = Some(path);
        self.dw_leases[drive] = Some(lease);
        Ok(())
    }

    /// A restored host mount becomes the drive's applied startup path, so the
    /// next settings change brings a differing drive back in line with the
    /// definition. A drive the guest changed keeps the startup path applied
    /// before the restore: only a changed setting may replace its media.
    fn restore_dw_startup(&mut self) {
        let Some(dw) = self.machine.bus.drivewire.as_ref() else {
            self.dw_startup = Default::default();
            return;
        };
        for drive in 0..drivewire::DRIVE_COUNT {
            if !dw.guest_changed(drive) {
                self.dw_startup[drive] = self.dw_paths[drive].clone();
            }
        }
    }

    /// Follow the guest's `dw disk insert`/`eject` in the session media. The
    /// server holds a guest image's lease; the replaced image's lease ends here.
    /// Settings and `dw_startup` stay as they are.
    pub(crate) fn sync_guest_dw_media(&mut self) {
        let Some(dw) = self.machine.bus.drivewire.as_mut() else {
            return;
        };
        for change in dw.take_guest_media_changes() {
            self.dw_paths[change.drive] = change.host_path;
            self.dw_leases[change.drive] = None;
        }
    }

    /// Lease the images a snapshot restore mounted. An image another running
    /// VM holds is ejected and reported in `notes`, like a missing one.
    pub(crate) fn lease_restored_dw_images(&mut self, notes: &mut Vec<String>) {
        self.restore_dw_startup();
        let registry = AccessRegistry::global();
        for drive in 0..drivewire::DRIVE_COUNT {
            let Some(path) = self.dw_paths[drive].clone() else {
                self.dw_leases[drive] = None;
                continue;
            };
            let mode = restored_lease_mode(self.machine.bus.drivewire.as_ref(), drive);
            let lease = std::fs::File::open(&path)
                .map_err(ShareError::from)
                .and_then(|file| registry.acquire(&file, &path, self.lease_owner, mode));
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

/// A hover line for a drive the guest mounted with `dw disk insert`, or "".
pub(crate) fn guest_media_note(dw: &drivewire::DWServer, drive: usize) -> &'static str {
    match dw.drive_media(drive) {
        Some(media) if media.origin != MediaOrigin::Guest => "",
        Some(media) if media.write_protected => "\nInserted by the guest, read-only",
        Some(_) => "\nInserted by the guest",
        None => "",
    }
}

/// A write-protected guest mount (from a read-only share) needs only a read lease.
fn restored_lease_mode(dw: Option<&drivewire::DWServer>, drive: usize) -> AccessMode {
    let protected = dw
        .and_then(|dw| dw.drive_media(drive))
        .is_some_and(|media| media.write_protected);
    if protected {
        AccessMode::Read
    } else {
        AccessMode::Write
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
pub(crate) mod tests;
