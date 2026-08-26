//! The `[peripherals]` section's DTO types — split out of `dto.rs` once it
//! grew past the project's ~500-line ceiling. See that module's doc for the
//! overall DTO convention.

use serde::{Deserialize, Serialize};

/// `[peripherals].cartridge` — what's plugged into the cartridge port. Maps
/// to [`crate::new_vm::CartridgeChoice`]; the `From` impls live there, like
/// [`super::SerialDTO`]'s pair in `new_vm.rs` (its doc explains why).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind")]
pub enum CartridgeDTO {
    #[default]
    #[serde(rename = "none")]
    None,
    /// FD-502 disk controller (Disk BASIC ROM + WD1773, empty drives).
    #[serde(rename = "fd502")]
    FD502,
    /// A program ROM Pak image, `path` resolved like `[media]`'s paths
    /// (`resolve_media_path`).
    #[serde(rename = "rompak")]
    ROMPak { path: String },
    /// Disto RTC. No boot ROM — pairs with a VHD boot; for RTC + floppies
    /// use an MPI slot.
    #[serde(rename = "rtc")]
    RTC,
    /// Deluxe RS-232 Pak. Bare-port only: there's no `mpi_insert_rs232`, so
    /// this kind never appears in [`PeripheralsDTO::slots`].
    #[serde(rename = "rs232")]
    RS232,
    /// Games Master Cartridge (banked ROM + SN76489A).
    #[serde(rename = "gmc")]
    GamesMaster { path: String },
    /// Orchestra-90/CC.
    #[serde(rename = "orch90")]
    Orch90 { path: String },
    /// Sound/Speech Cartridge.
    #[serde(rename = "ssc")]
    SSC,
    /// MultiPak Interface; `[peripherals].slots` then holds its 4 slots.
    #[serde(rename = "mpi")]
    MPI,
}

/// One MultiPak slot's occupant (`[peripherals].slots`) — [`CartridgeDTO`]'s
/// sibling minus the bare-port-only kinds (nested MPI, RS-232). Maps to
/// [`crate::new_vm::SlotChoice`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind")]
pub enum SlotDTO {
    #[default]
    #[serde(rename = "empty")]
    Empty,
    #[serde(rename = "fd502")]
    FD502,
    #[serde(rename = "rompak")]
    ROMPak { path: String },
    #[serde(rename = "rtc")]
    RTC,
    #[serde(rename = "gmc")]
    GamesMaster { path: String },
    #[serde(rename = "orch90")]
    Orch90 { path: String },
    #[serde(rename = "ssc")]
    SSC,
}

/// `[peripherals]` section — section itself optional (a definition with no
/// `[peripherals]` table at all has an empty port), but `cartridge` is
/// required *within* the section once it's present: no `#[serde(default)]`,
/// so a schema-1 file's leftover `[peripherals]` (`mpi`/`rtc`/`fd502`/`rs232`
/// booleans, no `cartridge` key) fails with a "missing field" error instead
/// of silently loading as an empty port and dropping whatever those flags
/// meant (`io::load_one` has no schema-1 migration by design).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PeripheralsDTO {
    pub cartridge: CartridgeDTO,
    /// The MPI's 4 slots; meaningless — and omitted on save — unless
    /// `cartridge` is [`CartridgeDTO::MPI`]. An absent key defaults to all
    /// [`SlotDTO::Empty`], but `io::load_one` separately rejects an absent
    /// `slots` key while `cartridge` is [`CartridgeDTO::MPI`] — a missing
    /// loadout, not an intentionally empty one.
    #[serde(default, skip_serializing_if = "slots_all_empty")]
    pub slots: [SlotDTO; crate::MPI_SLOT_COUNT],
}

fn slots_all_empty(slots: &[SlotDTO; crate::MPI_SLOT_COUNT]) -> bool {
    slots.iter().all(|slot| matches!(slot, SlotDTO::Empty))
}
