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
//! both TOML parse errors and `MachineConfig::validate` failures for its
//! list-row error badge.
//!
//! `manager.rs` wires this module in for the list rows, "New…" flow, and
//! detail/edit pane (`plan-machine-persistence.md` steps 2-4).

use std::fs;
use std::path::{Path, PathBuf};

use coco_core::{
    MachineConfig, MachineVariant, MemorySize, MonitorType, VdgVariant, VideoStandard,
};
use serde::{Deserialize, Serialize};

use crate::paths;

/// Schema version this build writes, and the newest it accepts on load.
/// Bump only on a breaking change to the TOML shape (`plan-machine-persistence.md`);
/// unknown *keys* stay forward-compatible (warned, not fatal) — only an
/// unknown *schema* number is fatal, since it means the shape itself may have
/// changed underneath us.
pub const CURRENT_SCHEMA: u32 = 1;

/// `chrono` format string for `[created]` — an ISO date, informational only
/// (`plan-machine-persistence.md` schema: `created = "2026-07-16"`).
pub const DATE_FORMAT: &str = "%Y-%m-%d";

/// `[hardware].variant`. Maps to [`coco_core::MachineVariant`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VariantDto {
    #[serde(rename = "coco1")]
    Coco1,
    #[serde(rename = "coco2")]
    Coco2,
    #[serde(rename = "coco3")]
    Coco3,
}

impl From<MachineVariant> for VariantDto {
    fn from(variant: MachineVariant) -> Self {
        match variant {
            MachineVariant::Coco1 => VariantDto::Coco1,
            MachineVariant::Coco2 => VariantDto::Coco2,
            MachineVariant::Coco3 => VariantDto::Coco3,
        }
    }
}

impl From<VariantDto> for MachineVariant {
    fn from(variant: VariantDto) -> Self {
        match variant {
            VariantDto::Coco1 => MachineVariant::Coco1,
            VariantDto::Coco2 => MachineVariant::Coco2,
            VariantDto::Coco3 => MachineVariant::Coco3,
        }
    }
}

/// `[hardware].ram`. Maps to [`coco_core::MemorySize`]; variant names like
/// `512k` aren't valid Rust identifiers, hence the explicit renames rather
/// than a derived `rename_all`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RamDto {
    #[serde(rename = "4k")]
    K4,
    #[serde(rename = "16k")]
    K16,
    #[serde(rename = "32k")]
    K32,
    #[serde(rename = "64k")]
    K64,
    #[serde(rename = "128k")]
    K128,
    #[serde(rename = "512k")]
    K512,
    #[serde(rename = "2048k")]
    K2048,
}

impl From<MemorySize> for RamDto {
    fn from(memory: MemorySize) -> Self {
        match memory {
            MemorySize::K4 => RamDto::K4,
            MemorySize::K16 => RamDto::K16,
            MemorySize::K32 => RamDto::K32,
            MemorySize::K64 => RamDto::K64,
            MemorySize::K128 => RamDto::K128,
            MemorySize::K512 => RamDto::K512,
            MemorySize::K2048 => RamDto::K2048,
        }
    }
}

impl From<RamDto> for MemorySize {
    fn from(ram: RamDto) -> Self {
        match ram {
            RamDto::K4 => MemorySize::K4,
            RamDto::K16 => MemorySize::K16,
            RamDto::K32 => MemorySize::K32,
            RamDto::K64 => MemorySize::K64,
            RamDto::K128 => MemorySize::K128,
            RamDto::K512 => MemorySize::K512,
            RamDto::K2048 => MemorySize::K2048,
        }
    }
}

/// `[hardware].video`. Maps to [`coco_core::VideoStandard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoDto {
    #[serde(rename = "ntsc")]
    Ntsc,
    #[serde(rename = "pal")]
    Pal,
}

impl From<VideoStandard> for VideoDto {
    fn from(video: VideoStandard) -> Self {
        match video {
            VideoStandard::Ntsc => VideoDto::Ntsc,
            VideoStandard::Pal => VideoDto::Pal,
        }
    }
}

