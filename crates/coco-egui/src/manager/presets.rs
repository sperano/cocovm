//! Built-in machine templates: ready-made definitions the toolbar's New menu
//! offers next to the plain default machine, and the starter machines
//! written on a real first run ([`seed_first_run`]).
//!
//! A template is compiled in, not a file: picking one creates an ordinary,
//! independent machine (unique slug, its own artifact dir) with nothing
//! linking it back, so a later release can change a template without
//! touching anyone's existing machines. Templates use only what the app's
//! asset bundle already ships (the DOS ROMs `machine_def::DosRom` names).

use std::path::Path;

use coco_core::{MachineConfig, MachineVariant, MemorySize};

use super::detail_map::{blank_disk_file, place_blank_file};
use crate::machine_def::{self, CartridgeDTO, DosRom, MachineDef};

/// Floppy drive of the blank disk a Disk BASIC template mounts.
const BLANK_DISK_DRIVE: usize = 0;

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
    if preset.has_blank_disk() {
        let root = artifacts_root.ok_or(super::NO_DATA_DIR)?;
        place_blank_file(root, slug, &blank_disk_file(BLANK_DISK_DRIVE))?;
    }
    machine_def::save(machines_dir, slug, &def)?;
    Ok(def)
}

/// On a real first run — `machines_dir` does not exist yet — write the
/// [`Preset::FIRST_RUN`] templates so the list starts with machines that
/// run. An existing directory, even an empty one, is left alone, so
/// machines the user deleted stay deleted.
pub(super) fn seed_first_run(
    machines_dir: &Path,
    artifacts_root: Option<&Path>,
    created: Option<String>,
) -> Result<(), String> {
    // An unreadable path counts as present: never seed over what may exist.
    if machines_dir.try_exists().unwrap_or(true) {
        return Ok(());
    }
    for preset in Preset::FIRST_RUN {
        let slug = machine_def::slugify(preset.name());
        install(preset, machines_dir, artifacts_root, &slug, created.clone())?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "presets_test.rs"]
mod tests;
