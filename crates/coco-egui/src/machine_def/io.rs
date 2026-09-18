//! Reading and writing `<slug>.toml` files: unknown-key forward-compat
//! handling ([`warn_unknown_keys`]/[`extract_unknown`]/[`merge_unknown`]),
//! plus [`load_all`]/[`save`] themselves. See the parent module doc for the
//! overall format and error-handling policy.

use std::fs;
use std::path::Path;

use super::{CURRENT_SCHEMA, MachineDef};
use crate::MPI_SLOT_COUNT;

const TOP_LEVEL_KEYS: &[&str] = &[
    "schema",
    "name",
    "created",
    "hardware",
    "media",
    "drivewire",
    "peripherals",
    "ports",
    "ui",
    "stats",
];
// `monitor` stays listed though saves no longer write it — legacy files
// carrying it must load without an unknown-key warning (`DisplayDTO`'s doc).
const HARDWARE_KEYS: &[&str] = &[
    "variant", "ram", "video", "monitor", "display", "vdg", "rom",
];
const MEDIA_KEYS: &[&str] = &["disk0", "disk1", "vhd0", "vhd1", "tape"];
const DRIVEWIRE_KEYS: &[&str] = &["enabled", "hdbdos_mode", "disk0", "disk1", "disk2", "disk3"];
// `mpi`/`rtc`/`fd502`/`rs232` (schema-1's booleans) are deliberately absent: a
// schema-1 file's leftover `[peripherals]` must report them as unknown, not
// silently accept them (`PeripheralsDTO::cartridge`'s doc explains why the
// missing-`cartridge` parse error is what actually rejects such a file).
const PERIPHERALS_KEYS: &[&str] = &["cartridge"];
const PORTS_KEYS: &[&str] = &["serial"];
// Recognize the retired `aspect_correct` key so legacy files load quietly
// and drop it on save instead of preserving it as an unknown future setting.
const UI_KEYS: &[&str] = &[
    "aspect_correct",
    "kb_mode",
    "joy_left",
    "joy_right",
    "hires_left",
    "hires_right",
    "tv_scanline",
    "tv_noise",
    "tv_overscan",
];
const STATS_KEYS: &[&str] = &["runtime_secs", "starts"];

/// Sections that nest under the top level, paired with their known-key
/// lists, so [`warn_unknown_keys`] can recurse one level without extra
/// machinery (deliberately not using a `serde_ignored`-style hook — see the
/// module doc's "keep it simple and contained").
const KNOWN_SECTIONS: &[(&str, &[&str])] = &[
    ("hardware", HARDWARE_KEYS),
    ("media", MEDIA_KEYS),
    ("drivewire", DRIVEWIRE_KEYS),
    ("peripherals", PERIPHERALS_KEYS),
    ("ports", PORTS_KEYS),
    ("ui", UI_KEYS),
    ("stats", STATS_KEYS),
];

/// Log a `tracing::warn` for every TOML key not in the known schema (top
/// level, one level into each known section). Unknown keys are forward-compat, not fatal.
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
            tracing::warn!("{}: unknown key '{full_key}' ignored", path.display());
        }
    }
}

/// The same unknown keys [`warn_unknown_keys`] warns about, collected into a
/// table shaped like the source file — `MachineDef::unknown`'s value, merged
/// back in by [`merge_unknown`] at save time.
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

/// Merge `unknown` (an [`extract_unknown`]-shaped table) into `table` so
/// keys this build ignored survive a Save. Section entries merge key-by-key
/// into the existing sub-table rather than overwriting it.
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

/// The 1-based front-panel switch positions that a
/// [`super::CartridgeDTO::MPI`]'s `switch` accepts, matching the UI's
/// "Slot 1"–"Slot 4".
const MPI_SWITCH_RANGE: std::ops::RangeInclusive<i64> = 1..=(MPI_SLOT_COUNT as i64);

