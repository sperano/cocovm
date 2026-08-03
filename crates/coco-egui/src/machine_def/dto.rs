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
    #[serde(rename = "cmp")]
    Composite,
    #[serde(rename = "tv")]
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
/// the same way `main.rs`'s CLI path and `new_vm.rs`'s `constrain_draft` do:
/// the T1 (CoCo 2B) on a CoCo 2, the plain MC6847 elsewhere (the only choice
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
    /// Legacy display key, read but never written since `display` (below)
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
    /// Build the `[hardware]` section from a config the "New…" flow or the
    /// manager's detail-pane form produced (`new_vm::config_form_rows`
    /// already ran [`MachineConfig::validate`]-compatible constraints on
    /// it). `display` is taken separately — the form's own pick, not
    /// derivable from the config (a CoCo 3 TV and a composite monitor both
    /// resolve to `monitor: Composite`); the legacy `monitor` key is never
    /// written. `vdg` is written exactly when the machine has the chip
    /// (`Some` per the config) — a CoCo 3 file carries no `vdg` key. `rom`
    /// is passed through as-is: the custom-ROM path isn't part of
    /// [`MachineConfig`] and has no editor yet.
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
/// directory (`data_dir()/machines/<slug>`), never embedded
/// (`plan-machine-persistence.md` "Media by reference, never embedded");
/// no caller resolves media paths yet (`plan-machine-persistence.md` step 5,
/// launch/media mounting — not implemented).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaDTO {
    #[serde(default)]
    pub cart: Option<String>,
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

/// `[peripherals]` section — section itself optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PeripheralsDTO {
    #[serde(default)]
    pub mpi: bool,
    #[serde(default)]
    pub rtc: bool,
    /// FD-502 disk controller. Also implied at launch by `[media]`
    /// disk0/disk1 being set, so older files without this key keep working.
    #[serde(default)]
    pub fd502: bool,
    /// Deluxe RS-232 Pak in the cartridge port. Like `rtc`/`fd502`, the
    /// schema keeps no slot layout: a slotted pak isn't representable yet —
    /// launch rejects `mpi && rs232` outright (`launch::check_cartridge_port`).
    #[serde(default)]
    pub rs232: bool,
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

/// Default for `[ui].aspect_correct` — `bool::default()` is `false`, but the
/// file-format default is `true` (aspect correction on), so this needs its
/// own default function rather than relying on `Default::default()`.
fn default_aspect_correct() -> bool {
    true
}

/// Default for `[ui].tv_scanline`: [`TVSettings::default`]'s strength, so an
/// absent key means "the tuned look", not "scanlines off".
fn default_tv_scanline() -> u8 {
    TVSettings::default().scanline_pct
}

/// `[ui]` section — section itself optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UIDTO {
    #[serde(default = "default_aspect_correct")]
    pub aspect_correct: bool,
    #[serde(default)]
    pub kb_mode: KbModeDTO,
    /// Absent ⇒ off, matching `JoystickInputs::new` (user decision
    /// 2026-07-29: nothing drives a port until it's opted in — same for
    /// both ports).
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
}

impl Default for UIDTO {
    fn default() -> Self {
        Self {
            aspect_correct: true,
            kb_mode: KbModeDTO::default(),
            joy_left: JoySourceDTO::default(),
            joy_right: JoySourceDTO::default(),
            tv_scanline: default_tv_scanline(),
        }
    }
}
