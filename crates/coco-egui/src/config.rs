//! Global config file (`~/.config/cocovm/config.toml`, `paths::config_dir`).
//! Per-parameter precedence: CLI flag > environment variable > config.toml >
//! built-in default. A missing file is fine (every parameter just falls
//! through); an unreadable or malformed one is fatal at startup — same
//! severity `machine_def::load_all` gives a bad machine definition.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::num::NonZeroU32;
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

/// Built-in default for `status_bar_icons_only`: icon-and-readout entries.
pub(crate) const DEFAULT_STATUS_BAR_ICONS_ONLY: bool = false;

/// Built-in default for `welcome_image_cycle`: the manager keeps the
/// welcome image it picked at startup.
pub(crate) const DEFAULT_WELCOME_IMAGE_CYCLE: bool = false;

/// Built-in default for `welcome_image_cycle_secs`.
pub(crate) const DEFAULT_WELCOME_IMAGE_CYCLE_SECS: NonZeroU32 = NonZeroU32::new(30).unwrap();

/// Built-in default for `welcome_image_shuffle`: cycle in file-name order.
pub(crate) const DEFAULT_WELCOME_IMAGE_SHUFFLE: bool = false;

/// Machine-list ordering values accepted by `config.toml`'s `manager_sort` key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ManagerSort {
    CreatedDesc,
    CreatedAsc,
    NameAsc,
    NameDesc,
}

/// Built-in machine-list ordering: newest definitions first.
pub(crate) const DEFAULT_MANAGER_SORT: ManagerSort = ManagerSort::CreatedDesc;

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
    pub(crate) status_bar_icons_only: Option<bool>,
    pub(crate) welcome_image_cycle: Option<bool>,
    /// `NonZeroU32`: a `0` in the file is a parse error, not a
    /// swap-every-frame loop.
    pub(crate) welcome_image_cycle_secs: Option<NonZeroU32>,
    pub(crate) welcome_image_shuffle: Option<bool>,
    pub(crate) manager_sort: Option<ManagerSort>,
}

