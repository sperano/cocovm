use clap::{Parser, ValueEnum};
use tracing_subscriber::filter::LevelFilter;

/// The app's whole CLI surface: the manager window is always what runs (see
/// `main.rs`). Besides the log level, the only other flag is the control
/// listener's port.
#[derive(Parser)]
#[command(name = "coco", version, about = "A Tandy Color Computer emulator")]
pub(crate) struct Cli {
    /// Lowest log level to print. `RUST_LOG` overrides this when set — it
    /// also filters per module, which a bare level cannot express (for example,
    /// `RUST_LOG=info,eframe=warn`).
    #[arg(
        short = 'L',
        long,
        value_enum,
        env = "COCOVM_LOG_LEVEL",
        default_value_t = LogLevel::Warn
    )]
    pub(crate) log_level: LogLevel,

    /// Loopback port the control listener accepts driver connections on
    /// (`coco_control::server`). `0` disables the listener entirely.
    #[arg(long, env = coco_control::PORT_ENV, default_value_t = coco_control::DEFAULT_PORT)]
    pub(crate) control_port: u16,
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
