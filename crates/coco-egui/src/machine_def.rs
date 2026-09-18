//! Versioned machine-definition file format: the persisted "cold" layer for
//! the CocoVM manager. A machine definition is a small, human-editable TOML
//! file describing hardware, attached media, and peripherals, one per file under
//! `config_dir()/machines/<slug>.toml`.
//!
//! [`MachineDef`] is deliberately its own DTO — a *Data Transfer Object*, a
//! struct whose only job is to mirror an external data format field-for-field
//! and be converted to/from the types the program actually runs on. It does
//! not reuse `coco_core::MachineConfig` as the file format. Its own serde
//! derives and readable kebab-case/lowercase strings ("512k", "mc6847t1") keep
//! internal `coco-core` refactors from silently changing what's on disk. The
//! manager gets one `Result<_, String>` surface
//! ([`MachineDef::to_machine_config`]) covering
//! both TOML parse errors and `MachineConfig::validate` failures. Any such
//! failure is fatal at startup ([`load_all`]) — the app refuses to run with
//! a config it can't fully read.
//!
//! `manager.rs` wires this module in for the list rows, "New…" flow, and
//! detail/edit pane.

use std::path::{Path, PathBuf};

use coco_core::{MachineConfig, MachineVariant, MemorySize, VDGVariant, VideoStandard};
use serde::{Deserialize, Serialize};

use crate::display::Display;
use crate::paths;

mod dto;
mod io;
mod peripherals_dto;

pub use dto::{
    DriveWireDTO, HardwareDTO, HiResInterfaceDTO, JoySourceDTO, KbModeDTO, MediaDTO, PortsDTO,
    SerialDTO, StatsDTO, UIDTO,
};
pub use peripherals_dto::{CartridgeDTO, PeripheralsDTO, RS232EndpointDTO, SlotDTO};
// Only tests build definitions with an explicit display DTO so far —
// production writers go through `HardwareDTO::from_config`.
#[cfg(test)]
pub use dto::DisplayDTO;
pub use io::{load_all, save};

/// Schema version this build writes, and the newest it accepts on load.
/// Bump only on a breaking change to the TOML shape.
/// Unknown *keys* stay forward-compatible (warned, not fatal) — only an
/// unknown *schema* number is fatal, since it means the shape itself may have
/// changed underneath us.
pub const CURRENT_SCHEMA: u32 = 1;

/// `chrono` format string for `[created]`—an informational ISO date.
pub const DATE_FORMAT: &str = "%Y-%m-%d";

/// A machine definition, as read from / written to `<slug>.toml`. See the
/// module doc for the schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MachineDef {
    /// Must equal [`CURRENT_SCHEMA`] to load; see that constant's doc.
    pub schema: u32,
    /// Display name — the manager list row's title.
    pub name: String,
    /// Informational only (for example, an ISO date); never interpreted.
    #[serde(default)]
    pub created: Option<String>,
    pub hardware: HardwareDTO,
    #[serde(default)]
    pub media: MediaDTO,
    #[serde(default)]
    pub drivewire: DriveWireDTO,
    #[serde(default)]
    pub peripherals: PeripheralsDTO,
    #[serde(default)]
    pub ports: PortsDTO,
    #[serde(default)]
    pub ui: UIDTO,
    /// `[stats]` section — see [`StatsDTO`]. Read-only from the form's point
    /// of view; only `manager::lifecycle` writes it.
    #[serde(default)]
    pub stats: StatsDTO,
    /// Keys the loader (`io::load_one`) found in the source file but doesn't
    /// know about (top-level, and one level into each known section) — the
    /// same set it warns about using `tracing::warn`. Never serialized itself
    /// (`#[serde(skip)]`); [`save`] merges it back into the freshly-written
    /// TOML so a Save from a build that doesn't yet understand a forward-
    /// compat key (or a user's hand-added key/comment-adjacent entry) can't
    /// silently erase it. Empty for definitions built in-memory
    /// (`from_config`), so equality/dirty-checking between two such drafts
    /// is unaffected.
    #[serde(skip)]
    pub unknown: toml::Table,
}