/// Every global parameter, resolved to a concrete value.
#[derive(Debug, PartialEq)]
pub(crate) struct Config {
    pub(crate) log_level: LogLevel,
    /// True when a CLI flag or env var supplied `log_level`; the Settings
    /// dialog then leaves the live subscriber alone on save.
    pub(crate) log_level_overridden: bool,
    pub(crate) control_port: u16,
    /// True when a CLI flag or env var supplied `control_port`; the Settings
    /// dialog then leaves the live listener alone on save.
    pub(crate) control_port_overridden: bool,
    pub(crate) assets_url: String,
    pub(crate) toolbar_icons_only: bool,
    /// True when a CLI flag or env var supplied `toolbar_icons_only`; the
    /// Settings dialog then leaves the live value alone on save.
    pub(crate) toolbar_icons_only_overridden: bool,
    pub(crate) status_bar_icons_only: bool,
    /// `toolbar_icons_only_overridden`'s counterpart for `status_bar_icons_only`.
    pub(crate) status_bar_icons_only_overridden: bool,
    pub(crate) welcome_image_cycle: bool,
    /// `toolbar_icons_only_overridden`'s counterpart for `welcome_image_cycle`.
    pub(crate) welcome_image_cycle_overridden: bool,
    pub(crate) welcome_image_cycle_secs: NonZeroU32,
    /// `toolbar_icons_only_overridden`'s counterpart for `welcome_image_cycle_secs`.
    pub(crate) welcome_image_cycle_secs_overridden: bool,
    pub(crate) welcome_image_shuffle: bool,
    /// `toolbar_icons_only_overridden`'s counterpart for `welcome_image_shuffle`.
    pub(crate) welcome_image_shuffle_overridden: bool,
    pub(crate) manager_sort: ManagerSort,
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
    let log_level_overridden = cli.log_level.is_some();
    let control_port_overridden = cli.control_port.is_some();
    let toolbar_icons_only_overridden = cli.toolbar_icons_only.is_some();
    let status_bar_icons_only_overridden = cli.status_bar_icons_only.is_some();
    let welcome_image_cycle_overridden = cli.welcome_image_cycle.is_some();
    let welcome_image_cycle_secs_overridden = cli.welcome_image_cycle_secs.is_some();
    let welcome_image_shuffle_overridden = cli.welcome_image_shuffle.is_some();
    Config {
        log_level: cli
            .log_level
            .or(file.log_level)
            .unwrap_or(DEFAULT_LOG_LEVEL),
        log_level_overridden,
        control_port: cli
            .control_port
            .or(file.control_port)
            .unwrap_or(crate::control::DEFAULT_PORT),
        control_port_overridden,
        assets_url: cli
            .assets_url
            .or(file.assets_url)
            .unwrap_or_else(|| crate::startup::DEFAULT_ASSETS_URL.to_string()),
        toolbar_icons_only: cli
            .toolbar_icons_only
            .or(file.toolbar_icons_only)
            .unwrap_or(DEFAULT_TOOLBAR_ICONS_ONLY),
        toolbar_icons_only_overridden,
        status_bar_icons_only: cli
            .status_bar_icons_only
            .or(file.status_bar_icons_only)
            .unwrap_or(DEFAULT_STATUS_BAR_ICONS_ONLY),
        status_bar_icons_only_overridden,
        welcome_image_cycle: cli
            .welcome_image_cycle
            .or(file.welcome_image_cycle)
            .unwrap_or(DEFAULT_WELCOME_IMAGE_CYCLE),
        welcome_image_cycle_overridden,
        welcome_image_cycle_secs: cli
            .welcome_image_cycle_secs
            .or(file.welcome_image_cycle_secs)
            .unwrap_or(DEFAULT_WELCOME_IMAGE_CYCLE_SECS),
        welcome_image_cycle_secs_overridden,
        welcome_image_shuffle: cli
            .welcome_image_shuffle
            .or(file.welcome_image_shuffle)
            .unwrap_or(DEFAULT_WELCOME_IMAGE_SHUFFLE),
        welcome_image_shuffle_overridden,
        manager_sort: file.manager_sort.unwrap_or(DEFAULT_MANAGER_SORT),
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

# draw every iconed VM status-bar entry as icon-only, readout moved into hover text
# status_bar_icons_only = {status_bar_icons_only}

# change the manager's welcome image on a timer
# welcome_image_cycle = {welcome_image_cycle}

# seconds between welcome-image changes (at least 1); only read while welcome_image_cycle is true
# welcome_image_cycle_secs = {welcome_image_cycle_secs}

# pick each next welcome image at random instead of in file-name order; only read while welcome_image_cycle is true
# welcome_image_shuffle = {welcome_image_shuffle}

# created-desc | created-asc | name-asc | name-desc
# manager_sort = \"{manager_sort}\"
",
        control_port = crate::control::DEFAULT_PORT,
        assets_url = crate::startup::DEFAULT_ASSETS_URL,
        toolbar_icons_only = DEFAULT_TOOLBAR_ICONS_ONLY,
        status_bar_icons_only = DEFAULT_STATUS_BAR_ICONS_ONLY,
        welcome_image_cycle = DEFAULT_WELCOME_IMAGE_CYCLE,
        welcome_image_cycle_secs = DEFAULT_WELCOME_IMAGE_CYCLE_SECS,
        welcome_image_shuffle = DEFAULT_WELCOME_IMAGE_SHUFFLE,
        manager_sort = manager_sort_name(DEFAULT_MANAGER_SORT),
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
/// re-serializing from scratch, so the user's comments survive. Each key
/// is set when `file` names it, or removed so it keeps tracking
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
    set_or_remove(
        &mut doc,
        "status_bar_icons_only",
        file.status_bar_icons_only,
    );
    set_or_remove(&mut doc, "welcome_image_cycle", file.welcome_image_cycle);
    set_or_remove(
        &mut doc,
        "welcome_image_cycle_secs",
        file.welcome_image_cycle_secs.map(|s| i64::from(s.get())),
    );
    set_or_remove(
        &mut doc,
        "welcome_image_shuffle",
        file.welcome_image_shuffle,
    );
    set_or_remove(
        &mut doc,
        "manager_sort",
        file.manager_sort.map(manager_sort_name),
    );

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let tmp_path = path.with_extension("toml.tmp");
    fs::write(&tmp_path, doc.to_string()).map_err(|e| format!("{}: {e}", tmp_path.display()))?;
    fs::rename(&tmp_path, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Updates only the manager sort preference while preserving every other
/// parsed setting and the file's comments.
pub(crate) fn save_manager_sort(path: &Path, order: ManagerSort) -> Result<(), String> {
    let mut file = load(Some(path))?;
    file.manager_sort = Some(order);
    save_file(path, &file)
}

fn manager_sort_name(order: ManagerSort) -> &'static str {
    match order {
        ManagerSort::CreatedDesc => "created-desc",
        ManagerSort::CreatedAsc => "created-asc",
        ManagerSort::NameAsc => "name-asc",
        ManagerSort::NameDesc => "name-desc",
    }
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
