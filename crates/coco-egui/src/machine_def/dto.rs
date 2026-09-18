//! The TOML-facing DTO (Data Transfer Object) types for [`super::MachineDef`]:
//! one enum/struct per section/field that needs its own serde shape rather
//! than reusing a `coco-core` type directly (see the parent module doc for
//! why). Each hardware-facing enum carries a pair of `From` impls to/from its
//! `coco-core` counterpart.

use coco_core::{MachineVariant, MemorySize, MonitorType, VDGVariant, VideoStandard};
use serde::{Deserialize, Serialize};

use coco_core::MachineConfig;

use crate::display::{Display, TV, TVSettings};

/// `[hardware].variant`. Maps to [`coco_core::MachineVariant`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MachineVariantDTO {
    #[serde(rename = "coco1")]
    Coco1,
    #[serde(rename = "coco2")]
    Coco2,
    #[serde(rename = "coco3")]
    Coco3,
}

impl From<MachineVariant> for MachineVariantDTO {
    fn from(variant: MachineVariant) -> Self {
        match variant {
            MachineVariant::Coco1 => MachineVariantDTO::Coco1,
            MachineVariant::Coco2 => MachineVariantDTO::Coco2,
            MachineVariant::Coco3 => MachineVariantDTO::Coco3,
        }
    }
}

impl From<MachineVariantDTO> for MachineVariant {
    fn from(variant: MachineVariantDTO) -> Self {
        match variant {
            MachineVariantDTO::Coco1 => MachineVariant::Coco1,
            MachineVariantDTO::Coco2 => MachineVariant::Coco2,
            MachineVariantDTO::Coco3 => MachineVariant::Coco3,
        }
    }
}

/// `[hardware].ram`. Maps to [`coco_core::MemorySize`]; variant names like
/// `512k` aren't valid Rust identifiers, hence the explicit renames rather
/// than a derived `rename_all`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RAMDTO {
    #[serde(rename = "4k")]
    K4,
    #[serde(rename = "16k")]
    K16,
    #[serde(rename = "32k")]
    K32,
    #[serde(rename = "64k")]
    K64,
    #[serde(rename = "128k")]
    K128,
    #[serde(rename = "512k")]
    K512,
    #[serde(rename = "2048k")]
    K2048,
}

impl From<MemorySize> for RAMDTO {
    fn from(memory: MemorySize) -> Self {
        match memory {
            MemorySize::K4 => RAMDTO::K4,
            MemorySize::K16 => RAMDTO::K16,
            MemorySize::K32 => RAMDTO::K32,
            MemorySize::K64 => RAMDTO::K64,
            MemorySize::K128 => RAMDTO::K128,
            MemorySize::K512 => RAMDTO::K512,
            MemorySize::K2048 => RAMDTO::K2048,
        }
    }
}

impl From<RAMDTO> for MemorySize {
    fn from(ram: RAMDTO) -> Self {
        match ram {
            RAMDTO::K4 => MemorySize::K4,
            RAMDTO::K16 => MemorySize::K16,
            RAMDTO::K32 => MemorySize::K32,
            RAMDTO::K64 => MemorySize::K64,
            RAMDTO::K128 => MemorySize::K128,
            RAMDTO::K512 => MemorySize::K512,
            RAMDTO::K2048 => MemorySize::K2048,
        }
    }
}

/// `[hardware].video`. Maps to [`coco_core::VideoStandard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoStandardDTO {
    #[serde(rename = "ntsc")]
    NTSC,
    #[serde(rename = "pal")]
    PAL,
}

impl From<VideoStandard> for VideoStandardDTO {
    fn from(video: VideoStandard) -> Self {
        match video {
            VideoStandard::NTSC => VideoStandardDTO::NTSC,
            VideoStandard::PAL => VideoStandardDTO::PAL,
        }
    }
}

impl From<VideoStandardDTO> for VideoStandard {
    fn from(video: VideoStandardDTO) -> Self {
        match video {
            VideoStandardDTO::NTSC => VideoStandard::NTSC,
            VideoStandardDTO::PAL => VideoStandard::PAL,
        }
    }
}

/// `[hardware].monitor`. Maps to [`coco_core::gime::MonitorType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MonitorDTO {
    #[serde(rename = "rgb")]
    RGB,
    #[serde(rename = "composite")]
    Composite,
}

impl From<MonitorType> for MonitorDTO {
    fn from(monitor: MonitorType) -> Self {
        match monitor {
            MonitorType::RGB => MonitorDTO::RGB,
            MonitorType::Composite => MonitorDTO::Composite,
        }
    }
}

impl From<MonitorDTO> for MonitorType {
    fn from(monitor: MonitorDTO) -> Self {
        match monitor {
            MonitorDTO::RGB => MonitorType::RGB,
            MonitorDTO::Composite => MonitorType::Composite,
        }
    }
}