impl From<VideoDto> for VideoStandard {
    fn from(video: VideoDto) -> Self {
        match video {
            VideoDto::Ntsc => VideoStandard::Ntsc,
            VideoDto::Pal => VideoStandard::Pal,
        }
    }
}

/// `[hardware].monitor`. Maps to [`coco_core::gime::MonitorType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MonitorDto {
    #[serde(rename = "rgb")]
    Rgb,
    #[serde(rename = "composite")]
    Composite,
}

impl From<MonitorType> for MonitorDto {
    fn from(monitor: MonitorType) -> Self {
        match monitor {
            MonitorType::Rgb => MonitorDto::Rgb,
            MonitorType::Composite => MonitorDto::Composite,
        }
    }
}

impl From<MonitorDto> for MonitorType {
    fn from(monitor: MonitorDto) -> Self {
        match monitor {
            MonitorDto::Rgb => MonitorType::Rgb,
            MonitorDto::Composite => MonitorType::Composite,
        }
    }
}

/// `[hardware].vdg`. Maps to [`coco_core::VdgVariant`]. Optional in the file
/// — when absent, [`MachineDef::to_machine_config`] defaults it per variant
/// the same way `main.rs`'s CLI path and `new_vm.rs`'s `constrain_draft` do:
/// the T1 (CoCo 2B) on a CoCo 2, the plain MC6847 elsewhere (the only choice
/// `MachineConfig::validate` accepts there).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VdgDto {
    #[serde(rename = "mc6847")]
    Mc6847,
    #[serde(rename = "mc6847t1")]
    Mc6847T1,
}

impl From<VdgVariant> for VdgDto {
    fn from(vdg: VdgVariant) -> Self {
        match vdg {
            VdgVariant::Mc6847 => VdgDto::Mc6847,
            VdgVariant::Mc6847T1 => VdgDto::Mc6847T1,
        }
    }
}

impl From<VdgDto> for VdgVariant {
    fn from(vdg: VdgDto) -> Self {
        match vdg {
            VdgDto::Mc6847 => VdgVariant::Mc6847,
            VdgDto::Mc6847T1 => VdgVariant::Mc6847T1,
        }
    }
}

/// `[ui].kb_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum KbModeDto {
    #[default]
    #[serde(rename = "positional")]
    Positional,
    #[serde(rename = "symbolic")]
    Symbolic,
}

/// `[hardware]` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareDto {
    pub variant: VariantDto,
    pub ram: RamDto,
    pub video: VideoDto,
    pub monitor: MonitorDto,
    /// Absent ⇒ per-variant default; see [`VdgDto`].
    #[serde(default)]
    pub vdg: Option<VdgDto>,
    /// Absent ⇒ default ROM composition (`rom_db`/`load_default_rom`).
    #[serde(default)]
    pub rom: Option<String>,
}

impl HardwareDto {
    /// Build the `[hardware]` section from a config the "New…" dialog or the
    /// manager's detail-pane form produced (`new_vm::config_form_rows`
    /// already ran [`MachineConfig::validate`]-compatible constraints on
    /// it). `vdg` is always written explicitly here — the dialog/pane always
    /// resolve a concrete choice, unlike a hand-written TOML file that may
    /// omit it to take the per-variant default (see [`VdgDto`]'s doc).
    /// `rom` is passed through as-is: the custom-ROM path isn't part of
    /// [`MachineConfig`] and has no editor yet.
    pub fn from_config(config: &MachineConfig, rom: Option<String>) -> Self {
        Self {
            variant: config.variant.into(),
            ram: config.memory.into(),
            video: config.video.into(),
            monitor: config.monitor.into(),
            vdg: Some(config.vdg.into()),
            rom,
        }
    }
}

