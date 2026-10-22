//! Built-in machine templates: ready-made definitions the toolbar's New menu
//! offers next to the plain default machine, and the starter machines
//! written on a real first run ([`seed_first_run`]).
//!
//! A template is compiled in, not a file: picking one creates an ordinary,
//! independent machine (unique slug, its own artifact dir) with nothing
//! linking it back, so a later release can change a template without
//! touching anyone's existing machines. Templates use only what the app's
//! asset download already fetched: the bundle's DOS ROMs
//! (`machine_def::DosRom`) and the NitrOS-9 EOU images (`nitros9.rs`), which
//! each EOU machine gets its own copy of.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use coco_core::{MachineConfig, MachineVariant, MemorySize};

use super::detail_map::{blank_disk_file, place_blank_file};
use crate::machine_def::{self, CartridgeDTO, DosRom, MachineDef};
use crate::nitros9::{EOU_BOOT_DISK, EOU_IMAGES, EOU_SYSTEM_VHD, eou_version, missing_eou_images};

/// Why the NitrOS-9 EOU template can't be created: its images were never
/// downloaded (their download failed or is still running, or the install
/// predates the template) or are gone. The New menu offers the download
/// instead.
pub(crate) const EOU_NOT_DOWNLOADED: &str = "the NitrOS-9 EOU images are not downloaded";

/// Floppy drive of the blank disk a Disk BASIC template mounts.
const BLANK_DISK_DRIVE: usize = 0;

/// Appended to the machines directory's name (behind a leading dot) to name
/// the sibling directory [`seed_first_run`] fills before moving it into place.
const SEED_STAGING_SUFFIX: &str = ".seeding";

/// Appended the same way to name the lock file that serializes
/// [`seed_first_run`] across app instances launched at the same time.
const SEED_LOCK_SUFFIX: &str = ".seeding.lock";

/// A built-in machine template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Preset {
    /// CoCo 3, 512K, RGB; FD-502 with Disk BASIC 1.1 and a blank disk in drive 0.
    CoCo3DiskBasic,
    /// CoCo 3, 512K, RGB; FD-502 with HDB-DOS, booting from the built-in
    /// DriveWire server over the Becker port.
    CoCo3DriveWire,
    /// CoCo 2, 64K, composite, MC6847T1; FD-502 with Disk BASIC 1.1 and a
    /// blank disk in drive 0.
    CoCo2DiskBasic,
    /// CoCo 3, 512K, RGB; FD-502 with Disk BASIC 1.1, NitrOS-9 EOU 1.0.1's
    /// boot floppy in drive 0 and its system disk on the VHD interface:
    /// the setup `coco-core`'s `vhd_boot` test boots to the shell.
    NitrOS9Eou,
}

impl Preset {
    /// Every template, in the order the New menu lists them.
    pub(crate) const ALL: [Self; 4] = [
        Self::CoCo3DiskBasic,
        Self::CoCo3DriveWire,
        Self::CoCo2DiskBasic,
        Self::NitrOS9Eou,
    ];

    /// The templates [`seed_first_run`] writes. DriveWire is left out: its
    /// drives start empty, so it has nothing to show until the user adds
    /// disk images. So is NitrOS-9 EOU: seeding runs before the asset
    /// dialog has downloaded its images.
    pub(crate) const FIRST_RUN: [Self; 2] = [Self::CoCo3DiskBasic, Self::CoCo2DiskBasic];