impl MachineDef {
    /// Convert to the `coco-core` config, running [`MachineConfig::validate`]
    /// so an unsupported hardware combination (for example, CoCo 2 + PAL) fails here
    /// rather than at boot.
    pub fn to_machine_config(&self) -> Result<MachineConfig, String> {
        self.validate_drivewire()?;
        let variant: MachineVariant = self.hardware.variant.into();
        let memory: MemorySize = self.hardware.ram.into();
        let video: VideoStandard = self.hardware.video.into();
        // A monitor choice flows through even on a CoCo 1/2 so `validate` rejects it with the
        // real reason.
        let monitor = self.display().to_monitor(variant);
        let vdg: Option<VDGVariant> = match self.hardware.vdg {
            Some(dto) => Some(dto.into()),
            None => crate::default_vdg(variant),
        };
        let config = MachineConfig {
            variant,
            video,
            memory,
            monitor,
            vdg,
        };
        config.validate()?;
        Ok(config)
    }

    pub(crate) fn validate_drivewire(&self) -> Result<(), String> {
        if self.drivewire.enabled && self.peripherals.cartridge.contains_games_master() {
            return Err(
                "DriveWire Becker port conflicts with the Games Master Cartridge at $FF41"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// The display device this definition asks for: `[hardware].display`,
    /// else the legacy `monitor` key it superseded, else the per-variant default.
    pub fn display(&self) -> Display {
        match (self.hardware.display, self.hardware.monitor) {
            (Some(display), _) => display.into(),
            (None, Some(monitor)) => Display::Monitor(monitor.into()),
            (None, None) => Display::default_for(self.hardware.variant.into()),
        }
    }

    /// Build a fresh definition from a config the manager's "New…" dialog
    /// produced. `media`/`peripherals`/`ui` start at their defaults, and the
    /// display is the config-implied one — both get refined in the detail pane.
    pub fn from_config(name: String, created: Option<String>, config: &MachineConfig) -> Self {
        Self {
            schema: CURRENT_SCHEMA,
            name,
            created,
            hardware: HardwareDTO::from_config(config, Display::from_config(config), None),
            media: MediaDTO::default(),
            drivewire: DriveWireDTO::default(),
            peripherals: PeripheralsDTO::default(),
            ports: PortsDTO::default(),
            ui: UIDTO::default(),
            stats: StatsDTO::default(),
            unknown: toml::Table::new(),
        }
    }
}

/// Lowercase, `[a-z0-9]` kept; runs of other characters collapse to a single
/// `-`, trimmed at the ends; an empty result becomes `"machine"`.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = false;
    for ch in name.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_lowercase() || lower.is_ascii_digit() {
            out.push(lower);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "machine".to_string()
    } else {
        trimmed.to_string()
    }
}

/// `base`, or `base-2`, `base-3`, … — the first candidate for which `taken`
/// returns `false`.
pub fn unique_slug(base: &str, taken: &dyn Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let mut suffix = 2u32;
    loop {
        let candidate = format!("{base}-{suffix}");
        if !taken(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

/// Directory holding every machine definition file (`config_dir()/machines`).
/// `None` when no home directory can be determined; not created here — [`save`] creates it on
/// demand.
pub fn machines_dir() -> Option<PathBuf> {
    paths::config_dir().map(|dir| dir.join("machines"))
}

/// Resolve one `[media]` path: absolute paths pass through; relative paths
/// resolve against this machine's artifact directory, `data_dir()/machines/<slug>`,
/// falling back to the process's current directory if no data directory exists.
pub fn resolve_media_path(raw: &str, slug: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    match artifacts_root() {
        Some(root) => root.join(slug).join(path),
        None => path.to_path_buf(),
    }
}

/// Root of every machine's artifact directory (`data_dir()/machines`);
/// artifacts live under `<root>/<slug>`. Split out so tests can inject a temp dir.
pub fn artifacts_root() -> Option<PathBuf> {
    paths::data_dir().map(|dir| dir.join("machines"))
}

#[cfg(test)]
#[path = "machine_def_test.rs"]
pub(crate) mod tests;