/// A `[peripherals].cartridge`'s `slots` array of the wrong length, or a
/// `switch` outside [`MPI_SWITCH_RANGE`], is caught here, before the typed
/// deserialize, so the error names the expected count/range instead of
/// serde's generic complaints. A missing `slots` key needs no help here: it's
/// a required field of [`super::CartridgeDTO::MPI`] itself, so serde's own
/// "missing field `slots`" error already names the problem.
fn check_mpi_slots(table: &toml::Table, path: &Path) -> Result<(), String> {
    let Some(toml::Value::Table(peripherals)) = table.get("peripherals") else {
        return Ok(());
    };
    let Some(toml::Value::Table(cartridge)) = peripherals.get("cartridge") else {
        return Ok(());
    };
    let is_mpi = cartridge.get("kind").and_then(toml::Value::as_str) == Some("mpi");
    if let Some(toml::Value::Array(slots)) = cartridge.get("slots")
        && is_mpi
    {
        if slots.len() != MPI_SLOT_COUNT {
            return Err(format!(
                "{}: [peripherals].cartridge.slots lists {} slots but a MultiPak Interface has \
                 exactly {MPI_SLOT_COUNT}",
                path.display(),
                slots.len()
            ));
        }
        check_single_rs232_slot(slots, path)?;
    }
    if let Some(toml::Value::Integer(switch)) = cartridge.get("switch")
        && is_mpi
        && !MPI_SWITCH_RANGE.contains(switch)
    {
        return Err(format!(
            "{}: [peripherals].cartridge.switch is {switch} but a MultiPak Interface's \
             front-panel switch must be {} to {}",
            path.display(),
            MPI_SWITCH_RANGE.start(),
            MPI_SWITCH_RANGE.end()
        ));
    }
    Ok(())
}

/// At most one `slots` entry may be `{ kind = "rs232" }`: two Deluxe RS-232
/// Paks would fight over the shared ACIA at `$FF68-$FF6B` (the pak decodes
/// the full address bus itself, reachable from any slot regardless of
/// switch/`$FF7F` selection — `coco_rs232.cpp:57-62`).
fn check_single_rs232_slot(slots: &[toml::Value], path: &Path) -> Result<(), String> {
    let rs232_count = slots
        .iter()
        .filter(|slot| {
            matches!(slot, toml::Value::Table(t) if t.get("kind").and_then(toml::Value::as_str) == Some("rs232"))
        })
        .count();
    if rs232_count > 1 {
        return Err(format!(
            "{}: [peripherals].cartridge.slots has {rs232_count} Deluxe RS-232 Paks, but only \
             one fits per machine (all would share the ACIA at $FF68)",
            path.display()
        ));
    }
    Ok(())
}

/// Load and validate one definition file (parse + schema check + hardware
/// validation), warning about unknown keys along the way.
fn load_one(path: &Path) -> Result<MachineDef, String> {
    let contents = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let table: toml::Table =
        toml::from_str(&contents).map_err(|e| format!("{}: {e}", path.display()))?;
    warn_unknown_keys(&table, path);
    check_mpi_slots(&table, path)?;
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

/// Load every `*.toml` file directly inside `dir` (dotfiles skipped), keyed
/// by slug, sorted. A missing `dir` yields an empty list, but any other
/// problem fails the whole load — a config problem is fatal at startup by design.
pub fn load_all(dir: &Path) -> Result<Vec<(String, MachineDef)>, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    let mut results: Vec<(String, MachineDef)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
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
        results.push((stem.to_string(), load_one(&path)?));
    }
    results.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(results)
}

/// Serialize `def` and write it to `<dir>/<slug>.toml`, merging `def.unknown`
/// back in so a Save never erases what this build doesn't understand. Writes
/// to a `.tmp` sibling and renames over the final path so a crash never
/// leaves a partial file.
pub fn save(dir: &Path, slug: &str, def: &MachineDef) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let toml::Value::Table(mut table) =
        toml::Value::try_from(def).map_err(|e| format!("serializing {slug}: {e}"))?
    else {
        // A struct always serializes to a TOML table; this arm exists only so `save` stays a
        // `Result`.
        return Err(format!(
            "serializing {slug}: expected a TOML table at the top level"
        ));
    };
    merge_unknown(&mut table, &def.unknown);
    let text = toml::to_string_pretty(&table).map_err(|e| format!("serializing {slug}: {e}"))?;
    let tmp_path = dir.join(format!("{slug}.toml.tmp"));
    let final_path = dir.join(format!("{slug}.toml"));
    fs::write(&tmp_path, text).map_err(|e| format!("{}: {e}", tmp_path.display()))?;
    fs::rename(&tmp_path, &final_path).map_err(|e| format!("{}: {e}", final_path.display()))?;
    Ok(())
}
