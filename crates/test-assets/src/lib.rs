//! Development-only helper for locating test assets shared by the workspace's
//! `tests/` and `#[cfg(test)]` suites. Every asset resolves under the same
//! XDG data directory used by the application. Repository-local asset
//! directories are deliberately ignored.

use std::path::PathBuf;

use etcetera::app_strategy::{AppStrategy, AppStrategyArgs};

/// Directory name for ROM images under a candidate root.
const ROMS_KIND: &str = "roms";
/// Directory name for disk/VHD images under a candidate root.
const DISKS_KIND: &str = "disks";

/// The app's XDG data directory (`~/.local/share/cocovm` on Linux/macOS), or
/// `None` if no home directory can be determined. The following `AppStrategyArgs`
/// must remain synchronized with the identical literal in
/// `coco-egui/src/paths.rs`'s `strategy()`. This function is public so
/// `coco-egui/src/paths_test.rs` can assert that.
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

/// Returns the path to ROM file `name` (see [`rom`] for well-known names):
/// `<xdg_data_dir>/roms/<name>`.
pub fn rom(name: &str) -> PathBuf {
    resolve_path(ROMS_KIND, name)
}

/// Returns the path to disk/VHD image `name` under `<xdg_data_dir>/disks`.
pub fn disk(name: &str) -> PathBuf {
    resolve_path(DISKS_KIND, name)
}

/// Returns the ROM asset directory under the XDG data directory.
pub fn roms_dir() -> PathBuf {
    resolve_dir(ROMS_KIND)
}

/// Returns the disk/VHD asset directory under the XDG data directory.
pub fn disks_dir() -> PathBuf {
    resolve_dir(DISKS_KIND)
}

/// Defines well-known ROM file names used across many call sites, avoiding
/// repeated bare string literals.
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
    /// GI SP0256-AL2 allophone mask ROM, the Sound/Speech Cartridge's speech
    /// chip (`coco-core/src/sp0256_test.rs`, `tests/ssc/speech.rs`). MAME's
    /// `sp0256-al2.bin` from the `coco_ssc` set, renamed.
    pub const SP0256_AL2: &str = "sp0256-al2.rom";
    /// The Sound/Speech Cartridge's TMS7040 firmware (`crates/tms7000`'s
    /// firmware tests and trace example). MAME's `pic-7040-510.bin` from
    /// the `coco_ssc` set, renamed.
    pub const SSC_TMS7040: &str = "ssc-tms7040.rom";
}

/// Defines well-known disk/VHD image names used across many call sites,
/// avoiding repeated bare string literals.
pub mod disk {
    /// EOU 1.0.1 boot floppy (OS9Boot carries the EmuDsk driver and `/h0`
    /// descriptors) — `tests/vhd_boot.rs`, `tests/bitbanger_os9.rs`.
    pub const EOU_BOOT: &str = "68EMU.dsk";
    /// EOU 1.0.1's 128 MB `emudsk` system VHD image — `tests/vhd_boot.rs`,
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
