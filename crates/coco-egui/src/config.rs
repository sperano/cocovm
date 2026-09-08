//! Global config file (`~/.config/cocovm/config.toml`, `paths::config_dir`).
//! Per-parameter precedence: CLI flag > environment variable > config.toml >
//! built-in default. A missing file is fine (every parameter just falls
//! through); an unreadable or malformed one is fatal at startup — same
//! severity `machine_def::load_all` gives a bad machine definition.

use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::{Cli, LogLevel};

/// Built-in default for `toolbar_icons_only`: caption-and-icon tiles, today's
/// only behavior before this file existed.
const DEFAULT_TOOLBAR_ICONS_ONLY: bool = false;

/// `config.toml`'s schema. Every field is optional so a partial file only
/// overrides what it names; `deny_unknown_fields` turns a typo'd key into a
/// startup error instead of a silently ignored setting.
#[derive(Debug, Default, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileConfig {
    pub(crate) log_level: Option<LogLevel>,
    pub(crate) control_port: Option<u16>,
    pub(crate) assets_url: Option<String>,
    pub(crate) toolbar_icons_only: Option<bool>,
}

/// Every global parameter, resolved to a concrete value.
#[derive(Debug)]
pub(crate) struct Config {
    pub(crate) log_level: LogLevel,
    pub(crate) control_port: u16,
    pub(crate) assets_url: String,
    pub(crate) toolbar_icons_only: bool,
}

/// Reads `config.toml`. `path` is `None` when no home directory could be
/// found (`paths::config_dir` docs) or missing on disk — both yield an empty
/// [`FileConfig`], so every parameter falls through to the next layer. Any
/// other read or parse failure is `Err`, naming the file, for the caller to
/// print and exit on (mirrors `machine_def::io::load_one`'s severity).
pub(crate) fn load(path: Option<PathBuf>) -> Result<FileConfig, String> {
    let Some(path) = path else {
        return Ok(FileConfig::default());
    };
    read(&path)
}

fn read(path: &Path) -> Result<FileConfig, String> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(FileConfig::default()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    toml::from_str(&contents).map_err(|e| format!("{}: {e}", path.display()))
}

/// Applies the precedence chain, one field at a time: the CLI value (already
/// folding in clap's own env fallback — see `cli.rs`'s struct doc) beats the
/// file value, which beats the built-in default.
pub(crate) fn resolve(cli: Cli, file: FileConfig) -> Config {
    Config {
        log_level: cli.log_level.or(file.log_level).unwrap_or(LogLevel::Warn),
        control_port: cli
            .control_port
            .or(file.control_port)
            .unwrap_or(crate::control::DEFAULT_PORT),
        assets_url: cli
            .assets_url
            .or(file.assets_url)
            .unwrap_or_else(|| crate::startup::DEFAULT_ASSETS_URL.to_string()),
        toolbar_icons_only: cli
            .toolbar_icons_only
            .or(file.toolbar_icons_only)
            .unwrap_or(DEFAULT_TOOLBAR_ICONS_ONLY),
    }
}

#[cfg(test)]
#[path = "config_test.rs"]
mod tests;
