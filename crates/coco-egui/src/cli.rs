use std::num::NonZeroU32;

use clap::{Parser, ValueEnum};
use tracing_subscriber::filter::LevelFilter;

/// The app's whole CLI surface. The manager window is what runs (see
/// `main.rs`); naming a machine's slug starts that machine's VM as the
/// manager opens, so a saved machine can be launched without finding its row
/// in the list. The rest are the log level, the control listener's port, the
/// asset bundle's URL, the toolbar and status-bar caption toggles, the
/// welcome-image cycle, and the startup update check.
/// Every settings field is `Option`: `None` means "not given here", so
/// `config::resolve` can fall through to `config.toml` and then the built-in
/// default (`config.rs`'s precedence chain). clap's own `env` fallback already
/// prefers a flag over the environment variable, so a `Some` here already
/// carries the flag-beats-env half of that chain for free.
#[derive(Parser)]
#[command(name = "coco", version, about = "A Tandy Color Computer emulator")]
pub(crate) struct Cli {
    /// Slug of the saved machine to start once the manager opens. The slug
    /// is the `<slug>.toml` file's stem under `config_dir()/machines` (the
    /// manager shows it under the Name field). Omitted, nothing starts.
    #[arg(value_name = "SLUG")]
    pub(crate) machine: Option<String>,

    /// Lowest log level to print. `RUST_LOG` overrides this when set — it
    /// also filters per module, which a bare level cannot express (for example,
    /// `RUST_LOG=info,eframe=warn`).
    #[arg(short = 'L', long, value_enum, env = "COCOVM_LOG_LEVEL")]
    pub(crate) log_level: Option<LogLevel>,

    /// Loopback port the built-in MCP server listens on, at
    /// `http://127.0.0.1:<port>/mcp` (`crate::control`). `0` disables it
    /// entirely.
    #[arg(long, env = crate::control::PORT_ENV)]
    pub(crate) control_port: Option<u16>,

    /// URL of the first-run asset bundle (a gzipped tar of the ROM and
    /// image directories — `manager/assets.rs`).
    #[arg(long, env = "COCOVM_ASSETS_URL")]
    pub(crate) assets_url: Option<String>,

    /// Draw every toolbar tile (manager and VM window) as icon-only, with
    /// its caption moved into the hover text. Bare `--toolbar-icons-only`
    /// means `true`; `--toolbar-icons-only=false` can override a
    /// `config.toml` (or env) `true`. `BoolishValueParser`: also accepts
    /// `yes`/`no`, `y`/`n`, `on`/`off`, `1`/`0` — the dominant spelling for
    /// an env-var boolean (`COCOVM_TOOLBAR_ICONS_ONLY=1`), and the pattern
    /// for any future boolean setting.
    #[arg(
        long,
        env = "COCOVM_TOOLBAR_ICONS_ONLY",
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub(crate) toolbar_icons_only: Option<bool>,

    /// Draw every iconed VM status-bar entry as its icon alone, with the readout
    /// moved into the hover text. Same flag grammar as `--toolbar-icons-only`.
    #[arg(
        long,
        env = "COCOVM_STATUS_BAR_ICONS_ONLY",
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub(crate) status_bar_icons_only: Option<bool>,

    /// Change the manager window's welcome image every
    /// `--welcome-image-cycle-secs`. Same flag grammar as `--toolbar-icons-only`.
    #[arg(
        long,
        env = "COCOVM_WELCOME_IMAGE_CYCLE",
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub(crate) welcome_image_cycle: Option<bool>,

    /// Seconds between welcome-image changes; only read while
    /// `--welcome-image-cycle` is on. Zero is rejected.
    #[arg(long, env = "COCOVM_WELCOME_IMAGE_CYCLE_SECS")]
    pub(crate) welcome_image_cycle_secs: Option<NonZeroU32>,

    /// Pick each next welcome image at random instead of in file-name
    /// order; only read while `--welcome-image-cycle` is on. Same flag
    /// grammar as `--toolbar-icons-only`.
    #[arg(
        long,
        env = "COCOVM_WELCOME_IMAGE_SHUFFLE",
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub(crate) welcome_image_shuffle: Option<bool>,

    /// Ask GitHub for the latest release at startup and show a notice when
    /// it is newer (`update.rs`). `--check-for-updates=false` turns it off
    /// for this launch. Same flag grammar as `--toolbar-icons-only`.
    #[arg(
        long,
        env = "COCOVM_CHECK_FOR_UPDATES",
        num_args = 0..=1,
        default_missing_value = "true",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub(crate) check_for_updates: Option<bool>,
}

/// `--log-level`, the CLI's spelling of a [`LevelFilter`]. Also `config.toml`'s
/// `log_level` key (`config::FileConfig`) — `rename_all = "lowercase"` keeps
/// the TOML spelling identical to clap's own (clap lowercases `ValueEnum`
/// variant names by default).
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<LogLevel> for LevelFilter {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Error => LevelFilter::ERROR,
            LogLevel::Warn => LevelFilter::WARN,
            LogLevel::Info => LevelFilter::INFO,
            LogLevel::Debug => LevelFilter::DEBUG,
            LogLevel::Trace => LevelFilter::TRACE,
        }
    }
}

#[cfg(test)]
#[path = "cli_test.rs"]
mod tests;
