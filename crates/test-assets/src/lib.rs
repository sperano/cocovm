//! Dev-only helper for locating git-ignored test assets (ROM images, disk
//! images) shared by the workspace's `tests/` and `#[cfg(test)]` suites.
//!
//! ROM images used to live at a hardcoded path under the repo root
//! (`roms/`), found by tests via `CARGO_MANIFEST_DIR/../../roms`. They've
//! since moved to the app's XDG data dir (`crates/coco-egui/src/paths.rs`'s
//! `data_dir()`, e.g. `~/.local/share/cocovm/roms` on Linux/macOS), which is
//! where a normal install now keeps them. This crate resolves both
//! locations for ROMs so tests keep working either way: a repo-root copy (a
//! local dev convenience, e.g. a symlink) is preferred when present,
//! falling back to the XDG data dir.
//!
//! Disk/VHD images have no such XDG home today: nothing in
//! `crates/coco-egui/src/paths.rs` resolves a `disks` directory (only
//! `roms_dir()` and `images_dir()` exist there), so in practice disks stay
//! repo-root-only. This crate still probes an XDG `disks` candidate
//! alongside the repo-root one regardless, so disk resolution starts
//! working automatically the day an XDG disks dir is introduced, with no
//! change needed here.

use std::path::{Path, PathBuf};

use etcetera::app_strategy::{AppStrategy, AppStrategyArgs};

/// Directory name for ROM images under a candidate root.
const ROMS_KIND: &str = "roms";
/// Directory name for disk/VHD images under a candidate root.
const DISKS_KIND: &str = "disks";

/// The repo root, computed as two `.parent()` calls up from this crate's own
/// `CARGO_MANIFEST_DIR` (`crates/test-assets/../..`).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/test-assets/Cargo.toml has two parent directories")
        .to_path_buf()
}

/// The app's XDG data dir (`~/.local/share/cocovm` on Linux/macOS), or
/// `None` if no home directory can be determined.
///
/// The `AppStrategyArgs` literal below must be kept in sync **by hand**
/// with the identical one in `crates/coco-egui/src/paths.rs`'s
/// `strategy()` — there's no dependency edge from this crate to coco-egui
/// to share it, so this is a manually-maintained duplication.
///
/// Exposed as `pub` (beyond this crate's own use in [`candidates`])
/// specifically so a cross-crate test in `crates/coco-egui/src/paths_test.rs`
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

/// Candidate asset-directory roots for `kind` (`"roms"` or `"disks"`), in
/// preference order: `repo_root/<kind>` first, then `<xdg_data_dir>/<kind>`.
/// Takes `repo_root` as a parameter (rather than calling [`repo_root`]
/// itself) so the join logic can be exercised directly against a fake root
/// in tests. Always yields at least one entry (the repo-root candidate).
fn candidates(repo_root: &Path, kind: &str) -> Vec<PathBuf> {
    let mut out = vec![repo_root.join(kind)];
    if let Some(xdg) = xdg_data_dir() {
        out.push(xdg.join(kind));
    }
    out
}

/// A directory candidate "qualifies" for [`resolve_dir`] if it exists *and*
/// holds at least one entry — an empty (or missing) directory doesn't count,
/// so a stray empty `roms/` at the repo root can't shadow a fully-populated
/// XDG set.
fn is_populated_dir(candidate: &Path) -> bool {
    std::fs::read_dir(candidate)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

/// Resolve the asset directory for `kind`: the first of [`candidates`] that
/// both exists and is non-empty, or the repo-root candidate if none qualify
/// — so a caller's panic/skip message keeps naming a path under the repo
/// root (predictable, greppable) rather than someone's home directory.
///
/// Unlike [`resolve_path`], which falls back per file, this commits to a
/// single directory: a repo-root `roms/` holding only some of the ROMs wins
/// over a complete XDG set, and its missing files are then simply missing.
fn resolve_dir(kind: &str) -> PathBuf {
    let mut rest = candidates(&repo_root(), kind).into_iter();
    let fallback = rest.next().expect("candidates always yields an entry");
    if is_populated_dir(&fallback) {
        return fallback;
    }
    for candidate in rest {
        if is_populated_dir(&candidate) {
            return candidate;
        }
    }
    fallback
}

/// Resolve the path to file `name` under asset directory `kind`: same
/// preference order as [`resolve_dir`], but checking the file itself
/// (`<candidate>/<name>`) rather than the directory, and falling back
/// per-file rather than committing to one directory.
fn resolve_path(kind: &str, name: &str) -> PathBuf {
    let mut rest = candidates(&repo_root(), kind).into_iter();
    let fallback = rest
        .next()
        .expect("candidates always yields an entry")
        .join(name);
    if fallback.exists() {
        return fallback;
    }
    for candidate in rest {
        let joined = candidate.join(name);
        if joined.exists() {
            return joined;
        }
    }
    fallback
}

/// Resolve the path to ROM file `name` (see [`rom`] for well-known names):
/// a repo-root `roms/<name>` if present, else the XDG data dir's
/// `roms/<name>`, else the repo-root path (for a predictable panic/skip
/// message).
pub fn rom(name: &str) -> PathBuf {
    resolve_path(ROMS_KIND, name)
}

/// Resolve the path to disk/VHD image `name` (see [`disk`] for well-known
/// names): same fallback order as [`rom`], under `disks/` instead of
/// `roms/`.
pub fn disk(name: &str) -> PathBuf {
    resolve_path(DISKS_KIND, name)
}

/// Resolve the ROM asset directory: the first of the repo-root `roms/` or
/// the XDG data dir's `roms/` that exists and is non-empty, or the
/// repo-root path if neither qualifies.
pub fn roms_dir() -> PathBuf {
    resolve_dir(ROMS_KIND)
}

/// Resolve the disk/VHD asset directory: same rule as [`roms_dir`], under
/// `disks/` instead of `roms/`.
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