    /// Menu label, and the new machine's display name (and slug source).
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::CoCo3DiskBasic => "CoCo 3 Disk BASIC",
            Self::CoCo3DriveWire => "CoCo 3 DriveWire (HDB-DOS)",
            Self::CoCo2DiskBasic => "CoCo 2 Disk BASIC",
            Self::NitrOS9Eou => "NitrOS-9 EOU",
        }
    }

    /// Hover text of the template's menu item.
    pub(crate) const fn description(self) -> &'static str {
        match self {
            Self::CoCo3DiskBasic => {
                "CoCo 3 with 512K and an RGB monitor, an FD-502 disk controller running Disk \
                BASIC 1.1, and a blank disk in drive 0"
            }
            Self::CoCo3DriveWire => {
                "CoCo 3 with 512K and an RGB monitor, an FD-502 running HDB-DOS, and \
                DriveWire enabled in HDB-DOS mode. Add disk images on the DriveWire tab"
            }
            Self::CoCo2DiskBasic => {
                "CoCo 2 with 64K and the MC6847T1 VDG, an FD-502 disk controller running \
                Disk BASIC 1.1, and a blank disk in drive 0"
            }
            Self::NitrOS9Eou => concat!(
                "CoCo 3 with 512K and an RGB monitor running NitrOS-9 Ease of Use ",
                eou_version!(),
                ": its boot disk in drive 0 and its system hard disk on the VHD \
                interface. Type DOS to boot"
            ),
        }
    }

    /// Whether the template mounts a blank floppy in [`BLANK_DISK_DRIVE`],
    /// which [`install`] must create in the machine's artifact dir.
    const fn has_blank_disk(self) -> bool {
        match self {
            Self::CoCo3DiskBasic | Self::CoCo2DiskBasic => true,
            Self::CoCo3DriveWire | Self::NitrOS9Eou => false,
        }
    }

    /// Whether the template can be created with the NitrOS-9 EOU images in
    /// `eou_dir`: only [`Self::NitrOS9Eou`] needs them.
    pub(crate) fn available(self, eou_dir: Option<&Path>) -> bool {
        match self {
            Self::NitrOS9Eou => eou_dir.is_some_and(|dir| missing_eou_images(dir).is_empty()),
            Self::CoCo3DiskBasic | Self::CoCo3DriveWire | Self::CoCo2DiskBasic => true,
        }
    }

    /// The template's definition, named [`Self::name`] and dated `created`.
    pub(crate) fn def(self, created: Option<String>) -> MachineDef {
        let config = match self {
            Self::CoCo3DiskBasic | Self::CoCo3DriveWire | Self::NitrOS9Eou => {
                MachineConfig::default()
            }
            Self::CoCo2DiskBasic => coco2_config(),
        };
        let mut def = MachineDef::from_config(self.name().to_string(), created, &config);
        let dos_rom = match self {
            Self::CoCo3DiskBasic | Self::CoCo2DiskBasic | Self::NitrOS9Eou => DosRom::DiskBasic,
            Self::CoCo3DriveWire => DosRom::HdbDosDw3,
        };
        def.peripherals.cartridge = CartridgeDTO::FD502 { dos_rom };
        if self.has_blank_disk() {
            def.media.disk0 = Some(blank_disk_file(BLANK_DISK_DRIVE));
        }
        if self == Self::NitrOS9Eou {
            // Relative, so they resolve in the machine's artifact dir, where
            // [`install`] copies them.
            def.media.disk0 = Some(EOU_BOOT_DISK.to_string());
            def.media.vhd0 = Some(EOU_SYSTEM_VHD.to_string());
        }
        if def.peripherals.cartridge.uses_hdbdos() {
            // What the form switches on when HDB-DOS is picked
            // (`MachineForm::default_drivewire_for_hdbdos`).
            def.drivewire.enabled = true;
            def.drivewire.hdbdos_mode = true;
        }
        def
    }
}

/// A stock 64K CoCo 2: NTSC (the default config's), the MC6847T1 VDG it shipped with, and the
/// composite display its variant defaults to.
fn coco2_config() -> MachineConfig {
    let variant = MachineVariant::Coco2;
    MachineConfig {
        variant,
        memory: MemorySize::K64,
        monitor: None,
        vdg: crate::default_vdg(variant),
        ..MachineConfig::default()
    }
}

/// Write `preset` as machine `slug`: its media into the artifact dir under
/// `artifacts_root` first (blank images, or copies of the NitrOS-9 EOU
/// images in `eou_dir`), then its definition into `machines_dir`, so a
/// failure never leaves a definition pointing at a missing image. Returns
/// the saved definition.
pub(super) fn install(
    preset: Preset,
    dirs: &InstallDirs<'_>,
    slug: &str,
    created: Option<String>,
) -> Result<MachineDef, String> {
    let def = preset.def(created);
    place_media(preset, dirs.artifacts_root, slug)?;
    let eou_copies = (preset == Preset::NitrOS9Eou)
        .then(|| place_eou_images(dirs, slug))
        .transpose()?;
    machine_def::save(dirs.machines_dir, slug, &def).inspect_err(|_| {
        // A 128 MB copy no definition points at is worth reclaiming.
        if let Some(artifact_dir) = &eou_copies {
            discard_eou_images(artifact_dir);
        }
    })?;
    Ok(def)
}

/// Where [`install`] writes a machine and finds what it copies.
pub(super) struct InstallDirs<'a> {
    /// `config_dir()/machines`, for the definition.
    pub(super) machines_dir: &'a Path,
    /// `data_dir()/machines`, holding each machine's artifact dir.
    pub(super) artifacts_root: Option<&'a Path>,
    /// `paths::eou_dir`, the downloaded NitrOS-9 EOU images.
    pub(super) eou_dir: Option<&'a Path>,
}