/// `[media]` section — every key optional, the section itself optional.
/// Relative paths are meant to resolve against the machine's artifact
/// directory (`data_dir()/machines/<slug>`), never embedded
/// (`plan-machine-persistence.md` "Media by reference, never embedded");
/// no caller resolves media paths yet (`plan-machine-persistence.md` step 5,
/// launch/media mounting — not implemented).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaDto {
    #[serde(default)]
    pub cart: Option<String>,
    #[serde(default)]
    pub disk0: Option<String>,
    #[serde(default)]
    pub disk1: Option<String>,
    #[serde(default)]
    pub vhd0: Option<String>,
    #[serde(default)]
    pub vhd1: Option<String>,
    #[serde(default)]
    pub tape: Option<String>,
}

/// `[peripherals]` section — section itself optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PeripheralsDto {
    #[serde(default)]
    pub mpi: bool,
    #[serde(default)]
    pub rtc: bool,
}

/// Default for `[ui].aspect_correct` — `bool::default()` is `false`, but the
/// file-format default is `true` (aspect correction on), so this needs its
/// own default function rather than relying on `Default::default()`.
fn default_aspect_correct() -> bool {
    true
}

/// `[ui]` section — section itself optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiDto {
    #[serde(default = "default_aspect_correct")]
    pub aspect_correct: bool,
    #[serde(default)]
    pub kb_mode: KbModeDto,
}

