use clap::{Parser, ValueEnum};
use tracing_subscriber::filter::LevelFilter;

/// The app's whole CLI surface: the manager window is always what runs (see
/// `main.rs`), so the only flag left is the one that has to be known before
/// the log subscriber is installed.
#[derive(Parser)]
#[command(name = "coco", version, about = "A Tandy Color Computer emulator")]
pub(crate) struct Cli {
    /// Lowest log level to print. `RUST_LOG` overrides this when set — it
    /// also filters per module, which a bare level cannot express (e.g.
    /// `RUST_LOG=info,eframe=warn`).
    #[arg(
        short = 'L',
        long,
        value_enum,
        env = "COCOVM_LOG_LEVEL",
        default_value_t = LogLevel::Warn
    )]
    pub(crate) log_level: LogLevel,
}

/// `--log-level`, the CLI's spelling of a [`LevelFilter`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
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