/// Create `preset`'s blank media in machine `slug`'s artifact dir under
/// `artifacts_root`.
fn place_media(preset: Preset, artifacts_root: Option<&Path>, slug: &str) -> Result<(), String> {
    if preset.has_blank_disk() {
        let root = artifacts_root.ok_or(super::NO_DATA_DIR)?;
        place_blank_file(root, slug, &blank_disk_file(BLANK_DISK_DRIVE))?;
    }
    Ok(())
}

/// Copy the downloaded [`EOU_IMAGES`] into machine `slug`'s artifact dir,
/// which it returns. A failure removes the copies it made, and the dir while
/// it is empty, such as the one the New menu created to claim `slug`
/// (`manager/eou_install.rs`).
fn place_eou_images(dirs: &InstallDirs<'_>, slug: &str) -> Result<PathBuf, String> {
    let root = dirs.artifacts_root.ok_or(super::NO_DATA_DIR)?;
    let artifact_dir = root.join(slug);
    let copied = dirs
        .eou_dir
        .ok_or_else(|| EOU_NOT_DOWNLOADED.to_string())
        .and_then(|eou_dir| copy_eou_images(eou_dir, &artifact_dir));
    if copied.is_err() {
        discard_eou_images(&artifact_dir);
    }
    copied.map(|()| artifact_dir)
}

/// Copy the [`EOU_IMAGES`] from `eou_dir` into `artifact_dir`, so the new
/// machine writes to its own system disk and never to the downloaded one.
///
/// The 128 MB VHD can take seconds on a hard disk, so the New menu calls
/// this through [`install`] on a thread of its own (`manager/eou_install.rs`).
fn copy_eou_images(eou_dir: &Path, artifact_dir: &Path) -> Result<(), String> {
    if !missing_eou_images(eou_dir).is_empty() {
        return Err(EOU_NOT_DOWNLOADED.to_string());
    }
    fs::create_dir_all(artifact_dir).map_err(|e| format!("{}: {e}", artifact_dir.display()))?;
    EOU_IMAGES.into_iter().try_for_each(|name| {
        let target = artifact_dir.join(name);
        fs::copy(eou_dir.join(name), &target)
            .map(drop)
            .map_err(|e| format!("{}: {e}", target.display()))
    })
}

/// Remove the [`EOU_IMAGES`] from `artifact_dir`, then the dir itself while
/// it is empty. Best effort: what stays only makes the slug count as taken.
pub(super) fn discard_eou_images(artifact_dir: &Path) {
    for name in EOU_IMAGES {
        let _ = fs::remove_file(artifact_dir.join(name));
    }
    let _ = fs::remove_dir(artifact_dir);
}

/// On a real first run — `machines_dir` does not exist yet — write the
/// [`Preset::FIRST_RUN`] templates so the list starts with machines that
/// run. An existing directory, even an empty one, is left alone, so
/// machines the user deleted stay deleted. A slug whose artifact dir
/// survived (data kept, config removed) gets a suffix, so no starter
/// machine starts on an old disk.
///
/// The machines are written into a staging directory that is renamed to
/// `machines_dir` only once all of them are in it, so a failure (or a crash)
/// partway never leaves a `machines_dir` that would stop the next launch
/// from seeding the rest. A failure discards the staging directory; one
/// left by a crash is discarded before seeding starts. Seeding holds a lock
/// file ([`lock_seeding`]), so a second instance launched at the same time
/// waits instead of discarding the first one's live staging directory, then
/// finds `machines_dir` in place and leaves it alone.
pub(super) fn seed_first_run(
    machines_dir: &Path,
    artifacts_root: Option<&Path>,
    created: Option<String>,
) -> Result<(), String> {
    if machines_dir_present(machines_dir) {
        return Ok(());
    }
    seed_under_lock(machines_dir, artifacts_root, created)
}

/// [`seed_first_run`] past its first check: take the seeding lock, check
/// again for `machines_dir` (another instance may have seeded while this one
/// waited), then stage the machines and move them into place.
fn seed_under_lock(
    machines_dir: &Path,
    artifacts_root: Option<&Path>,
    created: Option<String>,
) -> Result<(), String> {
    let _lock = lock_seeding(machines_dir)?;
    if machines_dir_present(machines_dir) {
        return Ok(());
    }
    let staging = staging_dir(machines_dir)?;
    discard_staging(&staging, artifacts_root)?;
    let seeded = stage(&staging, artifacts_root, created).and_then(|()| {
        fs::rename(&staging, machines_dir).map_err(|e| format!("{}: {e}", machines_dir.display()))
    });
    if seeded.is_err() {
        // Best effort: whatever survives is discarded on the next launch.
        let _ = discard_staging(&staging, artifacts_root);
    }
    seeded
}