impl Default for UiDto {
    fn default() -> Self {
        Self {
            aspect_correct: true,
            kb_mode: KbModeDto::default(),
        }
    }
}

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
    pub hardware: HardwareDto,
    #[serde(default)]
    pub media: MediaDto,
    #[serde(default)]
    pub peripherals: PeripheralsDto,
    #[serde(default)]
    pub ui: UiDto,
    /// Keys `load_one` found in the source file but doesn't know about
    /// (top-level, and one level into each of [`KNOWN_SECTIONS`]) — the same
    /// set [`warn_unknown_keys`] warns about. Never serialized itself
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
        let monitor: MonitorType = self.hardware.monitor.into();
        let vdg: VdgVariant = match self.hardware.vdg {
            Some(dto) => dto.into(),
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
            hardware: HardwareDto::from_config(config, None),
            media: MediaDto::default(),
            peripherals: PeripheralsDto::default(),
            ui: UiDto::default(),
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

const TOP_LEVEL_KEYS: &[&str] = &["schema", "name", "created", "hardware", "media", "peripherals", "ui"];
const HARDWARE_KEYS: &[&str] = &["variant", "ram", "video", "monitor", "vdg", "rom"];
const MEDIA_KEYS: &[&str] = &["cart", "disk0", "disk1", "vhd0", "vhd1", "tape"];
const PERIPHERALS_KEYS: &[&str] = &["mpi", "rtc"];
const UI_KEYS: &[&str] = &["aspect_correct", "kb_mode"];

/// Sections that nest under the top level, paired with their known-key
/// lists, so [`warn_unknown_keys`] can recurse one level without extra
/// machinery (deliberately not using a `serde_ignored`-style hook — see the
/// module doc's "keep it simple and contained").
const KNOWN_SECTIONS: &[(&str, &[&str])] = &[
    ("hardware", HARDWARE_KEYS),
    ("media", MEDIA_KEYS),
    ("peripherals", PERIPHERALS_KEYS),
    ("ui", UI_KEYS),
];

/// Log a `tracing::warn` naming `path` and the key for every TOML key not in
/// the known schema, at the top level and one level into each known
/// section. Unknown keys are forward-compat, not fatal — the file still
/// loads (`plan-machine-persistence.md` "Decisions").
fn warn_unknown_keys(table: &toml::Table, path: &Path) {
    check_known_keys(table, TOP_LEVEL_KEYS, path, "");
    for &(section, known) in KNOWN_SECTIONS {
        if let Some(toml::Value::Table(sub)) = table.get(section) {
            check_known_keys(sub, known, path, section);
        }
    }
}

fn check_known_keys(table: &toml::Table, known: &[&str], path: &Path, prefix: &str) {
    for key in table.keys() {
        if !known.contains(&key.as_str()) {
            let full_key = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            tracing::warn!(
                "{}: unknown key '{full_key}' ignored",
                path.display()
            );
        }
    }
}

/// The same unknown keys [`warn_unknown_keys`] warns about (top level, and
/// one level into each of [`KNOWN_SECTIONS`]), collected into a table shaped
/// like the source file rather than logged: `MachineDef::unknown`'s value,
/// merged back in by [`merge_unknown`] at save time so a Save doesn't erase
/// keys this build doesn't understand.
fn extract_unknown(table: &toml::Table) -> toml::Table {
    let mut unknown = toml::Table::new();
    for (key, value) in table {
        if !TOP_LEVEL_KEYS.contains(&key.as_str()) {
            unknown.insert(key.clone(), value.clone());
        }
    }
    for &(section, known) in KNOWN_SECTIONS {
        if let Some(toml::Value::Table(sub)) = table.get(section) {
            let mut extra = toml::Table::new();
            for (key, value) in sub {
                if !known.contains(&key.as_str()) {
                    extra.insert(key.clone(), value.clone());
                }
            }
            if !extra.is_empty() {
                unknown.insert(section.to_string(), toml::Value::Table(extra));
            }
        }
    }
    unknown
}

/// Merge `unknown` (an [`extract_unknown`]-shaped table) into `table` (the
/// freshly-serialized `MachineDef`), so the keys this build ignored survive
/// a Save. `table` never already has these keys — they're unknown to
/// `MachineDef`'s serde derive by construction — except for a known
/// section's table itself, which the derive always writes (even empty), so
/// section entries are merged key-by-key into the existing sub-table rather
/// than overwriting it.
fn merge_unknown(table: &mut toml::Table, unknown: &toml::Table) {
    for (key, value) in unknown {
        let is_known_section = KNOWN_SECTIONS.iter().any(|&(section, _)| section == key);
        match (is_known_section, value) {
            (true, toml::Value::Table(extra)) => {
                let entry = table
                    .entry(key.clone())
                    .or_insert_with(|| toml::Value::Table(toml::Table::new()));
                if let toml::Value::Table(sub) = entry {
                    for (extra_key, extra_value) in extra {
                        sub.insert(extra_key.clone(), extra_value.clone());
                    }
                }
            }
            _ => {
                table.insert(key.clone(), value.clone());
            }
        }
    }
}

/// Load and validate one definition file (parse + schema check + hardware
/// validation), warning about unknown keys along the way.
fn load_one(path: &Path) -> Result<MachineDef, String> {
    let contents = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let table: toml::Table =
        toml::from_str(&contents).map_err(|e| format!("{}: {e}", path.display()))?;
    warn_unknown_keys(&table, path);
    let mut def: MachineDef = table
        .clone()
        .try_into()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    def.unknown = extract_unknown(&table);
    if def.schema != CURRENT_SCHEMA {
        return Err(format!(
            "{}: schema {} is not supported (this build supports schema {CURRENT_SCHEMA})",
            path.display(),
            def.schema
        ));
    }
    def.to_machine_config()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(def)
}

/// Load every `*.toml` file directly inside `dir` (hidden files — dotfiles —
/// skipped), keyed by slug (file stem), sorted by slug. A missing `dir`
/// yields an empty list rather than an error (a fresh install has no
/// machines yet). A bad file's error is captured per-entry, never dropped
/// silently and never hiding the rest of the directory
/// (`plan-machine-persistence.md` step 1).
pub fn load_all(dir: &Path) -> Vec<(String, Result<MachineDef, String>)> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let mut results: Vec<(String, Result<MachineDef, String>)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if stem.starts_with('.') {
            continue;
        }
        results.push((stem.to_string(), load_one(&path)));
    }
    results.sort_by(|a, b| a.0.cmp(&b.0));
    results
}

