//! Versioned machine-definition file format: the persisted "cold" layer for
//! the CocoVM manager (`docs/plan-machine-persistence.md`). A machine
//! definition is a small, human-editable TOML file describing hardware +
//! attached media + peripherals, one per file under
//! `config_dir()/machines/<slug>.toml`.
//!
//! [`MachineDef`] is deliberately its own DTO — a *Data Transfer Object*, a
//! struct whose only job is to mirror an external data format field-for-field
//! and be converted to/from the types the program actually runs on — rather
//! than `coco_core::MachineConfig` reused directly as the file format
//! (`plan-machine-persistence.md` "Decisions"): a struct with its own serde
//! derives and readable
//! kebab/lowercase strings ("512k", "mc6847t1") means internal `coco-core`
//! refactors never silently change what's on disk, and the manager gets one
//! `Result<_, String>` surface ([`MachineDef::to_machine_config`]) covering
//! both TOML parse errors and `MachineConfig::validate` failures. Any such
//! failure is fatal at startup ([`load_all`]) — the app refuses to run with
//! a config it can't fully read.
//!
//! `manager.rs` wires this module in for the list rows, "New…" flow, and
//! detail/edit pane (`plan-machine-persistence.md` steps 2-4).

use std::path::{Path, PathBuf};

use coco_core::{
    MachineConfig, MachineVariant, MemorySize, MonitorType, VDGVariant, VideoStandard,
};
use serde::{Deserialize, Serialize};

use crate::paths;

mod dto;
mod io;

pub use dto::{
    HardwareDTO, JoySourceDTO, KbModeDTO, MediaDTO, PeripheralsDTO, PortsDTO, SerialDTO, UIDTO,
};
pub use io::{load_all, save};

/// Schema version this build writes, and the newest it accepts on load.
/// Bump only on a breaking change to the TOML shape (`plan-machine-persistence.md`);
/// unknown *keys* stay forward-compatible (warned, not fatal) — only an
/// unknown *schema* number is fatal, since it means the shape itself may have
/// changed underneath us.
pub const CURRENT_SCHEMA: u32 = 1;

/// `chrono` format string for `[created]` — an ISO date, informational only
/// (`plan-machine-persistence.md` schema: `created = "2026-07-16"`).
pub const DATE_FORMAT: &str = "%Y-%m-%d";

/// A machine definition, as read from / written to `<slug>.toml`. See the
/// module doc and `docs/plan-machine-persistence.md` for the schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MachineDef {
    /// Must equal [`CURRENT_SCHEMA`] to load; see that constant's doc.
    pub schema: u32,
    /// Display name — the manager list row's title.
    pub name: String,
    /// Informational only (e.g. an ISO date); never interpreted.
    #[serde(default)]
    pub created: Option<String>,
    pub hardware: HardwareDTO,
    #[serde(default)]
    pub media: MediaDTO,
    #[serde(default)]
    pub peripherals: PeripheralsDTO,
    #[serde(default)]
    pub ports: PortsDTO,
    #[serde(default)]
    pub ui: UIDTO,
    /// Keys the loader (`io::load_one`) found in the source file but doesn't
    /// know about (top-level, and one level into each known section) — the
    /// same set it warns about via `tracing::warn`. Never serialized itself
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
    /// Convert to the `coco-core` config, running
    /// [`MachineConfig::validate`] so a definition that requests an
    /// unsupported hardware combination (e.g. CoCo 2 + PAL) fails here
    /// rather than at boot.
    pub fn to_machine_config(&self) -> Result<MachineConfig, String> {
        let variant: MachineVariant = self.hardware.variant.into();
        let memory: MemorySize = self.hardware.ram.into();
        let video: VideoStandard = self.hardware.video.into();
        let monitor: Option<MonitorType> = match self.hardware.monitor {
            Some(dto) => Some(dto.into()),
            // Absent key ⇒ the machine's own default: RGB where a monitor
            // port exists (CoCo 3), nothing where it doesn't. An explicit
            // key on a CoCo 1/2 flows through so `validate` rejects it.
            None => match variant {
                MachineVariant::Coco3 => Some(MonitorType::RGB),
                MachineVariant::Coco1 | MachineVariant::Coco2 => None,
            },
        };
        let vdg: Option<VDGVariant> = match self.hardware.vdg {
            Some(dto) => Some(dto.into()),
            // Shared with main.rs's CLI path and new_vm.rs's `constrain` —
            // see VdgDto's doc comment and `default_vdg`'s.
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

    /// Build a fresh definition from a config the manager's "New…" dialog
    /// produced (`manager.rs`'s Create flow). `media`/`peripherals`/`ui`
    /// start at their defaults — the dialog doesn't attach media or toggle
    /// peripherals; that happens afterward in the detail pane.
    pub fn from_config(name: String, created: Option<String>, config: &MachineConfig) -> Self {
        Self {
            schema: CURRENT_SCHEMA,
            name,
            created,
            hardware: HardwareDTO::from_config(config, None),
            media: MediaDTO::default(),
            peripherals: PeripheralsDTO::default(),
            ports: PortsDTO::default(),
            ui: UIDTO::default(),
            unknown: toml::Table::new(),
        }
    }
}

/// Lowercase, `[a-z0-9]` kept; every run of other characters collapses to a
/// single `-`; the result is trimmed of leading/trailing `-`; an empty
/// result becomes `"machine"`. Used to derive a slug from a machine's
/// display name at creation time (`plan-machine-persistence.md` "Identity =
/// slug").
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

/// `base`, or `base-2`, `base-3`, … — the first candidate for which
/// `taken` returns `false`. `taken` is typically "does this slug already
/// have a definition file".
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

/// Directory holding every machine definition file
/// (`config_dir()/machines`). `None` when no home directory can be
/// determined (mirrors `paths::config_dir`); not created here — [`save`]
/// creates it on demand.
pub fn machines_dir() -> Option<PathBuf> {
    paths::config_dir().map(|dir| dir.join("machines"))
}

/// Resolve one `[media]` path (`MediaDto`'s fields) the way the schema
/// promises: absolute paths are used exactly as given; relative paths
/// resolve against this machine's artifact directory,
/// `data_dir()/machines/<slug>` (`plan-machine-persistence.md` "Media by
/// reference, never embedded" — mirrors [`machines_dir`], which is the
/// `config_dir()` sibling holding the *definition* files, not media). Falls
/// back to interpreting a relative path against the process's current
/// directory when no data directory can be determined at all (`paths::data_dir`
/// docs: no home directory found) — a degraded but non-panicking result for
/// a case unit tests can't easily hit.
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

/// Root of every machine's artifact directory (`data_dir()/machines`); a
/// machine's own artifacts (created blank disks, `thumbnail.png`, later
/// snapshots) live under `<root>/<slug>`. Split out so the manager can hold
/// one injectable copy of the root — tests point it at a temp dir instead
/// of the real per-user data directory.
pub fn artifacts_root() -> Option<PathBuf> {
    paths::data_dir().map(|dir| dir.join("machines"))
}

#[cfg(test)]
#[path = "machine_def_test.rs"]
pub(crate) mod tests;
