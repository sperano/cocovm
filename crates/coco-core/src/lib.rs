//! `coco-core` — the headless CoCo 3 machine: bus, GIME, MMU, PIAs, timing.
//! No UI dependencies, so it can be unit-tested and boot a ROM without a window.
//! See `DESIGN.md` §1.

pub mod acia6551;
pub mod audio;
pub mod ay8913;
pub mod bitbanger;
pub mod bus;
pub mod cart;
pub mod cassette;
pub mod cassette_wav;
pub mod config;
pub mod debug;
pub mod dmp;
pub mod dmp105;
mod dmp105_font;
pub mod dmp130;
mod dmp_charset;
mod dmp_symbols;
pub mod drivewire;
pub mod fdc;
mod font6847;
mod font_gime;
pub mod gime;
pub mod gime_video;
pub mod hires_joystick;
pub mod joystick;
pub mod keyboard;
mod machine;
pub mod orch90;
pub mod pia;
pub mod printer;
pub mod raster;
pub mod rom_db;
pub mod rs232;
pub mod rtc;
pub mod sam;
pub mod serde_util;
pub mod serial;
pub mod sn76489;
pub mod snapshot;
pub mod sp0256;
pub mod ssc;
pub mod vhd;
pub mod video;
pub mod wd1773;

pub use bus::SystemBus;
pub use config::{MachineConfig, MachineVariant, MemorySize, VDGVariant, VideoStandard};
pub use gime::{GIME, MonitorType};
pub use machine::{ActiveRect, Machine, StepEvent, StepKind};

/// The normal-speed CPU clock ([`machine`]'s private constant, re-exported
/// crate-wide so other modules can derive cycle counts from the real clock
/// instead of duplicating the value) — [`cassette::RECORD_IDLE_FINALIZE_CYCLES`].
pub(crate) use machine::CPU_HZ;