/// Serialize `def` and write it to `<dir>/<slug>.toml`, creating `dir` if
/// needed. `def.unknown` (keys `load_one` couldn't place, see its doc) is
/// merged back into the serialized table before writing, so a Save never
/// erases what this build doesn't understand. Writes to a `.tmp` sibling
/// first and renames over the final path, so a crash mid-write never leaves
/// a truncated/partial definition file behind — readers only ever see the
/// old file or the fully-written new one.
pub fn save(dir: &Path, slug: &str, def: &MachineDef) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let toml::Value::Table(mut table) =
        toml::Value::try_from(def).map_err(|e| format!("serializing {slug}: {e}"))?
    else {
        // A struct's top level always serializes to a TOML table
        // (`toml::Value::try_from` on any `#[derive(Serialize)]` struct);
        // this arm exists only so `save` stays a `Result`, not a panic, if
        // that guarantee ever stops holding.
        return Err(format!("serializing {slug}: expected a TOML table at the top level"));
    };
    merge_unknown(&mut table, &def.unknown);
    let text = toml::to_string_pretty(&table).map_err(|e| format!("serializing {slug}: {e}"))?;
    let tmp_path = dir.join(format!("{slug}.toml.tmp"));
    let final_path = dir.join(format!("{slug}.toml"));
    fs::write(&tmp_path, text).map_err(|e| format!("{}: {e}", tmp_path.display()))?;
    fs::rename(&tmp_path, &final_path).map_err(|e| format!("{}: {e}", final_path.display()))?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Unique temp directory per test, cleaned up on drop so parallel tests
    /// never collide and nothing lingers under the OS temp dir. `pub(crate)`
    /// so `ui_tests.rs` (a sibling module, both descendants of the crate
    /// root) shares this instead of keeping its own copy.
    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        pub(crate) fn new(tag: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "coco-egui-machine-def-test-{tag}-{}-{n}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn full_def() -> MachineDef {
        MachineDef {
            schema: CURRENT_SCHEMA,
            name: "Dev CoCo 3".to_string(),
            created: Some("2026-07-16".to_string()),
            hardware: HardwareDto {
                variant: VariantDto::Coco3,
                ram: RamDto::K512,
                video: VideoDto::Ntsc,
                monitor: MonitorDto::Rgb,
                vdg: Some(VdgDto::Mc6847),
                rom: Some("/path/custom.rom".to_string()),
            },
            media: MediaDto {
                cart: Some("/paks/arkanoid.ccc".to_string()),
                disk0: Some("dev.dsk".to_string()),
                disk1: Some("/shared/utils.dsk".to_string()),
                vhd0: Some("/vhd/68SDC.VHD".to_string()),
                vhd1: None,
                tape: Some("session.cas".to_string()),
            },
            peripherals: PeripheralsDto {
                mpi: true,
                rtc: true,
            },
            ui: UiDto {
                aspect_correct: false,
                kb_mode: KbModeDto::Symbolic,
            },
            unknown: toml::Table::new(),
        }
    }

    #[test]
    fn round_trip_full_definition() {
        let dir = TempDir::new("roundtrip");
        let def = full_def();
        save(dir.path(), "dev-coco-3", &def).expect("save should succeed");

        let loaded = load_all(dir.path());
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0, "dev-coco-3");
        let loaded_def = loaded[0].1.as_ref().expect("should parse and validate");
        assert_eq!(loaded_def, &def);
    }

    #[test]
    fn minimal_file_uses_defaults() {
        let dir = TempDir::new("minimal");
        let toml_text = r#"
schema = 1
name = "Bare"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"
"#;
        fs::write(dir.path().join("bare.toml"), toml_text).unwrap();

        let loaded = load_all(dir.path());
        assert_eq!(loaded.len(), 1);
        let def = loaded[0].1.as_ref().expect("minimal file should parse");
        assert_eq!(def.created, None);
        assert_eq!(def.hardware.vdg, None);
        assert_eq!(def.hardware.rom, None);
        assert_eq!(def.media, MediaDto::default());
        assert!(!def.peripherals.mpi);
        assert!(!def.peripherals.rtc);
        assert!(def.ui.aspect_correct);
        assert_eq!(def.ui.kb_mode, KbModeDto::Positional);

        // Default VDG is per-variant: CoCo 2 -> T1, else plain MC6847.
        let config = def.to_machine_config().expect("should validate");
        assert_eq!(config.vdg, VdgVariant::Mc6847);
    }

    #[test]
    fn minimal_coco2_defaults_to_t1_vdg() {
        let dir = TempDir::new("minimal-coco2");
        let toml_text = r#"
schema = 1
name = "Bare CoCo 2"

[hardware]
variant = "coco2"
ram = "64k"
video = "ntsc"
monitor = "rgb"
"#;
        fs::write(dir.path().join("bare2.toml"), toml_text).unwrap();
        let loaded = load_all(dir.path());
        let def = loaded[0].1.as_ref().expect("minimal file should parse");
        let config = def.to_machine_config().expect("should validate");
        assert_eq!(config.vdg, VdgVariant::Mc6847T1);
    }

    #[test]
    fn future_schema_errors_without_hiding_other_files() {
        let dir = TempDir::new("schema");
        fs::write(
            dir.path().join("too-new.toml"),
            r#"
schema = 2
name = "From the future"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"
"#,
        )
        .unwrap();
        let good = full_def();
        save(dir.path(), "good", &good).unwrap();

        let mut loaded = load_all(dir.path());
        loaded.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(loaded.len(), 2);

        let (slug, result) = &loaded[0];
        assert_eq!(slug, "good");
        assert!(result.is_ok());

        let (slug, result) = &loaded[1];
        assert_eq!(slug, "too-new");
        let err = result.as_ref().unwrap_err();
        assert!(err.contains('2'), "error should name the file's schema: {err}");
        assert!(
            err.contains(&CURRENT_SCHEMA.to_string()),
            "error should name the supported schema: {err}"
        );
    }

    #[test]
    fn invalid_hardware_combination_errors_via_validate() {
        let dir = TempDir::new("invalid-hw");
        // CoCo 2 + PAL: rejected by MachineConfig::validate (plain MC6847
        // PAL timing isn't modeled).
        fs::write(
            dir.path().join("bad.toml"),
            r#"
schema = 1
name = "Bad CoCo 2"

[hardware]
variant = "coco2"
ram = "64k"
video = "pal"
monitor = "rgb"
"#,
        )
        .unwrap();
        let loaded = load_all(dir.path());
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].1.is_err());
    }

    #[test]
    fn slugify_cases() {
        assert_eq!(slugify("Dev CoCo 3"), "dev-coco-3");
        assert_eq!(slugify("Éric's CoCo 3!!!"), "ric-s-coco-3");
        assert_eq!(slugify("   "), "machine");
        assert_eq!(slugify(""), "machine");
        assert_eq!(slugify("___"), "machine");
        assert_eq!(slugify("already-a-slug"), "already-a-slug");
        assert_eq!(slugify("Multiple   Spaces"), "multiple-spaces");
    }

    #[test]
    fn unique_slug_appends_numeric_suffix() {
        let taken = |s: &str| matches!(s, "dev" | "dev-2" | "dev-3");
        assert_eq!(unique_slug("dev", &taken), "dev-4");
        assert_eq!(unique_slug("fresh", &taken), "fresh");
    }

    #[test]
    fn unknown_keys_still_load() {
        let dir = TempDir::new("unknown-keys");
        fs::write(
            dir.path().join("extra.toml"),
            r#"
schema = 1
name = "Has Extras"
future_top_level_field = true

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"
future_hardware_field = "whatever"

[ui]
aspect_correct = true
future_ui_field = 42
"#,
        )
        .unwrap();
        let loaded = load_all(dir.path());
        assert_eq!(loaded.len(), 1);
        assert!(
            loaded[0].1.is_ok(),
            "unknown keys must warn, not fail: {:?}",
            loaded[0].1
        );
    }

    /// A Save must not erase keys this build doesn't understand — the
    /// loader treats them as forward-compatible (`unknown_keys_still_load`
    /// above), and the manager's detail pane round-trips every loaded
    /// definition through `save` on every edit, so losing them there would
    /// contradict that compatibility story.
    #[test]
    fn save_preserves_unknown_keys() {
        let dir = TempDir::new("preserve-unknown");
        fs::write(
            dir.path().join("extra.toml"),
            r#"
schema = 1
name = "Has Extras"
future_top_level_field = true

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"
future_hardware_field = "whatever"

[ui]
aspect_correct = true
future_ui_field = 42
"#,
        )
        .unwrap();
        let loaded = load_all(dir.path());
        let mut def = loaded[0].1.clone().expect("should parse despite unknown keys");

        // A real edit through the detail pane's flow: change something the
        // form actually owns, then save.
        def.name = "Has Extras (renamed)".to_string();
        save(dir.path(), "extra", &def).expect("save should succeed");

        let contents = fs::read_to_string(dir.path().join("extra.toml")).unwrap();
        let table: toml::Table = toml::from_str(&contents).unwrap();
        assert_eq!(table.get("future_top_level_field"), Some(&toml::Value::Boolean(true)));
        assert_eq!(
            table["hardware"].get("future_hardware_field"),
            Some(&toml::Value::String("whatever".to_string()))
        );
        assert_eq!(table["ui"].get("future_ui_field"), Some(&toml::Value::Integer(42)));

        // And the edit itself did take effect.
        assert_eq!(table["name"], toml::Value::String("Has Extras (renamed)".to_string()));
    }

    #[test]
    fn save_is_atomic_no_leftover_tmp_file() {
        let dir = TempDir::new("atomic-save");
        let def = full_def();
        save(dir.path(), "dev-coco-3", &def).expect("save should succeed");

        let tmp_path = dir.path().join("dev-coco-3.toml.tmp");
        assert!(!tmp_path.exists(), "no .tmp file should remain after save");

        let final_path = dir.path().join("dev-coco-3.toml");
        let contents = fs::read_to_string(&final_path).expect("final file should exist");
        let parsed: MachineDef = toml::from_str(&contents).expect("saved file should parse");
        assert_eq!(parsed, def);
    }

    /// [`MachineDef::from_config`] (the manager's "New…" Create path) must
    /// round-trip back through [`MachineDef::to_machine_config`] to exactly
    /// the config it was built from.
    #[test]
    fn from_config_round_trips_through_to_machine_config() {
        let config = MachineConfig {
            variant: MachineVariant::Coco2,
            video: VideoStandard::Ntsc,
            memory: MemorySize::K16,
            monitor: MonitorType::Composite,
            vdg: VdgVariant::Mc6847T1,
        };
        let def = MachineDef::from_config("Test CoCo 2".to_string(), None, &config);
        assert_eq!(def.hardware.vdg, Some(VdgDto::Mc6847T1));
        let round_tripped = def.to_machine_config().expect("from_config produces a valid def");
        // MachineConfig has no PartialEq derive — compare fields directly.
        assert_eq!(round_tripped.variant, config.variant);
        assert_eq!(round_tripped.video, config.video);
        assert_eq!(round_tripped.memory, config.memory);
        assert_eq!(round_tripped.monitor, config.monitor);
        assert_eq!(round_tripped.vdg, config.vdg);
    }

    #[test]
    fn resolve_media_path_leaves_absolute_paths_untouched() {
        let abs = if cfg!(windows) { r"C:\shared\utils.dsk" } else { "/shared/utils.dsk" };
        assert_eq!(resolve_media_path(abs, "dev-coco-3"), PathBuf::from(abs));
    }

    #[test]
    fn resolve_media_path_resolves_relative_against_the_slugs_artifact_dir() {
        let resolved = resolve_media_path("dev.dsk", "dev-coco-3");
        let data_dir = paths::data_dir().expect("home dir should exist in tests");
        assert_eq!(resolved, data_dir.join("machines").join("dev-coco-3").join("dev.dsk"));
    }

    #[test]
    fn missing_dir_returns_empty_list() {
        let dir = std::env::temp_dir().join("coco-egui-machine-def-test-does-not-exist");
        let _ = fs::remove_dir_all(&dir);
        assert!(load_all(&dir).is_empty());
    }
}
