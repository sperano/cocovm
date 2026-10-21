//! Built-in machine templates: ready-made definitions the toolbar's New menu
//! offers next to the plain default machine, and the starter machines
//! written on a real first run ([`seed_first_run`]).
//!
//! A template is compiled in, not a file: picking one creates an ordinary,
//! independent machine (unique slug, its own artifact dir) with nothing
//! linking it back, so a later release can change a template without
//! touching anyone's existing machines. Templates use only what the app's
//! asset bundle already ships (the DOS ROMs `machine_def::DosRom` names).

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use coco_core::{MachineConfig, MachineVariant, MemorySize};

use super::detail_map::{blank_disk_file, place_blank_file};
use crate::machine_def::{self, CartridgeDTO, DosRom, MachineDef};

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
}

impl Preset {
    /// Every template, in the order the New menu lists them.
    pub(crate) const ALL: [Self; 3] = [
        Self::CoCo3DiskBasic,
        Self::CoCo3DriveWire,
        Self::CoCo2DiskBasic,
    ];

    /// The templates [`seed_first_run`] writes. DriveWire is left out: its
    /// drives start empty, so it has nothing to show until the user adds
    /// disk images.
    pub(crate) const FIRST_RUN: [Self; 2] = [Self::CoCo3DiskBasic, Self::CoCo2DiskBasic];

    /// Menu label, and the new machine's display name (and slug source).
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::CoCo3DiskBasic => "CoCo 3 Disk BASIC",
            Self::CoCo3DriveWire => "CoCo 3 DriveWire (HDB-DOS)",
            Self::CoCo2DiskBasic => "CoCo 2 Disk BASIC",
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
        }
    }

    /// Whether the template mounts a blank floppy in [`BLANK_DISK_DRIVE`],
    /// which [`install`] must create in the machine's artifact dir.
    const fn has_blank_disk(self) -> bool {
        match self {
            Self::CoCo3DiskBasic | Self::CoCo2DiskBasic => true,
            Self::CoCo3DriveWire => false,
        }
    }

    /// The template's definition, named [`Self::name`] and dated `created`.
    pub(crate) fn def(self, created: Option<String>) -> MachineDef {
        let config = match self {
            Self::CoCo3DiskBasic | Self::CoCo3DriveWire => MachineConfig::default(),
            Self::CoCo2DiskBasic => coco2_config(),
        };
        let mut def = MachineDef::from_config(self.name().to_string(), created, &config);
        let dos_rom = match self {
            Self::CoCo3DiskBasic | Self::CoCo2DiskBasic => DosRom::DiskBasic,
            Self::CoCo3DriveWire => DosRom::HdbDosDw3,
        };
        def.peripherals.cartridge = CartridgeDTO::FD502 { dos_rom };
        if self.has_blank_disk() {
            def.media.disk0 = Some(blank_disk_file(BLANK_DISK_DRIVE));
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

/// Write `preset` as machine `slug`: its blank media into the artifact dir
/// under `artifacts_root` first, then its definition into `machines_dir`, so
/// a failure never leaves a definition pointing at a missing image. Returns
/// the saved definition.
pub(super) fn install(
    preset: Preset,
    machines_dir: &Path,
    artifacts_root: Option<&Path>,
    slug: &str,
    created: Option<String>,
) -> Result<MachineDef, String> {
    let def = preset.def(created);
    place_media(preset, artifacts_root, slug)?;
    machine_def::save(machines_dir, slug, &def)?;
    Ok(def)
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
    let _lock = lock_seeding(machines_dir)?;
    // Checked again under the lock: another instance may have seeded while
    // this one waited.
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
fn lock_seeding(machines_dir: &Path) -> Result<File, String> {
    let path = sibling_path(machines_dir, SEED_LOCK_SUFFIX)?;
    let error = |e: std::io::Error| format!("{}: {e}", path.display());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(error)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(error)?;
    file.lock().map_err(error)?;
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