/// `[hardware].display`. Maps to [`crate::display::Display`] — the display
/// device on the video cable (monitor or color/B&W TV, `display.rs`).
/// Supersedes [`MonitorDTO`]'s `monitor` key: `display` wins when both are
/// present, and saves write only `display`
/// ([`super::MachineDef::display`] / [`HardwareDTO::from_config`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisplayDTO {
    #[serde(rename = "rgb")]
    RGB,
    /// Alias: the legacy `monitor` key spelled it `"composite"`, and the
    /// CLI accepts both — a hand-migrated file must too.
    #[serde(rename = "cmp", alias = "composite")]
    Composite,
    #[serde(rename = "tv", alias = "tv-color")]
    TVColor,
    #[serde(rename = "tv-bw")]
    TVBW,
}

impl From<Display> for DisplayDTO {
    fn from(display: Display) -> Self {
        match display {
            Display::Monitor(MonitorType::RGB) => DisplayDTO::RGB,
            Display::Monitor(MonitorType::Composite) => DisplayDTO::Composite,
            Display::TV(TV::Color) => DisplayDTO::TVColor,
            Display::TV(TV::BW) => DisplayDTO::TVBW,
        }
    }
}

impl From<DisplayDTO> for Display {
    fn from(display: DisplayDTO) -> Self {
        match display {
            DisplayDTO::RGB => Display::Monitor(MonitorType::RGB),
            DisplayDTO::Composite => Display::Monitor(MonitorType::Composite),
            DisplayDTO::TVColor => Display::TV(TV::Color),
            DisplayDTO::TVBW => Display::TV(TV::BW),
        }
    }
}

/// `[hardware].vdg`. Maps to [`coco_core::VDGVariant`]. Optional in the file
/// — when absent, [`super::MachineDef::to_machine_config`] defaults it per variant
/// the same way `new_vm.rs`'s `constrain` does: the T1 (CoCo 2B) on a
/// CoCo 2, the plain MC6847 elsewhere (the only choice
/// `MachineConfig::validate` accepts there).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VDGVariantDTO {
    #[serde(rename = "mc6847")]
    MC6847,
    #[serde(rename = "mc6847t1")]
    MC6847T1,
}

impl From<VDGVariant> for VDGVariantDTO {
    fn from(vdg: VDGVariant) -> Self {
        match vdg {
            VDGVariant::MC6847 => VDGVariantDTO::MC6847,
            VDGVariant::MC6847T1 => VDGVariantDTO::MC6847T1,
        }
    }
}

impl From<VDGVariantDTO> for VDGVariant {
    fn from(vdg: VDGVariantDTO) -> Self {
        match vdg {
            VDGVariantDTO::MC6847 => VDGVariant::MC6847,
            VDGVariantDTO::MC6847T1 => VDGVariant::MC6847T1,
        }
    }
}

/// `[ui].kb_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum KbModeDTO {
    #[default]
    #[serde(rename = "positional")]
    Positional,
    #[serde(rename = "symbolic")]
    Symbolic,
}

/// `[ui].joy_left` / `[ui].joy_right`. Maps to `crate::joy::JoySource` (the
/// `From` impls live in `joy.rs`, alongside that enum, rather than here —
/// see its doc comment for why). Serialized lowercase, matching
/// [`SerialDTO`]'s convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum JoySourceDTO {
    #[default]
    None,
    Mouse,
    Gamepad,
    Keys,
}

/// `[hardware]` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareDTO {
    pub variant: MachineVariantDTO,
    pub ram: RAMDTO,
    pub video: VideoStandardDTO,
    /// Legacy display key, read but never written since `display` (following)
    /// superseded it: `display` wins when both are present; alone, it maps
    /// to the monitor half of [`crate::display::Display`]
    /// ([`super::MachineDef::display`]). An explicit key on a CoCo 1/2
    /// still fails [`MachineConfig::validate`] (no monitor port).
    #[serde(default)]
    pub monitor: Option<MonitorDTO>,
    /// Absent (and no legacy `monitor` either) ⇒ per-variant default: an
    /// RGB monitor on a CoCo 3, a color TV on a CoCo 1/2
    /// ([`crate::display::Display::default_for`]).
    #[serde(default)]
    pub display: Option<DisplayDTO>,
    /// Absent ⇒ per-variant default; see [`VDGVariantDTO`].
    #[serde(default)]
    pub vdg: Option<VDGVariantDTO>,
    /// Absent ⇒ default ROM composition (`rom_db`/`load_default_rom`).
    #[serde(default)]
    pub rom: Option<String>,
}

impl HardwareDTO {
    /// Build the `[hardware]` section from a config the "New…" flow or
    /// detail-pane form produced. `display` is taken separately since it's
    /// not derivable from the config (a CoCo 3 TV and composite monitor both
    /// resolve to `Composite`).
    pub fn from_config(config: &MachineConfig, display: Display, rom: Option<String>) -> Self {
        Self {
            variant: config.variant.into(),
            ram: config.memory.into(),
            video: config.video.into(),
            monitor: None,
            display: Some(display.into()),
            vdg: config.vdg.map(Into::into),
            rom,
        }
    }
}

