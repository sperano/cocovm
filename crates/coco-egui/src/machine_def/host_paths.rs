//! [`MachineDef::host_paths`]: the host files and folders a definition
//! names, for checks that span machines (a delete must not take another
//! machine's files with it).

use std::path::Path;

use super::MachineDef;

impl MachineDef {
    /// Every host file or folder this definition names that resolves like a
    /// `[media]` path ([`super::resolve_media_path`]), as written: `[media]`, the
    /// DriveWire disks, shares and print folder, and cartridge ROM images.
    /// `[hardware].rom` counts only when absolute, since a relative one names
    /// a file in the working directory, not in the artifact dir.
    pub fn host_paths(&self) -> Vec<&str> {
        let media = &self.media;
        let mut paths: Vec<&str> = [
            &media.disk0,
            &media.disk1,
            &media.vhd0,
            &media.vhd1,
            &media.tape,
        ]
        .into_iter()
        .filter_map(Option::as_deref)
        .collect();
        let drivewire = &self.drivewire;
        paths.extend(drivewire.disk_paths().into_iter().flatten());
        paths.extend(drivewire.shares.iter().map(|share| share.path.as_str()));
        paths.push(&drivewire.printer.folder);
        paths.extend(self.peripherals.cartridge.rom_paths());
        paths.extend(
            self.hardware
                .rom
                .as_deref()
                .filter(|rom| Path::new(rom).is_absolute()),
        );
        paths.retain(|path| !path.trim().is_empty());
        paths
    }
}

#[cfg(test)]
#[path = "host_paths_test.rs"]
mod tests;
