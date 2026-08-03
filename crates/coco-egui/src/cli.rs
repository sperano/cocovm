use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use coco_core::{MachineVariant, MemorySize, MonitorType, VDGVariant, VideoStandard};
use tracing_subscriber::filter::LevelFilter;

/// `clap` value parser for `--machine`.
pub(crate) fn parse_machine(s: &str) -> Result<MachineVariant, String> {
    match s {
        "coco1" => Ok(MachineVariant::Coco1),
        "coco2" => Ok(MachineVariant::Coco2),
        "coco3" => Ok(MachineVariant::Coco3),
        _ => Err(format!(
            "unknown machine '{s}' (expected coco1, coco2, or coco3)"
        )),
    }
}

/// Short label for the window title.
pub(crate) const fn machine_label(variant: MachineVariant) -> &'static str {
    match variant {
        MachineVariant::Coco1 => "CoCo 1",
        MachineVariant::Coco2 => "CoCo 2",
        MachineVariant::Coco3 => "CoCo 3",
    }
}

/// `clap` value parser for `--ram`. Accepts every [`MemorySize`] spelling
/// across both machine families (`docs/coco12-plan.md`) —
/// [`MachineConfig::validate`] rejects the wrong family for the chosen
/// `--machine`.
pub(crate) fn parse_ram(s: &str) -> Result<MemorySize, String> {
    match s {
        "4k" => Ok(MemorySize::K4),
        "16k" => Ok(MemorySize::K16),
        "32k" => Ok(MemorySize::K32),
        "64k" => Ok(MemorySize::K64),
        "128k" => Ok(MemorySize::K128),
        "512k" => Ok(MemorySize::K512),
        "2048k" => Ok(MemorySize::K2048),
        _ => Err(format!(
            "unknown RAM size '{s}' (expected 4k, 16k, 32k, 64k, 128k, 512k, or 2048k)"
        )),
    }
}

/// `clap` value parser for `--video`.
pub(crate) fn parse_video(s: &str) -> Result<VideoStandard, String> {
    match s {
        "ntsc" => Ok(VideoStandard::NTSC),
        "pal" => Ok(VideoStandard::PAL),
        _ => Err(format!(
            "unknown video standard '{s}' (expected ntsc or pal)"
        )),
    }
}

/// Composite vs RGB monitor cable. Mirrors [`MonitorType`].
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum MonitorArg {
    RGB,
    #[value(name = "cmp", alias = "composite")]
    Composite,
}

impl From<MonitorArg> for MonitorType {
    fn from(m: MonitorArg) -> Self {
        match m {
            MonitorArg::RGB => MonitorType::RGB,
            MonitorArg::Composite => MonitorType::Composite,
        }
    }
}

#[derive(Parser)]
#[command(name = "coco", version, about = "A Tandy Color Computer emulator")]
pub(crate) struct Cli {
    /// Which machine to emulate (coco1, coco2, coco3).
    #[arg(long, default_value = "coco3", value_parser = parse_machine)]
    pub(crate) machine: MachineVariant,

    /// Boot ROM image. Defaults, per `--machine`, to `roms/coco3.rom` (CoCo
    /// 3) or a flat image composed from `roms/bas1{0,1,2,3}.rom` +
    /// `roms/extbas1{0,1}.rom` (CoCo 1/2 — see `docs/coco12-plan.md`). When
    /// given explicitly for CoCo 1/2, must already be that same pre-composed
    /// flat layout (extbas at offset 0, Color BASIC at offset $2000).
    #[arg(long, value_name = "PATH")]
    pub(crate) rom: Option<PathBuf>,

    /// Cartridge ROM pak to insert at boot (`.rom`/`.ccc`/`.bin`). Without
    /// --mpi this plugs directly into the cartridge port (conflicts with
    /// --disk0/--disk1/--fd502, which also want that port); with --mpi it
    /// goes into slot 1 instead, alongside the FD-502 in slot 4.
    #[arg(long, value_name = "PATH")]
    pub(crate) cart: Option<PathBuf>,

    /// Floppy image for drive 0 (`.dsk`/`.jvc`/`.os9`); implies the FD-502
    /// disk controller (`roms/disk11.rom`), in the cartridge slot directly or
    /// (with --mpi) in slot 4.
    #[arg(long, value_name = "PATH")]
    pub(crate) disk0: Option<PathBuf>,

    /// Floppy image for drive 1 (see `--disk0`).
    #[arg(long, value_name = "PATH")]
    pub(crate) disk1: Option<PathBuf>,

    /// VHD (virtual hard disk) image for drive 0, for NitrOS-9's `emudsk`
    /// driver. A bus-level device ($FF80-$FF86) independent of the cartridge
    /// slot, so unlike --disk0/--disk1 this doesn't conflict with --cart.
    #[arg(long, value_name = "PATH")]
    pub(crate) vhd0: Option<PathBuf>,

    /// VHD image for drive 1 (see `--vhd0`).
    #[arg(long, value_name = "PATH")]
    pub(crate) vhd1: Option<PathBuf>,

