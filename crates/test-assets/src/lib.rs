//! Dev-only helper for locating test assets shared by the workspace's
//! `tests/` and `#[cfg(test)]` suites. Every asset resolves under the same
//! XDG data directory used by the application; repository-local asset
//! directories are deliberately ignored.

use std::path::PathBuf;

use etcetera::app_strategy::{AppStrategy, AppStrategyArgs};

/// Directory name for ROM images under a candidate root.
const ROMS_KIND: &str = "roms";
/// Directory name for disk/VHD images under a candidate root.
const DISKS_KIND: &str = "disks";

/// The app's XDG data dir (`~/.local/share/cocovm` on Linux/macOS), or
/// `None` if no home directory can be determined.
///
/// The `AppStrategyArgs` literal below must be kept in sync **by hand**
/// with the identical one in `crates/coco-egui/src/paths.rs`'s
/// `strategy()` — there's no dependency edge from this crate to coco-egui
/// to share it, so this is a manually-maintained duplication.
///
/// Exposed as `pub` so a cross-crate test in `crates/coco-egui/src/paths_test.rs`
/// can assert the two literals stay in sync, turning the "by hand" note
/// above into an enforced invariant instead of just a warning.
pub fn xdg_data_dir() -> Option<PathBuf> {
    etcetera::choose_app_strategy(AppStrategyArgs {
        top_level_domain: "quebec".to_string(),
        author: "spe".to_string(),
        app_name: "cocovm".to_string(),
    })
    .ok()
    .map(|s| s.data_dir())
}

fn resolve_dir(kind: &str) -> PathBuf {
    xdg_data_dir()
        .expect("cannot determine the cocovm XDG data directory")
        .join(kind)
}

fn resolve_path(kind: &str, name: &str) -> PathBuf {
    resolve_dir(kind).join(name)
}

/// Resolve the path to ROM file `name` (see [`rom`] for well-known names):
/// `<xdg_data_dir>/roms/<name>`.
pub fn rom(name: &str) -> PathBuf {
    resolve_path(ROMS_KIND, name)
}

/// Resolve the path to disk/VHD image `name` under `<xdg_data_dir>/disks`.
pub fn disk(name: &str) -> PathBuf {
    resolve_path(DISKS_KIND, name)
}

/// Resolve the ROM asset directory under the XDG data directory.
pub fn roms_dir() -> PathBuf {
    resolve_dir(ROMS_KIND)
}

/// Resolve the disk/VHD asset directory under the XDG data directory.
pub fn disks_dir() -> PathBuf {
    resolve_dir(DISKS_KIND)
}

/// Well-known ROM file names repeated verbatim across many call sites, so
/// they don't appear as bare string literals over and over.
pub mod rom {
    /// Super Extended Color BASIC — the CoCo 3 system ROM most integration
    /// tests boot.
    pub const COCO3: &str = "coco3.rom";
    /// Disk Extended Color BASIC.
    pub const DISK11: &str = "disk11.rom";
    /// HDB-DOS 1.1 DriveWire 3, Becker build for CoCo 3 (`tests/drivewire_boot.rs`).
    pub const HDBDW3BC3: &str = "hdbdw3bc3.rom";
    /// Color BASIC 1.2 (CoCo 1) / the CoCo 2's Color BASIC half of its flat
    /// image (`tests/coco1_boot.rs`, `tests/coco2_boot/common.rs`).
    pub const BAS12: &str = "bas12.rom";
    /// Extended Color BASIC 1.1, the CoCo 2's extbas half
    /// (`tests/coco2_boot/common.rs`).
    pub const EXTBAS11: &str = "extbas11.rom";
}

/// Well-known disk/VHD image names repeated verbatim across many call
/// sites, so they don't appear as bare string literals over and over.
pub mod disk {
    /// EOU 1.0.1 boot floppy (OS9Boot carries the EmuDsk driver and `/h0`
    /// descriptors) — `tests/vhd_boot.rs`, `tests/bitbanger_os9.rs`.
    pub const EOU_BOOT: &str = "68EMU.dsk";
    /// EOU 1.0.1's 128MB `emudsk` system VHD image — `tests/vhd_boot.rs`,
    /// `tests/bitbanger_os9.rs`.
    pub const EOU_SYSTEM_VHD: &str = "68SDC.VHD";
    /// Flat 35-track/18-sector DECB image carrying an auto-running Tetris
    /// clone, used by `tests/drivewire_boot.rs`'s `DIR` regression.
    pub const SPETRIS: &str = "spetris.dsk";
    /// Blank flat 35-track/18-sector DECB image, used by
    /// `tests/drivewire_boot.rs`'s `SAVE` regression.
    pub const BLANK02: &str = "blank02.dsk";
    /// Real NitrOS-9 Level 2 CoCo3 40-track disk image —
    /// `tests/fdc/image_geometry.rs`.
    pub const NOS9_L2_COCO3_40_TRACK: &str = "NOS9_6809_L2_v030300_coco3_40d_1.dsk";
    /// NitrOS-9 Level 2 CoCo3 image with the Becker-port driver preinstalled
    /// — `tests/drivewire_boot.rs`'s boot-to-shell regression.
    pub const NOS9_L2_COCO3_BECKER: &str = "nos96809l2v030300coco3_becker.dsk";
}

#[cfg(test)]
#[path = "lib_test.rs"]
mod tests;
