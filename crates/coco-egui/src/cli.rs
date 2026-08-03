use clap::{Parser, ValueEnum};
use coco_core::{MachineVariant, MemorySize, VDGVariant};
use tracing_subscriber::filter::LevelFilter;

/// Short label for the window title.
pub(crate) const fn machine_label(variant: MachineVariant) -> &'static str {
    match variant {
        MachineVariant::Coco1 => "CoCo 1",
        MachineVariant::Coco2 => "CoCo 2",
        MachineVariant::Coco3 => "CoCo 3",
    }
}

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

/// Per-variant default RAM size, used by the VM manager's "New…" dialog
/// (`docs/coco12-plan.md` Phase 5).
pub(crate) fn default_ram(variant: MachineVariant) -> MemorySize {
    match variant {
        MachineVariant::Coco3 => MemorySize::K512,
        MachineVariant::Coco1 | MachineVariant::Coco2 => MemorySize::K64,
    }
}

/// Per-variant default VDG chip when no explicit choice is made: the T1
/// (CoCo 2B) on a CoCo 2, the plain MC6847 on a CoCo 1 (the only choice
/// `MachineConfig::validate` accepts there), and `None` on a CoCo 3, which
/// has no VDG at all. Shared by `new_vm.rs`'s `constrain` and
/// `machine_def.rs`'s `to_machine_config`.
pub(crate) const fn default_vdg(variant: MachineVariant) -> Option<VDGVariant> {
    match variant {
        MachineVariant::Coco2 => Some(VDGVariant::MC6847T1),
        MachineVariant::Coco1 => Some(VDGVariant::MC6847),
        MachineVariant::Coco3 => None,
    }
}

#[cfg(test)]
#[path = "cli_test.rs"]
mod tests;
