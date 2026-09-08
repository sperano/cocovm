//! Global config file (`~/.config/cocovm/config.toml`, `paths::config_dir`).
//! Per-parameter precedence: CLI flag > environment variable > config.toml >
//! built-in default. A missing file is fine (every parameter just falls
//! through); an unreadable or malformed one is fatal at startup — same
//! severity `machine_def::load_all` gives a bad machine definition.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::Path;

use clap::ValueEnum;

use crate::cli::{Cli, LogLevel};

/// Global config file's name under [`crate::paths::config_dir`].
pub(crate) const CONFIG_FILE_NAME: &str = "config.toml";

/// Built-in default for `log_level` when neither a CLI flag, an environment
/// variable, nor `config.toml` names one.
pub(crate) const DEFAULT_LOG_LEVEL: LogLevel = LogLevel::Warn;

/// Built-in default for `toolbar_icons_only`: caption-and-icon tiles, today's
/// only behavior before this file existed.
pub(crate) const DEFAULT_TOOLBAR_ICONS_ONLY: bool = false;

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
#[derive(Debug, PartialEq)]
pub(crate) struct Config {
    pub(crate) log_level: LogLevel,
    pub(crate) control_port: u16,
    pub(crate) assets_url: String,
    pub(crate) toolbar_icons_only: bool,
    /// True when a CLI flag or env var supplied `toolbar_icons_only`; the
    /// Settings dialog then leaves the live value alone on save.
    pub(crate) toolbar_icons_only_overridden: bool,
}

/// Reads `config.toml`. `path` is `None` when no home directory could be
/// found (`paths::config_dir` docs) or missing on disk — both yield an empty
/// [`FileConfig`], so every parameter falls through to the next layer. Any
/// other read or parse failure is `Err`, naming the file, for the caller to
/// print and exit on (mirrors `machine_def::io::load_one`'s severity).
pub(crate) fn load(path: Option<&Path>) -> Result<FileConfig, String> {
    let Some(path) = path else {
        return Ok(FileConfig::default());
    };
    read(path)
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
    let toolbar_icons_only_overridden = cli.toolbar_icons_only.is_some();
    Config {
        log_level: cli
            .log_level
            .or(file.log_level)
            .unwrap_or(DEFAULT_LOG_LEVEL),
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
        toolbar_icons_only_overridden,
    }
}

/// Self-documenting template written by [`seed_default_file`]: every key
/// commented out, showing its built-in default, so a user who has never
/// touched `config.toml` still finds a reference instead of an empty file.
fn default_config_template() -> String {
    let log_level = log_level_name(DEFAULT_LOG_LEVEL);
    // `assets_url` is interpolated into a quoted TOML string unescaped: it
    // must never contain `"` or `\` (see `DEFAULT_ASSETS_URL`'s own doc in
    // startup.rs), or the generated file stops parsing.
    format!(
        "\
# cocovm global config.
# Every key below is optional; an unset key falls through to the next layer
# in the precedence chain: CLI flag > environment variable > this file >
# built-in default. The values shown here are the built-in defaults.

# error | warn | info | debug | trace
# log_level = \"{log_level}\"

# MCP control-server port; 0 disables it
# control_port = {control_port}

# first-run asset bundle URL
# assets_url = \"{assets_url}\"

# draw every toolbar as icon-only, caption moved into hover text
# toolbar_icons_only = {toolbar_icons_only}
",
        control_port = crate::control::DEFAULT_PORT,
        assets_url = crate::startup::DEFAULT_ASSETS_URL,
        toolbar_icons_only = DEFAULT_TOOLBAR_ICONS_ONLY,
    )
}

/// Writes [`default_config_template`] to `path` the first time cocovm starts
/// with no config file there yet. Never touches an existing file, and never
/// fatal — a permissions problem here just means the user keeps starting
/// with no config file, same as before this feature existed.
pub(crate) fn seed_default_file(path: &Path) {
    if let Some(parent) = path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        eprintln!("coco: cannot write {}: {e}", path.display());
        return;
    }
    // `create_new` makes the existence check and the write a single syscall,
    // so a concurrent creator can't race us between a separate check and write.
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return,
        Err(e) => {
            eprintln!("coco: cannot write {}: {e}", path.display());
            return;
        }
    };
    match file.write_all(default_config_template().as_bytes()) {
        Ok(()) => println!("Wrote default config to {}", path.display()),
        Err(e) => eprintln!("coco: cannot write {}: {e}", path.display()),
    }
}

/// Writes `file` into `path`, for the Settings dialog (`manager/settings.rs`).
/// Starts from `path`'s existing text, or [`default_config_template`] when
/// there is none yet, and edits it with `toml_edit` rather than
/// re-serializing from scratch, so the user's comments survive. Each of the
/// four keys is set when `file` names it, or removed so it keeps tracking
/// future built-in defaults. Written atomically (`.tmp` + rename), like
/// `machine_def::io::save`.
pub(crate) fn save_file(path: &Path, file: &FileConfig) -> Result<(), String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => default_config_template(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("{}: {e}", path.display()))?;

    set_or_remove(&mut doc, "log_level", file.log_level.map(log_level_name));
    set_or_remove(&mut doc, "control_port", file.control_port.map(i64::from));
    set_or_remove(&mut doc, "assets_url", file.assets_url.clone());
    set_or_remove(&mut doc, "toolbar_icons_only", file.toolbar_icons_only);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let tmp_path = path.with_extension("toml.tmp");
    fs::write(&tmp_path, doc.to_string()).map_err(|e| format!("{}: {e}", tmp_path.display()))?;
    fs::rename(&tmp_path, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// `level`'s TOML/CLI spelling — the lowercase `ValueEnum` name (`"warn"`).
pub(crate) fn log_level_name(level: LogLevel) -> String {
    level
        .to_possible_value()
        .expect("LogLevel has no skip_value variants")
        .get_name()
        .to_string()
}

/// Sets `doc[key]` to `value`, or removes `key` entirely when `value` is
/// `None` — [`save_file`]'s one rule applied per field.
fn set_or_remove<T: Into<toml_edit::Value>>(
    doc: &mut toml_edit::DocumentMut,
    key: &str,
    value: Option<T>,
) {
    match value {
        Some(v) => doc[key] = toml_edit::value(v),
        None => {
            doc.remove(key);
        }
    }
}

#[cfg(test)]
#[path = "config_test.rs"]
mod tests;