/// Whether `machines_dir` exists. An unreadable path counts as present:
/// never seed over what may exist.
fn machines_dir_present(machines_dir: &Path) -> bool {
    machines_dir.try_exists().unwrap_or(true)
}

/// Open the `.<name>.seeding.lock` file next to `machines_dir` and wait for
/// an exclusive lock on it; dropping the file releases the lock, and so does
/// a crash. The file is never removed: an instance still waiting on a
/// removed file would lock it while a third instance locks a new one.
///
/// When the lock can't be taken, seeding warns and goes ahead unlocked:
/// std reports some platforms' locks as unsupported, and NFS without a lock
/// manager fails with a plain OS error (ENOLCK). Refusing there would leave
/// the user without starter machines on every launch, to guard against a
/// race that needs two instances started at the same moment.
fn lock_seeding(machines_dir: &Path) -> Result<File, String> {
    let path = sibling_path(machines_dir, SEED_LOCK_SUFFIX)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if let Err(e) = file.lock() {
        eprintln!(
            "coco: {}: locking: {e}; seeding the starter machines unlocked",
            path.display()
        );
    }
    Ok(file)
}

/// The staging directory [`seed_first_run`] fills: `.<name>.seeding` next to
/// `machines_dir`, so the final rename stays on one file system.
fn staging_dir(machines_dir: &Path) -> Result<PathBuf, String> {
    sibling_path(machines_dir, SEED_STAGING_SUFFIX)
}

/// `.<name><suffix>` next to `machines_dir`.
fn sibling_path(machines_dir: &Path, suffix: &str) -> Result<PathBuf, String> {
    let name = machines_dir
        .file_name()
        .ok_or_else(|| format!("{}: no directory name", machines_dir.display()))?;
    let mut sibling = OsString::from(".");
    sibling.push(name);
    sibling.push(suffix);
    Ok(machines_dir.with_file_name(sibling))
}

/// Write the [`Preset::FIRST_RUN`] machines into `staging`. Each definition
/// is saved before its blank media is placed, so [`discard_staging`] finds
/// every artifact dir this created through the definitions left behind.
fn stage(
    staging: &Path,
    artifacts_root: Option<&Path>,
    created: Option<String>,
) -> Result<(), String> {
    let taken =
        |candidate: &str| machine_def::slug_on_disk(Some(staging), artifacts_root, candidate);
    for preset in Preset::FIRST_RUN {
        let slug = machine_def::unique_slug(&machine_def::slugify(preset.name()), &taken);
        machine_def::save(staging, &slug, &preset.def(created.clone()))?;
        place_media(preset, artifacts_root, &slug)?;
    }
    Ok(())
}

/// Remove `staging` and, for each definition in it, the blank media
/// [`stage`] placed. A missing `staging` is fine.
fn discard_staging(staging: &Path, artifacts_root: Option<&Path>) -> Result<(), String> {
    let entries = match fs::read_dir(staging) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", staging.display())),
    };
    if let Some(root) = artifacts_root {
        for path in entries.flatten().map(|entry| entry.path()) {
            if path.extension().and_then(|ext| ext.to_str()) == Some(machine_def::DEF_EXTENSION)
                && let Some(slug) = path.file_stem().and_then(|stem| stem.to_str())
            {
                discard_blank_media(&root.join(slug));
            }
        }
    }
    fs::remove_dir_all(staging).map_err(|e| format!("{}: {e}", staging.display()))
}

/// Remove the blank disk in `artifact_dir` while it is still empty, then the
/// dir itself while it is empty, so nothing the user wrote is ever deleted.
/// Best effort: what stays only makes the slug count as taken.
fn discard_blank_media(artifact_dir: &Path) {
    let disk = artifact_dir.join(blank_disk_file(BLANK_DISK_DRIVE));
    if fs::metadata(&disk).is_ok_and(|meta| meta.is_file() && meta.len() == 0) {
        let _ = fs::remove_file(&disk);
    }
    let _ = fs::remove_dir(artifact_dir);
}

#[cfg(test)]
#[path = "presets_test.rs"]
mod tests;
