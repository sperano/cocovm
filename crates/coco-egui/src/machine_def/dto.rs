//! The TOML-facing DTO (Data Transfer Object) types for [`super::MachineDef`]:
//! one enum/struct per section/field that needs its own serde shape rather
//! than reusing a `coco-core` type directly (see the parent module doc for
//! why). Each hardware-facing enum carries a pair of `From` impls to/from its
//! `coco-core` counterpart.

use coco_core::{MachineVariant, MemorySize, MonitorType, VDGVariant, VideoStandard};
use serde::{Deserialize, Serialize};

use coco_core::MachineConfig;

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
pub enum RamDTO {
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

impl From<MemorySize> for RamDTO {
    fn from(memory: MemorySize) -> Self {
        match memory {
            MemorySize::K4 => RamDTO::K4,
            MemorySize::K16 => RamDTO::K16,
            MemorySize::K32 => RamDTO::K32,
            MemorySize::K64 => RamDTO::K64,
            MemorySize::K128 => RamDTO::K128,
            MemorySize::K512 => RamDTO::K512,
            MemorySize::K2048 => RamDTO::K2048,
        }
    }
}

impl From<RamDTO> for MemorySize {
    fn from(ram: RamDTO) -> Self {
        match ram {
            RamDTO::K4 => MemorySize::K4,
            RamDTO::K16 => MemorySize::K16,
            RamDTO::K32 => MemorySize::K32,
            RamDTO::K64 => MemorySize::K64,
            RamDTO::K128 => MemorySize::K128,
            RamDTO::K512 => MemorySize::K512,
            RamDTO::K2048 => MemorySize::K2048,
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

/// `[hardware]` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareDTO {
    pub variant: MachineVariantDTO,
    pub ram: RamDTO,
    pub video: VideoStandardDTO,
    /// Absent ⇒ per-variant default: RGB on a CoCo 3, nothing on a CoCo 1/2
    /// (no monitor port — RF TV only; an explicit key there fails
    /// [`MachineConfig::validate`]).
    #[serde(default)]
    pub monitor: Option<MonitorDTO>,
    /// Absent ⇒ per-variant default; see [`VDGVariantDTO`].
    #[serde(default)]
    pub vdg: Option<VDGVariantDTO>,
    /// Absent ⇒ default ROM composition (`rom_db`/`load_default_rom`).
    #[serde(default)]
    pub rom: Option<String>,
}

impl HardwareDTO {
    /// Build the `[hardware]` section from a config the "New…" dialog or the
    /// manager's detail-pane form produced (`new_vm::config_form_rows`
    /// already ran [`MachineConfig::validate`]-compatible constraints on
    /// it). `monitor`/`vdg` are written exactly when the machine has the
    /// port/chip (`Some` per the config); a CoCo 1/2 file carries no
    /// `monitor` key and a CoCo 3 file no `vdg` key. `rom` is passed
    /// through as-is: the custom-ROM path isn't part of [`MachineConfig`]
    /// and has no editor yet.
    pub fn from_config(config: &MachineConfig, rom: Option<String>) -> Self {
        Self {
            variant: config.variant.into(),
            ram: config.memory.into(),
            video: config.video.into(),
            monitor: config.monitor.map(Into::into),
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
}

/// Default for `[ui].aspect_correct` — `bool::default()` is `false`, but the
/// file-format default is `true` (aspect correction on), so this needs its
/// own default function rather than relying on `Default::default()`.
fn default_aspect_correct() -> bool {
    true
}

/// `[ui]` section — section itself optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UIDTO {
    #[serde(default = "default_aspect_correct")]
    pub aspect_correct: bool,
    #[serde(default)]
    pub kb_mode: KbModeDTO,
}

impl Default for UIDTO {
    fn default() -> Self {
        Self {
            aspect_correct: true,
            kb_mode: KbModeDTO::default(),
        }
    }
}
