use clap::{Parser, ValueEnum};
use tracing_subscriber::filter::LevelFilter;

/// The app's whole CLI surface: the manager window is always what runs (see
/// `main.rs`). Besides the log level, the other flags are the control
/// listener's port, the asset bundle's URL, and the toolbar's caption toggle.
/// Every field is `Option`: `None` means "not given here", so `config::resolve`
/// can fall through to `config.toml` and then the built-in default
/// (`config.rs`'s precedence chain). clap's own `env` fallback already
/// prefers a flag over the environment variable, so a `Some` here already
/// carries the flag-beats-env half of that chain for free.
#[derive(Parser)]
#[command(name = "coco", version, about = "A Tandy Color Computer emulator")]
pub(crate) struct Cli {
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