    /// Enable the Becker port ($FF41/$FF42) for DriveWire disk access,
    /// hosting up to 4 virtual disk images. Implied by any --dw0/--dw1/--dw2/--dw3.
    #[arg(long, default_value_t = false)]
    pub(crate) becker: bool,

    /// DriveWire disk image for drive 0 (`.dsk`/`.os9`/`.img`/`.vhd`); implies
    /// --becker. A bus-level device independent of the cartridge slot, like VHD.
    #[arg(long, value_name = "PATH")]
    pub(crate) dw0: Option<PathBuf>,

    /// DriveWire disk image for drive 1 (see `--dw0`).
    #[arg(long, value_name = "PATH")]
    pub(crate) dw1: Option<PathBuf>,

    /// DriveWire disk image for drive 2 (see `--dw0`).
    #[arg(long, value_name = "PATH")]
    pub(crate) dw2: Option<PathBuf>,

    /// DriveWire disk image for drive 3 (see `--dw0`).
    #[arg(long, value_name = "PATH")]
    pub(crate) dw3: Option<PathBuf>,

    /// Enable HDB-DOS sector addressing mode on the DriveWire server (implies
    /// --becker): flat addressing for DECB format images instead of per-drive
    /// LSNs.
    #[arg(long, default_value_t = false)]
    pub(crate) hdbdos: bool,

    /// Insert the FD-502 disk controller with empty drives, so Disk BASIC
    /// boots and blank disks can be added (and DSKINI'd) from the menu.
    /// Implied by --disk0/--disk1.
    #[arg(long, default_value_t = false)]
    pub(crate) fd502: bool,

    /// Insert the Tandy Sound/Speech Cartridge. Without --mpi this plugs
    /// directly into the cartridge port (conflicts with
    /// --cart/--disk0/--disk1/--fd502/--rtc, which also want that port);
    /// with --mpi it goes into slot 2, alongside --cart in slot 1, --rtc in
    /// slot 3, and the FD-502 in slot 4.
    #[arg(long, default_value_t = false)]
    pub(crate) ssc: bool,

    /// Insert a 4-slot Tandy Multi-Pak Interface into the cartridge port
    /// instead of plugging --cart/--disk*/--fd502/--rtc directly into it:
    /// --cart goes into slot 1, the FD-502 (implied by --disk0/--disk1/
    /// --fd502) into slot 4, and the RTC into slot 3 — the conventional
    /// real-world layout (also MAME's default), letting a cartridge, the
    /// disk controller, and the clock coexist.
    #[arg(long, default_value_t = false)]
    pub(crate) mpi: bool,

    /// Insert a Disto real-time clock (OKI MSM6242 at $FF50-$FF53, for
    /// NitrOS-9's clock2_disto drivers), running on the host's local clock —
    /// directly in the cartridge port, or (with --mpi) in slot 3.
    #[arg(long, default_value_t = false)]
    pub(crate) rtc: bool,

    /// Installed RAM (4k, 16k, 32k, 64k, 128k, 512k, 2048k). Defaults, per
    /// `--machine`, to 512K (CoCo 3) or 64K (CoCo 1/2).
    #[arg(long, value_parser = parse_ram)]
    pub(crate) ram: Option<MemorySize>,

    /// Master video standard (crystal), independent of the GIME 50/60 Hz mode
    /// bit (ntsc or pal).
    #[arg(long, default_value = "ntsc", value_parser = parse_video)]
    pub(crate) video: VideoStandard,

    /// Composite vs RGB monitor cable (CoCo 3 only — a CoCo 1/2 has no
    /// monitor port, just RF out to a TV). Real hardware drives both
    /// signals simultaneously; this picks which one the emulated monitor
    /// decodes (also toggleable live from the View menu). Defaults to RGB
    /// on a CoCo 3.
    #[arg(long, value_enum)]
    pub(crate) monitor: Option<MonitorArg>,

    /// Also save a `.wav` of the tape audio alongside the canonical `.cas`
    /// on every tape write-back (see the "Also save tape audio (.wav)"
    /// Machine-menu checkbox, which this just sets the initial value of).
    #[arg(long, default_value_t = false)]
    pub(crate) tape_wav: bool,

    /// Start "print to text file" capture at this path as soon as the
    /// machine boots (create/truncate — see the Machine menu's "Start Print
    /// Capture…", which this is the CLI equivalent of).
    #[arg(long, value_name = "PATH")]
    pub(crate) print_capture: Option<PathBuf>,

    /// Boot straight into a saved state (`.ccstate`, see the Machine menu's
    /// "Save State…"/"Load State…"): applied last, after every other flag
    /// above has built and mounted its own machine — the snapshot's own
    /// config and media then replace it wholesale, so `--machine`/`--ram`/
    /// `--cart`/etc. only matter for a fresh boot without `--state`.
    #[arg(long, value_name = "PATH")]
    pub(crate) state: Option<PathBuf>,

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

/// Per-variant default RAM size when `--ram` isn't given
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
/// has no VDG at all. Shared by the CLI path below, `new_vm.rs`'s
/// `constrain`, and `machine_def.rs`'s `to_machine_config`.
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