/// `[media]` section — every key optional, the section itself optional.
/// Relative paths are meant to resolve against the machine's artifact
/// directory (`data_dir()/machines/<slug>`), never embedded. The cartridge
/// port's own ROM Pak/Games Master/Orchestra-90 image lives in
/// `[peripherals].cartridge`, not here — a cartridge is hardware, not media.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaDTO {
    #[serde(default)]
    pub disk0: Option<String>,
    #[serde(default)]
    pub disk1: Option<String>,
    #[serde(default)]
    pub vhd0: Option<String>,
    #[serde(default)]
    pub vhd1: Option<String>,
    #[serde(default)]
    pub tape: Option<String>,
}

/// `[drivewire]` cold-start settings. Disk paths use the same relative-path
/// resolution as `[media]` paths.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveWireDTO {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub hdbdos_mode: bool,
    #[serde(default)]
    pub disk0: Option<String>,
    #[serde(default)]
    pub disk1: Option<String>,
    #[serde(default)]
    pub disk2: Option<String>,
    #[serde(default)]
    pub disk3: Option<String>,
}

impl DriveWireDTO {
    pub fn disk_paths(&self) -> [Option<&str>; coco_core::drivewire::DRIVE_COUNT] {
        [
            self.disk0.as_deref(),
            self.disk1.as_deref(),
            self.disk2.as_deref(),
            self.disk3.as_deref(),
        ]
    }
}

/// `[ports].serial`. What host sink the built-in bit-banger serial port
/// (the 4-pin DIN every CoCo has — `coco_core::bitbanger::BitBanger`, not
/// the Deluxe RS-232 Pak's ACIA) starts wired to. Absent ⇒ nothing
/// attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SerialDTO {
    /// The DMP-105 dot-matrix printer, shown in the Printer Paper window.
    Printer,
    /// The DMP-130 dot-matrix printer, shown in the Printer Paper window.
    Dmp130,
    /// Plain text capture to `printout.txt` in the machine's artifact
    /// directory. The file is truncated on every launch (`FileSink::create`
    /// semantics, same as the runtime menu's Start Print Capture) — each
    /// power-on starts a fresh capture, not an appended log.
    File,
}

/// `[ports]` section — section itself optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PortsDTO {
    #[serde(default)]
    pub serial: Option<SerialDTO>,
}

/// Default for `[ui].tv_scanline`: [`TVSettings::default`]'s strength, so an
/// absent key means "the tuned look", not "scanlines off".
fn default_tv_scanline() -> u8 {
    TVSettings::default().scanline_pct
}

/// Default for `[ui].tv_noise` — [`default_tv_scanline`]'s sibling.
fn default_tv_noise() -> u8 {
    TVSettings::default().noise_pct
}

/// Default for `[ui].tv_overscan` — [`default_tv_scanline`]'s sibling.
fn default_tv_overscan() -> u8 {
    TVSettings::default().overscan_pct
}

/// `[ui]` section — section itself optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UIDTO {
    #[serde(default)]
    pub kb_mode: KbModeDTO,
    /// Absent ⇒ off, matching `JoystickInputs::new`: nothing drives a port
    /// until it's opted in — same for both ports.
    #[serde(default)]
    pub joy_left: JoySourceDTO,
    /// Absent ⇒ off; see [`Self::joy_left`].
    #[serde(default)]
    pub joy_right: JoySourceDTO,
    /// Scanline strength of the TV look, `0..=100`
    /// ([`TVSettings::scanline_pct`]). Kept even while the display is a
    /// monitor — switching back to a TV restores the tuned strength.
    #[serde(default = "default_tv_scanline")]
    pub tv_scanline: u8,
    /// RF-noise amount of the TV look, `0..=100`
    /// ([`TVSettings::noise_pct`]); kept while on a monitor like
    /// [`Self::tv_scanline`].
    #[serde(default = "default_tv_noise")]
    pub tv_noise: u8,
    /// Centered TV overscan crop per edge
    /// ([`TVSettings::overscan_pct`]); kept while on a monitor like
    /// [`Self::tv_scanline`], and clamped to the slider range at launch.
    #[serde(default = "default_tv_overscan")]
    pub tv_overscan: u8,
}

impl Default for UIDTO {
    fn default() -> Self {
        Self {
            kb_mode: KbModeDTO::default(),
            joy_left: JoySourceDTO::default(),
            joy_right: JoySourceDTO::default(),
            tv_scanline: default_tv_scanline(),
            tv_noise: default_tv_noise(),
            tv_overscan: default_tv_overscan(),
        }
    }
}

/// `[stats]` section — read-only usage statistics the manager maintains
/// itself (never edited through the form): cumulative powered-on time and the
/// count of times this machine has been launched from Powered Off (Resume
/// doesn't count — see `manager::lifecycle::resume_vm`'s doc). Section
/// always present, like `[ui]`/`[peripherals]`; absent in a legacy file it
/// defaults to zero.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatsDTO {
    /// Cumulative powered-on runtime, in whole seconds — folded in from a
    /// live VM's session runtime on Suspend, Stop, and quit
    /// (`manager::lifecycle::fold_runtime_into_def`).
    #[serde(default)]
    pub runtime_secs: u64,
    /// Number of times this machine has been launched from Powered Off
    /// (`manager::lifecycle::start_vm`).
    #[serde(default)]
    pub starts: u32,
}
